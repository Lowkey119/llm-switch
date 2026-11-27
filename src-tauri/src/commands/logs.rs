//! 日志页 IPC：运行日志查询与清理（访问日志走 usage 接口）。

use crate::gateway::State;
use crate::types::LogQuery;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State as TauriState;

#[derive(Debug, serde::Deserialize)]
pub struct LogQueryInput {
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub only_errors: bool,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[tauri::command]
pub fn logs_query(st: TauriState<Arc<State>>, input: LogQueryInput) -> Result<Value, String> {
    let rows = st
        .store
        .query_logs(&LogQuery {
            from: None,
            to: None,
            level: input.level,
            backend: input.backend,
            model: input.model,
            request_id: input.request_id,
            text: input.text,
            only_errors: input.only_errors,
            limit: input.limit.unwrap_or(200).min(2000),
            offset: input.offset.unwrap_or(0),
        })
        .map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn logs_purge(st: TauriState<Arc<State>>, before_ts: i64) -> Result<usize, String> {
    st.store
        .purge_logs_before(before_ts)
        .map_err(|e| e.to_string())
}

/// 日志页两个过滤下拉的选项：日志真值里出现过的后端 / 模型名（DISTINCT）。
#[tauri::command]
pub fn logs_facets(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let (backends, models) = st.store.log_facets().map_err(|e| e.to_string())?;
    serde_json::to_value(json!({ "backends": backends, "models": models }))
        .map_err(|e| e.to_string())
}

/// 导出 NDJSON（一行一条 JSON，UTF-8）；沿用查询过滤器。
#[tauri::command]
pub fn logs_export(st: TauriState<'_, Arc<State>>, input: LogQueryInput) -> Result<String, String> {
    let rows = st
        .store
        .query_logs(&LogQuery {
            from: None,
            to: None,
            level: input.level,
            backend: input.backend,
            model: input.model,
            request_id: input.request_id,
            text: input.text,
            only_errors: input.only_errors,
            limit: input.limit.unwrap_or(10_000).min(100_000),
            offset: input.offset.unwrap_or(0),
        })
        .map_err(|e| e.to_string())?;
    let mut out = String::new();
    for r in &rows {
        out.push_str(&serde_json::to_string(r).map_err(|e| e.to_string())?);
        out.push('\n');
    }
    Ok(out)
}

/// 网关日志保留策略：生效值 = settings 覆盖，未覆盖回落配置文件。
#[tauri::command]
pub fn logs_retention(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    Ok(json!({
        "retention_days": crate::gateway::maintenance::log_retention_days(&st),
        "max_mb": crate::gateway::maintenance::log_max_mb(&st),
    }))
}

/// 保存保留策略（settings 覆盖持久化）：维护 tick 每轮现读，保存即下一轮生效。
#[tauri::command]
pub fn set_logs_retention(
    st: TauriState<'_, Arc<State>>,
    retention_days: i64,
    max_mb: i64,
) -> Result<bool, String> {
    if !(1..=3650).contains(&retention_days) {
        return Err("保留天数需在 1~3650 之间".into());
    }
    if !(1..=1024).contains(&max_mb) {
        return Err("体积预算需在 1~1024 MB 之间".into());
    }
    st.store
        .set_setting("logs.retention_days", &retention_days.to_string())
        .map_err(|e| e.to_string())?;
    st.store
        .set_setting("logs.max_mb", &max_mb.to_string())
        .map_err(|e| e.to_string())?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "LG301 logs.retention_changed",
        message: &format!("log retention set to {retention_days}d / {max_mb}MB"),
        request_id: None,
        backend: None,
        model: None,
        extra: json!({ "retention_days": retention_days, "max_mb": max_mb }),
    });
    Ok(true)
}

#[tauri::command]
pub fn get_log_level(st: TauriState<'_, Arc<State>>) -> Result<String, String> {
    Ok(st
        .store
        .get_setting("logs.level")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| st.config.logs.level.clone()))
}

/// 热改级别（EnvFilter 指令串，如 "info" / "llm_switch=debug"），并持久化。
#[tauri::command]
pub fn set_log_level(st: TauriState<'_, Arc<State>>, level: String) -> Result<(), String> {
    crate::gateway::loglevel::set_level(&level)?;
    st.store
        .set_setting("logs.level", &level)
        .map_err(|e| e.to_string())?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "LG300 log.level_changed",
        message: &format!("log level set to {level}"),
        request_id: None,
        backend: None,
        model: None,
        extra: serde_json::json!({ "level": level }),
    });
    Ok(())
}
