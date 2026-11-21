// Deviation: replace_images_for_text_only_model (Provider/model_capabilities gated) not
// ported; llm-switch gates image stripping on BackendRow.capabilities in a later slice.
#![allow(dead_code)] // 移植层包含后续里程碑才会接线的函数

use crate::gateway::error::ProxyError;
use crate::gateway::tool_media::{
    strip_media_from_tool_value, tool_output_contains_media, ToolMediaScope,
};
use serde_json::{json, Value};

pub const UNSUPPORTED_IMAGE_MARKER: &str = "[Unsupported Image]";

pub fn contains_image_blocks(body: &Value) -> bool {
    messages_have_image_blocks(body)
        || responses_input_has_image_blocks(body.get("input"))
        || gemini_contents_have_image_blocks(body)
}

pub fn replace_image_blocks_with_marker(body: &mut Value) -> usize {
    replace_images_in_body(body)
}

pub fn is_unsupported_image_error(error: &ProxyError) -> bool {
    let ProxyError::UpstreamError { status, body } = error else {
        return false;
    };

    if !matches!(*status, 400 | 415 | 422 | 501) {
        return false;
    }

    let Some(body) = body.as_deref() else {
        return false;
    };

    let message = extract_error_text(body);
    let message = message.to_ascii_lowercase();

    // 自证性表述：这类短语本身就断言了"仅接受文本"，属于模态拒绝，无需再要求
    // 错误提到 image/media 等字样——火山方舟等网关的报错是
    // "Model only support text input"，全程不出现 image（issue #5025）。
    // 国产网关的英文常缺三单 s，因此带 s / 不带 s 两种形式都要列。
    const TEXT_ONLY_SELF_EVIDENT_HINTS: &[&str] = &["only support text", "only supports text"];
    if TEXT_ONLY_SELF_EVIDENT_HINTS
        .iter()
        .any(|hint| message.contains(hint))
    {
        return true;
    }

    let mentions_image = message.contains("image")
        || message.contains("vision")
        || message.contains("multimodal")
        || message.contains("multi-modal")
        || message.contains("modality")
        || message.contains("modalities")
        || message.contains("media")
        || message.contains("attachment");

    if !mentions_image {
        return false;
    }

    const UNSUPPORTED_HINTS: &[&str] = &[
        "unsupported",
        "not supported",
        "does not support",
        "doesn't support",
        "do not support",
        "don't support",
        "text only",
        "text-only",
        "invalid content type",
        "invalid message content",
        "unknown variant",
        "unknown content type",
        "unrecognized content type",
        "cannot process",
        "cannot handle",
        "can't process",
        "can't handle",
        "unable to process",
    ];

    UNSUPPORTED_HINTS.iter().any(|hint| message.contains(hint))
}

fn content_has_image_blocks(content: &Value) -> bool {
    let Some(blocks) = content.as_array() else {
        return false;
    };

    blocks.iter().any(|block| {
        is_image_block_type(block.get("type").and_then(Value::as_str))
            || block.get("content").is_some_and(|nested| {
                content_has_image_blocks(nested)
                    || (block.get("type").and_then(Value::as_str) == Some("tool_result")
                        && tool_output_contains_media(nested, ToolMediaScope::ImagesOnly))
            })
    })
}

fn replace_images_in_body(body: &mut Value) -> usize {
    let message_replacements = body
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .map(|messages| messages.iter_mut().map(replace_images_in_message).sum())
        .unwrap_or(0);

    message_replacements
        + body
            .get_mut("input")
            .map(replace_images_in_responses_input)
            .unwrap_or(0)
        + replace_images_in_gemini_contents(body)
}

fn replace_images_in_message(message: &mut Value) -> usize {
    let is_tool_message = message.get("role").and_then(Value::as_str) == Some("tool");
    let Some(content) = message.get_mut("content") else {
        return 0;
    };

    if is_tool_message {
        // Preserve the legacy typed-image replacement semantics first,
        // including Anthropic cache_control on the replacement text block.
        // The shared traversal then handles JSON strings, MCP wrappers, and
        // loose data-URL shapes that the legacy recursion does not recognize.
        let mut replaced = replace_images_in_content(content);
        let replacement_block = json!({
            "type":"text",
            "text":UNSUPPORTED_IMAGE_MARKER
        });
        let mut discarded_media = Vec::new();
        replaced += strip_media_from_tool_value(
            content,
            &mut discarded_media,
            ToolMediaScope::ImagesOnly,
            &replacement_block,
            UNSUPPORTED_IMAGE_MARKER,
        );
        replaced
    } else {
        replace_images_in_content(content)
    }
}

fn replace_images_in_content(content: &mut Value) -> usize {
    replace_images_in_content_with_text_type(content, "text")
}

fn replace_images_in_content_with_text_type(content: &mut Value, text_type: &str) -> usize {
    let Some(blocks) = content.as_array_mut() else {
        return 0;
    };

    let mut replaced = 0usize;
    for block in blocks {
        let block_type = block.get("type").and_then(Value::as_str);
        if is_image_block_type(block_type) {
            replace_image_block_with_text_marker(block, text_type);
            replaced += 1;
            continue;
        }

        let is_tool_result = block_type == Some("tool_result");
        if let Some(nested_content) = block.get_mut("content") {
            if is_tool_result {
                // Run the legacy typed-block replacement before the shared
                // payload-aware traversal. This makes replacement a superset
                // of detection and preserves cache_control on Anthropic image
                // blocks, while the second pass covers alternate tool shapes.
                replaced += replace_images_in_content_with_text_type(nested_content, text_type);
                let replacement_block = json!({
                    "type":text_type,
                    "text":UNSUPPORTED_IMAGE_MARKER
                });
                let mut discarded_media = Vec::new();
                replaced += strip_media_from_tool_value(
                    nested_content,
                    &mut discarded_media,
                    ToolMediaScope::ImagesOnly,
                    &replacement_block,
                    UNSUPPORTED_IMAGE_MARKER,
                );
            } else {
                replaced += replace_images_in_content_with_text_type(nested_content, text_type);
            }
        }
    }

    replaced
}

fn messages_have_image_blocks(body: &Value) -> bool {
    body.get("messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                let Some(content) = message.get("content") else {
                    return false;
                };
                content_has_image_blocks(content)
                    || (message.get("role").and_then(Value::as_str) == Some("tool")
                        && tool_output_contains_media(content, ToolMediaScope::ImagesOnly))
            })
        })
}

fn gemini_contents_have_image_blocks(body: &Value) -> bool {
    body.get("contents")
        .and_then(Value::as_array)
        .is_some_and(|contents| {
            contents.iter().any(|content| {
                content
                    .get("parts")
                    .and_then(Value::as_array)
                    .is_some_and(|parts| parts.iter().any(gemini_part_has_image))
            })
        })
}

fn gemini_part_has_image(part: &Value) -> bool {
    gemini_media_payload_is_image(part.get("inlineData").or_else(|| part.get("inline_data")))
        || gemini_media_payload_is_image(part.get("fileData").or_else(|| part.get("file_data")))
        || part
            .get("functionResponse")
            .or_else(|| part.get("function_response"))
            .and_then(|response| response.get("parts"))
            .and_then(Value::as_array)
            .is_some_and(|parts| parts.iter().any(gemini_part_has_image))
}

fn gemini_media_payload_is_image(payload: Option<&Value>) -> bool {
    payload
        .and_then(|payload| payload.get("mimeType").or_else(|| payload.get("mime_type")))
        .and_then(Value::as_str)
        .is_some_and(|mime_type| {
            mime_type
                .get(..6)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/"))
        })
}

fn replace_images_in_gemini_contents(body: &mut Value) -> usize {
    body.get_mut("contents")
        .and_then(Value::as_array_mut)
        .map(|contents| {
            contents
                .iter_mut()
                .filter_map(|content| content.get_mut("parts").and_then(Value::as_array_mut))
                .map(|parts| {
                    parts
                        .iter_mut()
                        .map(replace_images_in_gemini_part)
                        .sum::<usize>()
                })
                .sum()
        })
        .unwrap_or(0)
}

fn replace_images_in_gemini_part(part: &mut Value) -> usize {
    if gemini_media_payload_is_image(part.get("inlineData").or_else(|| part.get("inline_data")))
        || gemini_media_payload_is_image(part.get("fileData").or_else(|| part.get("file_data")))
    {
        *part = json!({"text":UNSUPPORTED_IMAGE_MARKER});
        return 1;
    }

    let response_key = if part.get("functionResponse").is_some() {
        "functionResponse"
    } else {
        "function_response"
    };
    let Some(function_response) = part.get_mut(response_key) else {
        return 0;
    };
    let Some(media_parts) = function_response
        .get_mut("parts")
        .and_then(Value::as_array_mut)
    else {
        return 0;
    };

    let before = media_parts.len();
    media_parts.retain(|media_part| !gemini_part_has_image(media_part));
    let replaced = before.saturating_sub(media_parts.len());
    if replaced > 0 {
        if let Some(response) = function_response
            .get_mut("response")
            .and_then(Value::as_object_mut)
        {
            response.insert(
                "cc_switch_media".to_string(),
                Value::String(UNSUPPORTED_IMAGE_MARKER.to_string()),
            );
        }
    }
    replaced
}

fn responses_input_has_image_blocks(input: Option<&Value>) -> bool {
    match input {
        Some(Value::Array(items)) => items.iter().any(responses_input_item_has_image_blocks),
        Some(item @ Value::Object(_)) => responses_input_item_has_image_blocks(item),
        _ => false,
    }
}

fn responses_input_item_has_image_blocks(item: &Value) -> bool {
    if item.get("type").and_then(Value::as_str) == Some("input_image") {
        return true;
    }

    item.get("content").is_some_and(content_has_image_blocks)
        || item
            .get("output")
            .is_some_and(|output| tool_output_contains_media(output, ToolMediaScope::ImagesOnly))
}

fn replace_images_in_responses_input(input: &mut Value) -> usize {
    match input {
        Value::Array(items) => items
            .iter_mut()
            .map(replace_images_in_responses_input_item)
            .sum(),
        Value::Object(_) => replace_images_in_responses_input_item(input),
        _ => 0,
    }
}

fn replace_images_in_responses_input_item(item: &mut Value) -> usize {
    let mut replaced = 0usize;

    if item.get("type").and_then(Value::as_str) == Some("input_image") {
        replace_image_block_with_text_marker(item, "input_text");
        replaced += 1;
    }

    if let Some(content) = item.get_mut("content") {
        replaced += replace_images_in_content_with_text_type(content, "input_text");
    }

    if let Some(output) = item.get_mut("output") {
        // The image-capability fallback deliberately strips images only.
        // Tool-output files/audio remain a known unsupported-modality gap.
        let replacement_block = json!({
            "type": "input_text",
            "text": UNSUPPORTED_IMAGE_MARKER
        });
        let mut discarded_media = Vec::new();
        replaced += strip_media_from_tool_value(
            output,
            &mut discarded_media,
            ToolMediaScope::ImagesOnly,
            &replacement_block,
            UNSUPPORTED_IMAGE_MARKER,
        );
    }

    replaced
}

fn is_image_block_type(block_type: Option<&str>) -> bool {
    matches!(block_type, Some("image" | "image_url" | "input_image"))
}

fn replace_image_block_with_text_marker(block: &mut Value, text_type: &str) {
    let cache_control = block.get("cache_control").cloned();
    *block = json!({
        "type": text_type,
        "text": UNSUPPORTED_IMAGE_MARKER
    });
    if let (Some(cache_control), Some(object)) = (cache_control, block.as_object_mut()) {
        object.insert("cache_control".to_string(), cache_control);
    }
}

fn extract_error_text(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        let candidates = [
            value.pointer("/error/message"),
            value.pointer("/message"),
            value.pointer("/detail"),
            value.pointer("/error"),
        ];
        if let Some(message) = candidates
            .into_iter()
            .flatten()
            .find_map(|value| value.as_str())
        {
            return message.to_string();
        }

        if let Ok(compact) = serde_json::to_string(&value) {
            return compact;
        }
    }

    body.to_string()
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_unsupported_image_errors() {
        let error = ProxyError::UpstreamError {
            status: 400,
            body: Some(
                r#"{"error":{"message":"This model does not support image input"}}"#.to_string(),
            ),
        };
        assert!(is_unsupported_image_error(&error));
    }

    #[test]
    fn detects_text_only_errors_without_image_mention() {
        let error = ProxyError::UpstreamError {
            status: 400,
            body: Some(
                r#"{"error":{"message":"Model only support text input Request id: 021783"}}"#
                    .to_string(),
            ),
        };
        assert!(is_unsupported_image_error(&error));
    }

    #[test]
    fn ignores_non_image_errors() {
        let error = ProxyError::UpstreamError {
            status: 400,
            body: Some(r#"{"error":{"message":"Invalid API key"}}"#.to_string()),
        };
        assert!(!is_unsupported_image_error(&error));
    }

    #[test]
    fn preserves_cache_control_when_replacing_image() {
        // image block 可能承载 prompt cache 断点；替换成标记时必须把
        // cache_control 迁移到新的 text block，否则会断掉缓存命中。
        let mut body = json!({
            "model": "deepseek-v4-pro",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": { "type": "base64", "media_type": "image/png", "data": "abc" },
                    "cache_control": { "type": "ephemeral" }
                }]
            }]
        });
        let count = replace_image_blocks_with_marker(&mut body);
        assert_eq!(count, 1);
        let block = &body["messages"][0]["content"][0];
        assert_eq!(block["type"], "text");
        assert_eq!(block["text"], UNSUPPORTED_IMAGE_MARKER);
        assert_eq!(block["cache_control"]["type"], "ephemeral");
    }
}
