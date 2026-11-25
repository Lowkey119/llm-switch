//! CodeBuddy：`<CODEBUDDY_CONFIG_DIR|.codebuddy>/models.json` + `settings.json`。
//!
//! 口径来自 codebuddy 2.162 自带文档与二进制（本机逆向确认）：
//! - `models.json` 是**扁平数组**，每个模型条目自带 `url` / `apiKey`（没有 pi 那样的
//!   provider 节点）；`availableModels` 是可选清单（并集，不藏用户已有模型）。
//!   定制模型的 `url` 必须以 `/chat/completions` 结尾。
//! - `settings.json` 的 `model` 用 `custom-local:<id>` 前缀标记定制模型（前缀在请求体
//!   前会被剥掉，故 `models[].id` 存裸名）；`variantModels` 把场景变体 `lite` /
//!   `reasoning` 映射到裸模型 id（值取自 `settings.variantModels`，见 env-vars 文档）。
//!
//! 接管原则（D24 + 用户拍板）：对**分配给 codebuddy 的网关模型**（默认档 +
//! lite/reasoning 变体，以及 settings 正点名要用的那批）按 id 重指向——已有同 id
//! 条目只改 `url` + `apiKey` 两个连接键，保留用户手配的 name/vendor/maxTokens/
//! supports*；没有同 id 才新增完整条目。没分给它的网关模型**不进清单**（旧版本
//! 全量并集会在 models[] 里留一堆这个端根本用不到的名字）。用户拍板（2026-11）：
//! 指本地/私网的用户条目与在线 API 的用户模型**一律不动**——直连配置归用户。
//! `availableModels` = 用户已有的那些 ∪ 分配给它的那批。

use super::{
    finish_json, json_str_eq, parse_json, read_raw, Endpoint, Evaluation, Selection, TakeoverEnv,
    PROVIDER_ID,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// CodeBuddy 的档位表：主对话默认档（`model`）之外，只有 lite / reasoning 两个场景变体。
pub const TIERS: [&str; 2] = ["lite", "reasoning"];

/// 定制模型前缀（`CUSTOM_LOCAL_MODEL_PREFIX`）：settings.model 用它，请求体前被剥掉。
const CUSTOM_LOCAL_PREFIX: &str = "custom-local:";

/// 配置目录：环境变量覆盖优先，否则 `~/.codebuddy`。
fn config_dir(env: &TakeoverEnv) -> PathBuf {
    env.dir("CODEBUDDY_CONFIG_DIR", ".codebuddy")
}

pub fn models_path(env: &TakeoverEnv) -> PathBuf {
    env.file(config_dir(env).join("models.json"))
}

pub fn settings_path(env: &TakeoverEnv) -> PathBuf {
    env.file(config_dir(env).join("settings.json"))
}

/// 网关的 OpenAI Chat Completions 地址：必须以 `/chat/completions` 结尾。
fn chat_url(ep: &Endpoint) -> String {
    let base = ep.api_root.trim_end_matches('/');
    format!("{base}/chat/completions")
}

/// 从 settings.model 里剥掉 `custom-local:` 前缀，得到裸模型 id。
fn strip_prefix(v: &str) -> &str {
    v.strip_prefix(CUSTOM_LOCAL_PREFIX).unwrap_or(v)
}

/// 现场读「codebuddy 现在真的在用哪些模型」：默认档 + 两个变体。
/// 可选清单（availableModels）不作为矩阵的 allow 口径——codebuddy 是档位制，
/// 由接管写并集，矩阵只管 档位/默认 三个位。
pub fn read_selection(env: &TakeoverEnv) -> Result<Selection, String> {
    let path = settings_path(env);
    let Some(text) = read_raw(&path)? else {
        return Ok(Selection::default());
    };
    let doc = parse_json(&text, &path)?;
    let default = doc
        .get("model")
        .and_then(Value::as_str)
        .map(strip_prefix)
        .map(str::to_string)
        .filter(|v| !v.trim().is_empty());
    let mut tiers = std::collections::BTreeMap::new();
    if let Some(vm) = doc.get("variantModels").and_then(Value::as_object) {
        for tier in TIERS {
            if let Some(v) = vm.get(tier).and_then(Value::as_str) {
                if !v.trim().is_empty() {
                    tiers.insert(tier.to_string(), v.to_string());
                }
            }
        }
    }
    Ok(Selection {
        manage_tiers: false,
        default,
        tiers,
        allow: None,
        ..Default::default()
    })
}

// ---------- models.json ----------

/// settings 里点名要用的模型（默认档 + 变体）：`models[]` 必须给它们留位，
/// 否则 settings 指过去就是一个没有连接键的模型名（一发请求就 401/404）。
fn used_by_settings(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(m) = doc
        .pointer("/model")
        .and_then(Value::as_str)
        .map(strip_prefix)
        .filter(|m| !m.trim().is_empty())
    {
        out.push(m.to_string());
    }
    if let Some(vm) = doc.pointer("/variantModels").and_then(Value::as_object) {
        for v in vm.values().filter_map(Value::as_str) {
            if !v.trim().is_empty() {
                out.push(v.to_string());
            }
        }
    }
    out
}

/// 把**分配给这个端**的网关模型按 id 写进扁平 `models[]`：已有同 id 只重指向
/// url/apiKey，没有才新增完整条目。`availableModels` 取并集（补上这批，保留用户已有）。
///
/// 没分给它的网关模型在这里一个字都不写——清单只该有它用得到的东西。
fn write_models(root: &mut Map<String, Value>, ep: &Endpoint, keep: &BTreeSet<String>) {
    let url = chat_url(ep);

    // models[]：按 id upsert（顺序沿用暴露清单，写盘字节稳定）
    let models_val = root.entry("models").or_insert_with(|| json!([])).clone();
    let arr = match models_val {
        Value::Array(a) => a,
        // 非数组属于用户文件形态异常，交由 evaluate 先拒绝，不在此兜底
        other => {
            root.insert("models".into(), other);
            return;
        }
    };
    let mut arr = arr;

    for m in ep.models.iter().filter(|m| keep.contains(*m)) {
        if let Some(existing) = arr
            .iter_mut()
            .find(|e| e.get("id").and_then(Value::as_str) == Some(m.as_str()))
        {
            // 已有同 id：只改我们拥有的两个连接键，保留用户手配的其余元数据。
            if let Some(obj) = existing.as_object_mut() {
                obj.insert("url".into(), json!(url));
                obj.insert("apiKey".into(), json!(ep.token));
            }
        } else {
            arr.push(new_entry(m, ep, &url));
        }
    }
    root.insert("models".into(), Value::Array(arr));

    // availableModels：并集（用户已有的不动）
    let avail = root
        .entry("availableModels")
        .or_insert_with(|| json!([]))
        .clone();
    if let Value::Array(existing) = avail {
        let mut ids: BTreeSet<String> = existing
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();
        // 没分配给它的网关模型连同 models[] 一起从清单里摘掉——留个没有连接键的
        // 名字在清单里，选中即 404。用户自己的 id（非网关暴露）一律保留。
        ids.retain(|id| !(ep.models.iter().any(|m| m == id) && !keep.contains(id)));
        ids.extend(keep.iter().cloned());
        root.insert(
            "availableModels".into(),
            Value::Array(ids.into_iter().map(Value::String).collect()),
        );
    } else {
        // 非数组 = 形态异常，重置为我们能管理的并集
        let ids: BTreeSet<String> = keep.iter().cloned().collect();
        root.insert(
            "availableModels".into(),
            Value::Array(ids.into_iter().map(Value::String).collect()),
        );
    }
}

/// 新增条目：全部是我们的，故写完整字段（含元数据 + 定制标记，保证变体走网关）。
fn new_entry(m: &str, ep: &Endpoint, url: &str) -> Value {
    let mut e = json!({
        "id": m,
        "name": m,
        "vendor": PROVIDER_ID,
        "url": url,
        "apiKey": ep.token,
        "supportsToolCall": true,
        // 定制标记：variantModels 取裸 id，靠 tags 认出定制模型才用条目自带的 url。
        "tags": ["custom"],
    });
    if let Some(info) = ep.meta.get(m) {
        if let Some(c) = info.ctx {
            e["maxInputTokens"] = json!(c);
        }
        if let Some(o) = info.output {
            e["maxOutputTokens"] = json!(o);
        }
        if let Some(r) = info.reasoning {
            e["supportsReasoning"] = json!(r);
        }
        if let Some(v) = info.vlm {
            e["supportsImages"] = json!(v);
        }
    }
    e
}

/// 我们拥有的连接键是否都在位：**分配给这个端的**每个网关模型 url/apiKey 指向我们，
/// 且 availableModels 覆盖了它们。
///
/// 一个都没分配 = 没有连接键要校验（清单由矩阵/面板分配驱动，先接管后分配是常态），
/// 判已到位——否则「待写入」永远空转、状态表却说没接管，矩阵跟着锁死。
fn models_current(current: &Value, ep: &Endpoint, keep: &BTreeSet<String>) -> bool {
    let url = chat_url(ep);
    let arr = current.pointer("/models").and_then(Value::as_array);
    let conn_ok = keep.iter().all(|m| {
        arr.and_then(|a| {
            a.iter()
                .find(|e| e.get("id").and_then(Value::as_str) == Some(m.as_str()))
        })
        .map(|e| json_str_eq(e, "/url", &url) && json_str_eq(e, "/apiKey", &ep.token))
        .unwrap_or(false)
    });
    let avail: BTreeSet<&str> = current
        .pointer("/availableModels")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let avail_ok = keep.iter().all(|m| avail.contains(m.as_str()));
    conn_ok && avail_ok
}

// ---------- settings.json ----------

/// 把默认档与变体写进 settings：`model` = `custom-local:<id>`，变体进 `variantModels`。
fn write_settings(root: &mut Map<String, Value>, sel: &Selection) {
    if let Some(def) = &sel.default {
        root.insert("model".into(), json!(format!("{CUSTOM_LOCAL_PREFIX}{def}")));
    } else if sel.manage_tiers {
        // 托管时清默认档：我们写的 custom-local 值要真删——否则「取消分配/
        // 连根拔」半途而废，主对话档还挂着这个模型。用户自己的 model 值不动。
        let ours = root
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(|s| s.starts_with(CUSTOM_LOCAL_PREFIX));
        if ours {
            root.remove("model");
        }
    }
    // 托管档位表：被移走的变体键要真的删掉，否则「移除了」是句空话。
    if sel.manage_tiers {
        if let Some(vm) = root.get_mut("variantModels").and_then(Value::as_object_mut) {
            for tier in TIERS {
                if !sel.tiers.contains_key(tier) {
                    vm.remove(tier);
                }
            }
        }
    }
    if !sel.tiers.is_empty() {
        let vm = root
            .entry("variantModels")
            .or_insert_with(|| json!({}))
            .clone();
        let mut obj = match vm {
            Value::Object(o) => o,
            other => other.as_object().cloned().unwrap_or_default(),
        };
        for (tier, model) in &sel.tiers {
            if TIERS.contains(&tier.as_str()) {
                obj.insert(tier.clone(), json!(model));
            }
        }
        root.insert("variantModels".into(), Value::Object(obj));
    }
}

/// settings 里我们拥有的键是否到位（默认档 + 变体；不校验残留键，那是托管时的事）。
fn settings_current(current: &Value, sel: &Selection) -> bool {
    if let Some(def) = &sel.default {
        if !json_str_eq(current, "/model", &format!("{CUSTOM_LOCAL_PREFIX}{def}")) {
            return false;
        }
    } else if sel.manage_tiers
        && current
            .pointer("/model")
            .and_then(Value::as_str)
            .is_some_and(|s| s.starts_with(CUSTOM_LOCAL_PREFIX))
    {
        // 清默认档也是改动：文件里还挂着我们的 custom-local 值 = 没到位。
        return false;
    }
    for (tier, model) in &sel.tiers {
        if !TIERS.contains(&tier.as_str()) {
            continue;
        }
        if !json_str_eq(current, &format!("/variantModels/{tier}"), model) {
            return false;
        }
    }
    if sel.manage_tiers {
        for tier in TIERS {
            if sel.tiers.contains_key(tier) {
                continue;
            }
            if current.pointer(&format!("/variantModels/{tier}")).is_some() {
                return false;
            }
        }
    }
    true
}

// ---------- 评测 ----------

pub fn evaluate(env: &TakeoverEnv, ep: &Endpoint, sel: &Selection) -> Result<Evaluation, String> {
    // --- settings.json（先算：models[] 必须覆盖 settings 点名要用的模型）---
    let spath = settings_path(env);
    let sraw = read_raw(&spath)?;
    let scurrent = match &sraw {
        Some(text) => parse_json(text, &spath)?,
        None => json!({}),
    };
    if !scurrent.is_object() {
        return Err(format!("{} 根节点不是对象，已拒绝接管", spath.display()));
    }
    let mut swanted = scurrent.clone();
    {
        let root = swanted.as_object_mut().expect("checked object");
        write_settings(root, sel);
    }
    // 分配给这个端的网关模型 = 占位（默认档 + 变体）∪ settings 点名要用的，∩ 网关暴露。
    // settings 取**改写后**的期望：托管时被移走的变体不该继续占位；反过来，
    // 没托管时 settings 里用户自己留下的变体键仍被引用，models[] 就得给它留位。
    let mut assigned: BTreeSet<String> = sel.slot_models();
    assigned.extend(used_by_settings(&swanted));
    let keep: BTreeSet<String> = assigned
        .into_iter()
        .filter(|m| ep.models.iter().any(|x| x == m))
        .collect();
    let s_managed = settings_current(&scurrent, sel);
    let s_note = if sraw.is_none() {
        "文件不存在，接管时会创建（回滚时删除）".to_string()
    } else if s_managed {
        "默认档与变体已到位".to_string()
    } else {
        "写 settings.model（custom-local:）与 variantModels".to_string()
    };
    let (seval, sedit) = finish_json(&spath, sraw, swanted, s_managed, s_note);

    // --- models.json ---
    let mpath = models_path(env);
    let mraw = read_raw(&mpath)?;
    let mcurrent = match &mraw {
        Some(text) => parse_json(text, &mpath)?,
        None => json!({}),
    };
    if !mcurrent.is_object() {
        return Err(format!("{} 根节点不是对象，已拒绝接管", mpath.display()));
    }
    let mut mwanted = mcurrent.clone();
    {
        let root = mwanted.as_object_mut().expect("checked object");
        // 形态异常（models 非数组）先拒绝，别把用户的数组改成对象。
        if let Some(v) = root.get("models") {
            if !v.is_array() {
                return Err(format!(
                    "{} 的 models 不是数组，已拒绝接管",
                    mpath.display()
                ));
            }
        }
        write_models(root, ep, &keep);
    }
    let m_managed = models_current(&mcurrent, ep, &keep);
    let m_note = if mraw.is_none() {
        "文件不存在，接管时会创建（回滚时删除）".to_string()
    } else if m_managed {
        "已指向本地网关".to_string()
    } else {
        "按 id 重指向分配给它的 models[].url/apiKey，并集补 availableModels".to_string()
    };
    let (meval, medit) = finish_json(&mpath, mraw, mwanted, m_managed, m_note);

    Ok(Evaluation {
        files: vec![meval, seval],
        edits: medit.into_iter().chain(sedit).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takeover::atomic_write_private;
    use crate::takeover::Endpoint;
    use tempfile::TempDir;

    fn ep(port: u16) -> Endpoint {
        Endpoint {
            meta: Default::default(),
            base_root: format!("http://127.0.0.1:{port}"),
            api_root: format!("http://127.0.0.1:{port}/v1"),
            token: "tk".into(),
            models: vec!["m1".into(), "m2".into()],
            codex_wire_api: "chat".into(),
            direct: false,
        }
    }

    fn env(tmp: &TempDir) -> TakeoverEnv {
        TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("backups"))
    }

    fn write_all(evaluation: &Evaluation) {
        for edit in &evaluation.edits {
            atomic_write_private(&edit.path, &edit.content).unwrap();
        }
    }

    #[test]
    fn creates_missing_files_and_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let sel = Selection {
            default: Some("m1".into()),
            tiers: [("lite".to_string(), "m2".to_string())]
                .into_iter()
                .collect(),
            manage_tiers: true,
            allow: None,
            ..Default::default()
        };
        let evaluation = evaluate(&e, &ep(8787), &sel).unwrap();
        // 两个文件 = 两条待写
        assert_eq!(evaluation.edits.len(), 2);
        assert!(!evaluation.managed());
        write_all(&evaluation);

        let models: Value =
            serde_json::from_str(&std::fs::read_to_string(models_path(&e)).unwrap()).unwrap();
        assert_eq!(
            models["models"][0]["url"],
            "http://127.0.0.1:8787/v1/chat/completions"
        );
        assert_eq!(models["models"][0]["apiKey"], "tk");
        assert_eq!(models["availableModels"], json!(["m1", "m2"]));
        // 新增条目带定制标记
        assert_eq!(models["models"][0]["tags"][0], "custom");

        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&e)).unwrap()).unwrap();
        assert_eq!(settings["model"], "custom-local:m1");
        assert_eq!(settings["variantModels"]["lite"], "m2");

        // 再评测：接管态，无差异
        let again = evaluate(&e, &ep(8787), &sel).unwrap();
        assert!(again.edits.is_empty());
        assert!(again.managed());
    }

    #[test]
    fn repoints_existing_by_id_and_preserves_user_metadata() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        // 用户手配的同 id 条目（不同 url/key + 专属元数据）
        let seed = json!({
            "models": [{
                "id": "m1",
                "name": "用户起的中文名",
                "vendor": "OpenAI",
                "apiKey": "old-key",
                "url": "http://127.0.0.1:8787/v1/chat/completions",
                "maxInputTokens": 123456,
                "supportsImages": true,
            }],
            "availableModels": ["m1"],
        });
        atomic_write_private(
            &models_path(&e),
            &serde_json::to_string_pretty(&seed).unwrap(),
        )
        .unwrap();

        let sel = Selection {
            default: Some("m1".into()),
            ..Selection::default()
        };
        let evaluation = evaluate(&e, &ep(9000), &sel).unwrap();
        write_all(&evaluation);

        let models: Value =
            serde_json::from_str(&std::fs::read_to_string(models_path(&e)).unwrap()).unwrap();
        let m1 = &models["models"][0];
        // 只改连接键
        assert_eq!(m1["url"], "http://127.0.0.1:9000/v1/chat/completions");
        assert_eq!(m1["apiKey"], "tk");
        // 手配元数据原样保留
        assert_eq!(m1["name"], "用户起的中文名");
        assert_eq!(m1["vendor"], "OpenAI");
        assert_eq!(m1["maxInputTokens"], 123456);
        assert_eq!(m1["supportsImages"], true);
        // availableModels：只并上**分配给它**的那批（没分给它的 m2 不再被捎带）
        let mut avail: Vec<&str> = models["availableModels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        avail.sort();
        assert_eq!(avail, vec!["m1"]);
    }

    #[test]
    fn keeps_user_local_models() {
        // 用户拍板（2026-11）：指本地/私网的用户模型一个字不动，
        // 接管只写网关分配的那批；availableModels 里用户已有的 id 也保留。
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let seed = json!({
            "models": [
                {"id": "lab-local", "url": "http://192.168.1.9:8000/v1/chat/completions", "apiKey": "k"},
                {"id": "online-api", "url": "https://api.openai.com/v1/chat/completions", "apiKey": "k"},
            ],
            "availableModels": ["lab-local", "online-api", "m1", "m2"],
        });
        atomic_write_private(&models_path(&e), &serde_json::to_string(&seed).unwrap()).unwrap();

        // m1 = 默认档、m2 = lite 变体，都是分给 codebuddy 的。
        let sel = Selection {
            default: Some("m1".into()),
            tiers: [("lite".to_string(), "m2".to_string())]
                .into_iter()
                .collect(),
            manage_tiers: true,
            ..Selection::default()
        };
        let evaluation = evaluate(&e, &ep(9000), &sel).unwrap();
        write_all(&evaluation);
        let after: Value =
            serde_json::from_str(&std::fs::read_to_string(models_path(&e)).unwrap()).unwrap();
        let ids: Vec<&str> = after["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.get("id").and_then(Value::as_str))
            .collect();
        assert!(ids.contains(&"lab-local"), "用户本地模型不得摘走: {ids:?}");
        assert!(ids.contains(&"online-api"), "在线 API 模型不得动: {ids:?}");
        assert!(
            ids.contains(&"m1") && ids.contains(&"m2"),
            "网关那批要在: {ids:?}"
        );
        let avail: Vec<&str> = after["availableModels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(
            avail.contains(&"lab-local"),
            "availableModels 里用户 id 要保留: {avail:?}"
        );

        let again = evaluate(&e, &ep(9000), &sel).unwrap();
        assert!(again.edits.is_empty(), "写到位之后必须幂等");
    }

    #[test]
    fn unassigned_gateway_models_stay_out_of_the_list() {
        // 一个模型都没分配给 codebuddy：网关那批一个都不该进它的 models[] /
        // availableModels（旧版本按全量并集写，这个端会挂着 16 个用不到的名字）。
        // 在线 API 的用户模型照旧保留——那是它自己的东西。
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let seed = json!({
            "models": [
                {"id": "online-api", "url": "https://api.openai.com/v1/chat/completions", "apiKey": "k"},
            ],
            "availableModels": ["online-api", "m1", "m2"],
        });
        atomic_write_private(&models_path(&e), &serde_json::to_string(&seed).unwrap()).unwrap();

        let evaluation = evaluate(&e, &ep(9000), &Selection::default()).unwrap();
        write_all(&evaluation);
        let after: Value =
            serde_json::from_str(&std::fs::read_to_string(models_path(&e)).unwrap()).unwrap();
        let ids: Vec<&str> = after["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.get("id").and_then(Value::as_str))
            .collect();
        assert_eq!(ids, vec!["online-api"], "没分配就不该有网关条目: {ids:?}");
        // availableModels 里没分给它的 m1/m2 一并摘掉
        let avail: Vec<&str> = after["availableModels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(avail, vec!["online-api"], "{avail:?}");
    }

    #[test]
    fn leaves_unserved_user_models_untouched() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let seed = json!({
            "models": [
                {"id": "user-only", "url": "http://elsewhere/v1/chat/completions", "apiKey": "u"},
                {"id": "m1", "url": "http://old/v1/chat/completions", "apiKey": "old"},
            ],
            "availableModels": ["user-only", "m1"],
        });
        atomic_write_private(
            &models_path(&e),
            &serde_json::to_string_pretty(&seed).unwrap(),
        )
        .unwrap();

        // m1 分给了 codebuddy（默认档），才轮得到重指向；user-only 没分也没暴露，
        // 是用户自己的在线条目，一个字不动。
        let sel = Selection {
            default: Some("m1".into()),
            ..Selection::default()
        };
        let evaluation = evaluate(&e, &ep(9000), &sel).unwrap();
        write_all(&evaluation);
        let models: Value =
            serde_json::from_str(&std::fs::read_to_string(models_path(&e)).unwrap()).unwrap();
        // 不暴露的 user-only：原样
        assert_eq!(
            models["models"][0]["url"],
            "http://elsewhere/v1/chat/completions"
        );
        assert_eq!(models["models"][0]["apiKey"], "u");
        // m1 被重指向
        assert_eq!(models["models"][1]["apiKey"], "tk");
    }

    #[test]
    fn read_selection_strips_prefix_and_reads_variants() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let seed = json!({
            "model": "custom-local:abc",
            "variantModels": {"lite": "fast", "reasoning": "big", "other": "x"},
        });
        atomic_write_private(
            &settings_path(&e),
            &serde_json::to_string_pretty(&seed).unwrap(),
        )
        .unwrap();
        let sel = read_selection(&e).unwrap();
        assert_eq!(sel.default.as_deref(), Some("abc"));
        assert_eq!(sel.tiers.get("lite").map(String::as_str), Some("fast"));
        assert_eq!(sel.tiers.get("reasoning").map(String::as_str), Some("big"));
        assert!(!sel.tiers.contains_key("other"));
        assert!(sel.allow.is_none());
    }

    #[test]
    fn rejects_non_object_roots() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        atomic_write_private(&models_path(&e), "[1,2,3]").unwrap();
        let err = evaluate(&e, &ep(8787), &Selection::default()).unwrap_err();
        assert!(err.contains("models"), "报错要点名文件: {err}");
    }
}
