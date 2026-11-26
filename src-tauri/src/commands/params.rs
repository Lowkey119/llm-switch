//! 参数中心 IPC：param_overrides 的查/改/删，写后立即热重载。

use crate::gateway::{self, State};
use crate::types::ParamOverride;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::State as TauriState;

#[tauri::command]
pub fn list_param_overrides(st: TauriState<Arc<State>>) -> Result<Value, String> {
    let rows = st.store.list_param_overrides().map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
pub struct ParamInput {
    pub scope: String,
    #[serde(default)]
    pub ref_id: String,
    pub key: String,
    pub value: Value,
    #[serde(default = "default_policy")]
    pub policy: String,
}

fn default_policy() -> String {
    "fill".into()
}

fn validate(i: &ParamInput) -> Result<Value, String> {
    if !matches!(i.scope.as_str(), "global" | "backend" | "model") {
        return Err("scope must be global|backend|model".into());
    }
    if !matches!(i.policy.as_str(), "fill" | "override" | "clamp") {
        return Err("policy must be fill|override|clamp".into());
    }
    // 值形状校验（与注入口同口径）：空值拒收、数值键纠偏——坏值一律进不了库。
    gateway::params::normalize_value(&i.key, &i.policy, &i.value)
}

/// 保存一条覆盖，并把模型层的这条**下发到后端本体**（各家原生配置方法）。
/// 返回值是下发战报（`param_push::Outcome`）；非模型层返回 `null`。
#[tauri::command]
pub async fn upsert_param_override(
    app: tauri::AppHandle,
    st: TauriState<'_, Arc<State>>,
    input: ParamInput,
) -> Result<Value, String> {
    let value = validate(&input)?;
    let row = ParamOverride {
        id: 0,
        scope: input.scope,
        ref_id: input.ref_id,
        key: input.key,
        value,
        policy: input.policy,
    };
    st.store
        .upsert_param_override(&row)
        .map_err(|e| e.to_string())?;
    gateway::on_params_changed(&Arc::clone(&st));
    use tauri::Emitter;
    let _ = app.emit("params://changed", ());
    let report = crate::services::param_push::push_override(
        &st,
        &row.scope,
        &row.ref_id,
        &row.key,
        Some(row.value.clone()),
    )
    .await
    .unwrap_or(Value::Null);
    Ok(report)
}

/// 删除一条覆盖 = 「恢复后端默认」，模型层的这条要同步**清回后端自己的默认**，
/// 否则后端里还留着上次下发的值，UI 说「已恢复默认」而实际没有。
#[tauri::command]
pub async fn delete_param_override(
    app: tauri::AppHandle,
    st: TauriState<'_, Arc<State>>,
    id: i64,
) -> Result<Value, String> {
    let gone = st
        .store
        .list_param_overrides()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|r| r.id == id);
    st.store
        .delete_param_override(id)
        .map_err(|e| e.to_string())?;
    gateway::on_params_changed(&Arc::clone(&st));
    use tauri::Emitter;
    let _ = app.emit("params://changed", ());
    let report = match gone {
        Some(r) => {
            crate::services::param_push::push_override(&st, &r.scope, &r.ref_id, &r.key, None)
                .await
                .unwrap_or(Value::Null)
        }
        None => Value::Null,
    };
    Ok(report)
}

/// 参数目录（UI 分组渲染用）：键/组/默认策略。
#[tauri::command]
pub fn get_param_catalog() -> Value {
    serde_json::to_value(
        crate::gateway::catalog::ENTRIES
            .iter()
            .map(|en| {
                serde_json::json!({
                    "key": en.key,
                    "group": en.group,
                    "default_policy": en.default_policy,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or(Value::Null)
}

/// 某后端的生效能力（UI 灰显用）：预设 ⊕ 声明。
#[tauri::command]
pub fn get_backend_capabilities(
    st: TauriState<'_, Arc<State>>,
    backend: String,
) -> Result<Value, String> {
    let row = st
        .store
        .list_backends()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.name == backend)
        .ok_or_else(|| format!("no backend named `{backend}`"))?;
    let caps = crate::gateway::capability::effective(row.engine.as_deref(), &row.capabilities);
    let caps_json = caps.to_json();
    let mut keys: Vec<&str> = caps_json
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    keys.sort_unstable();
    Ok(serde_json::json!({ "engine": row.engine, "keys": keys }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input(key: &str, value: Value, policy: &str) -> ParamInput {
        ParamInput {
            scope: "model".into(),
            ref_id: "b:m".into(),
            key: key.into(),
            value,
            policy: policy.into(),
        }
    }

    #[test]
    fn empty_value_rejected_at_save() {
        // top_p:"" 事故复现：空值进不了库（清空 = 删除覆盖，走 delete 路径）。
        let e = validate(&input("top_p", json!(""), "fill")).unwrap_err();
        assert!(e.contains("值为空"), "{e}");
        assert!(validate(&input("top_p", json!(null), "fill")).is_err());
        assert!(validate(&input("temperature", json!("abc"), "fill")).is_err());
        assert!(validate(&input("max_tokens", json!(""), "clamp")).is_err());
    }

    #[test]
    fn numeric_string_normalized_before_store() {
        // 数字串存前纠偏成数字，杜绝"看起来是数字实则字符串"的坏行。
        assert_eq!(
            validate(&input("top_p", json!("0.9"), "fill")).unwrap(),
            json!(0.9)
        );
        assert_eq!(
            validate(&input("seed", json!("42"), "fill")).unwrap(),
            json!(42)
        );
        assert!(validate(&input("stop", json!("END"), "fill")).is_ok());
        assert!(validate(&input(
            "response_format",
            json!({"type":"json_object"}),
            "fill"
        ))
        .is_ok());
    }
}
