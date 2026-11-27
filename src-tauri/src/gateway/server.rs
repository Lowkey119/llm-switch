use super::{routes, State};
use crate::config::Config;
use crate::store::Store;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

/// 让 axum 把对端地址塞进请求扩展（`ConnectInfo`）。
/// 连接归属（[`super::origin`]）靠它反查「是谁在说话」，少了这一步扩展里永远是空的。
use axum::extract::connect_info::IntoMakeServiceWithConnectInfo;

fn with_peer(
    app: axum::Router,
) -> IntoMakeServiceWithConnectInfo<axum::Router, std::net::SocketAddr> {
    app.into_make_service_with_connect_info::<std::net::SocketAddr>()
}

/// Bind and serve until the process is asked to stop (CLI/headless entry).
pub async fn run(store: Arc<Store>, config: Arc<Config>) -> Result<(), Box<dyn std::error::Error>> {
    let state = State::new(store, config);
    boot_init(&state);
    let cors = build_cors(&state.config);
    let addr = listen_addr(&state).to_string();
    let listener = TcpListener::bind(&addr).await?;
    let app = routes::router(state).layer(cors);
    tracing::info!(
        target: "llm_switch::gateway",
        addr = %addr,
        "gateway listening"
    );
    axum::serve(listener, with_peer(app))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Serve an already-built State forever (used by the desktop shell, whose
/// lifecycle is the tray/menu, not ctrl-c). 端口热切换（UI 重设计）：
/// `port_watch` 一变就优雅关停当前监听、用新端口重绑；绑定失败退避重试。
pub async fn serve_forever(state: Arc<State>) -> Result<(), Box<dyn std::error::Error>> {
    boot_init(&state);
    let cors = build_cors(&state.config);
    loop {
        // 先订阅再读端口：两步之间的变更不会漏（变更只会向前推进版本）。
        let mut rx = state.port_watch.subscribe();
        let addr = listen_addr(&state);
        let listener = match TcpListener::bind(addr.to_string()).await {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(
                    target: "llm_switch::gateway",
                    addr = %addr, error = %e,
                    "bind failed, retrying in 1s"
                );
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };
        tracing::info!(
            target: "llm_switch::gateway",
            addr = %addr,
            backends = state.config.backends.len(),
            "gateway listening"
        );
        // 环回自检：按客户端读系统代理的方式访问一次本地路由，回不到自己说明请求会被
        // Clash 一类系统代理接走（空正文 502）。后台跑，不阻塞服务。
        {
            let port = state.listen_port();
            tokio::spawn(async move {
                let origin = format!("http://127.0.0.1:{port}");
                if let Some(reason) = super::loopback_probe::hijack_reason(&origin).await {
                    tracing::warn!(
                        target: "llm_switch::gateway",
                        origin = %origin, reason = %reason,
                        "环回自检：本地路由可能被系统代理接走，客户端请求或无法到达（检查系统代理绕过列表是否含 127.0.0.1）"
                    );
                }
            });
        }
        let app = routes::router(state.clone()).layer(cors.clone());
        if let Err(e) = axum::serve(listener, with_peer(app))
            .with_graceful_shutdown(async move {
                let _ = rx.changed().await;
            })
            .await
        {
            tracing::warn!(target: "llm_switch::gateway", error = %e, "listener exited");
        }
        // serve 返回：要么端口变更已消费（重绑新值），要么意外退出（下轮重试）。
    }
}

fn listen_addr(state: &State) -> String {
    format!("{}:{}", state.config.server.host, state.listen_port())
}

/// 允许跨源访问的来源清单（桌面壳 + 开发服务器）。
///
/// 早先这里用的是 `CorsLayer::permissive()`——**任意网页**都能对网关发跨源
/// 请求；在 `server.access_token` 未设的默认配置下，浏览器里的恶意站点可以
/// 直接把 127.0.0.1:8787 当成免费的 LLM 代理用。现在只放行桌面壳自己与
/// Vite 开发服务器；要从别的来源访问（例如另一台机器上的网页）请显式写
/// `server.cors_allow_origins`。
const LOCAL_ORIGINS: &[&str] = &[
    // Vite 开发服务器（tauri.conf.json 的 devUrl = ui/vite.config.ts 的端口）
    "http://localhost:1420",
    "http://127.0.0.1:1420",
    // Tauri 桌面壳 webview 的来源（各平台不同）
    "tauri://localhost",
    "http://tauri.localhost",
];

/// CORS 只取决于静态配置，循环外构建一次（CorsLayer: Clone）。
fn build_cors(config: &Config) -> CorsLayer {
    use axum::http::{HeaderValue, Method};
    use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin};
    let mut origins: Vec<HeaderValue> = LOCAL_ORIGINS
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    origins.extend(
        config
            .server
            .cors_allow_origins
            .iter()
            .filter_map(|o| o.parse().ok()),
    );
    CorsLayer::new()
        .allow_methods(AllowMethods::list(vec![
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ]))
        .allow_headers(AllowHeaders::any())
        // `list()` 收到空清单会退化成 Any，这里不可能为空（LOCAL_ORIGINS 非空）。
        .allow_origin(AllowOrigin::list(origins))
        .max_age(std::time::Duration::from_secs(600))
}

/// 一次性启动动作：usage 补算 + 后台维护循环。
fn boot_init(state: &Arc<State>) {
    // 启动补算：上次退出期间新增的 usage 行并入日聚合。
    if let Err(e) = state.store.rollup_usage_daily() {
        tracing::warn!(target: "llm_switch::gateway", error = %e, "startup usage_daily rollup failed");
    }
    // 后台维护：探活/亲和修剪/日志保留（间隔 0 可关）。
    // 丢弃 JoinHandle 不会取消任务（tokio 语义），任务随进程存活。
    drop(super::maintenance::spawn(state.clone()));
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!(target: "llm_switch::gateway", "shutdown signal received");
}
