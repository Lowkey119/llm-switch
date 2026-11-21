//! 请求分类器：把入站请求分成 user / agent 发起，并标注 warmup / compact / subagent。
//!
//! 智能择路（warmup 探针降级到小模型、subagent/compact 不计 premium）铺路。本模块是纯函数，
//! 不改变转发正确性。
//!
//! 判定口径（与 copilot-api 的 merge-then-classify 对齐）：
//! - 最后一条消息非 user → user（subagent 时 agent）
//! - 最后一条 user 消息含 tool_result → agent（工具续写），否则 user
//! - compact / subagent 一律 agent

use serde_json::Value;

/// 请求分类结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestClassification {
    /// "user" 或 "agent"。
    pub initiator: &'static str,
    /// 是否为 warmup/探针请求（可降级到小模型）。
    pub is_warmup: bool,
    /// 是否为上下文压缩/compact 请求。
    pub is_compact: bool,
    /// 是否为子代理（Agent tool 生成的 subagent）请求。
    pub is_subagent: bool,
}

/// 分类 Anthropic 格式的请求体。
///
/// - `has_anthropic_beta`：请求头是否带 `anthropic-beta`（Claude Code warmup 探针标志）。
/// - `compact_detection` / `subagent_detection`：是否启用对应检测。
pub fn classify_request(
    body: &Value,
    has_anthropic_beta: bool,
    compact_detection: bool,
    subagent_detection: bool,
) -> RequestClassification {
    let is_compact = compact_detection && is_compact_request(body);
    let is_subagent = subagent_detection && detect_subagent(body);

    let messages = match body.get("messages").and_then(|m| m.as_array()) {
        Some(msgs) if !msgs.is_empty() => msgs,
        _ => {
            return RequestClassification {
                initiator: "user",
                is_warmup: is_warmup_request(body, has_anthropic_beta, false),
                is_compact: false,
                is_subagent,
            }
        }
    };

    let last_msg = &messages[messages.len() - 1];
    let role = last_msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

    // 只有 role=user 的消息需要细分。
    if role != "user" {
        return RequestClassification {
            initiator: if is_subagent { "agent" } else { "user" },
            is_warmup: false,
            is_compact,
            is_subagent,
        };
    }

    // 只要 content 数组含 tool_result → 工具续写（agent），否则用户发起（user）。
    let is_user_initiated = match last_msg.get("content") {
        Some(Value::Array(blocks)) => !blocks
            .iter()
            .any(|block| block.get("type").and_then(|t| t.as_str()) == Some("tool_result")),
        Some(Value::String(_)) => true,
        _ => false,
    };

    let initiator = if is_subagent || !is_user_initiated || is_compact {
        "agent"
    } else {
        "user"
    };

    RequestClassification {
        initiator,
        is_warmup: initiator == "user" && is_warmup_request(body, has_anthropic_beta, is_compact),
        is_compact,
        is_subagent,
    }
}

/// warmup/探针请求：三条件同时满足——带 `anthropic-beta`、无 tools、非 compact。
fn is_warmup_request(body: &Value, has_anthropic_beta: bool, is_compact: bool) -> bool {
    if !has_anthropic_beta || is_compact {
        return false;
    }
    body.get("tools")
        .and_then(|tools| tools.as_array())
        .is_none_or(|tools| tools.is_empty())
}

/// compact 请求：只匹配 Claude Code 内部生成的机器特征，不匹配用户可能手输的通用短语。
fn is_compact_request(body: &Value) -> bool {
    // 信号 1：system prompt 以 Claude Code compact 专用前缀开头（用户无法手动设置）。
    if extract_system_text(body)
        .starts_with("You are a helpful AI assistant tasked with summarizing conversations")
    {
        return true;
    }

    let messages = match body.get("messages").and_then(|m| m.as_array()) {
        Some(msgs) => msgs,
        None => return false,
    };

    if let Some(last_msg) = messages.last() {
        if last_msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            return false;
        }
        let text = extract_text_from_message(last_msg);
        // 信号 2：compact 的机器指令（大小写敏感，精确匹配）。
        if text.contains("CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.") {
            return true;
        }
        // 信号 3：compact 的结构标记（两个同时出现才算）。
        if text.contains("Pending Tasks:") && text.contains("Current Work:") {
            return true;
        }
    }

    false
}

/// 子代理请求：显式 `__SUBAGENT_MARKER__`，或 metadata.user_id 含 `_agent_`。
fn detect_subagent(body: &Value) -> bool {
    if extract_system_text(body).contains("__SUBAGENT_MARKER__") {
        return true;
    }
    if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
                continue;
            }
            if extract_text_from_message(msg).contains("__SUBAGENT_MARKER__") {
                return true;
            }
        }
    }
    if let Some(user_id) = body.pointer("/metadata/user_id").and_then(|v| v.as_str()) {
        if user_id.contains("_agent_") {
            return true;
        }
    }
    false
}

/// 提取 system 文本（string 或 text block 数组）。
fn extract_system_text(body: &Value) -> String {
    match body.get("system") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                    b.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 提取一条消息的文本（string 或 text block 数组）。
fn extract_text_from_message(msg: &Value) -> String {
    match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                    b.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_user_message_is_user() {
        let body = json!({
            "messages": [{"role": "user", "content": "hello"}]
        });
        let c = classify_request(&body, false, true, true);
        assert_eq!(c.initiator, "user");
        assert!(!c.is_warmup);
    }

    #[test]
    fn tool_result_continuation_is_agent() {
        let body = json!({
            "messages": [
                {"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "x", "input": {}}]},
                {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]}
            ]
        });
        let c = classify_request(&body, false, true, true);
        assert_eq!(c.initiator, "agent");
    }

    #[test]
    fn warmup_probe_is_flagged() {
        let body = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": []
        });
        let c = classify_request(&body, true, true, true);
        assert_eq!(c.initiator, "user");
        assert!(c.is_warmup);
    }

    #[test]
    fn compact_by_system_prefix_is_agent() {
        let body = json!({
            "system": "You are a helpful AI assistant tasked with summarizing conversations",
            "messages": [{"role": "user", "content": "summarize"}]
        });
        let c = classify_request(&body, false, true, true);
        assert!(c.is_compact);
        assert_eq!(c.initiator, "agent");
    }

    #[test]
    fn subagent_by_metadata_is_agent() {
        let body = json!({
            "messages": [{"role": "user", "content": "do the task"}],
            "metadata": {"user_id": "sess123_agent_abc"}
        });
        let c = classify_request(&body, false, true, true);
        assert!(c.is_subagent);
        assert_eq!(c.initiator, "agent");
    }
}
