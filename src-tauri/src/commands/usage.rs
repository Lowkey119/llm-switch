//! Usage 页 IPC：日聚合趋势（查询前顺带增量 rollup）。

use crate::gateway::State;
use crate::types::UsageRecordQuery;
use serde_json::Value;
use std::sync::Arc;
use tauri::State as TauriState;

#[tauri::command]
pub fn usage_daily(st: TauriState<'_, Arc<State>>, days: Option<i64>) -> Result<Value, String> {
    let _ = st.store.rollup_usage_daily();
    let rows = st
        .store
        .usage_daily_list(days.unwrap_or(30).clamp(1, 365))
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

/// 媒体计量聚合（11d 用量页「媒体计量」卡）：区间内媒体行按 模型×后端 汇总。
#[tauri::command]
pub fn usage_media(
    st: TauriState<'_, Arc<State>>,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<Value, String> {
    let to = to.unwrap_or_else(crate::types::now_ms);
    let from = from.unwrap_or(to - 7 * 24 * 3600 * 1000);
    let rows = st.store.usage_media(from, to).map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

/// 明细分页（Usage 页下钻 / Logs 联动定位）。
#[tauri::command]
pub fn usage_records(
    st: TauriState<'_, Arc<State>>,
    input: UsageRecordQuery,
) -> Result<Value, String> {
    let rows = st
        .store
        .usage_records_query(&input)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

/// 区间成本（USD，片4：本地不计费 / 在线手工单价）。
#[tauri::command]
pub fn usage_cost(
    st: TauriState<'_, Arc<State>>,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<Value, String> {
    let to = to.unwrap_or_else(crate::types::now_ms);
    let from = from.unwrap_or(to - 24 * 3600 * 1000);
    crate::services::pricing::usage_cost(&st.store, from, to).map_err(|e| e.to_string())
}

/// M8-B 聚合（与 REST /admin/usage/stats 同构）。
#[tauri::command]
pub fn usage_stats(
    st: TauriState<'_, Arc<State>>,
    from: Option<i64>,
    to: Option<i64>,
) -> Result<Value, String> {
    let to = to.unwrap_or_else(crate::types::now_ms);
    let from = from.unwrap_or(to - 7 * 24 * 3600 * 1000);
    let (p50, p95) = st
        .store
        .usage_ttft_percentiles(from, to)
        .map_err(|e| e.to_string())?;
    let cache = st
        .store
        .usage_cache_counts(from, to)
        .map_err(|e| e.to_string())?;
    let clients = st
        .store
        .usage_by_client(from, to)
        .map_err(|e| e.to_string())?;
    let mut series = serde_json::Map::new();
    for dim in ["backend", "model", "client"] {
        let rows = st
            .store
            .usage_series(from, to, dim)
            .map_err(|e| e.to_string())?;
        series.insert(
            dim.into(),
            rows.into_iter()
                .map(|(day, key, reqs, toks)| {
                    serde_json::json!({"day": day, "key": key, "requests": reqs, "tokens": toks})
                })
                .collect(),
        );
    }
    let hit = cache.get("hit").copied().unwrap_or(0);
    let miss = cache.get("miss").copied().unwrap_or(0);
    let denom = hit + miss;
    Ok(serde_json::json!({
        "from": from, "to": to,
        "ttft_p50_ms": p50, "ttft_p95_ms": p95,
        "cache": {
            "hit": hit, "miss": miss,
            "bypass": cache.get("bypass").copied().unwrap_or(0),
            "hit_rate": if denom > 0 { hit as f64 / denom as f64 } else { 0.0 },
        },
        "by_client": clients.iter().map(|c| serde_json::json!({
            "client": c.client, "requests": c.requests,
            "input_tokens": c.input, "output_tokens": c.output,
        })).collect::<Vec<_>>(),
        "series": series,
    }))
}

/// 明细保留天数查询（默认 90，0=不清理）。
#[tauri::command]
pub fn usage_retention(st: TauriState<'_, Arc<State>>) -> Result<i64, String> {
    Ok(
        match st
            .store
            .get_setting("usage.retention_days")
            .map_err(|e| e.to_string())?
        {
            Some(v) => v.trim().parse::<i64>().unwrap_or(90),
            None => 90,
        },
    )
}

#[tauri::command]
pub fn set_usage_retention(st: TauriState<'_, Arc<State>>, days: i64) -> Result<bool, String> {
    if !(0..=3650).contains(&days) {
        return Err("保留天数需在 0（不清理）~3650".into());
    }
    st.store
        .set_setting("usage.retention_days", &days.to_string())
        .map_err(|e| e.to_string())?;
    Ok(true)
}
