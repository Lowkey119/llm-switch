//! Monitor 活跃流注册表：进行中的交换在这里挂号，GUI 每秒快照后经
//! Tauri event 推给 Monitor 页（headless 模式无人订阅也保持零成本语义）。
//!
//! M12-C1 起登记时机从「上游响应头到达」提前到「请求进入网关」，于是浮条能看到
//! 排队与 prefill 两段——原先那会儿还没挂号，最需要看的「上游在啃 prompt」阶段是空的。
//! 提前登记的代价是「中途返回的路径会不会忘了注销」，用 [`ActiveSlot`] 的 Drop 兜住：
//! 谁持有谁负责，最后一个持有者松手就注销，漏不掉。

use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// One in-flight streaming exchange.
pub struct ActiveStream {
    pub id: u64,
    pub backend: String,
    pub model: String,
    pub entry: String,
    /// 发起这条请求的代理（`origin` 从 socket 反查出来的）；None = 认不出。
    /// 单行速率线靠它只看自己那一家，usage 的 `client` 列也落这个。
    pub app: Option<String>,
    /// 发起方代理的**进程号**（`origin` 一并反查）。同一家代理开多个实例时
    /// `app` 分不出谁是谁（BUG-1），速率出口按 pid 链过滤，各实例只看自己。
    pub pid: Option<u32>,
    /// 请求进入网关的时刻（排队段计时起点）。
    pub started: Instant,
    /// 真正把请求发给上游的时刻；None = 还在网关这边排队/装配。
    pub dispatched: Option<Instant>,
    /// 估算输入 token（没纠偏系数时的兜底分子，字节启发式 ≈ 老口径）。
    pub est_input: u64,
    /// 请求块内容长度（新口径 prefill 分子：发给上游的整包字节）。
    pub req_bytes: u64,
    pub ttft_ms: Option<u64>,
    /// decode 首字送达时刻（`ttft_ms` 的真值锚点，速率相减用）。
    pub first_at: Option<Instant>,
    /// 末个响应块送达时刻（封口 decode 跨度用；实时速率的「块尾」取此刻）。
    pub last_at: Option<Instant>,
    pub bytes: u64,
    pub data_events: u64,
}

/// 登记槽（RAII）：最后一个持有者 Drop 时自动注销。
/// 成功流由 UsageRecorder 持有（跟着响应体一起活到流结束），
/// 提前返回的错误路径则由 ExchangeMeta 带着一起落地。
pub struct ActiveSlot {
    pub id: u64,
    pub est_input: u64,
    /// 请求块字节（usage 收口时喂 prefill 纠偏样本）。
    pub req_bytes: u64,
    /// 挂号时的模型名（= 暴露名/别名，浮窗行与速率出口同源；纠偏键必须与它对齐）。
    pub model: String,
    /// 发起方代理（`origin` 反查）；usage 的 `client` 列由它落。
    pub app: Option<String>,
    reg: Arc<ActiveRegistry>,
}

impl ActiveSlot {
    pub fn mark_first_byte(&self) {
        self.reg.mark_first_byte(self.id);
    }
    pub fn bump(&self, bytes: u64, data_event: bool) {
        self.reg.bump(self.id, bytes, data_event);
    }
    /// 响应流累计字节（封口时喂 decode 纠偏样本；非流式恒 0，不混池）。
    pub fn stream_bytes(&self) -> u64 {
        self.reg
            .streams
            .lock()
            .expect("monitor lock poisoned")
            .get(&self.id)
            .map(|s| s.bytes)
            .unwrap_or(0)
    }
    /// 封口 prefill 跨度（发送起始 → decode 首字）；None = 没走完两段。
    pub fn prefill_span(&self) -> Option<std::time::Duration> {
        let map = self.reg.streams.lock().expect("monitor lock poisoned");
        let s = map.get(&self.id)?;
        s.first_at?.checked_duration_since(s.dispatched?)
    }
}

impl Drop for ActiveSlot {
    fn drop(&mut self) {
        self.reg.unregister(self.id);
    }
}

/// Wire-friendly view pushed to the UI (1 s cadence).
#[derive(Serialize, Clone)]
pub struct StreamSnapshot {
    pub id: u64,
    pub backend: String,
    pub model: String,
    pub entry: String,
    /// 发起方代理名（见 [`ActiveStream::app`]）。
    pub app: Option<String>,
    /// 发起方代理进程号（见 [`ActiveStream::pid`]）；速率出口按它分流多实例。
    pub pid: Option<u32>,
    pub elapsed_ms: u64,
    pub ttft_ms: Option<u64>,
    /// 响应流累计字节（新口径 decode 分子）。
    pub bytes: u64,
    pub data_events: u64,
    /// 请求块内容长度（新口径 prefill 分子）。
    pub req_bytes: u64,
    /// `queued`（还在网关这边）| `prefill`（已发出、还没吐字）| `generating`。
    pub phase: &'static str,
    /// 网关侧排队/装配耗时。
    pub queue_ms: u64,
    /// prefill 时长（已结束则为最终值，进行中则为实时值）。
    pub prefill_ms: u64,
    /// 估算输入 token（无纠偏系数时的兜底分子）。
    pub est_input_tokens: u64,
}

pub struct ActiveRegistry {
    next_id: AtomicU64,
    streams: Mutex<BTreeMap<u64, ActiveStream>>,
}

impl ActiveRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            next_id: AtomicU64::new(1),
            streams: Mutex::new(BTreeMap::new()),
        })
    }

    /// Register a stream; returns its slot id.
    pub fn register(&self, backend: &str, model: &str, entry: &str) -> u64 {
        self.register_with(backend, model, entry, 0, 0, None, None)
    }

    /// 登记并返回 RAII 槽（推荐入口：忘不掉注销）。
    pub fn slot(
        self: &Arc<Self>,
        backend: &str,
        model: &str,
        entry: &str,
        est_input: u64,
        req_bytes: u64,
        app: Option<String>,
        pid: Option<u32>,
    ) -> Arc<ActiveSlot> {
        let id = self.register_with(backend, model, entry, est_input, req_bytes, app.clone(), pid);
        Arc::new(ActiveSlot {
            id,
            est_input,
            req_bytes,
            model: model.to_string(),
            app: app.clone(),
            reg: self.clone(),
        })
    }

    fn register_with(
        &self,
        backend: &str,
        model: &str,
        entry: &str,
        est_input: u64,
        req_bytes: u64,
        app: Option<String>,
        pid: Option<u32>,
    ) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let stream = ActiveStream {
            id,
            backend: backend.to_string(),
            model: model.to_string(),
            entry: entry.to_string(),
            app,
            pid,
            started: Instant::now(),
            dispatched: None,
            est_input,
            req_bytes,
            ttft_ms: None,
            first_at: None,
            last_at: None,
            bytes: 0,
            data_events: 0,
        };
        self.streams
            .lock()
            .expect("monitor lock poisoned")
            .insert(id, stream);
        id
    }

    /// 请求已发出（prefill 计时起点）。
    pub fn mark_dispatched(&self, id: u64) {
        if let Some(s) = self
            .streams
            .lock()
            .expect("monitor lock poisoned")
            .get_mut(&id)
        {
            if s.dispatched.is_none() {
                s.dispatched = Some(Instant::now());
            }
        }
    }

    pub fn unregister(&self, id: u64) {
        self.streams
            .lock()
            .expect("monitor lock poisoned")
            .remove(&id);
    }

    /// First upstream byte observed (TTFT anchor = decode 首字送达).
    pub fn mark_first_byte(&self, id: u64) {
        let mut map = self.streams.lock().expect("monitor lock poisoned");
        if let Some(s) = map.get_mut(&id) {
            if s.ttft_ms.is_none() {
                s.ttft_ms = Some(s.started.elapsed().as_millis() as u64);
                s.first_at = Some(Instant::now());
            }
        }
    }

    /// Account bytes and one parsed SSE data event（bytes>0 即「块尾」推进）。
    pub fn bump(&self, id: u64, bytes: u64, data_event: bool) {
        let mut map = self.streams.lock().expect("monitor lock poisoned");
        if let Some(s) = map.get_mut(&id) {
            s.bytes += bytes;
            if bytes > 0 {
                s.last_at = Some(Instant::now());
            }
            if data_event {
                s.data_events += 1;
            }
        }
    }

    /// Live stream count per backend name (least_busy scoring input).
    pub fn count_by_backend(&self) -> std::collections::HashMap<String, usize> {
        let map = self.streams.lock().expect("monitor lock poisoned");
        let mut out = std::collections::HashMap::new();
        for s in map.values() {
            *out.entry(s.backend.clone()).or_insert(0) += 1;
        }
        out
    }

    /// Snapshot for the 1 s monitor push.
    /// 按模型统计此刻在飞的流数（M12-B：监控页与浮条都靠它判断「正在工作」）。
    pub fn count_by_model(&self) -> std::collections::HashMap<String, usize> {
        let mut out: std::collections::HashMap<String, usize> = Default::default();
        for s in self.streams.lock().expect("monitor lock poisoned").values() {
            *out.entry(s.model.clone()).or_insert(0) += 1;
        }
        out
    }

    /// 在飞流按 (后端, 模型) 计数（切片 17：监控页按后端分卡的「在飞」口径）。
    pub fn count_by_backend_model(&self) -> std::collections::HashMap<(String, String), usize> {
        let mut out: std::collections::HashMap<(String, String), usize> = Default::default();
        for s in self.streams.lock().expect("monitor lock poisoned").values() {
            *out.entry((s.backend.clone(), s.model.clone())).or_insert(0) += 1;
        }
        out
    }

    pub fn snapshot(&self) -> Vec<StreamSnapshot> {
        let map = self.streams.lock().expect("monitor lock poisoned");
        map.values()
            .map(|s| {
                let elapsed_ms = s.started.elapsed().as_millis() as u64;
                // 三段计时：排队（进网关→发出）/ prefill（发出→首个内容字节）/ 解码。
                let queue_ms = s
                    .dispatched
                    .map(|d| d.duration_since(s.started).as_millis() as u64)
                    .unwrap_or(elapsed_ms);
                let prefill_ms = match s.ttft_ms {
                    Some(t) => t.saturating_sub(queue_ms),
                    None if s.dispatched.is_some() => elapsed_ms.saturating_sub(queue_ms),
                    None => 0,
                };
                let phase = if s.dispatched.is_none() {
                    "queued"
                } else if s.ttft_ms.is_none() {
                    "prefill"
                } else {
                    "generating"
                };
                StreamSnapshot {
                    id: s.id,
                    backend: s.backend.clone(),
                    model: s.model.clone(),
                    entry: s.entry.clone(),
                    app: s.app.clone(),
                    pid: s.pid,
                    elapsed_ms,
                    ttft_ms: s.ttft_ms,
                    bytes: s.bytes,
                    data_events: s.data_events,
                    req_bytes: s.req_bytes,
                    phase,
                    queue_ms,
                    prefill_ms,
                    est_input_tokens: s.est_input,
                }
            })
            .collect()
    }
}

impl Default for ActiveRegistry {
    fn default() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            streams: Mutex::new(BTreeMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_drop_unregisters() {
        let reg = ActiveRegistry::new();
        let slot = reg.slot("b1", "m1", "openai", 120, 999, Some("pi".into()), Some(4242));
        assert_eq!(reg.snapshot().len(), 1);
        assert_eq!(reg.snapshot()[0].est_input_tokens, 120);
        assert_eq!(
            reg.snapshot()[0].req_bytes,
            999,
            "请求块长度要挂上，prefill 分子靠它"
        );
        assert_eq!(slot.model, "m1", "纠偏键要与浮窗行看到的模型名一致");
        assert_eq!(
            reg.snapshot()[0].app.as_deref(),
            Some("pi"),
            "归属要跟着快照出来，单行速率线只认这个字段"
        );
        assert_eq!(reg.snapshot()[0].phase, "queued");
        assert!(
            slot.prefill_span().is_none(),
            "没发也没首字，谈不上 prefill 跨度"
        );
        reg.mark_dispatched(slot.id);
        assert_eq!(reg.snapshot()[0].phase, "prefill");
        reg.mark_first_byte(slot.id);
        assert_eq!(reg.snapshot()[0].phase, "generating");
        assert!(
            slot.prefill_span().is_some(),
            "发出 → 首字就是封口 prefill 跨度（速率样本的分母）"
        );
        reg.bump(slot.id, 50, false);
        assert_eq!(
            slot.stream_bytes(),
            50,
            "响应流字节要能读回来喂 decode 纠偏"
        );
        drop(slot);
        assert!(reg.snapshot().is_empty(), "槽被丢弃即注销，不该漏");
    }

    #[test]
    fn register_bump_snapshot_unregister() {
        let reg = ActiveRegistry::new();
        let id = reg.register("b1", "m1", "anthropic");
        reg.mark_first_byte(id);
        reg.bump(id, 100, true);
        reg.bump(id, 50, true);
        let snap = reg.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].bytes, 150);
        assert_eq!(snap[0].data_events, 2);
        assert!(snap[0].ttft_ms.is_some());
        reg.unregister(id);
        assert!(reg.snapshot().is_empty());
    }
}
