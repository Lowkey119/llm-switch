//! Codex CLI：`<CODEX_HOME|.codex>/config.toml`。
//!
//! 拥有的键只有两处（其余注释与排版全部由 `toml_edit` 原样保留）：
//! - 顶层 `model_provider = "llm-switch"`（选中我们）
//! - 表 `[model_providers.llm-switch]` 的 `name` / `base_url` / `wire_api` /
//!   `experimental_bearer_token`
//!
//! 也不要求用户去 export 环境变量（`env_key` 缺变量时 Codex 会直接报错）。
//! 接管后 `[model_providers]` 里指向本地/私网的别家 provider 会被摘走（在线 API
//! 那类公网 provider 一律不动）——摘走的原件在接管快照里，放弃接管逐字节还原。
//!
//! `wire_api` 默认 `chat`（网关注册了 `/v1/chat/completions`），
//! 想用 `/v1/responses` 时把 settings 键 `takeover.codex.wire_api` 设成 `responses`。

use super::{
    is_local_endpoint, Edit, Endpoint, Evaluation, FileEval, Selection, TakeoverEnv, PROVIDER_ID,
};
use std::path::PathBuf;
use toml_edit::{value, DocumentMut, Item, Table};

pub fn config_path(env: &TakeoverEnv) -> PathBuf {
    env.file(env.dir("CODEX_HOME", ".codex").join("config.toml"))
}

/// `[model_providers]` 里该摘走的别家 provider：`base_url` 指向本地/私网的那些。
///
/// codex 没有「禁用某 provider」的开关，只能摘表；原件在接管快照里，回滚还原。
/// 在线 API（公网 `base_url`）不动——那是用户自己的账号能力。
fn hideable_provider_tables(doc: &DocumentMut) -> Vec<String> {
    let mut out = Vec::new();
    let Some(providers) = doc.get("model_providers").and_then(|i| i.as_table_like()) else {
        return out;
    };
    for (id, item) in providers.iter() {
        if id == PROVIDER_ID {
            continue;
        }
        let base = item.get("base_url").and_then(|v| v.as_str()).unwrap_or("");
        if is_local_endpoint(base) {
            out.push(id.to_string());
        }
    }
    out.sort();
    out
}

/// 我们拥有的 provider 字段（base_url / wire_api / token）。
fn wanted_fields(ep: &Endpoint) -> [(&'static str, String); 4] {
    [
        ("name", PROVIDER_ID.to_string()),
        ("base_url", ep.api_root.clone()),
        ("wire_api", ep.codex_wire_api.clone()),
        ("experimental_bearer_token", ep.token.clone()),
    ]
}

fn provider_is_current(doc: &DocumentMut, ep: &Endpoint) -> bool {
    if doc.get("model_provider").and_then(Item::as_str) != Some(PROVIDER_ID) {
        return false;
    }
    let Some(provider) = doc
        .get("model_providers")
        .and_then(|item| item.as_table_like())
        .and_then(|t| t.get(PROVIDER_ID))
        .and_then(|item| item.as_table_like())
    else {
        return false;
    };
    wanted_fields(ep)
        .iter()
        .all(|(key, want)| provider.get(key).and_then(|i| i.as_str()) == Some(want.as_str()))
}

/// codex 配置文件里我们认定的「死键」：顶层 `base_url` / `wire_api` 从来不是
/// codex 每天警告一次。只有当 `base_url` 确实指向那个早已不存在的 litellm 端口
/// 时才删——万一是别人真有意义的键，我们不猜。
fn is_legacy_dead_endpoint(value: Option<&str>) -> bool {
    let Some(url) = value else { return false };
    let url = url.trim();
    [
        "http://localhost:4000",
        "http://127.0.0.1:4000",
        "https://localhost:4000",
    ]
    .iter()
    .any(|prefix| url.starts_with(prefix))
}

/// 摘掉成对出现的死键，返回是否动了。
fn prune_dead_keys(doc: &mut DocumentMut) -> bool {
    if !is_legacy_dead_endpoint(doc.get("base_url").and_then(Item::as_str)) {
        return false;
    }
    doc.as_table_mut().remove("base_url");
    // 顶层 wire_api 只有在跟着这条废地址时才一起走：它们是同一次遗留的产物
    if doc.get("wire_api").and_then(Item::as_str).is_some() {
        doc.as_table_mut().remove("wire_api");
    }
    true
}

/// 我们自己生成的 catalog 文件名（放在 codex 的配置目录下）。
pub const CATALOG_FILE: &str = "llm-switch-model-catalog.json";

/// 模板：优先用我们已经生成的那份（稳定），否则借用用户已有 catalog 的首条。
/// 拿不到模板就**不生成**——catalog 里那些 base_instructions 之类的字段编不出来，
/// 硬造一份看起来像的反而更糟。
/// 有没有可用的 catalog 模板（没有就没法生成清单，宁缺不造）。
pub fn has_catalog_template(env: &TakeoverEnv) -> bool {
    match super::read_raw(&config_path(env)) {
        Ok(Some(text)) => text
            .parse::<DocumentMut>()
            .map(|doc| catalog_template(env, &doc).is_some())
            .unwrap_or(false),
        _ => false,
    }
}

fn catalog_template(env: &TakeoverEnv, doc: &DocumentMut) -> Option<serde_json::Value> {
    let dir = config_path(env).parent()?.to_path_buf();
    let mine = dir.join(CATALOG_FILE);
    if let Ok(text) = std::fs::read_to_string(&mine) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(first) = v
                .get("models")
                .and_then(|m| m.as_array())
                .and_then(|a| a.first())
            {
                return Some(first.clone());
            }
        }
    }
    let referenced = doc.get("model_catalog_json").and_then(Item::as_str)?;
    let path = if std::path::Path::new(referenced).is_absolute() {
        std::path::PathBuf::from(referenced)
    } else {
        dir.join(referenced)
    };
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()?
        .get("models")?
        .as_array()?
        .first()
        .cloned()
}

/// 为每个暴露模型复刻一条 catalog 条目：只换身份字段，其余照模板。
/// ctx 真值已知时覆写 `context_window`（模板里有 `max_context_window` 就成对改），
/// 未知则沿用模板值——真值优先，没有才用模板（不再对每个模型报同一窗口）。
/// reasoning/vlm 不写：codex 的 catalog 没有可靠对应槽位
/// （`supports_reasoning_summaries` 说的是摘要不是能力），宁缺勿错。
fn render_catalog(
    template: &serde_json::Value,
    models: &[String],
    meta: &std::collections::BTreeMap<String, crate::types::EffectiveMeta>,
) -> Option<String> {
    let base = template.as_object()?;
    let entries: Vec<serde_json::Value> = models
        .iter()
        .map(|m| {
            let mut e = base.clone();
            e.insert("slug".into(), serde_json::json!(m));
            e.insert("display_name".into(), serde_json::json!(m));
            e.insert(
                "description".into(),
                serde_json::json!(format!("{m} via llm-switch")),
            );
            if let Some(c) = meta.get(m).and_then(|i| i.ctx) {
                let had_max = e.contains_key("max_context_window");
                e.insert("context_window".into(), serde_json::json!(c));
                if had_max {
                    e.insert("max_context_window".into(), serde_json::json!(c));
                }
                // 真值来自推理引擎自己声明的 max_model_len/n_ctx，那就是硬上限；
                // 模板带来的 effective_context_window_percent（常见 95）是在硬上限上
                // 再砍一刀，等于白扔 5% 窗口。真值已知就按 100 全量交给 codex 用，
                // 逼近上限时由 codex 自己的压缩兜底；沿用模板值时不动它（那只是借来的形状）。
                if e.contains_key("effective_context_window_percent") {
                    e.insert(
                        "effective_context_window_percent".into(),
                        serde_json::json!(100),
                    );
                }
            }
            serde_json::Value::Object(e)
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "models": entries })).ok()
}

/// 读回 codex 现在真的在用哪个模型。
pub fn read_selection(env: &TakeoverEnv) -> Result<Selection, String> {
    let path = config_path(env);
    let Some(text) = super::read_raw(&path)? else {
        return Ok(Selection::default());
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Selection::default());
    }
    let doc = trimmed
        .parse::<DocumentMut>()
        .map_err(|e| format!("{} 不是合法 TOML（{e}）", path.display()))?;
    // 可用清单的真值就是我们生成的那份 catalog：里面的 slug 就是 codex
    // 「Select Model」面板会列出来的东西。
    let allow = std::fs::read_to_string(path.parent().unwrap().join(CATALOG_FILE))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| {
            let slugs: Vec<String> = v
                .get("models")
                .and_then(|m| m.as_array())
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|e| e.get("slug").and_then(|s| s.as_str()))
                .map(str::to_string)
                .collect();
            (!slugs.is_empty()).then(|| slugs.into_iter().collect())
        });
    Ok(Selection {
        manage_tiers: false,
        default: doc
            .get("model")
            .and_then(Item::as_str)
            .map(str::to_string)
            .filter(|v| !v.trim().is_empty()),
        // 副模型槽：现场读 [agents].default_subagent_model（codex 官方 subagents 键）。
        codex_sub: doc
            .get("agents")
            .and_then(|item| item.as_table_like())
            .and_then(|t| t.get("default_subagent_model"))
            .and_then(|item| item.as_str())
            .map(str::to_string)
            .filter(|v| !v.trim().is_empty()),
        tiers: Default::default(),
        allow,
        ..Default::default()
    })
}

pub fn evaluate(env: &TakeoverEnv, ep: &Endpoint, sel: &Selection) -> Result<Evaluation, String> {
    let path = config_path(env);
    let raw = super::read_raw(&path)?;
    let exists = raw.is_some();
    let text = raw.clone().unwrap_or_default();
    let mut doc = if text.trim().is_empty() {
        DocumentMut::new()
    } else {
        text.parse::<DocumentMut>().map_err(|e| {
            format!(
                "{} 不是合法 TOML（{e}），已拒绝接管，请先修复",
                path.display()
            )
        })?
    };

    // 默认模型：顶层 `model`。codex 只认这一个地方，写别处它看不见。
    let model_is_current = match &sel.default {
        Some(want) => doc.get("model").and_then(Item::as_str) == Some(want.as_str()),
        None => true,
    };
    if !model_is_current {
        if let Some(want) = &sel.default {
            doc["model"] = value(want.clone());
        }
    }

    // 副模型槽：`[agents].default_subagent_model`（codex 官方 subagents 键）。
    // 只动我们这一个键——表里用户自己的键（enabled / max_depth …）原样保留；
    // 托管过又清了槽才移除：从没托管过时 None 不碰用户手写的值（同 manage_tiers 的边界）。
    let sub_now = doc
        .get("agents")
        .and_then(|item| item.as_table_like())
        .and_then(|t| t.get("default_subagent_model"))
        .and_then(|item| item.as_str());
    let sub_is_current = match (&sel.codex_sub, sel.manage_codex) {
        (Some(want), _) => sub_now == Some(want.as_str()),
        (None, false) => true,
        (None, true) => sub_now.is_none(),
    };
    if !sub_is_current {
        match &sel.codex_sub {
            Some(want) => {
                if doc.get("agents").is_none() {
                    doc["agents"] = Item::Table(Table::new());
                }
                let agents = doc
                    .get_mut("agents")
                    .and_then(|item| item.as_table_like_mut())
                    .ok_or_else(|| {
                        format!(
                            "{} 的 agents 不是表（内联写法也需可展开），已拒绝写入子代理槽",
                            path.display()
                        )
                    })?;
                agents.insert("default_subagent_model", value(want.clone()));
            }
            None => {
                if let Some(agents) = doc
                    .get_mut("agents")
                    .and_then(|item| item.as_table_like_mut())
                {
                    agents.remove("default_subagent_model");
                    // 我们清空后表空了就连表一起删，别在用户文件里留个空 [agents] 块
                    if agents.is_empty() {
                        doc.as_table_mut().remove("agents");
                    }
                }
            }
        }
    }

    let pruned = prune_dead_keys(&mut doc);

    // 模型 catalog：codex 查不到模型元信息就用兜底（实测警告 `fallback metadata`），
    // 上下文窗口猜错还会引发乱压缩。清单写**占槽 ∪ 显式清单**（∩暴露）——
    // 槽位制只约束主/子代理两个档位槽，catalog 允许多模型清单（BUG-4，用户
    // 2026-11-16）：格子点「加入清单」的模型也进 catalog，由用户在 codex 的
    // Select Model 面板里选择使用。网关暴露但没分给它的模型不进 catalog。
    // 没有模板（用户从没配过 catalog）就不生成——那些 base_instructions 之类的字段
    // 我们编不出来，宁缺不造。
    // 目录随 config 走（config 被改路径时 catalog 同侧），单独给 catalog 设的
    // 覆盖也在这里收口（config 未改时 catalog 的默认位置才对得上身份键）。
    let catalog_path = env.file(
        path.parent()
            .map(|dir| dir.join(CATALOG_FILE))
            .unwrap_or_else(|| path.clone()),
    );
    let listed = super::listed_models(&sel.assigned_models(), &ep.models);
    let rendered_cat =
        catalog_template(env, &doc).and_then(|t| render_catalog(&t, &listed, &ep.meta));
    let current_cat = std::fs::read_to_string(&catalog_path).unwrap_or_default();
    let catalog_fresh = match &rendered_cat {
        None => true,
        Some(want) => want == &current_cat,
    };
    if rendered_cat.is_some()
        && doc.get("model_catalog_json").and_then(Item::as_str) != Some(CATALOG_FILE)
    {
        doc["model_catalog_json"] = value(CATALOG_FILE.to_string());
    }

    let hide = hideable_provider_tables(&doc);
    let managed = provider_is_current(&doc, ep)
        && model_is_current
        && sub_is_current
        && !pruned
        && catalog_fresh
        && hide.is_empty();
    if !managed {
        doc["model_provider"] = value(PROVIDER_ID);
        if doc.get("model_providers").is_none() {
            doc["model_providers"] = Item::Table(Table::new());
        }
        let providers = doc
            .get_mut("model_providers")
            .and_then(|item| item.as_table_like_mut())
            .ok_or_else(|| {
                format!(
                    "{} 的 model_providers 不是表（内联写法也需可展开），已拒绝接管",
                    path.display()
                )
            })?;
        // 本地端点的别家 provider 摘走（原件在接管快照里，回滚还原）；在线 API 不动。
        if !hide.is_empty() {
            for id in &hide {
                providers.remove(id);
            }
        }
        if providers.get(PROVIDER_ID).is_none() {
            providers.insert(PROVIDER_ID, Item::Table(Table::new()));
        }
        let provider = providers
            .get_mut(PROVIDER_ID)
            .and_then(|item| item.as_table_like_mut())
            .ok_or_else(|| {
                format!(
                    "{} 的 [model_providers.{PROVIDER_ID}] 不是表，已拒绝接管",
                    path.display()
                )
            })?;
        for (key, val) in wanted_fields(ep) {
            provider.insert(key, value(val));
        }
    }

    let rendered = doc.to_string();
    let dirty = raw.as_deref() != Some(rendered.as_str());
    let note = if !exists {
        "文件不存在，接管时会创建（回滚时删除）".to_string()
    } else if managed {
        "已选中 llm-switch provider".to_string()
    } else {
        "写入 model_provider / 默认模型 / 子代理槽，并清理死键".to_string()
    };

    let mut files = vec![FileEval {
        path: path.display().to_string(),
        format: super::Format::Toml.as_str(),
        exists,
        managed,
        note,
        ..Default::default()
    }];
    let mut edits: Vec<Edit> = if dirty {
        vec![Edit {
            path: path.clone(),
            content: rendered,
        }]
    } else {
        Vec::new()
    };

    let (cat_exists, cat_note) = if rendered_cat.is_some() {
        (
            !current_cat.is_empty(),
            if catalog_fresh {
                "模型 catalog 与暴露清单一致"
            } else {
                "按暴露清单重写模型 catalog（上下文窗口沿用模板值）"
            },
        )
    } else {
        (
            catalog_path.exists(),
            "无模板可套用，未生成模型 catalog（codex 会用兜底元信息）",
        )
    };
    files.push(FileEval {
        path: catalog_path.display().to_string(),
        format: super::Format::Json.as_str(),
        exists: cat_exists,
        managed: catalog_fresh,
        note: cat_note.to_string(),
        ..Default::default()
    });
    if let Some(want) = rendered_cat {
        if !catalog_fresh {
            edits.push(Edit {
                path: catalog_path.clone(),
                content: want,
            });
        }
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
    fn keeps_user_comments_and_adds_provider() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(
            config_path(&e),
            "# 我自己的注释\nmodel = \"gpt-5.1\"\nnotify = [ \"echo hi\" ]\n",
        )
        .unwrap();

        let evaluation = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        let text = &evaluation.edits[0].content;
        assert!(text.contains("# 我自己的注释"), "{text}");
        assert!(text.contains("notify"), "{text}");
        assert!(text.contains("model_provider = \"llm-switch\""), "{text}");
        assert!(text.contains("[model_providers.llm-switch]"), "{text}");
        assert!(
            text.contains("base_url = \"http://127.0.0.1:8787/v1\""),
            "{text}"
        );
    }

    #[test]
    fn hides_local_provider_tables_and_keeps_online_ones() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(
            config_path(&e),
            "[model_providers.local-lab]\nname = \"local-lab\"\nbase_url = \"http://192.168.1.9:8000/v1\"\n\n[model_providers.online]\nname = \"online\"\nbase_url = \"https://api.openai.com/v1\"\n",
        )
        .unwrap();

        let evaluation = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        let text = &evaluation.edits[0].content;
        assert!(
            !text.contains("local-lab"),
            "本地端点 provider 该摘走: {text}"
        );
        assert!(text.contains("api.openai.com"), "在线 API 不得动: {text}");
        assert!(text.contains("[model_providers.llm-switch]"), "{text}");

        atomic_write_private(&evaluation.edits[0].path, &evaluation.edits[0].content).unwrap();
        let again = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        assert!(again.edits.is_empty(), "摘干净之后必须幂等，不该每次重写");
    }

    #[test]
    fn idempotent_second_pass() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(config_path(&e), "model = \"gpt-5.1\"\n").unwrap();

        let first = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        atomic_write_private(&first.edits[0].path, &first.edits[0].content).unwrap();
        let second = evaluate(&e, &ep(8787), &Selection::default()).unwrap();
        assert!(second.edits.is_empty(), "第二次不该再有差异");
        assert!(second.managed());
    }

    #[test]
    fn refuses_broken_toml() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(config_path(&e), "model_provider = \n").unwrap();
        assert!(evaluate(&e, &ep(8787), &Selection::default()).is_err());
    }

    #[test]
    fn catalog_context_window_prefers_truth_falls_back_to_template() {
        use std::collections::BTreeMap;
        let template = serde_json::json!({
            "models": [{
                "slug": "tpl",
                "display_name": "tpl",
                "description": "d",
                "context_window": 200000,
                "max_context_window": 200000,
                "effective_context_window_percent": 95,
                "base_instructions": "keep me"
            }]
        })["models"][0]
            .clone();

        // 真值已知：context_window / max_context_window 成对覆写，窗口留白一并放开。
        let mut meta: BTreeMap<String, crate::types::EffectiveMeta> = BTreeMap::new();
        meta.insert(
            "m-a".into(),
            crate::types::EffectiveMeta {
                ctx: Some(131_072),
                ..Default::default()
            },
        );
        let out = render_catalog(&template, &["m-a".into(), "m-b".into()], &meta).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
        let entries = doc["models"].as_array().unwrap();
        assert_eq!(entries[0]["slug"], "m-a");
        assert_eq!(entries[0]["context_window"], 131072);
        assert_eq!(entries[0]["max_context_window"], 131072);
        assert_eq!(entries[0]["base_instructions"], "keep me");
        // 引擎声明的是硬上限，模板那份 95% 留白不再砍窗口。
        assert_eq!(entries[0]["effective_context_window_percent"], 100);
        // 未知的模型沿用模板值（不猜）。
        assert_eq!(entries[1]["slug"], "m-b");
        assert_eq!(entries[1]["context_window"], 200000);
        assert_eq!(entries[1]["effective_context_window_percent"], 95);
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
            codex_wire_api: "responses".into(),
        }
    }

    #[test]
    fn writes_top_level_model_and_reads_it_back() {
        let tmp = TempDir::new().unwrap();
        let e = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"));
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(config_path(&e), "model = \"old\"\nnotify = [ \"echo\" ]\n").unwrap();

        let sel = Selection {
            default: Some("qwen-mini".into()),
            tiers: Default::default(),
            ..Default::default()
        };
        let ev = evaluate(&e, &ep(), &sel).unwrap();
        atomic_write_private(&ev.edits[0].path, &ev.edits[0].content).unwrap();
        let text = std::fs::read_to_string(config_path(&e)).unwrap();
        assert!(text.contains("model = \"qwen-mini\""), "{text}");
        // 别人的键与注释都得留着（toml_edit 原位改，不是整份重排）
        assert!(text.contains("notify"), "{text}");
        assert_eq!(
            read_selection(&e).unwrap().default.as_deref(),
            Some("qwen-mini")
        );
    }

    /// 副槽（子代理模型）：写进 [agents] default_subagent_model，读得回、二次评估幂等。
    #[test]
    fn sub_slot_writes_agents_table_and_reads_back() {
        let tmp = TempDir::new().unwrap();
        let e = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"));
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(config_path(&e), "model = \"main-m\"\n").unwrap();

        let sel = Selection {
            default: Some("main-m".into()),
            codex_sub: Some("sub-m".into()),
            manage_codex: true,
            ..Default::default()
        };
        let ev = evaluate(&e, &ep(), &sel).unwrap();
        atomic_write_private(&ev.edits[0].path, &ev.edits[0].content).unwrap();
        let text = std::fs::read_to_string(config_path(&e)).unwrap();
        assert!(
            text.contains("default_subagent_model = \"sub-m\""),
            "{text}"
        );

        let back = read_selection(&e).unwrap();
        assert_eq!(back.codex_sub.as_deref(), Some("sub-m"));
        assert_eq!(back.default.as_deref(), Some("main-m"));

        // 幂等：同一份期望再评估一次不该再有差异，且状态是「已托管」
        let again = evaluate(&e, &ep(), &sel).unwrap();
        assert!(again.edits.is_empty(), "{:?}", again.edits);
        assert!(again.managed());
    }

    /// 清副槽只动我们自己的键：托管过才移除，用户的 [agents] 其余键原样保留；
    /// 从没托管过（manage_codex=false）时，用户手写的副槽不碰。
    #[test]
    fn sub_slot_clear_keeps_foreign_agents_keys() {
        let tmp = TempDir::new().unwrap();
        let e = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"));
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(
            config_path(&e),
            "model = \"m\"\n\n[agents]\nenabled = true\ndefault_subagent_model = \"hand-written\"\n",
        )
        .unwrap();

        // 没托管过：None 不碰手写的副槽（即便这次评估顺手写 provider，
        // 副槽键也得原样留在结果里）
        let untouched = evaluate(&e, &ep(), &Selection::default()).unwrap();
        let text = untouched
            .edits
            .first()
            .map(|ed| ed.content.clone())
            .unwrap_or_else(|| std::fs::read_to_string(config_path(&e)).unwrap());
        assert!(
            text.contains("default_subagent_model = \"hand-written\""),
            "{text}"
        );

        // 托管过且矩阵清了槽：只摘 default_subagent_model，enabled 与 [agents] 表都留着
        let clearing = Selection {
            manage_codex: true,
            ..Default::default()
        };
        let ev = evaluate(&e, &ep(), &clearing).unwrap();
        atomic_write_private(&ev.edits[0].path, &ev.edits[0].content).unwrap();
        let text = std::fs::read_to_string(config_path(&e)).unwrap();
        assert!(!text.contains("default_subagent_model"), "{text}");
        assert!(text.contains("enabled = true"), "{text}");
        assert!(text.contains("[agents]"), "{text}");
        assert_eq!(read_selection(&e).unwrap().codex_sub, None);
    }

    /// 清副槽后 [agents] 空了就连表一起删，不留空块。
    #[test]
    fn sub_slot_clear_removes_emptied_agents_table() {
        let tmp = TempDir::new().unwrap();
        let e = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"));
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(
            config_path(&e),
            "model = \"m\"\n\n[agents]\ndefault_subagent_model = \"sub-m\"\n",
        )
        .unwrap();

        let clearing = Selection {
            manage_codex: true,
            ..Default::default()
        };
        let ev = evaluate(&e, &ep(), &clearing).unwrap();
        atomic_write_private(&ev.edits[0].path, &ev.edits[0].content).unwrap();
        let text = std::fs::read_to_string(config_path(&e)).unwrap();
        assert!(!text.contains("[agents]"), "{text}");
        assert!(!text.contains("default_subagent_model"), "{text}");
        assert!(text.contains("model = \"m\""), "{text}");
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use crate::takeover::{atomic_write_private, Selection};
    use tempfile::TempDir;

    fn ep() -> Endpoint {
        Endpoint {
            meta: Default::default(),
            base_root: "http://127.0.0.1:8787".into(),
            api_root: "http://127.0.0.1:8787/v1".into(),
            token: "tk".into(),
            models: vec!["m-one".into(), "m-two".into()],
            codex_wire_api: "responses".into(),
        }
    }

    fn env(tmp: &TempDir) -> TakeoverEnv {
        TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("b"))
    }

    #[test]
    fn dead_litellm_keys_are_pruned_but_real_ones_are_not() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(
            config_path(&e),
            "base_url = \"http://localhost:4000/v1\"\nwire_api = \"responses\"\nmodel = \"x\"\n",
        )
        .unwrap();
        let ev = evaluate(&e, &ep(), &Selection::default()).unwrap();
        // 只看顶层：provider 表里的 base_url / wire_api 是真字段，不能碰
        let doc = ev.edits[0].content.parse::<DocumentMut>().unwrap();
        assert!(doc.get("base_url").is_none(), "{}", ev.edits[0].content);
        assert!(doc.get("wire_api").is_none(), "{}", ev.edits[0].content);
        assert!(doc.get("model").is_some(), "{}", ev.edits[0].content);
        let provider = doc
            .get("model_providers")
            .and_then(|i| i.as_table_like())
            .and_then(|t| t.get(PROVIDER_ID))
            .and_then(|i| i.as_table_like())
            .expect("provider 表还在");
        assert!(
            provider.get("wire_api").is_some(),
            "provider 的 wire_api 不该动"
        );

        // 别的地址不动它：那不是我们认定的垃圾
        std::fs::write(config_path(&e), "base_url = \"http://elsewhere:9/v1\"\n").unwrap();
        let ev = evaluate(&e, &ep(), &Selection::default()).unwrap();
        let doc = ev.edits[0].content.parse::<DocumentMut>().unwrap();
        assert_eq!(
            doc.get("base_url").and_then(Item::as_str),
            Some("http://elsewhere:9/v1")
        );
        assert!(
            ev.edits[0].content.contains("elsewhere:9"),
            "{}",
            ev.edits[0].content
        );
    }

    #[test]
    fn catalog_is_generated_from_template_and_points_at_it() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(
            config_path(&e),
            "model = \"x\"\nmodel_catalog_json = \"old-catalog.json\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".codex/old-catalog.json"),
            r#"{"models":[{"slug":"old","display_name":"Old","description":"d","context_window":200000,"base_instructions":"keep me"}]}"#,
        )
        .unwrap();

        // 清单（catalog）只写**占了槽**的模型：主槽 + 副槽，正好两个。
        let sel = Selection {
            default: Some("m-one".into()),
            codex_sub: Some("m-two".into()),
            ..Selection::default()
        };
        let ev = evaluate(&e, &ep(), &sel).unwrap();
        // 两条编辑：config.toml 指向我们的 catalog + 生成 catalog 本体
        assert_eq!(ev.edits.len(), 2, "{:?}", ev.edits.len());
        let cfg = &ev.edits[0].content;
        assert!(
            cfg.contains(&format!("model_catalog_json = \"{CATALOG_FILE}\"")),
            "{cfg}"
        );
        let cat: serde_json::Value = serde_json::from_str(&ev.edits[1].content).unwrap();
        let models = cat["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], "m-one");
        // 模板里编不出来的字段原样保留，不自己瞎编
        assert_eq!(models[0]["base_instructions"], "keep me");
        assert_eq!(models[0]["context_window"], 200000);

        for edit in &ev.edits {
            atomic_write_private(&edit.path, &edit.content).unwrap();
        }
        assert!(evaluate(&e, &ep(), &sel).unwrap().managed());
    }

    #[test]
    fn no_template_means_no_fake_catalog() {
        let tmp = TempDir::new().unwrap();
        let e = env(&tmp);
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(config_path(&e), "model = \"x\"\n").unwrap();
        let ev = evaluate(&e, &ep(), &Selection::default()).unwrap();
        assert_eq!(ev.edits.len(), 1, "只该写 config.toml，不该凭空造 catalog");
        assert!(
            ev.files[1].note.contains("无模板"),
            "{:?}",
            ev.files[1].note
        );
    }
}
