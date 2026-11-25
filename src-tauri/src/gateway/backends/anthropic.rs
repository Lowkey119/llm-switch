//! Anthropic Messages backend adapter (mostly pass-through for M1).

use super::adapter::{auth_header_value, ProviderAdapter};
use crate::gateway::error::ProxyError;
use crate::types::Protocol;
use http::{HeaderName, HeaderValue};
use serde_json::Value;

pub struct AnthropicAdapter;

#[allow(dead_code)]
fn trim_base(base: &str) -> &str {
    base.trim_end_matches('/')
}

impl ProviderAdapter for AnthropicAdapter {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn build_url(&self, base_url: &str) -> String {
        crate::gateway::backends::adapter::join_path(base_url, "/v1/messages")
    }

    fn auth_headers(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<(HeaderName, HeaderValue)>, ProxyError> {
        let mut headers = vec![(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        )];
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            headers.push((
                HeaderName::from_static("x-api-key"),
                auth_header_value(key)?,
            ));
        }
        Ok(headers)
    }

    fn needs_transform(&self, entry: Protocol) -> bool {
        entry != Protocol::Anthropic
    }

    fn transform_request(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError> {
        match entry {
            Protocol::OpenaiChat => super::chat_anthropic::chat_request_to_anthropic(body),
            Protocol::Anthropic => Ok(body),
            other => Err(ProxyError::InvalidRequest(format!(
                "entry protocol {other} is not supported by the anthropic backend adapter yet"
            ))),
        }
    }

    fn transform_response(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError> {
        match entry {
            Protocol::Anthropic => Ok(body),
            Protocol::OpenaiChat => super::chat_anthropic::anthropic_response_to_chat(body),
            other => Err(ProxyError::InvalidRequest(format!(
                "entry protocol {other} is not supported by the anthropic backend adapter yet"
            ))),
        }
    }
}
