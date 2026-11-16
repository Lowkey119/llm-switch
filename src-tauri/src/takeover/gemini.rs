//! Gemini CLI：`~/.gemini/.env` + `~/.gemini/settings.json`。
//!
//! - `.env` 写 `GOOGLE_GEMINI_BASE_URL` / `GEMINI_API_KEY`（原位改写，别人的行不动）；
//! - `settings.json` 只写 `security.auth.selectedType = "gemini-api-key"`
//!   ——不写这一条，CLI 会继续走 OAuth 而忽略我们的地址。
//!
//! 顶层 `model.name` 是接管面的一部分（M12-E）：既然地址都归网关管了，
//! 默认模型也该由网关写，否则切一半留一半最容易出问题。

use super::{
    dotenv_has, dotenv_patch, finish_json, json_str_eq, parse_json, read_raw, Edit, Endpoint,
    Evaluation, FileEval, Format, Selection, TakeoverEnv,
};
use serde_json::{json, Value};
use std::path::PathBuf;

/// 走 API Key 认证（第三方/代理）的取值。
pub const SELECTED_TYPE_API_KEY: &str = "gemini-api-key";

pub fn dot_env_path(env: &TakeoverEnv) -> PathBuf {
    env.file(env.home.join(".gemini").join(".env"))
}

pub fn settings_path(env: &TakeoverEnv) -> PathBuf {
    env.file(env.home.join(".gemini").join("settings.json"))
}

/// 读回 gemini 现在的默认模型（settings.json 的 `model.name`）。
pub fn read_selection(env: &TakeoverEnv) -> Result<Selection, String> {
    let path = settings_path(env);
    let Some(text) = read_raw(&path)? else {
        return Ok(Selection::default());
    };
    let doc = parse_json(&text, &path)?;
    Ok(Selection {
        manage_tiers: false,
        default: doc
            .pointer("/model/name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|v| !v.trim().is_empty()),
        tiers: Default::default(),
        allow: None,
        ..Default::default()
    })
}

pub fn evaluate(env: &TakeoverEnv, ep: &Endpoint, sel: &Selection) -> Result<Evaluation, String> {
    let mut files = Vec::new();
    let mut edits = Vec::new();

    // --- .env ---
    let env_path = dot_env_path(env);
    let env_raw = read_raw(&env_path)?;
    let env_wanted: Vec<(&str, &str)> = vec![
        ("GOOGLE_GEMINI_BASE_URL", ep.base_root.as_str()),
        ("GEMINI_API_KEY", ep.token.as_str()),
    ];
    let env_source = env_raw.clone().unwrap_or_default();
    let env_managed = env_wanted
        .iter()
        .all(|(k, v)| dotenv_has(&env_source, k, v));
    let env_rendered = dotenv_patch(&env_source, &env_wanted);
    files.push(FileEval {
        path: env_path.display().to_string(),
        format: Format::Text.as_str(),
        exists: env_raw.is_some(),
        managed: env_managed,
        note: if env_raw.is_none() {
            "文件不存在，接管时会创建（回滚时删除）".to_string()
        } else {
            "写入 GOOGLE_GEMINI_BASE_URL / GEMINI_API_KEY".to_string()
        },
        ..Default::default()
    });
    if env_raw.as_deref() != Some(env_rendered.as_str()) {
        edits.push(Edit {
            path: env_path.clone(),
            content: env_rendered,
        });
    }

    // --- settings.json ---
    let settings_path = settings_path(env);
    let raw = read_raw(&settings_path)?;
    let current = match &raw {
        Some(text) => parse_json(text, &settings_path)?,
        None => json!({}),
    };
    if !current.is_object() {
        return Err(format!(
            "{} 根节点不是对象，已拒绝接管",
            settings_path.display()
        ));
    }
    let model_is_current = match &sel.default {
        Some(want) => json_str_eq(&current, "/model/name", want),
        None => true,
    };
    let managed = json_str_eq(
        &current,
        "/security/auth/selectedType",
        SELECTED_TYPE_API_KEY,
    ) && model_is_current;
    let mut wanted: Value = current.clone();
    {
        let root = wanted.as_object_mut().expect("checked object");
        let security = root.entry("security").or_insert_with(|| json!({}));
        if !security.is_object() {
            return Err(format!(
                "{} 的 security 不是对象，已拒绝接管",
                settings_path.display()
            ));
        }
        let auth = security
            .as_object_mut()
            .expect("checked object")
            .entry("auth")
            .or_insert_with(|| json!({}));
        if !auth.is_object() {
            return Err(format!(
                "{} 的 security.auth 不是对象，已拒绝接管",
                settings_path.display()
            ));
        }
        auth.as_object_mut()
            .expect("checked object")
            .insert("selectedType".into(), json!(SELECTED_TYPE_API_KEY));
        if let Some(want) = &sel.default {
            if !model_is_current {
                let root = wanted.as_object_mut().expect("checked object");
                root.insert("model".to_string(), json!({ "name": want }));
            }
        }
    }
    let note = if raw.is_none() {
        "文件不存在，接管时会创建（回滚时删除）".to_string()
    } else if managed {
        "已选择 API Key 认证".to_string()
    } else {
        "写入 security.auth.selectedType = gemini-api-key".to_string()
    };
    let (eval, edit) = finish_json(&settings_path, raw, wanted, managed, note);
    files.push(eval);
    if let Some(edit) = edit {
        edits.push(edit);
    }

    Ok(Evaluation { files, edits })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takeover::atomic_write_private;
    use tempfile::TempDir;

    fn ep(port: u16) -> Endpoint {
        Endpoint {
            meta: Default::default(),
            base_root: format!("http://127.0.0.1:{port}"),
            api_root: format!("http://127.0.0.1:{port}/v1"),
            token: "tk".into(),
            models: vec![],
            codex_wire_api: "chat".into(),
        }
    }

    fn env(tmp: &TempDir) -> TakeoverEnv {
        TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("backups"))
    }

    #[test]
    fn patches_env_in_place_and_keeps_other_lines() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".gemini")).unwrap();
        std::fs::write(dot_env_path(&e), "# mine\nFOO=bar\nGEMINI_API_KEY=old\n").unwrap();

        let evaluation = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        assert_eq!(evaluation.edits.len(), 2, "两个文件都要写");
        let env_edit = evaluation
            .edits
            .iter()
            .find(|x| x.path == dot_env_path(&e))
            .unwrap();
        assert!(env_edit.content.contains("# mine"), "{}", env_edit.content);
        assert!(env_edit.content.contains("FOO=bar"));
        assert!(env_edit.content.contains("GEMINI_API_KEY=tk"));
        assert!(env_edit
            .content
            .contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:8787"));
        assert!(!env_edit.content.contains("old"));
    }

    #[test]
    fn idempotent_after_write() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let evaluation = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        for edit in &evaluation.edits {
            atomic_write_private(&edit.path, &edit.content).unwrap();
        }
        let again = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        assert!(again.edits.is_empty(), "{:?}", again.edits);
        assert!(again.managed());
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
            models: vec![],
            codex_wire_api: "chat".into(),
        }
    }

    #[test]
    fn writes_model_name_into_settings() {
        let tmp = TempDir::new().unwrap();
        let e = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"));
        let sel = Selection {
            default: Some("gemini-x".into()),
            tiers: Default::default(),
            ..Default::default()
        };
        let ev = evaluate(&e, &ep(), &sel).unwrap();
        for edit in &ev.edits {
            atomic_write_private(&edit.path, &edit.content).unwrap();
        }
        assert_eq!(
            read_selection(&e).unwrap().default.as_deref(),
            Some("gemini-x")
        );
        assert!(evaluate(&e, &ep(), &sel).unwrap().edits.is_empty());
    }
}
