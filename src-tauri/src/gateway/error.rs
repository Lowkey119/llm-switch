//! but trimmed to the variants M1 actually needs.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// The client request is malformed or unsupported.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// 请求体超过代理自身的上限（`server.max_body_mb`），还没回源就被拒了。
    #[error("request body too large: {0}")]
    PayloadTooLarge(String),

    /// Protocol conversion failed.
    #[error("transform error: {0}")]
    TransformError(String),

    /// Missing/invalid credentials for the chosen backend.
    #[error("auth error: {0}")]
    AuthError(String),

    /// Outbound network failure before any response was received.
    #[error("network error: {0}")]
    NetworkError(String),

    /// Connect / first-byte / total deadline exceeded.
    #[error("timeout: {0}")]
    Timeout(String),

    /// An open stream produced no chunk within the idle window.
    #[error("stream idle for {0}s")]
    StreamIdleTimeout(u64),

    /// Upstream answered with a non-success status.
    #[error("upstream returned {status}")]
    UpstreamError { status: u16, body: Option<String> },

    /// No enabled backend can serve the requested model.
    #[error("no backend available for the requested model")]
    NoBackendAvailable,

    /// 没有任何后端声明过这个模型（配错了模型名，和「后端在但暂时不可用」是两回事）。
    #[error("model `{0}` is not served by any configured backend")]
    ModelNotFound(String),

    /// Anything else.
    #[error("internal error: {0}")]
    Internal(String),
}

impl ProxyError {
    /// 这条错误是**请求本身**的问题，而不是后端故障。
    ///
    /// 关键区别在于该不该记到后端账上：客户端把入口协议用错、body 翻不出来，
    /// 换一个后端也一样失败，把它喂给熔断器就会出现「有人打错几个请求，
    /// 健康后端对所有人黑屏 30 秒」（实测踩过，见 tests/e2e_breaker_police.rs）。
    /// 注意 `AuthError` 不算：后端密钥错确实是这台后端不健康。
    pub fn is_client_error(&self) -> bool {
        matches!(
            self,
            ProxyError::InvalidRequest(_)
                | ProxyError::TransformError(_)
                | ProxyError::PayloadTooLarge(_)
        )
    }

    /// 熔断记账口径：**后端是否仍然健康**。
    ///
    /// 上游回了 4xx = 它活着、只是不收这个请求，跟 M11-A 对客户端错的纪律同理，
    /// 不该记成后端故障把它熔掉（实测踩过：Claude Code 一个畸形请求把健康后端熔了 30s，
    /// 后面全是 `no backend available`）。
    /// 例外：401/403（密钥不对，对我们就是不可用）、408/425/429（超时与过载信号）
    /// 仍算故障——它们说的确实是「这台现在不好使」。
    pub fn backend_healthy(&self) -> bool {
        if self.is_client_error() {
            return true;
        }
        match self {
            ProxyError::UpstreamError { status, .. } => {
                (400..=499).contains(status) && !matches!(status, 401 | 403 | 408 | 425 | 429)
            }
            _ => false,
        }
    }

    /// 面向日志与客户端的诊断文本：上游非 2xx 时把上游原文带上（截断），
    /// 否则「upstream returned 400」这句话什么也诊断不出来。
    ///
    /// 红线（#43）：上游错误体常回显凭据（401 把 key 原样吐回来是常态），
    /// 出口统一过 `redact::secrets`——日志与客户端响应都不许见到 API。
    pub fn diagnostic(&self) -> String {
        let raw = match self {
            ProxyError::UpstreamError {
                status,
                body: Some(b),
            } if !b.trim().is_empty() => {
                let flat: String = b.chars().take(400).collect();
                let flat: String = flat.split_whitespace().collect::<Vec<_>>().join(" ");
                format!("upstream returned {status}: {flat}")
            }
            other => other.to_string(),
        };
        crate::redact::secrets(&raw)
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            ProxyError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            ProxyError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            ProxyError::TransformError(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ProxyError::AuthError(_) => StatusCode::UNAUTHORIZED,
            ProxyError::NetworkError(_) => StatusCode::BAD_GATEWAY,
            ProxyError::Timeout(_) => StatusCode::GATEWAY_TIMEOUT,
            ProxyError::StreamIdleTimeout(_) => StatusCode::GATEWAY_TIMEOUT,
            ProxyError::UpstreamError { status, .. } => {
                StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY)
            }
            ProxyError::NoBackendAvailable => StatusCode::SERVICE_UNAVAILABLE,
            ProxyError::ModelNotFound(_) => StatusCode::NOT_FOUND,
            ProxyError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Anthropic-style error `type` string (clients key off this).
    pub fn error_type(&self) -> &'static str {
        match self {
            ProxyError::InvalidRequest(_) | ProxyError::PayloadTooLarge(_) => {
                "invalid_request_error"
            }
            ProxyError::TransformError(_) => "api_error",
            ProxyError::AuthError(_) => "authentication_error",
            ProxyError::NetworkError(_) | ProxyError::UpstreamError { .. } => "api_error",
            ProxyError::Timeout(_) | ProxyError::StreamIdleTimeout(_) => "api_error",
            ProxyError::NoBackendAvailable => "not_found_error",
            ProxyError::ModelNotFound(_) => "model_not_found",
            ProxyError::Internal(_) => "api_error",
        }
    }
}

impl From<crate::store::StoreError> for ProxyError {
    fn from(err: crate::store::StoreError) -> Self {
        ProxyError::Internal(err.to_string())
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let body = json!({
            "type": "error",
            "error": {
                "type": self.error_type(),
                "message": self.diagnostic(),
            }
        });
        (self.status_code(), Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 红线（#43）：上游错误体回显的 key 绝不能出现在诊断文本——它同时
    /// 进网关日志与客户端响应（`into_response` 也吃 `diagnostic()`）。
    #[test]
    fn diagnostic_redacts_upstream_echoed_key() {
        let e = ProxyError::UpstreamError {
            status: 401,
            body: Some(
                r#"{"error":{"message":"Incorrect API key provided: sk-proj-Abc123Def456Ghi789"}}"#
                    .into(),
            ),
        };
        let d = e.diagnostic();
        assert!(d.contains("upstream returned 401"), "诊断要有状态：{d}");
        assert!(!d.contains("sk-proj-Abc"), "key 泄漏进日志/响应了：{d}");
        assert!(d.contains("***"), "应以掩码呈现：{d}");
    }

    #[test]
    fn diagnostic_redacts_bearer_in_body() {
        let e = ProxyError::UpstreamError {
            status: 401,
            body: Some("authorization failed for Bearer abc123def456ghi789".into()),
        };
        let d = e.diagnostic();
        assert!(!d.contains("abc123def456"), "Bearer 泄漏：{d}");
    }
}
