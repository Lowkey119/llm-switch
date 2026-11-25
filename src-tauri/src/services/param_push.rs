//! 参数下发（参数中心 → 后端本体）。
//!
//! **为什么需要它**：`gateway::params` 的注入只作用于**经过网关的请求体**，而且
//! 请求层绝对最高（客户端自己带了 `temperature` 就永远压不过它）。后端自己的模型
//! 配置一个字节都没被动过——用户在 Unsloth / oMLX 自己的控制台里看到的仍是引擎
//! 默认，参数中心看起来就像"改了没作用"。本模块补的正是「写回后端」这条路；
//! 请求注入保留为兜底（没有下发通道的后端只剩那一条路）。
//!
//! **分派轴是 engine**（各家配置方法不同），不是 OS：
//! - `unsloth`：Studio 的模型级采样默认值 `PUT /api/settings/model-sampling-defaults`
//!   体 `{model_id, sampling:{k: number|null}}`（null = 跟随模型推荐 = 清除）
//! - `omlx`：管理 API 的模型设置 `PUT /admin/api/models/{id}/settings`
//!   （只发本次涉及的键 = 部分更新；null 清回默认；落 `~/.omlx/model_settings.json`）
//! - `halogen` / `gufo`：SSH 经 `hal_env.py` / `gu_env.py` 写引擎参数文件——
//!   **引擎级**（该后端所有模型共用）且**重启才生效**，故 `restart_required = true`
//! - 其余（vllm / sglang / ollama / lmstudio / custom）：没有模型级配置通道，
//!   如实回报 `channel: "none"`，UI 说清「仅对经网关的请求生效」，绝不假装成功
//!
//! 目录键 → 各家键名的映射表在下方常量里，**表里没有的键进 `unsupported`**，
//! 不猜、不硬塞（猜错就是把错键写进别人家的引擎配置，比不写坏得多）。

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::config::HalctlCfg;
use crate::engine::Engine;
use crate::gateway::State;
use crate::services::halctl::{self, Channel};
use crate::types::BackendRow;
use reqwest::Url;

/// 一次下发的变更项。
pub struct Change<'a> {
    /// 参数中心的目录键（`temperature` / `ctx` …）。
    pub key: &'a str,
    /// `Some(v)` = 写入；`None` = 清除（让后端恢复它自己的默认）。
    pub value: Option<Value>,
}

/// 一次下发的战报：UI 直接照着说人话，字段名即前端契约。
pub struct Outcome {
    /// `unsloth` / `omlx` / `halogen-env` / `gufo-env` / `none`。
    pub channel: &'static str,
    /// 下发是否成功（没有要下发的键也算成功，`pushed` 为空）。
    pub ok: bool,
    pub pushed: Vec<String>,
    pub cleared: Vec<String>,
    /// 该后端的模型级配置不认的键（仍可由请求注入兜底）。
    pub unsupported: Vec<String>,
    /// 写进去了但要重启才生效（halogen / gufo 的引擎级参数）。
    pub restart_required: bool,
    /// 后端自己的回话或错误说明。
    pub detail: String,
}

impl Outcome {
    fn ok(channel: &'static str, detail: impl Into<String>) -> Self {
        Self {
            channel,
            ok: true,
            pushed: Vec::new(),
            cleared: Vec::new(),
            unsupported: Vec::new(),
            restart_required: false,
            detail: detail.into(),
        }
    }

    fn fail(channel: &'static str, detail: impl Into<String>) -> Self {
        Self {
            channel,
            ok: false,
            detail: detail.into(),
            ..Self::ok(channel, "")
        }
    }

    /// 该引擎没有模型级配置通道：不是失败（请求注入那条路照常），但要说清。
    fn no_channel(engine: &str) -> Self {
        Self {
            unsupported: Vec::new(),
            detail: format!("引擎 {engine} 不提供模型级参数配置，参数只对经网关的请求生效"),
            ..Self::ok("none", "")
        }
    }

    pub fn to_value(&self) -> Value {
        json!({
            "channel": self.channel,
            "ok": self.ok,
            "pushed": self.pushed,
            "cleared": self.cleared,
            "unsupported": self.unsupported,
            "restart_required": self.restart_required,
            "detail": self.detail,
        })
    }
}

/// Unsloth Studio 模型级采样默认值接受的键（实测 `GET /api/inference/sampling-defaults`
/// 的返回键集，且 payload 只收 number|null）。
const UNSLOTH_KEYS: &[&str] = &[
    "temperature",
    "top_p",
    "top_k",
    "min_p",
    "repetition_penalty",
    "presence_penalty",
];

/// omlx `ModelSettingsRequest` 里同名可直发的键（其余同名不存在 → unsupported）。
const OMLX_KEYS: &[&str] = &[
    "temperature",
    "top_p",
    "top_k",
    "min_p",
    "repetition_penalty",
    "presence_penalty",
    "max_tokens",
];

/// 目录键 → omlx 字段名的改名表（目录用行业通用名，omlx 叫法不同）。
const OMLX_RENAMES: &[(&str, &str)] = &[("ctx", "max_context_window")];

/// 目录键 → halogen.env / gufo.conf 的白名单键（远端 `hal_env.py spec` 实测）。
/// 引擎级：写下去影响该后端所有模型，重启才生效。
const HALOGEN_KEYS: &[(&str, &str)] = &[
    ("temperature", "HALOGEN_TEMPERATURE"),
    ("top_p", "HALOGEN_TOP_P"),
    ("top_k", "HALOGEN_TOP_K"),
    ("min_p", "HALOGEN_MIN_P"),
    ("presence_penalty", "HALOGEN_PRESENCE_PENALTY"),
    ("frequency_penalty", "HALOGEN_FREQUENCY_PENALTY"),
    ("repetition_penalty", "HALOGEN_REPETITION_PENALTY"),
    ("reasoning_effort", "HALOGEN_REASONING_EFFORT"),
    ("thinking_budget", "HALOGEN_MAX_THINKING_TOKENS"),
    ("max_tokens", "HALOGEN_MAX_TOKENS_DEFAULT"),
    ("ctx", "HALOGEN_CTX"),
];

/// gufo 的参数文件（gufo.conf）白名单窄得多：实测 `gu_env.py spec` 只有
/// 引擎/网络两组裸键，**没有任何采样键**（采样由请求方决定，引擎不存默认）。
/// 所以只有 ctx 能下发，其余键如实进 unsupported，别硬塞。
const GUFO_KEYS: &[(&str, &str)] = &[("ctx", "CONTEXT")];

/// `model` 层的 ref_id = `<后端名>:<模型 id>`；模型 id 里可能还有冒号，只切第一刀。
pub fn split_model_ref(ref_id: &str) -> Option<(&str, &str)> {
    let (backend, model) = ref_id.split_once(':')?;
    if backend.trim().is_empty() || model.trim().is_empty() {
        return None;
    }
    Some((backend, model))
}

/// 「保存/删除一条覆盖 → 模型层顺手下发」的统一入口（IPC 与 admin REST 共用，
/// 保证两个写口口径一致）。返回战报 JSON；非模型层或 ref_id 形态不对返回
/// `None`（没有下发这回事，调用方维持原话术）。
pub async fn push_override(
    st: &State,
    scope: &str,
    ref_id: &str,
    key: &str,
    value: Option<Value>,
) -> Option<Value> {
    if scope != "model" {
        return None;
    }
    let (backend, model) = split_model_ref(ref_id)?;
    let change = Change { key, value };
    Some(push(st, backend, model, &[change]).await.to_value())
}

/// 把参数中心的模型层变更下发到后端本体。永远返回战报（失败也带原因），
/// 让调用方能把话说明白，而不是吞掉错误。
pub async fn push(st: &State, backend: &str, model: &str, changes: &[Change<'_>]) -> Outcome {
    let row = match st.store.backend_by_name(backend) {
        Ok(Some(r)) => r,
        Ok(None) => return Outcome::fail("none", format!("后端不存在：{backend}")),
        Err(e) => return Outcome::fail("none", format!("读取后端配置失败：{e}")),
    };
    let engine = row.engine.as_deref().unwrap_or("custom");
    match Engine::parse(engine) {
        Some(Engine::Unsloth) => unsloth::push(&row, model, changes).await,
        Some(Engine::Omlx) => omlx::push(&row, model, changes).await,
        Some(Engine::Halogen) => {
            envfile::push("halogen-env", &st.config.halctl, &halctl::HAL, changes).await
        }
        Some(Engine::Gufo) => {
            envfile::push("gufo-env", &st.config.guctl, &halctl::GU, changes).await
        }
        _ => Outcome::no_channel(engine),
    }
}

/// 管理接口的根地址：`base_url` 常写成 `http://host:port/v1`，而配置端点挂在根上。
fn api_root(base_url: &str) -> Option<Url> {
    let mut u = Url::parse(base_url.trim()).ok()?;
    let path = u.path().trim_end_matches('/').to_string();
    let root = path
        .strip_suffix("/v1")
        .unwrap_or(&path)
        .trim_end_matches('/')
        .to_string();
    u.set_path(if root.is_empty() { "/" } else { &root });
    u.set_query(None);
    u.set_fragment(None);
    Some(u)
}

/// 在根地址后拼路径段：逐段走 `path_segments_mut`（段内特殊字符自动 URL 编码，
/// 模型 id 带斜杠也安全），且不会出现 `//` 双斜杠——Unsloth 的 FastAPI 对
/// `//api/...` 直接 405（实测），字符串拼接正是这个坑。
fn join_path(root: &Url, segs: &[&str]) -> Url {
    let mut u = root.clone();
    if let Ok(mut seg) = u.path_segments_mut() {
        seg.pop_if_empty();
        for s in segs {
            seg.push(s);
        }
    }
    u
}

/// 后端自己的 api key（下发用推理同一个 key：管理端点同样认它）。
fn bearer(row: &BackendRow) -> Option<String> {
    row.api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 数值化：库里存的可能是数字串，各家配置端点都要真数字。
fn as_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(_) => v.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// 参数文件（halogen.env / gufo.conf）的值形态：数字去掉多余的 `.0`，
/// 字符串原样，`None` = 空串 = 远端脚本按「移除该键、跟随默认」处理。
fn env_value(v: &Option<Value>) -> String {
    let Some(v) = v else { return String::new() };
    if let Some(n) = v.as_f64() {
        return if n.fract() == 0.0 {
            format!("{}", n as i64)
        } else {
            format!("{n}")
        };
    }
    match v {
        Value::String(s) => s.trim().to_string(),
        other => other.to_string(),
    }
}

/// 共享的 HTTP 客户端：下发是低频人工动作，连接 5s / 整体 30s 足够，
/// 但要显式设——后端假死时不能把一次保存 IPC 无限挂住。
fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client")
    })
}

/// 非 2xx 的统一说法：状态码 + 后端回话片段（配置端点的报错都在 body 里）。
async fn err_detail(resp: reqwest::Response) -> String {
    let code = resp.status();
    let body = tokio::time::timeout(Duration::from_secs(5), resp.text())
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default();
    let snippet: String = body.trim().chars().take(400).collect();
    if snippet.is_empty() {
        format!("HTTP {code}")
    } else {
        format!("HTTP {code}：{snippet}")
    }
}

/// Unsloth Studio：模型级采样默认值。
mod unsloth {
    use super::*;

    pub async fn push(row: &BackendRow, model: &str, changes: &[Change<'_>]) -> Outcome {
        let mut out = Outcome::ok("unsloth", "");
        let Some(root) = api_root(&row.base_url) else {
            return Outcome::fail("unsloth", format!("后端地址无法解析：{}", row.base_url));
        };
        let mut sampling = Map::new();
        for c in changes {
            if !UNSLOTH_KEYS.contains(&c.key) {
                out.unsupported.push(c.key.to_string());
                continue;
            }
            match &c.value {
                None => {
                    sampling.insert(c.key.to_string(), Value::Null);
                    out.cleared.push(c.key.to_string());
                }
                Some(v) => match as_number(v) {
                    Some(n) => {
                        sampling.insert(c.key.to_string(), json!(n));
                        out.pushed.push(c.key.to_string());
                    }
                    None => out.unsupported.push(c.key.to_string()),
                },
            }
        }
        if sampling.is_empty() {
            out.detail = "本次的键都不在 Unsloth 的模型级采样配置里，未下发".into();
            return out;
        }
        let url = join_path(&root, &["api", "settings", "model-sampling-defaults"]);
        let mut req = client()
            .put(url)
            .json(&json!({ "model_id": model, "sampling": sampling }));
        if let Some(k) = bearer(row) {
            req = req.bearer_auth(k);
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => {
                out.detail = format!("已写入 Unsloth 模型 {model} 的采样默认值");
                out
            }
            Ok(r) => {
                let d = err_detail(r).await;
                out.ok = false;
                out.detail = d;
                out
            }
            Err(e) => {
                out.ok = false;
                out.detail = format!("请求 Unsloth 失败：{e}");
                out
            }
        }
    }
}

/// omlx：管理 API 的模型设置（部分更新，只发本次的键）。
mod omlx {
    use super::*;

    /// 目录键 → omlx 字段名；`None` = omlx 不认这个键。
    pub(crate) fn map_key(key: &str) -> Option<&'static str> {
        if let Some((_, to)) = OMLX_RENAMES.iter().find(|(from, _)| *from == key) {
            return Some(to);
        }
        OMLX_KEYS.iter().find(|k| **k == key).copied()
    }

    pub async fn push(row: &BackendRow, model: &str, changes: &[Change<'_>]) -> Outcome {
        let mut out = Outcome::ok("omlx", "");
        let Some(root) = api_root(&row.base_url) else {
            return Outcome::fail("omlx", format!("后端地址无法解析：{}", row.base_url));
        };
        // 模型 id 可能带斜杠（HF 形态），走 path segment 让 Url 去编码。
        let url = join_path(&root, &["admin", "api", "models", model, "settings"]);
        let mut payload = Map::new();
        for c in changes {
            let Some(target) = map_key(c.key) else {
                out.unsupported.push(c.key.to_string());
                continue;
            };
            match &c.value {
                None => {
                    payload.insert(target.to_string(), Value::Null);
                    out.cleared.push(c.key.to_string());
                }
                Some(v) => {
                    // 采样/长度键都是数值；非数值（对象/布尔）不猜，交请求注入兜底。
                    match as_number(v) {
                        Some(n) => {
                            payload.insert(target.to_string(), json!(n));
                            out.pushed.push(c.key.to_string());
                        }
                        None => out.unsupported.push(c.key.to_string()),
                    }
                }
            }
        }
        if payload.is_empty() {
            out.detail = "本次的键都不在 omlx 的模型设置里，未下发".into();
            return out;
        }
        let mut req = client().put(url).json(&payload);
        if let Some(k) = bearer(row) {
            req = req.bearer_auth(k);
        }
        match req.send().await {
            Ok(r) if r.status().is_success() => {
                out.detail = format!("已写入 omlx 模型 {model} 的设置");
                out
            }
            Ok(r) => {
                let d = err_detail(r).await;
                out.ok = false;
                out.detail = d;
                out
            }
            Err(e) => {
                out.ok = false;
                out.detail = format!("请求 omlx 管理接口失败：{e}");
                out
            }
        }
    }
}

/// halogen / gufo：SSH 写引擎参数文件（引擎级、重启生效）。
mod envfile {
    use super::*;

    pub async fn push(
        channel: &'static str,
        cfg: &HalctlCfg,
        ch: &'static Channel,
        changes: &[Change<'_>],
    ) -> Outcome {
        let mut out = Outcome::ok(channel, "");
        out.restart_required = true;
        let table: &[(&str, &str)] = if channel == "gufo-env" {
            GUFO_KEYS
        } else {
            HALOGEN_KEYS
        };
        let mut values = Map::new();
        for c in changes {
            let Some((_, env_key)) = table.iter().find(|(k, _)| *k == c.key) else {
                out.unsupported.push(c.key.to_string());
                continue;
            };
            values.insert((*env_key).to_string(), json!(env_value(&c.value)));
            if c.value.is_some() {
                out.pushed.push(c.key.to_string());
            } else {
                out.cleared.push(c.key.to_string());
            }
        }
        if values.is_empty() {
            out.restart_required = false;
            out.detail = "本次的键都不在该引擎的参数文件白名单里，未下发".into();
            return out;
        }
        match halctl::env_save(cfg, ch, Value::Object(values)).await {
            Ok(v) if v.get("ok").and_then(Value::as_bool) == Some(true) => {
                out.detail = "已写入引擎参数文件（引擎级：影响该后端所有模型，重启后生效）".into();
                out
            }
            Ok(v) => {
                out.ok = false;
                out.restart_required = false;
                out.detail = v
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or("远端参数写入失败")
                    .to_string();
                out
            }
            Err(e) => {
                out.ok = false;
                out.restart_required = false;
                out.detail = e;
                out
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn api_root_strips_v1_and_trailing_slash() {
        assert_eq!(
            api_root("http://127.0.0.1:8888/v1").unwrap().as_str(),
            "http://127.0.0.1:8888/"
        );
        assert_eq!(
            api_root("http://127.0.0.1:8888/").unwrap().as_str(),
            "http://127.0.0.1:8888/"
        );
        // 带路径前缀的部署要保住前缀，只剥尾巴的 /v1。
        assert_eq!(
            api_root("http://h:8000/omlx/v1/").unwrap().as_str(),
            "http://h:8000/omlx"
        );
        assert!(api_root("not a url").is_none());
    }

    #[test]
    fn join_path_avoids_double_slash_and_keeps_prefix() {
        let root = api_root("http://127.0.0.1:8888").unwrap();
        // 根路径拼接不得出现 `//api`——FastAPI 对双斜杠直接 405（Unsloth 实测）。
        assert_eq!(
            join_path(&root, &["api", "settings", "x"]).as_str(),
            "http://127.0.0.1:8888/api/settings/x"
        );
        // 带部署前缀的根地址要保住前缀。
        let prefixed = api_root("http://h:8000/omlx/v1/").unwrap();
        assert_eq!(
            join_path(&prefixed, &["admin", "api"]).as_str(),
            "http://h:8000/omlx/admin/api"
        );
        // 模型 id 里的斜杠按段编码，不会变成多一层路径。
        let r = api_root("http://h:8000").unwrap();
        assert_eq!(
            join_path(&r, &["models", "a/b", "settings"]).as_str(),
            "http://h:8000/models/a%2Fb/settings"
        );
    }

    #[test]
    fn env_value_shapes() {
        // 参数文件是文本：整数不写 .0，清除 = 空串（远端按移除键处理）。
        assert_eq!(env_value(&Some(json!(4))), "4");
        assert_eq!(env_value(&Some(json!(0.7))), "0.7");
        assert_eq!(env_value(&Some(json!("0.90"))), "0.90");
        assert_eq!(env_value(&Some(json!("xhigh"))), "xhigh");
        assert_eq!(env_value(&None), "");
    }

    #[test]
    fn numeric_strings_are_coerced() {
        assert_eq!(as_number(&json!("0.9")), Some(0.9));
        assert_eq!(as_number(&json!(20)), Some(20.0));
        assert_eq!(as_number(&json!("abc")), None);
        assert_eq!(as_number(&json!(true)), None);
    }

    #[test]
    fn omlx_key_mapping() {
        assert_eq!(omlx::map_key("temperature"), Some("temperature"));
        // 目录用通用名 ctx，omlx 的字段是 max_context_window。
        assert_eq!(omlx::map_key("ctx"), Some("max_context_window"));
        assert_eq!(omlx::map_key("beam_width"), None);
    }

    #[test]
    fn no_channel_is_honest() {
        let o = Outcome::no_channel("vllm");
        assert_eq!(o.channel, "none");
        assert!(o.detail.contains("vllm"), "{}", o.detail);
        assert!(!o.restart_required);
    }
}
