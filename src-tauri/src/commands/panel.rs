//! 后端页重设计配套命令（docs/ui_pages/01-后端页.md）。
//!
//! 四组能力：
//! · `model_metrics` —— ②区模型行的数据源（数据流点、prefill/decode、实际ctx、其他指示灯）；
//! · `refresh_backend` —— ①区 ⟳ 按钮：探活 + 重扫上游 + diff 合并（二次确认在 UI 侧把关）；
//! · `check_port` / `set_listen_port` —— 顶部状态胶囊的双击改端口（试占探测 + 热重绑）；
//! · `copilot_get` / `copilot_set` —— copilot（Ollama 门面下游）的虚拟分配池：
//!   没有可写的客户端配置文件，清单存 settings 表，门面 `/api/tags` 按其过滤。

use crate::gateway::State;
use crate::services::{backend, discover};
use crate::types::now_ms;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;
use tauri::State as TauriState;

/// 历史统计窗口：7 天。
const STATS_WINDOW_MS: i64 = 7 * 86_400_000;

/// ②区模型指标：每个暴露模型一条，UI 每秒轮询（仅展开时）。
/// 口径（与规格一致）：空闲显历史均值，在飞切实时；拿不到一律 null（UI 显 `—`）。
#[tauri::command]
pub fn model_metrics(st: TauriState<Arc<State>>) -> Result<Value, String> {
    let rows = st.store.list_backends().map_err(|e| e.to_string())?;
    let mut models: BTreeSet<String> = Default::default();
    for b in &rows {
        models.extend(b.models.iter().cloned());
    }
    let snaps = st.monitor.snapshot();
    let agg = st.metrics.snapshot_models(&st.monitor.count_by_model());
    let since = now_ms() - STATS_WINDOW_MS;
    // 元数据有效值（人工声明 ⊕ 后端参数 ⊕ 探测，跨后端取最小；未知 = null 不瞎猜）。
    let meta = st.store.effective_model_meta().unwrap_or_default();

    let out: Vec<Value> = models
        .iter()
        .map(|m| {
            let live = snaps.iter().find(|s| &s.model == m);
            let mm = agg.iter().find(|x| &x.model == m);
            let (avg_pf, ctx_used, other) = st
                .store
                .model_usage_stats(m, since)
                .unwrap_or((None, None, false));
            let streaming = live.is_some() || mm.map(|x| x.active_streams > 0).unwrap_or(false);
            // 实时速率：与浮条同一公式、同一纠偏键（分模型分后端，rates.rs 统一出口）。
            let prefill = live
                .filter(|s| s.phase == "prefill" && s.prefill_ms > 0)
                .map(|s| {
                    let k = st.metrics.pre_coeff(&s.backend, &s.model);
                    crate::gateway::rates::prefill_rate(
                        s.req_bytes,
                        s.est_input_tokens,
                        s.prefill_ms,
                        k,
                    )
                })
                .or(avg_pf);
            let decode = live
                .map(|s| {
                    let k = st.metrics.dec_coeff(&s.backend, &s.model);
                    let gen = s
                        .ttft_ms
                        .map(|t| s.elapsed_ms.saturating_sub(t))
                        .unwrap_or(0);
                    crate::gateway::rates::dec_rate(s.bytes, s.data_events, gen, k)
                })
                .filter(|v| *v > 0.0)
                .or_else(|| mm.map(|x| x.tokens_per_s).filter(|v| *v > 0.0));
            json!({
                "model": m,
                "streaming": streaming,
                "prefill_tps": prefill.map(|v| v.round() as i64),
                "decode_tps": decode.map(|v| v.round() as i64),
                "ctx_used": ctx_used,
                // 支持 ctx：模型元数据解析（人工 > 后端参数/探测 > 未知 null，UI 显 —）。
                "ctx_max": meta.get(m).and_then(|e| e.ctx).map(Value::from).unwrap_or(Value::Null),
                "other_active": other,
                // VLM 三态（模型元数据 VLM 声明（人工>探测）> 已确认名单 > Unknown 放行）。
                "vlm": match crate::gateway::model_capabilities::resolve_image_input_capability(
                    m, meta.get(m).and_then(|e| e.vlm), true,
                ) {
                    crate::gateway::model_capabilities::ImageInputCapability::Supported => "sup",
                    crate::gateway::model_capabilities::ImageInputCapability::Unsupported => "uns",
                    crate::gateway::model_capabilities::ImageInputCapability::Unknown => "unk",
                },
            })
        })
        .collect();
    Ok(Value::Array(out))
}

/// ①区 ⟳：探活 + 重扫上游模型清单 + diff 合并。
/// 不健康时不扫描、不动清单（本地数据原样保留，恢复靠下次刷新）。
/// 移除项只从对外清单摘下、配置原地暂存（摘挂不销毁，见 `sync_backend_models`）。
#[tauri::command]
pub async fn refresh_backend(
    st: TauriState<'_, Arc<State>>,
    name: String,
) -> Result<Value, String> {
    let store = st.store.clone();
    let row = store
        .list_backends()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.name == name)
        .ok_or_else(|| format!("后端 `{name}` 不存在"))?;
    if !row.enabled {
        // 停用后端不探活：互斥引擎（halogen/gufo）共用地址，探活打到的是
        // 别家在应答，报「成功」纯属说谎（UI 侧 ⟳ 同步禁用，这里兜底）。
        return Err(format!("后端 `{name}` 已停用，启用后才能探活"));
    }
    let probe = backend::probe(&store, &name).await?;
    if !probe.healthy {
        return Ok(json!({
            "healthy": false,
            "scan_failed": false,
            "latency_ms": probe.latency_ms,
            "error": probe.error,
            "added": [], "removed": [], "kept": row.models.len(),
        }));
    }
    let input = discover::DiscoverInput {
        base_url: row.base_url.clone(),
        protocol: row.protocol.clone(),
        // 列表接口只回掩码；这里拿的是库内真值，防御性再过一遍掩码。
        api_key: row.api_key.filter(|k| !k.is_empty() && k != "***"),
    };
    // 探活过了、扫描却失败：这是「扫描失败」，不是「后端挂了」。以前这里直接 Err，
    // UI 弹「刷新失败：扫描上游失败：…」，读起来像后端连不上（实报：oMLX 那行）。
    // 拆开报：healthy=true + scan_failed=true，本地清单原样保留。
    let disc = match discover::discover(&input).await {
        Ok(d) => d,
        Err(e) => {
            return Ok(json!({
                "healthy": true,
                "scan_failed": true,
                "latency_ms": probe.latency_ms,
                "error": e,
                "added": [], "removed": [], "kept": row.models.len(),
            }));
        }
    };
    let found: Vec<crate::types::ScannedModel> = disc
        .models
        .into_iter()
        .map(|c| crate::types::ScannedModel {
            id: c.id,
            ctx: c.ctx,
            output: c.output,
            reasoning: c.reasoning,
            vlm: c.vlm,
        })
        .collect();
    let sync = store
        .sync_backend_models(&name, &found)
        .map_err(|e| e.to_string())?;
    // 持久化：⟳ 改了 DB 清单就必须镜像回 config.yaml，否则下次启动 seed_backends
    // 会拿陈旧 yaml 把新增模型洗掉（实报：刷新追加的模型重启后消失，再刷新才回来）。
    if let Err(e) = super::backend::persist_config_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::backend", backend = %name, "persist config.yaml failed: {e}");
    }
    // 刷新顺带落了新的探测真值：受管端的接管配置跟着自动重写（无差异不写盘）。
    super::backend::resync_takeovers(st.inner());
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "BE320 backend.refresh",
        message: "backend model list refreshed through the panel",
        request_id: None,
        backend: Some(&name),
        model: None,
        extra: json!({
            "added": sync.added.len(), "removed": sync.removed.len(),
            "kept": sync.kept, "via": "ipc",
        }),
    });
    Ok(json!({
        "healthy": true,
        "scan_failed": false,
        "latency_ms": probe.latency_ms,
        "error": Value::Null,
        "added": sync.added, "removed": sync.removed, "kept": sync.kept,
    }))
}

/// 端口试占探测（不含自身正在监听的端口 —— 改成当前值等于没改，UI 侧拦）。
#[tauri::command]
pub fn check_port(st: TauriState<Arc<State>>, port: u16) -> Result<bool, String> {
    if port == 0 {
        return Err("端口需为 1–65535".into());
    }
    Ok(port_free(&st.config.server.host, port))
}

/// 更换统一转发端口：试占 → 持久化 settings → 广播 serve 循环重绑。
/// 返回新的 `host:port`。竞态抢占（试占后被人抢）由重绑失败的退避重试兜底。
#[tauri::command]
pub fn set_listen_port(st: TauriState<Arc<State>>, port: u16) -> Result<String, String> {
    if port == 0 {
        return Err("端口需为 1–65535".into());
    }
    if port != st.listen_port() && !port_free(&st.config.server.host, port) {
        return Err(format!("端口 {port} 已被占用"));
    }
    // 旧端口先拿住：接管库里记的就是它，后面联动重写靠它认领接管态。
    let old_root = crate::takeover::Endpoint::base_root(&st.config.server.host, st.listen_port());
    st.store
        .set_setting("server.port", &port.to_string())
        .map_err(|e| e.to_string())?;
    st.broadcast_port(port);
    // 代理侧速率扩展靠发现文件找网关，改端口必须同步重写（否则扩展集体取不到数）。
    if let Err(e) = crate::services::ext_install::write_endpoint_file_for_app(
        &st.config.data_dir(),
        &st.config.server.host,
        port,
        st.config.server.access_token.as_deref(),
    ) {
        tracing::warn!(target: "llm_switch::ext", "速率端点发现文件重写失败：{e}");
    }
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Warn,
        code: "GW420 gateway.port_changed",
        message: "unified listen port changed; in-flight requests were cut over",
        request_id: None,
        backend: None,
        model: None,
        extra: json!({ "port": port, "via": "ipc" }),
    });
    // 接管联动（用户 2026-11-18）：接管写进各家配置的 base_url 带着旧端口，不重写
    // 客户端就一直往旧端口打。口径与启动 resync / 元数据 resync 一致，只重写我们
    // 自己写过的条目；失败只告警不阻断改端口本身（端口已经生效）。
    let mut new_cfg = st.config.as_ref().clone();
    new_cfg.server.port = port;
    let env = crate::takeover::TakeoverEnv::from_config(&st.config, &st.store);
    for r in crate::takeover::resync_after_endpoint_change(&env, &st.store, &new_cfg, &old_root) {
        if r.get("error").is_some() {
            tracing::warn!(target: "llm_switch::takeover", report = %r, "改端口后重写接管配置失败");
            continue;
        }
        st.log.log(crate::gateway::log::Event {
            level: crate::gateway::log::Level::Info,
            code: "TK006 takeover.port_resynced",
            message: "taken-over client config rewritten after listen port change",
            request_id: None,
            backend: None,
            model: None,
            extra: r.clone(),
        });
    }
    Ok(format!("{}:{}", st.config.server.host, port))
}

/// 用系统默认浏览器打开外链（用户 2026-11-18：发布仓库那两个链接点了要拉起浏览器）。
///
/// webview 里 `target="_blank"` 不会把链接交给系统浏览器，必须走进程入口。只放行
/// http/https：入口一旦开了就该自己把关，`file://` / `javascript:` 这类一律拒。
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err("只允许打开 http/https 链接".into());
    }
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let opener = "xdg-open";
    std::process::Command::new(opener)
        .arg(u)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 在**应用内**开一个 webview 窗口加载外链（用户 2026-11-19 拍板：全部外链统一
/// 走内置浏览器，系统浏览器 `open_url` 只作兜底）——控制台 iframe 证明过 web
/// 内容在本应用里能正常跑，外链同理，不必跳出到 Safari/Chrome。
///
/// 同一 URL 复用同一窗口：label 由 URL 哈希派生，窗口还开着就聚焦它，
/// 关了就重建；窗口自带原生标题栏（关闭 / 缩放 / 全屏都是系统标准件）。
#[tauri::command]
pub fn open_internal(app: tauri::AppHandle, url: String) -> Result<(), String> {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return Err("只允许打开 http/https 链接".into());
    }
    let parsed: url::Url = u.parse().map_err(|e| format!("地址无效：{e}"))?;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use tauri::Manager;
    let mut h = DefaultHasher::new();
    u.hash(&mut h);
    let label = format!("web-{:016x}", h.finish());
    if let Some(win) = app.get_webview_window(&label) {
        // 复用不等于活着：窗口可能被最小化或隐藏，只 set_focus 会「点了没反应」
        // （用户 2026-11-20：第二次点同一网址打不开）。三步齐做，全平台无害。
        let _ = win.unminimize();
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }
    let title = parsed.host_str().unwrap_or("llm-switch").to_string();
    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(parsed))
        .title(title)
        .inner_size(1100.0, 780.0)
        .center()
        .build()
        .map(|win| {
            let _ = win.set_focus();
        })
        .map_err(|e| e.to_string())
}

fn port_free(host: &str, port: u16) -> bool {
    std::net::TcpListener::bind((host, port)).is_ok()
}

// ── copilot 虚拟分配池 ─────────────────────────────────────────────

/// settings 里的 copilot 清单；`None` = 缺省全暴露（与矩阵其它端同规矩）。
/// 读入口径在 store（网关门面也要读，那里吃不到 gui 特性）。
#[tauri::command]
pub fn copilot_get(st: TauriState<Arc<State>>) -> Result<Value, String> {
    Ok(json!({ "allow": st.store.copilot_allow() }))
}

/// 分配/摘除 copilot 可见模型。缺省（allow=null）= 全部暴露可见。
/// 写逻辑与矩阵格子共用 `takeover::copilot_assign`（分配统一入口）。
#[tauri::command]
pub fn copilot_set(st: TauriState<Arc<State>>, model: String, add: bool) -> Result<Value, String> {
    crate::takeover::copilot_assign(&st.store, &model, add)
}
