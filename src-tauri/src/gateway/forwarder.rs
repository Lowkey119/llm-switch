//! Outbound HTTP leg: one shared reqwest client plus the four timeout
//! classes mandated by the development plan — connect, first byte, stream
//! idle, and total. Connect/first-byte are enforced here; idle/total are
//! enforced on the byte stream by `response_processor`.
//!
//! Client-abort behaviour: the upstream byte stream is piped straight into
//! the axum response body, so when the client disconnects hyper drops the
//! body stream, which drops the reqwest response and releases the upstream
//! connection immediately (well under the 1s budget).

use crate::config::TimeoutCfg;
use crate::gateway::error::ProxyError;
use http::{HeaderName, HeaderValue};
use serde_json::Value;
use std::time::Duration;

pub struct Forwarder {
    client: reqwest::Client,
    first_byte: Duration,
    idle: Duration,
    total: Duration,
}

impl Forwarder {
    pub fn new(timeouts: &TimeoutCfg) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(timeouts.connect_ms))
            .tcp_nodelay(true)
            .build()
            .expect("reqwest client");
        Self {
            client,
            first_byte: Duration::from_millis(timeouts.first_byte_ms),
            idle: Duration::from_millis(timeouts.idle_stream_ms),
            total: Duration::from_millis(timeouts.total_ms),
        }
    }

    pub fn idle_timeout(&self) -> Duration {
        self.idle
    }

    pub fn total_timeout(&self) -> Duration {
        self.total
    }

    /// POST a JSON body and resolve once upstream response *headers* are in.
    /// Enforces the first-byte timeout; connect failures surface as
    /// `NetworkError` so the router can fail over.
    pub async fn post_json(
        &self,
        url: &str,
        headers: &[(HeaderName, HeaderValue)],
        body: &Value,
    ) -> Result<reqwest::Response, ProxyError> {
        let mut req = self.client.post(url);
        for (name, value) in headers {
            req = req.header(name.clone(), value.clone());
        }
        let req = req.json(body);

        match tokio::time::timeout(self.first_byte, req.send()).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => Err(map_reqwest_error(e)),
            Err(_) => Err(ProxyError::Timeout(format!(
                "upstream did not send response headers within {}ms",
                self.first_byte.as_millis()
            ))),
        }
    }
}

/// 把上游错误转成可回传/可落库的错误串。
///
/// 两个约束：① reqwest 的 `Display` 会把**完整 URL**塞进消息，而上游 URL 的
/// query 可能带着客户端的 `?key=`——那句话会一路写进运行日志与用量库
/// （`handlers::finalize_error`），所以先 `without_url()` 再格式化，只留主机+路径；
/// ② 连接/超时分类要在摘 URL 之前判定（`is_connect`/`is_timeout` 与 URL 无关，
/// 但 `without_url` 会消耗掉这个错误值）。
fn map_reqwest_error(e: reqwest::Error) -> ProxyError {
    let is_connect = e.is_connect();
    let is_timeout = e.is_timeout();
    let at = e
        .url()
        .map(|u| format!("{}{}", u.host_str().unwrap_or("?"), u.path()))
        .unwrap_or_default();
    let e = e.without_url();
    let detail = if at.is_empty() {
        format!("{e}")
    } else {
        format!("{e} @ {at}")
    };
    if is_connect {
        ProxyError::NetworkError(format!("connect failed: {detail}"))
    } else if is_timeout {
        ProxyError::Timeout(detail)
    } else {
        ProxyError::NetworkError(detail)
    }
}
