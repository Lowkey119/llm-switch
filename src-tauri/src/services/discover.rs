//! 只读探测：把某个后端「到底有哪些模型」捞出来，省得用户手敲每个模型 id。
//!
//! 设计约束（M9 后与用户对齐）：
//! - **纯只读**，绝不落库；勾选与保存是前端的事，这里只负责「探」。
//! - 候选端点依次降级，第一个拿到非空列表的就收工（各家引擎习惯不同）。
//! - 解析尽量宽容（`data` / 顶层数组 / `models` / `items` 四种外壳都认），
//!   但拿不到就明确报错，不猜。

use crate::gateway::backends::adapter_for;
use http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 一次探测最多返回多少个模型（防御异常大的上游列表把前端列表撑爆）。
const MAX_MODELS: usize = 500;

#[derive(Debug, Clone, Deserialize)]
pub struct DiscoverInput {
    pub base_url: String,
    /// `openai_chat` | `anthropic` | `openai_responses` | `gemini`，空按 openai_chat 试。
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelCandidate {
    pub id: String,
    /// 上游愿意给的友好名（Gemini 的 `displayName`），没有就 None。
    pub display: Option<String>,
    /// 上游顺带给的上下文长度（vLLM `max_model_len`、Gemini `inputTokenLimit` 等）。
    /// 没给 = None：未知不猜（`skip_serializing_if` 让「没给」在响应里彻底缺席）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx: Option<i64>,
    /// 上游顺带给的最大输出 token（`max_output_tokens` / `outputTokenLimit`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<i64>,
    /// 是否思考模型（上游明说才有值：litellm `supports_reasoning`、ollama `thinking`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    /// 是否有视觉输入（VLM；litellm `supports_vision`、ollama `vision` 能力位）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlm: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Discovery {
    /// 命中的是哪一路：`openai` | `ollama` | `gemini`。
    pub source: String,
    /// 实际打到哪个 URL，方便排错。
    pub url: String,
    pub models: Vec<ModelCandidate>,
}

/// (来源标签, 路径)。顺序即尝试顺序。
pub fn candidates() -> [(&'static str, &'static str); 4] {
    [
        ("openai", "/v1/models"), // vLLM / SGLang / llama.cpp / LM Studio / oMLX / Anthropic 官方
        ("openai", "/models"),    // base_url 已经带 /v1 的那些
        ("ollama", "/api/tags"),  // Ollama 不走 OpenAI 形状
        ("gemini", "/v1beta/models"),
    ]
}

/// 规整 base_url：去尾斜杠、校验协议。返回干净 base 或人话错误。
fn normalize_base(raw: &str) -> Result<String, String> {
    let base = raw.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err("需要先填 Base URL".to_string());
    }
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(format!(
            "Base URL 要以 http:// 或 https:// 开头（现在是 `{base}`）"
        ));
    }
    Ok(base)
}

/// 从列表条目里捞上下文/输出真值。认各家惯用键，没有就是没有（不猜）。
fn item_limits(item: &Value) -> (Option<i64>, Option<i64>) {
    // 上下文：vLLM/SGLang `max_model_len`、OpenAI 生态 `max_input_tokens`、
    // 一些网关 `context_length`/`context_window`、Gemini `inputTokenLimit`。
    let ctx = [
        "max_model_len",
        "max_input_tokens",
        "context_length",
        "context_window",
        "inputTokenLimit",
    ]
    .iter()
    .find_map(|k| item.get(*k).and_then(Value::as_i64));
    // 输出：`max_output_tokens` / `outputTokenLimit`；halogen 声明的是
    // `max_tokens_cap`（硬上限）+ `max_tokens_default`（默认值）——取 cap，
    // 与 ctx 侧取 max_model_len（上限）同一口径。
    let output = ["max_output_tokens", "outputTokenLimit", "max_tokens_cap"]
        .iter()
        .find_map(|k| item.get(*k).and_then(Value::as_i64));
    (ctx.filter(|v| *v > 0), output.filter(|v| *v > 0))
}

/// 从列表条目里捐 reasoning/vlm 能力位。只认「明确说了」的形状（布尔键或
/// 能力数组命中）；没说是 None——能力位缺席不等于否定，不猜。
fn item_bools(item: &Value) -> (Option<bool>, Option<bool>) {
    let flag = |keys: [&str; 2]| {
        keys.iter()
            .find_map(|k| item.get(*k).and_then(Value::as_bool))
    };
    let mut reasoning = flag(["supports_reasoning", "reasoning"]);
    let mut vlm = flag(["supports_vision", "supports_image_input"]);
    // 模态/能力数组形态：`input_modalities:["text","image"]`、`capabilities:["vision"]`。
    let arr_hit = |key: &str, words: &[&str]| {
        item.get(key).and_then(Value::as_array).is_some_and(|arr| {
            arr.iter()
                .any(|v| v.as_str().is_some_and(|s| words.contains(&s)))
        })
    };
    if vlm.is_none()
        && ["input_modalities", "modalities", "capabilities"]
            .iter()
            .any(|k| arr_hit(k, &["image", "vision"]))
    {
        vlm = Some(true);
    }
    if reasoning.is_none()
        && ["capabilities", "input_modalities"]
            .iter()
            .any(|k| arr_hit(k, &["thinking", "reasoning"]))
    {
        reasoning = Some(true);
    }
    (reasoning, vlm)
}

/// 从一个 JSON 响应体里尽力捞出模型数组。认四种外壳，元素认 `id`/`name`/`model`。
fn parse_models(body: &Value) -> Vec<ModelCandidate> {
    let arrays: [&str; 4] = ["data", "models", "items", ""];
    let mut picked: Option<&Vec<Value>> = None;
    for key in arrays {
        let arr = if key.is_empty() {
            body.as_array()
        } else {
            body.get(key).and_then(Value::as_array)
        };
        if let Some(arr) = arr {
            if !arr.is_empty() {
                picked = Some(arr);
                break;
            }
        }
    }
    let Some(arr) = picked else {
        return Vec::new();
    };

    let mut out: Vec<ModelCandidate> = Vec::new();
    for item in arr {
        let raw = ["id", "name", "model"]
            .iter()
            .filter_map(|k| item.get(*k).and_then(Value::as_str))
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if raw.is_empty() {
            continue;
        }
        // Gemini 的 name 形如 `models/gemini-2.5-pro`，客户端要说的是后半截。
        let id = raw.strip_prefix("models/").unwrap_or(&raw).to_string();
        let display = item
            .get("displayName")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|d| !d.trim().is_empty() && *d != id);
        if out.iter().any(|m| m.id == id) {
            continue;
        }
        let (ctx, output) = item_limits(item);
        let (reasoning, vlm) = item_bools(item);
        out.push(ModelCandidate {
            id,
            display,
            ctx,
            output,
            reasoning,
            vlm,
        });
        if out.len() >= MAX_MODELS {
            break;
        }
    }
    out
}

/// 把 `/model/info`（LiteLLM / OpenAI 生态的扩展端点）里的真值并进候选清单。
///
/// 为什么单开这一枪：**根因现场就是 LiteLLM——它的 `/v1/models` 不带 ctx，
/// 真值只在 `/model/info`**（`max_input_tokens`/`max_output_tokens`，有的包在
/// `model_info` 里）。只补缺口（已有值不覆盖），端点不存在/超时就静默放弃。
fn merge_model_info(body: &Value, models: &mut [ModelCandidate]) {
    let Some(arr) = body.get("data").and_then(Value::as_array) else {
        return;
    };
    for item in arr {
        // LiteLLM：`model_name` 在顶层，token 真值可能在顶层也可能在 `model_info` 里。
        let id = ["model_name", "id"]
            .iter()
            .filter_map(|k| item.get(*k).and_then(Value::as_str))
            .next();
        let Some(id) = id else { continue };
        let id = id.strip_prefix("models/").unwrap_or(id);
        let (ctx, output) = item_limits(item);
        let (ctx2, output2) = item_limits(item.get("model_info").unwrap_or(&Value::Null));
        let ctx = ctx.or(ctx2);
        let output = output.or(output2);
        // 能力位：顶层和 model_info 两层都认（litellm 两家摆放位置都见过）。
        let (re1, vl1) = item_bools(item);
        let (re2, vl2) = item_bools(item.get("model_info").unwrap_or(&Value::Null));
        let reasoning = re1.or(re2);
        let vlm = vl1.or(vl2);
        if ctx.is_none() && output.is_none() && reasoning.is_none() && vlm.is_none() {
            continue;
        }
        if let Some(slot) = models.iter_mut().find(|m| m.id == id) {
            slot.ctx = slot.ctx.or(ctx);
            slot.output = slot.output.or(output);
            slot.reasoning = slot.reasoning.or(reasoning);
            slot.vlm = slot.vlm.or(vlm);
        }
    }
}

/// 取 base 的站点根（`scheme://host[:port]`）：各家引擎的私有元数据端点挂在
/// 站点根而非 `/v1` 下（base_url 常常带着 `/v1`，直接拼会拼出 404）。
/// 探活/扫描的报错也用它 —— 连接级失败（端口没开、DNS 不通）报的是**站点根**，
/// 不再是某个具体端点的半截 URL。
pub(crate) fn origin(base: &str) -> Option<String> {
    let (scheme, rest) = base.split_once("://")?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

/// reqwest 错误 → 一句人话。
///
/// 为什么要自己转译：`e.to_string()` 长这样——
/// `error sending request for url (http://localhost:8000/v1/models)`
/// 旧代码在 60 字处硬截，正好把 URL 切成 `http://localhost:8000/v1/mode` 这种半截，
/// 既读不懂也搜不到（实报：后端行里挂着这么一串天书，没人看得出是端口没开）。
/// 这里按错误类型给结论，URL 由调用方按站点根给完整的一份。
pub(crate) fn explain(e: &reqwest::Error) -> String {
    if e.is_connect() {
        return "TCP 连不上（端口没在监听 / 服务未启动）".to_string();
    }
    if e.is_timeout() {
        return "超时（对端没在约定时间内应答）".to_string();
    }
    if e.is_decode() {
        return "响应解不开（不是预期的 JSON）".to_string();
    }
    if e.is_body() {
        return "响应读到一半被对端断开".to_string();
    }
    // 兜底：去掉 `for url (...)` 尾巴，只留短因，别把整条 URL 拼进来。
    let raw = e.to_string();
    let head = raw.split(" for url (").next().unwrap_or(&raw).trim();
    head.chars().take(60).collect()
}

/// 一次深探最多追多少个模型（ollama /api/show 是按模型 POST，防异常大列表拖死扫描）。
const ENRICH_MAX: usize = 64;

/// ollama：`POST /api/show` 逐模型补默认参数真值。
///
/// 为什么值得追：ollama 的 `/api/tags` 只给名字，ctx 真值和 vision/thinking 能力位
/// 全在 GGUF 元数据里（`model_info.*.context_length` / `capabilities`）。只补缺口。
async fn enrich_ollama(client: &reqwest::Client, base: &str, models: &mut [ModelCandidate]) {
    let Some(root) = origin(base) else { return };
    let url = format!("{root}/api/show");
    for m in models.iter_mut().take(ENRICH_MAX) {
        let Ok(resp) = client
            .post(&url)
            .json(&serde_json::json!({ "name": m.id }))
            .send()
            .await
        else {
            continue;
        };
        if resp.status().as_u16() >= 400 {
            continue;
        }
        let Ok(v) = resp.json::<Value>().await else {
            continue;
        };
        if m.ctx.is_none() {
            if let Some(mi) = v.get("model_info").and_then(Value::as_object) {
                m.ctx = mi.iter().find_map(|(k, val)| {
                    (k.ends_with(".context_length") || k.ends_with(".num_ctx"))
                        .then(|| val.as_i64())
                        .flatten()
                        .filter(|n| *n > 0)
                });
            }
        }
        // 新版 ollama 把能力位平铺在 `capabilities` 数组里（vision/thinking）。
        let (re, vl) = item_bools(&v);
        m.reasoning = m.reasoning.or(re);
        m.vlm = m.vlm.or(vl);
    }
}

/// llama.cpp server：`GET /props` 带已加载模型的 ctx。只在能对上手时采用：
/// 别名/模型文件名能对住某个候选，或全场只有一个候选（单模型部署的常态）。
async fn enrich_llama_cpp(client: &reqwest::Client, base: &str, models: &mut [ModelCandidate]) {
    let Some(root) = origin(base) else { return };
    let url = format!("{root}/props");
    let Ok(resp) = client.get(&url).send().await else {
        return;
    };
    if resp.status().as_u16() >= 400 {
        return;
    }
    let Ok(v) = resp.json::<Value>().await else {
        return;
    };
    let ctx = ["n_ctx", "context_length"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_i64))
        .or_else(|| {
            v.get("default_generation_parameters")
                .and_then(|p| p.get("n_ctx"))
                .and_then(Value::as_i64)
        })
        .filter(|n| *n > 0);
    let Some(ctx) = ctx else { return };
    let alias = ["model_alias", "model_path"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .unwrap_or_default();
    let stem = std::path::Path::new(alias)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(alias)
        .to_ascii_lowercase();
    let applies = |m: &ModelCandidate| {
        !stem.is_empty()
            && (m.id.to_ascii_lowercase() == stem || m.id.to_ascii_lowercase().contains(&stem))
    };
    let hits = models.iter().filter(|m| applies(m)).count();
    let solo = models.len() == 1;
    for m in models.iter_mut() {
        if m.ctx.is_none() && (hits > 0 && applies(m) || solo) {
            m.ctx = Some(ctx);
        }
    }
}

/// LM Studio：`GET /api/v0/models` 带 context 与能力位（OpenAI 兼容层的 `/v1/models`
/// 不带这些）。只补缺口，形状不认识就静默放弃。
async fn enrich_lmstudio(client: &reqwest::Client, base: &str, models: &mut [ModelCandidate]) {
    let Some(root) = origin(base) else { return };
    let url = format!("{root}/api/v0/models");
    let Ok(resp) = client.get(&url).send().await else {
        return;
    };
    if resp.status().as_u16() >= 400 {
        return;
    }
    let Ok(v) = resp.json::<Value>().await else {
        return;
    };
    let Some(arr) = v.get("data").and_then(Value::as_array) else {
        return;
    };
    for item in arr {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(slot) = models.iter_mut().find(|m| m.id == id) else {
            continue;
        };
        let (ctx, output) = item_limits(item);
        let (re, vl) = item_bools(item);
        slot.ctx = slot.ctx.or(ctx);
        slot.output = slot.output.or(output);
        slot.reasoning = slot.reasoning.or(re);
        slot.vlm = slot.vlm.or(vl);
    }
}

/// 探一把。Ok 里保证 `models` 非空。
pub async fn discover(input: &DiscoverInput) -> Result<Discovery, String> {
    let base = normalize_base(&input.base_url)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    let mut headers = HeaderMap::new();
    if let Some(adapter) = adapter_for(&input.protocol) {
        let pairs = adapter
            .auth_headers(input.api_key.as_deref())
            .map_err(|e| e.to_string())?;
        for (k, v) in pairs {
            headers.insert(k, v);
        }
    } else if let Some(key) = input.api_key.as_deref().filter(|k| !k.is_empty()) {
        // 没适配器的协议也别裸奔，至少按 Bearer 试。
        if let Ok(v) = key.parse() {
            headers.insert(http::header::AUTHORIZATION, v);
        }
    }

    let site = origin(&base).unwrap_or_else(|| base.clone());
    let mut reasons: Vec<String> = Vec::new();
    for (source, path) in candidates() {
        let url = crate::gateway::backends::adapter::join_path(&base, path);
        let resp = match client
            .get(&url)
            .headers(headers.clone())
            .header(http::header::ACCEPT, "application/json")
            .send()
            .await
        {
            Ok(r) => r,
            // 连接级失败（端口没开 / DNS 不通 / 对端没应答）：四个候选端点打的是同一个
            // origin，再挨个试只是把同一条错误抄四遍、把最多 5s 的超时 ×4 白等掉。
            // 直接按站点根报一条结论，结束扫描。
            Err(e) if e.is_connect() || e.is_timeout() => {
                reasons.push(format!("连不上 {site}（{}）", explain(&e)));
                break;
            }
            Err(e) => {
                reasons.push(format!("{path} 打不开（{}）", explain(&e)));
                continue;
            }
        };
        let status = resp.status().as_u16();
        if status >= 400 {
            reasons.push(format!("{path} 返回 {status}"));
            continue;
        }
        let body: Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => {
                reasons.push(format!("{path} 返回的不是 JSON"));
                continue;
            }
        };
        let mut models = parse_models(&body);
        if models.is_empty() {
            reasons.push(format!("{path} 没解析出模型"));
            continue;
        }
        if source == "openai" {
            // LiteLLM/OpenAI 生态增强：`/v1/models` 常不带 ctx，真值在 `/model/info`。
            // 尽力而为——404/超时/怪形状都静默放弃，绝不拖垮扫描主流程。
            let info_url = crate::gateway::backends::adapter::join_path(&base, "/model/info");
            if let Ok(resp) = client
                .get(&info_url)
                .headers(headers.clone())
                .header(http::header::ACCEPT, "application/json")
                .send()
                .await
            {
                if resp.status().as_u16() < 400 {
                    if let Ok(v) = resp.json::<Value>().await {
                        merge_model_info(&v, &mut models);
                    }
                }
            }
            // 引擎私有端点深探（都是尽力而为，探不到不影响主流程）：
            // llama.cpp `/props`、LM Studio `/api/v0/models`。
            enrich_llama_cpp(&client, &base, &mut models).await;
            enrich_lmstudio(&client, &base, &mut models).await;
        } else if source == "ollama" {
            enrich_ollama(&client, &base, &mut models).await;
        }
        return Ok(Discovery {
            source: source.to_string(),
            url,
            models,
        });
    }

    // 单条结论（连接级失败）就别再套「N 个候选端点」的壳——那层壳只会让用户
    // 以为是模型列表格式的问题，其实是端口没开。
    if reasons.len() == 1 {
        return Err(reasons.remove(0));
    }
    Err(format!(
        "{} 个候选端点都没拿到模型列表：{}",
        reasons.len(),
        reasons.join("；")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ids(v: &Value) -> Vec<String> {
        parse_models(v).into_iter().map(|m| m.id).collect()
    }

    #[test]
    fn openai_shape() {
        assert_eq!(
            ids(&json!({"data":[{"id":"qwen3-30b"},{"id":"glm-4"}]})),
            vec!["qwen3-30b", "glm-4"]
        );
    }

    #[test]
    fn bare_array_and_alt_keys() {
        assert_eq!(
            ids(&json!([{"id":"a"},{"name":"b"},{"model":"c"}])),
            vec!["a", "b", "c"]
        );
        assert_eq!(ids(&json!({"items":[{"id":"x"}]})), vec!["x"]);
    }

    #[test]
    fn ollama_shape_uses_name() {
        assert_eq!(
            ids(&json!({"models":[{"name":"qwen3:30b","size":1},{"name":"llama3:8b"}]})),
            vec!["qwen3:30b", "llama3:8b"]
        );
    }

    #[test]
    fn gemini_strips_models_prefix_and_keeps_display() {
        let got = parse_models(&json!({"models":[
            {"name":"models/gemini-2.5-pro","displayName":"Gemini 2.5 Pro"}
        ]}));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "gemini-2.5-pro");
        assert_eq!(got[0].display.as_deref(), Some("Gemini 2.5 Pro"));
    }

    #[test]
    fn junk_is_ignored_not_fatal() {
        // 空数组不算命中（继续往下试），缺 id 的元素跳过，重复 id 去重。
        assert!(parse_models(&json!({"data": []})).is_empty());
        assert_eq!(
            ids(&json!({"data":[{"foo":1},{"id":"a"},{"id":"a"}]})),
            vec!["a"]
        );
        assert!(parse_models(&json!("hello")).is_empty());
    }

    #[test]
    fn bool_flags_recognize_declared_shapes_only() {
        let parsed = parse_models(&json!({"data":[
            {"id":"a","supports_vision":true,"supports_reasoning":true},
            {"id":"b","capabilities":["completion","vision"]},
            {"id":"c","input_modalities":["text"]},
            {"id":"d","supports_vision":false},
            {"id":"e"}
        ]}));
        let get = |id: &str| parsed.iter().find(|m| m.id == id).unwrap();
        assert_eq!(get("a").vlm, Some(true));
        assert_eq!(get("a").reasoning, Some(true));
        assert_eq!(get("b").vlm, Some(true), "ollama 风能力数组");
        assert_eq!(get("b").reasoning, None);
        assert_eq!(get("c").vlm, None, "模态列表没提 image = 未知，不是否定");
        assert_eq!(get("d").vlm, Some(false), "明确说不支持 = 假，不是未知");
        assert_eq!(get("e").vlm, None);
        assert_eq!(get("e").reasoning, None);
    }

    #[test]
    fn model_info_merge_fills_bool_gaps_too() {
        let mut models = vec![
            ModelCandidate {
                id: "a".into(),
                display: None,
                ctx: None,
                output: None,
                reasoning: None,
                vlm: None,
            },
            ModelCandidate {
                id: "b".into(),
                display: None,
                ctx: None,
                output: None,
                reasoning: Some(true),
                vlm: None,
            },
        ];
        merge_model_info(
            &json!({"data":[
                {"model_name":"a","model_info":{"supports_vision":true,"supports_reasoning":false}},
                {"model_name":"b","model_info":{"supports_vision":false}}
            ]}),
            &mut models,
        );
        assert_eq!(models[0].vlm, Some(true));
        assert_eq!(
            models[0].reasoning,
            Some(false),
            "false 也是真值，要能合进来"
        );
        assert_eq!(models[1].vlm, Some(false));
        assert_eq!(models[1].reasoning, Some(true), "已有的不被覆盖");
    }

    #[test]
    fn base_url_is_cleaned_and_checked() {
        assert_eq!(
            normalize_base(" http://a.b:8000/ ").unwrap(),
            "http://a.b:8000"
        );
        assert!(normalize_base("").is_err());
        assert!(normalize_base("127.0.0.1:8000").is_err());
    }

    #[test]
    fn ctx_output_fields_are_parsed_when_present() {
        // vLLM 形状
        let vllm = parse_models(&json!({"data":[
            {"id":"m","max_model_len":131072,"max_output_tokens":8192}
        ]}));
        assert_eq!(vllm[0].ctx, Some(131_072));
        assert_eq!(vllm[0].output, Some(8192));
        // Gemini 形状
        let gem = parse_models(&json!({"models":[
            {"name":"models/g","inputTokenLimit":1048576,"outputTokenLimit":65536}
        ]}));
        assert_eq!(gem[0].ctx, Some(1_048_576));
        assert_eq!(gem[0].output, Some(65_536));
        // halogen 形状：max_tokens_cap 是输出硬上限（根因现场：曾被漏认 → pi 兜底 16384）
        let hal = parse_models(&json!({"data":[
            {"id":"m","context_length":262144,"max_tokens_cap":65536,"max_tokens_default":8192}
        ]}));
        assert_eq!(hal[0].ctx, Some(262_144));
        assert_eq!(hal[0].output, Some(65_536));
        // Ollama / 列表里没有这些键 = 未知（None，不猜）
        let plain = parse_models(&json!({"data":[{"id":"m"}]}));
        assert_eq!(plain[0].ctx, None);
        assert_eq!(plain[0].output, None);
        // 0 / 负数不算真值
        let zero = parse_models(&json!({"data":[{"id":"m","max_model_len":0}]}));
        assert_eq!(zero[0].ctx, None);
    }

    #[test]
    fn model_info_merge_fills_gaps_without_overwriting() {
        // LiteLLM 根因现场：/v1/models 不带 ctx，真值只在 /model/info。
        let mut models = parse_models(&json!({"data":[
            {"id":"a"}, {"id":"b","max_model_len":32000}
        ]}));
        merge_model_info(
            &json!({"data":[
                {"model_name":"a","max_input_tokens":200000,"max_output_tokens":32768},
                {"model_name":"b","model_info":{"max_input_tokens":1111}},
                {"model_name":"not-in-list","max_input_tokens":42}
            ]}),
            &mut models,
        );
        // 补缺口
        assert_eq!(models[0].ctx, Some(200_000));
        assert_eq!(models[0].output, Some(32_768));
        // 已有值不被覆盖（嵌套 model_info 的字段也认）
        assert_eq!(models[1].ctx, Some(32_000));
        // 清单外的 id 不乱加
        assert_eq!(models.len(), 2);
    }
}
