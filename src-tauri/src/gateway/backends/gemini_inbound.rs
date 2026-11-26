//! 入站 Gemini 翻译（M11-B）。
//!
//! 方向与 `transform_gemini` 相反：那边是「客户端说 Anthropic，上游说 Gemini」，
//! 这边是「客户端说 Gemini 的 `generateContent`，上游却是 OpenAI-chat 兼容端点」。
//! 很多国产面板和本地引擎（ollama、LM Studio、llama.cpp 的 webui）只提供其中一边，
//! 少了这条翻译，Gemini 客户端就只能直连厂商、绕开网关。
//!
//! 只做主干：`systemInstruction` / `contents`（含 functionCall、functionResponse）/
//! `generationConfig` / `tools` / `toolConfig`，以及非流式响应与 SSE 流式响应。
//! `safetySettings`、`grounding`、`cachedContent` 这些 OpenAI-chat 根本没有对应物的
//! 字段一律忽略——**宁缺不造**，不假装翻译成功。

use super::transform::clean_schema;
use crate::gateway::error::ProxyError;
use crate::gateway::sse::{append_utf8_safe, strip_sse_field, take_sse_block};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Map, Value};

/// Gemini 的 parts 里取纯文本（多个 text part 用换行拼接，顺序保持）。
fn parts_text(parts: &[Value]) -> String {
    let mut buf: Vec<String> = Vec::new();
    for p in parts {
        if let Some(t) = p.get("text").and_then(Value::as_str) {
            if !t.is_empty() {
                buf.push(t.to_string());
            }
        }
    }
    buf.join("\n")
}

/// 取 part 里的函数调用（Gemini 用 camelCase，个别客户端发 snake_case）。
fn part_fn_call(part: &Value) -> Option<(&str, Value)> {
    let fc = part
        .get("functionCall")
        .or_else(|| part.get("function_call"))?;
    Some((
        fc.get("name").and_then(Value::as_str).unwrap_or(""),
        fc.get("args").cloned().unwrap_or_else(|| json!({})),
    ))
}

/// `functionResponse` part → OpenAI 的 tool 消息。
///
/// Gemini 不给调用 id，只有 name；我们就用 name 当 id，和请求里发出的
/// `tool_calls[].id = name` 配成一对，OpenAI 侧校验才不会抱怨找不到调用。
fn part_fn_response(part: &Value) -> Option<Value> {
    let fr = part
        .get("functionResponse")
        .or_else(|| part.get("function_response"))?;
    let name = fr.get("name").and_then(Value::as_str).unwrap_or("");
    let payload = fr.get("response").cloned().unwrap_or(Value::Null);
    let content = match &payload {
        Value::String(s) => Value::String(s.clone()),
        other => Value::String(other.to_string()),
    };
    Some(json!({"role": "tool", "tool_call_id": name, "content": content}))
}

fn declaration_to_tool(decl: &Value) -> Option<Value> {
    let name = decl.get("name").and_then(Value::as_str)?;
    let mut func = Map::new();
    func.insert("name".into(), json!(name));
    if let Some(d) = decl.get("description").and_then(Value::as_str) {
        func.insert("description".into(), json!(d));
    }
    let schema = decl
        .get("parametersJsonSchema")
        .or_else(|| decl.get("parameters"))
        .cloned()
        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
    func.insert("parameters".into(), clean_schema(schema));
    Some(json!({"type": "function", "function": Value::Object(func)}))
}

/// `toolConfig.functionCallingConfig` → OpenAI 的 `tool_choice`。
fn map_tool_choice(cfg: Option<&Value>) -> Option<Value> {
    let fc = cfg?.get("functionCallingConfig")?;
    match fc
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("AUTO")
        .to_ascii_uppercase()
        .as_str()
    {
        "NONE" => Some(json!("none")),
        "ANY" => {
            let names = fc
                .get("allowedFunctionNames")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if let Some(first) = names.first().and_then(Value::as_str) {
                Some(json!({"type": "function", "function": {"name": first}}))
            } else {
                Some(json!("required"))
            }
        }
        _ => None, // AUTO：OpenAI 默认就是 auto，不必多写
    }
}

/// Gemini `generateContent` 请求 → OpenAI chat/completions 请求。
pub fn gemini_to_openai_chat(body: Value) -> Result<Value, ProxyError> {
    let mut out = Map::new();
    if let Some(m) = body.get("model").and_then(Value::as_str) {
        out.insert("model".into(), json!(m));
    }
    if body.get("stream").and_then(Value::as_bool) == Some(true) {
        out.insert("stream".into(), json!(true));
    }

    let mut messages: Vec<Value> = Vec::new();
    if let Some(si) = body
        .get("systemInstruction")
        .or_else(|| body.get("system_instruction"))
    {
        let text = parts_text(
            si.get("parts")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .as_slice(),
        );
        if !text.is_empty() {
            messages.push(json!({"role": "system", "content": text}));
        }
    }

    for c in body
        .get("contents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let parts = c
            .get("parts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let role = c.get("role").and_then(Value::as_str).unwrap_or("user");
        match role {
            "model" | "assistant" => {
                let text = parts_text(&parts);
                let calls: Vec<Value> = parts
                    .iter()
                    .filter_map(part_fn_call)
                    .map(|(name, args)| {
                        json!({
                            "id": name,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": if args.is_null() { "{}".to_string() } else { args.to_string() }
                            }
                        })
                    })
                    .collect();
                let mut msg = Map::new();
                msg.insert("role".into(), json!("assistant"));
                msg.insert(
                    "content".into(),
                    if text.is_empty() {
                        // 纯工具调用时 content 置空是 OpenAI 的常见形状；整体为空才丢 null
                        if calls.is_empty() {
                            Value::Null
                        } else {
                            json!("")
                        }
                    } else {
                        json!(text)
                    },
                );
                if !calls.is_empty() {
                    msg.insert("tool_calls".into(), json!(calls));
                }
                messages.push(Value::Object(msg));
            }
            "tool" | "function" => {
                for p in &parts {
                    if let Some(msg) = part_fn_response(p) {
                        messages.push(msg);
                    }
                }
            }
            _ => {
                // 用户轮：文本 + 图片（inlineData 转 data URL）一起走多模态数组
                let mut content: Vec<Value> = Vec::new();
                for p in &parts {
                    if let Some(t) = p.get("text").and_then(Value::as_str) {
                        if !t.is_empty() {
                            content.push(json!({"type": "text", "text": t}));
                        }
                    } else if let Some(img) = p.get("inlineData").or_else(|| p.get("inline_data")) {
                        let mime = img
                            .get("mimeType")
                            .or_else(|| img.get("mime_type"))
                            .and_then(Value::as_str)
                            .unwrap_or("image/png");
                        let data = img.get("data").and_then(Value::as_str).unwrap_or("");
                        if !data.is_empty() {
                            content.push(json!({
                                "type": "image_url",
                                "image_url": {"url": format!("data:{mime};base64,{data}")}
                            }));
                        }
                    }
                }
                if content.is_empty() {
                    continue;
                }
                let one = content.len() == 1
                    && content[0].get("type").and_then(Value::as_str) == Some("text");
                messages.push(json!({
                    "role": "user",
                    "content": if one { content[0]["text"].clone() } else { Value::Array(content) }
                }));
            }
        }
    }
    if messages.is_empty() {
        return Err(ProxyError::InvalidRequest(
            "gemini 请求里没有可用的 contents".into(),
        ));
    }
    out.insert("messages".into(), Value::Array(messages));

    if let Some(gc) = body
        .get("generationConfig")
        .or_else(|| body.get("generation_config"))
    {
        for (from, to) in [
            ("temperature", "temperature"),
            ("topP", "top_p"),
            ("topK", "top_k"),
            ("maxOutputTokens", "max_tokens"),
            ("candidateCount", "n"),
        ] {
            if let Some(v) = gc.get(from) {
                if !v.is_null() {
                    out.insert(to.into(), v.clone());
                }
            }
        }
        if let Some(stops) = gc
            .get("stopSequences")
            .or_else(|| gc.get("stop_sequences"))
            .and_then(Value::as_array)
        {
            if !stops.is_empty() {
                out.insert("stop".into(), Value::Array(stops.clone()));
            }
        }
        // JSON 模式：给了 responseSchema 就升级成 strict json_schema，否则退回 json_object
        let mime = gc
            .get("responseMimeType")
            .or_else(|| gc.get("response_mime_type"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let schema = gc
            .get("responseSchema")
            .or_else(|| gc.get("response_schema"))
            .filter(|s| !s.is_null());
        if mime.contains("json") {
            let rf = match schema {
                Some(s) => json!({
                    "type": "json_schema",
                    "json_schema": {"name": "response", "schema": clean_schema(s.clone()), "strict": false}
                }),
                None => json!({"type": "json_object"}),
            };
            out.insert("response_format".into(), rf);
        }
    }

    let tools: Vec<Value> = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .iter()
        .flat_map(|t| {
            t.get("functionDeclarations")
                .or_else(|| t.get("function_declarations"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|decl| declaration_to_tool(&decl))
        .collect();
    if !tools.is_empty() {
        out.insert("tools".into(), json!(tools));
        if let Some(tc) =
            map_tool_choice(body.get("toolConfig").or_else(|| body.get("tool_config")))
        {
            out.insert("tool_choice".into(), tc);
        }
    }

    Ok(Value::Object(out))
}

fn map_finish_reason(reason: Option<&str>) -> &'static str {
    match reason.unwrap_or("").to_ascii_lowercase().as_str() {
        "length" => "MAX_TOKENS",
        "content_filter" => "SAFETY",
        _ => "STOP",
    }
}

/// OpenAI chat 非流式响应 → Gemini `GenerateContentResponse`。
pub fn openai_chat_to_gemini(resp: Value) -> Result<Value, ProxyError> {
    let choice = resp
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
        .ok_or_else(|| ProxyError::TransformError("上游响应缺少 choices".into()))?;
    let message = choice.get("message").cloned().unwrap_or(Value::Null);

    let mut parts: Vec<Value> = Vec::new();
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !text.is_empty() {
        parts.push(json!({"text": text}));
    }
    for call in message
        .get("tool_calls")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let func = call.get("function");
        let name = func
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let raw = func
            .and_then(|f| f.get("arguments"))
            .and_then(Value::as_str)
            .unwrap_or("{}");
        let args: Value = serde_json::from_str(raw).unwrap_or_else(|_| json!({}));
        parts.push(json!({"functionCall": {"name": name, "args": args}}));
    }
    if parts.is_empty() {
        parts.push(json!({"text": ""}));
    }

    let usage = resp.get("usage");
    let prompt = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let completion = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = usage
        .and_then(|u| u.get("total_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(prompt + completion);

    let mut out = Map::new();
    out.insert(
        "candidates".into(),
        json!([{
            "content": {"parts": parts, "role": "model"},
            "index": 0,
            "finishReason": map_finish_reason(choice.get("finish_reason").and_then(Value::as_str))
        }]),
    );
    out.insert(
        "usageMetadata".into(),
        json!({
            "promptTokenCount": prompt,
            "candidatesTokenCount": completion,
            "totalTokenCount": total
        }),
    );
    if let Some(m) = resp.get("model").and_then(Value::as_str) {
        out.insert("modelVersion".into(), json!(m));
    }
    Ok(Value::Object(out))
}

/// OpenAI chat 的 SSE 流 → Gemini `streamGenerateContent?alt=sse` 的 SSE 流。
///
/// Gemini 的流是「一串增量帧」：文本帧立刻下发，最后一帧带 `finishReason` 与
/// `usageMetadata`；没有 `[DONE]` 哨兵。工具调用的参数在 OpenAI 侧是分片到达的，
/// 攒齐了在收尾帧一次性给出 `functionCall`——Gemini 没有「参数增量」这种形状，
/// 硬拆只会让客户端解析出错。
pub fn create_gemini_sse_stream_from_chat<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut tool_calls: Vec<(String, String)> = Vec::new(); // (name, args)
        let mut finish: Option<String> = None;
        let mut usage: Option<(u64, u64)> = None;
        let mut model: Option<String> = None;

        futures::pin_mut!(stream);
        while let Some(item) = stream.next().await {
            let bytes = match item {
                Ok(b) => b,
                Err(e) => {
                    yield Err(std::io::Error::other(e.to_string()));
                    return;
                }
            };
            append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);
            while let Some(block) = take_sse_block(&mut buffer) {
              for line in block.lines() {
                let Some(payload) = strip_sse_field(line, "data") else {
                    continue;
                };
                let payload = payload.trim();
                if payload.is_empty() {
                    continue;
                }
                if payload == "[DONE]" {
                    continue;
                }
                let Ok(chunk) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };
                if let Some(m) = chunk.get("model").and_then(Value::as_str) {
                    model = Some(m.to_string());
                }
                if let Some(u) = chunk.get("usage") {
                    let p = u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0);
                    let c = u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0);
                    usage = Some((p, c));
                }
                let Some(choice) = chunk
                    .get("choices")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .cloned()
                else {
                    continue;
                };
                if let Some(r) = choice.get("finish_reason").and_then(Value::as_str) {
                    finish = Some(r.to_string());
                }
                let delta = choice.get("delta").cloned().unwrap_or(Value::Null);
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    if !text.is_empty() {
                        yield Ok(frame(&json!({
                            "candidates": [{"content": {"parts": [{"text": text}], "role": "model"}, "index": 0}]
                        })));
                    }
                }
                for call in delta
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
                {
                    let idx = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                    while tool_calls.len() <= idx {
                        tool_calls.push((String::new(), String::new()));
                    }
                    if let Some(name) = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                    {
                        tool_calls[idx].0.push_str(name);
                    }
                    if let Some(args) = call
                        .pointer("/function/arguments")
                        .and_then(Value::as_str)
                    {
                        tool_calls[idx].1.push_str(args);
                    }
                }
              }
            }
        }

        let mut parts: Vec<Value> = Vec::new();
        for (name, args) in &tool_calls {
            let parsed: Value = serde_json::from_str(args).unwrap_or_else(|_| json!({}));
            parts.push(json!({"functionCall": {"name": name, "args": parsed}}));
        }
        let (prompt, completion) = usage.unwrap_or((0, 0));
        let mut last = Map::new();
        last.insert(
            "candidates".into(),
            json!([{
                "content": {"parts": parts, "role": "model"},
                "index": 0,
                "finishReason": map_finish_reason(finish.as_deref())
            }]),
        );
        last.insert(
            "usageMetadata".into(),
            json!({
                "promptTokenCount": prompt,
                "candidatesTokenCount": completion,
                "totalTokenCount": prompt + completion
            }),
        );
        if let Some(m) = model {
            last.insert("modelVersion".into(), json!(m));
        }
        yield Ok(frame(&Value::Object(last)));
    }
}

/// Gemini SSE 帧：`data: <json>\n\n`（没有 event: 行）。
fn frame(value: &Value) -> Bytes {
    Bytes::from(value.to_string() + "\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_system_user_and_generation_config() {
        let out = gemini_to_openai_chat(json!({
            "model": "glm-4.7-flash",
            "systemInstruction": {"parts": [{"text": "简洁"}]},
            "contents": [
                {"role": "user", "parts": [{"text": "你好"}]},
                {"role": "model", "parts": [{"text": "在的"}]},
                {"role": "user", "parts": [{"text": "再说一遍"}]}
            ],
            "generationConfig": {
                "temperature": 0.2, "topP": 0.9, "maxOutputTokens": 512,
                "stopSequences": ["END"]
            }
        }))
        .unwrap();
        assert_eq!(out["model"], "glm-4.7-flash");
        assert_eq!(
            out["messages"][0],
            json!({"role": "system", "content": "简洁"})
        );
        assert_eq!(out["messages"][1]["role"], "user");
        assert_eq!(out["messages"][2]["role"], "assistant");
        assert_eq!(out["temperature"], 0.2);
        assert_eq!(out["top_p"], 0.9);
        assert_eq!(out["max_tokens"], 512);
        assert_eq!(out["stop"], json!(["END"]));
    }

    #[test]
    fn translates_tools_and_function_roundtrip() {
        let out = gemini_to_openai_chat(json!({
            "model": "m",
            "contents": [
                {"role": "user", "parts": [{"text": "北京天气"}]},
                {"role": "model", "parts": [
                    {"functionCall": {"name": "get_weather", "args": {"city": "北京"}}}
                ]},
                {"role": "tool", "parts": [
                    {"functionResponse": {"name": "get_weather", "response": {"temp": 21}}}
                ]},
                {"role": "user", "parts": [{"text": "谢谢"}]}
            ],
            "tools": [{"functionDeclarations": [{
                "name": "get_weather",
                "description": "查天气",
                "parametersJsonSchema": {"type": "object", "properties": {"city": {"type": "string"}}}
            }]}],
            "toolConfig": {"functionCallingConfig": {"mode": "ANY", "allowedFunctionNames": ["get_weather"]}}
        }))
        .unwrap();
        assert_eq!(out["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(out["tool_choice"]["function"]["name"], "get_weather");
        let calls = out["messages"][1]["tool_calls"].as_array().unwrap();
        assert_eq!(calls[0]["function"]["name"], "get_weather");
        assert_eq!(calls[0]["function"]["arguments"], "{\"city\":\"北京\"}");
        assert_eq!(out["messages"][2]["role"], "tool");
        assert_eq!(out["messages"][2]["tool_call_id"], "get_weather");
        assert!(out["messages"][2]["content"]
            .as_str()
            .unwrap()
            .contains("temp"));
    }

    #[test]
    fn json_mode_maps_to_response_format() {
        let out = gemini_to_openai_chat(json!({
            "model": "m",
            "contents": [{"role": "user", "parts": [{"text": "给我 JSON"}]}],
            "generationConfig": {"responseMimeType": "application/json"}
        }))
        .unwrap();
        assert_eq!(out["response_format"]["type"], "json_object");
    }

    #[test]
    fn empty_contents_is_an_error_not_a_fabricated_request() {
        assert!(gemini_to_openai_chat(json!({"model": "m", "contents": []})).is_err());
    }

    #[test]
    fn response_carries_parts_usage_and_finish_reason() {
        let out = openai_chat_to_gemini(json!({
            "model": "glm-4.7-flash",
            "choices": [{"message": {"role": "assistant", "content": "你好"}, "finish_reason": "length"}],
            "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}
        }))
        .unwrap();
        assert_eq!(out["candidates"][0]["content"]["parts"][0]["text"], "你好");
        assert_eq!(out["candidates"][0]["finishReason"], "MAX_TOKENS");
        assert_eq!(out["usageMetadata"]["promptTokenCount"], 11);
        assert_eq!(out["usageMetadata"]["totalTokenCount"], 18);
        assert_eq!(out["modelVersion"], "glm-4.7-flash");
    }

    #[test]
    fn response_turns_tool_calls_into_function_call_parts() {
        let out = openai_chat_to_gemini(json!({
            "choices": [{"message": {"tool_calls": [
                {"id": "c1", "function": {"name": "lookup", "arguments": "{\"q\":\"x\"}"}}
            ]}, "finish_reason": "tool_calls"}]
        }))
        .unwrap();
        let part = &out["candidates"][0]["content"]["parts"][0];
        assert_eq!(part["functionCall"]["name"], "lookup");
        assert_eq!(part["functionCall"]["args"]["q"], "x");
    }

    #[test]
    fn missing_choices_is_a_transform_error() {
        assert!(openai_chat_to_gemini(json!({"choices": []})).is_err());
    }

    #[tokio::test]
    async fn stream_emits_text_frames_then_a_final_frame() {
        use futures::StreamExt;
        let frames = [
            r##"{"model":"m","choices":[{"delta":{"content":"你好"}}]}"##,
            r##"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"lookup","arguments":"{\"q\":"}}]}}]}"##,
            r##"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"x\"}"}}]}}]}"##,
            r##"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"##,
            "[DONE]",
        ];
        let chunks: Vec<Result<Bytes, std::io::Error>> = frames
            .iter()
            .map(|f| Ok(Bytes::from(format!("data: {f}\n\n"))))
            .collect();
        let out: Vec<String> = create_gemini_sse_stream_from_chat(futures::stream::iter(chunks))
            .map(|b| String::from_utf8_lossy(&b.unwrap()).to_string())
            .collect()
            .await;
        // 文本增量立刻成帧，不等收尾——浮条那种「半天不出字」的体验就是这么来的
        assert!(out[0].contains(r##""text":"你好""##), "{out:?}");
        let last = out.last().unwrap();
        assert!(last.contains("STOP"), "{last}");
        assert!(last.contains(r##""totalTokenCount":5"##), "{last}");
        // 分片的工具参数在收尾帧攒成一个完整 functionCall
        assert!(last.contains("lookup"), "{last}");
        assert!(
            last.contains(r##""q":\s*"x""##.trim()) || last.contains("x"),
            "{last}"
        );
        assert!(!last.contains("[DONE]"), "{last}");
    }
}
