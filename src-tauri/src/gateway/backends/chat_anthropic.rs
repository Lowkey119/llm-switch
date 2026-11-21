//! OpenAI Chat ⇄ Anthropic Messages bridging (our own code, not a port).
//!
//! Serves the `openai_chat` entry against an `anthropic` backend:
//! - request: Chat Completions body → Anthropic Messages body;
//! - response: Anthropic JSON → Chat Completion JSON;
//! - streaming: Anthropic SSE → Chat Completions SSE chunks.
//!
//! Coverage: text, images (base64/url), system messages, tools
//! (`tools`/`tool_choice`), tool results, sampling params, thinking →
//! `reasoning_content`, and usage mapping. Unsupported corners degrade with
//! sane defaults rather than hard errors.

use crate::gateway::error::ProxyError;
use crate::gateway::sse::{append_utf8_safe, strip_sse_field, take_sse_block};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Map, Value};

/// Default output ceiling when the chat request omits `max_tokens`
const DEFAULT_MAX_TOKENS: u64 = 8192;

// --------------------------------------------------------------- request leg

fn chat_image_to_anthropic(part: &Value) -> Option<Value> {
    let url = part.pointer("/image_url/url").and_then(Value::as_str)?;
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        let media_type = meta.split(';').next().unwrap_or("image/png");
        Some(json!({
            "type": "image",
            "source": {"type": "base64", "media_type": media_type, "data": data}
        }))
    } else if url.starts_with("http://") || url.starts_with("https://") {
        Some(json!({
            "type": "image",
            "source": {"type": "url", "url": url}
        }))
    } else {
        None
    }
}

fn chat_content_to_anthropic_blocks(content: &Value) -> Vec<Value> {
    match content {
        Value::String(s) => {
            if s.is_empty() {
                Vec::new()
            } else {
                vec![json!({"type": "text", "text": s})]
            }
        }
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                Some("text") => part
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|t| json!({"type": "text", "text": t})),
                Some("image_url") => chat_image_to_anthropic(part),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn chat_tool_to_anthropic(tool: &Value) -> Option<Value> {
    let func = tool.get("function").filter(|f| !f.is_null())?;
    let name = func.get("name").and_then(Value::as_str)?;
    let mut out = Map::new();
    out.insert("name".into(), json!(name));
    if let Some(desc) = func.get("description").and_then(Value::as_str) {
        out.insert("description".into(), json!(desc));
    }
    let schema = func
        .get("parameters")
        .filter(|p| !p.is_null())
        .cloned()
        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
    out.insert("input_schema".into(), schema);
    Some(Value::Object(out))
}

fn map_tool_choice(choice: &Value) -> Option<Value> {
    match choice {
        Value::String(s) => match s.as_str() {
            "auto" => Some(json!({"type": "auto"})),
            "required" => Some(json!({"type": "any"})),
            "none" => None,
            _ => Some(json!({"type": "auto"})),
        },
        Value::Object(_) => choice
            .pointer("/function/name")
            .and_then(Value::as_str)
            .map(|name| json!({"type": "tool", "name": name})),
        _ => None,
    }
}

/// Chat Completions request → Anthropic Messages request.
pub fn chat_request_to_anthropic(body: Value) -> Result<Value, ProxyError> {
    let mut out = Map::new();
    if let Some(model) = body.get("model").and_then(Value::as_str) {
        out.insert("model".into(), json!(model));
    }

    let chat_messages = body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // system messages collapse into Anthropic `system` (string).
    let system: Vec<String> = chat_messages
        .iter()
        .filter(|m| m.get("role").and_then(Value::as_str) == Some("system"))
        .filter_map(|m| match m.get("content") {
            Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
            Some(Value::Array(parts)) => {
                let joined: Vec<&str> = parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect();
                (!joined.is_empty()).then(|| joined.join("\n"))
            }
            _ => None,
        })
        .collect();
    if !system.is_empty() {
        out.insert("system".into(), json!(system.join("\n\n")));
    }

    let mut messages: Vec<Value> = Vec::new();
    for msg in chat_messages
        .iter()
        .filter(|m| m.get("role").and_then(Value::as_str) != Some("system"))
    {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("user");
        match role {
            "tool" => {
                let call_id = msg
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("call_unknown");
                let text = match msg.get("content") {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                messages.push(json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": [{"type": "text", "text": text}]
                    }]
                }));
            }
            "assistant" => {
                let mut blocks =
                    chat_content_to_anthropic_blocks(msg.get("content").unwrap_or(&Value::Null));
                if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        let name = call
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let raw = call
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        let input =
                            serde_json::from_str::<Value>(raw).unwrap_or_else(|_| json!({}));
                        messages_has_tool(
                            &mut blocks,
                            call.get("id").and_then(Value::as_str),
                            name,
                            input,
                        );
                    }
                }
                if !blocks.is_empty() {
                    messages.push(json!({"role": "assistant", "content": blocks}));
                }
            }
            _ => {
                let blocks =
                    chat_content_to_anthropic_blocks(msg.get("content").unwrap_or(&Value::Null));
                if !blocks.is_empty() {
                    messages.push(json!({"role": "user", "content": blocks}));
                }
            }
        }
    }
    if messages.is_empty() {
        return Err(ProxyError::InvalidRequest(
            "no usable messages for anthropic backend".into(),
        ));
    }
    out.insert("messages".into(), Value::Array(messages));

    let max_tokens = body
        .get("max_completion_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    out.insert("max_tokens".into(), json!(max_tokens));

    for key in ["temperature", "top_p", "stop_sequences"] {
        if let Some(v) = body.get(key).filter(|v| !v.is_null()) {
            out.insert(key.into(), v.clone());
        }
    }
    if let Some(stop) = body.get("stop").filter(|v| !v.is_null()) {
        let seqs = match stop {
            Value::String(s) => json!([s]),
            Value::Array(_) => stop.clone(),
            _ => Value::Null,
        };
        if !seqs.is_null() {
            out.insert("stop_sequences".into(), seqs);
        }
    }
    if body.get("stream").and_then(Value::as_bool) == Some(true) {
        out.insert("stream".into(), json!(true));
    }
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        let converted: Vec<Value> = tools.iter().filter_map(chat_tool_to_anthropic).collect();
        if !converted.is_empty() {
            out.insert("tools".into(), Value::Array(converted));
        }
    }
    if let Some(tc) = body.get("tool_choice").filter(|v| !v.is_null()) {
        if let Some(mapped) = map_tool_choice(tc) {
            out.insert("tool_choice".into(), mapped);
        }
    }
    Ok(Value::Object(out))
}

fn messages_has_tool(blocks: &mut Vec<Value>, id: Option<&str>, name: &str, input: Value) {
    blocks.push(json!({
        "type": "tool_use",
        "id": id.unwrap_or("toolu_unknown"),
        "name": name,
        "input": input
    }));
}

// -------------------------------------------------------------- response leg

fn map_stop_reason(reason: Option<&str>) -> &'static str {
    match reason {
        Some("tool_use") => "tool_calls",
        Some("max_tokens") => "length",
        Some("stop_sequence") => "stop",
        _ => "stop",
    }
}

/// Anthropic Messages JSON → Chat Completion JSON.
pub fn anthropic_response_to_chat(body: Value) -> Result<Value, ProxyError> {
    let mut text = String::new();
    let mut thinking = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    for block in body
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or(&vec![])
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => text.push_str(block.get("text").and_then(Value::as_str).unwrap_or("")),
            Some("thinking") => {
                thinking.push_str(block.get("thinking").and_then(Value::as_str).unwrap_or(""))
            }
            Some("tool_use") => tool_calls.push(json!({
                "id": block.get("id").and_then(Value::as_str).unwrap_or("call_0"),
                "type": "function",
                "function": {
                    "name": block.get("name").and_then(Value::as_str).unwrap_or(""),
                    "arguments": serde_json::to_string(
                        block.get("input").unwrap_or(&json!({}))
                    ).unwrap_or_else(|_| "{}".into())
                }
            })),
            _ => {}
        }
    }
    let finish = map_stop_reason(body.get("stop_reason").and_then(Value::as_str));
    let mut message = Map::new();
    message.insert("role".into(), json!("assistant"));
    message.insert(
        "content".into(),
        if text.is_empty() && !tool_calls.is_empty() {
            Value::Null
        } else {
            json!(text)
        },
    );
    if !thinking.is_empty() {
        message.insert("reasoning_content".into(), json!(thinking));
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(tool_calls));
    }
    let usage = body.get("usage").cloned().unwrap_or(json!({}));
    Ok(json!({
        "id": body.get("id").and_then(Value::as_str).unwrap_or("chatcmpl-anthropic"),
        "object": "chat.completion",
        "model": body.get("model").cloned().unwrap_or(Value::Null),
        "choices": [{"index": 0, "message": Value::Object(message), "finish_reason": finish}],
        "usage": {
            "prompt_tokens": usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
            "completion_tokens": usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
            "total_tokens": usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0)
                + usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0)
        }
    }))
}

// --------------------------------------------------------------- stream leg

/// Convert an Anthropic Messages SSE stream into OpenAI Chat Completions SSE.
pub fn create_chat_sse_stream_from_anthropic<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut model = String::new();
        let mut message_id = String::from("chatcmpl-anthropic");
        let _created = 0u64;
        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;
        let mut finish_sent = false;
        // tool_use blocks: index → (id, name)
        let mut tool_index: i64 = -1;
        let mut tool_ids: Vec<(String, String)> = Vec::new();

        let chunk = |obj: Map<String, Value>, message_id: &str, model: &str| -> Bytes {
            let mut root = Map::new();
            root.insert("id".into(), json!(message_id));
            root.insert("object".into(), json!("chat.completion.chunk"));
            root.insert("model".into(), json!(model));
            root.insert("choices".into(), json!([Value::Object(obj)]));
            Bytes::from(format!("data: {}\n\n", Value::Object(root)))
        };

        tokio::pin!(stream);
        loop {
            let item = tokio::select! { maybe = stream.next() => maybe, };
            let Some(res) = item else { break };
            match res {
                Ok(bytes) => append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes),
                Err(e) => {
                    yield Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()));
                    break;
                }
            }
        while let Some(block) = take_sse_block(&mut buffer) {
            let Some(data) = block
                .lines()
                .find_map(|l| strip_sse_field(l, "data"))
                .map(str::to_string)
            else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
            match ty {
                "message_start" => {
                    if let Some(msg) = event.get("message") {
                        model = msg.get("model").and_then(Value::as_str).unwrap_or("").to_string();
                        if let Some(id) = msg.get("id").and_then(Value::as_str) {
                            message_id = id.to_string();
                        }
                        if let Some(u) = msg.get("usage") {
                            input_tokens = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                        }
                    }
                    let mut c = Map::new();
                    c.insert("index".into(), json!(0));
                    c.insert("delta".into(), json!({"role": "assistant", "content": ""}));
                    yield Ok(chunk(c, &message_id, &model));
                }
                "content_block_start" => {
                    let block_ty = event
                        .pointer("/content_block/type")
                        .and_then(Value::as_str)
                        .unwrap_or("text");
                    if block_ty == "tool_use" {
                        tool_index += 1;
                        let id = event
                            .pointer("/content_block/id")
                            .and_then(Value::as_str)
                            .unwrap_or("call_0")
                            .to_string();
                        let name = event
                            .pointer("/content_block/name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        tool_ids.push((id.clone(), name.clone()));
                        let mut c = Map::new();
                        c.insert("index".into(), json!(0));
                        c.insert("delta".into(), json!({"tool_calls": [{
                            "index": tool_index, "id": id, "type": "function",
                            "function": {"name": name, "arguments": ""}
                        }]}));
                        yield Ok(chunk(c, &message_id, &model));
                    }
                }
                "content_block_delta" => {
                    let dt = event
                        .pointer("/delta/type")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if dt == "text_delta" {
                        let t = event.pointer("/delta/text").and_then(Value::as_str).unwrap_or("");
                        if !t.is_empty() {
                            let mut c = Map::new();
                            c.insert("index".into(), json!(0));
                            c.insert("delta".into(), json!({"content": t}));
                            yield Ok(chunk(c, &message_id, &model));
                        }
                    } else if dt == "thinking_delta" {
                        let t = event
                            .pointer("/delta/thinking")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if !t.is_empty() {
                            let mut c = Map::new();
                            c.insert("index".into(), json!(0));
                            c.insert("delta".into(), json!({"reasoning_content": t}));
                            yield Ok(chunk(c, &message_id, &model));
                        }
                    } else if dt == "input_json_delta" {
                        let t = event.pointer("/delta/partial_json").and_then(Value::as_str).unwrap_or("");
                        if !t.is_empty() && tool_index >= 0 {
                            let mut c = Map::new();
                            c.insert("index".into(), json!(0));
                            c.insert("delta".into(), json!({"tool_calls": [{
                                "index": tool_index,
                                "function": {"arguments": t}
                            }]}));
                            yield Ok(chunk(c, &message_id, &model));
                        }
                    }
                }
                "message_delta" => {
                    if let Some(u) = event.get("usage") {
                        output_tokens =
                            u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                    }
                    if let Some(reason) = event.pointer("/delta/stop_reason").and_then(Value::as_str) {
                        let mut c = Map::new();
                        c.insert("index".into(), json!(0));
                        c.insert("delta".into(), json!({}));
                        c.insert("finish_reason".into(), json!(map_stop_reason(Some(reason))));
                        yield Ok(chunk(c, &message_id, &model));
                        finish_sent = true;
                    }
                }
                "message_stop" => {
                    if !finish_sent {
                        let mut c = Map::new();
                        c.insert("index".into(), json!(0));
                        c.insert("delta".into(), json!({}));
                        c.insert("finish_reason".into(), json!("stop"));
                        yield Ok(chunk(c, &message_id, &model));
                    }
                    let mut root = Map::new();
                    root.insert("id".into(), json!(message_id));
                    root.insert("object".into(), json!("chat.completion.chunk"));
                    root.insert("model".into(), json!(model));
                    root.insert("choices".into(), json!([]));
                    root.insert("usage".into(), json!({
                        "prompt_tokens": input_tokens,
                        "completion_tokens": output_tokens,
                        "total_tokens": input_tokens + output_tokens
                    }));
                    yield Ok(Bytes::from(format!("data: {}\n\n", Value::Object(root))));
                    yield Ok(Bytes::from_static(b"data: [DONE]\n\n"));
                }
                _ => {}
            }
        }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_system_user_assistant_tool_roundtrip() {
        let out = chat_request_to_anthropic(json!({
            "model": "claude-x",
            "messages": [
                {"role": "system", "content": "be nice"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "yo", "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "bash", "arguments": "{\"cmd\":\"ls\"}"}}
                ]},
                {"role": "tool", "tool_call_id": "call_1", "content": "file.txt"}
            ],
            "max_tokens": 128,
            "temperature": 0.5
        }))
        .unwrap();
        assert_eq!(out["system"], "be nice");
        assert_eq!(out["max_tokens"], 128);
        assert_eq!(out["temperature"], 0.5);
        let msgs = out["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[1]["content"][1]["type"], "tool_use");
        assert_eq!(msgs[1]["content"][1]["input"]["cmd"], "ls");
        assert_eq!(msgs[2]["role"], "user");
        assert_eq!(msgs[2]["content"][0]["type"], "tool_result");
        assert_eq!(msgs[2]["content"][0]["tool_use_id"], "call_1");
    }

    #[test]
    fn request_tools_and_tool_choice_and_stop_mapping() {
        let out = chat_request_to_anthropic(json!({
            "model": "m",
            "messages": [{"role": "user", "content": [{"type": "text", "text": "x"}]}],
            "tools": [{"type": "function", "function": {"name": "f", "description": "d", "parameters": {"type": "object"}}}],
            "tool_choice": {"type": "function", "function": {"name": "f"}},
            "stop": ["END"],
            "stream": true
        }))
        .unwrap();
        assert_eq!(out["tools"][0]["name"], "f");
        assert_eq!(out["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(out["tool_choice"]["type"], "tool");
        assert_eq!(out["stop_sequences"][0], "END");
        assert_eq!(out["stream"], true);
        assert!(out.get("max_tokens").is_some());
    }

    #[test]
    fn response_text_and_tools_and_usage() {
        let out = anthropic_response_to_chat(json!({
            "id": "msg_1", "model": "claude-x",
            "content": [
                {"type": "text", "text": "answer"},
                {"type": "tool_use", "id": "tu_1", "name": "bash", "input": {"cmd": "ls"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 4}
        }))
        .unwrap();
        assert_eq!(out["object"], "chat.completion");
        assert_eq!(out["choices"][0]["message"]["content"], "answer");
        assert_eq!(out["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            out["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "bash"
        );
        assert_eq!(out["usage"]["prompt_tokens"], 10);
        assert_eq!(out["usage"]["total_tokens"], 14);
    }

    #[tokio::test]
    async fn stream_translates_text_and_usage() {
        let raw = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_9\",\"model\":\"claude-x\",\"usage\":{\"input_tokens\":7}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"he\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"llo\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let input = futures::stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(raw))]);
        let out: Vec<_> = create_chat_sse_stream_from_anthropic(input)
            .map(|r| r.unwrap())
            .collect()
            .await;
        let text: String = out
            .iter()
            .map(|b| String::from_utf8_lossy(b).to_string())
            .collect();
        assert!(text.contains("\"role\":\"assistant\""));
        assert!(text.contains("he") && text.contains("llo"));
        assert!(text.contains("\"finish_reason\":\"stop\""));
        assert!(text.contains("\"completion_tokens\":5"));
        assert!(text.trim_end().ends_with("data: [DONE]"));
    }
}
