//! 媒体端点透传代理（媒体端点透传代理 11b）。
//!
//! 四个无状态入口，按模型工种类选后端，原始体透传、响应流式回传：
//! - `POST /v1/images/generations`、`POST /v1/images/edits` → `image_gen`
//! - `POST /v1/audio/speech` → `tts`
//! - `POST /v1/audio/transcriptions` → `asr`
//!
//! 红线（计划拍板，越界即范围失控）：
//! - **不做媒体协议互转**（上游本就 OpenAI 风格，路径原样拼到 base_url）；
//! - **不做媒体内容转码**（网关不落地、不解析、不重编码，二进制流式透传）；
//! - **不进响应缓存**（与「带工具调用的请求永不缓存」同级）。
//!
//! 工种类择路：媒体请求只在「声明了该工种且启用」的后端里选（勾选是主过滤，
//! 暴露开关在 `resolve_upstream` 里生效）；反向的「媒体工种不进聊天择路」
//! 在 `handlers::candidates`。记账（张/秒/分钟、metrics、统一日志）在 11c，
//! 本片只落路由 + 透传 + 熔断 + 路由级日志。

use super::backends::adapter::{adapter_for, join_path};
use super::circuit_breaker::HalfOpenPermit;
use super::error::ProxyError;
use super::model_kind::{self, ModelKind};
use super::monitor::ActiveSlot;
use super::response_processor::guarded;
use super::usage::logger;
use super::{handlers, log, State};
use crate::types::BackendRow;
use axum::body::Body;
use axum::extract::State as AxumState;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use bytes::Bytes;
use futures::StreamExt;
use http::HeaderName;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;

/// 媒体推理入口：与聊天入口同一套暂停闸门与归属识别中间件。
pub fn router(state: Arc<State>) -> axum::Router<Arc<State>> {
    axum::Router::new()
        .route("/v1/images/generations", post(images_generations))
        .route("/v1/images/edits", post(images_edits))
        .route("/v1/audio/speech", post(audio_speech))
        .route("/v1/audio/transcriptions", post(audio_transcriptions))
        .route_layer(from_fn_with_state(
            state.clone(),
            handlers::reject_when_paused,
        ))
        .layer(from_fn_with_state(state, handlers::identify_client))
}

async fn images_generations(
    AxumState(st): AxumState<Arc<State>>,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> Result<Response, ProxyError> {
    proxy_media(
        st,
        headers,
        req,
        ModelKind::ImageGen,
        "/v1/images/generations",
    )
    .await
}

async fn images_edits(
    AxumState(st): AxumState<Arc<State>>,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> Result<Response, ProxyError> {
    proxy_media(st, headers, req, ModelKind::ImageGen, "/v1/images/edits").await
}

async fn audio_speech(
    AxumState(st): AxumState<Arc<State>>,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> Result<Response, ProxyError> {
    proxy_media(st, headers, req, ModelKind::Tts, "/v1/audio/speech").await
}

async fn audio_transcriptions(
    AxumState(st): AxumState<Arc<State>>,
    headers: HeaderMap,
    req: axum::extract::Request,
) -> Result<Response, ProxyError> {
    proxy_media(st, headers, req, ModelKind::Asr, "/v1/audio/transcriptions").await
}

/// 透传主流程：读原始体 → 取 model → 工种类择路 → 熔断闸门 → 逐候选转发。
async fn proxy_media(
    st: Arc<State>,
    headers: HeaderMap,
    req: axum::extract::Request,
    kind: ModelKind,
    path: &'static str,
) -> Result<Response, ProxyError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // 发起方代理名与进程号：identify_client 中间件已从 socket 反查挂进 extensions。
    let (client, pid) = req
        .extensions()
        .get::<super::origin::Origin>()
        .map(|o| (Some(o.app.clone()), Some(o.pid)))
        .unwrap_or((None, None));
    // 原始体整包读进内存再透传：媒体请求体（音频/图片）有 max_body_bytes 闸门兜底，
    // 读进来才能既取路由键（model）又原样转发（multipart 不重新编码）。
    let limit = st.config.server.max_body_bytes();
    let body = axum::body::to_bytes(req.into_body(), limit)
        .await
        .map_err(|e| ProxyError::PayloadTooLarge(format!("媒体请求体超限或读取失败：{e}")))?;
    let model = extract_model(&content_type, &body)?;
    let rid = uuid::Uuid::new_v4().to_string();
    let began = Instant::now();

    let rows = st.store.list_backends().unwrap_or_default();
    // 候选 = 启用 + 声明该模型（暴露开关在 resolve_upstream 生效）+ 工种类匹配。
    let targets: Vec<(BackendRow, String)> = rows
        .iter()
        .filter(|b| b.enabled)
        .filter_map(|b| {
            if model_kind::request_kind(&b.model_flags, &model) != kind {
                return None;
            }
            b.resolve_upstream(&model).map(|up| (b.clone(), up))
        })
        .collect();
    if targets.is_empty() {
        // 三种「没得选」分开报，口径与聊天择路一致（不瞎猜）：
        // 没后端声明 → 404；声明了但工种类对不上 → 400（指路人工声明）；
        // 工种类对但全下线/被熔断 → 503。
        let declared = rows.iter().any(|b| b.resolve_upstream(&model).is_some());
        let declared_kind = rows.iter().any(|b| {
            b.resolve_upstream(&model).is_some()
                && model_kind::request_kind(&b.model_flags, &model) == kind
        });
        let (err, code, message) = if !declared {
            (
                ProxyError::ModelNotFound(model.clone()),
                "MP404 media.unknown_model",
                "no configured backend declares this model",
            )
        } else if !declared_kind {
            (
                ProxyError::InvalidRequest(format!(
                    "模型 `{model}` 的工种类不是 {}；在后端模型列表里人工声明工种后再试",
                    kind.as_str()
                )),
                "MP400 media.kind_mismatch",
                "model kind does not match this media endpoint",
            )
        } else {
            (
                ProxyError::NoBackendAvailable,
                "MP503 media.no_backend",
                "no enabled backend can serve this media model right now",
            )
        };
        st.log.log(log::Event {
            level: log::Level::Warn,
            code,
            message,
            request_id: Some(&rid),
            backend: None,
            model: Some(&model),
            extra: json!({ "endpoint": path, "kind": kind.as_str() }),
        });
        return Err(err);
    }

    // 熔断与重试纪律与聊天同口径（名额 RAII 归还、后端健康度决定记账）。
    let routing = st.routing.read().expect("routing lock poisoned").clone();
    let cb_on = routing.circuit_breaker.enabled;
    let max_attempts = 1 + routing.retries;
    let mut last_err = ProxyError::NoBackendAvailable;
    let mut attempts = 0u32;
    let mut last_backend: Option<String> = None;
    for (backend, upstream) in targets {
        if attempts >= max_attempts {
            break;
        }
        let breaker = if cb_on {
            Some(crate::gateway::breaker_for(&st, backend.id))
        } else {
            None
        };
        let mut guard;
        if let Some(br) = &breaker {
            let allow = br.allow_request().await;
            if !allow.allowed {
                st.log.log(log::Event {
                    level: log::Level::Warn,
                    code: "CB110 cb.reject",
                    message: "circuit open, backend skipped",
                    request_id: Some(&rid),
                    backend: Some(&backend.name),
                    model: Some(&model),
                    extra: json!({ "endpoint": path }),
                });
                continue;
            }
            guard = HalfOpenPermit::from_allow(Some(br.clone()), allow.used_half_open_permit);
        } else {
            guard = HalfOpenPermit::none();
        }
        attempts += 1;
        last_backend = Some(backend.name.clone());
        let exch = Exchange {
            rid: &rid,
            model: &model,
            kind,
            path,
            client: client.clone(),
            pid,
            began,
            attempts,
        };
        match forward_media(&st, &backend, &upstream, &content_type, body.clone(), &exch).await {
            Ok(resp) => {
                if let Some(br) = &breaker {
                    br.record_success(false).await;
                }
                guard.release();
                // MP200 交换级日志挪到 MediaRecorder::finish——响应流真正流完
                // （或客户端断开）才算完成，耗时与媒体计量都在那里。
                return Ok(resp);
            }
            Err(e) => {
                let retriable = handlers::retryable(&e);
                let healthy = e.backend_healthy();
                if let Some(br) = &breaker {
                    if healthy {
                        br.record_success(false).await;
                    } else {
                        br.record_failure(false).await;
                    }
                }
                guard.release();
                let err_msg = e.diagnostic();
                st.log.log(log::Event {
                    level: if retriable {
                        log::Level::Warn
                    } else if healthy {
                        log::Level::Info
                    } else {
                        log::Level::Error
                    },
                    code: if retriable {
                        "MP5xx media.retry"
                    } else if healthy {
                        "MP4xx media.rejected"
                    } else {
                        "MP5xx media.forward_failed"
                    },
                    message: &err_msg,
                    request_id: Some(&rid),
                    backend: Some(&backend.name),
                    model: Some(&model),
                    extra: json!({ "endpoint": path, "attempt": attempts, "retried": retriable }),
                });
                last_err = e;
                if !retriable {
                    break;
                }
            }
        }
    }
    // 失败的媒体交换也落一行（与聊天同口径：请求计数保持诚实）。
    // 失败路径不给实时页挂号：请求已经结束，挂上去只会闪一个零速率的空行。
    if attempts > 0 {
        let mut rec = MediaRecorder::new(
            st.clone(),
            rid,
            last_backend.unwrap_or_default(),
            model,
            client,
            path,
            kind,
            began,
            last_err.status_code().as_u16(),
            attempts,
            false,
            None,
        );
        rec.finish(false, Some(last_err.diagnostic()));
    }
    Err(last_err)
}

/// 单后端转发：原始体透传 + 响应流式回传（不落地、不解析媒体内容）。
/// 响应流挂一只计量探针（[`MediaRecorder`]）：客户端照常拿字节，流结束或
/// 客户端断开（Drop）时解析媒体计量并落库 + metrics + 交换级日志。
async fn forward_media(
    st: &Arc<State>,
    backend: &BackendRow,
    upstream: &str,
    content_type: &str,
    body: Bytes,
    exch: &Exchange<'_>,
) -> Result<Response, ProxyError> {
    let url = join_path(&backend.base_url, exch.path);
    let mut headers: Vec<(HeaderName, HeaderValue)> = Vec::new();
    if let Some(adapter) = adapter_for(&backend.protocol) {
        headers.extend(adapter.auth_headers(backend.api_key.as_deref())?);
    }
    // Content-Type 必须原样带上：multipart 的 boundary 就在里面，摘了上游没法解包。
    if !content_type.is_empty() {
        headers.push((
            header::CONTENT_TYPE,
            HeaderValue::from_str(content_type)
                .map_err(|_| ProxyError::InvalidRequest("Content-Type 头不合法".into()))?,
        ));
    }
    let outbound = rewrite_model(content_type, body, exch.model, upstream)?;
    // 入口挂号（跟聊天入口 M12-C1 同款纪律）：登记必须在发出上游请求之前，浮窗/实时页
    // 才看得到上传+推理那一段。媒体没有 token 语义，est_input 记 0，分子只有请求字节；
    // 首块到达后相位转 generating，速率口径就是响应字节/秒（音频流式回传时是真数）。
    // ActiveSlot 是 RAII：MediaRecorder 松手即注销，失败路径也不会漏。
    let req_bytes = outbound.len() as u64;
    let slot = st.monitor.slot(
        &backend.name,
        exch.model,
        exch.kind.as_str(),
        0,
        req_bytes,
        exch.client.clone(),
        exch.pid,
    );
    st.monitor.mark_dispatched(slot.id);
    let resp = match st.forwarder.post_raw(&url, &headers, outbound).await {
        Ok(r) => r,
        Err(e) => {
            drop(slot);
            return Err(e);
        }
    };
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = tokio::time::timeout(st.forwarder.total_timeout(), resp.text())
            .await
            .ok()
            .and_then(|r| r.ok())
            .map(|mut t| {
                t.truncate(4096);
                t
            });
        return Err(ProxyError::UpstreamError { status, body: text });
    }
    // 响应流式回传：音频二进制 / base64 JSON 都不落地；守卫只挂 idle/total 超时，
    // 不挂 token 记账（媒体没有 token 语义）。
    let status = resp.status();
    let out_ct = resp.headers().get(header::CONTENT_TYPE).cloned();
    let is_json = out_ct
        .as_ref()
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .contains("json");
    let mut rec = MediaRecorder::new(
        st.clone(),
        exch.rid.to_string(),
        backend.name.clone(),
        exch.model.to_string(),
        exch.client.clone(),
        exch.path,
        exch.kind,
        exch.began,
        status.as_u16(),
        exch.attempts,
        is_json,
        Some(slot),
    );
    let guarded = guarded(
        resp.bytes_stream(),
        None,
        st.forwarder.idle_timeout(),
        st.forwarder.total_timeout(),
    );
    // 旁路采样：字节原样过手转给客户端，探针只抄一份有界的 JSON 前缀用于计量。
    let stream = async_stream::stream! {
        tokio::pin!(guarded);
        while let Some(item) = guarded.next().await {
            match &item {
                Ok(b) => rec.feed(b),
                Err(e) => rec.stream_err = Some(e.to_string()),
            }
            let failed = item.is_err();
            yield item;
            if failed {
                break;
            }
        }
    };
    let mut builder = axum::http::Response::builder().status(status);
    if let Some(ct) = out_ct {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    builder
        .body(Body::from_stream(stream))
        .map_err(|e| ProxyError::Internal(e.to_string()))
        .map(IntoResponse::into_response)
}

/// 媒体计量采样上限：响应 JSON 在此上限内才解析真数；超限（如整包 base64
/// 的大图响应）记 ———不造假数字，也不做脆弱字节扫描。
const MEDIA_METER_CAP: usize = 8 * 1024 * 1024;

/// 一次媒体请求的交换上下文（转发与计量共用，避免参数爆炸）。
struct Exchange<'a> {
    rid: &'a str,
    model: &'a str,
    kind: ModelKind,
    path: &'static str,
    client: Option<String>,
    /// 发起方进程号（认不出为 None）：与聊天挂号同源，速率出口按它分流。
    pid: Option<u32>,
    began: Instant,
    attempts: u32,
}

/// 响应侧计量记录器：透传流上 tap 字节（不拦客户端拿字节），流结束或客户端
/// 断开（Drop）时解析媒体计量并落库 + metrics + 交换级日志——与聊天
/// `UsageRecorder` 同款 Drop 纪律（客户端中途断开也落账，请求计数保持诚实）。
struct MediaRecorder {
    st: Arc<State>,
    rid: String,
    backend: String,
    model: String,
    client: Option<String>,
    endpoint: &'static str,
    kind: ModelKind,
    began: Instant,
    status: u16,
    attempts: u32,
    json: bool,
    buf: Vec<u8>,
    capped: bool,
    resp_bytes: u64,
    stream_err: Option<String>,
    done: bool,
    /// 观测实时页的在飞登记（失败路径没挂号就是 None）。
    slot: Option<Arc<ActiveSlot>>,
}

impl MediaRecorder {
    #[allow(clippy::too_many_arguments)]
    fn new(
        st: Arc<State>,
        rid: String,
        backend: String,
        model: String,
        client: Option<String>,
        endpoint: &'static str,
        kind: ModelKind,
        began: Instant,
        status: u16,
        attempts: u32,
        json: bool,
        slot: Option<Arc<ActiveSlot>>,
    ) -> Self {
        Self {
            st,
            rid,
            backend,
            model,
            client,
            endpoint,
            kind,
            began,
            status,
            attempts,
            json,
            buf: Vec::new(),
            capped: false,
            resp_bytes: 0,
            stream_err: None,
            done: false,
            slot,
        }
    }

    fn feed(&mut self, b: &[u8]) {
        if let Some(s) = &self.slot {
            if self.resp_bytes == 0 {
                s.mark_first_byte();
            }
            s.bump(b.len() as u64, true);
        }
        self.resp_bytes += b.len() as u64;
        if !self.json || self.capped {
            return;
        }
        if self.buf.len() + b.len() > MEDIA_METER_CAP {
            self.buf.clear();
            self.capped = true;
            return;
        }
        self.buf.extend_from_slice(b);
    }

    /// 封口落账：用量行（走 logger 唯一漏斗）+ 媒体 metrics + 交换级日志。
    fn finish(&mut self, ok: bool, error: Option<String>) {
        if self.done {
            return;
        }
        self.done = true;
        let dur_ms = self.began.elapsed().as_millis() as u64;
        let (images, seconds) = parse_media_metrics(self.kind, self.json, self.capped, &self.buf);
        let ctx = logger::UsageContext {
            backend: self.backend.clone(),
            model: self.model.clone(),
            entry_protocol: self.endpoint.to_string(),
            // 媒体响应不是 token 流：stream=false，不污染流式占比与 TTFT 池。
            stream: false,
            status: self.status as i64,
            ok,
            ttft_ms: None,
            duration_ms: Some(dur_ms as i64),
            error,
            retry_trace: None,
            decode_tps: None,
            decode_span_ms: None,
            estimated: false,
            estimated_output: None,
            est_input_tokens: 0,
            rate_model: None,
            // 媒体没有 token 语义：不喂纠偏样本（req_bytes=0 天然被拦）；响应字节如实记。
            req_bytes: 0,
            resp_bytes: self.resp_bytes,
            prefill_ms: None,
            // 媒体永不进响应缓存（红线），无缓存口径。
            cache_status: None,
            client: self.client.clone(),
            timings: None,
            media_images: images,
            media_seconds: seconds,
        };
        logger::record(&self.st.store, Some(&self.st.metrics), &ctx, None);
        self.st
            .metrics
            .on_media_completion(self.kind.as_str(), ok, dur_ms);
        self.st.log.log(log::Event {
            level: if ok {
                log::Level::Info
            } else {
                log::Level::Warn
            },
            code: if ok {
                "MP200 media.finished"
            } else {
                "MP502 media.stream_failed"
            },
            message: if ok {
                "media exchange finished"
            } else {
                "media exchange failed"
            },
            request_id: Some(&self.rid),
            backend: Some(&self.backend),
            model: Some(&self.model),
            extra: json!({
                "endpoint": self.endpoint,
                "kind": self.kind.as_str(),
                "attempts": self.attempts,
                "duration_ms": dur_ms,
                "images": images,
                "audio_seconds": seconds,
            }),
        });
    }
}

impl Drop for MediaRecorder {
    fn drop(&mut self) {
        let ok = (200..300).contains(&self.status) && self.stream_err.is_none();
        self.finish(ok, self.stream_err.clone());
    }
}

/// 解析媒体计量（计划口径：图像张数、转写时长；取不到一律 None = —，不造假数字）。
/// 只信完整解析的 JSON：解析失败或超采样上限即放弃。
/// - 图像张数：images 端点响应的 `data` 数组长度；
/// - 音频秒数：转写端点响应的 `duration` 字段（whisper verbose_json 口径）。
fn parse_media_metrics(
    kind: ModelKind,
    json: bool,
    capped: bool,
    buf: &[u8],
) -> (Option<i64>, Option<f64>) {
    if !json || capped {
        return (None, None);
    }
    let Ok(v) = serde_json::from_slice::<Value>(buf) else {
        return (None, None);
    };
    let images = (kind == ModelKind::ImageGen)
        .then(|| {
            v.get("data")
                .and_then(Value::as_array)
                .map(|a| a.len() as i64)
        })
        .flatten();
    let seconds = v
        .get("duration")
        .and_then(Value::as_f64)
        .filter(|d| d.is_finite() && *d >= 0.0);
    (images, seconds)
}

/// 从请求体里取 `model`（路由键）。JSON 端点读字段；multipart 端点扫表单字段。
fn extract_model(content_type: &str, body: &[u8]) -> Result<String, ProxyError> {
    if content_type.contains("multipart/form-data") {
        let (s, e) = multipart_field_span(content_type, body, "model").ok_or_else(|| {
            ProxyError::InvalidRequest("multipart 请求体里找不到 `model` 字段".into())
        })?;
        let v = std::str::from_utf8(&body[s..e])
            .map_err(|_| ProxyError::InvalidRequest("`model` 字段不是 UTF-8".into()))?
            .trim()
            .to_string();
        if v.is_empty() {
            return Err(ProxyError::InvalidRequest("missing `model`".into()));
        }
        return Ok(v);
    }
    let value: Value = serde_json::from_slice(body).map_err(|e| {
        ProxyError::InvalidRequest(format!(
            "媒体端点请求体解析失败（既非 JSON 也非 multipart）: {e}"
        ))
    })?;
    value
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProxyError::InvalidRequest("missing `model`".into()))
}

/// 改名/别名场景把请求体里的 `model` 换成上游真名；名字本来就对得上时
/// 一个字节都不动（透传保真）。multipart 只替换 model 字段的值字节，
/// 文件部分原样保留。
fn rewrite_model(
    content_type: &str,
    body: Bytes,
    requested: &str,
    upstream: &str,
) -> Result<Bytes, ProxyError> {
    if requested == upstream {
        return Ok(body);
    }
    if content_type.contains("multipart/form-data") {
        let (s, e) = multipart_field_span(content_type, &body, "model")
            .ok_or_else(|| ProxyError::Internal("multipart 里找不到 model 字段".into()))?;
        let mut out = Vec::with_capacity(body.len() + upstream.len());
        out.extend_from_slice(&body[..s]);
        out.extend_from_slice(upstream.as_bytes());
        out.extend_from_slice(&body[e..]);
        return Ok(Bytes::from(out));
    }
    let mut value: Value = serde_json::from_slice(&body)
        .map_err(|e| ProxyError::Internal(format!("媒体请求体 JSON 解析失败: {e}")))?;
    value["model"] = json!(upstream);
    Ok(Bytes::from(serde_json::to_vec(&value).map_err(|e| {
        ProxyError::Internal(format!("媒体请求体重序列化失败: {e}"))
    })?))
}

/// 从 Content-Type 里取 multipart boundary（可能带引号，畸形分号宽容截断）。
fn boundary_of(content_type: &str) -> Option<String> {
    let lower = content_type.to_ascii_lowercase();
    let at = lower.find("boundary=")? + "boundary=".len();
    let raw = content_type[at..].trim();
    let raw = raw
        .strip_prefix('"')
        .map(|r| r.trim_end_matches('"'))
        .unwrap_or(raw);
    let b = raw.split(';').next().unwrap_or("").trim();
    (!b.is_empty()).then(|| b.to_string())
}

/// 在字节流里找子串（首字节命中才做全量比较——媒体体可达几十 MB，
/// 别拿 windows 全量比）。
fn find_slice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let n0 = needle[0];
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i] == n0 && &hay[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 在 multipart 体里定位字段值（返回值的字节区间）。
/// 只认 `Content-Disposition: form-data; name="field"` 的普通字段；
/// 文件 part 的 name 是 `file`/`image` 等，不会误中。
fn multipart_field_span(content_type: &str, body: &[u8], field: &str) -> Option<(usize, usize)> {
    let b = boundary_of(content_type)?;
    let delim = format!("--{b}");
    let needle = format!("name=\"{field}\"");
    let mut pos = 0;
    while let Some(rel) = find_slice(&body[pos..], delim.as_bytes()) {
        let seg_start = pos + rel + delim.len();
        let seg_end = match find_slice(&body[seg_start..], delim.as_bytes()) {
            Some(r) => seg_start + r,
            None => body.len(),
        };
        // part = 头 \r\n\r\n 值；值末尾的 \r\n 属于分隔符不属于值。
        if let Some(hr) = find_slice(&body[seg_start..seg_end], b"\r\n\r\n") {
            let headers = &body[seg_start..seg_start + hr];
            let val_start = seg_start + hr + 4;
            let mut val_end = seg_end;
            if val_end >= val_start + 2 && &body[val_end - 2..val_end] == b"\r\n" {
                val_end -= 2;
            }
            if find_slice(headers, needle.as_bytes()).is_some() {
                return Some((val_start, val_end));
            }
        }
        if seg_end == body.len() {
            break;
        }
        pos = seg_end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: &str = "----WebKitFormBoundary7MA4YWxkTrZu0gW";

    fn multipart_body(model: &str, file_bytes: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(format!("--{B}\r\n").as_bytes());
        b.extend_from_slice(b"Content-Disposition: form-data; name=\"model\"\r\n\r\n");
        b.extend_from_slice(model.as_bytes());
        b.extend_from_slice(format!("\r\n--{B}\r\n").as_bytes());
        b.extend_from_slice(
            b"Content-Disposition: form-data; name=\"file\"; filename=\"a.mp3\"\r\nContent-Type: audio/mpeg\r\n\r\n",
        );
        b.extend_from_slice(file_bytes);
        b.extend_from_slice(format!("\r\n--{B}--\r\n").as_bytes());
        b
    }

    #[test]
    fn multipart_model_extraction_and_file_untouched() {
        let file = [0u8, 1, 2, 255, b'-', b'-', b'\r', b'\n'];
        let body = multipart_body("qwen3-asr-2b", &file);
        let ct = format!("multipart/form-data; boundary={B}");
        assert_eq!(extract_model(&ct, &body).unwrap(), "qwen3-asr-2b");
        // 引号包 boundary 的写法也认。
        let ct_q = format!("multipart/form-data; boundary=\"{B}\"");
        assert_eq!(extract_model(&ct_q, &body).unwrap(), "qwen3-asr-2b");
        // 文件字节里带 `--` 与 CRLF 也不误切。
        let (s, e) = multipart_field_span(&ct, &body, "file").unwrap();
        assert_eq!(&body[s..e], &file[..]);
    }

    #[test]
    fn multipart_model_missing_is_clear_error() {
        let body = format!(
            "--{B}\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\nx\r\n--{B}--\r\n"
        );
        let ct = format!("multipart/form-data; boundary={B}");
        let err = extract_model(&ct, body.as_bytes()).unwrap_err();
        assert!(matches!(err, ProxyError::InvalidRequest(_)));
    }

    #[test]
    fn json_model_extraction() {
        let body = br#"{"model":"flux2-klein-4b","prompt":"cat"}"#;
        assert_eq!(
            extract_model("application/json", body).unwrap(),
            "flux2-klein-4b"
        );
        // 空 model / 缺 model 都报清晰错误。
        assert!(extract_model("application/json", br#"{"model":""}"#).is_err());
        assert!(extract_model("application/json", br#"{"prompt":"x"}"#).is_err());
        assert!(extract_model("application/json", b"not json").is_err());
    }

    #[test]
    fn media_metrics_only_trust_fully_parsed_json() {
        // 图像张数：data 数组长度。
        let img = br#"{"created":1,"data":[{"b64_json":"AAA"},{"url":"u"}]}"#;
        assert_eq!(
            parse_media_metrics(ModelKind::ImageGen, true, false, img),
            (Some(2), None)
        );
        // 转写时长：duration 字段（whisper verbose_json）。
        let asr = br#"{"text":"hi","duration":61.5}"#;
        assert_eq!(
            parse_media_metrics(ModelKind::Asr, true, false, asr),
            (None, Some(61.5))
        );
        // 非 images 工种不瞎报张数；负时长/NaN 不要。
        assert_eq!(
            parse_media_metrics(ModelKind::Tts, true, false, img),
            (None, None)
        );
        assert_eq!(
            parse_media_metrics(ModelKind::Asr, true, false, br#"{"duration":-1}"#),
            (None, None)
        );
        // 解析失败 / 非 JSON / 超采样上限 → 一律 None（记 —，不造假数字）。
        assert_eq!(
            parse_media_metrics(ModelKind::ImageGen, true, false, b"not json"),
            (None, None)
        );
        assert_eq!(
            parse_media_metrics(ModelKind::ImageGen, false, false, img),
            (None, None)
        );
        assert_eq!(
            parse_media_metrics(ModelKind::ImageGen, true, true, img),
            (None, None)
        );
    }

    #[test]
    fn rewrite_model_only_touches_the_model_field() {
        // JSON：改名场景换成上游真名，其余字段保留。
        let body = Bytes::from_static(br#"{"model":"tts-fast","input":"hi"}"#);
        let out = rewrite_model("application/json", body, "tts-fast", "qwen3-tts-x").unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["model"], "qwen3-tts-x");
        assert_eq!(v["input"], "hi");
        // 名字一致 → 原样透传（同一个 Bytes，零拷贝零改动）。
        let same = Bytes::from_static(br#"{"model":"m"}"#);
        let out = rewrite_model("application/json", same.clone(), "m", "m").unwrap();
        assert_eq!(out.as_ptr(), same.as_ptr());
        // multipart：只换 model 值，文件字节原样。
        let file = [7u8, 8, 9];
        let mp = multipart_body("tts-fast", &file);
        let ct = format!("multipart/form-data; boundary={B}");
        let out = rewrite_model(&ct, Bytes::from(mp.clone()), "tts-fast", "qwen3-tts-x").unwrap();
        assert!(out.windows(file.len()).any(|w| w == file));
        assert_eq!(extract_model(&ct, &out).unwrap(), "qwen3-tts-x");
        assert!(!out.windows(b"tts-fast".len()).any(|w| w == b"tts-fast"));
    }
}
