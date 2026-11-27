//! 运行日志入库（M2 简化版 sink）：关键节点一行式结构化日志。
//!
//! 偏离计划说明：计划 §4.2 设想通用 tracing Layer；此处改为显式 `LogSink`
//! 助手——同样镜像一份到 tracing（RUST_LOG 控制台），但入库路径确定、
//! 可在测试中断言，不与全局 subscriber 装配耦合。后台批量写降级为同步
//! 小批量写（本地每请求 2~4 行，SQLite 微秒级，无背压风险）。

use crate::store::Store;
use crate::types::{now_ms, LogRow};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const RING_CAP: usize = 5000;

/// Short-coded operational log writer: SQLite (`logs`) + in-memory ring.
pub struct LogSink {
    store: Arc<Store>,
    ring: Mutex<VecDeque<LogRow>>,
    /// Tail fan-out for live viewers (GUI event pump subscribes).
    tx: tokio::sync::broadcast::Sender<LogRow>,
}

impl LogSink {
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        let (tx, _) = tokio::sync::broadcast::channel(1024);
        Arc::new(Self {
            store,
            ring: Mutex::new(VecDeque::with_capacity(RING_CAP)),
            tx,
        })
    }

    /// Record one structured event. `code` is the short code (e.g.
    /// `GW401 route.no_backend`); failures to persist never bubble up.
    pub fn log(&self, ev: Event<'_>) {
        let Event {
            level,
            code,
            message,
            request_id,
            backend,
            model,
            extra,
        } = ev;
        // Mirror to tracing for console/RUST_LOG consumers.
        let evt = tracing::span!(tracing::Level::INFO, "log");
        let _guard = evt.enter();
        match level {
            Level::Info => tracing::info!(
                code,
                request_id = request_id.unwrap_or(""),
                backend = backend.unwrap_or(""),
                model = model.unwrap_or(""),
                "{message}"
            ),
            Level::Warn => tracing::warn!(
                code,
                request_id = request_id.unwrap_or(""),
                backend = backend.unwrap_or(""),
                model = model.unwrap_or(""),
                "{message}"
            ),
            Level::Error => tracing::error!(
                code,
                request_id = request_id.unwrap_or(""),
                backend = backend.unwrap_or(""),
                model = model.unwrap_or(""),
                "{message}"
            ),
        }

        let row = LogRow {
            id: 0,
            ts: now_ms(),
            level: level.as_str().to_string(),
            code: Some(code.to_string()),
            message: message.to_string(),
            request_id: request_id.map(str::to_string),
            backend: backend.map(str::to_string),
            model: model.map(str::to_string),
            extra,
        };
        if let Err(e) = self.store.insert_logs(std::slice::from_ref(&row)) {
            tracing::warn!(target: "llm_switch::logs", error = %e, "log insert failed");
        }
        let mut ring = self.ring.lock().expect("log ring poisoned");
        if ring.len() >= RING_CAP {
            ring.pop_front();
        }
        ring.push_back(row.clone());
        drop(ring);
        // 无人订阅时发送失败属正常，忽略。
        let _ = self.tx.send(row);
    }

    /// Subscribe to the live tail stream (lossy on slow consumers).
    pub fn subscribe_tail(&self) -> tokio::sync::broadcast::Receiver<LogRow> {
        self.tx.subscribe()
    }

    /// Recent rows from the in-memory ring (newest first), for tail views.
    pub fn recent(&self, limit: usize) -> Vec<LogRow> {
        let ring = self.ring.lock().expect("log ring poisoned");
        ring.iter().rev().take(limit).cloned().collect()
    }
}

/// One structured log event (borrowed view; stored copies are owned).
pub struct Event<'a> {
    pub level: Level,
    pub code: &'a str,
    pub message: &'a str,
    pub request_id: Option<&'a str>,
    pub backend: Option<&'a str>,
    pub model: Option<&'a str>,
    pub extra: Value,
}

#[derive(Clone, Copy)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// Convenience builder for the per-exchange trace fields.
pub fn exchange_extra(attempts: u32, status: u16, stream: bool, duration_ms: u128) -> Value {
    json!({ "attempts": attempts, "status": status, "stream": stream, "duration_ms": duration_ms })
}
