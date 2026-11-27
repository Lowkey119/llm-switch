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
use bytes::Bytes;
use http::{HeaderName, HeaderValue};
use serde_json::Value;
use std::time::Duration;

pub struct Forwarder {
    client: reqwest::Client,
    first_byte: Duration,
    /// 媒体端点（生图/TTS/转写）的首字节档位——上游真的在生成内容，
    /// 几十秒起步是常态，不能用聊天的 first_byte 卡它（见 TimeoutCfg）。
    media_first_byte: Duration,
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
            media_first_byte: Duration::from_millis(timeouts.media_first_byte_ms),
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

    pub fn media_first_byte_timeout(&self) -> Duration {
        self.media_first_byte
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
        self.send_first_byte(req, self.first_byte).await
    }

    /// 原始体透传（媒体端点 11b）：bytes 进、上游响应头就绪即返回，响应体由
    /// 调用方流式回传。Content-Type 由调用方放进 `headers`——multipart 的
    /// boundary 就在里面，摘了上游没法解包。首字节走**媒体档位**：生图 / TTS
    /// 合成是上游真的在算，几十秒起步是常态，别拿聊天的闸门卡它。
    pub async fn post_raw(
        &self,
        url: &str,
        headers: &[(HeaderName, HeaderValue)],
        body: Bytes,
    ) -> Result<reqwest::Response, ProxyError> {
        let mut req = self.client.post(url);
        for (name, value) in headers {
            req = req.header(name.clone(), value.clone());
        }
        let req = req.body(body);
        self.send_first_byte(req, self.media_first_byte).await
    }

    /// 原始体 GET（视频任务轮询/取件 12）：响应头就绪即返回，体由调用方处理。
    /// 首字节走媒体档位——取件是大文件，轮询虽快但共用一条纪律即可。
    pub async fn get_raw(
        &self,
        url: &str,
        headers: &[(HeaderName, HeaderValue)],
    ) -> Result<reqwest::Response, ProxyError> {
        let mut req = self.client.get(url);
        for (name, value) in headers {
            req = req.header(name.clone(), value.clone());
        }
        self.send_first_byte(req, self.media_first_byte).await
    }

    /// 共同的发送腿：超时到点即判超时，连接错误归 `NetworkError` 供择路重试。
    async fn send_first_byte(
        &self,
        req: reqwest::RequestBuilder,
        first_byte: Duration,
    ) -> Result<reqwest::Response, ProxyError> {
        match tokio::time::timeout(first_byte, req.send()).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => Err(map_reqwest_error(e)),
            Err(_) => Err(ProxyError::Timeout(format!(
                "upstream did not send response headers within {}ms",
                first_byte.as_millis()
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
