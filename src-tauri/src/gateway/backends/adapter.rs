//! keyed on our `BackendRow` + entry protocol pair instead of a rich Provider
//! config. Only what M1 needs: URL building, auth headers, and request/response
//! transforms for the non-streaming leg (stream transforms live in
//! `response_processor`, keyed by the same `needs_transform` answer).

use super::gemini::GeminiAdapter;
use super::gemini_shadow::GeminiShadowStore;
use super::openai_responses::OpenAiResponsesAdapter;
use super::transform_gemini::AnthropicToolSchemaHints;
use super::{anthropic, openai_chat};
use crate::gateway::error::ProxyError;
use crate::types::Protocol;
use http::{HeaderName, HeaderValue};
use serde_json::Value;

/// Session-scoped extras for transforms that need more than the body itself
/// (Gemini shadow replay + tool-schema hints). Adapters that do not care
/// simply ignore it.
pub struct TransformCtx<'a> {
    pub gemini_shadow: Option<&'a GeminiShadowStore>,
    pub provider_id: &'a str,
    pub session_id: &'a str,
    pub gemini_hints: Option<&'a AnthropicToolSchemaHints>,
}

pub trait ProviderAdapter: Send + Sync {
    /// Adapter name for logs/diagnostics.
    #[allow(dead_code)] // wired into retry traces in a later slice
    fn name(&self) -> &'static str;

    /// Canonical inference URL for this backend protocol.
    fn build_url(&self, base_url: &str) -> String;

    /// URL variant that may consult the outbound body and the client's
    /// stream flag (Gemini embeds model name and generate/stream action in
    /// its path; the translated Gemini body itself carries no `stream`).
    fn build_url_for(&self, base_url: &str, _body: &Value, _client_wants_stream: bool) -> String {
        self.build_url(base_url)
    }

    /// Auth (and protocol-mandatory) headers for the outbound request.
    fn auth_headers(
        &self,
        api_key: Option<&str>,
    ) -> Result<Vec<(HeaderName, HeaderValue)>, ProxyError>;

    /// Whether bodies must be translated between the entry protocol the
    /// client spoke and this backend's native protocol.
    fn needs_transform(&self, entry: Protocol) -> bool;

    /// Translate a client request body into this backend's native shape.
    fn transform_request(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError>;

    /// Translate a client request body with session context attached.
    fn transform_request_ctx(
        &self,
        body: Value,
        entry: Protocol,
        _ctx: &TransformCtx,
    ) -> Result<Value, ProxyError> {
        self.transform_request(body, entry)
    }

    /// Translate a native non-streaming response into the entry shape.
    fn transform_response(&self, body: Value, entry: Protocol) -> Result<Value, ProxyError>;

    /// Translate a native non-streaming response with session context.
    fn transform_response_ctx(
        &self,
        body: Value,
        entry: Protocol,
        _ctx: &TransformCtx,
    ) -> Result<Value, ProxyError> {
        self.transform_response(body, entry)
    }
}

/// Pick the adapter for a backend protocol string (stored on `BackendRow`).
/// 把协议路径拼到 base_url 后面，并且**容忍 base_url 已经带了这一段**。
///
/// 大家习惯把「OpenAI 兼容根」当 base_url 抄过来（`http://h:8000/v1`），
/// 而适配器又要往后拼 `/v1/chat/completions`， naive 拼接就成了
/// `/v1/v1/chat/completions` —— 探活和推理一起 404。这里做一次坍缩：
/// base 结尾正好是路径的第一段，就不重复第二遍。
pub fn join_path(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let trimmed = path.trim_start_matches('/');
    let head = trimmed.split('/').next().unwrap_or("");
    if !head.is_empty() && base.ends_with(&format!("/{head}")) {
        let rest = match trimmed.split_once('/') {
            Some((_, tail)) => format!("/{tail}"),
            None => String::new(),
        };
        return format!("{base}{rest}");
    }
    format!("{base}/{trimmed}")
}

pub fn adapter_for(protocol: &str) -> Option<&'static dyn ProviderAdapter> {
    match Protocol::parse(protocol) {
        Some(Protocol::OpenaiChat) => Some(&openai_chat::OpenAiChatAdapter),
        Some(Protocol::Anthropic) => Some(&anthropic::AnthropicAdapter),
        Some(Protocol::Gemini) => Some(&GeminiAdapter),
        Some(Protocol::OpenaiResponses) => Some(&OpenAiResponsesAdapter),
        _ => None,
    }
}

/// Build an HTTP header value from user-supplied credentials, turning an
/// undecodable value into a clean auth error instead of a panic.
pub fn auth_header_value(s: &str) -> Result<HeaderValue, ProxyError> {
    HeaderValue::from_str(s)
        .map_err(|e| ProxyError::AuthError(format!("invalid auth header value: {e}")))
}

#[cfg(test)]
mod tests {
    use super::join_path;

    #[test]
    fn plain_root_gets_the_full_path() {
        assert_eq!(
            join_path("http://h:8000", "/v1/chat/completions"),
            "http://h:8000/v1/chat/completions"
        );
        assert_eq!(
            join_path("http://h:8000/", "/v1/messages"),
            "http://h:8000/v1/messages"
        );
    }

    #[test]
    fn base_already_carrying_the_segment_is_not_doubled() {
        assert_eq!(
            join_path("http://h:8000/v1", "/v1/chat/completions"),
            "http://h:8000/v1/chat/completions"
        );
        assert_eq!(
            join_path("http://h:8000/v1/", "/v1/models"),
            "http://h:8000/v1/models"
        );
        assert_eq!(
            join_path(
                "https://p/foo/v1beta",
                "/v1beta/models/gemini:generateContent"
            ),
            "https://p/foo/v1beta/models/gemini:generateContent"
        );
    }

    #[test]
    fn path_prefixed_proxy_keeps_its_prefix() {
        assert_eq!(
            join_path("https://gateway.example/tenant-a/v1", "/v1/responses"),
            "https://gateway.example/tenant-a/v1/responses"
        );
        // 只有整段相等才坍缩，`/v1x` 这种前缀相似不算。
        assert_eq!(
            join_path("http://h/v1x", "/v1/models"),
            "http://h/v1x/v1/models"
        );
    }
}
