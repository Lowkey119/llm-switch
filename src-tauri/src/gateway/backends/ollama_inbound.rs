//! Ollama 入站门面（P1-A）。
//!
//! 为什么要它：GitHub Copilot Chat 想把外部模型指进来，唯一还活着的口子是
//! `chat.byok.ollamaEndpoint`（老的 `chat.customOAIModels` 已被官方归进 `Deprecated`）。
//! 于是「接管 Copilot 下游」实际等于「我们会说 Ollama 协议」。
//!
//! 以下客户端行为是从 Copilot Chat 0.48.1 的发行包（`dist/extension.js`）里读出来的，
//! 不是我猜的：
//! · 先 `GET /api/version`，版本必须 ≥ **0.6.4**（字符串 `"0.0.0"` 被特赦直通）；
//! · 再 `GET /api/tags`，读 `.models[].model` 当模型清单；
//! · 聊天走 `POST /api/chat`，`stream:true` 时是 **NDJSON**（一行一个 JSON，没有 `data:` 前缀）。
//!
//! 我们只做门面：翻译进出，路由、熔断、遥测、参数中心全都复用既有通道。

use crate::gateway::error::ProxyError;
use bytes::Bytes;
use futures::Stream;
use futures::StreamExt;
use serde_json::{json, Value};
use std::io;

/// 我们对外声称的 Ollama 版本。Copilot Chat 卡的是 `>= 0.6.4`，
/// 报低于这个数的版本号会被它直接拒之门外（并让用户去升级 Ollama，纯属误导）。
pub const CLAIMED_OLLAMA_VERSION: &str = "0.6.4";

pub fn version_payload() -> Value {
    json!({ "version": CLAIMED_OLLAMA_VERSION })
}

/// `/api/tags`：模型清单。Ollama 的返回里 `name` 与 `model` 通常同值，
/// 客户端两个都可能读，所以都给。
pub fn tags_payload(models: &[String]) -> Value {
    json!({
        "models": models.iter().map(|m| json!({
            "name": m,
            "model": m,
            "modified_at": "",
            "size": 0,
            "details": { "family": "llm-switch", "parents": [], "parent_model": "" }
        })).collect::<Vec<_>>()
    })
}

/// `/api/show`：模型详情。Ollama 客户端主要读 `model_info` 的家族与参数规模，
/// 我们没有这些真值，就**不给假的**——只回名字与模板说明。
pub fn show_payload(model: &str) -> Value {
    json!({
        "license": "",
        "modelfile": format!("FROM {model}"),
        "parameters": "",
        "template": "",
        "details": { "family": "llm-switch", "parents": [], "parent_model": "" }
    })
}

/// Ollama `/api/chat` 请求 → OpenAI chat 请求。
///
/// 差异点：
/// · `options.num_predict` → `max_tokens`，`options.temperature/top_p/stop` 同名搬家；
/// · `format:"json"` → `response_format`；
/// · `messages[].images[]`（裸 base64）→ OpenAI 的 `image_url` data URL；
/// · `stream` 由调用方单独取走（响应形状不同），这里保留在 body 里由下游决定。
pub fn ollama_to_openai_chat(body: &Value) -> Result<Value, ProxyError> {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProxyError::InvalidRequest("ollama /api/chat 缺少 model".into()))?;
    let src_messages = body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut messages = Vec::with_capacity(src_messages.len());
    for msg in &src_messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
        let text = msg.get("content").and_then(Value::as_str).unwrap_or("");
        let images = msg
            .get("images")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if images.is_empty() {
            messages.push(json!({ "role": role, "content": text }));
            continue;
        }
        // 图片走 data URL：Ollama 给的是裸 base64，没有 MIME，按 PNG 兜
        let mut parts: Vec<Value> = Vec::with_capacity(images.len() + 1);
        if !text.is_empty() {
            parts.push(json!({ "type": "text", "text": text }));
        }
        for img in &images {
            if let Some(b64) = img.as_str() {
                parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": format!("data:image/png;base64,{b64}") }
                }));
            }
        }
        messages.push(json!({ "role": role, "content": parts }));
    }

    let mut out = json!({ "model": model, "messages": messages });
    let root = out.as_object_mut().expect("just built object");
    if let Some(opts) = body.get("options").and_then(Value::as_object) {
        for (from, to) in [
            ("temperature", "temperature"),
            ("top_p", "top_p"),
            ("seed", "seed"),
        ] {
            if let Some(v) = opts.get(from) {
                root.insert(to.to_string(), v.clone());
            }
        }
        if let Some(n) = opts.get("num_predict").and_then(Value::as_u64) {
            if n > 0 {
                root.insert("max_tokens".to_string(), json!(n));
            }
        }
        if let Some(stop) = opts.get("stop").cloned() {
            root.insert("stop".to_string(), stop);
        }
    }
    if body.get("format").and_then(Value::as_str) == Some("json") {
        root.insert(
            "response_format".to_string(),
            json!({ "type": "json_object" }),
        );
    }
    if let Some(stream) = body.get("stream").and_then(Value::as_bool) {
        root.insert("stream".to_string(), json!(stream));
    }
    Ok(out)
}

/// 非流式：OpenAI chat 响应 → Ollama `/api/chat` 一次性响应。
pub fn openai_chat_to_ollama(resp: &Value) -> Result<Value, ProxyError> {
    let choice = resp
        .pointer("/choices/0/message")
        .cloned()
        .unwrap_or_else(|| json!({ "role": "assistant", "content": "" }));
    let usage = resp.get("usage").cloned().unwrap_or_else(|| json!({}));
    Ok(json!({
        "model": resp.get("model").and_then(Value::as_str).unwrap_or(""),
        "created_at": "",
        "message": choice,
        "done": true,
        "done_reason": "stop",
        "prompt_eval_count": usage.get("prompt_tokens").cloned().unwrap_or(json!(0)),
        "eval_count": usage.get("completion_tokens").cloned().unwrap_or(json!(0)),
    }))
}

/// 流式：我们的 OpenAI SSE → Ollama NDJSON（一行一个 JSON）。
///
/// 只搬内容增量与结束帧；用量统计在最后一帧给，跟 Ollama 的习惯一致。
pub fn create_ollama_ndjson_stream_from_chat<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut model = String::new();
        let mut prompt_tokens = 0u64;
        let mut completion_tokens = 0u64;

        futures::pin_mut!(stream);
        while let Some(item) = stream.next().await {
            let bytes = match item {
                Ok(b) => b,
                Err(e) => {
                    yield Err(io::Error::other(e.to_string()));
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&bytes));
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer.drain(..=pos);
                let Some(payload) = line.strip_prefix("data:") else {
                    continue;
                };
                let payload = payload.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }
                let Ok(chunk) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };
                if let Some(m) = chunk.get("model").and_then(Value::as_str) {
                    if !m.is_empty() {
                        model = m.to_string();
                    }
                }
                if let Some(u) = chunk.get("usage") {
                    prompt_tokens = u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(prompt_tokens);
                    completion_tokens = u
                        .get("completion_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(completion_tokens);
                }
                let delta = chunk.pointer("/choices/0/delta");
                let content = delta
                    .and_then(|d| d.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if !content.is_empty() {
                    let line = json!({
                        "model": model,
                        "created_at": "",
                        "message": { "role": "assistant", "content": content },
                        "done": false,
                    });
                    yield Ok(Bytes::from(line.to_string() + "\n"));
                }
            }
        }
        let last = json!({
            "model": model,
            "created_at": "",
            "message": { "role": "assistant", "content": "" },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": prompt_tokens,
            "eval_count": completion_tokens,
        });
        yield Ok(Bytes::from(last.to_string() + "\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_lists_every_exposed_model() {
        let models: Vec<String> = ["a-one".to_string(), "b-two".to_string()].into();
        let payload = tags_payload(&models);
        let list = payload["models"].as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["model"], json!("a-one"));
        assert_eq!(list[1]["name"], json!("b-two"));
    }

    #[test]
    fn chat_request_translates_options_and_images() {
        let body = json!({
            "model": "m-one",
            "messages": [
                { "role": "system", "content": "你是助手" },
                { "role": "user", "content": "看图", "images": ["AAAB"] }
            ],
            "options": { "temperature": 0.2, "num_predict": 512, "stop": ["END"] },
            "format": "json",
            "stream": true
        });
        let out = ollama_to_openai_chat(&body).unwrap();
        assert_eq!(out["model"], json!("m-one"));
        assert_eq!(out["temperature"], json!(0.2));
        assert_eq!(out["max_tokens"], json!(512));
        assert_eq!(out["stop"], json!(["END"]));
        assert_eq!(out["response_format"]["type"], json!("json_object"));
        assert_eq!(out["stream"], json!(true));
        assert_eq!(out["messages"][0]["content"], json!("你是助手"));
        let parts = out["messages"][1]["content"].as_array().unwrap();
        assert_eq!(
            parts[1]["image_url"]["url"],
            json!("data:image/png;base64,AAAB")
        );
    }

    #[test]
    fn chat_request_requires_model() {
        let err = ollama_to_openai_chat(&json!({ "messages": [] })).unwrap_err();
        assert!(err.to_string().contains("model"), "{err}");
    }

    #[test]
    fn non_stream_response_carries_usage_counts() {
        let resp = json!({
            "model": "m-one",
            "choices": [{ "message": { "role": "assistant", "content": "好" } }],
            "usage": { "prompt_tokens": 11, "completion_tokens": 5 }
        });
        let out = openai_chat_to_ollama(&resp).unwrap();
        assert_eq!(out["message"]["content"], json!("好"));
        assert_eq!(out["done"], json!(true));
        assert_eq!(out["prompt_eval_count"], json!(11));
        assert_eq!(out["eval_count"], json!(5));
    }

    #[tokio::test]
    async fn sse_becomes_ndjson_with_final_usage_frame() {
        use futures::stream;
        let chunks = vec![
            Ok::<_, std::io::Error>(Bytes::from(
                "data: {\"model\":\"m-one\",\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\n",
            )),
            Ok(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"content\":\"好\"}}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2}}\n\ndata: [DONE]\n",
            )),
        ];
        let out: Vec<_> = create_ollama_ndjson_stream_from_chat(stream::iter(chunks))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();
        let text: String = out
            .iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect();
        let lines: Vec<Value> = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert_eq!(lines[0]["message"]["content"], json!("你"));
        assert_eq!(lines[0]["done"], json!(false));
        assert_eq!(lines[2]["done"], json!(true));
        assert_eq!(lines[2]["prompt_eval_count"], json!(7));
        assert_eq!(lines[2]["eval_count"], json!(2));
    }
}
