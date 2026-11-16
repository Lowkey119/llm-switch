//! 响应缓存（M5.5-A）：完全相同的请求直接回放上次结果，不回源。
//!
//! 设计口径（与计划 §5.5 的落地取舍）：
//! - 缓存键含**入口协议**（§5.5.1），因此缓存的是「客户端出口形状」的最终响应
//!   （非流式 = 变换后的 JSON body；流式 = 已发给客户端的 SSE 帧序列），
//!   回放无需再过适配器，逐字节还原，正确性最好。跨协议共用一条缓存需要
//!   归一化事件模型（4 解码 + 4 编码），收益近零（不同协议的入站 body 本就
//!   算出不同的键），故不做。
//! - 参数中心（M5 分层）的改动不进键：参数一改由 `params://changed` 广播清理
//!   相关条目（§5.5.4，C 片接线）。
//! - 本切片只有内存热层（LRU + 字节预算）；磁盘冷层在 B 片接入同一结构。

use crate::config::{KeyGranularity, ResponseCacheCfg};
use crate::gateway::usage::parser::TokenUsage;
use crate::types::{now_ms, Protocol};
use axum::body::Bytes;
use axum::http::HeaderMap;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use futures::stream::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// 单条缓存条目上限：超过的大回答不值得占共享预算。
const MAX_ENTRY_BYTES: usize = 4 * 1024 * 1024;
/// 从回放帧里提取 usage 时的行数上限（与 UsageRecorder 同口径）。
const EVENT_CAP: usize = 2048;

/// 一条可回放的响应（已是客户端出口形状）。
#[derive(Clone)]
pub struct CachedResponse {
    /// 产生该响应的后端名（usage 行的 backend 列）。
    pub producer: String,
    pub model: String,
    pub stream: bool,
    pub status: u16,
    /// 非流式：JSON 文本；流式：拼接后的 SSE 帧。
    pub body: Arc<Vec<u8>>,
    pub created_at_ms: i64,
    pub ttl_secs: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

impl CachedResponse {
    fn expired(&self, now: i64) -> bool {
        self.ttl_secs > 0 && now - self.created_at_ms > self.ttl_secs * 1000
    }
    fn bytes(&self) -> usize {
        self.body.len()
    }
}

/// 生效策略（global ⊕ 策略表 ⊕ per_model 之后的单请求视角）。
#[derive(Debug, Clone)]
pub struct AppliedPolicy {
    pub ttl_secs: i64,
    pub granularity: KeyGranularity,
    pub replay: String,
}

/// 策略表叠加层（M5.5-C，cache_policies 表）：global 行 + model 行。
/// 优先级从高到低：model 表行 > yaml per_model > global 表行 > yaml 全局。
#[derive(Default, Clone)]
pub struct PolicyOverlay {
    pub global: Option<Value>,
    pub models: HashMap<String, Value>,
}

/// 按优先级高→低收集策略 JSON 层。
fn layers<'a>(
    cfg: &'a ResponseCacheCfg,
    overlays: &'a PolicyOverlay,
    model: &str,
) -> Vec<&'a Value> {
    let mut v: Vec<&Value> = Vec::new();
    if let Some(m) = overlays.models.get(model) {
        v.push(m);
    }
    if let Some(m) = cfg.per_model.get(model) {
        v.push(m);
    }
    if let Some(g) = overlays.global.as_ref() {
        v.push(g);
    }
    v
}

/// 全局开关 + 策略表 + per_model 覆写解析成单模型的生效策略。
/// 字段名：enabled / ttl / key_granularity(strict|loose) / replay。
pub fn policy_for(cfg: &ResponseCacheCfg, overlays: &PolicyOverlay, model: &str) -> AppliedPolicy {
    let mut p = AppliedPolicy {
        ttl_secs: cfg.default_ttl_secs,
        granularity: cfg.key_granularity,
        replay: cfg.replay.clone(),
    };
    // 低优先到高逐层覆盖。
    for layer in layers(cfg, overlays, model).into_iter().rev() {
        if let Some(ttl) = layer.get("ttl").and_then(Value::as_i64) {
            p.ttl_secs = ttl;
        }
        if let Some(g) = layer.get("key_granularity").and_then(Value::as_str) {
            p.granularity = match g {
                "loose" => KeyGranularity::Loose,
                "strict" => KeyGranularity::Strict,
                _ => p.granularity,
            };
        }
        if let Some(r) = layer.get("replay").and_then(Value::as_str) {
            p.replay = r.to_string();
        }
    }
    p
}

/// 该模型的缓存总开关：取第一个带 enabled 的层，都没有则回 yaml 全局。
fn enabled_for(cfg: &ResponseCacheCfg, overlays: &PolicyOverlay, model: &str) -> bool {
    for layer in layers(cfg, overlays, model) {
        if let Some(Value::Bool(b)) = layer.get("enabled") {
            return *b;
        }
    }
    cfg.enabled
}

/// 单请求的缓存判定结果。
pub enum Decision {
    /// 该模型未启用响应缓存：静默（不打日志、不写 cache_status）。
    Off,
    /// 启用了但本请求不适合缓存（附原因，写 `bypass:<reason>`）。
    Bypass(&'static str),
    /// 参与缓存：给出键与生效策略。
    Lookup { key: String, policy: AppliedPolicy },
}

/// 温度缺省按协议默认 1.0（未知采样 ⇒ 不回放，宁保守）。
fn temperature_of(body: &Value) -> f64 {
    body.get("temperature")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
}

fn has_tools(body: &Value) -> bool {
    body.get("tools")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
}

/// 计算缓存键（strict=整个 body；loose=对话+工具，忽略采样参数）。
pub fn cache_key(entry: Protocol, model: &str, body: &Value, gran: KeyGranularity) -> String {
    use crate::gateway::json_canonical::canonical_json_string;
    let src = match gran {
        KeyGranularity::Strict => canonical_json_string(body),
        KeyGranularity::Loose => {
            let convo = body
                .get("messages")
                .or_else(|| body.get("contents"))
                .cloned()
                .unwrap_or(Value::Null);
            let tools = body.get("tools").cloned().unwrap_or(Value::Null);
            format!(
                "{}|{}",
                canonical_json_string(&convo),
                canonical_json_string(&tools)
            )
        }
    };
    let mut h = Sha256::new();
    h.update(entry.as_str().as_bytes());
    h.update([0u8]);
    h.update(model.as_bytes());
    h.update([0u8]);
    h.update(src.as_bytes());
    let digest = h.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

/// 全局开关 + 策略表 + per_model 叠加后的单请求判定：开关 → bypass 规则 → 键。
pub fn decide(
    cfg: &ResponseCacheCfg,
    overlays: &PolicyOverlay,
    entry: Protocol,
    model: &str,
    body: &Value,
    headers: &HeaderMap,
) -> Decision {
    if !enabled_for(cfg, overlays, model) {
        return Decision::Off;
    }
    if headers
        .get("x-llmswitch-no-cache")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "1")
    {
        return Decision::Bypass("client");
    }
    if cfg.exclude_when_tools && has_tools(body) {
        return Decision::Bypass("tools");
    }
    if temperature_of(body) > cfg.max_temperature + f64::EPSILON {
        return Decision::Bypass("temperature");
    }
    let policy = policy_for(cfg, overlays, model);
    Decision::Lookup {
        key: cache_key(entry, model, body, policy.granularity),
        policy,
    }
}

/// 对外统计（Cache 页 / metrics 在 E 片消费）。
#[derive(Default)]
pub struct CacheStats {
    pub hits: AtomicU64,
    pub misses: AtomicU64,
    pub bypass: AtomicU64,
    pub saved_input_tokens: AtomicU64,
    pub saved_output_tokens: AtomicU64,
}

struct Slot {
    entry: CachedResponse,
    last_used: u64,
}

/// 磁盘冷层索引元数据（内容留在盘上，按需载入）。
#[derive(Clone)]
struct DiskSlot {
    size: u64,
    created_at_ms: i64,
    ttl_secs: i64,
    last_used: u64,
    /// 轻量元数据副本：purge 按模型/后端过滤时不必读盘。
    model: String,
    producer: String,
}

impl DiskSlot {
    fn expired(&self, now: i64) -> bool {
        self.ttl_secs > 0 && now - self.created_at_ms > self.ttl_secs * 1000
    }
}

/// 磁盘冷层（B 片，借 omlx paged_ssd_cache 思路）：每条一个文件，
/// 按 key 前两位 hex 分桶目录；硬上限由用户设定（D9，GB，0=关闭）。
struct DiskTier {
    root: PathBuf,
    budget_bytes: u64,
}

/// 冷层文件的自包含序列化格式（body 走 base64，二进制安全）。
#[derive(Serialize, Deserialize)]
struct DiskEntry {
    producer: String,
    model: String,
    stream: bool,
    status: u16,
    body_b64: String,
    created_at_ms: i64,
    ttl_secs: i64,
    input_tokens: i64,
    output_tokens: i64,
}

impl DiskEntry {
    fn from_entry(entry: &CachedResponse) -> Option<Self> {
        Some(Self {
            producer: entry.producer.clone(),
            model: entry.model.clone(),
            stream: entry.stream,
            status: entry.status,
            body_b64: STANDARD.encode(entry.body.as_slice()),
            created_at_ms: entry.created_at_ms,
            ttl_secs: entry.ttl_secs,
            input_tokens: entry.input_tokens,
            output_tokens: entry.output_tokens,
        })
    }
    fn into_entry(self) -> Option<CachedResponse> {
        let body = STANDARD.decode(&self.body_b64).ok()?;
        Some(CachedResponse {
            producer: self.producer,
            model: self.model,
            stream: self.stream,
            status: self.status,
            body: Arc::new(body),
            created_at_ms: self.created_at_ms,
            ttl_secs: self.ttl_secs,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        })
    }
}

#[derive(Default)]
struct Inner {
    map: HashMap<String, Slot>,
    seq: u64,
    bytes: usize,
    /// 冷层索引：key → 元数据（启动扫盘重建，运行期随写穿/删除增量维护）。
    disk_index: HashMap<String, DiskSlot>,
    disk_bytes: u64,
}

/// 分层响应缓存：内存热层（LRU+TTL）+ 可选磁盘冷层（写穿）。
/// 新条目写穿到盘（重启扫盘继续命中，验收口径）；热层淘汰只丢内存副本。
pub struct ResponseCache {
    inner: Mutex<Inner>,
    budget_bytes: usize,
    disk: Option<DiskTier>,
    pub stats: CacheStats,
}

/// key（小写 hex）前两位作分桶目录名。
fn disk_path(root: &Path, key: &str) -> Option<PathBuf> {
    let mut chars = key.chars();
    let (a, b) = (chars.next()?, chars.next()?);
    let mut path = root.join(format!("{a}{b}"));
    path.push(format!("{key}.json"));
    Some(path)
}

/// 冷层索引给出的一条待读记录。`stale=true` 表示验期没过、索引已在锁内摘掉，
/// 调用方只需（在锁外）删文件。
struct DiskProbe {
    path: PathBuf,
    size: u64,
    stale: bool,
}

impl ResponseCache {
    pub fn new(memory_mb: u32) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            budget_bytes: (memory_mb as usize).max(1) * 1024 * 1024,
            disk: None,
            stats: CacheStats::default(),
        }
    }

    /// 启用磁盘冷层并立即扫盘恢复（persist && disk_gb>0 的判定在调用方）。
    pub fn new_with_disk(memory_mb: u32, root: PathBuf, disk_gb: u64) -> Self {
        Self::with_disk_budget(memory_mb, root, disk_gb * 1024 * 1024 * 1024)
    }

    fn with_disk_budget(memory_mb: u32, root: PathBuf, disk_budget_bytes: u64) -> Self {
        let me = Self {
            inner: Mutex::new(Inner::default()),
            budget_bytes: (memory_mb as usize).max(1) * 1024 * 1024,
            disk: Some(DiskTier {
                root,
                budget_bytes: disk_budget_bytes.max(1),
            }),
            stats: CacheStats::default(),
        };
        me.scan_startup();
        me
    }

    pub fn get(&self, key: &str) -> Option<CachedResponse> {
        // 这把锁是全网关 get/put 的公共通道，所以锁里**只做内存记账与元数据
        // 查询**：读文件、解析、删文件全部放到锁外（一次冷读可能就是几 MB，
        // 以前整段 `fs::read` 都压在锁上，等于让所有并发请求排队等一个磁盘读）。
        let probe = {
            // 先解引用成 &mut Inner：混用 guard 的 Deref/DerefMut 会被借用检查拆开。
            let inner = &mut *self.inner.lock().expect("cache lock poisoned");
            let now = now_ms();
            inner.seq += 1;
            let seq = inner.seq;
            if let Some(slot) = inner.map.get_mut(key) {
                if slot.entry.expired(now) {
                    inner.bytes -= slot.entry.bytes();
                    inner.map.remove(key);
                } else {
                    slot.last_used = seq;
                    return Some(slot.entry.clone());
                }
            }
            self.disk_probe(inner, key)
        };

        // —— 锁外：真正的磁盘访问。 ——
        let probe = probe?;
        if probe.stale {
            // 过期项的索引与字节已在锁内摘掉，这里只补一个删文件。
            let _ = fs::remove_file(&probe.path);
            return None;
        }
        let raw = match fs::read(&probe.path) {
            Ok(b) => b,
            Err(_) => {
                self.disk_drop(key, probe.size);
                return None;
            }
        };
        let entry = match serde_json::from_slice::<DiskEntry>(&raw)
            .ok()
            .and_then(DiskEntry::into_entry)
        {
            Some(e) => e,
            None => {
                // 自愈：坏文件（截断/损坏的 base64）删掉，索引同步清干净——
                // 原来走 `into_entry()?` 那条路既不删文件也不清索引，坏文件
                // 会被每次冷读重新读一遍。
                let _ = fs::remove_file(&probe.path);
                self.disk_drop(key, probe.size);
                return None;
            }
        };

        // 回锁：提升回热层（热层满则由 mem_insert 顺带淘汰）。
        let mut inner = self.inner.lock().expect("cache lock poisoned");
        let inner = &mut *inner;
        inner.seq += 1;
        let seq = inner.seq;
        if let Some(s) = inner.disk_index.get_mut(key) {
            s.last_used = seq;
        }
        self.mem_insert(inner, key.to_string(), entry.clone());
        Some(entry)
    }

    /// 冷层元数据（**只在锁内跑**）：验期 → 摘掉过期项 → 给出锁外要用的路径。
    fn disk_probe(&self, inner: &mut Inner, key: &str) -> Option<DiskProbe> {
        let tier = self.disk.as_ref()?;
        let slot = inner.disk_index.get(key)?.clone();
        let path = disk_path(&tier.root, key)?;
        if slot.expired(now_ms()) {
            inner.disk_bytes = inner.disk_bytes.saturating_sub(slot.size);
            inner.disk_index.remove(key);
            return Some(DiskProbe {
                path,
                size: slot.size,
                stale: true,
            });
        }
        Some(DiskProbe {
            path,
            size: slot.size,
            stale: false,
        })
    }

    /// 锁内把冷层索引里的一个条目摘掉（文件的删除由调用方在锁外做）。
    fn disk_drop(&self, key: &str, size: u64) {
        let mut inner = self.inner.lock().expect("cache lock poisoned");
        if inner.disk_index.remove(key).is_some() {
            inner.disk_bytes = inner.disk_bytes.saturating_sub(size);
        }
    }

    pub fn put(&self, key: String, entry: CachedResponse) {
        let inserted = {
            let inner = &mut *self.inner.lock().expect("cache lock poisoned");
            self.mem_insert(inner, key.clone(), entry.clone())
        };
        if inserted {
            // 写穿落盘**必须在锁外**：`create_dir_all` + `fs::write` 是同步磁盘
            // IO，压在锁里等于让全网关的缓存操作排队等一次写盘。
            self.demote(key, &entry);
        }
    }

    /// 热层插入 + LRU 淘汰。返回是否真的落进热层（超大条目直接不收）。
    fn mem_insert(&self, inner: &mut Inner, key: String, entry: CachedResponse) -> bool {
        let size = entry.bytes();
        if size == 0 || size > MAX_ENTRY_BYTES || size > self.budget_bytes {
            return false;
        }
        if let Some(old) = inner.map.remove(&key) {
            inner.bytes -= old.entry.bytes();
        }
        // 先清过期，再按最久未用逐个淘汰到放得下。
        let now = now_ms();
        let expired: Vec<String> = inner
            .map
            .iter()
            .filter(|(_, s)| s.entry.expired(now))
            .map(|(k, _)| k.clone())
            .collect();
        for k in expired {
            if let Some(v) = inner.map.remove(&k) {
                inner.bytes -= v.entry.bytes();
            }
        }
        while inner.bytes + size > self.budget_bytes {
            let victim = inner
                .map
                .iter()
                .min_by_key(|(_, s)| s.last_used)
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    if let Some(v) = inner.map.remove(&k) {
                        inner.bytes -= v.entry.bytes();
                        // 淘汰者的冷层副本早已写穿，这里只管丢内存态。
                    }
                }
                None => break,
            }
        }
        inner.seq += 1;
        let seq = inner.seq;
        inner.bytes += size;
        inner.map.insert(
            key,
            Slot {
                entry,
                last_used: seq,
            },
        );
        true
    }

    /// 热 → 冷写入（写穿）。**先在锁外写文件，再回锁更新索引。**
    ///
    /// 同一 key 被 put 两次时**覆盖旧副本**：老实现「盘上已有就跳过」会让冷层
    /// 停在上一次的值——热层被淘汰、或者重启扫盘之后，读回的是**陈旧响应**
    /// （`x-llmswitch-no-cache` 强制刷新之后必然踩到）。
    fn demote(&self, key: String, entry: &CachedResponse) {
        let Some(tier) = &self.disk else { return };
        if entry.expired(now_ms()) {
            return;
        }
        let Some(path) = disk_path(&tier.root, &key) else {
            return;
        };
        let Some(de) = DiskEntry::from_entry(entry) else {
            return;
        };
        let Ok(raw) = serde_json::to_vec(&de) else {
            return;
        };
        // 锁外写盘（`fs::write` 先截断后写，同 key 并发 put 的场景下索引 size
        // 可能短暂对不上文件，这是纯记账问题，用 saturating 记账兜住即可）。
        if path
            .parent()
            .is_some_and(|d| fs::create_dir_all(d).is_err())
            || fs::write(&path, &raw).is_err()
        {
            tracing::warn!(target: "llm_switch::cache", ?path, "disk tier write failed");
            return;
        }
        let size = raw.len() as u64;
        // 锁内只做索引与字节记账；超限逐出要删的文件同样留到锁外。
        let stale_paths = {
            let mut inner = self.inner.lock().expect("cache lock poisoned");
            let inner = &mut *inner;
            inner.seq += 1;
            let seq = inner.seq;
            if let Some(old) = inner.disk_index.insert(
                key,
                DiskSlot {
                    size,
                    created_at_ms: entry.created_at_ms,
                    ttl_secs: entry.ttl_secs,
                    last_used: seq,
                    model: entry.model.clone(),
                    producer: entry.producer.clone(),
                },
            ) {
                inner.disk_bytes = inner.disk_bytes.saturating_sub(old.size);
            }
            inner.disk_bytes += size;
            self.trim_disk(inner)
        };
        for p in stale_paths {
            let _ = fs::remove_file(p);
        }
    }

    /// 冷层超硬上限：逐出最旧，直到目录体积回到上限内。
    /// **只在锁内摘索引记账**，返回要删的文件交给调用方锁外删。
    fn trim_disk(&self, inner: &mut Inner) -> Vec<PathBuf> {
        let mut stale = Vec::new();
        let Some(tier) = &self.disk else { return stale };
        while inner.disk_bytes > tier.budget_bytes {
            let victim = inner
                .disk_index
                .iter()
                .min_by_key(|(_, s)| s.last_used)
                .map(|(k, _)| k.clone());
            let Some(k) = victim else { break };
            if let Some(v) = inner.disk_index.remove(&k) {
                inner.disk_bytes = inner.disk_bytes.saturating_sub(v.size);
                if let Some(p) = disk_path(&tier.root, &k) {
                    stale.push(p);
                }
            }
        }
        stale
    }

    /// 启动扫盘：重建冷层索引、清过期与坏文件、超限削顶。
    fn scan_startup(&self) {
        let Some(tier) = &self.disk else { return };
        let Ok(buckets) = fs::read_dir(&tier.root) else {
            return;
        };
        let inner = &mut *self.inner.lock().expect("cache lock poisoned");
        let now = now_ms();
        for bucket in buckets.flatten() {
            if !bucket.path().is_dir() {
                continue;
            }
            let Ok(files) = fs::read_dir(bucket.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
                    continue;
                };
                let Ok(raw) = fs::read(&path) else { continue };
                match serde_json::from_slice::<DiskEntry>(&raw) {
                    Ok(de) if !(de.ttl_secs > 0 && now - de.created_at_ms > de.ttl_secs * 1000) => {
                        inner.seq += 1;
                        let seq = inner.seq;
                        inner.disk_bytes += raw.len() as u64;
                        inner.disk_index.insert(
                            stem,
                            DiskSlot {
                                size: raw.len() as u64,
                                created_at_ms: de.created_at_ms,
                                ttl_secs: de.ttl_secs,
                                last_used: seq,
                                model: de.model,
                                producer: de.producer,
                            },
                        );
                    }
                    _ => {
                        // 过期或损坏：自愈删除。
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }
        // 启动期无人竞争，削顶要删的文件就地删掉（锁内 IO 仅此一处）。
        for p in self.trim_disk(inner) {
            let _ = fs::remove_file(p);
        }
        if !inner.disk_index.is_empty() {
            tracing::info!(target: "llm_switch::cache",
                entries = inner.disk_index.len(),
                bytes = inner.disk_bytes,
                "response cache disk tier scanned");
        }
    }

    /// 手动清空（热层+冷层，C 片 purge 入口用）。返回清掉的条数（写穿下同一条
    /// 目两层都有，按 key 去重只计一次）。
    pub fn clear(&self) -> usize {
        // 索引在锁内清，**删文件留到锁外**：一次清空可能就是几百个文件。
        let (n, stale) = {
            let inner = &mut *self.inner.lock().expect("cache lock poisoned");
            let mut keys: std::collections::HashSet<&String> = inner.map.keys().collect();
            for k in inner.disk_index.keys() {
                keys.insert(k);
            }
            let n = keys.len();
            inner.map.clear();
            inner.bytes = 0;
            let mut stale = Vec::new();
            if let Some(tier) = &self.disk {
                for k in inner.disk_index.keys() {
                    if let Some(p) = disk_path(&tier.root, k) {
                        stale.push(p);
                    }
                }
                inner.disk_index.clear();
                inner.disk_bytes = 0;
            }
            (n, stale)
        };
        for p in stale {
            let _ = fs::remove_file(p);
        }
        n
    }

    /// 按条件清理（C 片）：match_fn(model, producer_backend) 为真则删（热层+冷层，
    /// 写穿下同一条目两层都有，按 key 去重只计一次）。
    pub fn purge_where(&self, match_fn: impl Fn(&str, &str) -> bool) -> usize {
        // 显式持有 guard 而不是 `&mut *lock()`：循环末尾要先放锁再删文件。
        let mut guard = self.inner.lock().expect("cache lock poisoned");
        let inner = &mut *guard;
        let mut victims: std::collections::HashSet<String> = inner
            .map
            .iter()
            .filter(|(_, s)| match_fn(&s.entry.model, &s.entry.producer))
            .map(|(k, _)| k.clone())
            .collect();
        if self.disk.is_some() {
            for (k, _s) in inner
                .disk_index
                .iter()
                .filter(|(_, s)| match_fn(&s.model, &s.producer))
            {
                victims.insert(k.clone());
            }
        }
        let n = victims.len();
        // 同 clear：锁内摘索引，锁外删文件。
        let mut stale = Vec::new();
        for k in victims {
            if let Some(v) = inner.map.remove(&k) {
                inner.bytes -= v.entry.bytes();
            }
            if let Some(v) = inner.disk_index.remove(&k) {
                inner.disk_bytes = inner.disk_bytes.saturating_sub(v.size);
                if let Some(tier) = &self.disk {
                    if let Some(p) = disk_path(tier.root.as_path(), &k) {
                        stale.push(p);
                    }
                }
            }
        }
        drop(guard);
        for p in stale {
            let _ = fs::remove_file(p);
        }
        n
    }

    pub fn len(&self) -> usize {
        let inner = self.inner.lock().expect("cache lock poisoned");
        inner.map.len() + inner.disk_index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 热层占用字节。
    pub fn bytes(&self) -> usize {
        self.inner.lock().expect("cache lock poisoned").bytes
    }

    /// 冷层占用字节（Cache 页显示「当前占用」）。
    pub fn disk_bytes(&self) -> u64 {
        self.inner.lock().expect("cache lock poisoned").disk_bytes
    }

    pub fn budget_bytes(&self) -> usize {
        self.budget_bytes
    }

    /// 冷层硬上限字节（0 表示未启用）。
    pub fn disk_budget_bytes(&self) -> u64 {
        self.disk.as_ref().map(|d| d.budget_bytes).unwrap_or(0)
    }

    /// 命中记账（含本次省下的 token 数）。
    pub fn note_hit(&self, saved_input: u64, saved_output: u64) {
        self.stats.hits.fetch_add(1, Ordering::Relaxed);
        self.stats
            .saved_input_tokens
            .fetch_add(saved_input, Ordering::Relaxed);
        self.stats
            .saved_output_tokens
            .fetch_add(saved_output, Ordering::Relaxed);
    }

    pub fn note_miss(&self) {
        self.stats.misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_bypass(&self) {
        self.stats.bypass.fetch_add(1, Ordering::Relaxed);
    }
}

// -- 分层预算的运行时覆盖层 ----------------------------------------------------
//
// settings 表 `cache.*` 覆盖 config.yaml 出厂值（与 `server.port` 同款口径）。预算改动**下次网关重启生效**——ResponseCache 在 State 构造时定型，
// 热换预算要么丢缓存要么重建索引，收益不值当，UI 如实标注。

/// 冷热分层生效预算（覆盖层解析后的单视角）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TierCfg {
    pub memory_mb: u32,
    pub disk_gb: u32,
    pub persist: bool,
}

/// 读覆盖层：settings 有值且合法才覆盖 yaml；脏值静默忽略（保存前已校验）。
pub fn tier_from_store(store: &crate::store::Store, yaml: &ResponseCacheCfg) -> TierCfg {
    let get = |key: &str| store.get_setting(key).ok().flatten();
    let mut t = TierCfg {
        memory_mb: yaml.memory_mb,
        disk_gb: yaml.disk_gb,
        persist: yaml.persist,
    };
    if let Some(v) = get("cache.memory_mb") {
        if let Ok(n) = v.trim().parse::<u32>() {
            if (1..=MAX_TIER_MEMORY_MB).contains(&n) {
                t.memory_mb = n;
            }
        }
    }
    if let Some(v) = get("cache.disk_gb") {
        if let Ok(n) = v.trim().parse::<u32>() {
            if n <= MAX_TIER_DISK_GB {
                t.disk_gb = n;
            }
        }
    }
    if let Some(v) = get("cache.persist") {
        match v.trim() {
            "true" => t.persist = true,
            "false" => t.persist = false,
            _ => {}
        }
    }
    t
}

/// 保存覆盖层：空串 = 回 yaml 出厂值；非法值当场拒绝（与静默读侧互补）。
pub fn tier_save(
    store: &crate::store::Store,
    yaml: &ResponseCacheCfg,
    values: &Value,
) -> Result<TierCfg, String> {
    let mut staged: Vec<(&'static str, String)> = Vec::new();
    if let Some(v) = values.get("memory_mb") {
        let s = v.as_str().map(str::trim).unwrap_or("").to_string();
        if s.is_empty() {
            staged.push(("cache.memory_mb", String::new()));
        } else {
            let n: u32 = s.parse().map_err(|_| {
                format!("热层预算须是整数 MB（1..={MAX_TIER_MEMORY_MB}），收到 {s:?}")
            })?;
            if !(1..=MAX_TIER_MEMORY_MB).contains(&n) {
                return Err(format!(
                    "热层预算须在 1..={MAX_TIER_MEMORY_MB} MB，收到 {n}"
                ));
            }
            staged.push(("cache.memory_mb", n.to_string()));
        }
    }
    if let Some(v) = values.get("disk_gb") {
        let s = v.as_str().map(str::trim).unwrap_or("").to_string();
        if s.is_empty() {
            staged.push(("cache.disk_gb", String::new()));
        } else {
            let n: u32 = s.parse().map_err(|_| {
                format!("冷层上限须是整数 GB（0..={MAX_TIER_DISK_GB}，0=关闭），收到 {s:?}")
            })?;
            if n > MAX_TIER_DISK_GB {
                return Err(format!("冷层上限不能超过 {MAX_TIER_DISK_GB} GB，收到 {n}"));
            }
            staged.push(("cache.disk_gb", n.to_string()));
        }
    }
    if let Some(v) = values.get("persist") {
        let s = v.as_str().map(str::trim).unwrap_or("").to_string();
        if !s.is_empty() && s != "true" && s != "false" {
            return Err(format!("持久化开关只认 true/false，收到 {s:?}"));
        }
        staged.push(("cache.persist", s));
    }
    for (k, v) in staged {
        store
            .set_setting(k, &v)
            .map_err(|e| format!("写入 {k} 失败：{e}"))?;
    }
    Ok(tier_from_store(store, yaml))
}

/// 热层 MB 上限（64 GB——防手滑多打一个 0 的量级）。
const MAX_TIER_MEMORY_MB: u32 = 65_536;
/// 冷层 GB 上限（4 TB，防手滑多打一个 0）。
const MAX_TIER_DISK_GB: u32 = 4_096;

/// 从回放帧（入口协议形状）里再抽一遍 usage，供命中行记账。
pub fn usage_from_frames(entry: Protocol, body: &[u8]) -> Option<TokenUsage> {
    let text = std::str::from_utf8(body).ok()?;
    if text.trim_start().starts_with('{') {
        // 非流式：整体就是一个 JSON body。
        let v: Value = serde_json::from_str(text).ok()?;
        return match entry {
            Protocol::OpenaiChat => TokenUsage::from_openai_response(&v),
            Protocol::OpenaiResponses => TokenUsage::from_codex_response_auto(&v),
            Protocol::Gemini => TokenUsage::from_gemini_response(&v),
            Protocol::Anthropic => TokenUsage::from_claude_response(&v),
        };
    }
    let mut events: Vec<Value> = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let payload = rest.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(payload) {
            if events.len() < EVENT_CAP {
                events.push(v);
            }
        }
    }
    match entry {
        Protocol::OpenaiChat => TokenUsage::from_openai_stream_events(&events),
        Protocol::OpenaiResponses => TokenUsage::from_codex_stream_events_auto(&events),
        Protocol::Gemini => TokenUsage::from_gemini_stream_chunks(&events),
        Protocol::Anthropic => TokenUsage::from_claude_stream_events(&events),
    }
}

/// 写缓存所需的单次交换上下文。
pub struct CaptureSpec {
    pub key: String,
    pub producer: String,
    pub model: String,
    pub ttl_secs: i64,
    pub status: u16,
    pub entry: Protocol,
}

/// 流式旁路录制：把即将发给客户端的 SSE 帧攒下来，流正常结束后写入缓存；
/// 中途出错（含客户端提前断开导致 drop）则整条放弃，绝不缓存半截。
pub fn capture_stream<S>(
    stream: S,
    cache: Arc<ResponseCache>,
    spec: CaptureSpec,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    async_stream::stream! {
        let mut stream = Box::pin(stream);
        let mut buf: Vec<u8> = Vec::new();
        let mut abandoned = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(b) => {
                    if buf.len() <= MAX_ENTRY_BYTES {
                        buf.extend_from_slice(&b);
                    }
                    yield Ok(b);
                }
                Err(e) => {
                    abandoned = true;
                    yield Err(e);
                    break;
                }
            }
        }
        if !abandoned && !buf.is_empty() && buf.len() <= MAX_ENTRY_BYTES {
            let usage = usage_from_frames(spec.entry, &buf);
            cache.put(
                spec.key,
                CachedResponse {
                    producer: spec.producer,
                    model: spec.model,
                    stream: true,
                    status: spec.status,
                    body: Arc::new(buf),
                    created_at_ms: now_ms(),
                    ttl_secs: spec.ttl_secs,
                    input_tokens: usage.as_ref().map(|u| u.input_tokens as i64).unwrap_or(0),
                    output_tokens: usage.as_ref().map(|u| u.output_tokens as i64).unwrap_or(0),
                },
            );
        }
    }
}

/// 非流式写缓存（finalize_ok 末尾调用）。
pub fn capture_body(cache: &ResponseCache, spec: CaptureSpec, out_body: &Value) {
    let Ok(text) = serde_json::to_vec(out_body) else {
        return;
    };
    let usage = match spec.entry {
        Protocol::OpenaiChat => TokenUsage::from_openai_response(out_body),
        Protocol::OpenaiResponses => TokenUsage::from_codex_response_auto(out_body),
        Protocol::Gemini => TokenUsage::from_gemini_response(out_body),
        Protocol::Anthropic => TokenUsage::from_claude_response(out_body),
    };
    cache.put(
        spec.key,
        CachedResponse {
            producer: spec.producer,
            model: spec.model,
            stream: false,
            status: spec.status,
            body: Arc::new(text),
            created_at_ms: now_ms(),
            ttl_secs: spec.ttl_secs,
            input_tokens: usage.as_ref().map(|u| u.input_tokens as i64).unwrap_or(0),
            output_tokens: usage.as_ref().map(|u| u.output_tokens as i64).unwrap_or(0),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use serde_json::json;

    fn cfg_on() -> ResponseCacheCfg {
        ResponseCacheCfg {
            enabled: true,
            ..Default::default()
        }
    }

    fn body(temp: f64) -> Value {
        json!({"model": "m", "messages": [{"role": "user", "content": "hi"}], "temperature": temp})
    }

    #[test]
    fn global_off_means_silent() {
        let cfg = ResponseCacheCfg::default();
        assert!(matches!(
            decide(
                &cfg,
                &PolicyOverlay::default(),
                Protocol::OpenaiChat,
                "m",
                &body(0.0),
                &HeaderMap::new()
            ),
            Decision::Off
        ));
    }

    #[test]
    fn per_model_enables_despite_global_off() {
        let mut cfg = ResponseCacheCfg::default();
        cfg.per_model.insert("m".into(), json!({"enabled": true}));
        assert!(matches!(
            decide(
                &cfg,
                &PolicyOverlay::default(),
                Protocol::OpenaiChat,
                "m",
                &body(0.0),
                &HeaderMap::new()
            ),
            Decision::Lookup { .. }
        ));
    }

    #[test]
    fn per_model_disables_despite_global_on() {
        let mut cfg = cfg_on();
        cfg.per_model.insert("m".into(), json!({"enabled": false}));
        assert!(matches!(
            decide(
                &cfg,
                &PolicyOverlay::default(),
                Protocol::OpenaiChat,
                "m",
                &body(0.0),
                &HeaderMap::new()
            ),
            Decision::Off
        ));
    }

    #[test]
    fn bypass_rules() {
        let cfg = cfg_on();
        // 温度高于阈值（默认 0.0）→ bypass。
        assert!(matches!(
            decide(
                &cfg,
                &PolicyOverlay::default(),
                Protocol::OpenaiChat,
                "m",
                &body(0.7),
                &HeaderMap::new()
            ),
            Decision::Bypass("temperature")
        ));
        // 缺省温度按 1.0 处理，同样 bypass。
        let no_temp = json!({"model": "m", "messages": []});
        assert!(matches!(
            decide(
                &cfg,
                &PolicyOverlay::default(),
                Protocol::OpenaiChat,
                "m",
                &no_temp,
                &HeaderMap::new()
            ),
            Decision::Bypass("temperature")
        ));
        // 带 tools 默认拒绝。
        let with_tools =
            json!({"model":"m","temperature":0.0,"messages":[],"tools":[{"name":"f"}]});
        assert!(matches!(
            decide(
                &cfg,
                &PolicyOverlay::default(),
                Protocol::OpenaiChat,
                "m",
                &with_tools,
                &HeaderMap::new()
            ),
            Decision::Bypass("tools")
        ));
        // 客户端显式绕开。
        let mut h = HeaderMap::new();
        h.insert("x-llmswitch-no-cache", "1".parse().unwrap());
        assert!(matches!(
            decide(
                &cfg,
                &PolicyOverlay::default(),
                Protocol::OpenaiChat,
                "m",
                &body(0.0),
                &h
            ),
            Decision::Bypass("client")
        ));
    }

    #[test]
    fn strict_vs_loose_keys() {
        let base =
            json!({"model":"m","messages":[{"role":"user","content":"x"}],"temperature":0.0});
        let other_temp =
            json!({"model":"m","messages":[{"role":"user","content":"x"}],"temperature":0.2});
        let k1 = cache_key(Protocol::OpenaiChat, "m", &base, KeyGranularity::Strict);
        let k2 = cache_key(
            Protocol::OpenaiChat,
            "m",
            &other_temp,
            KeyGranularity::Strict,
        );
        assert_ne!(k1, k2, "strict must react to sampling params");
        let l1 = cache_key(Protocol::OpenaiChat, "m", &base, KeyGranularity::Loose);
        let l2 = cache_key(
            Protocol::OpenaiChat,
            "m",
            &other_temp,
            KeyGranularity::Loose,
        );
        assert_eq!(l1, l2, "loose must ignore sampling params");
        // 入口协议参与键：不同入口互不复用。
        assert_ne!(
            k1,
            cache_key(Protocol::Anthropic, "m", &base, KeyGranularity::Strict)
        );
    }

    fn entry(body: &[u8], ttl: i64) -> CachedResponse {
        CachedResponse {
            producer: "b".into(),
            model: "m".into(),
            stream: false,
            status: 200,
            body: Arc::new(body.to_vec()),
            created_at_ms: now_ms(),
            ttl_secs: ttl,
            input_tokens: 1,
            output_tokens: 2,
        }
    }

    #[test]
    fn lru_evicts_under_pressure() {
        // 预算 1500B：塞三条 600B，最久未用的第一条应被淘汰。
        let c = ResponseCache {
            inner: Mutex::new(Inner::default()),
            budget_bytes: 1500,
            disk: None,
            stats: CacheStats::default(),
        };
        c.put("k1".into(), entry(&vec![b'a'; 600], 60));
        c.put("k2".into(), entry(&vec![b'b'; 600], 60));
        // 触碰 k2 使其更“新”。
        assert!(c.get("k2").is_some());
        c.put("k3".into(), entry(&vec![b'c'; 600], 60));
        assert!(c.get("k1").is_none(), "oldest must be evicted");
        assert!(c.get("k2").is_some());
        assert!(c.get("k3").is_some());
        assert_eq!(c.bytes(), 1200);
    }

    #[test]
    fn ttl_expiry_on_get() {
        let c = ResponseCache::new(16);
        let mut e = entry(b"x", 1);
        e.created_at_ms = now_ms() - 2_000;
        c.put("k".into(), e);
        assert!(c.get("k").is_none());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn oversized_entry_skipped() {
        let c = ResponseCache {
            inner: Mutex::new(Inner::default()),
            budget_bytes: 100,
            disk: None,
            stats: CacheStats::default(),
        };
        c.put("big".into(), entry(&vec![b'z'; 500], 60));
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn overlay_table_beats_yaml_layers() {
        // yaml 全局关；策略表 global 行开；yaml per_model 关 m；策略表 model 行再开。
        let mut cfg = ResponseCacheCfg::default();
        cfg.per_model.insert("m".into(), json!({"enabled": false}));
        let mut ov = PolicyOverlay {
            global: Some(json!({"enabled": true, "ttl": 100})),
            models: HashMap::new(),
        };
        // 优先级：model(yaml) > global(表) → m 仍是关。
        assert!(matches!(
            decide(
                &cfg,
                &ov,
                Protocol::OpenaiChat,
                "m",
                &body(0.0),
                &HeaderMap::new()
            ),
            Decision::Off
        ));
        // model 表行最高，压过 yaml per_model 的关闭。
        ov.models
            .insert("m".into(), json!({"enabled": true, "ttl": 5}));
        let d = decide(
            &cfg,
            &ov,
            Protocol::OpenaiChat,
            "m",
            &body(0.0),
            &HeaderMap::new(),
        );
        assert!(matches!(d, Decision::Lookup { .. }));
        if let Decision::Lookup { policy, .. } = d {
            assert_eq!(policy.ttl_secs, 5, "model row must win over global row");
        }
        // 未点名模型吃 global 表行。
        let d2 = decide(
            &cfg,
            &ov,
            Protocol::OpenaiChat,
            "other",
            &body(0.0),
            &HeaderMap::new(),
        );
        if let Decision::Lookup { policy, .. } = d2 {
            assert_eq!(policy.ttl_secs, 100);
        } else {
            panic!("global overlay row should enable other models");
        }
    }

    #[test]
    fn purge_where_filters_model_and_producer() {
        let c = ResponseCache::new(16);
        let mut e1 = entry(b"x", 60);
        e1.model = "a".into();
        e1.producer = "b1".into();
        let mut e2 = entry(b"y", 60);
        e2.model = "b".into();
        e2.producer = "b2".into();
        c.put("k1".into(), e1);
        c.put("k2".into(), e2);
        assert_eq!(c.purge_where(|m, _| m == "a"), 1);
        assert!(c.get("k1").is_none());
        assert!(c.get("k2").is_some());
        assert_eq!(c.purge_where(|_, p| p == "b2"), 1);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn disk_demote_promote_and_restart_scan() {
        let dir = tempfile::tempdir().unwrap();
        let c = ResponseCache::with_disk_budget(1, dir.path().to_path_buf(), 10_000);
        c.put("k1aa".into(), entry(&vec![b'a'; 600], 600));
        c.put("k2bb".into(), entry(&vec![b'b'; 600], 600));
        assert!(c.get("k2bb").is_some()); // 触一下 k2，让 k1 成为最旧
        c.put("k3cc".into(), entry(&vec![b'c'; 600], 600)); // 挤爆热层 → k1 淘汰
                                                            // 写穿：落盘文件存在（分桶目录 = key 前两位）。
        assert!(dir.path().join("k1").join("k1aa.json").is_file());
        // 冷层取回并提升。
        assert!(
            c.get("k1aa").is_some(),
            "cold tier must serve demoted entry"
        );
        // 模拟重启：新实例扫同一目录。
        let c2 = ResponseCache::with_disk_budget(1, dir.path().to_path_buf(), 10_000);
        assert!(c2.get("k1aa").is_some(), "startup scan must restore hits");
    }

    #[test]
    fn disk_hard_cap_does_not_grow() {
        let dir = tempfile::tempdir().unwrap();
        let c = ResponseCache::with_disk_budget(1, dir.path().to_path_buf(), 1500);
        // demote 现在自己加锁（文件 IO 在锁外），直接连着写穿即可。
        for i in 0..5 {
            c.demote(format!("kk{i}0"), &entry(&vec![b'x'; 600], 600));
        }
        assert!(c.disk_bytes() <= 1500, "cold tier must respect hard cap");
    }

    /// 回归：同一 key 被 put 两次时，冷层必须跟着换新。老实现「盘上已有就跳过」，
    /// 于是热层是新响应、冷层是上一次的旧响应——热层被 LRU 淘汰、或者重启扫盘
    /// 之后，读回的是**陈旧结果**（`x-llmswitch-no-cache` 强制刷新后必然踩到）。
    #[test]
    fn second_put_overwrites_the_stale_disk_copy() {
        let dir = tempfile::tempdir().unwrap();
        let c = ResponseCache::with_disk_budget(1, dir.path().to_path_buf(), 10_000);
        c.put("aa11".into(), entry(b"first-response", 600));
        c.put("aa11".into(), entry(b"second-response", 600));

        // 直接读盘上的字节，绕开热层。
        let raw = std::fs::read(dir.path().join("aa").join("aa11.json")).unwrap();
        let de: DiskEntry = serde_json::from_slice(&raw).expect("cold copy must be valid json");
        let body = STANDARD
            .decode(&de.body_b64)
            .expect("cold copy must decode");
        assert_eq!(body, b"second-response", "冷层留着上一次的陈旧响应");

        // 新实例扫盘读到的也必须是新值。
        let c2 = ResponseCache::with_disk_budget(1, dir.path().to_path_buf(), 10_000);
        let got = c2.get("aa11").expect("restart must restore the hit");
        assert_eq!(&got.body[..], b"second-response");
    }

    #[test]
    fn scan_self_heals_corrupt_and_expired() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("zz")).unwrap();
        std::fs::write(dir.path().join("zz/zz01.json"), b"not json").unwrap();
        let expired = serde_json::json!({
            "producer": "b", "model": "m", "stream": false, "status": 200,
            "body_b64": "", "created_at_ms": 1, "ttl_secs": 1,
            "input_tokens": 0, "output_tokens": 0
        });
        std::fs::write(
            dir.path().join("zz/zz02.json"),
            serde_json::to_vec(&expired).unwrap(),
        )
        .unwrap();
        let c = ResponseCache::with_disk_budget(1, dir.path().to_path_buf(), 10_000);
        assert!(
            !dir.path().join("zz/zz01.json").exists(),
            "corrupt file must be purged"
        );
        assert!(
            !dir.path().join("zz/zz02.json").exists(),
            "expired file must be purged"
        );
        assert_eq!(c.disk_bytes(), 0);
    }
    #[test]
    fn usage_parsed_from_replay_frames() {
        let sse = "data: {\"choices\":[{\"message\":{\"content\":\"hi\"},\"index\":0}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\ndata: [DONE]\n\n";
        let u = usage_from_frames(Protocol::OpenaiChat, sse.as_bytes()).unwrap();
        assert_eq!(u.input_tokens, 7);
        assert_eq!(u.output_tokens, 3);
    }

    #[test]
    fn tier_overlay_beats_yaml_and_ignores_dirty() {
        let store = crate::store::Store::in_memory().unwrap();
        let yaml = ResponseCacheCfg {
            memory_mb: 256,
            disk_gb: 8,
            persist: true,
            ..Default::default()
        };
        // 没覆盖 = yaml 出厂值。
        assert_eq!(
            tier_from_store(&store, &yaml),
            TierCfg {
                memory_mb: 256,
                disk_gb: 8,
                persist: true
            }
        );
        // 合法覆盖。
        store.set_setting("cache.memory_mb", "512").unwrap();
        store.set_setting("cache.disk_gb", "0").unwrap();
        store.set_setting("cache.persist", "false").unwrap();
        assert_eq!(
            tier_from_store(&store, &yaml),
            TierCfg {
                memory_mb: 512,
                disk_gb: 0,
                persist: false
            }
        );
        // 脏值静默忽略 = 回 yaml 出厂值（保守兜底口径）。
        store.set_setting("cache.memory_mb", "abc").unwrap();
        store.set_setting("cache.persist", "yes").unwrap();
        let t = tier_from_store(&store, &yaml);
        assert_eq!(t.memory_mb, 256, "脏值回 yaml 出厂值");
        assert!(t.persist, "脏 persist 回 yaml 出厂值");
    }

    #[test]
    fn tier_save_validates_and_resets_to_yaml() {
        let store = crate::store::Store::in_memory().unwrap();
        let yaml = ResponseCacheCfg {
            memory_mb: 256,
            disk_gb: 8,
            persist: true,
            ..Default::default()
        };
        let t = tier_save(
            &store,
            &yaml,
            &json!({ "memory_mb": "1024", "disk_gb": "20", "persist": "false" }),
        )
        .unwrap();
        assert_eq!(t.memory_mb, 1024);
        assert_eq!(t.disk_gb, 20);
        assert!(!t.persist);
        // 非法值当场拒，且不改库。
        assert!(tier_save(&store, &yaml, &json!({ "memory_mb": "0" })).is_err());
        assert!(tier_save(&store, &yaml, &json!({ "memory_mb": "70000" })).is_err());
        assert!(tier_save(&store, &yaml, &json!({ "disk_gb": "-1" })).is_err());
        assert!(tier_save(&store, &yaml, &json!({ "persist": "maybe" })).is_err());
        assert_eq!(
            store.get_setting("cache.memory_mb").unwrap().unwrap(),
            "1024"
        );
        // 空串 = 回 yaml。
        let t = tier_save(&store, &yaml, &json!({ "memory_mb": "", "disk_gb": "" })).unwrap();
        assert_eq!(t.memory_mb, 256);
        assert_eq!(t.disk_gb, 8);
    }
}
