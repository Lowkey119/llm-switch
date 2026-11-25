//! OpenCode：`<$XDG_CONFIG_HOME|~/.config>/opencode/opencode.json`。
//!
//! 只拥有 `provider.llm-switch` 这一个节点（`npm`/`name`/`options.baseURL`/
//! `options.apiKey`/`models`），别人的 provider 和顶层 `model`（用户在 OpenCode
//! 里自己选的默认模型）都不碰——接管只负责「让网关出现在可选 provider 里」，
//! 默认模型写顶层 `model = "llm-switch/<名>"`（M12-E）：接管前是人家自己的选择，
//! 接管后就该由网关说了算，否则切模型还得去客户端里点。

use super::{
    finish_json, is_local_endpoint, json_str_eq, parse_json, read_raw, Edit, Endpoint, Evaluation,
    FileEval, Selection, TakeoverEnv, PROVIDER_ID,
};
use serde_json::{json, Value};
use std::path::PathBuf;

/// 同一个内核，两家发行：opencode 原版与 DevEco Code（华为 CLI，内核就是 opencode）。
/// 区别只在配置落点与文件名；provider 节点形状一模一样。
#[derive(Debug, Clone, Copy)]
pub struct Kernel {
    pub dir: &'static str,
    pub file: &'static str,
    pub label: &'static str,
    /// `.jsonc` 允许注释。无损 JSONC 编辑器落地之前，我们不敢覆写带注释的文件。
    pub jsonc: bool,
    /// DevEco 的配置校验器强制要求每个模型都带完整的 `limit`（`context` 与
    /// `output` 两把都缺不起，缺整个 DevEco Code 直接拒绝启动——实测
    /// `Missing key provider.llm-switch.models.<名>.limit.context`）。接管因此
    /// **强制兜底写**：真值优先，未知写默认值。opencode 原版没有「必须齐全」这条
    /// 校验，但同样不许写半截（为什么见 [`limit_node`]）。
    pub force_limits: bool,
}

/// DevEco 强制 `limit.output` 而真值未知时的兜底值（32k：现代模型输出上限的
/// 中位数；上游大多还会按自己的上限再钳一次）。
const FALLBACK_OUTPUT_LIMIT: i64 = 32_768;

/// DevEco 强制 `limit.context` 而真值未知时的兜底值。
///
/// 取 128k 而不是更小：这个键只用来做客户端侧的上下文闸门，写小了会提前压缩、
/// 白白丢历史；写大了才有「撑爆上游」的风险，而 llm-switch 网关在转换层还会按
/// 上游上限再钳一次。128k 是当代 agent 模型的公约数，两头都不吃亏。
const FALLBACK_CONTEXT_LIMIT: i64 = 128_000;

/// 一个模型的 `limit` 节点（OpenCode 侧的上下文闸门）。返回 None = 这个键干脆不写。
///
/// 「半截不写」是硬规矩：opencode 的配置正规化把只带 `context` 的 `limit` 判成畸形，
/// 而一处畸形拖垮**整个 provider 节点**——真机日志
/// `path=$.provider.llm-switch kind=invalid action="skipped malformed recognized
/// value"`，后果是接管明明写进去 16 个模型、客户端清单里一个都不出现（实测
/// `opencode models` 里 `llm-switch/` 为零）。所以真值只探到一把时宁可不写
/// （opencode 退回它自己的默认），不猜。
///
/// DevEco 是反方向的硬要求：两把必须都在，缺就兜底（`force_limits`）。
fn limit_node(
    kernel: &Kernel,
    info: Option<&crate::types::EffectiveMeta>,
) -> Option<serde_json::Map<String, Value>> {
    let mut limit = serde_json::Map::new();
    if let Some(info) = info {
        if let Some(c) = info.ctx {
            limit.insert("context".into(), json!(c));
        }
        if let Some(o) = info.output {
            limit.insert("output".into(), json!(o));
        }
    }
    if kernel.force_limits {
        limit
            .entry("context")
            .or_insert_with(|| json!(FALLBACK_CONTEXT_LIMIT));
        limit
            .entry("output")
            .or_insert_with(|| json!(FALLBACK_OUTPUT_LIMIT));
        return Some(limit);
    }
    (limit.contains_key("context") && limit.contains_key("output")).then_some(limit)
}

pub const OPENCODE: Kernel = Kernel {
    dir: "opencode",
    file: "opencode.json",
    label: "opencode",
    jsonc: false,
    force_limits: false,
};

pub const DEV_ECO: Kernel = Kernel {
    dir: "deveco",
    file: "deveco.jsonc",
    label: "DevEco Code",
    jsonc: true,
    force_limits: true,
};

/// 文件里指向本地/自建端点的 provider（网关自己除外）。
///
/// 用户拍板（2026-11）：接管**不再禁用**用户自己的本地直连 provider——直连配置
/// 归用户。这份名单现在只用来**解除**旧版本接管曾写进 `disabled_providers`
/// 的条目（resync 时自愈）；用户自己写进名单的条目（不在这份名单里）不动。
fn hideable_provider_ids(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(map) = doc.get("provider").and_then(|p| p.as_object()) {
        for (id, node) in map {
            if id == PROVIDER_ID {
                continue;
            }
            let base = node
                .get("options")
                .and_then(|o| o.get("baseURL"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if is_local_endpoint(base) {
                out.push(id.clone());
            }
        }
    }
    out.sort();
    out
}

/// 文件里现有的 `disabled_providers`（用户自己写的条目原样保留）。
fn existing_disabled(doc: &Value) -> Vec<String> {
    doc.get("disabled_providers")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub fn config_path(env: &TakeoverEnv) -> PathBuf {
    config_path_in(env, &OPENCODE)
}

pub fn config_path_in(env: &TakeoverEnv, kernel: &Kernel) -> PathBuf {
    env.file(
        env.dir("XDG_CONFIG_HOME", ".config")
            .join(kernel.dir)
            .join(kernel.file),
    )
}

/// 读配置文件并解析。`.jsonc` 先剥注释再读——**读懂**不等于能原样写回。
pub fn read_doc(env: &TakeoverEnv, kernel: &Kernel) -> Result<Option<Value>, String> {
    let path = config_path_in(env, kernel);
    let Some(text) = read_raw(&path)? else {
        return Ok(None);
    };
    let src = if kernel.jsonc {
        strip_jsonc_comments(&text)
    } else {
        text.clone()
    };
    Ok(Some(parse_json(&src, &path)?))
}

/// 扫一遍文本找真注释（跳过字符串内的 `//`，比如 `https://`）。
fn has_jsonc_comments(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    while i < b.len() {
        match b[i] {
            b'"' if !in_str => in_str = true,
            b'"' if in_str => in_str = false,
            b'\\' if in_str => i += 1,
            b'/' if i + 1 < b.len() && !in_str && (b[i + 1] == b'/' || b[i + 1] == b'*') => {
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// 剥掉 `//` 与 `/* */` 注释；字符串里的不动。写回时注释会消失，所以只在读时用。
fn strip_jsonc_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '/' => match chars.peek() {
                Some('/') => {
                    for n in chars.by_ref() {
                        if n == '\n' {
                            out.push(n);
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next(); // 吃掉 `*`
                    loop {
                        match chars.next() {
                            None => break,
                            Some('*') => {
                                if chars.peek() == Some(&'/') {
                                    chars.next();
                                    break;
                                }
                            }
                            Some(_) => {}
                        }
                    }
                }
                _ => out.push(c),
            },
            _ => out.push(c),
        }
    }
    out
}

/// opencode 系的默认模型写作 `llm-switch/<名>`；不是我们的 provider 就当没设。
pub fn read_selection(env: &TakeoverEnv) -> Result<Selection, String> {
    read_selection_in(env, &OPENCODE)
}

pub fn read_selection_in(env: &TakeoverEnv, kernel: &Kernel) -> Result<Selection, String> {
    let Some(doc) = read_doc(env, kernel)? else {
        return Ok(Selection::default());
    };
    let prefix = format!("{PROVIDER_ID}/");
    let default = doc
        .get("model")
        .and_then(Value::as_str)
        .and_then(|m| m.strip_prefix(prefix.as_str()))
        .map(str::to_string)
        .filter(|v| !v.is_empty());
    // 我们的 provider 节点在不在 = 这份清单该不该由我们维护：
    // · 节点不在（从没接管过）→ `None`，零配置默认态（全暴露）；
    // · 节点在 → 就算 models 是空的也是「我们写的空清单」，必须读成
    //   `Some(空集)`，否则矩阵会把「一个都没分配」当成「清单缺省=全暴露」，
    //   整列恒亮——那正是这张表要消灭的假象。
    let node = doc.pointer(&format!("/provider/{PROVIDER_ID}")).cloned();
    let keys: Vec<String> = node
        .as_ref()
        .and_then(|n| n.get("models"))
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    Ok(Selection {
        manage_tiers: false,
        default,
        tiers: Default::default(),
        allow: node.map(|_| keys.into_iter().collect()),
        ..Default::default()
    })
}

pub fn evaluate(env: &TakeoverEnv, ep: &Endpoint, sel: &Selection) -> Result<Evaluation, String> {
    evaluate_in(env, ep, sel, &OPENCODE)
}

pub fn evaluate_in(
    env: &TakeoverEnv,
    ep: &Endpoint,
    sel: &Selection,
    kernel: &Kernel,
) -> Result<Evaluation, String> {
    let path = config_path_in(env, kernel);
    let raw = read_raw(&path)?;
    if kernel.jsonc && raw.as_deref().is_some_and(has_jsonc_comments) {
        return Err(format!(
            "{} 里有注释：现在的接管会把注释洗掉，已拒绝写入。",
            path.display()
        ));
    }
    let current = match &raw {
        Some(text) => {
            let src = if kernel.jsonc {
                strip_jsonc_comments(text)
            } else {
                text.clone()
            };
            parse_json(&src, &path)?
        }
        None => json!({}),
    };
    if !current.is_object() {
        return Err(format!("{} 根节点不是对象，已拒绝接管", path.display()));
    }

    // 旧版本接管压进 disabled_providers 的那批本地 provider（用户拍板 2026-11：
    // 不再禁用直连）。resync 时解除这笔旧账；用户自己禁的条目不在名单里、不动。
    let hide = hideable_provider_ids(&current);
    let disabled_now = existing_disabled(&current);
    let stale: Vec<String> = disabled_now
        .iter()
        .filter(|id| hide.iter().any(|h| h == *id))
        .cloned()
        .collect();
    let managed = json_str_eq(
        &current,
        &format!("/provider/{PROVIDER_ID}/options/baseURL"),
        &ep.api_root,
    ) && json_str_eq(
        &current,
        &format!("/provider/{PROVIDER_ID}/options/apiKey"),
        &ep.token,
    ) && stale.is_empty();

    let mut wanted = current.clone();
    // 模型列表只写**分配给这个端的模型**：槽位/默认档 + 用户在矩阵里勾的清单，
    // 外加文件里顶层 `model` 正指着我们的那个（我们自己写进去的键，清单里没有
    // 就是个必然 404）。网关暴露但没分配给它的模型一律不写——接管的意义就是
    // 「这个端只看到它该用的那几个」。
    let mut assigned = sel.assigned_models();
    // 顶层 `model` 若还指着我们的模型（写过、且这次没托管/没设默认档，所以不会被
    // 改写），清单里必须留着它——否则配置指着一个清单里没有的名字，必然 404。
    // 默认档被我们改写时以 `sel.default` 为准，别把**上一次**的默认档钉死在清单里。
    if sel.default.is_none() {
        if let Some(m) = current
            .get("model")
            .and_then(Value::as_str)
            .and_then(|m| m.strip_prefix(&format!("{PROVIDER_ID}/")))
            .filter(|m| !m.is_empty())
        {
            assigned.insert(m.to_string());
        }
    }
    let listed = super::listed_models(&assigned, &ep.models);
    let node = json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": PROVIDER_ID,
        "options": { "baseURL": ep.api_root, "apiKey": ep.token },
        "models": listed
            .iter()
            .map(|m| {
                let mut v = json!({ "name": m });
                let info = ep.meta.get(m.as_str());
                if let Some(limit) = limit_node(kernel, info) {
                    v["limit"] = Value::Object(limit);
                }
                if let Some(info) = info {
                    // 不写 `reasoning`：opencode 已把它当遗留键忽略（每次启动一条
                    // `omitted unsupported legacy setting` 告警），写了既不生效又脏日志。
                    // VLM → 输入模态（opencode 的 `modalities.input`）。
                    if let Some(vis) = info.vlm {
                        v["modalities"] = if vis {
                            json!({ "input": ["text", "image"], "output": ["text"] })
                        } else {
                            json!({ "input": ["text"], "output": ["text"] })
                        };
                    }
                }
                (m.clone(), v)
            })
            .collect::<serde_json::Map<String, Value>>(),
    });
    {
        let root = wanted.as_object_mut().expect("checked object");
        let providers = root.entry("provider").or_insert_with(|| json!({}));
        if !providers.is_object() {
            return Err(format!(
                "{} 的 provider 不是对象，已拒绝接管",
                path.display()
            ));
        }
        providers
            .as_object_mut()
            .expect("checked object")
            .insert(PROVIDER_ID.to_string(), node);
    }

    // 解除旧版本接管压上的禁用条目（只动这批 id，别的一律不碰）。
    if !stale.is_empty() {
        let kept: Vec<String> = disabled_now
            .into_iter()
            .filter(|id| !hide.iter().any(|h| h == id))
            .collect();
        let root = wanted.as_object_mut().expect("checked object");
        if kept.is_empty() {
            root.remove("disabled_providers");
        } else {
            root.insert("disabled_providers".to_string(), json!(kept));
        }
    }

    // 默认模型：opencode 用 "provider/模型名" 形式。
    let model_is_current = match &sel.default {
        Some(want) => json_str_eq(&current, "/model", &format!("{PROVIDER_ID}/{want}")),
        None => true,
    };
    if let Some(want) = &sel.default {
        if !model_is_current {
            let root = wanted.as_object_mut().expect("checked object");
            root.insert("model".to_string(), json!(format!("{PROVIDER_ID}/{want}")));
        }
    }
    let managed = managed && model_is_current;

    let note = if raw.is_none() {
        format!("{} 不存在，接管时会创建（回滚时删除）", kernel.label)
    } else if managed {
        format!("已注册 {PROVIDER_ID} provider")
    } else {
        format!("写入 provider.{PROVIDER_ID} 与默认模型")
    };

    let (eval, edit): (FileEval, Option<Edit>) = finish_json(&path, raw, wanted, managed, note);
    Ok(Evaluation {
        files: vec![eval],
        edits: edit.into_iter().collect(),
    })
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
            models: vec!["m1".into(), "m2".into()],
            codex_wire_api: "chat".into(),
            direct: false,
        }
    }

    fn env(tmp: &TempDir) -> TakeoverEnv {
        TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("backups"))
    }

    /// 清单只写**分配给这个端**的模型；本组只关心写进去的字段长什么样，
    /// 所以给个「全都分配掉」的选择，让清单回到全量。
    fn assign_all(ep: &Endpoint) -> Selection {
        Selection {
            allow: Some(ep.models.iter().cloned().collect()),
            ..Default::default()
        }
    }

    #[test]
    fn writes_provider_and_keeps_top_level_model() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let dir = config_path(&e).parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            config_path(&e),
            "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"model\": \"other/foo\"\n}\n",
        )
        .unwrap();

        let evaluation = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        let text = &evaluation.edits[0].content;
        assert!(text.contains("$schema"), "{text}");
        assert!(text.contains("\"model\": \"other/foo\""), "{text}");
        assert!(text.contains("@ai-sdk/openai-compatible"), "{text}");
        assert!(text.contains("http://127.0.0.1:8787/v1"), "{text}");

        atomic_write_private(&evaluation.edits[0].path, &evaluation.edits[0].content).unwrap();
        assert!(evaluate(&e, &ep(8787), &Selection::default())
            .unwrap()
            .edits
            .is_empty());
    }

    #[test]
    fn writes_limit_when_known_and_omits_when_not() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);

        let mut with = ep(8787);
        with.meta.insert(
            "m1".into(),
            crate::types::EffectiveMeta {
                ctx: Some(131_072),
                output: Some(8_192),
                ..Default::default()
            },
        );
        let text = &evaluate(&e, &with, &assign_all(&with)).unwrap().edits[0].content;
        assert!(text.contains("\"limit\""), "{text}");
        assert!(text.contains("\"context\": 131072"), "{text}");
        assert!(text.contains("\"output\": 8192"), "{text}");

        // 未知不写 limit（不猜）。
        let text2 = &evaluate(&e, &ep(8787), &assign_all(&ep(8787)))
            .unwrap()
            .edits[0]
            .content;
        assert!(!text2.contains("\"limit\""), "{text2}");
    }

    #[test]
    fn takeover_never_disables_user_providers_and_lifts_old_debt() {
        // 用户拍板（2026-11）：接管不再禁用用户自己的本地直连 provider；
        // 旧版本压进 disabled_providers 的那批在 resync 时解除，用户自己写的
        // 条目（`user-off` 不在 provider 清单里，算用户自己的）一个字不动。
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let dir = config_path(&e).parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            config_path(&e),
            r#"{ "disabled_providers": ["lab", "user-off"],
                "provider": {
                  "lab": { "npm": "@ai-sdk/openai-compatible", "options": { "baseURL": "http://10.0.0.21:8000/v1" } },
                  "anthropic": { "npm": "@ai-sdk/anthropic", "options": { "baseURL": "https://api.anthropic.com" } }
                } }"#,
        )
        .unwrap();

        let evaluation = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        let text = &evaluation.edits[0].content;
        let written: Value = serde_json::from_str(text).expect("写回的仍是合法 JSON");
        let disabled: Vec<&str> = written["disabled_providers"]
            .as_array()
            .expect("该保留 disabled_providers")
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(
            disabled,
            vec!["user-off"],
            "旧接管压的本地条目解除，用户自己的不动"
        );
        // 别家 provider 本体一个字不动（包括在线 API）
        assert!(text.contains("api.anthropic.com"), "{text}");
        assert!(text.contains("@ai-sdk/anthropic"), "{text}");

        atomic_write_private(&evaluation.edits[0].path, &evaluation.edits[0].content).unwrap();
        assert!(
            evaluate(&e, &ep(8787), &Selection::default())
                .unwrap()
                .edits
                .is_empty(),
            "解除干净就该幂等，不该每次重写"
        );
    }

    #[test]
    fn local_endpoint_rule() {
        for url in [
            "http://127.0.0.1:8787/v1",
            "http://localhost:4000",
            "http://10.0.0.21:8000/v1",
            "http://192.168.1.7:1234",
            "http://omlx.local:8000",
            "http://[::1]:8080",
        ] {
            assert!(crate::takeover::is_local_endpoint(url), "{url} 应判本地");
        }
        for url in [
            "https://api.anthropic.com",
            "https://open.bigmodel.cn/api/paas/v4",
            "http://opencode.ai/config.json",
            "",
        ] {
            assert!(
                !crate::takeover::is_local_endpoint(url),
                "{url} 是在线 API，不该判本地"
            );
        }
    }

    #[test]
    fn half_limit_never_reaches_opencode() {
        // 只探到 ctx、output 未知：绝不能写半截 limit。opencode 会把整个
        // provider 节点判成畸形一起丢掉（真机 kind=invalid），分配再多次
        // 客户端清单里也是空的——这条钉住「齐全才写」。
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let mut with = ep(8787);
        with.meta.insert(
            "m1".into(),
            crate::types::EffectiveMeta {
                ctx: Some(256_000),
                output: None,
                ..Default::default()
            },
        );
        let text = &evaluate(&e, &with, &assign_all(&with)).unwrap().edits[0].content;
        assert!(
            !text.contains("\"limit\""),
            "半截 limit 会拖垮整个 provider 节点: {text}"
        );
        // 思考位不再写：opencode 已把 `reasoning` 当遗留键忽略。
        assert!(!text.contains("\"reasoning\""), "{text}");

        // DevEco 相反：两把必须都在，缺的那把兜底补上，否则它拒绝启动。
        let dev = &evaluate_in(&e, &with, &assign_all(&with), &DEV_ECO)
            .unwrap()
            .edits[0]
            .content;
        assert!(dev.contains("\"context\": 256000"), "{dev}");
        assert!(dev.contains("\"output\": 32768"), "{dev}");
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
            direct: false,
        }
    }

    #[test]
    fn writes_provider_prefixed_model() {
        let tmp = TempDir::new().unwrap();
        let e = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"));
        let sel = Selection {
            default: Some("qwen-mini".into()),
            tiers: Default::default(),
            ..Default::default()
        };
        let ev = evaluate(&e, &ep(), &sel).unwrap();
        atomic_write_private(&ev.edits[0].path, &ev.edits[0].content).unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(config_path(&e)).unwrap()).unwrap();
        assert_eq!(doc["model"], "llm-switch/qwen-mini");
        assert_eq!(
            read_selection(&e).unwrap().default.as_deref(),
            Some("qwen-mini")
        );
    }

    #[test]
    fn foreign_provider_is_reported_as_unset() {
        let tmp = TempDir::new().unwrap();
        let e = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"));
        std::fs::create_dir_all(config_path(&e).parent().unwrap()).unwrap();
        std::fs::write(config_path(&e), "{\"model\": \"other/x\"}").unwrap();
        // 不是我们的 provider 就别冒充成「当前模型」
        assert!(read_selection(&e).unwrap().default.is_none());
    }
}

#[cfg(test)]
mod deveco_tests {
    //! DevEco Code（内核 opencode，配置 `~/.config/deveco/deveco.jsonc`）。
    use super::*;
    use crate::takeover::atomic_write_private;
    use tempfile::TempDir;

    fn ep() -> Endpoint {
        Endpoint {
            meta: Default::default(),
            base_root: "http://127.0.0.1:8787".into(),
            api_root: "http://127.0.0.1:8787/v1".into(),
            token: "tk".into(),
            models: vec!["m-one".into(), "m-two".into()],
            codex_wire_api: "chat".into(),
            direct: false,
        }
    }

    fn env(tmp: &TempDir) -> TakeoverEnv {
        TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("backups"))
    }

    fn seed(e: &TakeoverEnv, body: &str) {
        let path = config_path_in(e, &DEV_ECO);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
    }

    /// 清单只写**分配给这个端**的模型；本组只关心写进去的字段长什么样，
    /// 所以给个「全都分配掉」的选择，让清单回到全量。
    fn assign_all(ep: &Endpoint) -> Selection {
        Selection {
            allow: Some(ep.models.iter().cloned().collect()),
            ..Default::default()
        }
    }

    #[test]
    fn takes_over_deveco_without_touching_other_providers() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        assert!(config_path_in(&e, &DEV_ECO)
            .display()
            .to_string()
            .contains("deveco/deveco.jsonc"));
        seed(
            &e,
            "{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"provider\": { \"litellm\": { \"npm\": \"@ai-sdk/openai-compatible\" } }\n}\n",
        );

        let sel = Selection {
            default: Some("m-one".into()),
            ..Selection::default()
        };
        let evaluation = evaluate_in(&e, &ep(), &sel, &DEV_ECO).unwrap();
        // 配置编辑器只暴露 evaluate 报回来的那几个文件：DevEco 就该只有一份
        // deveco.jsonc，`keys` / `token.dek` 那些鉴权材料根本进不了视野。
        assert_eq!(evaluation.files.len(), 1, "只暴露配置文件本身");
        assert!(
            evaluation.files[0].path.ends_with("deveco/deveco.jsonc"),
            "{}",
            evaluation.files[0].path
        );
        let text = &evaluation.edits[0].content;
        assert!(text.contains("litellm"), "别家的 provider 不能丢: {text}");
        assert!(text.contains(PROVIDER_ID), "{text}");
        assert!(
            text.contains(&format!("\"model\": \"{PROVIDER_ID}/m-one\"")),
            "{text}"
        );

        atomic_write_private(&evaluation.edits[0].path, &evaluation.edits[0].content).unwrap();
        assert!(evaluate_in(&e, &ep(), &sel, &DEV_ECO)
            .unwrap()
            .edits
            .is_empty());
        let read = read_selection_in(&e, &DEV_ECO).unwrap();
        assert_eq!(read.default.as_deref(), Some("m-one"));
        // 清单只留分配给它的那个（m-two 没分给 DevEco，不进它的清单）
        assert_eq!(read.allow.map(|s| s.len()), Some(1));
    }

    #[test]
    fn forces_full_limit_even_when_meta_unknown() {
        // DevEco 校验器要求 limit 的 context 与 output 两把齐全，缺任何一把
        // DevEco 都拒绝启动（实测 Missing key ...limit.context）。真值未知也必须
        // 兜底写——这是接管能用的硬前提。
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let text = &evaluate_in(&e, &ep(), &assign_all(&ep()), &DEV_ECO)
            .unwrap()
            .edits[0]
            .content;
        assert_eq!(
            text.matches("\"limit\"").count(),
            2,
            "两个模型都要有 limit: {text}"
        );
        assert_eq!(
            text.matches("\"output\": 32768").count(),
            2,
            "output 真值未知时兜底: {text}"
        );
        assert_eq!(
            text.matches("\"context\": 128000").count(),
            2,
            "context 真值未知时同样兜底: {text}"
        );

        // 真值已知时用真值，不写兜底；另一半模型仍未知 → 仍兜底。
        let mut with = ep();
        with.meta.insert(
            "m-one".into(),
            crate::types::EffectiveMeta {
                output: Some(8_192),
                ..Default::default()
            },
        );
        let text2 = &evaluate_in(&e, &with, &assign_all(&with), &DEV_ECO)
            .unwrap()
            .edits[0]
            .content;
        assert!(text2.contains("\"output\": 8192"), "{text2}");
        assert_eq!(
            text2.matches("\"output\": 32768").count(),
            1,
            "m-two 未知仍兜底: {text2}"
        );

        // opencode 原版无此要求，「未知不写」口径不变。
        let text3 = &evaluate(&e, &ep(), &assign_all(&ep())).unwrap().edits[0].content;
        assert!(!text3.contains("\"limit\""), "{text3}");
    }

    #[test]
    fn jsonc_reads_comments_but_never_washes_them_out() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        // 注意 schema 里的 `//` 在字符串里，不该被当成注释
        seed(
            &e,
            "{\n  // 我的 DevEco 配置\n  \"$schema\": \"https://opencode.ai/config.json\"\n}\n",
        );
        assert!(read_selection_in(&e, &DEV_ECO).is_ok(), "读要能扛住注释");
        let err = evaluate_in(&e, &ep(), &Selection::default(), &DEV_ECO).unwrap_err();
        assert!(err.contains("注释"), "{err}");
        assert!(!has_jsonc_comments(
            "{\"u\":\"https://opencode.ai/config.json\"}"
        ));
    }
}
