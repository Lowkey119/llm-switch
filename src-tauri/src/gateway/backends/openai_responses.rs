//! OpenAI Responses backend adapter (`POST /v1/responses` upstream).
//!
//! Anthropic-entry clients are translated through the ported
//! `transform_responses` / `streaming_responses` modules; Responses-entry
//! clients pass through untouched.

use super::adapter::{auth_header_value, ProviderAdapter};
use super::transform_responses::{anthropic_to_responses, responses_to_anthropic};
use crate::gateway::error::ProxyError;
use crate::types::Protocol;
use http::{HeaderName, HeaderValue};
use serde_json::Value;

pub struct OpenAiResponsesAdapter;

#[allow(dead_code)]
fn trim_base(base: &str) -> &str {
    base.trim_end_matches('/')
}

impl ProviderAdapter for OpenAiResponsesAdapter {
    fn name(&self) -> &'static str {
        "openai_responses"
    }

    fn build_url(&self, base_url: &str) -> String {
        crate::gateway::backends::adapter::join_path(base_url, "/v1/responses")
    }

    fn auth_headers(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<(HeaderName, HeaderValue)>, ProxyError> {
        let mut headers = Vec::new();
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            headers.push((
                HeaderName::from_static("authorization"),
                auth_header_value(&format!("Bearer {key}"))?,
            ));
        }
        Ok(headers)
    }

    fn needs_transform(&self, entry: Protocol) -> bool {
        matches!(entry, Protocol::Anthropic)
    }

    fn transform_request(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError> {
        match entry {
            Protocol::Anthropic => anthropic_to_responses(body, None, false, false),
            Protocol::OpenaiResponses => Ok(body),
            other => Err(ProxyError::InvalidRequest(format!(
                "entry protocol {other} against an openai_responses backend is not supported"
            ))),
        }
    }

    fn transform_response(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError> {
        match entry {
            Protocol::Anthropic => responses_to_anthropic(body),
            Protocol::OpenaiResponses => Ok(body),
            other => Err(ProxyError::InvalidRequest(format!(
                "entry protocol {other} against an openai_responses backend is not supported"
            ))),
        }
    }
}
