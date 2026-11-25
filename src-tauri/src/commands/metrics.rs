//! Monitor 速率快照 IPC（事件之外的兜底轮询通道）。

use crate::gateway::State;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State as TauriState;

/// 浮条口径（M12-C1）：在飞交换的相位 / prefill 实时速率 / dec 速率。
#[tauri::command]
pub fn get_rates(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    Ok(crate::gateway::rates::snapshot(&st))
}

/// 按模型的速率（M12-B）：监控页「按模型」面板用。
#[tauri::command]
pub fn get_model_rates(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let rows = st.metrics.snapshot_models(&st.monitor.count_by_model());
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

/// 按 (后端, 模型) 的速率（切片 17）：监控页「每后端一卡、卡内模型行」吃这份。
///
/// BUG-2 口径统一：模型行的吞吐与下方活跃流表逐条速率**必须对得上**。
/// 历史桶值（Σtoken ÷ Σ跨度）把已完成的请求也摊进来，与「此刻在飞的这几条」
/// 天然不等——用户看到的就是「曲线 ≠ 下方之和」。有在飞流时，模型吞吐改为
/// 其在飞流瞬时 decode 之和（与流表逐行同源）；没有在飞流才退回历史桶值。
#[tauri::command]
pub fn get_backend_model_rates(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let rows = st
        .metrics
        .snapshot_backend_models(&st.monitor.count_by_backend_model());
    let mut live: std::collections::HashMap<(String, String), f64> = Default::default();
    for r in crate::gateway::rates::inflight_json(&st) {
        if r["phase"].as_str() == Some("generating") {
            *live
                .entry((
                    r["backend"].as_str().unwrap_or_default().to_string(),
                    r["model"].as_str().unwrap_or_default().to_string(),
                ))
                .or_default() += r["dec_tps"].as_f64().unwrap_or(0.0);
        }
    }
    let mut out = serde_json::to_value(rows).map_err(|e| e.to_string())?;
    if let Some(arr) = out.as_array_mut() {
        for row in arr.iter_mut() {
            let key = (
                row["backend"].as_str().unwrap_or_default().to_string(),
                row["model"].as_str().unwrap_or_default().to_string(),
            );
            if let Some(sum) = live.get(&key) {
                row["tokens_per_s"] = json!((sum * 10.0).round() / 10.0);
            }
        }
    }
    Ok(out)
}

/// 引擎原生速率（切片 11）：halogen 指标接口差分，TTL 合流。
/// 监控页与浮窗都吃这一份；halogen 的 SSH 往返每 TTL 至多一次。
#[tauri::command]
pub async fn get_engine_rates(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let ch = crate::services::halctl::channel_of("halogen")?;
    Ok(st.engine_rates.snapshot(&st.config.halctl, ch).await)
}

/// 「收到 API 流量自动弹出浮动监视器」开关（切片 11，settings `monitor.auto_float`）。
#[tauri::command]
pub fn auto_float_get(st: TauriState<'_, Arc<State>>) -> Result<bool, String> {
    Ok(st
        .store
        .get_setting("monitor.auto_float")
        .map_err(|e| e.to_string())?
        .as_deref()
        == Some("true"))
}

/// 同上，保存（即时生效，gui 事件泵每秒读一次）。
#[tauri::command]
pub fn auto_float_set(st: TauriState<'_, Arc<State>>, on: bool) -> Result<bool, String> {
    st.store
        .set_setting("monitor.auto_float", if on { "true" } else { "false" })
        .map_err(|e| e.to_string())?;
    Ok(on)
}

#[tauri::command]
pub fn get_metrics(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let snap = st.metrics.snapshot(st.monitor.snapshot().len());
    let mut v = serde_json::to_value(snap).map_err(|e| e.to_string())?;
    // 缓存命中指标同步进 IPC（M5.5-E）。
    if let Some(obj) = v.as_object_mut() {
        let s = &st.cache.stats;
        use std::sync::atomic::Ordering;
        let hits = s.hits.load(Ordering::Relaxed);
        let misses = s.misses.load(Ordering::Relaxed);
        obj.insert(
            "cache".into(),
            json!({
                "hits": hits,
                "misses": misses,
                "hit_rate": if hits + misses > 0 { hits as f64 / (hits + misses) as f64 } else { 0.0 },
                "saved_input_tokens": s.saved_input_tokens.load(Ordering::Relaxed),
                "saved_output_tokens": s.saved_output_tokens.load(Ordering::Relaxed),
            }),
        );
    }
    Ok(v)
}
