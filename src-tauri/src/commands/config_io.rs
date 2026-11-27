//! JSON Tab（U-E）：Store 投影的后端配置直编——**DB 是真值**，config.yaml 只是首次启动的种子。
//!
//! 三道闸（对齐 docs/UI-ALIGNMENT.md §JSON Tab）：
//! ① 解析：非法 JSON 直接报错不落盘（前端防抖时也挡一遍）；
//! ② 语义校验：名称/地址/协议/权重逐项验，不过整体拒绝（422 语义）；
//! ③ 并发指纹：导出与写回之间 DB 被别处改过 → 拒绝覆盖，让用户重新加载。
//! 写回前把改动前的导出文本快照进 data_dir/config_snapshots（滚动 20 份）。
//! 密钥导出一律打码 `***`；导入时 `***` = 保持原值（新后端禁止，防止把星号当真密钥）。
//! 同理，`health_path` / `capabilities` / `params` 这三列（探测、引擎预设写进来的，
//! **不在导出投影里**）导入时沿用既有值——`upsert_backend` 是整行 UPDATE，
//! 缺字段不保留就等于「写回一次 JSON 就把能力表清空」（P1 修复）。给了则用给的。

use crate::commands::backend::persist_config_yaml;
use crate::gateway::State;
use crate::takeover::sha256_hex;
use crate::types::{BackendRow, BackendUpsert, Protocol};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State as TauriState;

const KEEP_SNAPSHOTS: usize = 20;

fn to_s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 当前 DB 状态的导出投影（密钥打码）。顺序稳定：按后端名排序，指纹才可复现。
fn export_items(st: &State) -> Result<Vec<Value>, String> {
    let mut rows = st.store.list_backends().map_err(to_s)?;
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(rows
        .into_iter()
        .map(|r| {
            json!({
                "name": r.name,
                "base_url": r.base_url,
                "protocol": r.protocol,
                "api_key": r.api_key.as_ref().map(|_| "***".to_string()),
                "weight": r.weight,
                "enabled": r.enabled,
                "engine": r.engine,
                "models": r.models,
                "aliases": r.aliases,
                "webui_url": r.webui_url,
                "webui_user": r.webui_user,
                "webui_password": r.webui_password.as_ref().map(|_| "***".to_string()),
            })
        })
        .collect())
}

fn fingerprint(items: &[Value]) -> Result<String, String> {
    serde_json::to_vec(items)
        .map(|b| sha256_hex(&b))
        .map_err(to_s)
}

/// 闸②：逐项语义校验，返回按序解析好的条目。任何一项不过就整体拒绝——
/// 半套配置落盘比不落盘危险得多。
/// `existing` 是当前 DB 的后端行：那三列不在导出投影里，缺失时要沿用旧值。
fn validate(
    items: &[Value],
    existing: &std::collections::HashMap<String, BackendRow>,
) -> Result<Vec<BackendUpsert>, String> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let obj = it
            .as_object()
            .ok_or_else(|| format!("第 {} 项不是对象", i + 1))?;
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("第 {} 项缺少非空 name", i + 1))?
            .to_string();
        if seen.contains(&name) {
            return Err(format!("后端名重复：{name}"));
        }
        seen.push(name.clone());
        let base_url = obj
            .get("base_url")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("{name} 缺少非空 base_url"))?
            .to_string();
        let protocol = obj
            .get("protocol")
            .and_then(|v| v.as_str())
            .map(|p| Protocol::parse(p).ok_or_else(|| format!("{name} 协议无效：{p}")))
            .transpose()?
            .unwrap_or_default();
        let weight = obj
            .get("weight")
            .and_then(|v| v.as_i64())
            .unwrap_or(1)
            .max(1);
        let enabled = obj.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
        let models = obj
            .get("models")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .map(|m| {
                        m.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| format!("{name} models 含非字符串"))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let aliases = obj
            .get("aliases")
            .cloned()
            .filter(|v| v.is_object())
            .unwrap_or(json!({}));
        let str_field = |k: &str| obj.get(k).and_then(|v| v.as_str()).map(str::to_string);
        // 探测/预设写出来的三列：给了就用给的，没给沿用旧值（新后端才是真 NULL）。
        let prev = existing.get(&name);
        out.push(BackendUpsert {
            name,
            base_url,
            protocol,
            api_key: str_field("api_key").filter(|s| !s.is_empty()),
            weight,
            enabled,
            engine: str_field("engine").filter(|s| !s.is_empty()),
            webui_url: str_field("webui_url").filter(|s| !s.trim().is_empty()),
            webui_user: str_field("webui_user").filter(|s| !s.trim().is_empty()),
            webui_password: str_field("webui_password").filter(|s| !s.is_empty()),
            health_path: str_field("health_path")
                .filter(|s| !s.trim().is_empty())
                .or_else(|| prev.and_then(|p| p.health_path.clone())),
            models,
            aliases,
            capabilities: obj
                .get("capabilities")
                .cloned()
                .or_else(|| prev.map(|p| p.capabilities.clone()))
                .unwrap_or(Value::Null),
            params: obj
                .get("params")
                .cloned()
                .or_else(|| prev.map(|p| p.params.clone()))
                .unwrap_or(Value::Null),
        });
    }
    Ok(out)
}

/// 快照改动前的导出文本，滚动修剪到 KEEP_SNAPSHOTS 份。
fn snapshot(dir: &std::path::Path, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(to_s)?;
    let name = format!("{}.json", crate::types::now_ms());
    std::fs::write(dir.join(name), text).map_err(to_s)?;
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(to_s)?
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    entries.sort();
    while entries.len() > KEEP_SNAPSHOTS {
        let _ = std::fs::remove_file(dir.join(entries.remove(0)));
    }
    Ok(())
}

#[tauri::command]
pub fn config_export(st: TauriState<Arc<State>>) -> Result<Value, String> {
    let items = export_items(&st)?;
    let text = serde_json::to_string_pretty(&items).map_err(to_s)?;
    Ok(json!({ "text": text, "sha": fingerprint(&items)? }))
}

/// 写回：闸①解析 → 闸②校验 → 闸③指纹 → 快照 → 差异落库（先增改后删）。
#[tauri::command]
pub fn config_import(
    st: TauriState<Arc<State>>,
    content: String,
    base_sha: String,
) -> Result<Value, String> {
    // 闸①：解析（前端防抖时也挡一遍，这里是最后一道）。
    let parsed: Vec<Value> =
        serde_json::from_str(&content).map_err(|e| format!("解析失败：{e}"))?;
    // 当前 DB 行：闸②要用它保留探测/预设写出来的三列，闸③的打码回填也要用它。
    let current: std::collections::HashMap<String, BackendRow> = st
        .store
        .list_backends()
        .map_err(to_s)?
        .into_iter()
        .map(|r| (r.name.clone(), r))
        .collect();
    // 闸②：语义校验。
    let wanted = validate(&parsed, &current)?;
    // 闸③：并发指纹——导出到现在 DB 被别处（UI/另一窗口）改过就拒绝覆盖。
    let cur_items = export_items(&st)?;
    if fingerprint(&cur_items)? != base_sha {
        return Err("配置已被别处改动，本次写回已拒绝：请重新加载最新配置后再改".to_string());
    }
    // 快照改动前的导出文本（滚动 20 份），万一写坏了还能从这里捞。
    let snap_dir = st.config.data_dir().join("config_snapshots");
    snapshot(
        &snap_dir,
        &serde_json::to_string_pretty(&cur_items).map_err(to_s)?,
    )?;

    // 打码回填：`***` = 保持原值；新后端拿 *** 当密钥直接拒绝。
    let mut ups = Vec::new();
    for mut u in wanted {
        let cur = current.get(&u.name);
        if u.api_key.as_deref() == Some("***") {
            u.api_key = cur.and_then(|c| c.api_key.clone());
            if u.api_key.is_none() {
                return Err(format!(
                    "新后端 {} 不能用 *** 当密钥：请填写真实密钥或删除该字段",
                    u.name
                ));
            }
        }
        if u.webui_password.as_deref() == Some("***") {
            u.webui_password = cur.and_then(|c| c.webui_password.clone());
        }
        ups.push(u);
    }

    let existing: std::collections::HashSet<String> = current.keys().cloned().collect();
    let mut added = 0usize;
    let mut updated = 0usize;
    for u in &ups {
        let is_new = !existing.contains(&u.name);
        st.store.upsert_backend(u).map_err(to_s)?;
        if is_new {
            added += 1;
        } else {
            updated += 1;
        }
    }
    let wanted_names: std::collections::HashSet<String> =
        ups.iter().map(|u| u.name.clone()).collect();
    let mut removed = 0usize;
    let mut kept: Vec<String> = Vec::new();
    for name in existing.difference(&wanted_names) {
        // halogen 是速率标定与控制台的锚点：JSON 里把它整段删掉也不算数，留着并如实上报，
        // 别让用户以为写回成功就真把锚点摘了（与 delete_backend / REST 同一道闸）。
        let row = current.get(name).expect("existing 来自 current，键必在");
        if crate::engine::is_protected_halogen(&row.name, row.engine.as_deref()) {
            kept.push(name.clone());
            continue;
        }
        // 存在性在上面 `existing` 里已核过，这里拿 Option 只为满足返回类型。
        st.store
            .delete_backend(name)
            .map_err(to_s)?
            .ok_or_else(|| format!("没有名为 `{name}` 的后端"))?;
        removed += 1;
    }
    let items = export_items(&st)?;
    // 持久化：JSON Tab 写回也改 DB，回写 config.yaml 否则重启被 seed_backends 复活。
    if let Err(e) = persist_config_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::config", "persist config.yaml failed: {e}");
    }
    Ok(json!({
        "added": added,
        "updated": updated,
        "removed": removed,
        // 被这道闸留下的受保护后端名（UI 据此提示「halogen 未删除」）。
        "kept_protected": kept,
        "sha": fingerprint(&items)?,
        "text": serde_json::to_string_pretty(&items).map_err(to_s)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str, proto: &str) -> Value {
        json!({"name": name, "base_url": "http://x", "protocol": proto})
    }

    fn none() -> std::collections::HashMap<String, BackendRow> {
        Default::default()
    }

    /// 已存在的后端行（`validate` 只读这三列，其余用默认形状即可）。
    fn prev_row(name: &str) -> BackendRow {
        let mut r = crate::types::backend_row_for_test(name);
        r.health_path = Some("/health".into());
        r.capabilities = json!({"vlm": true});
        r.params = json!({"temperature": 0.5});
        r
    }

    fn with_prev(name: &str) -> std::collections::HashMap<String, BackendRow> {
        let mut m = none();
        m.insert(name.to_string(), prev_row(name));
        m
    }

    #[test]
    fn validate_rejects_bad_input() {
        assert!(validate(&[json!("nope")], &none()).is_err());
        assert!(validate(&[item("", "openai_chat")], &none()).is_err());
        assert!(validate(&[item("a", "openai_chat"), item("a", "anthropic")], &none()).is_err());
        assert!(validate(&[item("a", "teleport")], &none()).is_err());
        assert!(validate(&[item("a", "openai_chat"), item("b", "anthropic")], &none()).is_ok());
    }

    /// P1 回归：这三列不在导出投影里，JSON 缺字段时必须沿用旧行——
    /// `upsert_backend` 是整行 UPDATE，不保留就等于写回一次 JSON 就清空能力表。
    #[test]
    fn validate_preserves_probe_written_columns_when_absent() {
        let got = validate(&[item("lab", "openai_chat")], &with_prev("lab")).unwrap();
        assert_eq!(got[0].health_path.as_deref(), Some("/health"));
        assert_eq!(got[0].capabilities, json!({"vlm": true}));
        assert_eq!(got[0].params, json!({"temperature": 0.5}));

        // 新后端没有旧行可留：保持真空（而不是编一个空对象出来）。
        let got = validate(&[item("fresh", "openai_chat")], &with_prev("lab")).unwrap();
        assert_eq!(got[0].health_path, None);
        assert_eq!(got[0].capabilities, Value::Null);
        assert_eq!(got[0].params, Value::Null);
    }

    /// JSON 里显式给了就用给的（包括显式 null = 清空）。
    #[test]
    fn validate_honours_explicit_probe_columns() {
        let item = json!({
            "name": "lab", "base_url": "http://x", "protocol": "openai_chat",
            "health_path": "/livez",
            "capabilities": {"vlm": false},
            "params": null
        });
        let got = validate(&[item], &with_prev("lab")).unwrap();
        assert_eq!(got[0].health_path.as_deref(), Some("/livez"));
        assert_eq!(got[0].capabilities, json!({"vlm": false}));
        assert_eq!(got[0].params, Value::Null);
    }

    #[test]
    fn fingerprint_is_order_sensitive_and_stable() {
        let a = vec![item("a", "openai_chat"), item("b", "anthropic")];
        let b = vec![item("b", "anthropic"), item("a", "openai_chat")];
        assert_eq!(fingerprint(&a).unwrap(), fingerprint(&a.clone()).unwrap());
        assert_ne!(fingerprint(&a).unwrap(), fingerprint(&b).unwrap());
    }
}
