//! Pi：`<PI_CODING_AGENT_DIR|~/.pi/agent>/models.json` 的 provider 节点。
//!
//! （`settings.json`），我们只在 `models.json` 里登记一个 provider 节点：
//! `baseUrl` / `api` / `apiKey` / `models`。别人的 provider 节点原样保留。

use super::{
    finish_json, is_local_endpoint, json_str_eq, parse_json, read_raw, Edit, Endpoint, Evaluation,
    FileEval, Selection, TakeoverEnv, PROVIDER_ID,
};
use serde_json::{json, Value};
use std::path::PathBuf;

/// Pi 侧的 OpenAI Chat Completions 协议标识。
pub const PI_API_KIND: &str = "openai-completions";

pub fn models_path(env: &TakeoverEnv) -> PathBuf {
    env.file(
        env.dir("PI_CODING_AGENT_DIR", ".pi/agent")
            .join("models.json"),
    )
}

/// 接管后该摘掉的别家 provider：`baseUrl` 指向本地/私网的那些（在线 API 一律不动）。
///
/// 与 opencode 系用 `disabled_providers` 不同，pi 没有「禁用某 provider」的开关，
/// 只能把节点从 `models.json` 里摘走。摘走不等于丢数据：`apply` 写盘前已把原件
/// 整份快照，放弃接管逐字节还原。
fn hideable_providers(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(map) = doc.get("providers").and_then(|p| p.as_object()) {
        for (id, node) in map {
            if id == PROVIDER_ID {
                continue;
            }
            if is_local_endpoint(node.get("baseUrl").and_then(Value::as_str).unwrap_or("")) {
                out.push(id.clone());
            }
        }
    }
    out.sort();
    out
}

/// pi 的默认模型在它自己的 models-store.json 里（客户端状态，我们不碰），
/// 所以 default 恒空——界面据此显示「由 pi 自己管理」，而不是假装写成功了。
/// 可用清单倒是能从 models.json 里读出真值。
pub fn read_selection(env: &TakeoverEnv) -> Result<Selection, String> {
    let path = models_path(env);
    let Some(text) = read_raw(&path)? else {
        return Ok(Selection::default());
    };
    let doc = parse_json(&text, &path)?;
    let ids: Vec<String> = doc
        .pointer(&format!("/providers/{PROVIDER_ID}/models"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    // 大小模型槽：现场读 piSlots 标记（llm-switch 为 pi 写的大小模型配置）。
    let slots = doc.pointer(&format!("/providers/{PROVIDER_ID}/piSlots"));
    let slot = |key: &str| {
        slots
            .and_then(|s| s.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    Ok(Selection {
        manage_tiers: false,
        default: None,
        tiers: Default::default(),
        allow: (!ids.is_empty()).then(|| ids.into_iter().collect()),
        pi_main: slot("main"),
        pi_small: slot("small"),
        ..Default::default()
    })
}

pub fn evaluate(env: &TakeoverEnv, ep: &Endpoint, sel: &Selection) -> Result<Evaluation, String> {
    let path = models_path(env);
    let raw = read_raw(&path)?;
    let current = match &raw {
        Some(text) => parse_json(text, &path)?,
        None => json!({}),
    };
    if !current.is_object() {
        return Err(format!("{} 根节点不是对象，已拒绝接管", path.display()));
    }

    let base = format!("/providers/{PROVIDER_ID}");
    let hide = hideable_providers(&current);
    let managed = json_str_eq(&current, &format!("{base}/baseUrl"), &ep.api_root)
        && json_str_eq(&current, &format!("{base}/apiKey"), &ep.token)
        && json_str_eq(&current, &format!("{base}/api"), PI_API_KIND)
        && hide.is_empty();

    let mut wanted = current.clone();
    // pi 的模型列表 = 占槽 ∪ 显式清单（BUG-4，用户 2026-11-16）：槽位制只约束
    // 主/小两个档位槽，配置文件（models 数组）允许多个模型清单——格子点「加入清单」
    // 的模型也写进来，由用户在 pi 侧选择使用。∩暴露，没分配的不写。
    let listed = super::listed_models(&sel.assigned_models(), &ep.models);
    let mut node = json!({
        "name": PROVIDER_ID,
        "baseUrl": ep.api_root,
        "api": PI_API_KIND,
        "apiKey": ep.token,
        "models": listed
            .iter()
            .map(|m| {
                let mut e = json!({ "id": m, "name": m });
                // 元数据（ctx/output/reasoning/vlm）：真值才写，未知不写——让 Pi 自己的缺省去兜底，
                // 但不再对已知的模型报 128k（根因现场）。
                if let Some(info) = ep.meta.get(m.as_str()) {
                    if let Some(c) = info.ctx {
                        e["contextWindow"] = json!(c);
                    }
                    if let Some(o) = info.output {
                        e["maxTokens"] = json!(o);
                    }
                    // 思考能力：pi 的 `reasoning` 布尔决定 /thinking 可选档位。
                    if let Some(r) = info.reasoning {
                        e["reasoning"] = json!(r);
                    }
                    // VLM → 输入模态：开 = text+image，关 = 只 text（未知不写，pi 默认 text）。
                    if let Some(v) = info.vlm {
                        e["input"] = if v {
                            json!(["text", "image"])
                        } else {
                            json!(["text"])
                        };
                    }
                }
                e
            })
            .collect::<Vec<Value>>(),
    });
    // 大小模型槽位标记：只写已设的槽，供 pi 的 virtual-model 路由器消费
    // （high→main，low→small）。两个槽都空则不写该键，保持零配置默认态。
    let mut slots = serde_json::Map::new();
    if let Some(m) = &sel.pi_main {
        slots.insert("main".into(), json!(m));
    }
    if let Some(s) = &sel.pi_small {
        slots.insert("small".into(), json!(s));
    }
    if !slots.is_empty() {
        node["piSlots"] = Value::Object(slots);
    }
    {
        let root = wanted.as_object_mut().expect("checked object");
        let providers = root.entry("providers").or_insert_with(|| json!({}));
        if !providers.is_object() {
            return Err(format!(
                "{} 的 providers 不是对象，已拒绝接管",
                path.display()
            ));
        }
        providers
            .as_object_mut()
            .expect("checked object")
            .insert(PROVIDER_ID.to_string(), node);
        // 本地端点的别家 provider 摘走（原件在接管快照里，回滚还原）。
        if !hide.is_empty() {
            if let Some(map) = root.get_mut("providers").and_then(|p| p.as_object_mut()) {
                for id in &hide {
                    map.remove(id);
                }
            }
        }
    }

    let note = if raw.is_none() {
        "文件不存在，接管时会创建（回滚时删除）".to_string()
    } else if managed {
        "已登记 llm-switch provider".to_string()
    } else if ep.models.is_empty() {
        "写入 providers.llm-switch（网关暂无可用模型，models 为空）".to_string()
    } else {
        "写入 providers.llm-switch（含网关模型清单）".to_string()
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
            models: vec!["qwen3".into()],
            codex_wire_api: "chat".into(),
        }
    }

    fn env(tmp: &TempDir) -> TakeoverEnv {
        TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("backups"))
    }

    /// pi 的模型清单只写**占了槽**的模型；本组只关心写进去的字段长什么样，
    /// 所以先给它占上主槽。
    fn with_main(model: &str) -> Selection {
        Selection {
            pi_main: Some(model.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn hides_local_providers_keeps_online_and_lists_models() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let path = models_path(&e);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // litellm 指向本地（接管后该摘走），anthropic 是在线 API（一个字不许动）。
        std::fs::write(
            &path,
            "{\"providers\":{\"litellm\":{\"baseUrl\":\"http://localhost:4000/v1\",\"models\":[]},\"anthropic\":{\"baseUrl\":\"https://api.anthropic.com\",\"models\":[]}}}\n",
        )
        .unwrap();

        let evaluation = evaluate(&e, &ep(8787), &with_main("qwen3")).unwrap();
        let text = &evaluation.edits[0].content;
        assert!(
            !text.contains("litellm"),
            "本地端点 provider 该摘走: {text}"
        );
        assert!(
            text.contains("api.anthropic.com"),
            "在线 API 不得动: {text}"
        );
        assert!(text.contains("openai-completions"), "{text}");
        assert!(text.contains("qwen3"), "{text}");

        atomic_write_private(&evaluation.edits[0].path, &evaluation.edits[0].content).unwrap();
        let again = evaluate(&e, &ep(8787), &with_main("qwen3")).unwrap();
        assert!(again.edits.is_empty());
        assert!(again.managed());
    }

    #[test]
    fn refuses_broken_models_json() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let path = models_path(&e);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
        let err = evaluate(&e, &ep(8787), &Selection::default()).unwrap_err();
        assert!(err.contains("空文件"), "{err}");
    }

    #[test]
    fn writes_context_window_when_known_and_omits_when_not() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        let path = models_path(&e);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();

        let mut with = ep(8787);
        with.meta.insert(
            "qwen3".into(),
            crate::types::EffectiveMeta {
                ctx: Some(262_144),
                output: Some(32_768),
                ..Default::default()
            },
        );
        let ev = evaluate(&e, &with, &with_main("qwen3")).unwrap();
        let text = &ev.edits[0].content;
        assert!(text.contains("\"contextWindow\": 262144"), "{text}");
        assert!(text.contains("\"maxTokens\": 32768"), "{text}");

        // 未知的模型不写字段（不猜），Pi 自己的缺省去兜底。
        let without = ep(8788);
        let ev2 = evaluate(&e, &without, &with_main("qwen3")).unwrap();
        let text2 = &ev2.edits[0].content;
        assert!(!text2.contains("contextWindow"), "{text2}");
        assert!(!text2.contains("maxTokens"), "{text2}");
    }

    #[test]
    fn writes_reasoning_and_input_modality_when_known() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(models_path(&e).parent().unwrap()).unwrap();

        // 真值已知：reasoning 直写，VLM 开 = text+image。
        let mut with = ep(8787);
        with.meta.insert(
            "qwen3".into(),
            crate::types::EffectiveMeta {
                reasoning: Some(true),
                vlm: Some(true),
                ..Default::default()
            },
        );
        let text = &evaluate(&e, &with, &with_main("qwen3")).unwrap().edits[0].content;
        let doc: serde_json::Value = serde_json::from_str(text).unwrap();
        let m = &doc["providers"][PROVIDER_ID]["models"][0];
        assert_eq!(m["reasoning"], json!(true));
        assert_eq!(m["input"], json!(["text", "image"]));

        // VLM 关 = 只 text；reasoning 未知仍不写。
        let mut off = ep(8787);
        off.meta.insert(
            "qwen3".into(),
            crate::types::EffectiveMeta {
                vlm: Some(false),
                ..Default::default()
            },
        );
        let text2 = &evaluate(&e, &off, &with_main("qwen3")).unwrap().edits[0].content;
        let doc2: serde_json::Value = serde_json::from_str(text2).unwrap();
        let m2 = &doc2["providers"][PROVIDER_ID]["models"][0];
        assert_eq!(m2["input"], json!(["text"]));
        assert!(m2.get("reasoning").is_none(), "{text2}");

        // 未知：两个字段都不写（不猜）。
        let ev3 = evaluate(&e, &ep(8789), &with_main("qwen3")).unwrap();
        let text3 = &ev3.edits[0].content;
        assert!(!text3.contains("\"input\""), "{text3}");
        assert!(!text3.contains("reasoning"), "{text3}");
    }

    /// 大小模型槽：evaluate 把 piSlots 写进 provider 节点，read_selection 再读回来，
    /// 两个槽都空则不写键（零配置默认态不留垃圾）。
    #[test]
    fn pi_slots_roundtrip_through_provider_node() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);

        // 全空：不写 piSlots
        let ev = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        let doc: Value = serde_json::from_str(&ev.edits[0].content).unwrap();
        assert!(
            doc["providers"][PROVIDER_ID].get("piSlots").is_none(),
            "空槽不该写 piSlots: {doc}"
        );

        // 设两个槽：写进去，read_selection 原样读回
        let sel = Selection {
            pi_main: Some("ornith-35b-oq6e-mtp".into()),
            pi_small: Some("ornith-9b-mlx-8bit".into()),
            manage_pi: true,
            ..Default::default()
        };
        let ev = evaluate(&e, &ep(8787), &sel).unwrap();
        let text = &ev.edits[0].content;
        let doc: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            doc["providers"][PROVIDER_ID]["piSlots"]["main"],
            json!("ornith-35b-oq6e-mtp")
        );
        assert_eq!(
            doc["providers"][PROVIDER_ID]["piSlots"]["small"],
            json!("ornith-9b-mlx-8bit")
        );
        atomic_write_private(&ev.edits[0].path, text).unwrap();

        let back = read_selection(&e).unwrap();
        assert_eq!(back.pi_main.as_deref(), Some("ornith-35b-oq6e-mtp"));
        assert_eq!(back.pi_small.as_deref(), Some("ornith-9b-mlx-8bit"));

        // 只清空槽：piSlots 键整个消失
        let ev = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        let doc: Value = serde_json::from_str(&ev.edits[0].content).unwrap();
        assert!(
            doc["providers"][PROVIDER_ID].get("piSlots").is_none(),
            "清空后不该残留 piSlots: {doc}"
        );
    }
}
