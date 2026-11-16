//! Unified exit for proxied exchanges: streaming and non-streaming alike.
//!
//! Responsibilities here:
//! - enforce idle/total timeouts on the byte stream (connect/first-byte were
//!   enforced by the forwarder);
//! - optionally translate the SSE stream to the entry protocol;
//! - tap the *native* upstream events to extract token usage and persist it
//!   exactly once when the exchange completes — including client aborts,
//!   which drop the stream and fire the recorder's `Drop`.

use crate::gateway::backends::adapter::{ProviderAdapter, TransformCtx};
use crate::gateway::backends::chat_anthropic::create_chat_sse_stream_from_anthropic;
use crate::gateway::backends::gemini_inbound::create_gemini_sse_stream_from_chat;
use crate::gateway::backends::streaming::create_anthropic_sse_stream;
use crate::gateway::backends::streaming_codex_chat::create_responses_sse_stream_from_chat;
use crate::gateway::backends::streaming_gemini::create_anthropic_sse_stream_from_gemini;
use crate::gateway::backends::streaming_responses::create_anthropic_sse_stream_from_responses;
use crate::gateway::backends::transform_gemini::AnthropicToolSchemaHints;
use crate::gateway::error::ProxyError;
use crate::gateway::usage::{
    logger,
    parser::{EngineTimings, TokenUsage},
};
use crate::gateway::State;
use crate::types::Protocol;
use axum::body::Body;
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Value};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Per-exchange metadata shared by both legs.
pub struct ExchangeMeta {
    pub entry: Protocol,
    pub backend_proto: Protocol,
    pub backend: String,
    pub model: String,
    pub started: Instant,
    /// Session id used for Gemini shadow replay (and future affinity).
    pub session_id: String,
    /// Tool schema hints captured from the original Anthropic request body.
    pub gemini_hints: Option<AnthropicToolSchemaHints>,
    /// Shared retry trace (prior failed attempts of this client request).
    pub trace: Option<SharedTrace>,
    /// Rate aggregator fed at write time (None in bare-store tests).
    pub metrics: Option<std::sync::Arc<crate::gateway::metrics::Metrics>>,
    /// 响应缓存键（M5.5）：Some 表示本请求参与缓存且未命中，成功后需写入。
    pub cache_key: Option<String>,
    /// 生效 TTL（秒）。
    pub cache_ttl: i64,
    /// 本请求的缓存口径（miss / bypass:<reason>），写进 usage 行。
    pub cache_status: Option<String>,
    /// 活跃登记槽（M12-C1）：入口处挂号，成功/失败两条路都由它带上；
    /// 流式路径由 UsageRecorder 续着持有，其余路径靠 Drop 自动注销。
    pub active: Option<std::sync::Arc<crate::gateway::monitor::ActiveSlot>>,
}

/// Attempts ledger shared between the retry loop and the usage writer.
pub type SharedTrace = Arc<std::sync::Mutex<Vec<Value>>>;

/// Serialize the attempt ledger for `usage_records.retry_trace`.
pub fn trace_json(trace: &Option<SharedTrace>) -> Option<String> {
    let guard = trace.as_ref()?.lock().ok()?;
    if guard.is_empty() {
        return None;
    }
    serde_json::to_string(&*guard).ok()
}

/// Max bytes scanned in one SSE line before giving up on it.
const LINE_CAP: usize = 1 << 20;
/// Upper bound on retained candidate events per stream.
const EVENT_CAP: usize = 2048;

/// Accumulates raw upstream bytes, picks out SSE `data:` JSON payloads and
/// writes the usage row exactly once on drop (also covers client aborts).
struct UsageRecorder {
    store: Arc<crate::store::Store>,
    backend: String,
    model: String,
    entry: Protocol,
    backend_proto: Protocol,
    status: i64,
    started: Instant,
    line: Vec<u8>,
    line_overflow: bool,
    events: Vec<Value>,
    first_chunk_at: Option<Instant>,
    last_content_at: Option<Instant>,
    content_bytes: Vec<u8>,
    /// Monitor 登记槽（入口挂的号，见 ExchangeMeta.active）。
    active: Option<std::sync::Arc<crate::gateway::monitor::ActiveSlot>>,
    trace: Option<SharedTrace>,
    /// Rate aggregator fed at write time (None in bare-store tests).
    metrics: Option<std::sync::Arc<crate::gateway::metrics::Metrics>>,
    /// 缓存口径透传到 usage 行（miss / bypass:*）。
    cache_status: Option<String>,
}

impl UsageRecorder {
    fn new(store: Arc<crate::store::Store>, meta: &ExchangeMeta, status: u16) -> Self {
        // 不再自己挂号：号在入口就发了（否则 prefill 期间查不到这条交换）。
        let active = meta.active.clone();
        Self {
            store,
            backend: meta.backend.clone(),
            model: meta.model.clone(),
            entry: meta.entry,
            backend_proto: meta.backend_proto,
            status: status as i64,
            started: meta.started,
            line: Vec::new(),
            line_overflow: false,
            events: Vec::new(),
            first_chunk_at: None,
            last_content_at: None,
            content_bytes: Vec::new(),
            active,
            trace: meta.trace.clone(),
            metrics: meta.metrics.clone(),
            cache_status: meta.cache_status.clone(),
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        if !chunk.is_empty() {
            self.last_content_at = Some(Instant::now());
            const CAP: usize = 64 * 1024; // 估算只需样本，封顶 64KB。
            if self.content_bytes.len() < CAP {
                self.content_bytes.extend_from_slice(chunk);
            }
        }
        if self.first_chunk_at.is_none() {
            self.first_chunk_at = Some(Instant::now());
            if let Some(slot) = &self.active {
                slot.mark_first_byte();
            }
        }
        if let Some(slot) = &self.active {
            slot.bump(chunk.len() as u64, false);
        }
        for &b in chunk {
            if b == b'\n' {
                if !self.line_overflow {
                    self.take_line();
                }
                self.line.clear();
                self.line_overflow = false;
            } else if self.line.len() < LINE_CAP {
                self.line.push(b);
            } else {
                self.line_overflow = true;
            }
        }
    }

    fn take_line(&mut self) {
        let line = match std::str::from_utf8(&self.line) {
            Ok(s) => s.trim(),
            Err(_) => return,
        };
        let Some(rest) = line.strip_prefix("data:") else {
            return;
        };
        let payload = rest.trim();
        if payload.is_empty() || payload == "[DONE]" {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        // Keep anything that can carry usage or identity; drop the flood of
        // plain deltas to bound memory.
        let interesting = value.get("usage").is_some()
            || value.get("usageMetadata").is_some()
            || value.pointer("/response/usage").is_some()
            || value.get("message").is_some()
            || value.get("type").and_then(Value::as_str) == Some("message_start")
            // llama.cpp 引擎计时挂在尾块（llama-server 不带 --usage 时可能没有 usage）。
            || value.get("timings").is_some();
        if interesting && self.events.len() < EVENT_CAP {
            self.events.push(value);
        }
        if let Some(slot) = &self.active {
            // 一个 SSE data 事件 = 一个 chunk，浮条的 dec 速率就是这个口径。
            slot.bump(0, true);
        }
    }

    fn extract_usage(&self) -> Option<TokenUsage> {
        match self.backend_proto {
            Protocol::OpenaiChat => TokenUsage::from_openai_stream_events(&self.events),
            Protocol::OpenaiResponses => TokenUsage::from_codex_stream_events_auto(&self.events),
            Protocol::Gemini => TokenUsage::from_gemini_stream_chunks(&self.events),
            _ => TokenUsage::from_claude_stream_events(&self.events),
        }
    }

    /// llama.cpp 引擎计时：倒扫保留块（timings 挂在尾块），取第一个能解析出的。
    fn extract_timings(&self) -> Option<EngineTimings> {
        self.events
            .iter()
            .rev()
            .find_map(|e| EngineTimings::from_value(e.get("timings")))
    }
}

impl Drop for UsageRecorder {
    fn drop(&mut self) {
        let ttft_ms = self
            .first_chunk_at
            .map(|at| (at - self.started).as_millis() as i64);
        let ctx = logger::UsageContext {
            backend: std::mem::take(&mut self.backend),
            model: std::mem::take(&mut self.model),
            entry_protocol: self.entry.as_str().to_string(),
            stream: true,
            status: self.status,
            ok: (200..300).contains(&self.status),
            ttft_ms,
            duration_ms: Some(self.started.elapsed().as_millis() as i64),
            error: None,
            retry_trace: trace_json(&self.trace),
            decode_tps: None,
            decode_span_ms: None,
            estimated: false,
            estimated_output: None,
            est_input_tokens: 0,
            rate_model: None,
            req_bytes: 0,
            resp_bytes: 0,
            prefill_ms: None,
            cache_status: self.cache_status.clone(),
            client: self.active.as_ref().and_then(|s| s.app.clone()),
            timings: None,
        };
        let mut usage = self.extract_usage();
        // llama.cpp 引擎真值通道：usage 缺席/为零时用 timings 回填真值 token
        //（输入把 cache_n 加回，对齐 usage.prompt_tokens 语义，口径同 Unsloth
        // `_backfill_usage_from_timings`）——引擎实测不是估算，estimated 保持 false。
        let timings = self.extract_timings();
        if let Some((input, output)) = timings.as_ref().and_then(|t| t.usage_pair()) {
            let u = usage.get_or_insert_with(TokenUsage::default);
            if u.input_tokens == 0 && input > 0 {
                u.input_tokens = input.min(u32::MAX as u64) as u32;
            }
            if u.output_tokens == 0 && output > 0 {
                u.output_tokens = output.min(u32::MAX as u64) as u32;
            }
        }
        // 口径（ARCHITECTURE §6）：decode_tps = 输出 token / (末次内容 − 首次内容)；
        // 上游没报 usage 时用字节启发式估算并打 estimated 标记。
        let decode_span = match (self.first_chunk_at, self.last_content_at) {
            (Some(f), Some(l)) if l > f => Some((l - f).as_secs_f64()),
            _ => None,
        };
        let mut ctx = ctx;
        let real_out = usage.as_ref().map(|u| u.output_tokens).unwrap_or(0);
        // 跨度毫秒与 decode_tps 同源同锚，成对带给模型桶算 tok/s。
        ctx.decode_span_ms = decode_span.map(|s| (s * 1000.0).round() as u64);
        if real_out > 0 {
            ctx.decode_tps = decode_span.map(|s| real_out as f64 / s.max(0.001));
        } else {
            let est = crate::gateway::usage::parser::estimate_tokens(&self.content_bytes);
            if est > 0 {
                ctx.estimated = true;
                ctx.estimated_output = Some(est);
                ctx.decode_tps = decode_span.map(|s| est as f64 / s.max(0.001));
            }
        }
        // 引擎自测跨度（predicted_ms）优先于线时钟：不掺 SSE 投递抖动与
        // 投机解码的事件粒度误差，分子分母同为引擎实测，是封口速率的真值锚。
        if let Some(t) = timings.filter(|t| t.decode_tps().is_some()) {
            ctx.decode_tps = t.decode_tps();
            ctx.decode_span_ms = t.decode_span_ms();
        }
        ctx.timings = timings;
        // 估算输入与流测量随上下文带走：纠偏样本、封口速率回放都从这里取
        //（键必须是槽上的模型名，与浮窗行看到的一致）。
        if let Some(slot) = &self.active {
            ctx.est_input_tokens = slot.est_input;
            ctx.rate_model = Some(slot.model.clone());
            ctx.req_bytes = slot.req_bytes;
            ctx.resp_bytes = slot.stream_bytes();
            ctx.prefill_ms = slot.prefill_span().map(|d| d.as_millis() as u64);
        }
        logger::record(&self.store, self.metrics.as_deref(), &ctx, usage.as_ref());
        // 登记槽由 Arc 的 Drop 注销，这里不用手工收尾。
    }
}

/// Wrap the raw upstream byte stream: enforces idle + total timeouts and owns
/// the usage recorder. Yields `io::Error` items so it composes with both the
/// pass-through and the Anthropic SSE transformer.
fn guarded(
    raw: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    recorder: UsageRecorder,
    idle: Duration,
    total: Duration,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut rec = recorder;
        let mut raw = Box::pin(raw);
        let started = Instant::now();
        loop {
            let elapsed = started.elapsed();
            if elapsed >= total {
                yield Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("stream exceeded total budget of {}ms", total.as_millis()),
                ));
                break;
            }
            let window = idle.min(total - elapsed);
            match tokio::time::timeout(window, raw.next()).await {
                Ok(Some(Ok(bytes))) => {
                    rec.feed(&bytes);
                    yield Ok(bytes);
                }
                Ok(Some(Err(e))) => {
                    yield Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        e.to_string(),
                    ));
                    break;
                }
                Ok(None) => break,
                Err(_) => {
                    yield Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("no upstream data within {}ms", window.as_millis()),
                    ));
                    break;
                }
            }
        }
        // Recorder drops here → usage row written exactly once (early client
        // disconnects drop this generator and fire the same Drop).
    }
}

type OutStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;

/// Turn a successful (2xx) upstream response into the client response,
/// translating formats when the adapter says so.
pub async fn finalize_ok(
    resp: reqwest::Response,
    meta: &ExchangeMeta,
    adapter: &dyn ProviderAdapter,
    st: &Arc<State>,
    idle: Duration,
    total: Duration,
) -> Result<Response, ProxyError> {
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let is_sse = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    if is_sse {
        let recorder = UsageRecorder::new(st.store.clone(), meta, status.as_u16());
        let guarded = guarded(resp.bytes_stream(), recorder, idle, total);
        let out: OutStream = if adapter.needs_transform(meta.entry) {
            match (meta.backend_proto, meta.entry) {
                (Protocol::OpenaiChat, Protocol::Anthropic) => {
                    Box::pin(create_anthropic_sse_stream(guarded))
                }
                (Protocol::OpenaiChat, Protocol::OpenaiResponses) => {
                    Box::pin(create_responses_sse_stream_from_chat(guarded))
                }
                (Protocol::OpenaiResponses, Protocol::Anthropic) => {
                    Box::pin(create_anthropic_sse_stream_from_responses(guarded))
                }
                (Protocol::Anthropic, Protocol::OpenaiChat) => {
                    Box::pin(create_chat_sse_stream_from_anthropic(guarded))
                }
                // 入站 Gemini + OpenAI-chat 上游（M11-B）
                (Protocol::OpenaiChat, Protocol::Gemini) => {
                    Box::pin(create_gemini_sse_stream_from_chat(guarded))
                }
                (Protocol::Gemini, Protocol::Anthropic) => {
                    Box::pin(create_anthropic_sse_stream_from_gemini(
                        guarded,
                        Some(st.gemini_shadow.clone()),
                        Some(meta.backend.clone()),
                        Some(meta.session_id.clone()),
                        meta.gemini_hints.clone(),
                    ))
                }
                _ => {
                    return Err(ProxyError::TransformError(format!(
                        "no streaming transform from {} to {}",
                        meta.backend_proto, meta.entry
                    )));
                }
            }
        } else {
            Box::pin(guarded)
        };

        // 响应缓存（M5.5-A）：旁路录制出口帧，流完整结束后入库；半途而废不存。
        let out: OutStream = match (&meta.cache_key, meta.cache_ttl) {
            (Some(key), ttl) => Box::pin(super::cache::capture_stream(
                out,
                st.cache.clone(),
                super::cache::CaptureSpec {
                    key: key.clone(),
                    producer: meta.backend.clone(),
                    model: meta.model.clone(),
                    ttl_secs: ttl,
                    status: status.as_u16(),
                    entry: meta.entry,
                },
            )),
            (None, _) => out,
        };

        let builder = axum::http::Response::builder()
            .status(status)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            )
            .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
            .header(
                HeaderName::from_static("x-accel-buffering"),
                HeaderValue::from_static("no"),
            );
        let resp = builder
            .body(Body::from_stream(out))
            .map_err(|e| ProxyError::Internal(e.to_string()))?;
        return Ok(resp.into_response());
    }

    // Non-streaming leg: read the body within the remaining total budget.
    let remaining_total = total.saturating_sub(meta.started.elapsed());
    let text = match tokio::time::timeout(remaining_total, resp.text()).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return Err(ProxyError::NetworkError(e.to_string())),
        Err(_) => {
            return Err(ProxyError::Timeout(format!(
                "response body not received within {}ms",
                total.as_millis()
            )))
        }
    };

    let native: Value = if text.is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text)
            .map_err(|e| ProxyError::TransformError(format!("upstream returned non-JSON: {e}")))?
    };

    let mut usage = match meta.backend_proto {
        Protocol::OpenaiChat => TokenUsage::from_openai_response(&native),
        Protocol::OpenaiResponses => TokenUsage::from_codex_response_auto(&native),
        Protocol::Gemini => TokenUsage::from_gemini_response(&native),
        _ => TokenUsage::from_claude_response(&native),
    };
    // llama.cpp 引擎真值通道（非流式挂在响应体顶层 timings），语义同流式腿。
    let timings = EngineTimings::from_value(native.get("timings"));
    if let Some((input, output)) = timings.as_ref().and_then(|t| t.usage_pair()) {
        let u = usage.get_or_insert_with(TokenUsage::default);
        if u.input_tokens == 0 && input > 0 {
            u.input_tokens = input.min(u32::MAX as u64) as u32;
        }
        if u.output_tokens == 0 && output > 0 {
            u.output_tokens = output.min(u32::MAX as u64) as u32;
        }
    }
    let est_output = match usage.as_ref().map(|u| u.output_tokens) {
        Some(0) | None => {
            let e = crate::gateway::usage::parser::estimate_tokens(text.as_bytes());
            (e > 0).then_some(e)
        }
        _ => None,
    };
    let estimated = est_output.is_some();
    let ctx = logger::UsageContext {
        backend: meta.backend.clone(),
        model: meta.model.clone(),
        entry_protocol: meta.entry.as_str().to_string(),
        stream: false,
        status: status.as_u16() as i64,
        ok: status.is_success(),
        ttft_ms: None,
        duration_ms: Some(meta.started.elapsed().as_millis() as i64),
        error: None,
        retry_trace: trace_json(&meta.trace),
        // 非流式没有逐块线时钟：引擎 timings 是 decode 速率/跨度的唯一来源。
        decode_tps: timings.as_ref().and_then(|t| t.decode_tps()),
        decode_span_ms: timings.as_ref().and_then(|t| t.decode_span_ms()),
        estimated,
        estimated_output: est_output,
        // 非流式同样挂测量：prefill 纠偏照喂；没有逐块响应测量（resp_bytes=0），
        // decode 样本不与流式封装混池，封口跨度也没有锚点（prefill_ms=None）。
        est_input_tokens: meta.active.as_ref().map(|s| s.est_input).unwrap_or(0),
        rate_model: meta.active.as_ref().map(|s| s.model.clone()),
        req_bytes: meta.active.as_ref().map(|s| s.req_bytes).unwrap_or(0),
        resp_bytes: 0,
        prefill_ms: None,
        cache_status: meta.cache_status.clone(),
        client: meta.active.as_ref().and_then(|s| s.app.clone()),
        timings,
    };
    logger::record(&st.store, Some(&st.metrics), &ctx, usage.as_ref());

    let out_body = if adapter.needs_transform(meta.entry) {
        let ctx = TransformCtx {
            gemini_shadow: Some(st.gemini_shadow.as_ref()),
            provider_id: &meta.backend,
            session_id: &meta.session_id,
            gemini_hints: meta.gemini_hints.as_ref(),
        };
        adapter.transform_response_ctx(native, meta.entry, &ctx)?
    } else {
        native
    };

    // 响应缓存（M5.5-A）：非流式直接存变换后的出口 body。
    if let Some(key) = &meta.cache_key {
        super::cache::capture_body(
            &st.cache,
            super::cache::CaptureSpec {
                key: key.clone(),
                producer: meta.backend.clone(),
                model: meta.model.clone(),
                ttl_secs: meta.cache_ttl,
                status: status.as_u16(),
                entry: meta.entry,
            },
            &out_body,
        );
    }
    Ok((status, Json(out_body)).into_response())
}

/// Upstream answered with an error status: drain the body (bounded) and
/// surface it as a `ProxyError` so the router can decide to fail over.
pub async fn finalize_error(
    resp: reqwest::Response,
    meta: &ExchangeMeta,
    st: &Arc<State>,
    total: Duration,
) -> ProxyError {
    let status = resp.status().as_u16();
    let remaining = total.saturating_sub(meta.started.elapsed());
    let body = match tokio::time::timeout(remaining, resp.text()).await {
        Ok(Ok(mut t)) => {
            t.truncate(4096);
            Some(t)
        }
        _ => None,
    };
    let ctx = logger::UsageContext {
        backend: meta.backend.clone(),
        model: meta.model.clone(),
        entry_protocol: meta.entry.as_str().to_string(),
        stream: false,
        status: status as i64,
        ok: false,
        ttft_ms: None,
        duration_ms: Some(meta.started.elapsed().as_millis() as i64),
        error: body.clone(),
        retry_trace: trace_json(&meta.trace),
        decode_tps: None,
        decode_span_ms: None,
        estimated: false,
        estimated_output: None,
        est_input_tokens: meta.active.as_ref().map(|s| s.est_input).unwrap_or(0),
        rate_model: meta.active.as_ref().map(|s| s.model.clone()),
        req_bytes: meta.active.as_ref().map(|s| s.req_bytes).unwrap_or(0),
        resp_bytes: 0,
        prefill_ms: None,
        cache_status: meta.cache_status.clone(),
        client: meta.active.as_ref().and_then(|s| s.app.clone()),
        timings: None,
    };
    logger::record(&st.store, Some(&st.metrics), &ctx, None);
    ProxyError::UpstreamError { status, body }
}
