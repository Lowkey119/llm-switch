//! Settings 页路由段 IPC：热更路由策略与熔断开关（持久化到 settings 表）。

use crate::config::{RoutingCfg, RoutingPolicy};
use crate::gateway::State;
use serde_json::Value;
use std::sync::Arc;
use tauri::State as TauriState;

const OVERLAY_KEY: &str = "routing.overlay";

#[tauri::command]
pub fn get_routing(st: TauriState<Arc<State>>) -> Result<Value, String> {
    let rc = st.routing.read().expect("routing lock poisoned");
    serde_json::to_value(&*rc).map_err(|e| e.to_string())
}

/// Accepts a full RoutingCfg-shaped JSON (missing fields fall back to
/// defaults). Applies hot, persists, and pushes breaker config to live breakers.
#[tauri::command]
pub async fn set_routing(st: TauriState<'_, Arc<State>>, routing: Value) -> Result<(), String> {
    let incoming: RoutingCfg =
        serde_json::from_value(routing).map_err(|e| format!("invalid routing: {e}"))?;
    // 白名单校验（policy 由枚举 serde 保证）。
    let _ = RoutingPolicy::default();
    *st.routing.write().expect("routing lock poisoned") = incoming.clone();
    st.store
        .set_setting(
            OVERLAY_KEY,
            &serde_json::to_string(&incoming).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    // 已存在的熔断器实例热更配置（不重置状态）。
    let cfg =
        crate::gateway::circuit_breaker::CircuitBreakerConfig::from(&incoming.circuit_breaker);
    let breakers: Vec<_> = st
        .breakers
        .read()
        .expect("breakers lock poisoned")
        .values()
        .cloned()
        .collect();
    for b in breakers {
        b.update_config(cfg.clone()).await;
    }
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "RT300 routing.changed",
        message: "routing configuration updated",
        request_id: None,
        backend: None,
        model: None,
        extra: serde_json::json!({ "policy": incoming.policy.to_string() }),
    });
    Ok(())
}
