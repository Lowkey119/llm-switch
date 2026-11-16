use super::{ApiError, State};
use crate::store::Store;
use crate::types::{now_ms, LogQuery};
use axum::extract::{Path, Query, State as AxumState};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

pub fn router(state: Arc<State>) -> axum::Router {
    use axum::routing::{any, delete, get, post, put};
    // 控制面也要能收大 body（例如配置文件编辑器），统一用同一个上限；
    // 推理入口另有 `LimitedJson` 给出可读的 413。
    let body_limit = state.config.server.max_body_bytes();
    axum::Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/v1/models", get(list_models))
        .route("/v1/models/:id", get(get_model))
        // M1/M2 inference entrypoints (Anthropic + OpenAI Chat + Responses + Gemini).
        .merge(super::handlers::routes(state.clone()))
        // Control plane consumed by the desktop UI.
        .route("/admin/backends", get(list_backends))
        .route("/admin/backends/:name/enabled", post(set_enabled))
        .route("/admin/backends/:name", delete(delete_backend))
        .route("/admin/backends/:name/console", put(update_backend_console))
        // 模型级开关（M12-A）。模型名一律走请求体：LM Studio 之类的上游会给出
        // `bartowski/xxx.gguf` 这种带斜杠的 id，塞进路径段会直接把路由切碎。
        .route(
            "/admin/backends/:name/model/exposed",
            put(model_set_exposed),
        )
        .route("/admin/backends/:name/model/accept", put(model_set_accept))
        .route("/admin/backends/:name/model/rename", put(model_rename))
        .route("/admin/backends/:name/model", delete(model_remove))
        .route("/admin/backends/probe-models", post(probe_models))
        .route("/admin/logs", get(query_logs))
        // M7 · 后端控制台反向代理：目标取自 backends 表登记的 webui_url/base_url，
        // 请求方只贡献 path/query——但这道「白名单」由登记方可写，所以它依赖
        // 访问令牌（见下方 auth 层）；凭据注入还额外受 console.rs 的同主机约束。
        // 入口统一用无尾斜杠 /console/:id：matchit 对 "/console/:id/" 空尾路径不匹配（实测 404），
        // 根页面注入 <base> 后，子资源自然落到 /console/:id/*path 被代理。
        .route("/console/:id", any(super::console::root))
        .route("/console/:id/*path", any(super::console::proxied))
        .route("/admin/params", get(list_params).put(put_param))
        .route("/admin/params/:id", delete(delete_param))
        .route("/admin/usage/summary", get(usage_summary))
        .route("/admin/usage/daily", get(usage_daily))
        .route("/admin/usage/records", get(usage_records))
        .route("/admin/usage/cost", get(usage_cost_route))
        .route("/admin/usage/stats", get(usage_stats_route))
        .route("/admin/metrics", get(metrics_snapshot))
        .route("/admin/metrics/models", get(metrics_by_model))
        .route("/admin/rates", get(rates_outlet))
        .route("/admin/rates/line", get(rates_line))
        .route("/admin/telemetry", get(telemetry_snapshot))
        .route("/admin/cache/stats", get(cache_stats))
        .route("/admin/cache/purge", post(cache_purge))
        .route(
            "/admin/cache/policies",
            get(list_cache_policies).put(put_cache_policy),
        )
        .route("/admin/cache/policies/:scope", delete(delete_cache_policy))
        // Skills（M6-A）：列表/扫描/开关/卸载/备份/全局分发方式。
        .route("/admin/skills", get(list_skills))
        .route("/admin/skills/scan", post(scan_skills))
        .route("/admin/skills/resync", post(resync_skills))
        .route(
            "/admin/skills/settings",
            get(get_skill_settings).put(put_skill_settings),
        )
        .route("/admin/skills/backups", get(list_skill_backups))
        .route("/admin/skills/backups/:id", delete(delete_skill_backup))
        .route(
            "/admin/skills/backups/:id/restore",
            post(restore_skill_backup),
        )
        // Skills 安装/导入/未纳管/仓库（M6-B）。
        .route("/admin/skills/install", post(install_skill))
        .route("/admin/skills/import-zip", post(import_skills_zip))
        .route("/admin/skills/unmanaged", get(list_unmanaged_skills))
        .route("/admin/skills/claim", post(claim_skill))
        .route(
            "/admin/skills/repos",
            get(list_skill_repos).post(add_skill_repo),
        )
        .route("/admin/skills/repos/:id", delete(delete_skill_repo))
        // Skills 发现与更新（M6-C）。
        .route("/admin/skills/discover", post(discover_skills))
        .route("/admin/skills/search", get(search_skills_endpoint))
        .route("/admin/skills/check-updates", post(check_skill_updates))
        .route("/admin/skills/update", post(update_skill_endpoint))
        .route("/admin/skills/:dir/doc", get(read_skill_doc))
        .route("/admin/skills/:dir/apps/:app", put(set_skill_app_enabled))
        .route("/admin/skills/:dir", delete(uninstall_skill))
        // 客户端接管（M9-A）：状态 / 接管 / 回滚 / 备份 / 设置。
        .route("/admin/takeover", get(takeover_status))
        .route("/admin/matrix", get(matrix_get))
        .route("/admin/matrix/cell", post(matrix_cell))
        // 预设配置一键还原：现状 + 还原（app 可选，缺省全量）。
        .route("/admin/preset", get(preset_status))
        .route("/admin/preset/restore", post(preset_restore))
        .route(
            "/admin/takeover/claude/switches",
            get(claude_switches_get).put(claude_switches_put),
        )
        .route(
            "/admin/takeover/settings",
            get(get_takeover_settings).put(put_takeover_settings),
        )
        .route("/admin/takeover/backups", get(list_takeover_backups))
        .route(
            "/admin/takeover/backups/:id",
            delete(delete_takeover_backup),
        )
        .route(
            "/admin/takeover/backups/:id/restore",
            post(restore_takeover_backup),
        )
        // 快速启停（M9-C）：暂停/恢复接单，与菜单栏同一开关。
        .route("/admin/gateway/pause", post(pause_gateway))
        .route("/admin/gateway/resume", post(resume_gateway))
        // 配置文件编辑器（M12-D）：路径一律由接管清单决定，接口不吃任意路径。
        .route("/admin/takeover/:app/files", get(takeover_files))
        .route(
            "/admin/takeover/:app/file",
            get(takeover_file_read).put(takeover_file_write),
        )
        .route(
            "/admin/takeover/:app/model",
            get(takeover_model_get).put(takeover_model_put),
        )
        .route("/admin/takeover/:app/apply", post(apply_takeover))
        .route("/admin/takeover/:app/revert", post(revert_takeover))
        .layer(axum::extract::DefaultBodyLimit::max(body_limit))
        // 访问令牌（P0）：最后加的一层 = 最外层，先于 body limit 与任何 handler
        // 拦下未授权请求。`server.access_token` 未设时这里全放行（开放回环代理），
        // 但那时监听地址必须是回环，见 Config::validate。
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            super::auth::require_token,
        ))
        .with_state(state)
}

async fn health(
    AxumState(st): AxumState<Arc<State>>,
    headers: HeaderMap,
) -> (HeaderMap, Json<Value>) {
    // 环回自检：若请求带探针头，原样写回响应头，自检据此判断请求是否回到了本地路由。
    let mut out = HeaderMap::new();
    if let Some(nonce) = headers.get(super::loopback_probe::PROBE_HEADER) {
        if let Ok(v) = HeaderValue::from_str(nonce.to_str().unwrap_or("")) {
            out.insert(super::loopback_probe::PROBE_HEADER, v);
        }
    }
    (
        out,
        Json(json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_ms": st.uptime_ms(),
        })),
    )
}

#[derive(Serialize)]
struct StatusBackend {
    name: String,
    protocol: String,
    engine: Option<String>,
    enabled: bool,
    health_state: String,
    models: Vec<String>,
    webui_url: Option<String>,
}

async fn status(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let backends: Vec<StatusBackend> = st
        .store
        .list_backends()?
        .into_iter()
        .map(|b| StatusBackend {
            name: b.name,
            protocol: b.protocol,
            engine: b.engine,
            enabled: b.enabled,
            health_state: b.health_state,
            models: b.models,
            webui_url: b.webui_url,
        })
        .collect();

    // Rolling window for the header pill: last 24h.
    let day_ago = now_ms() - 24 * 3600 * 1000;
    let usage = st.store.usage_summary(day_ago, now_ms())?;

    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_ms": st.uptime_ms(),
        // 实际监听端口（settings 覆盖 + 胶囊热切换都在 `listen_port` 里），
        // 不能用 config.server.port——端口被改过之后这两个值是不一样的。
        "listen": format!("{}:{}", st.config.server.host, st.listen_port()),
        "routing_policy": st.routing.read().expect("routing lock poisoned").policy,
        "paused": st.is_paused(),
        "cache": {
            "response_enabled": st.config.cache.response.enabled,
            "disk_gb": st.config.cache.response.disk_gb,
            "affinity": st.config.cache.affinity.enabled,
        },
        "backends": backends,
        "usage_24h": usage,
    })))
}

#[derive(Serialize)]
struct ModelCard {
    id: String,
    object: &'static str,
    owned_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_length: Option<u32>,
}

/// Union of every enabled backend's models, de-duplicated by public name.
async fn list_models(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let mut seen: std::collections::BTreeMap<String, String> = Default::default();
    for b in st.store.list_backends()? {
        if !b.enabled {
            continue;
        }
        // 只列暴露的模型名（M12-A 开关）：被关掉的对外等于不存在。
        for m in b.exposed_models() {
            seen.entry(m).or_insert_with(|| b.name.clone());
        }
    }
    // ctx 真值：人工声明 ⊕ 后端参数/探测（跨后端取最小）；未知的模型不带该字段。
    let meta = st.store.effective_model_meta().unwrap_or_default();
    let data: Vec<ModelCard> = seen
        .into_iter()
        .map(|(id, owner)| ModelCard {
            context_length: meta
                .get(&id)
                .and_then(|m| m.ctx)
                .and_then(|c| u32::try_from(c).ok()),
            id,
            object: "model",
            owned_by: owner,
        })
        .collect();
    Ok(Json(json!({ "object": "list", "data": data })))
}

async fn get_model(
    AxumState(st): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let owner = st
        .store
        .list_backends()?
        .iter()
        .find(|b| b.enabled && b.resolve_upstream(&id).is_some())
        .map(|b| b.name.clone())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                "model_not_found",
                format!("unknown model `{id}`"),
            )
        })?;
    Ok(Json(json!({
        "id": id, "object": "model", "owned_by": owner, "created": 0
    })))
}

// ------------------------------------------------------------ admin surface

async fn list_backends(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let rows = st.store.list_backends()?;
    // Never leak keys over the control plane.
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|mut b| {
            b.api_key = b.api_key.map(|_| "***".to_string());
            serde_json::to_value(b).unwrap_or(Value::Null)
        })
        .collect();
    Ok(Json(json!({ "data": rows })))
}

#[derive(Deserialize)]
struct EnabledBody {
    enabled: bool,
}

async fn set_enabled(
    AxumState(st): AxumState<Arc<State>>,
    Path(name): Path<String>,
    Json(body): Json<EnabledBody>,
) -> Result<Json<Value>, ApiError> {
    let changed = st.store.set_backend_enabled(&name, body.enabled)?;
    if !changed {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "backend_not_found",
            format!("no backend named `{name}`"),
        ));
    }
    tracing::info!(target: "llm_switch::routing", backend = %name, enabled = body.enabled, "backend toggled");
    // 持久化：回写 yaml（enabled 镜像为 None），让 DB 运行态在重启后仍胜出。
    if let Err(e) = crate::config::persist_backends_to_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::routes", backend = %name, "persist config.yaml failed: {e}");
    }
    Ok(Json(json!({ "name": name, "enabled": body.enabled })))
}

#[derive(Deserialize)]
struct ModelExposedBody {
    model: String,
    exposed: bool,
}

#[derive(Deserialize)]
struct ModelAcceptBody {
    model: String,
    accept: bool,
}

#[derive(Deserialize)]
struct ModelRenameBody {
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct ModelRemoveBody {
    model: String,
}

/// 模型级校验失败（后端/模型不存在、暴露名撞车）一律 400，别混进 500。
fn model_err(e: crate::store::StoreError) -> ApiError {
    match e {
        crate::store::StoreError::Invalid(msg) => {
            ApiError::new(StatusCode::BAD_REQUEST, "model_error", msg)
        }
        other => ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store",
            other.to_string(),
        ),
    }
}

pub(crate) fn model_log(
    st: &State,
    code: &'static str,
    message: String,
    backend: &str,
    extra: Value,
) {
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code,
        message: &message,
        request_id: None,
        backend: Some(backend),
        model: None,
        extra,
    });
}

/// 暴露开关：关掉 = 对客户端彻底隐身（`/v1/models` 不列、路由不选、硬打 404）。
async fn model_set_exposed(
    AxumState(st): AxumState<Arc<State>>,
    Path(name): Path<String>,
    Json(body): Json<ModelExposedBody>,
) -> Result<Json<Value>, ApiError> {
    let hit = st
        .store
        .set_model_exposed(&name, &body.model, body.exposed)
        .map_err(model_err)?;
    if !hit {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "model_not_found",
            format!("后端 `{name}` 上没有模型 `{}`", body.model),
        ));
    }
    model_log(
        &st,
        if body.exposed {
            "MD200 model.exposed"
        } else {
            "MD200 model.hidden"
        },
        format!("模型 `{}` 暴露状态改为 {}", body.model, body.exposed),
        &name,
        json!({ "model": body.model, "exposed": body.exposed }),
    );
    Ok(Json(
        json!({ "backend": name, "model": body.model, "exposed": body.exposed }),
    ))
}

/// 改名后的过渡开关：是否同时接受上游原名。
async fn model_set_accept(
    AxumState(st): AxumState<Arc<State>>,
    Path(name): Path<String>,
    Json(body): Json<ModelAcceptBody>,
) -> Result<Json<Value>, ApiError> {
    let hit = st
        .store
        .set_model_accept_upstream(&name, &body.model, body.accept)
        .map_err(model_err)?;
    if !hit {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "model_not_found",
            format!("后端 `{name}` 上没有模型 `{}`", body.model),
        ));
    }
    model_log(
        &st,
        "MD201 model.accept_upstream",
        format!("模型 `{}` 的上游原名过渡开关 = {}", body.model, body.accept),
        &name,
        json!({ "model": body.model, "accept": body.accept }),
    );
    Ok(Json(
        json!({ "backend": name, "model": body.model, "accept": body.accept }),
    ))
}

/// 换暴露名（缺省 = 上游 id）。参数覆盖与缓存策略的 ref_id 在同事务里跟着搬。
async fn model_rename(
    AxumState(st): AxumState<Arc<State>>,
    Path(name): Path<String>,
    Json(body): Json<ModelRenameBody>,
) -> Result<Json<Value>, ApiError> {
    let out = st
        .store
        .rename_model(&name, &body.from, &body.to)
        .map_err(model_err)?;
    // 持久化：改名（暴露名）要回写 config.yaml，否则重启后旧名复活。
    if let Err(e) = crate::config::persist_backends_to_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::routes", backend = %name, "persist config.yaml failed: {e}");
    }
    model_log(
        &st,
        "MD202 model.renamed",
        format!("模型 `{}` 的暴露名改为 `{}`", body.from, out.public_name),
        &name,
        json!({
            "from": body.from,
            "to": out.public_name,
            "upstream": out.upstream_name,
            "migrated_params": out.migrated_params,
            "migrated_cache_policies": out.migrated_cache_policies,
        }),
    );
    Ok(Json(serde_json::to_value(out).unwrap_or(Value::Null)))
}

/// 从该后端摘掉一个模型，连带清掉它的参数覆盖与缓存策略。
async fn model_remove(
    AxumState(st): AxumState<Arc<State>>,
    Path(name): Path<String>,
    Json(body): Json<ModelRemoveBody>,
) -> Result<Json<Value>, ApiError> {
    let out = st
        .store
        .remove_model(&name, &body.model)
        .map_err(model_err)?;
    // 持久化：摘模型要回写 config.yaml，否则重启后模型被 yaml 原样灌回。
    if let Err(e) = crate::config::persist_backends_to_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::routes", backend = %name, "persist config.yaml failed: {e}");
    }
    model_log(
        &st,
        "MD203 model.removed",
        format!("从后端摘掉模型 `{}`", body.model),
        &name,
        json!({
            "model": body.model,
            "dropped_params": out.dropped_params,
            "dropped_cache_policies": out.dropped_cache_policies,
        }),
    );
    Ok(Json(serde_json::to_value(out).unwrap_or(Value::Null)))
}

async fn delete_backend(
    AxumState(st): AxumState<Arc<State>>,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    // 与 IPC 同一道闸：halogen 后端不许删（REST 侧也要有，别留后门）。
    if let Some(row) = st.store.backend_by_name(&name)? {
        if crate::engine::is_protected_halogen(&row.name, row.engine.as_deref()) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "halogen_backend_protected",
                crate::engine::HALOGEN_PROTECTED,
            ));
        }
    }
    let Some(out) = st.store.delete_backend(&name)? else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "backend_not_found",
            format!("no backend named `{name}`"),
        ));
    };
    // 持久化：删后端要回写 config.yaml，否则下次启动 seed_backends 会复活它。
    if let Err(e) = crate::config::persist_backends_to_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::routes", backend = %name, "persist config.yaml failed: {e}");
    }
    Ok(Json(
        json!({ "deleted": name, "dropped_params": out.dropped_params,
                    "dropped_meta": out.dropped_meta }),
    ))
}

#[derive(Deserialize)]
struct LogQueryParams {
    level: Option<String>,
    backend: Option<String>,
    model: Option<String>,
    request_id: Option<String>,
    text: Option<String>,
    #[serde(default)]
    errors_only: bool,
    from: Option<i64>,
    to: Option<i64>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(serde::Deserialize)]
struct DailyParams {
    days: Option<i64>,
}

async fn usage_daily(
    AxumState(st): AxumState<Arc<State>>,
    Query(q): Query<DailyParams>,
) -> Result<Json<Value>, ApiError> {
    // 查询前先做一次增量 rollup，保证页面看到的聚合不滞后于维护周期。
    let _ = st.store.rollup_usage_daily();
    let rows = st
        .store
        .usage_daily_list(q.days.unwrap_or(30).clamp(1, 365))?;
    Ok(Json(serde_json::to_value(rows).unwrap_or(Value::Null)))
}

#[derive(serde::Deserialize)]
struct RecordsParams {
    backend: Option<String>,
    model: Option<String>,
    entry: Option<String>,
    request_id: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
    only_errors: Option<bool>,
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn usage_records(
    AxumState(st): AxumState<Arc<State>>,
    Query(q): Query<RecordsParams>,
) -> Result<Json<Value>, ApiError> {
    let rows = st
        .store
        .usage_records_query(&crate::types::UsageRecordQuery {
            backend: q.backend,
            model: q.model,
            entry_protocol: q.entry,
            request_id: q.request_id,
            from: q.from,
            to: q.to,
            only_errors: q.only_errors.unwrap_or(false),
            limit: q.limit.unwrap_or(100),
            offset: q.offset.unwrap_or(0),
        })?;
    Ok(Json(serde_json::to_value(rows).unwrap_or(Value::Null)))
}

async fn metrics_snapshot(AxumState(st): AxumState<Arc<State>>) -> Json<Value> {
    let snap = st.metrics.snapshot(st.monitor.snapshot().len());
    let mut v = serde_json::to_value(snap).unwrap_or(Value::Null);
    // 缓存命中指标并进速率快照（M5.5-E，§5.5.5）。
    if let Some(obj) = v.as_object_mut() {
        obj.insert("cache".into(), cache_stats_json(&st));
    }
    Json(v)
}

/// 浮条/浮窗的统一出口（M12-C1）：在飞明细 + 按模型速率 + 全局摘要。
async fn rates_outlet(AxumState(st): AxumState<Arc<State>>) -> Json<Value> {
    Json(super::rates::snapshot(&st))
}

#[derive(Deserialize)]
struct LineParams {
    /// 发起方代理（pi / claude / codex / …）；缺省 = 不过滤（与浮窗同一口径）。
    app: Option<String>,
    /// 调用方进程祖先链（逗号分隔，自己在前）；同代理多实例分流用（BUG-1）。
    /// 缺省 = 不按 pid 过滤，旧扩展行为不变。
    pids: Option<String>,
}

/// 单行速率线的出口：各家代理的速率扩展**只**从这里取数（口径见 `rates::line_json`）。
async fn rates_line(
    AxumState(st): AxumState<Arc<State>>,
    Query(q): Query<LineParams>,
) -> Json<Value> {
    let chain: Vec<u32> = q
        .pids
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .collect();
    Json(super::rates::line_json(&st, q.app.as_deref(), Some(&chain)).await)
}

/// 按模型的速率（M12-B）：监控页「按模型」面板与浮条共用这一个出口。
async fn metrics_by_model(AxumState(st): AxumState<Arc<State>>) -> Json<Value> {
    let rows = st.metrics.snapshot_models(&st.monitor.count_by_model());
    Json(json!({ "data": rows }))
}

async fn telemetry_snapshot(AxumState(st): AxumState<Arc<State>>) -> Json<Value> {
    Json(Value::Array(super::telemetry::snapshot_json(&st)))
}

async fn query_logs(
    AxumState(st): AxumState<Arc<State>>,
    Query(q): Query<LogQueryParams>,
) -> Result<Json<Value>, ApiError> {
    let rows = st.store.query_logs(&LogQuery {
        from: q.from,
        to: q.to,
        level: q.level,
        backend: q.backend,
        model: q.model,
        request_id: q.request_id,
        text: q.text,
        only_errors: q.errors_only,
        limit: q.limit.unwrap_or(200).min(2000),
        offset: q.offset.unwrap_or(0),
    })?;
    Ok(Json(json!({ "data": rows })))
}

#[derive(Deserialize)]
struct WindowParams {
    from: Option<i64>,
    to: Option<i64>,
}

async fn usage_summary(
    AxumState(st): AxumState<Arc<State>>,
    Query(w): Query<WindowParams>,
) -> Result<Json<Value>, ApiError> {
    let to = w.to.unwrap_or_else(now_ms);
    let from = w.from.unwrap_or(to - 24 * 3600 * 1000);
    let summary = st.store.usage_summary(from, to)?;
    let by_model = st.store.usage_by_model(from, to)?;
    Ok(Json(json!({
        "from": from,
        "to": to,
        "summary": summary,
        "by_model": by_model.iter().map(|(m, reqs, toks)| json!({
            "model": m, "requests": reqs, "tokens": toks
        })).collect::<Vec<_>>(),
    })))
}

/// Seed the database from the config file so a fresh install is usable at once.
pub fn seed_backends(store: &Store, cfg: &crate::config::Config) -> Result<usize, String> {
    // config.yaml 只是种子，DB 才是真值（见 commands/config_io.rs 头注）：
    // `enabled` 没写进 yaml（None）时不覆盖 DB 里的运行态——UI 启用开关只写 DB、
    // 不回写 yaml，若按默认 true 整行洗回去，用户关掉的后端每次启动都会复活
    // （实报：关掉 lmstudio，下次进来又是开的）。显式写了 true/false 才覆盖。
    // 控制台三元组（webui_url/user/password）同理：webui 地址是用户在控制台页
    // 手工输入、独立于 base_url 的持久化配置（实报：每次启动都被 yaml 播种洗掉，
    // 推导值/NULL 洗回去，内嵌 iframe 就打不开真实控制台）。yaml 没显式写（或空串）
    // 就保留 DB 现值；写了非空值才以 yaml 为准——播种没有清除语义，清空走控制台页。
    // **api_key 同病且更隐蔽**：DB→yaml 镜像（row_to_cfg）刻意把密钥写成 None
    // （秘密不进 yaml），于是镜像跑过一次后 yaml 永远无 key，下次启动播种就把
    // 用户填的 API Key 洗成 NULL（实报：Unsloth 密钥被 headless 实例播种清空，
    // 下发全 401）。同 enabled/webui 三元组口径：yaml 没写就保留 DB 现值。
    struct Prev {
        enabled: bool,
        api_key: Option<String>,
        webui_url: Option<String>,
        webui_user: Option<String>,
        webui_password: Option<String>,
    }
    let specified = |s: &Option<String>| s.as_deref().is_some_and(|v| !v.trim().is_empty());
    let existing: std::collections::HashMap<String, Prev> = store
        .list_backends()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|b| {
            (
                b.name.clone(),
                Prev {
                    enabled: b.enabled,
                    api_key: b.api_key,
                    webui_url: b.webui_url,
                    webui_user: b.webui_user,
                    webui_password: b.webui_password,
                },
            )
        })
        .collect();
    let mut n = 0;
    for b in &cfg.backends {
        let mut upsert = b.to_upsert()?;
        if let Some(p) = existing.get(&b.name) {
            if b.enabled.is_none() {
                upsert.enabled = p.enabled;
            }
            if !specified(&b.api_key) {
                upsert.api_key = p.api_key.clone();
            }
            if !specified(&b.webui_url) {
                upsert.webui_url = p.webui_url.clone();
            }
            if !specified(&b.webui_user) {
                upsert.webui_user = p.webui_user.clone();
            }
            if !specified(&b.webui_password) {
                upsert.webui_password = p.webui_password.clone();
            }
        }
        store.upsert_backend(&upsert).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// 启用开关持久化（用户实报：关掉 lmstudio，下次进来又是开的）：
    /// config.yaml 只是种子——没显式写 `enabled` 的后端，播种不许把 DB 运行态洗掉。
    #[test]
    fn seed_backends_preserves_runtime_enabled_unless_configured() {
        use crate::types::BackendUpsert;
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&BackendUpsert {
                name: "be".into(),
                base_url: "http://127.0.0.1:9".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: false,
                models: vec![],
                weight: 1,
                ..Default::default()
            })
            .unwrap();

        let mut cfg = Config::default();
        cfg.backends.push(crate::config::BackendCfg {
            name: "be".into(),
            base_url: "http://127.0.0.1:9".into(),
            protocol: "openai_chat".into(),
            ..Default::default()
        });

        // ① yaml 没写 enabled（None）→ 播种必须保留 DB 里的关闭态，不许复活。
        seed_backends(&store, &cfg).unwrap();
        assert!(
            !store.list_backends().unwrap()[0].enabled,
            "yaml 未指定 enabled 时，用户关掉的后端不能被启动播种洗活"
        );

        // ② yaml 显式 `enabled: false` → 以 yaml 为准（headless 管理口径）。
        store.set_backend_enabled("be", true).unwrap();
        cfg.backends[0].enabled = Some(false);
        seed_backends(&store, &cfg).unwrap();
        assert!(
            !store.list_backends().unwrap()[0].enabled,
            "显式写死的 enabled 应每次启动覆盖 DB"
        );
    }

    /// 控制台三元组持久化（用户实报：控制台页填的 webui 地址每次重启都被洗掉，
    /// 内嵌 iframe 打不开真实控制台）：config.yaml 只是种子——没显式写 webui_* 的
    /// 后端，播种不许覆盖 DB 里用户手填的地址/账号/密码；写了非空值才以 yaml 为准。
    #[test]
    fn seed_backends_preserves_webui_triple_unless_configured() {
        use crate::types::BackendUpsert;
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&BackendUpsert {
                name: "be".into(),
                base_url: "http://127.0.0.1:9".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                models: vec![],
                weight: 1,
                webui_url: Some("http://10.0.0.2:9999/".into()),
                webui_user: Some("u".into()),
                webui_password: Some("p".into()),
                ..Default::default()
            })
            .unwrap();

        let mut cfg = Config::default();
        cfg.backends.push(crate::config::BackendCfg {
            name: "be".into(),
            base_url: "http://127.0.0.1:9".into(),
            protocol: "openai_chat".into(),
            ..Default::default()
        });

        // ① yaml 没写 webui_* → 播种必须原样保留 DB 里的手工输入（不许洗成 NULL/推导值）。
        seed_backends(&store, &cfg).unwrap();
        let row = &store.list_backends().unwrap()[0];
        assert_eq!(row.webui_url.as_deref(), Some("http://10.0.0.2:9999/"));
        assert_eq!(row.webui_user.as_deref(), Some("u"));
        assert_eq!(row.webui_password.as_deref(), Some("p"));

        // ② yaml 显式写非空 URL → 以 yaml 为准（headless 管理口径）。
        cfg.backends[0].webui_url = Some("http://10.0.0.2:8888/".into());
        seed_backends(&store, &cfg).unwrap();
        let row = &store.list_backends().unwrap()[0];
        assert_eq!(row.webui_url.as_deref(), Some("http://10.0.0.2:8888/"));
        assert_eq!(
            row.webui_user.as_deref(),
            Some("u"),
            "yaml 未写的两项仍保留"
        );

        // ③ 空串视为没写 → 仍保留 DB（播种没有清除语义，清空走控制台页）。
        cfg.backends[0].webui_url = Some(String::new());
        seed_backends(&store, &cfg).unwrap();
        let row = &store.list_backends().unwrap()[0];
        assert_eq!(row.webui_url.as_deref(), Some("http://10.0.0.2:8888/"));
    }

    /// API Key 持久化（实报：Unsloth 密钥被 headless 实例播种清空、下发全 401）：
    /// DB→yaml 镜像刻意不把密钥写进 yaml（row_to_cfg 恒 None），若播种不保留
    /// DB 现值，镜像跑过一次后每次启动都会把用户填的 key 洗成 NULL。
    #[test]
    fn seed_backends_preserves_api_key_unless_configured() {
        use crate::types::BackendUpsert;
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&BackendUpsert {
                name: "be".into(),
                base_url: "http://127.0.0.1:9".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                models: vec![],
                weight: 1,
                api_key: Some("sk-real".into()),
                ..Default::default()
            })
            .unwrap();

        let mut cfg = Config::default();
        cfg.backends.push(crate::config::BackendCfg {
            name: "be".into(),
            base_url: "http://127.0.0.1:9".into(),
            protocol: "openai_chat".into(),
            ..Default::default()
        });

        // ① yaml 没写 api_key（镜像回写的常态）→ 播种必须保住 DB 里的密钥。
        seed_backends(&store, &cfg).unwrap();
        assert_eq!(
            store.list_backends().unwrap()[0].api_key.as_deref(),
            Some("sk-real"),
            "yaml 无 key 时播种不许把 DB 密钥洗成 NULL"
        );

        // ② yaml 显式写非空 key → 以 yaml 为准（headless 管理口径）。
        cfg.backends[0].api_key = Some("sk-yaml".into());
        seed_backends(&store, &cfg).unwrap();
        assert_eq!(
            store.list_backends().unwrap()[0].api_key.as_deref(),
            Some("sk-yaml")
        );
    }

    #[tokio::test]
    async fn models_endpoint_unions_enabled_backends() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::util::ServiceExt;

        let store = Arc::new(Store::in_memory().unwrap());
        let cfg = Config::default();
        let st = State::new(store.clone(), Arc::new(cfg));

        let mut on = crate::types::BackendUpsert {
            name: "on".into(),
            base_url: "http://x".into(),
            protocol: crate::types::Protocol::OpenaiChat,
            enabled: true,
            models: vec!["shared".into(), "only-on".into()],
            weight: 1,
            ..Default::default()
        };
        on.capabilities = json!({});
        store.upsert_backend(&on).unwrap();
        let off = crate::types::BackendUpsert {
            name: "off".into(),
            base_url: "http://y".into(),
            protocol: crate::types::Protocol::Anthropic,
            enabled: false,
            models: vec!["shared".into(), "hidden".into()],
            weight: 1,
            ..Default::default()
        };
        store.upsert_backend(&off).unwrap();

        let app = router(st);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/models")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        let ids: Vec<&str> = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["only-on", "shared"]);
    }
}

// ------------------------------------------------------------- param center

#[derive(Deserialize)]
struct ParamPut {
    scope: String,
    #[serde(default)]
    ref_id: String,
    key: String,
    value: Value,
    #[serde(default = "default_policy")]
    policy: String,
}

fn default_policy() -> String {
    "fill".into()
}

async fn list_params(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let rows = st
        .store
        .list_param_overrides()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({ "overrides": rows })))
}

async fn put_param(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<ParamPut>,
) -> Result<Json<Value>, ApiError> {
    if !matches!(req.scope.as_str(), "global" | "backend" | "model") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "scope must be global|backend|model",
        ));
    }
    if !matches!(req.policy.as_str(), "fill" | "override" | "clamp") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_policy",
            "policy must be fill|override|clamp",
        ));
    }
    // 值形状校验（与 IPC 保存口/注入口同口径）：空值拒收、数值键纠偏。
    let value = crate::gateway::params::normalize_value(&req.key, &req.policy, &req.value)
        .map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "invalid_value", m))?;
    let row = crate::types::ParamOverride {
        id: 0,
        scope: req.scope,
        ref_id: req.ref_id,
        key: req.key,
        value,
        policy: req.policy,
    };
    st.store
        .upsert_param_override(&row)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    super::on_params_changed(&st);
    // 与 IPC 保存口同口径：模型层顺手下发到后端本体，战报随响应带回（非模型层为 null）。
    let push = crate::services::param_push::push_override(
        &st,
        &row.scope,
        &row.ref_id,
        &row.key,
        Some(row.value.clone()),
    )
    .await;
    Ok(Json(json!({ "ok": true, "push": push })))
}

async fn delete_param(
    AxumState(st): AxumState<Arc<State>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    // 先取行再删：模型层删除要把后端里的值清回默认，删了才知道该清哪家的哪个键。
    let gone = st
        .store
        .list_param_overrides()
        .map_err(|e| ApiError::internal(e.to_string()))?
        .into_iter()
        .find(|r| r.id == id);
    st.store
        .delete_param_override(id)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    super::on_params_changed(&st);
    let push = match &gone {
        Some(r) => {
            crate::services::param_push::push_override(&st, &r.scope, &r.ref_id, &r.key, None).await
        }
        None => None,
    };
    Ok(Json(json!({ "ok": true, "push": push })))
}

// ------------------------------------------------------------------- cache
// M5.5-C：响应缓存的可观测与控制入口（Cache 页消费）。

use std::sync::atomic::Ordering as AtomicOrdering;

async fn cache_stats(AxumState(st): AxumState<Arc<State>>) -> Json<Value> {
    Json(cache_stats_json(&st))
}

/// 缓存统计 JSON（/admin/cache/stats 与 /admin/metrics 融合层共用）。
fn cache_stats_json(st: &Arc<State>) -> Value {
    let c = &st.cache;
    let s = &c.stats;
    let hits = s.hits.load(AtomicOrdering::Relaxed);
    let misses = s.misses.load(AtomicOrdering::Relaxed);
    json!({
        "hits": hits,
        "misses": misses,
        "hit_rate": if hits + misses > 0 { hits as f64 / (hits + misses) as f64 } else { 0.0 },
        "bypass": s.bypass.load(AtomicOrdering::Relaxed),
        "saved_input_tokens": s.saved_input_tokens.load(AtomicOrdering::Relaxed),
        "saved_output_tokens": s.saved_output_tokens.load(AtomicOrdering::Relaxed),
        "entries": c.len(),
        "memory_bytes": c.bytes(),
        "memory_budget_bytes": c.budget_bytes(),
        "disk_bytes": c.disk_bytes(),
        "disk_budget_bytes": c.disk_budget_bytes(),
        "enabled_globally": st.config.cache.response.enabled,
    })
}

#[derive(Deserialize)]
struct PurgeReq {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    all: bool,
}

async fn cache_purge(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<PurgeReq>,
) -> Result<Json<Value>, ApiError> {
    let cleared = if req.all || (req.model.is_none() && req.backend.is_none()) {
        st.cache.clear()
    } else {
        let m = req.model.as_deref();
        let b = req.backend.as_deref();
        st.cache.purge_where(move |model, producer| {
            m.is_some_and(|mm| model == mm) || b.is_some_and(|bb| producer == bb)
        })
    };
    st.log.log(super::log::Event {
        level: super::log::Level::Info,
        code: "CA104 cache.purged",
        message: "response cache purged",
        request_id: None,
        backend: req.backend.as_deref(),
        model: req.model.as_deref(),
        extra: json!({ "cleared": cleared }),
    });
    Ok(Json(json!({ "cleared": cleared })))
}

async fn list_cache_policies(
    AxumState(st): AxumState<Arc<State>>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let rows = st.store.list_cache_policies()?;
    Ok(Json(
        rows.into_iter()
            .map(|(scope, policy)| json!({ "scope": scope, "policy": policy }))
            .collect(),
    ))
}

#[derive(Deserialize)]
struct PolicyReq {
    /// "global" | "model:<名>"
    scope: String,
    policy: Value,
}

async fn put_cache_policy(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<PolicyReq>,
) -> Result<Json<Value>, ApiError> {
    if req.scope != "global" && !req.scope.starts_with("model:") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "scope must be `global` or `model:<name>`",
        ));
    }
    if !req.policy.is_object() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_policy",
            "policy must be a JSON object",
        ));
    }
    st.store.upsert_cache_policy(&req.scope, &req.policy)?;
    super::reload_cache_policies(&st);
    Ok(Json(json!({ "ok": true })))
}

async fn delete_cache_policy(
    AxumState(st): AxumState<Arc<State>>,
    Path(scope): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let removed = st.store.delete_cache_policy(&scope)?;
    super::reload_cache_policies(&st);
    Ok(Json(json!({ "removed": removed })))
}

// ------------------------------------------------------------------- skills (M6-A)

/// 技能服务错误 → 400（均为面向用户的操作拒绝：非法名/归属存疑/已存在等）。
fn skill_err(err: String) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "skill_error", err)
}

fn skill_env(st: &State) -> crate::services::skill::SkillEnv {
    crate::services::skill::SkillEnv::from_config(&st.config)
}

async fn list_skills(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let skills = crate::services::skill::list(&env, &st.store).map_err(skill_err)?;
    Ok(Json(json!({ "skills": skills })))
}

async fn scan_skills(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let report = crate::services::skill::scan(&env, &st.store).map_err(skill_err)?;
    st.log.log(super::log::Event {
        level: super::log::Level::Info,
        code: "SK001 skills.scan",
        message: "skills SSOT scan completed",
        request_id: None,
        backend: None,
        model: None,
        extra: report.clone(),
    });
    Ok(Json(report))
}

async fn resync_skills(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let outcomes = crate::services::skill::resync(&env, &st.store).map_err(skill_err)?;
    Ok(Json(json!({ "outcomes": outcomes })))
}

#[derive(Deserialize)]
struct SkillAppReq {
    enabled: bool,
}

async fn set_skill_app_enabled(
    AxumState(st): AxumState<Arc<State>>,
    Path((dir, app)): Path<(String, String)>,
    Json(req): Json<SkillAppReq>,
) -> Result<Json<Value>, ApiError> {
    let app = crate::services::skill::SkillApp::parse(&app).map_err(skill_err)?;
    let env = skill_env(&st);
    let state = crate::services::skill::set_enabled(&env, &st.store, &dir, app, req.enabled)
        .map_err(skill_err)?;
    Ok(Json(json!({ "app_state": state })))
}

async fn uninstall_skill(
    AxumState(st): AxumState<Arc<State>>,
    Path(dir): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let backup = crate::services::skill::uninstall(&env, &st.store, &dir).map_err(skill_err)?;
    st.log.log(super::log::Event {
        level: super::log::Level::Info,
        code: "SK003 skills.uninstalled",
        message: "skill uninstalled (backup retained)",
        request_id: None,
        backend: None,
        model: None,
        extra: json!({ "dir": dir, "backup": backup }),
    });
    Ok(Json(json!({ "backup": backup })))
}

async fn list_skill_backups(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let backups = crate::services::skill::list_backups(&env).map_err(skill_err)?;
    Ok(Json(json!({ "backups": backups })))
}

async fn delete_skill_backup(
    AxumState(st): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    crate::services::skill::delete_backup(&env, &id).map_err(skill_err)?;
    Ok(Json(json!({ "ok": true })))
}

async fn restore_skill_backup(
    AxumState(st): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let view = crate::services::skill::restore_backup(&env, &st.store, &id).map_err(skill_err)?;
    st.log.log(super::log::Event {
        level: super::log::Level::Info,
        code: "SK004 skills.restored",
        message: "skill restored from backup",
        request_id: None,
        backend: None,
        model: None,
        extra: json!({ "dir": view.dir, "backup": id }),
    });
    Ok(Json(json!({ "skill": view })))
}

async fn get_skill_settings(AxumState(st): AxumState<Arc<State>>) -> Json<Value> {
    Json(json!({
        "sync_method": crate::services::skill::get_sync_method(&st.store),
        "download_proxy": crate::services::skill::get_download_proxy(&st.store),
    }))
}

#[derive(Deserialize)]
struct SkillSettingsReq {
    sync_method: Option<String>,
    download_proxy: Option<String>,
}

async fn put_skill_settings(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<SkillSettingsReq>,
) -> Result<Json<Value>, ApiError> {
    if let Some(m) = req.sync_method.as_deref() {
        crate::services::skill::set_sync_method(&st.store, m).map_err(skill_err)?;
    }
    if let Some(p) = req.download_proxy.as_deref() {
        crate::services::skill::set_download_proxy(&st.store, p).map_err(skill_err)?;
    }
    Ok(Json(json!({
        "sync_method": crate::services::skill::get_sync_method(&st.store),
        "download_proxy": crate::services::skill::get_download_proxy(&st.store),
    })))
}

#[derive(Deserialize)]
struct InstallReq {
    owner: String,
    name: String,
    #[serde(default)]
    branch: Option<String>,
    /// 仓库内技能子目录（多技能仓库必填）。
    #[serde(default)]
    path: Option<String>,
}

async fn install_skill(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<InstallReq>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let view = crate::services::skill::install_from_repo(
        &env,
        &st.store,
        &req.owner,
        &req.name,
        req.branch.as_deref().unwrap_or("main"),
        req.path.as_deref(),
    )
    .await
    .map_err(skill_err)?;
    st.log.log(super::log::Event {
        level: super::log::Level::Info,
        code: "SK002 skills.installed",
        message: "skill installed from repo",
        request_id: None,
        backend: None,
        model: None,
        extra: json!({ "dir": view.dir, "repo": format!("{}/{}", req.owner, req.name) }),
    });
    Ok(Json(json!({ "skill": view })))
}

#[derive(Deserialize)]
struct ImportZipReq {
    path: String,
}

async fn import_skills_zip(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<ImportZipReq>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let report =
        crate::services::skill::import_zip(&env, &st.store, std::path::Path::new(&req.path))
            .map_err(skill_err)?;
    Ok(Json(report))
}

async fn list_unmanaged_skills(
    AxumState(st): AxumState<Arc<State>>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let items = crate::services::skill::scan_unmanaged(&env, &st.store).map_err(skill_err)?;
    Ok(Json(json!({ "unmanaged": items })))
}

#[derive(Deserialize)]
struct ClaimReq {
    dir: String,
    #[serde(default)]
    apps: Vec<String>,
}

async fn claim_skill(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<ClaimReq>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let view =
        crate::services::skill::claim(&env, &st.store, &req.dir, &req.apps).map_err(skill_err)?;
    Ok(Json(json!({ "skill": view })))
}

async fn list_skill_repos(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let repos = crate::services::skill::list_repos(&st.store).map_err(skill_err)?;
    Ok(Json(json!({ "repos": repos })))
}

#[derive(Deserialize)]
struct RepoReq {
    owner: String,
    name: String,
    #[serde(default)]
    branch: Option<String>,
}

async fn add_skill_repo(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<RepoReq>,
) -> Result<Json<Value>, ApiError> {
    crate::services::skill::add_repo(
        &st.store,
        &req.owner,
        &req.name,
        req.branch.as_deref().unwrap_or("main"),
    )
    .map_err(skill_err)?;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_skill_repo(
    AxumState(st): AxumState<Arc<State>>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, ApiError> {
    let removed = crate::services::skill::delete_repo(&st.store, id).map_err(skill_err)?;
    Ok(Json(json!({ "removed": removed })))
}

#[derive(Deserialize)]
struct DiscoverReq {
    #[serde(default)]
    force: bool,
}

async fn discover_skills(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<DiscoverReq>,
) -> Result<Json<Value>, ApiError> {
    let out = crate::services::skill::discover(&st.store, req.force)
        .await
        .map_err(skill_err)?;
    Ok(Json(out))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn search_skills_endpoint(
    Query(params): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let out = crate::services::skill::search_skills_sh(
        &params.q,
        params.limit.unwrap_or(20),
        params.offset.unwrap_or(0),
    )
    .await
    .map_err(skill_err)?;
    Ok(Json(out))
}

async fn check_skill_updates(
    AxumState(st): AxumState<Arc<State>>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let out = crate::services::skill::check_updates(&env, &st.store)
        .await
        .map_err(skill_err)?;
    Ok(Json(out))
}

#[derive(Deserialize)]
struct UpdateReq {
    /// 省略时对全部「可更新」技能逐个更新。
    #[serde(default)]
    dir: Option<String>,
}

async fn update_skill_endpoint(
    AxumState(st): AxumState<Arc<State>>,
    Json(req): Json<UpdateReq>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let out = match req.dir.as_deref() {
        Some(dir) => {
            let view = crate::services::skill::update_skill(&env, &st.store, dir)
                .await
                .map_err(skill_err)?;
            json!({ "skill": view })
        }
        None => crate::services::skill::update_all(&env, &st.store)
            .await
            .map_err(skill_err)?,
    };
    Ok(Json(out))
}

async fn read_skill_doc(
    AxumState(st): AxumState<Arc<State>>,
    Path(dir): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let env = skill_env(&st);
    let content = crate::services::skill::read_doc(&env, &dir).map_err(skill_err)?;
    Ok(Json(json!({ "content": content })))
}

#[derive(Deserialize)]
struct ConsoleBody {
    #[serde(default)]
    webui_url: Option<String>,
    #[serde(default)]
    webui_user: Option<String>,
    #[serde(default)]
    webui_password: Option<String>,
    /// 启停脚本路径（与地址/凭据同属控制台「入口设置」卡）。
    #[serde(default)]
    start_script: Option<String>,
    #[serde(default)]
    stop_script: Option<String>,
}

async fn update_backend_console(
    AxumState(st): AxumState<Arc<State>>,
    Path(name): Path<String>,
    Json(body): Json<ConsoleBody>,
) -> Result<Json<Value>, ApiError> {
    let ok = st.store.set_backend_console(
        &name,
        body.webui_url.as_deref(),
        body.webui_user.as_deref(),
        body.webui_password.as_deref(),
        body.start_script.as_deref(),
        body.stop_script.as_deref(),
    )?;
    if !ok {
        return Err(ApiError::new(
            axum::http::StatusCode::NOT_FOUND,
            "not_found",
            "后端不存在",
        ));
    }
    // 持久化：控制台入口（webui/凭据/脚本）改动要回写 yaml，否则清空后被旧值复活。
    if let Err(e) = crate::config::persist_backends_to_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::routes", backend = %name, "persist config.yaml failed: {e}");
    }
    Ok(Json(json!({ "ok": true })))
}

async fn usage_cost_route(
    AxumState(st): AxumState<Arc<State>>,
    Query(w): Query<WindowParams>,
) -> Result<Json<Value>, ApiError> {
    let to = w.to.unwrap_or_else(now_ms);
    let from = w.from.unwrap_or(to - 24 * 3600 * 1000);
    crate::services::pricing::usage_cost(&st.store, from, to)
        .map(Json)
        .map_err(Into::into)
}

/// M8-B：一页聚合（百分位/缓存/客户端/三维日序列），Usage 页一次拉齐。
async fn usage_stats_route(
    AxumState(st): AxumState<Arc<State>>,
    Query(w): Query<WindowParams>,
) -> Result<Json<Value>, ApiError> {
    let to = w.to.unwrap_or_else(now_ms);
    let from = w.from.unwrap_or(to - 7 * 24 * 3600 * 1000);
    let (p50, p95) = st.store.usage_ttft_percentiles(from, to)?;
    let cache = st.store.usage_cache_counts(from, to)?;
    let clients = st.store.usage_by_client(from, to)?;
    let series = |dim: &str| -> Result<Value, ApiError> {
        let rows = st.store.usage_series(from, to, dim)?;
        Ok(Value::Array(
            rows.into_iter()
                .map(|(day, key, reqs, toks)| json!({"day": day, "key": key, "requests": reqs, "tokens": toks}))
                .collect(),
        ))
    };
    let hit = cache.get("hit").copied().unwrap_or(0);
    let miss = cache.get("miss").copied().unwrap_or(0);
    let denom = hit + miss;
    Ok(Json(json!({
        "from": from,
        "to": to,
        "ttft_p50_ms": p50,
        "ttft_p95_ms": p95,
        "cache": {
            "hit": hit, "miss": miss,
            "bypass": cache.get("bypass").copied().unwrap_or(0),
            "hit_rate": if denom > 0 { hit as f64 / denom as f64 } else { 0.0 },
        },
        "by_client": clients.iter().map(|c| json!({
            "client": c.client, "requests": c.requests,
            "input_tokens": c.input, "output_tokens": c.output,
        })).collect::<Vec<_>>(),
        "series": {
            "backend": series("backend")?,
            "model": series("model")?,
            "client": series("client")?,
        },
    })))
}

// ============================ 客户端接管（M9-A） ============================

async fn probe_models(
    Json(input): Json<crate::services::discover::DiscoverInput>,
) -> Result<Json<Value>, ApiError> {
    let found = crate::services::discover::discover(&input)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, "discover_error", e))?;
    serde_json::to_value(found).map(Json).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize",
            e.to_string(),
        )
    })
}

fn takeover_err(err: String) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "takeover_error", err)
}

fn takeover_ctx(st: &State) -> (crate::takeover::TakeoverEnv, crate::takeover::Endpoint) {
    (
        crate::takeover::TakeoverEnv::from_config(&st.config, &st.store),
        crate::takeover::Endpoint::build(&st.config, &st.store),
    )
}

fn takeover_log(st: &State, code: &'static str, message: &'static str, extra: Value) {
    st.log.log(super::log::Event {
        level: super::log::Level::Info,
        code,
        message,
        request_id: None,
        backend: None,
        model: None,
        extra,
    });
}

async fn takeover_status(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let (env, ep) = takeover_ctx(&st);
    let rows = crate::takeover::status(&env, &st.store, &ep).map_err(takeover_err)?;
    Ok(Json(json!({
        "endpoint": { "base_root": ep.base_root, "api_root": ep.api_root, "models": ep.models },
        "clients": rows,
    })))
}

#[derive(Debug, serde::Deserialize)]
struct FileReadQuery {
    index: usize,
    #[serde(default)]
    reveal: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
struct FileWriteBody {
    index: usize,
    content: String,
    base_sha: String,
}

/// 编辑器的手势错误：并发冲突与格式错误要能被前端分辨（一个是 409，一个是 422）。
fn editor_err(err: String) -> ApiError {
    if err.starts_with("文件在你打开之后被改过") {
        ApiError::new(StatusCode::CONFLICT, "takeover_conflict", err)
    } else if err.contains("解析失败") {
        ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_format", err)
    } else {
        takeover_err(err)
    }
}

async fn takeover_files(
    AxumState(st): AxumState<Arc<State>>,
    Path(app): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let app = crate::takeover::ClientApp::parse(&app).map_err(takeover_err)?;
    let files = crate::takeover::editor::list(&st, app).map_err(editor_err)?;
    Ok(Json(json!({ "files": files })))
}

async fn takeover_file_read(
    AxumState(st): AxumState<Arc<State>>,
    Path(app): Path<String>,
    Query(q): Query<FileReadQuery>,
) -> Result<Json<Value>, ApiError> {
    let app = crate::takeover::ClientApp::parse(&app).map_err(takeover_err)?;
    let doc = crate::takeover::editor::read(&st, app, q.index, q.reveal.unwrap_or(false))
        .map_err(editor_err)?;
    serde_json::to_value(doc).map(Json).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize",
            e.to_string(),
        )
    })
}

async fn takeover_file_write(
    AxumState(st): AxumState<Arc<State>>,
    Path(app): Path<String>,
    Json(body): Json<FileWriteBody>,
) -> Result<Json<Value>, ApiError> {
    let app = crate::takeover::ClientApp::parse(&app).map_err(takeover_err)?;
    let out = crate::takeover::editor::write(&st, app, body.index, &body.content, &body.base_sha)
        .map_err(editor_err)?;
    Ok(Json(out))
}

async fn apply_takeover(
    AxumState(st): AxumState<Arc<State>>,
    Path(app): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let app = crate::takeover::ClientApp::parse(&app).map_err(takeover_err)?;
    let (env, ep) = takeover_ctx(&st);
    let report = crate::takeover::apply(&env, &st.store, &ep, app).map_err(takeover_err)?;
    if report["changed"] == json!(true) {
        takeover_log(
            &st,
            "TK001 takeover.applied",
            "client configuration taken over by local gateway",
            report.clone(),
        );
    }
    Ok(Json(report))
}

/// claude 语义开关（P0-a）：读现状 / 改一个。真值在 settings.json，写前留快照。
async fn claude_switches_get(
    AxumState(st): AxumState<Arc<State>>,
) -> Result<Json<Value>, ApiError> {
    let (env, _) = takeover_ctx(&st);
    crate::takeover::claude_switches::read_switches(&env)
        .map(Json)
        .map_err(takeover_err)
}

async fn claude_switches_put(
    AxumState(st): AxumState<Arc<State>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let key = body
        .get("key")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "takeover_error", "缺少 key"))?;
    let value = body.get("value").cloned().unwrap_or(Value::Null);
    let (env, ep) = takeover_ctx(&st);
    crate::takeover::claude_switches::set_switch(&env, &st.store, &ep, key, &value)
        .map(Json)
        .map_err(takeover_err)
}

/// 模型矩阵（M12-G）：行 = 暴露清单，列 = 客户端，格子 = 该端用不用它。
async fn matrix_get(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let (env, ep) = takeover_ctx(&st);
    crate::takeover::matrix::matrix(&env, &st.store, &ep)
        .map(Json)
        .map_err(takeover_err)
}

/// 改一个格子（加入 / 移出 / 设为默认），立刻落盘。
async fn matrix_cell(
    AxumState(st): AxumState<Arc<State>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let app =
        crate::takeover::ClientApp::parse(body.get("app").and_then(Value::as_str).unwrap_or(""))
            .map_err(takeover_err)?;
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid_request", "缺 model"))?;
    let op = crate::takeover::matrix::CellOp::parse(
        body.get("op").and_then(Value::as_str).unwrap_or(""),
    )
    .map_err(takeover_err)?;
    let slot = body
        .get("slot")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);
    let op = match op {
        crate::takeover::matrix::CellOp::Add { .. } => {
            crate::takeover::matrix::CellOp::Add { slot }
        }
        other => other,
    };
    let (env, ep) = takeover_ctx(&st);
    let report = crate::takeover::matrix::apply_cell(&env, &st.store, &ep, app, model, op)
        .map_err(takeover_err)?;
    takeover_log(
        &st,
        "TK007 takeover.matrix_cell",
        "model assignment changed through the matrix",
        report.clone(),
    );
    Ok(Json(report))
}

/// 预设配置现状（一键还原面板）。
async fn preset_status(AxumState(st): AxumState<Arc<State>>) -> Result<Json<Value>, ApiError> {
    let (env, _) = takeover_ctx(&st);
    crate::takeover::preset::status(&env)
        .map(Json)
        .map_err(takeover_err)
}

/// 一键还原预设配置：body 里带 `app` 只还原该代理，缺省全量还原。
async fn preset_restore(
    AxumState(st): AxumState<Arc<State>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let app = body
        .get("app")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);
    let (env, _) = takeover_ctx(&st);
    let report = crate::takeover::preset::restore(&env, app.as_deref()).map_err(takeover_err)?;
    takeover_log(
        &st,
        "TK008 takeover.preset_restore",
        "agent configs restored from preset directory",
        report.clone(),
    );
    Ok(Json(report))
}

/// 某端的模型现状：文件里的真值 + 我们的期望 + 可选清单。
async fn takeover_model_get(
    AxumState(st): AxumState<Arc<State>>,
    Path(app): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let app = crate::takeover::ClientApp::parse(&app).map_err(takeover_err)?;
    let (env, ep) = takeover_ctx(&st);
    crate::takeover::model_view(&env, &st.store, &ep, app)
        .map(Json)
        .map_err(takeover_err)
}

/// 切模型：存期望 + 立刻落盘（走接管的快照与原子写），未接管一律拒绝。
async fn takeover_model_put(
    AxumState(st): AxumState<Arc<State>>,
    Path(app): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let app = crate::takeover::ClientApp::parse(&app).map_err(takeover_err)?;
    let sel: crate::takeover::Selection = serde_json::from_value(body)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "invalid_request", e.to_string()))?;
    let (env, ep) = takeover_ctx(&st);
    let report =
        crate::takeover::set_model(&env, &st.store, &ep, app, sel).map_err(takeover_err)?;
    takeover_log(
        &st,
        "TK006 takeover.model_set",
        "client default model switched through gateway",
        report.clone(),
    );
    Ok(Json(report))
}

async fn revert_takeover(
    AxumState(st): AxumState<Arc<State>>,
    Path(app): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let app = crate::takeover::ClientApp::parse(&app).map_err(takeover_err)?;
    let (env, _) = takeover_ctx(&st);
    let report = crate::takeover::revert(&env, &st.store, app).map_err(takeover_err)?;
    takeover_log(
        &st,
        "TK002 takeover.reverted",
        "client configuration restored from takeover backup",
        report.clone(),
    );
    Ok(Json(report))
}

async fn list_takeover_backups(
    AxumState(st): AxumState<Arc<State>>,
) -> Result<Json<Value>, ApiError> {
    let (env, _) = takeover_ctx(&st);
    let backups = crate::takeover::list_backups(&env).map_err(takeover_err)?;
    Ok(Json(json!({ "backups": backups })))
}

async fn restore_takeover_backup(
    AxumState(st): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (env, _) = takeover_ctx(&st);
    let report = crate::takeover::restore_backup(&env, &st.store, &id).map_err(takeover_err)?;
    takeover_log(
        &st,
        "TK003 takeover.restored",
        "takeover backup restored",
        report.clone(),
    );
    Ok(Json(report))
}

async fn delete_takeover_backup(
    AxumState(st): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (env, _) = takeover_ctx(&st);
    crate::takeover::delete_backup(&env, &id).map_err(takeover_err)?;
    takeover_log(
        &st,
        "TK004 takeover.backup_deleted",
        "takeover backup deleted",
        json!({ "backup_id": id }),
    );
    Ok(Json(json!({ "deleted": true })))
}

#[derive(Deserialize)]
struct TakeoverSettings {
    /// Codex 的 wire_api：`chat` | `responses`。
    codex_wire_api: String,
}

async fn get_takeover_settings(
    AxumState(st): AxumState<Arc<State>>,
) -> Result<Json<Value>, ApiError> {
    let (_, ep) = takeover_ctx(&st);
    Ok(Json(json!({ "codex_wire_api": ep.codex_wire_api })))
}

async fn put_takeover_settings(
    AxumState(st): AxumState<Arc<State>>,
    Json(input): Json<TakeoverSettings>,
) -> Result<Json<Value>, ApiError> {
    let value = input.codex_wire_api.trim().to_ascii_lowercase();
    if value != "chat" && value != "responses" {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_input",
            "codex_wire_api 只能是 chat 或 responses",
        ));
    }
    st.store
        .set_setting(crate::takeover::CODEX_WIRE_API_SETTING, &value)
        .map_err(|e| takeover_err(e.to_string()))?;
    takeover_log(
        &st,
        "TK005 takeover.settings_changed",
        "takeover settings updated",
        json!({ "codex_wire_api": value }),
    );
    Ok(Json(json!({ "codex_wire_api": value })))
}

// ============================ 快速启停（M9-C） ============================

fn set_paused(st: &State, paused: bool) -> Json<Value> {
    let changed = st.set_paused(paused);
    if changed {
        let (code, message) = if paused {
            (
                "GW410 gateway.paused",
                "gateway stopped accepting inference traffic",
            )
        } else {
            (
                "GW411 gateway.resumed",
                "gateway resumed accepting inference traffic",
            )
        };
        takeover_log(st, code, message, json!({ "paused": paused }));
    }
    Json(json!({ "paused": st.is_paused() }))
}

async fn pause_gateway(AxumState(st): AxumState<Arc<State>>) -> Json<Value> {
    set_paused(&st, true)
}

async fn resume_gateway(AxumState(st): AxumState<Arc<State>>) -> Json<Value> {
    set_paused(&st, false)
}
