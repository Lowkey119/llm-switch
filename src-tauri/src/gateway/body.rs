//! 带显式上限的 JSON 请求体提取器。
//!
//! 为什么要自己写：axum 的 `Json` 提取器受 `DefaultBodyLimit` 约束，默认只有
//! **2 MiB**，超了就直接回一个 `text/plain` 的 413。Claude Code 之类的客户端只
//! 看状态码，于是把代理自己的 2MB 闸门报成上游那句 "Request too large (max 32MB)"，
//! 把人引向完全错误的方向（去 /compact，其实跟 32MB 无关）。
//!
//! 这里改成：边读边按 `server.max_body_mb` 计数，超限时返回 Anthropic 形状的错误
//! JSON，消息里写清楚**是谁的上限、上限多少、实际多大**。

use super::error::ProxyError;
use super::State;
use axum::extract::{FromRequest, Request};
use futures::TryStreamExt;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct LimitedJson(pub Value);

/// 人话版的超限说明：谁限的、限值、当前大小，以及怎么变小。
fn too_large_msg(got: usize, limit: usize, configured_mb: u32) -> String {
    let mb = |n: usize| n as f64 / (1024.0 * 1024.0);
    format!(
        "请求体约 {:.1} MB，超过 llm-switch 自身的入站上限 {:.1} MB（配置项 server.max_body_mb，当前 {}）。这是代理的闸门而非上游模型的限制；如需更大请调高该配置，或减少图片/附件、缩短上下文。",
        mb(got),
        mb(limit),
        if configured_mb == 0 {
            64
        } else {
            configured_mb
        }
    )
}

#[axum::async_trait]
impl FromRequest<Arc<State>> for LimitedJson {
    type Rejection = ProxyError;

    async fn from_request(req: Request, state: &Arc<State>) -> Result<Self, Self::Rejection> {
        let limit = state.config.server.max_body_bytes();
        let configured_mb = state.config.server.max_body_mb;
        let (_parts, body) = req.into_parts();

        let mut buf: Vec<u8> = Vec::new();
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|e| ProxyError::InvalidRequest(format!("读取请求体失败: {e}")))?
        {
            if buf.len() + chunk.len() > limit {
                return Err(ProxyError::PayloadTooLarge(too_large_msg(
                    buf.len() + chunk.len(),
                    limit,
                    configured_mb,
                )));
            }
            buf.extend_from_slice(&chunk);
        }

        if buf.is_empty() {
            // Gemini 的 `:generateContent` 允许空体；其余入口会在下游报缺字段。
            return Ok(LimitedJson(json!({})));
        }

        let value: Value = serde_json::from_slice(&buf)
            .map_err(|e| ProxyError::InvalidRequest(format!("invalid JSON body: {e}")))?;
        Ok(LimitedJson(value))
    }
}
