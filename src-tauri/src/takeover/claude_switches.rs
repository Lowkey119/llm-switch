//! claude 语义开关（P0-a）：把 `settings.json` 里那几个「有名字的意思」抽成一张表。
//!
//!
//! 为什么不是表单而是表：这些开关的本质是**固定的 env 键 + 固定字面值**，抄自
//! 键名一旦写死在代码里，界面就不会漂。
//!
//! 铁律跟接管其它东西一样：
//! · **真值在文件里**——读的时候现场读 `settings.json`，不看库；
//! · **写之前留快照**，走 `write_backup` + 原子写，跟 apply 同一套——但只有
//!   「还没进入接管态」时才真备一份：接管态内的重写沿用接管时那份原件备份；
//! · **与 apply 共用同一把接管锁**——「读 settings → 改一个键 → 写回」是读改写，
//!   不持锁就会被并发的接管/还原插进来，双方互相覆盖；
//! · **未接管不许改**——没接管就没有替别人改配置的资格；
//! · 「隐藏 AI 署名」写的是 JSON 字段不是 env，单独一路，别塞进 env 块。

use super::{
    atomic_write_private, claude::settings_path, connection_managed, lock_takeover, read_raw,
    write_backup, ClientApp, Endpoint, TakeoverEnv,
};
use crate::store::Store;
use serde_json::{json, Map, Value};

pub struct EnvSwitch {
    pub key: &'static str,
    pub label: &'static str,
    pub on: &'static str,
}

pub const ENV_SWITCHES: &[EnvSwitch] = &[
    EnvSwitch {
        key: "DISABLE_AUTOUPDATER",
        label: "禁用自动升级",
        on: "1",
    },
    EnvSwitch {
        key: "ENABLE_TOOL_SEARCH",
        label: "启用 Tool Search",
        on: "true",
    },
    EnvSwitch {
        key: "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS",
        label: "Teammates 模式",
        on: "1",
    },
    EnvSwitch {
        key: "CLAUDE_CODE_DISABLE_ARTIFACT",
        label: "禁用 Artifact 工具",
        on: "1",
    },
    EnvSwitch {
        key: "CLAUDE_CODE_EFFORT_LEVEL",
        label: "最大强度思考",
        on: "max",
    },
];

/// 数值型：开着是个数字，关掉就是没这个键。
pub const NUMERIC_KEYS: &[(&str, &str)] = &[
    ("CLAUDE_CODE_MAX_CONTEXT_TOKENS", "上下文上限"),
    ("CLAUDE_CODE_MAX_OUTPUT_TOKENS", "输出上限"),
];

/// 「隐藏 AI 署名」的特殊键：写 `attribution` 三个字段，不是 env。
pub const ATTRIBUTION_KEY: &str = "__attribution_hidden";

/// 开关表本身（键、中文名、类型）——界面照着渲染，别在前端再拄一遍。
pub fn defs() -> Value {
    let mut out: Vec<Value> = ENV_SWITCHES
        .iter()
        .map(|s| json!({ "key": s.key, "label": s.label, "kind": "bool" }))
        .collect();
    for (k, label) in NUMERIC_KEYS {
        out.push(json!({ "key": k, "label": label, "kind": "number" }));
    }
    out.push(json!({ "key": ATTRIBUTION_KEY, "label": "隐藏 AI 署名", "kind": "bool" }));
    Value::Array(out)
}

/// 读现状：`{ defs, state }`。state 里布尔开关是 `true/false`，数值键是字符串或 null。
pub fn read_switches(env: &TakeoverEnv) -> Result<Value, String> {
    let path = settings_path(env);
    let state = match read_raw(&path)? {
        Some(text) => {
            let doc: Value =
                serde_json::from_str(&text).map_err(|e| format!("settings.json 解析失败: {e}"))?;
            state_from(&doc)
        }
        None => empty_state(),
    };
    Ok(json!({ "defs": defs(), "state": state }))
}

/// 开关表的空壳：所有布尔开关 = 关，数值键 = 无。
fn blank_state() -> Map<String, Value> {
    let mut map = Map::new();
    for s in ENV_SWITCHES {
        map.insert(s.key.to_string(), json!(false));
    }
    for (k, _) in NUMERIC_KEYS {
        map.insert((*k).to_string(), Value::Null);
    }
    map.insert(ATTRIBUTION_KEY.into(), json!(false));
    map
}

fn empty_state() -> Value {
    Value::Object(blank_state())
}

fn state_from(doc: &Value) -> Value {
    let mut obj = blank_state();
    let env = doc.get("env").and_then(Value::as_object);
    for s in ENV_SWITCHES {
        // claude 世界里这些值大多是字符串，但历史上也见过数字 1 / 布尔，宽容比对
        let on = env
            .and_then(|e| e.get(s.key))
            .map(|v| {
                matches!(v, Value::String(t) if t.eq_ignore_ascii_case(s.on)) || {
                    match s.on {
                        "1" => v.as_i64() == Some(1) || v.as_bool() == Some(true),
                        "true" => v.as_bool() == Some(true),
                        _ => false,
                    }
                }
            })
            .unwrap_or(false);
        obj.insert(s.key.to_string(), json!(on));
    }
    for (k, _) in NUMERIC_KEYS {
        let v = env.and_then(|e| e.get(*k)).and_then(|v| match v {
            Value::String(t) => Some(json!(t.clone())),
            Value::Number(n) => Some(json!(n.to_string())),
            _ => None,
        });
        obj.insert((*k).to_string(), v.unwrap_or(Value::Null));
    }
    // 署名：三个字段都"清干净"才算开了隐藏
    let hidden = doc
        .get("attribution")
        .map(|a| {
            a.get("sessionUrl").and_then(Value::as_bool) == Some(false)
                && a.get("commit").and_then(Value::as_str) == Some("")
                && a.get("pr").and_then(Value::as_str) == Some("")
        })
        .unwrap_or(false);
    obj.insert(ATTRIBUTION_KEY.into(), json!(hidden));
    Value::Object(obj)
}

/// 改一个开关。`value`：布尔开关传 `Bool`，数值键传字符串（空串 = 清除）。
pub fn set_switch(
    env: &TakeoverEnv,
    store: &Store,
    ep: &Endpoint,
    key: &str,
    value: &Value,
) -> Result<Value, String> {
    // 与接管同一把锁：`set_switch` 是「读 settings → 改一个键 → 写回」的
    // 读改写序列，不持锁就会和并发的 apply/还原互相覆盖（丢改写）。
    let _guard = lock_takeover();
    if !connection_managed(env, store, ep, ClientApp::Claude)? {
        return Err("Claude Code 尚未接管，不能替它改配置".into());
    }
    let known = ENV_SWITCHES.iter().any(|s| s.key == key)
        || NUMERIC_KEYS.iter().any(|(k, _)| *k == key)
        || key == ATTRIBUTION_KEY;
    if !known {
        return Err(format!("未知开关: {key}"));
    }
    let path = settings_path(env);
    let text =
        read_raw(&path)?.ok_or_else(|| "settings.json 不存在，先接管再接配置".to_string())?;
    let mut doc: Value =
        serde_json::from_str(&text).map_err(|e| format!("settings.json 解析失败: {e}"))?;
    if !doc.is_object() {
        return Err("settings.json 顶层不是对象".into());
    }

    if key == ATTRIBUTION_KEY {
        let on = value.as_bool().unwrap_or(false);
        let root = doc.as_object_mut().expect("checked object");
        if on {
            let mut attr = Map::new();
            attr.insert("sessionUrl".into(), json!(false));
            attr.insert("commit".into(), json!(""));
            attr.insert("pr".into(), json!(""));
            root.insert("attribution".into(), Value::Object(attr));
        } else {
            root.remove("attribution");
        }
    } else {
        let root = doc.as_object_mut().expect("checked object");
        let env_obj = root
            .entry("env")
            .or_insert_with(|| Value::Object(Map::new()));
        let env_obj = env_obj
            .as_object_mut()
            .ok_or_else(|| "settings.json 的 env 不是对象".to_string())?;
        let clear = match value {
            Value::Null => true,
            Value::Bool(false) => true,
            Value::String(s) => s.trim().is_empty(),
            _ => false,
        };
        if clear {
            env_obj.remove(key);
        } else if let Some(s) = ENV_SWITCHES.iter().find(|s| s.key == key) {
            env_obj.insert(key.into(), json!(s.on));
        } else {
            // 数值键：存成字符串（claude 读到的就是字符串），并挡住非数字
            let raw = match value {
                Value::String(s) => s.trim().to_string(),
                Value::Number(n) => n.to_string(),
                other => return Err(format!("{key} 需要数字，收到 {other}")),
            };
            if raw.parse::<u64>().is_err() {
                return Err(format!("{key} 必须是正整数，收到 {raw}"));
            }
            env_obj.insert(key.into(), json!(raw));
        }
    }

    let rendered = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n";
    // 开关切换是**接管态内的重写**：原件备份在进入接管态那一刻就留过了，
    // 这里不再新增（否则每点一次开关就多一份「备份」，回滚认的却还是原件）。
    let state = super::read_state(store, ClientApp::Claude);
    let taken_over = state.applied_at.is_some() && state.reverted_at.is_none();
    let backup_id = if taken_over {
        state.backup_id
    } else {
        Some(write_backup(
            env,
            ClientApp::Claude,
            std::slice::from_ref(&path),
            &ep.base_root,
        )?)
    };
    atomic_write_private(&path, &rendered)?;
    Ok(json!({
        "key": key,
        "backup_id": backup_id.map(Value::String).unwrap_or(Value::Null),
        "state": state_from(&doc),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takeover::{apply, Endpoint};
    use tempfile::TempDir;

    struct Fix {
        env: TakeoverEnv,
        store: Store,
        ep: Endpoint,
        _tmp: TempDir,
    }

    fn fix() -> Fix {
        let tmp = TempDir::new().unwrap();
        let env = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("backups"));
        let store = Store::in_memory().unwrap();
        let ep = Endpoint {
            meta: Default::default(),
            base_root: "http://127.0.0.1:8787".into(),
            api_root: "http://127.0.0.1:8787/v1".into(),
            token: "tk".into(),
            models: vec!["m-one".into()],
            codex_wire_api: "chat".into(),
            direct: false,
        };
        Fix {
            env,
            store,
            ep,
            _tmp: tmp,
        }
    }

    #[test]
    fn switches_roundtrip_and_leave_no_trace_when_off() {
        let f = fix();
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        set_switch(
            &f.env,
            &f.store,
            &f.ep,
            "CLAUDE_CODE_EFFORT_LEVEL",
            &json!(true),
        )
        .unwrap();
        set_switch(
            &f.env,
            &f.store,
            &f.ep,
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
            &json!("200000"),
        )
        .unwrap();
        let st = read_switches(&f.env).unwrap()["state"].clone();
        assert_eq!(st["CLAUDE_CODE_EFFORT_LEVEL"], json!(true));
        assert_eq!(st["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], json!("200000"));

        let raw = std::fs::read_to_string(settings_path(&f.env)).unwrap();
        assert!(raw.contains("\"max\""), "{raw}");
        assert!(raw.contains("200000"));

        // 关掉要真的删键，不是写个 false 留在那儿
        set_switch(
            &f.env,
            &f.store,
            &f.ep,
            "CLAUDE_CODE_EFFORT_LEVEL",
            &json!(false),
        )
        .unwrap();
        set_switch(
            &f.env,
            &f.store,
            &f.ep,
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
            &json!(""),
        )
        .unwrap();
        let raw = std::fs::read_to_string(settings_path(&f.env)).unwrap();
        assert!(!raw.contains("EFFORT_LEVEL"), "{raw}");
        assert!(!raw.contains("200000"));
    }

    #[test]
    fn attribution_writes_json_fields_not_env() {
        let f = fix();
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        set_switch(&f.env, &f.store, &f.ep, ATTRIBUTION_KEY, &json!(true)).unwrap();
        let doc: Value =
            serde_json::from_str(&std::fs::read_to_string(settings_path(&f.env)).unwrap()).unwrap();
        assert_eq!(doc["attribution"]["sessionUrl"], json!(false));
        assert_eq!(doc["attribution"]["commit"], json!(""));
        let st = read_switches(&f.env).unwrap()["state"].clone();
        assert!(st[ATTRIBUTION_KEY].as_bool().unwrap());
        set_switch(&f.env, &f.store, &f.ep, ATTRIBUTION_KEY, &json!(false)).unwrap();
        let st = read_switches(&f.env).unwrap()["state"].clone();
        assert!(!st[ATTRIBUTION_KEY].as_bool().unwrap_or(false));
    }

    #[test]
    fn refuses_unknown_keys_bad_numbers_and_untaken_clients() {
        let f = fix();
        let err =
            set_switch(&f.env, &f.store, &f.ep, "DISABLE_AUTOUPDATER", &json!(true)).unwrap_err();
        assert!(err.contains("尚未接管"), "{err}");
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        assert!(set_switch(&f.env, &f.store, &f.ep, "NO_SUCH_SWITCH", &json!(true)).is_err());
        let err = set_switch(
            &f.env,
            &f.store,
            &f.ep,
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            &json!("abc"),
        )
        .unwrap_err();
        assert!(err.contains("正整数"), "{err}");
    }
}
