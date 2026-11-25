//! OpenAI Chat Completions backend adapter.

use super::adapter::{auth_header_value, ProviderAdapter};
use super::gemini_inbound::{gemini_to_openai_chat, openai_chat_to_gemini};
use super::transform::{
    anthropic_to_openai_with_reasoning_content, inject_openai_stream_include_usage,
    openai_to_anthropic,
};
use super::transform_codex_chat::{chat_completion_to_response, responses_to_chat_completions};
use crate::gateway::error::ProxyError;
use crate::types::Protocol;
use http::{HeaderName, HeaderValue};
use serde_json::Value;

pub struct OpenAiChatAdapter;

#[allow(dead_code)]
fn trim_base(base: &str) -> &str {
    base.trim_end_matches('/')
}

impl ProviderAdapter for OpenAiChatAdapter {
    fn name(&self) -> &'static str {
        "openai_chat"
    }

    fn build_url(&self, base_url: &str) -> String {
        crate::gateway::backends::adapter::join_path(base_url, "/v1/chat/completions")
    }

    fn auth_headers(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<(HeaderName, HeaderValue)>, ProxyError> {
        let Some(key) = api_key.filter(|k| !k.is_empty()) else {
            return Ok(Vec::new());
        };
        let value = auth_header_value(&format!("Bearer {key}"))?;
        Ok(vec![(HeaderName::from_static("authorization"), value)])
    }

    fn needs_transform(&self, entry: Protocol) -> bool {
        entry != Protocol::OpenaiChat
    }

    fn transform_request(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError> {
        match entry {
            Protocol::Anthropic => {
                let mut out = anthropic_to_openai_with_reasoning_content(body, false)?;
                inject_openai_stream_include_usage(&mut out);
                Ok(out)
            }
            Protocol::OpenaiChat => Ok(body),
            Protocol::OpenaiResponses => responses_to_chat_completions(body),
            // 入站 Gemini（M11-B）：客户端说 generateContent，上游是 OpenAI-chat。
            Protocol::Gemini => {
                let mut out = gemini_to_openai_chat(body)?;
                inject_openai_stream_include_usage(&mut out);
                Ok(out)
            }
            // ComfyUI 只是后端侧视频协议，永远不会是聊天入口协议。
            Protocol::ComfyUI => Err(ProxyError::Internal("comfyui 不是入口协议".into())),
        }
    }

    fn transform_response(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError> {
        match entry {
            Protocol::Anthropic => openai_to_anthropic(body),
            Protocol::OpenaiChat => Ok(body),
            Protocol::OpenaiResponses => chat_completion_to_response(body),
            Protocol::Gemini => openai_chat_to_gemini(body),
            Protocol::ComfyUI => Err(ProxyError::Internal("comfyui 不是入口协议".into())),
        }
    }
}
