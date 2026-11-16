//! Claude Code：`<CLAUDE_CONFIG_DIR|.claude>/settings.json` 的 `env` 块。
//!
//! `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`（Claude Code 实际读的就是这两个，
//! `~/.claude.json` 管的是 mcpServers/onboarding，不在接管面上）。
//! 其余键（model、hooks、permissions、别人塞的 env）一律原样保留。

use super::{
    finish_json, json_str_eq, parse_json, read_raw, Edit, Endpoint, Evaluation, FileEval,
    Selection, TakeoverEnv,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 只写默认档会让子任务、后台摘要去打没暴露的模型名。
pub const TIERS: [&str; 5] = ["haiku", "sonnet", "opus", "fable", "subagent"];

/// 档位 → 环境变量名。
pub fn tier_env_key(tier: &str) -> String {
    if tier.eq_ignore_ascii_case("subagent") {
        "CLAUDE_CODE_SUBAGENT_MODEL".to_string()
    } else {
        format!("ANTHROPIC_DEFAULT_{}_MODEL", tier.to_ascii_uppercase())
    }
}

pub fn settings_path(env: &TakeoverEnv) -> PathBuf {
    // 路径自定义收口（代理 tab 片三）：读、写、编辑器都经 env.file 同侧生效。
    env.file(
        env.dir("CLAUDE_CONFIG_DIR", ".claude")
            .join("settings.json"),
    )
}

/// 把模型选择写进 settings：默认档同时写 `env.ANTHROPIC_MODEL` 与顶层 `model`，
/// 档位写 `ANTHROPIC_DEFAULT_<TIER>_MODEL`（外加 `_NAME`，否则 /status 里露内部名）。
fn write_selection(env_obj: &mut Map<String, Value>, sel: &Selection) {
    if let Some(model) = &sel.default {
        env_obj.insert("ANTHROPIC_MODEL".into(), json!(model));
    } else if sel.manage_tiers {
        // 托管时清默认档：env 里的默认档键是我们写的，要真删——否则
        // 「取消分配/连根拔」半途而废，读回来默认档还在（点击没反应的实报根因之一）。
        env_obj.remove("ANTHROPIC_MODEL");
    }
    if sel.manage_tiers {
        // 我们在托管这张表：被移走的槽位要真的删掉，否则「移除了」是句空话
        for tier in TIERS {
            let key = tier_env_key(tier);
            if !sel.tiers.contains_key(tier) {
                env_obj.remove(&key);
                env_obj.remove(&format!("{key}_NAME"));
            }
        }
    }
    for (tier, model) in &sel.tiers {
        let key = tier_env_key(tier);
        if key != "CLAUDE_CODE_SUBAGENT_MODEL" {
            env_obj.insert(format!("{key}_NAME"), json!(model));
        }
        env_obj.insert(key, json!(model));
    }
}

/// 期望的模型键是否都已到位。
fn selection_is_current(current: &Value, sel: &Selection, ep: &Endpoint) -> bool {
    if let Some(model) = &sel.default {
        if !json_str_eq(current, "/env/ANTHROPIC_MODEL", model)
            || !json_str_eq(current, "/model", model)
        {
            return false;
        }
    }
    if !sel.tiers.iter().all(|(tier, model)| {
        let pointer = format!("/env/{}", tier_env_key(tier));
        json_str_eq(current, &pointer, model)
    }) {
        return false;
    }
    if sel.manage_tiers {
        // 清默认档也是改动：env 默认档键一律算残留；顶层 model 只在指向
        // 网关暴露的模型时才算我们的残留（用户自己的 "opus" 等设置不动）。
        if sel.default.is_none()
            && (current.pointer("/env/ANTHROPIC_MODEL").is_some()
                || current
                    .pointer("/model")
                    .and_then(Value::as_str)
                    .is_some_and(|m| ep.models.iter().any(|e| e == m)))
        {
            return false;
        }
        // 托管状态下，多余的档位键也算没到位（等着被剪掉）
        for tier in TIERS {
            if sel.tiers.contains_key(tier) {
                continue;
            }
            if current
                .pointer(&format!("/env/{}", tier_env_key(tier)))
                .is_some()
            {
                return false;
            }
        }
    }
    true
}

/// 现场读回「这台 claude 现在真的用哪些模型」（真相在文件里，不在库里）。
pub fn read_selection(env: &TakeoverEnv) -> Result<Selection, String> {
    let path = settings_path(env);
    let Some(text) = read_raw(&path)? else {
        return Ok(Selection::default());
    };
    let doc = parse_json(&text, &path)?;
    let env_obj = doc.get("env").and_then(Value::as_object);
    let default = env_obj
        .and_then(|e| e.get("ANTHROPIC_MODEL"))
        .and_then(Value::as_str)
        .or_else(|| doc.get("model").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|v| !v.trim().is_empty());
    let mut tiers = BTreeMap::new();
    for tier in TIERS {
        if let Some(v) = env_obj
            .and_then(|e| e.get(&tier_env_key(tier)))
            .and_then(Value::as_str)
        {
            if !v.trim().is_empty() {
                tiers.insert(tier.to_string(), v.to_string());
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

pub fn evaluate(env: &TakeoverEnv, ep: &Endpoint, sel: &Selection) -> Result<Evaluation, String> {
    let path = settings_path(env);
    let raw = read_raw(&path)?;
    let current = match &raw {
        Some(text) => parse_json(text, &path)?,
        None => json!({}),
    };
    if !current.is_object() {
        return Err(format!("{} 根节点不是对象，已拒绝接管", path.display()));
    }

    let mut wanted = current.clone();
    let env_obj = wanted
        .as_object_mut()
        .expect("checked object")
        .entry("env")
        .or_insert_with(|| json!({}));
    if !env_obj.is_object() {
        return Err(format!("{} 的 env 不是对象，已拒绝接管", path.display()));
    }
    let env_obj = env_obj.as_object_mut().expect("checked object");
    env_obj.insert("ANTHROPIC_BASE_URL".into(), json!(ep.base_root.as_str()));
    env_obj.insert("ANTHROPIC_AUTH_TOKEN".into(), json!(ep.token.as_str()));

    {
        let root = wanted.as_object_mut().expect("checked object");
        // 顶层 model 与 env.ANTHROPIC_MODEL 一起写：前者是 claude 的起手档，
        // 后者是它内部各处（摘要、子任务）读的兜底。
        if let Some(model) = &sel.default {
            root.insert("model".to_string(), json!(model));
        } else if sel.manage_tiers {
            // 托管时清默认档：顶层 model 若指向网关暴露的模型，那是我们写的，
            // 要真删（否则读回来默认档还在）；用户自己的设置（"opus" 档位名等）不动。
            let ours = root
                .get("model")
                .and_then(Value::as_str)
                .is_some_and(|m| ep.models.iter().any(|e| e == m));
            if ours {
                root.remove("model");
            }
        }
        let env_obj = root
            .get_mut("env")
            .and_then(|v| v.as_object_mut())
            .expect("checked above");
        write_selection(env_obj, sel);
    }

    let managed = json_str_eq(&current, "/env/ANTHROPIC_BASE_URL", &ep.base_root)
        && json_str_eq(&current, "/env/ANTHROPIC_AUTH_TOKEN", &ep.token)
        && selection_is_current(&current, sel, ep);
    let note = if raw.is_none() {
        "文件不存在，接管时会创建（回滚时删除）".to_string()
    } else if managed {
        "已指向本地网关".to_string()
    } else {
        "写入 env.ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN".to_string()
    };

    let (eval, edit) = finish_json(&path, raw, wanted, managed, note);
    Ok(pack(eval, edit))
}

fn pack(eval: FileEval, edit: Option<Edit>) -> Evaluation {
    Evaluation {
        files: vec![eval],
        edits: edit.into_iter().collect(),
    }
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
            models: vec!["m1".into()],
            codex_wire_api: "chat".into(),
        }
    }

    fn env(tmp: &TempDir) -> TakeoverEnv {
        TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("backups"))
    }

    #[test]
    fn creates_missing_file_and_keeps_other_keys() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let evaluation = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        assert_eq!(evaluation.edits.len(), 1);
        assert!(!evaluation.managed());

        atomic_write_private(&evaluation.edits[0].path, &evaluation.edits[0].content).unwrap();
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&e)).unwrap()).unwrap();
        assert_eq!(
            written["env"]["ANTHROPIC_BASE_URL"],
            "http://127.0.0.1:8787"
        );

        // 再评测一次：接管态，无差异。
        let again = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        assert!(again.edits.is_empty());
        assert!(again.managed());
    }

    #[test]
    fn preserves_foreign_keys_and_comments_free_json() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            settings_path(&e),
            "{\n  \"model\": \"opus\",\n  \"env\": {\"ANTHROPIC_API_KEY\": \"mine\"},\n  \"includeCoAuthoredBy\": false\n}\n",
        )
        .unwrap();

        let evaluation = evaluate(&e, &ep(9000), &Selection::default()).unwrap();
        let text = &evaluation.edits[0].content;
        assert!(text.contains("\"model\": \"opus\""), "{text}");
        assert!(text.contains("includeCoAuthoredBy"), "{text}");
        assert!(text.contains("ANTHROPIC_API_KEY"), "{text}");
        assert!(text.contains("ANTHROPIC_BASE_URL"), "{text}");
    }

    #[test]
    fn refuses_broken_json_instead_of_clobbering() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(settings_path(&e), "{ not json").unwrap();
        let err = evaluate(&e, &ep(8787), &Selection::default()).unwrap_err();
        assert!(err.contains("不是合法 JSON"), "{err}");
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;
    use crate::takeover::{atomic_write_private, Selection};
    use tempfile::TempDir;

    fn ep() -> Endpoint {
        Endpoint {
            meta: Default::default(),
            base_root: "http://127.0.0.1:8787".into(),
            api_root: "http://127.0.0.1:8787/v1".into(),
            token: "tk".into(),
            models: vec!["m1".into()],
            codex_wire_api: "chat".into(),
        }
    }

    #[test]
    fn writes_default_and_tier_table() {
        let tmp = TempDir::new().unwrap();
        let e = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"));
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        let sel = Selection {
            default: Some("qwen-mini".into()),
            tiers: [("haiku", "tiny"), ("opus", "big"), ("subagent", "mid")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        };
        let ev = evaluate(&e, &ep(), &sel).unwrap();
        atomic_write_private(&ev.edits[0].path, &ev.edits[0].content).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&e)).unwrap()).unwrap();
        assert_eq!(written["model"], "qwen-mini");
        assert_eq!(written["env"]["ANTHROPIC_MODEL"], "qwen-mini");
        assert_eq!(written["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "tiny");
        assert_eq!(written["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME"], "tiny");
        assert_eq!(written["env"]["CLAUDE_CODE_SUBAGENT_MODEL"], "mid");
        // 档位不参与 _NAME 的只有 subagent
        assert!(written["env"]
            .get("CLAUDE_CODE_SUBAGENT_MODEL_NAME")
            .is_none());

        // 再评测：接管态、无差异；读回的现状与写下去的一致
        assert!(evaluate(&e, &ep(), &sel).unwrap().edits.is_empty());
        assert_eq!(read_selection(&e).unwrap(), sel);
    }

    #[test]
    fn stale_model_makes_the_client_pending() {
        let tmp = TempDir::new().unwrap();
        let e = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"));
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        let sel = Selection {
            default: Some("a".into()),
            tiers: Default::default(),
            ..Default::default()
        };
        let ev = evaluate(&e, &ep(), &sel).unwrap();
        atomic_write_private(&ev.edits[0].path, &ev.edits[0].content).unwrap();
        assert!(evaluate(&e, &ep(), &sel).unwrap().managed());

        // 换成 b：连接键没动，但模型漂了 → 必须重新变 pending
        let other = Selection {
            default: Some("b".into()),
            tiers: Default::default(),
            ..Default::default()
        };
        let again = evaluate(&e, &ep(), &other).unwrap();
        assert!(!again.managed());
        assert_eq!(again.edits.len(), 1);
    }
}
