//! Inference entrypoints: `POST /v1/messages`, `POST /v1/messages/count_tokens`
//! and `POST /v1/chat/completions`. They share one pipeline:
//! resolve model → pick backend(s) → transform → forward → process exit.

use super::backends::adapter::{adapter_for, TransformCtx};
use super::backends::transform_gemini::extract_anthropic_tool_schema_hints;
use super::body::LimitedJson;
use super::cache;
use super::error::ProxyError;
use super::forwarder::Forwarder;
use super::model_kind;
use super::response_processor::{finalize_error, finalize_ok, ExchangeMeta};
use super::State;
use crate::config::RoutingPolicy;
use crate::gateway::usage::{logger, parser::TokenUsage};
use crate::types::{BackendRow, Protocol};
use axum::body::Body;
use axum::extract::State as AxumState;
use axum::http::{header, HeaderMap};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bytes::Bytes;
use http::{HeaderName, HeaderValue, StatusCode};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

/// Ollama 非流式入口把上游响应**整体读进内存**再翻译成 Ollama 形状，这是那次
/// 读取的字节上限。与入站 `server.max_body_mb`（默认 64MiB）不是一个口径：那是
/// 闸门客户端打进来的请求体，这里是闸门上游吐回来的响应体，各自独立。超限时该
/// 入口返回 500（读取上游响应失败），不会截断成半个 JSON。
const MAX_UPSTREAM_BODY_BYTES: usize = 32 * 1024 * 1024;

/// One candidate backend plus the upstream model name to use for it.
struct Target {
    backend: BackendRow,
    upstream_model: String,
}

/// Parsed pieces of a Gemini-style inbound path (`/v1beta/models/<m>:<action>`).
struct GeminiRoute {
    action: String,
    query: String,
}

/// Per-request extras threaded through the forward attempt.
struct AttemptCtx<'a> {
    session_id: &'a str,
    /// 发起方代理（`origin` 从 socket 反查）；None = 认不出。
    app: Option<&'a str>,
    /// 发起方代理进程号（`origin` 一并反查）；速率出口按 pid 链分流同代理多实例。
    pid: Option<u32>,
    gemini_route: Option<&'a GeminiRoute>,
    trace: Option<crate::gateway::response_processor::SharedTrace>,
    request_id: &'a str,
    /// 响应缓存：参与缓存且未命中时的键（成功后写入）。
    cache_key: Option<&'a str>,
    cache_ttl: i64,
    /// 写进 usage 行的缓存口径（miss / bypass:<reason>）。
    cache_status: Option<&'static str>,
}

/// Best-effort session id for shadow/replay state: well-known client headers,
/// then Anthropic `metadata.user_id`, else a fresh id.
fn session_id_of(headers: &HeaderMap, body: &Value) -> String {
    for name in ["x-claude-code-session-id", "x-session-id"] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    if let Some(uid) = body
        .get("metadata")
        .and_then(|m| m.get("user_id"))
        .and_then(Value::as_str)
    {
        if !uid.is_empty() {
            return uid.to_string();
        }
    }
    uuid::Uuid::new_v4().simple().to_string()
}

/// Enabled backends that can serve `model` (directly or via alias),
/// ordered by the configured routing policy.
/// Stable conversation fingerprint for prefix affinity: canonical hash of
/// the message list (any protocol spelling) once it is worth pinning.
fn affinity_prefix(st: &State, body: &Value) -> Option<String> {
    let cfg = &st.config.cache.affinity;
    if !cfg.enabled {
        return None;
    }
    let convo = body.get("messages").or_else(|| body.get("contents"))?;
    let canon = crate::gateway::json_canonical::canonical_json_string(convo);
    if canon.len() < cfg.min_prefix_chars {
        return None;
    }
    Some(crate::gateway::json_canonical::short_sha256_hex(
        canon.as_bytes(),
    ))
}

async fn candidates(st: &State, model: &str, prefix_hash: Option<&str>) -> Vec<Target> {
    let mut hits: Vec<Target> = st
        .store
        .list_backends()
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.enabled)
        .filter_map(|b| {
            // 工种类互斥（11b）：媒体工种（生图/TTS/ASR/视频）永不进聊天择路。
            // 判定口径见 `model_kind::request_kind`——未知默认聊天，宁可放行。
            if model_kind::request_kind(&b.model_flags, model) != model_kind::ModelKind::Chat {
                return None;
            }
            // 暴露开关在这里生效：被关掉的模型对路由来说等于不存在。
            // `BackendRow::resolve_upstream` 一并处理暴露名、别名与改名后的过渡开关。
            b.resolve_upstream(model).map(|upstream_model| Target {
                upstream_model,
                backend: b,
            })
        })
        .collect();

    match st.routing.read().expect("routing lock poisoned").policy {
        RoutingPolicy::WeightedRoundRobin => {
            // Rotate proportionally to weights: pick a rotation offset in the
            // expanded weight space and start the candidate list there.
            let total: i64 = hits.iter().map(|t| t.backend.weight.max(1)).sum();
            if total > 0 {
                let tick = st.rotate.fetch_add(1, Ordering::Relaxed) % total as u64;
                let mut acc: u64 = 0;
                let mut split = 0usize;
                for (i, t) in hits.iter().enumerate() {
                    acc += t.backend.weight.max(1) as u64;
                    if tick < acc {
                        split = i;
                        break;
                    }
                }
                hits.rotate_left(split);
            }
        }
        RoutingPolicy::RoundRobin => {
            if !hits.is_empty() {
                let n = hits.len();
                let off = st.rotate.fetch_add(1, Ordering::Relaxed) % n as u64;
                hits.rotate_left(off as usize);
            }
        }
        // CacheAware 在下方统一用遥测分数决胜。
        RoutingPolicy::Failover | RoutingPolicy::CacheAware => {}
        RoutingPolicy::LeastBusy => {
            // 活跃流数升序，其次近期平均 TTFT 升序（无样本排最后、并列保序）。
            let counts = st.monitor.count_by_backend();
            let mut scored: Vec<(usize, f64, Target)> = hits
                .into_iter()
                .map(|t| {
                    let n = counts.get(&t.backend.name).copied().unwrap_or(0);
                    let ttft = st
                        .store
                        .recent_avg_ttft(&t.backend.name)
                        .ok()
                        .flatten()
                        .unwrap_or(f64::MAX);
                    (n, ttft, t)
                })
                .collect();
            scored.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            });
            hits = scored.into_iter().map(|(_, _, t)| t).collect();
        }
    }

    // 熔断过滤：Open（未到半开窗口）的后端不进候选；HalfOpen 保留，
    // 真正的探测名额在 allow_request 处竞争。
    if st
        .routing
        .read()
        .expect("routing lock poisoned")
        .circuit_breaker
        .enabled
    {
        let mut avail = Vec::with_capacity(hits.len());
        for t in hits {
            let breaker = crate::gateway::breaker_for(st, t.backend.id);
            if breaker.is_available().await {
                avail.push(t);
            } else {
                tracing::debug!(target: "llm_switch::cb", backend = %t.backend.name, "circuit open, excluded from candidates");
            }
        }
        hits = avail;
    }

    // 亲和/缓存感知排序：
    // - cache_aware（M5.5-D）：亲和置信度 ⊕ KV 空闲度 ⊕ 命中率 ⊕ SSD 存活层 − 负载，
    //   分数降序（稳定排序，同分保持上游策略次序）；遥测缺席优雅退化。
    // - 其它策略：同前缀历史后端排最前（稳定排序，其余保持轮转次序）。
    let cache_aware =
        st.routing.read().expect("routing lock poisoned").policy == RoutingPolicy::CacheAware;
    if cache_aware {
        let ttl = st.config.cache.affinity.ttl_secs;
        let aff: Vec<(i64, f64)> = prefix_hash
            .and_then(|h| st.store.affinity_candidates(h, ttl).ok())
            .unwrap_or_default();
        let telem = st.telemetry.read().expect("telemetry lock poisoned");
        let conf = |id: i64| -> f64 {
            aff.iter()
                .find(|(b, _)| *b == id)
                .map(|(_, w)| *w)
                .unwrap_or(0.0)
        };
        let mut scored: Vec<(f64, Target)> = hits
            .into_iter()
            .map(|t| {
                (
                    crate::gateway::telemetry::score(telem.get(&t.backend.id), conf(t.backend.id)),
                    t,
                )
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        hits = scored.into_iter().map(|(_, t)| t).collect();
    } else if let Some(hash) = prefix_hash {
        let ttl = st.config.cache.affinity.ttl_secs;
        let ranked = st.store.affinity_candidates(hash, ttl).unwrap_or_default();
        if !ranked.is_empty() {
            let rank = |id: i64| -> u32 {
                ranked
                    .iter()
                    .position(|(bid, _)| *bid == id)
                    .unwrap_or(u32::MAX as usize) as u32
            };
            hits.sort_by_key(|t| rank(t.backend.id));
        }
    }
    hits
}

/// 日志里展示缓存键前缀就够了（完整键太长会污染日志行）。
fn short_key(key: &str) -> String {
    let n = key
        .char_indices()
        .nth(12)
        .map(|(i, _)| i)
        .unwrap_or(key.len());
    key[..n].to_string()
}

/// 命中回放（M5.5-A）：不碰后端，逐字节还原上次出口响应；usage 行记 hit。
async fn replay_hit(
    st: &Arc<State>,
    entry: Protocol,
    key: &str,
    hit: cache::CachedResponse,
    rid: &str,
    began: Instant,
    app: Option<String>,
) -> Result<Response, ProxyError> {
    st.cache.note_hit(
        hit.input_tokens.max(0) as u64,
        hit.output_tokens.max(0) as u64,
    );
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "CA100 cache.hit",
        message: "response cache hit",
        request_id: Some(rid),
        backend: Some(&hit.producer),
        model: Some(&hit.model),
        extra: json!({
            "key": short_key(key),
            "stream": hit.stream,
            "saved_input": hit.input_tokens,
            "saved_output": hit.output_tokens,
        }),
    });
    let ctx = logger::UsageContext {
        backend: hit.producer.clone(),
        model: hit.model.clone(),
        entry_protocol: entry.as_str().to_string(),
        stream: hit.stream,
        status: hit.status as i64,
        ok: true,
        ttft_ms: None,
        duration_ms: Some(began.elapsed().as_millis() as i64),
        error: None,
        retry_trace: None,
        decode_tps: None,
        decode_span_ms: None,
        estimated: false,
        estimated_output: None,
        est_input_tokens: 0,
        // 缓存回放没有在飞的测量数据：不喂纠偏样本（req_bytes=0 天然被拦）。
        rate_model: None,
        req_bytes: 0,
        resp_bytes: 0,
        prefill_ms: None,
        cache_status: Some("hit".to_string()),
        // 回放不回源，但发起方照样认得出：用量表里别把命中记成匿名。
        client: app,
        // 缓存回放不回源，没有引擎计时可言。
        timings: None,
        media_images: None,
        media_seconds: None,
    };
    let usage = TokenUsage {
        input_tokens: hit.input_tokens.max(0) as u32,
        output_tokens: hit.output_tokens.max(0) as u32,
        ..Default::default()
    };
    logger::record(&st.store, Some(&st.metrics), &ctx, Some(&usage));

    let status = StatusCode::from_u16(hit.status).unwrap_or(StatusCode::OK);
    if hit.stream {
        let bytes = Bytes::from(hit.body.as_ref().clone());
        let stream = async_stream::stream! { yield Ok::<_, std::io::Error>(bytes); };
        let resp = axum::http::Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(HeaderName::from_static("x-accel-buffering"), "no")
            .header(HeaderName::from_static("x-llmswitch-cache"), "hit")
            .body(Body::from_stream(stream))
            .map_err(|e| ProxyError::Internal(e.to_string()))?;
        Ok(resp.into_response())
    } else {
        let value: Value = serde_json::from_slice(&hit.body)
            .map_err(|e| ProxyError::Internal(format!("bad cache entry: {e}")))?;
        let mut resp = (status, Json(value)).into_response();
        resp.headers_mut().insert(
            HeaderName::from_static("x-llmswitch-cache"),
            HeaderValue::from_static("hit"),
        );
        Ok(resp)
    }
}

/// Retryable = transport-level trouble or overloaded/5xx upstream answers.
pub(crate) fn retryable(err: &ProxyError) -> bool {
    match err {
        ProxyError::NetworkError(_) | ProxyError::Timeout(_) => true,
        ProxyError::UpstreamError { status, .. } => *status >= 500 || *status == 429,
        _ => false,
    }
}

async fn forward_once(
    st: &Arc<State>,
    fwd: &Forwarder,
    entry: Protocol,
    target: &Target,
    mut body: Value,
    started: Instant,
    actx: &AttemptCtx<'_>,
) -> Result<Response, ProxyError> {
    let proto = Protocol::parse(&target.backend.protocol)
        .ok_or_else(|| ProxyError::Internal("bad backend protocol".into()))?;
    let adapter = adapter_for(&target.backend.protocol)
        .ok_or_else(|| ProxyError::Internal("unsupported backend protocol".into()))?;

    // 改写前先留一份客户端请求的模型名（= 暴露名/别名）：模型层参数按暴露名存
    // （UI 口径），下一行就把 body.model 换成上游真名，不留这份就会查丢模型层参数。
    let client_model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(&target.upstream_model)
        .to_string();

    if entry != Protocol::Gemini {
        body["model"] = json!(target.upstream_model);
    } else if proto != Protocol::Gemini
        && actx
            .gemini_route
            .as_ref()
            .is_some_and(|r| r.action == "streamGenerateContent")
    {
        // Gemini 把「我要流式」写在 URL 的动作里而不是请求体里；换成 OpenAI-chat
        // 这类上游时必须翻成 body.stream，否则上游按非流式回、我们也拿不到 usage。
        body["stream"] = json!(true);
    }
    // The client's stream intent (some request transforms drop it).
    let client_wants_stream = body.get("stream").and_then(Value::as_bool) == Some(true);

    // 参数中心：请求 > 模型 > 全局 > 后端，在入口形状上单点应用（请求层隐式最高）。
    let requested_model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(&target.upstream_model)
        .to_string();
    let mut param_audit = Vec::new();
    if let Some(map) = body.as_object_mut() {
        let caps = crate::gateway::capability::effective(
            target.backend.engine.as_deref(),
            &target.backend.capabilities,
        );
        // 审计注入随结构化日志入库（M5-C：PM200）。模型层双候选键：暴露名在前、
        // 上游真名在后（改名模型两边都认）。
        param_audit = st.params.read().expect("params lock poisoned").apply_for(
            &target.backend.name,
            &[client_model.as_str(), requested_model.as_str()],
            map,
            Some(&caps),
        );
    }
    if !param_audit.is_empty() {
        let injected: Vec<_> = param_audit
            .iter()
            .filter(|i| i.applied)
            .map(|i| serde_json::json!({"key": i.key, "layer": i.layer, "policy": i.policy}))
            .collect();
        let skipped: Vec<_> = param_audit
            .iter()
            .filter(|i| !i.applied)
            .map(|i| i.key.clone())
            .collect();
        st.log.log(crate::gateway::log::Event {
            level: crate::gateway::log::Level::Info,
            code: "PM200 params.injected",
            message: "parameter center applied overrides",
            request_id: Some(actx.request_id),
            backend: Some(&target.backend.name),
            model: Some(requested_model.as_str()),
            extra: serde_json::json!({ "injected": injected, "skipped": skipped }),
        });
    }

    // Tool-schema hints ride along for the gemini→anthropic response leg.
    let gemini_hints = if proto == Protocol::Gemini && entry == Protocol::Anthropic {
        let hints = extract_anthropic_tool_schema_hints(&body);
        (!hints.is_empty()).then_some(hints)
    } else {
        None
    };
    let ctx = TransformCtx {
        gemini_shadow: Some(st.gemini_shadow.as_ref()),
        provider_id: &target.backend.name,
        session_id: actx.session_id,
        gemini_hints: gemini_hints.as_ref(),
    };

    let mut outbound = if adapter.needs_transform(entry) {
        adapter.transform_request_ctx(body, entry, &ctx)?
    } else {
        body
    };
    // Gemini 的模型名写在 URL 里，入口阶段只当路由键用；到这里才知道上游是谁。
    // · 上游也是 Gemini：它的 proto JSON 不吃未知字段，必须摘掉我们塞的 model；
    // · 上游是别的协议（M11-B 入站翻译）：model 要换成真正的上游名，而流式意图
    //   藏在 URL 动作（streamGenerateContent）里，得替客户端写成 body.stream。
    if entry == Protocol::Gemini {
        if let Some(map) = outbound.as_object_mut() {
            if proto == Protocol::Gemini {
                map.remove("model");
            } else {
                map.insert(
                    "model".to_string(),
                    serde_json::Value::String(target.upstream_model.clone()),
                );
            }
        }
    }

    // openai_chat 上游统一显式索要 usage（与 claude/codex 转换路径同口径）：
    // 直通路径（entry=openai_chat）此前不注入，上游便永远不在流尾回 usage，
    // usage 落库与分模型分后端的纠偏样本（note_pre_coeff/note_dec_coeff）双双断粮。
    // 幂等，非流式不动。
    if proto == Protocol::OpenaiChat {
        super::backends::transform::inject_openai_stream_include_usage(&mut outbound);
    }

    // 图文能力整流（P0-b + VLM 开关）：已证实只吃文本的模型碰到图片块，就地降级成文字标记。
    // 判定口径：模型元数据的 VLM 声明（人工 > 探测）> 已确认纯文本网状表 > Unknown（放行），
    // 详见 model_capabilities。
    let declared_vlm = st
        .store
        .effective_model_meta_cached()
        .ok()
        .and_then(|m| m.get(requested_model.as_str()).and_then(|e| e.vlm));
    let stripped = crate::gateway::model_capabilities::rectify_image_blocks(
        &mut outbound,
        &target.upstream_model,
        declared_vlm,
    );
    if stripped > 0 {
        tracing::debug!(
            model = %target.upstream_model,
            stripped,
            "纯文本模型：图片块已降级为文字标记"
        );
    }

    let url = match (&actx.gemini_route, proto) {
        (Some(route), Protocol::Gemini) => format!(
            "{}/v1beta/models/{}:{}{}",
            target.backend.base_url.trim_end_matches('/'),
            target.upstream_model,
            route.action,
            route.query
        ),
        _ => adapter.build_url_for(&target.backend.base_url, &outbound, client_wants_stream),
    };
    let mut headers = adapter.auth_headers(target.backend.api_key.as_deref())?;
    headers.push((
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/json"),
    ));

    // 入口挂号（M12-C1）：登记时机必须在发出请求之前，否则浮条看不到 prefill 段
    // ——而那正是最需要看到的一段。请求块整包字节一并挂上（新口径 prefill 分子），
    // 估算输入只作没纠偏系数时的兜底先验。
    let req_body = serde_json::to_vec(&outbound).unwrap_or_default();
    let est_input = crate::gateway::usage::parser::estimate_tokens(&req_body).max(0) as u64;
    let slot = st.monitor.slot(
        &target.backend.name,
        // 挂号记**暴露名/别名**（改写前留的 client_model）：速率出口 /admin/rates/line
        // 的 model 字段与浮窗同源，代理侧扩展显示的就是用户配的别名，不是上游真名。
        &client_model,
        entry.as_str(),
        est_input,
        req_body.len() as u64,
        actx.app.map(str::to_string),
        actx.pid,
    );
    // 代理挂号即报（#46）：GUI 装了通道就把代理名丢过去，速率线按 rateline.auto
    // 自动弹出；headless 没装通道，锁一下即放、静默跳过，不挡转发热路径。
    if let Some(hit) = actx.app {
        if let Some(tx) = st
            .agent_hit_tx
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = tx.send(hit.to_string());
        }
    }
    let meta = ExchangeMeta {
        entry,
        backend_proto: proto,
        backend: target.backend.name.clone(),
        model: target.upstream_model.clone(),
        started,
        session_id: actx.session_id.to_string(),
        gemini_hints,
        trace: actx.trace.clone(),
        metrics: Some(st.metrics.clone()),
        cache_key: actx.cache_key.map(str::to_string),
        cache_ttl: actx.cache_ttl,
        cache_status: actx.cache_status.map(str::to_string),
        active: Some(slot.clone()),
    };
    // 就在交给上游的前一刻打标：prefill 从这一刻起算，网关这边的排队单列。
    st.monitor.mark_dispatched(slot.id);
    let resp = fwd.post_json(&url, &headers, &outbound).await?;

    if !resp.status().is_success() {
        // 上游 5xx 也是「本尝试失败」，计入重试轨迹。
        if let Some(tr) = &actx.trace {
            if let Ok(mut g) = tr.lock() {
                g.push(serde_json::json!({
                    "backend": target.backend.name,
                    "status": resp.status().as_u16(),
                }));
            }
        }
        return Err(finalize_error(resp, &meta, st, fwd.total_timeout()).await);
    }
    finalize_ok(
        resp,
        &meta,
        adapter,
        st,
        fwd.idle_timeout(),
        fwd.total_timeout(),
    )
    .await
}

async fn proxy_entry(
    st: Arc<State>,
    entry: Protocol,
    body: Value,
    headers: HeaderMap,
    gemini_route: Option<GeminiRoute>,
    origin: Option<super::origin::Origin>,
) -> Result<Response, ProxyError> {
    let app = origin.as_ref().map(|o| o.app.clone());
    let pid = origin.as_ref().map(|o| o.pid);
    // Gemini names the model in the URL; the caller already parsed it out.
    let model = match (&body, gemini_route.as_ref()) {
        (b, Some(_)) => b
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .ok_or_else(|| ProxyError::InvalidRequest("missing gemini model".into()))?,
        _ => body
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| !m.is_empty())
            .ok_or_else(|| ProxyError::InvalidRequest("missing `model`".into()))?,
    };
    let session_id = session_id_of(&headers, &body);
    // 一行一请求的运行日志轨迹：request_id 贯穿本次代理的所有日志行。
    let rid = uuid::Uuid::new_v4().to_string();
    let began = Instant::now();

    // 响应缓存判定（M5.5-A）：命中直接回放，不回源。
    let (cache_key, cache_ttl, cache_status) = match cache::decide(
        &st.config.cache.response,
        &st.cache_overlays(),
        entry,
        model,
        &body,
        &headers,
    ) {
        cache::Decision::Off => (None, 0i64, None),
        cache::Decision::Bypass(reason) => {
            st.cache.note_bypass();
            st.log.log(crate::gateway::log::Event {
                level: crate::gateway::log::Level::Info,
                code: "CA102 cache.bypass",
                message: "request excluded from response cache",
                request_id: Some(&rid),
                backend: None,
                model: Some(model),
                extra: json!({ "reason": reason }),
            });
            (None, 0, Some(reason))
        }
        cache::Decision::Lookup { key, policy } => match st.cache.get(&key) {
            Some(hit) => return replay_hit(&st, entry, &key, hit, &rid, began, app.clone()).await,
            None => {
                st.cache.note_miss();
                st.log.log(crate::gateway::log::Event {
                    level: crate::gateway::log::Level::Info,
                    code: "CA101 cache.miss",
                    message: "response cache miss, forwarding",
                    request_id: Some(&rid),
                    backend: None,
                    model: Some(model),
                    extra: json!({ "key": short_key(&key) }),
                });
                (Some(key), policy.ttl_secs, Some("miss"))
            }
        },
    };

    // 重试轨迹账本：失败的尝试逐一记录，随 usage 行落库（retry_trace）。
    let trace: crate::gateway::response_processor::SharedTrace =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let actx = AttemptCtx {
        session_id: &session_id,
        app: app.as_deref(),
        pid,
        gemini_route: gemini_route.as_ref(),
        trace: Some(trace.clone()),
        request_id: &rid,
        cache_key: cache_key.as_deref(),
        cache_ttl,
        cache_status,
    };

    let prefix_hash = affinity_prefix(&st, &body);
    let targets = candidates(&st, model, prefix_hash.as_deref()).await;
    if targets.is_empty() {
        // 分清两种「空」：没有任何后端声明过这个模型（客户端写错 → 404），
        // 与「声明了但此刻全下线/被熔断」（运维问题 → 503）。日志码一并分开，
        // 否则 Logs 页里两种病因混成一行，排查时白绕路。
        // 工种类互斥（11b）：媒体模型打进聊天入口按「聊天域没人提供它」判 404——
        // 后端确实服务这个模型，但它不是聊天模型，换后端也一样。
        let declared = st
            .store
            .list_backends()
            .map(|rows| {
                rows.iter().any(|b| {
                    b.resolve_upstream(model).is_some()
                        && model_kind::request_kind(&b.model_flags, model)
                            == model_kind::ModelKind::Chat
                })
            })
            .unwrap_or(false);
        let err = if declared {
            ProxyError::NoBackendAvailable
        } else {
            ProxyError::ModelNotFound(model.to_string())
        };
        st.log.log(crate::gateway::log::Event {
            level: crate::gateway::log::Level::Warn,
            code: if declared {
                "GW503 route.no_backend"
            } else {
                "GW404 route.unknown_model"
            },
            message: if declared {
                "no enabled backend can serve this model right now"
            } else {
                "no configured backend declares this model"
            },
            request_id: Some(&rid),
            backend: None,
            model: Some(model),
            extra: serde_json::json!({ "entry": format!("{entry:?}") }),
        });
        return Err(err);
    }

    // 幂等保护：重试只发生在「响应尚未交给客户端」阶段（forward_once 在收到
    // 上游响应头之前返回 Err）；一旦流开始向客户端转发就不可能再重试——
    // 流中途失败由 UsageRecorder 记错误行，绝不重放。
    let routing = st.routing.read().expect("routing lock poisoned").clone();
    let max_attempts = 1 + routing.retries;
    let mut last_err = ProxyError::NoBackendAvailable;
    let mut attempts = 0u32;
    let cb_on = routing.circuit_breaker.enabled;
    for target in targets {
        if attempts >= max_attempts {
            break;
        }
        // 熔断闸门：被拒的候选不消耗重试预算。
        let breaker = if cb_on {
            Some(crate::gateway::breaker_for(&st, target.backend.id))
        } else {
            None
        };
        // 名额一旦占用立刻交由 RAII 守护：本次转发无论正常返回、报错还是
        // future 被丢弃（客户端断连），名额都会归还——被丢弃时由 Drop 兜底，
        // 兜住 2026-10-11 半开名额泄漏致熔断器永久卡死的事故。
        let mut guard;
        if let Some(br) = &breaker {
            let prev = br.get_state().await;
            let allow = br.allow_request().await;
            if !allow.allowed {
                st.log.log(crate::gateway::log::Event {
                    level: crate::gateway::log::Level::Warn,
                    code: "CB110 cb.reject",
                    message: "circuit open, backend skipped",
                    request_id: Some(&rid),
                    backend: Some(&target.backend.name),
                    model: Some(model),
                    extra: serde_json::json!({ "state": prev.to_string() }),
                });
                continue;
            }
            guard = crate::gateway::circuit_breaker::HalfOpenPermit::from_allow(
                Some(br.clone()),
                allow.used_half_open_permit,
            );
        } else {
            guard = crate::gateway::circuit_breaker::HalfOpenPermit::none();
        }
        let prev_state = if let Some(br) = &breaker {
            Some(br.get_state().await)
        } else {
            None
        };
        attempts += 1;
        let started = Instant::now();
        match forward_once(
            &st,
            &st.forwarder,
            entry,
            &target,
            body.clone(),
            started,
            &actx,
        )
        .await
        {
            Ok(resp) => {
                // `forward_once` 把一切非 2xx 都转成 Err（见 forward_once 尾部的
                // `finalize_error`），失败记账由下面的 Err 分支按 `backend_healthy()`
                // 决定；所以走到这里**一定是 2xx**——这里只记成功，不要再写
                // `status >= 500` 的死分支（读代码的人会以为 5xx 在这记）。
                let status = resp.status().as_u16();
                if let Some(br) = &breaker {
                    // 名额由 guard 归还，这里只记健康度（used=false 防二次释放）。
                    br.record_success(false).await;
                    log_cb_transition(&st, &target.backend.name, model, &rid, prev_state, br).await;
                }
                // 记账完毕立即归还名额（Drop 亦幂等兜底）。
                guard.release();
                // 亲和记账：本前缀刚刚由该后端成功服务。
                if let Some(hash) = &prefix_hash {
                    let _ = st.store.touch_affinity(hash, target.backend.id);
                }
                st.log.log(crate::gateway::log::Event {
                    level: crate::gateway::log::Level::Info,
                    code: "GW200 exchange.finished",
                    message: "exchange finished",
                    request_id: Some(&rid),
                    backend: Some(&target.backend.name),
                    model: Some(model),
                    extra: crate::gateway::log::exchange_extra(
                        attempts,
                        status,
                        is_streaming(&body),
                        began.elapsed().as_millis(),
                    ),
                });
                return Ok(resp);
            }
            Err(e) => {
                let retriable = retryable(&e);
                // 「谁的错」决定要不要记账：客户端毛病与上游 4xx 都只释放半开名额，
                // 不记失败（见 ProxyError::backend_healthy）。
                let healthy = e.backend_healthy();
                if let Some(br) = &breaker {
                    // 名额由 guard 归还，这里只记健康度（used=false 防二次释放）。
                    if healthy {
                        br.record_success(false).await;
                    } else {
                        br.record_failure(false).await;
                    }
                    log_cb_transition(&st, &target.backend.name, model, &rid, prev_state, br).await;
                }
                // 记账完毕立即归还名额（Drop 亦幂等兜底）。
                guard.release();
                let err_msg = e.diagnostic();
                st.log.log(crate::gateway::log::Event {
                    level: if retriable {
                        crate::gateway::log::Level::Warn
                    } else if healthy {
                        crate::gateway::log::Level::Info
                    } else {
                        crate::gateway::log::Level::Error
                    },
                    code: if retriable {
                        "GW5xx route.retry"
                    } else if healthy {
                        "GW4xx forward.rejected"
                    } else {
                        "GW5xx forward.failed"
                    },
                    message: &err_msg,
                    request_id: Some(&rid),
                    backend: Some(&target.backend.name),
                    model: Some(model),
                    extra: serde_json::json!({ "attempt": attempts, "retried": retriable }),
                });
                if let Ok(mut g) = trace.lock() {
                    g.push(serde_json::json!({
                        "attempt": attempts,
                        "backend": target.backend.name,
                        "error": err_msg,
                    }));
                }
                last_err = e;
                if !retriable {
                    break;
                }
            }
        }
    }
    Err(last_err)
}

/// Log a breaker state jump into the ops log (Logs 页可见熔断跳变).
pub(crate) async fn log_cb_transition(
    st: &Arc<State>,
    backend: &str,
    model: &str,
    rid: &str,
    prev: Option<crate::gateway::circuit_breaker::CircuitState>,
    br: &crate::gateway::circuit_breaker::CircuitBreaker,
) -> () {
    if let Some(prev) = prev {
        let now = br.get_state().await;
        if now != prev {
            st.log.log(crate::gateway::log::Event {
                level: crate::gateway::log::Level::Warn,
                code: "CB120 cb.state_changed",
                message: "circuit state changed",
                request_id: Some(rid),
                backend: Some(backend),
                model: Some(model),
                extra: serde_json::json!({ "from": prev.to_string(), "to": now.to_string() }),
            });
        }
    }
}

/// Whether the client asked for a streamed response (any protocol spelling).
fn is_streaming(body: &Value) -> bool {
    body.get("stream").and_then(Value::as_bool) == Some(true)
        || body.get("stream").and_then(Value::as_str) == Some("true")
}

/// `POST /v1/messages` — Anthropic Messages entry.
async fn messages(
    AxumState(st): AxumState<Arc<State>>,
    origin: Option<axum::Extension<super::origin::Origin>>,
    headers: HeaderMap,
    LimitedJson(body): LimitedJson,
) -> Result<Response, ProxyError> {
    proxy_entry(
        st,
        Protocol::Anthropic,
        body,
        headers,
        None,
        origin.map(|o| o.0),
    )
    .await
}

/// `POST /v1/chat/completions` — OpenAI Chat entry.
async fn chat_completions(
    AxumState(st): AxumState<Arc<State>>,
    origin: Option<axum::Extension<super::origin::Origin>>,
    headers: HeaderMap,
    LimitedJson(body): LimitedJson,
) -> Result<Response, ProxyError> {
    proxy_entry(
        st,
        Protocol::OpenaiChat,
        body,
        headers,
        None,
        origin.map(|o| o.0),
    )
    .await
}

/// `POST /v1beta/models/<model>:<action>` — Gemini native entry (pass-through
/// to Gemini backends, translated for others). Pass-through query keeps
/// protocol knobs (`alt`, `fields`, …) but drops credential params: a client's
/// own `?key=` identifies *us*, it must never be forwarded upstream (the
/// upstream always gets the backend's registered key, see `backends/gemini.rs`).
async fn gemini_models(
    AxumState(st): AxumState<Arc<State>>,
    origin: Option<axum::Extension<super::origin::Origin>>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    axum::extract::Path(path): axum::extract::Path<String>,
    LimitedJson(mut value): LimitedJson,
) -> Result<Response, ProxyError> {
    // `path` looks like `<model>:generateContent` or `<model>:streamGenerateContent`.
    let (model, action) = path
        .split_once(':')
        .filter(|(_, a)| matches!(*a, "generateContent" | "streamGenerateContent"))
        .ok_or_else(|| {
            ProxyError::InvalidRequest(
                "expected <model>:generateContent[|streamGenerateContent]".into(),
            )
        })?;
    // Routing keys off `model`; the outbound URL is rebuilt per-target anyway.
    value["model"] = json!(model);
    let query = super::auth::sanitize_forwarded_query(uri.query());
    proxy_entry(
        st,
        Protocol::Gemini,
        value,
        headers,
        Some(GeminiRoute {
            action: action.to_string(),
            query,
        }),
        origin.map(|o| o.0),
    )
    .await
}

/// `POST /v1/responses` — OpenAI Responses entry (Codex CLI).
async fn responses(
    AxumState(st): AxumState<Arc<State>>,
    origin: Option<axum::Extension<super::origin::Origin>>,
    headers: HeaderMap,
    LimitedJson(body): LimitedJson,
) -> Result<Response, ProxyError> {
    proxy_entry(
        st,
        Protocol::OpenaiResponses,
        body,
        headers,
        None,
        origin.map(|o| o.0),
    )
    .await
}

/// `POST /v1/messages/count_tokens` — cheap local estimate (~4 chars/token).
/// Exact tokenizer integration is deliberately out of scope for M1.
async fn count_tokens(LimitedJson(body): LimitedJson) -> Result<Json<Value>, ProxyError> {
    let text = serde_json::to_string(&body).unwrap_or_default();
    let est = (text.chars().count() / 4).max(1);
    Ok(Json(json!({ "input_tokens": est })))
}

/// 连接归属中间件：把「谁在跟网关说话」挂进请求扩展，供挂号槽与 usage 落库取用。
///
/// 各家 CLI 都不发统一身份头，只能从 socket 反查（见 [`super::origin`]）。命中
/// 缓存时零成本；认不出就不挂，下游一律按「未知代理」处理——浮窗退回显示全部，
/// 而不是猜一家把速率串到别人头上。
pub(crate) async fn identify_client(
    AxumState(st): AxumState<Arc<State>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let peer = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|c| c.0);
    if let Some(peer) = peer {
        // 端口取实际监听值：`server.port` 能热切，写死配置值会一直查空表。
        if let Some(origin) = super::origin::resolve(st.listen_port(), peer).await {
            req.extensions_mut().insert(origin);
        }
    }
    next.run(req).await
}

/// 菜单栏「快速启停」的闸门（D22）：暂停接单时推理入口一律 503，
/// 控制面不受影响。放在中间件里是为了将来新增推理路由不会漏掉。
pub(crate) async fn reject_when_paused(
    AxumState(st): AxumState<Arc<State>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if st.is_paused() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "type": "error",
                "error": {
                    "type": "api_error",
                    "message": "llm-switch 正在暂停接单（菜单栏「恢复接单」可立即恢复）",
                },
            })),
        )
            .into_response();
    }
    next.run(req).await
}

/// Ollama 门面（P1-A）：专为 Copilot Chat 的 BYOK 开口。
/// 它只认 Ollama，我们就扮 Ollama；进去以后走的仍是普通 OpenAI-chat 通道。
async fn ollama_version() -> axum::Json<Value> {
    axum::Json(crate::gateway::backends::ollama_inbound::version_payload())
}

async fn ollama_tags(
    AxumState(st): AxumState<Arc<State>>,
) -> Result<axum::Json<Value>, ProxyError> {
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for b in st
        .store
        .list_backends()
        .map_err(|e| ProxyError::Internal(e.to_string()))?
    {
        if !b.enabled {
            continue;
        }
        for m in b.exposed_models() {
            // 工种类互斥（11b）：Ollama 门面是聊天门面，媒体模型不列。
            if model_kind::request_kind(&b.model_flags, &m) != model_kind::ModelKind::Chat {
                continue;
            }
            seen.insert(m);
        }
    }
    // copilot 虚拟分配池（UI 重设计）：settings 清单存了就只列交集，缺省全暴露。
    if let Some(allow) = st.store.copilot_allow() {
        let allow: std::collections::BTreeSet<String> = allow.into_iter().collect();
        seen = seen.intersection(&allow).cloned().collect();
    }
    Ok(axum::Json(
        crate::gateway::backends::ollama_inbound::tags_payload(
            &seen.into_iter().collect::<Vec<_>>(),
        ),
    ))
}

async fn ollama_show(Json(body): Json<Value>) -> axum::Json<Value> {
    let model = body
        .get("model")
        .or_else(|| body.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    axum::Json(crate::gateway::backends::ollama_inbound::show_payload(
        model,
    ))
}

async fn ollama_chat(
    AxumState(st): AxumState<Arc<State>>,
    origin: Option<axum::Extension<super::origin::Origin>>,
    headers: HeaderMap,
    LimitedJson(body): LimitedJson,
) -> Result<Response, ProxyError> {
    use crate::gateway::backends::ollama_inbound as oi;
    // Ollama 默认流式（不传 stream 也是 true），跟它的客户端习惯一致。
    let wants_stream = body.get("stream").and_then(Value::as_bool).unwrap_or(true);
    let mut chat = oi::ollama_to_openai_chat(&body)?;
    chat.as_object_mut()
        .expect("translated body is object")
        .insert("stream".to_string(), json!(wants_stream));
    let resp = proxy_entry(
        st,
        Protocol::OpenaiChat,
        chat,
        headers,
        None,
        origin.map(|o| o.0),
    )
    .await?;

    if !wants_stream {
        let bytes = axum::body::to_bytes(resp.into_body(), MAX_UPSTREAM_BODY_BYTES)
            .await
            .map_err(|e| ProxyError::Internal(format!("读取上游响应失败: {e}")))?;
        let parsed: Value = serde_json::from_slice(&bytes)
            .map_err(|e| ProxyError::Internal(format!("上游响应不是合法 JSON: {e}")))?;
        return Ok(axum::Json(oi::openai_chat_to_ollama(&parsed)?).into_response());
    }

    let ndjson = oi::create_ollama_ndjson_stream_from_chat(resp.into_body().into_data_stream());
    Response::builder()
        .status(200)
        .header("content-type", "application/x-ndjson")
        .body(axum::body::Body::from_stream(ndjson))
        .map_err(|e| ProxyError::Internal(format!("构造 NDJSON 响应失败: {e}")))
}

pub fn routes(state: Arc<State>) -> axum::Router<Arc<State>> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses))
        .route("/v1beta/models/*model-action", post(gemini_models))
        .route("/api/version", get(ollama_version))
        .route("/api/tags", get(ollama_tags))
        .route("/api/show", post(ollama_show))
        .route("/api/chat", post(ollama_chat))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            reject_when_paused,
        ))
        // 归属识别挂在最外层：暂停接单与否都要先认人，且将来新增推理路由不会漏。
        .layer(axum::middleware::from_fn_with_state(state, identify_client))
}
