//! 响应缓存 IPC（M5.5-C/E）：统计、清理、策略矩阵。

use crate::gateway::{self, State};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::State as TauriState;

/// 各后端 KV 遥测快照（Monitor 页 KV 行，M5.5-E）。
#[tauri::command]
pub fn get_backend_telemetry(st: TauriState<'_, Arc<State>>) -> Result<Vec<Value>, String> {
    Ok(gateway::telemetry::snapshot_json(&st))
}

#[tauri::command]
pub fn get_cache_stats(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let c = &st.cache;
    let s = &c.stats;
    let hits = s.hits.load(Ordering::Relaxed);
    let misses = s.misses.load(Ordering::Relaxed);
    // 命中率：0/0 时给 0，别让前端算出 NaN%。
    let hit_rate = if hits + misses > 0 {
        hits as f64 / (hits + misses) as f64
    } else {
        0.0
    };
    // 全局开关的有效值：global 策略行的 enabled 优先，没有才回落 yaml 全局。
    // 此前只读静态 config，导致前端开关写入策略行后读数不变、开关弹回。
    let enabled_globally = st
        .cache_overlays()
        .global
        .as_ref()
        .and_then(|g| g.get("enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(st.config.cache.response.enabled);
    Ok(json!({
        "hits": hits,
        "misses": misses,
        "hit_rate": hit_rate,
        "bypass": s.bypass.load(Ordering::Relaxed),
        "saved_input_tokens": s.saved_input_tokens.load(Ordering::Relaxed),
        "saved_output_tokens": s.saved_output_tokens.load(Ordering::Relaxed),
        "entries": c.len(),
        "memory_bytes": c.bytes(),
        "memory_budget_bytes": c.budget_bytes(),
        "disk_bytes": c.disk_bytes(),
        "disk_budget_bytes": c.disk_budget_bytes(),
        "enabled_globally": enabled_globally,
    }))
}

/// 清理：all=true 清全部；否则按 model / backend(producer) 命中清理。
#[tauri::command]
pub fn purge_cache(
    st: TauriState<'_, Arc<State>>,
    model: Option<String>,
    backend: Option<String>,
    all: Option<bool>,
) -> Result<i64, String> {
    let cleared = if all.unwrap_or(false) || (model.is_none() && backend.is_none()) {
        st.cache.clear()
    } else {
        let m = model.as_deref();
        let b = backend.as_deref();
        st.cache
            .purge_where(move |mo, pr| m.is_some_and(|mm| mo == mm) || b.is_some_and(|bb| pr == bb))
    };
    Ok(cleared as i64)
}

#[tauri::command]
pub fn list_cache_policies(st: TauriState<'_, Arc<State>>) -> Result<Vec<Value>, String> {
    let rows = st.store.list_cache_policies().map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(scope, policy)| json!({ "scope": scope, "policy": policy }))
        .collect())
}

/// 冷热分层预算：生效值（settings 覆盖 yaml）+ yaml 出厂值 + 在跑实例实况。
/// 预算改动下次网关重启才换到在跑实例——`restart_pending` 如实标出来。
#[tauri::command]
pub fn get_cache_tier(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let yaml = &st.config.cache.response;
    let eff = gateway::cache::tier_from_store(&st.store, yaml);
    let live_mem = st.cache.budget_bytes();
    let live_disk = st.cache.disk_budget_bytes();
    let want_disk_on = eff.persist && eff.disk_gb > 0;
    Ok(json!({
        "effective": eff,
        "yaml": { "memory_mb": yaml.memory_mb, "disk_gb": yaml.disk_gb, "persist": yaml.persist },
        "live": {
            "memory_budget_bytes": live_mem,
            "disk_budget_bytes": live_disk,
        },
        "restart_pending": (eff.memory_mb as u64) * 1024 * 1024 != live_mem as u64
            || want_disk_on != (live_disk > 0),
    }))
}

/// 保存分层预算覆盖层（空串 = 回 yaml 出厂值）。返回保存后的生效值。
#[tauri::command]
pub fn save_cache_tier(st: TauriState<'_, Arc<State>>, values: Value) -> Result<Value, String> {
    let eff = gateway::cache::tier_save(&st.store, &st.config.cache.response, &values)?;
    serde_json::to_value(&eff).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_cache_policy(
    st: TauriState<'_, Arc<State>>,
    scope: String,
    policy: Value,
) -> Result<(), String> {
    if scope != "global" && !scope.starts_with("model:") {
        return Err("scope must be `global` or `model:<name>`".into());
    }
    if !policy.is_object() {
        return Err("policy must be a JSON object".into());
    }
    st.store
        .upsert_cache_policy(&scope, &policy)
        .map_err(|e| e.to_string())?;
    gateway::reload_cache_policies(&Arc::clone(&st));
    Ok(())
}

#[tauri::command]
pub fn delete_cache_policy(st: TauriState<'_, Arc<State>>, scope: String) -> Result<bool, String> {
    let removed = st
        .store
        .delete_cache_policy(&scope)
        .map_err(|e| e.to_string())?;
    gateway::reload_cache_policies(&Arc::clone(&st));
    Ok(removed)
}
