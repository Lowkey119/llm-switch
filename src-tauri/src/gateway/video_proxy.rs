//! 视频任务亲和代理（媒体端点透传代理 · 切片 B / 12）。
//!
//! 视频生成没有统一协议、任务 id 是后端私有的 → 只能做**任务亲和代理**：
//! - `POST /v1/videos`（建任务）→ 按工种类 `video_gen` 选后端，转发成功后把
//!   上游任务 id 登记进 `job_id → 后端` 内存亲和表，响应里的任务 id 换成
//!   网关命名空间 id（`{后端名}:{上游任务id}`，防跨后端撞号）；
//! - `GET /v1/videos/:id`、`GET /v1/videos/:id/content` 按表路由回同一后端。
//!
//! 红线（计划拍板）：
//! - 亲和表是**内存表，网关重启即失**（任务生命周期短，不做持久化）——
//!   重启后旧任务 id 查不到，404 里如实说明；
//! - 不做协议互转、不转码；状态体透传（OpenAI 风格上游已把进度归一：
//!   `queued/in_progress/completed/failed` + `progress 0-100`，unsloth Studio
//!   的 `/v1/videos` 即此形状，网关原样回传）。
//!
//! 引擎适配（薄）：按后端协议映射路径与任务 id 字段——
//! - OpenAI 风格（unsloth Studio、halogen 等）：`POST /v1/videos` →
//!   `GET /v1/videos/{job}` → `GET /v1/videos/{job}/content`，任务 id 字段 `id`；
//! - 云 ComfyUI：`POST /prompt` → `prompt_id`；`GET /history/{job}` 轮询
//!   （响应以任务 id 为顶层键）；取件走产物 `GET /view?filename=…`。
//!
//! 熔断/日志/记账同切片 A 口径：视频**按任务数计**（建任务落一行用量，
//! 轮询与取件不重复计）；工种类互斥焊死——`video_gen` 永不进聊天择路。

use super::backends::adapter::{adapter_for, join_path};
use super::circuit_breaker::HalfOpenPermit;
use super::error::ProxyError;
use super::model_kind::{self, ModelKind};
use super::response_processor::guarded;
use super::usage::logger;
use super::{handlers, log, State};
use crate::types::{BackendRow, Protocol};
use axum::body::Body;
use axum::extract::{Path, RawQuery, State as AxumState};
use axum::http::{header, HeaderValue};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use http::HeaderName;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

/// 亲和表条目 TTL：视频任务生命周期以小时计，超时视为过期可清。
const JOB_TTL: Duration = Duration::from_secs(24 * 3600);

/// 视频引擎路径映射（薄适配器）。键是后端协议字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEngine {
    /// OpenAI Videos API 形状（unsloth Studio `/v1/videos`、halogen 等）。
    Openai,
    /// 云 ComfyUI worker（`/prompt` → `/history` 轮询 → `/view` 取件）。
    ComfyUi,
}

pub fn video_engine_of(protocol: &str) -> VideoEngine {
    match Protocol::parse(protocol) {
        Some(Protocol::ComfyUI) => VideoEngine::ComfyUi,
        _ => VideoEngine::Openai,
    }
}

impl VideoEngine {
    fn create_path(self) -> &'static str {
        match self {
            VideoEngine::Openai => "/v1/videos",
            VideoEngine::ComfyUi => "/prompt",
        }
    }

    fn status_path(self, job: &str) -> String {
        match self {
            VideoEngine::Openai => format!("/v1/videos/{job}"),
            VideoEngine::ComfyUi => format!("/history/{job}"),
        }
    }

    /// 建任务响应里的任务 id 字段。
    fn job_id_field(self) -> &'static str {
        match self {
            VideoEngine::Openai => "id",
            VideoEngine::ComfyUi => "prompt_id",
        }
    }

    /// 把上游响应里的任务 id 换成网关命名空间 id（客户端只认网关 id）。
    /// OpenAI 形状：`id` 字段；ComfyUI：建任务响应换 `prompt_id`，
    /// `/history` 响应以任务 id 为顶层键 → 换键。
    fn rewrite_ids(self, v: &mut Value, job: &str, namespaced: &str) {
        match self {
            VideoEngine::Openai => {
                if v.get("id").is_some() {
                    v["id"] = json!(namespaced);
                }
            }
            VideoEngine::ComfyUi => {
                if v.get("prompt_id").is_some() {
                    v["prompt_id"] = json!(namespaced);
                } else if let Some(map) = v.as_object_mut() {
                    if let Some(entry) = map.remove(job) {
                        map.insert(namespaced.to_string(), entry);
                    }
                }
            }
        }
    }
}

/// 一条任务亲和记录：命名空间 id → 该回哪个后端的哪个上游任务。
#[derive(Debug, Clone)]
pub struct JobRoute {
    pub backend_id: i64,
    pub backend_name: String,
    pub job: String,
    pub created: SystemTime,
}

/// 任务亲和表（内存，网关重启即失——计划拍板，任务生命周期短不做持久化）。
/// 命名空间 `{后端名}:{上游任务id}`：后端名全库唯一，跨后端撞号不可能。
#[derive(Default)]
pub struct JobTable {
    inner: Mutex<HashMap<String, JobRoute>>,
}

impl JobTable {
    /// 登记并返回命名空间 id；登记顺带清过期条目（表不该无界增长）。
    pub fn register(&self, backend_id: i64, backend_name: &str, job: &str) -> String {
        let key = format!("{backend_name}:{job}");
        let mut g = self.inner.lock().expect("video jobs lock poisoned");
        g.retain(|_, r| r.created.elapsed().map(|d| d < JOB_TTL).unwrap_or(true));
        g.insert(
            key.clone(),
            JobRoute {
                backend_id,
                backend_name: backend_name.to_string(),
                job: job.to_string(),
                created: SystemTime::now(),
            },
        );
        key
    }

    pub fn lookup(&self, key: &str) -> Option<JobRoute> {
        let g = self.inner.lock().expect("video jobs lock poisoned");
        g.get(key)
            .filter(|r| r.created.elapsed().map(|d| d < JOB_TTL).unwrap_or(true))
            .cloned()
    }

    #[cfg(test)]
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.inner.lock().expect("video jobs lock poisoned").len()
    }
}

/// 视频任务入口：与媒体端点同一套暂停闸门与归属识别中间件。
pub fn router(state: Arc<State>) -> axum::Router<Arc<State>> {
    axum::Router::new()
        .route("/v1/videos", post(videos_create))
        .route("/v1/videos/:id", get(video_status))
        .route("/v1/videos/:id/content", get(video_content))
        .route_layer(from_fn_with_state(
            state.clone(),
            handlers::reject_when_paused,
        ))
        .layer(from_fn_with_state(state, handlers::identify_client))
}

/// 建任务：读 JSON 体取 `model` 路由键 → `video_gen` 择路 → 熔断闸门 →
/// 逐候选转发 → 登记亲和 + 命名空间改写 + 用量/metrics/日志（按任务数计）。
async fn videos_create(
    AxumState(st): AxumState<Arc<State>>,
    req: axum::extract::Request,
) -> Result<Response, ProxyError> {
    // 发起方代理名（identify_client 挂进 extensions，into_body 前取）。
    let client = req
        .extensions()
        .get::<super::origin::Origin>()
        .map(|o| o.app.clone());
    let limit = st.config.server.max_body_bytes();
    let body = axum::body::to_bytes(req.into_body(), limit)
        .await
        .map_err(|e| ProxyError::PayloadTooLarge(format!("视频建任务请求体超限或读取失败：{e}")))?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|e| ProxyError::InvalidRequest(format!("视频建任务请求体需为 JSON：{e}")))?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            ProxyError::InvalidRequest(
                "视频建任务需要 `model` 字段（路由键；工种类须声明为 video_gen）".into(),
            )
        })?
        .to_string();
    let rid = uuid::Uuid::new_v4().to_string();
    let began = Instant::now();

    let rows = st.store.list_backends().unwrap_or_default();
    let targets: Vec<(BackendRow, String)> = rows
        .iter()
        .filter(|b| b.enabled)
        .filter_map(|b| {
            if model_kind::request_kind(&b.model_flags, &model) != ModelKind::VideoGen {
                return None;
            }
            b.resolve_upstream(&model).map(|up| (b.clone(), up))
        })
        .collect();
    if targets.is_empty() {
        // 三种「没得选」分开报，口径与媒体透传（11b）一致。
        let declared = rows.iter().any(|b| b.resolve_upstream(&model).is_some());
        let declared_kind = rows.iter().any(|b| {
            b.resolve_upstream(&model).is_some()
                && model_kind::request_kind(&b.model_flags, &model) == ModelKind::VideoGen
        });
        let (err, code, message) = if !declared {
            (
                ProxyError::ModelNotFound(model.clone()),
                "VD404 video.unknown_model",
                "no configured backend declares this model",
            )
        } else if !declared_kind {
            (
                ProxyError::InvalidRequest(format!(
                    "模型 `{model}` 的工种类不是 video_gen；在后端模型列表里人工声明工种后再试"
                )),
                "VD400 video.kind_mismatch",
                "model kind is not video_gen",
            )
        } else {
            (
                ProxyError::NoBackendAvailable,
                "VD503 video.no_backend",
                "no enabled backend can serve this video model right now",
            )
        };
        st.log.log(log::Event {
            level: log::Level::Warn,
            code,
            message,
            request_id: Some(&rid),
            backend: None,
            model: Some(&model),
            extra: json!({}),
        });
        return Err(err);
    }

    let routing = st.routing.read().expect("routing lock poisoned").clone();
    let cb_on = routing.circuit_breaker.enabled;
    let max_attempts = 1 + routing.retries;
    let mut last_err = ProxyError::NoBackendAvailable;
    let mut attempts = 0u32;
    let mut last_backend = String::new();
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
                    extra: json!({ "endpoint": "/v1/videos" }),
                });
                continue;
            }
            guard = HalfOpenPermit::from_allow(Some(br.clone()), allow.used_half_open_permit);
        } else {
            guard = HalfOpenPermit::none();
        }
        attempts += 1;
        last_backend = backend.name.clone();
        match forward_create(&st, &backend, &upstream, &value).await {
            Ok(job_value) => {
                if let Some(br) = &breaker {
                    br.record_success(false).await;
                }
                guard.release();
                let dur = began.elapsed().as_millis() as u64;
                record_video(&st, &client, &backend.name, &model, true, 200, dur, None);
                st.log.log(log::Event {
                    level: log::Level::Info,
                    code: "VD200 video.job_created",
                    message: "video job created",
                    request_id: Some(&rid),
                    backend: Some(&backend.name),
                    model: Some(&model),
                    extra: json!({
                        "job": job_value.get("gateway_job").and_then(Value::as_str),
                        "attempts": attempts,
                        "duration_ms": dur,
                    }),
                });
                return Ok(axum::Json(job_value).into_response());
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
                        "VD5xx video.retry"
                    } else if healthy {
                        "VD4xx video.rejected"
                    } else {
                        "VD5xx video.create_failed"
                    },
                    message: &err_msg,
                    request_id: Some(&rid),
                    backend: Some(&backend.name),
                    model: Some(&model),
                    extra: json!({ "attempt": attempts, "retried": retriable }),
                });
                last_err = e;
                if !retriable {
                    break;
                }
            }
        }
    }
    // 建任务失败也落一行（请求计数诚实；视频按任务数计）。
    if attempts > 0 {
        let dur = began.elapsed().as_millis() as u64;
        record_video(
            &st,
            &client,
            &last_backend,
            &model,
            false,
            last_err.status_code().as_u16(),
            dur,
            Some(last_err.diagnostic()),
        );
    }
    Err(last_err)
}

/// 单后端建任务：转发 → 取上游任务 id → 登记亲和 → 命名空间改写。
/// 返回给客户端的 JSON 里额外带 `gateway_job`（命名空间 id，方便客户端直取）。
async fn forward_create(
    st: &Arc<State>,
    backend: &BackendRow,
    upstream: &str,
    body: &Value,
) -> Result<Value, ProxyError> {
    let engine = video_engine_of(&backend.protocol);
    let url = join_path(&backend.base_url, engine.create_path());
    let headers = auth_headers(backend)?;
    let mut outbound = body.clone();
    outbound["model"] = json!(upstream);
    let resp = st.forwarder.post_json(&url, &headers, &outbound).await?;
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
    let mut v: Value = resp
        .json()
        .await
        .map_err(|e| ProxyError::Internal(format!("上游视频任务响应不是 JSON: {e}")))?;
    let field = engine.job_id_field();
    let job = v
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProxyError::Internal(format!("上游建任务未返回任务 id（缺 `{field}`）")))?
        .to_string();
    let namespaced = st.video_jobs.register(backend.id, &backend.name, &job);
    engine.rewrite_ids(&mut v, &job, &namespaced);
    v["gateway_job"] = json!(namespaced);
    Ok(v)
}

/// 轮询任务状态：按亲和表回同一后端，状态体透传（进度归一在上游）。
/// 熔断只记成败不设闸——任务建了就必须能轮询到终态，卡住轮询等于把任务
/// 永远悬在「生成中」；轮询失败照样喂熔断，影响的是后续建任务的候选。
async fn video_status(
    AxumState(st): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Response, ProxyError> {
    let (route, backend) = resolve_job(&st, &id)?;
    let engine = video_engine_of(&backend.protocol);
    let url = join_path(&backend.base_url, &engine.status_path(&route.job));
    let headers = auth_headers(&backend)?;
    let breaker = crate::gateway::breaker_for(&st, backend.id);
    match st.forwarder.get_raw(&url, &headers).await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                breaker.record_success(false).await;
            } else {
                breaker.record_failure(false).await;
            }
            let mut v: Value = resp
                .json()
                .await
                .map_err(|e| ProxyError::Internal(format!("上游任务状态不是 JSON: {e}")))?;
            engine.rewrite_ids(&mut v, &route.job, &id);
            Ok(axum::Json(v).into_response())
        }
        Err(e) => {
            if !e.backend_healthy() {
                breaker.record_failure(false).await;
            }
            st.log.log(log::Event {
                level: log::Level::Warn,
                code: "VD5xx video.status_failed",
                message: &e.diagnostic(),
                request_id: None,
                backend: Some(&backend.name),
                model: None,
                extra: json!({ "job": id }),
            });
            Err(e)
        }
    }
}

/// 取件：OpenAI 风格直转 content 端点（query 原样带上，如 `variant=thumbnail`）；
/// ComfyUI 先查 `/history` 拿产物文件名，再转 `/view` 流式回传。
async fn video_content(
    AxumState(st): AxumState<Arc<State>>,
    Path(id): Path<String>,
    RawQuery(q): RawQuery,
) -> Result<Response, ProxyError> {
    let (route, backend) = resolve_job(&st, &id)?;
    let engine = video_engine_of(&backend.protocol);
    let headers = auth_headers(&backend)?;
    let breaker = crate::gateway::breaker_for(&st, backend.id);
    let url = match engine {
        VideoEngine::Openai => {
            let mut u = join_path(&backend.base_url, &engine.status_path(&route.job));
            u.push_str("/content");
            if let Some(q) = q.as_deref().filter(|q| !q.is_empty()) {
                u.push('?');
                u.push_str(q);
            }
            u
        }
        VideoEngine::ComfyUi => {
            let hist_url = join_path(&backend.base_url, &engine.status_path(&route.job));
            let resp = st.forwarder.get_raw(&hist_url, &headers).await?;
            if !resp.status().is_success() {
                return Err(ProxyError::UpstreamError {
                    status: resp.status().as_u16(),
                    body: None,
                });
            }
            let hist: Value = resp
                .json()
                .await
                .map_err(|e| ProxyError::Internal(format!("ComfyUI history 不是 JSON: {e}")))?;
            let qs = comfyui_artifact_query(&hist).ok_or_else(|| {
                ProxyError::Internal("ComfyUI 任务还没有可取件的产物（仍在生成或已失败）".into())
            })?;
            format!("{}?{}", join_path(&backend.base_url, "/view"), qs)
        }
    };
    // 取件失败不喂熔断记账：这里是同步流式上下文，且取件成败不改变
    // 「该后端还能不能建任务」的判断——熔断口径以建任务与轮询为准。
    let resp = st.forwarder.get_raw(&url, &headers).await?;
    if !resp.status().is_success() {
        breaker.record_failure(false).await;
        return Err(ProxyError::UpstreamError {
            status: resp.status().as_u16(),
            body: None,
        });
    }
    breaker.record_success(false).await;
    let status = resp.status();
    let out_ct = resp.headers().get(header::CONTENT_TYPE).cloned();
    let stream = guarded(
        resp.bytes_stream(),
        None,
        st.forwarder.idle_timeout(),
        st.forwarder.total_timeout(),
    );
    let mut builder = axum::http::Response::builder().status(status);
    if let Some(ct) = out_ct {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    builder
        .body(Body::from_stream(stream))
        .map_err(|e| ProxyError::Internal(e.to_string()))
        .map(IntoResponse::into_response)
}

/// 命名空间 id → (亲和记录, 可用后端行)。查不到一律 404 并如实说明原因。
fn resolve_job(st: &Arc<State>, id: &str) -> Result<(JobRoute, BackendRow), ProxyError> {
    let route = st.video_jobs.lookup(id).ok_or_else(|| {
        ProxyError::ModelNotFound(format!(
            "{id}（视频任务亲和是内存表：网关重启即失，或任务已过期）"
        ))
    })?;
    let backend = st
        .store
        .list_backends()
        .unwrap_or_default()
        .into_iter()
        .find(|b| b.id == route.backend_id)
        .ok_or(ProxyError::NoBackendAvailable)?; // 后端已删除：任务无处可回
    if !backend.enabled {
        return Err(ProxyError::NoBackendAvailable);
    }
    Ok((route, backend))
}

/// 后端鉴权头（无适配器的协议如 comfyui 无鉴权，返回空头）。
fn auth_headers(backend: &BackendRow) -> Result<Vec<(HeaderName, HeaderValue)>, ProxyError> {
    let mut headers: Vec<(HeaderName, HeaderValue)> = Vec::new();
    if let Some(adapter) = adapter_for(&backend.protocol) {
        headers.extend(adapter.auth_headers(backend.api_key.as_deref())?);
    }
    Ok(headers)
}

/// 视频用量落库（走 logger 唯一漏斗；张/秒列不适用 = None，视频按任务数计）。
#[allow(clippy::too_many_arguments)]
fn record_video(
    st: &Arc<State>,
    client: &Option<String>,
    backend: &str,
    model: &str,
    ok: bool,
    status: u16,
    dur_ms: u64,
    error: Option<String>,
) {
    let ctx = logger::UsageContext {
        backend: backend.to_string(),
        model: model.to_string(),
        entry_protocol: "/v1/videos".to_string(),
        stream: false,
        status: status as i64,
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
        req_bytes: 0,
        resp_bytes: 0,
        prefill_ms: None,
        cache_status: None,
        client: client.clone(),
        timings: None,
        media_images: None,
        media_seconds: None,
    };
    logger::record(&st.store, Some(&st.metrics), &ctx, None);
    st.metrics
        .on_media_completion(ModelKind::VideoGen.as_str(), ok, dur_ms);
}

/// ComfyUI `/history/{job}` 响应里取第一个产物，拼 `/view` 查询串。
/// 形状：`{"<job>": {"outputs": {"<node>": {"images": [{filename, subfolder, type}]}}}}`
/// （视频常挂在 `images`/`gifs` 键下，逐键扫，取第一个带 filename 的）。
fn comfyui_artifact_query(history: &Value) -> Option<String> {
    let entry = history.as_object()?.values().next()?;
    let outputs = entry.get("outputs")?.as_object()?;
    for node in outputs.values() {
        let arrays: Vec<&Value> = match node {
            Value::Object(map) => map.values().collect(),
            Value::Array(_) => vec![node],
            _ => continue,
        };
        for arr in arrays {
            let Some(items) = arr.as_array() else {
                continue;
            };
            for it in items {
                if let Some(f) = it.get("filename").and_then(Value::as_str) {
                    let sub = it.get("subfolder").and_then(Value::as_str).unwrap_or("");
                    let ty = it.get("type").and_then(Value::as_str).unwrap_or("output");
                    return Some(format!(
                        "filename={}&subfolder={}&type={}",
                        qenc(f),
                        qenc(sub),
                        qenc(ty)
                    ));
                }
            }
        }
    }
    None
}

/// 查询串最小百分号编码（无新依赖；非安全字符一律 %XX）。
fn qenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_mapping_and_paths() {
        assert_eq!(video_engine_of("comfyui"), VideoEngine::ComfyUi);
        assert_eq!(video_engine_of("openai_chat"), VideoEngine::Openai);
        assert_eq!(video_engine_of("gemini"), VideoEngine::Openai);
        assert_eq!(VideoEngine::Openai.create_path(), "/v1/videos");
        assert_eq!(VideoEngine::ComfyUi.create_path(), "/prompt");
        assert_eq!(VideoEngine::Openai.status_path("j1"), "/v1/videos/j1");
        assert_eq!(VideoEngine::ComfyUi.status_path("j1"), "/history/j1");
    }

    #[test]
    fn rewrite_ids_namespaces_upstream_job() {
        // OpenAI 形状：id 字段换命名空间 id。
        let mut v = json!({"id": "job-1", "status": "queued"});
        VideoEngine::Openai.rewrite_ids(&mut v, "job-1", "lab:job-1");
        assert_eq!(v["id"], "lab:job-1");
        // ComfyUI 建任务：prompt_id 字段。
        let mut c = json!({"prompt_id": "p-9", "node_errors": {}});
        VideoEngine::ComfyUi.rewrite_ids(&mut c, "p-9", "comf:p-9");
        assert_eq!(c["prompt_id"], "comf:p-9");
        // ComfyUI /history：顶层键换命名空间键。
        let mut h = json!({"p-9": {"outputs": {}}});
        VideoEngine::ComfyUi.rewrite_ids(&mut h, "p-9", "comf:p-9");
        assert!(h.get("comf:p-9").is_some());
        assert!(h.get("p-9").is_none());
    }

    #[test]
    fn job_table_register_lookup_and_namespace() {
        let t = JobTable::default();
        let k1 = t.register(1, "lab", "job-1");
        let k2 = t.register(2, "comf", "job-1");
        assert_eq!(k1, "lab:job-1");
        // 同名任务跨后端不撞号（命名空间的意义）。
        assert_eq!(k2, "comf:job-1");
        assert_eq!(t.lookup("lab:job-1").unwrap().backend_id, 1);
        assert_eq!(t.lookup("comf:job-1").unwrap().backend_id, 2);
        assert!(t.lookup("ghost:job-1").is_none());
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn comfyui_artifact_query_picks_first_file() {
        let h = json!({"p-9": {"outputs": {"9": {"images": [
            {"filename": "v 00001.mp4", "subfolder": "out/dir", "type": "output"}
        ]}}}});
        assert_eq!(
            comfyui_artifact_query(&h).unwrap(),
            "filename=v%2000001.mp4&subfolder=out%2Fdir&type=output"
        );
        // 无产物 / 空 outputs → None（不瞎拼 URL）。
        assert!(comfyui_artifact_query(&json!({"p-9": {"outputs": {}}})).is_none());
        assert!(comfyui_artifact_query(&json!({})).is_none());
    }

    #[test]
    fn qenc_escapes_query_unsafe_bytes() {
        assert_eq!(qenc("a-b_C.~1"), "a-b_C.~1");
        assert_eq!(qenc("a b&c=d"), "a%20b%26c%3Dd");
    }
}
