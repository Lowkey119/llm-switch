//! Gemini native backend adapter (`generateContent` / `streamGenerateContent`).
//!
//! Two entry shapes are served:
//! - Anthropic entry: full request/response translation via the ported
//!   `transform_gemini` module (with shadow-store replay when available);
//! - Gemini entry: pass-through (the caller hands us the original
//!   `/v1beta/models/<model>:<action>` endpoint and we forward verbatim).

use super::adapter::{auth_header_value, ProviderAdapter, TransformCtx};
use super::transform_gemini::{
    anthropic_to_gemini_with_shadow, gemini_to_anthropic_with_shadow_and_hints,
};
use crate::gateway::error::ProxyError;
use crate::types::Protocol;
use http::{HeaderName, HeaderValue};
use serde_json::Value;

pub struct GeminiAdapter;

#[allow(dead_code)]
fn trim_base(base: &str) -> &str {
    base.trim_end_matches('/')
}

impl ProviderAdapter for GeminiAdapter {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn build_url(&self, base_url: &str) -> String {
        // Fallback without body context; the real URL carries the model name
        // and is produced by `build_url_for`.
        crate::gateway::backends::adapter::join_path(base_url, "/v1beta/models")
    }

    fn build_url_for(&self, base_url: &str, body: &Value, client_wants_stream: bool) -> String {
        let model = body
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .unwrap_or("unknown-model");
        let streaming =
            client_wants_stream || body.get("stream").and_then(Value::as_bool) == Some(true);
        let action = if streaming {
            "streamGenerateContent?alt=sse"
        } else {
            "generateContent"
        };
        crate::gateway::backends::adapter::join_path(
            base_url,
            &format!("/v1beta/models/{model}:{action}"),
        )
    }

    fn auth_headers(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<(HeaderName, HeaderValue)>, ProxyError> {
        let mut headers = Vec::new();
        if let Some(key) = api_key.filter(|k| !k.is_empty()) {
            headers.push((
                HeaderName::from_static("x-goog-api-key"),
                auth_header_value(key)?,
            ));
        }
        Ok(headers)
    }

    fn needs_transform(&self, entry: Protocol) -> bool {
        matches!(entry, Protocol::Anthropic)
    }

    fn transform_request(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError> {
        match entry {
            Protocol::Anthropic => anthropic_to_gemini_with_shadow(body, None, None, None),
            Protocol::Gemini => Ok(body),
            other => Err(ProxyError::InvalidRequest(format!(
                "entry protocol {other} against a gemini backend is not supported"
            ))),
        }
    }

    fn transform_request_ctx(
        &self,
        body: Value,
        entry: Protocol,
        ctx: &TransformCtx,
    ) -> Result<Value, ProxyError> {
        match entry {
            Protocol::Anthropic => anthropic_to_gemini_with_shadow(
                body,
                ctx.gemini_shadow,
                Some(ctx.provider_id),
                Some(ctx.session_id),
            ),
            _ => self.transform_request(body, entry),
        }
    }

    fn transform_response(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError> {
        match entry {
            Protocol::Anthropic => {
                gemini_to_anthropic_with_shadow_and_hints(body, None, None, None, None)
            }
            Protocol::Gemini => Ok(body),
            other => Err(ProxyError::InvalidRequest(format!(
                "entry protocol {other} against a gemini backend is not supported"
            ))),
        }
    }

    fn transform_response_ctx(
        &self,
        body: Value,
        entry: Protocol,
        ctx: &TransformCtx,
    ) -> Result<Value, ProxyError> {
        match entry {
            Protocol::Anthropic => gemini_to_anthropic_with_shadow_and_hints(
                body,
                ctx.gemini_shadow,
                Some(ctx.provider_id),
                Some(ctx.session_id),
                ctx.gemini_hints,
            ),
            _ => self.transform_response(body, entry),
        }
    }
}
