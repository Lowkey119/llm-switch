//! 日志级别热改（M4-C）：EnvFilter 装进 reload::Layer，运行时整体替换。
//! 初始值 RUST_LOG > settings 表持久值 > config.logs.level。

use std::sync::OnceLock;

use tracing_subscriber::prelude::*;
use tracing_subscriber::reload::Handle;
use tracing_subscriber::{EnvFilter, Registry};

static HANDLE: OnceLock<Handle<EnvFilter, Registry>> = OnceLock::new();

/// 装配全局 fmt subscriber 并登记 reload 句柄（main 入口调用一次）。
pub fn init(default_directive: &str) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_directive));
    let (layer, handle) = tracing_subscriber::reload::Layer::new(filter);
    tracing_subscriber::registry()
        .with(layer)
        .with(tracing_subscriber::fmt::layer())
        .init();
    let _ = HANDLE.set(handle);
}

/// 运行时切换级别；未初始化（测试环境）时静默跳过。
pub fn set_level(directive: &str) -> Result<(), String> {
    let directive = directive.to_string();
    HANDLE
        .get()
        .ok_or_else(|| "logging not initialized".to_string())?
        .modify(move |f| {
            *f = match EnvFilter::try_new(&directive) {
                Ok(f) => f,
                Err(_) => EnvFilter::new("info"),
            };
        })
        .map_err(|e| e.to_string())
}

pub fn is_initialized() -> bool {
    HANDLE.get().is_some()
}
