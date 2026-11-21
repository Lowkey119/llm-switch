//! HTTP gateway: axum router, shared state, and the endpoints that exist today.
//!
//! M0 ships the control-plane surface (`/health`, `/status`, `/v1/models`).
//! The four client protocols arrive in M1/M2 and plug into the same `State`.

pub mod auth;
pub mod backends;
pub mod body;
pub mod cache;
pub mod capability;
pub mod catalog;
pub mod circuit_breaker;
pub mod console;
mod error;
mod forwarder;
mod handlers;
mod json_canonical;
pub mod log;
pub mod loglevel;
pub mod loopback_probe;
pub mod maintenance;
pub mod media_proxy;
mod media_sanitizer;
pub mod metrics;
pub mod model_capabilities;
pub mod model_kind;
pub mod monitor;
pub mod origin;
pub mod params;
pub mod rates;
pub mod request_classifier;
mod response_processor;
pub mod routes;
mod server;
pub mod sse;
pub mod telemetry;
mod tool_media;
mod usage;
pub mod video_proxy;

pub use routes::seed_backends;
pub use server::{run, serve_forever};

use crate::config::Config;
use crate::store::Store;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Instant;

pub struct State {
    pub store: Arc<Store>,
    pub config: Arc<Config>,
    pub started_at: Instant,
    pub forwarder: Arc<forwarder::Forwarder>,
    /// Rotation cursor for the weighted/round-robin routing policies.
    pub rotate: AtomicU64,
    /// Session-scoped Gemini replay state (thought signatures / tool turns).
    pub gemini_shadow: Arc<backends::gemini_shadow::GeminiShadowStore>,
    /// Hot-swappable layered parameter table (参数中心).
    pub params: std::sync::RwLock<params::ParamTable>,
    /// Structured operational log sink (运行日志入库).
    pub log: Arc<log::LogSink>,
    /// In-flight streaming exchanges for the Monitor page.
    pub monitor: Arc<monitor::ActiveRegistry>,
    /// 1s 滑窗速率聚合（M4-A）：usage 落库漏斗喂数。
    pub metrics: std::sync::Arc<metrics::Metrics>,
    /// 响应缓存（M5.5-A）：内存热层 LRU，B 片挂磁盘冷层。
    pub cache: std::sync::Arc<cache::ResponseCache>,
    /// 缓存策略表叠加（M5.5-C）：Settings/Cache 页写 cache_policies 表，启动叠加。
    pub cache_policies: std::sync::RwLock<cache::PolicyOverlay>,
    /// 后端 KV 遥测快照（M5.5-D）：维护循环周期刷新，cache_aware 路由消费。
    pub telemetry: std::sync::RwLock<std::collections::HashMap<i64, telemetry::KvTelemetry>>,
    /// Hot routing section (Settings 页可改，持久化 settings 表覆盖)。
    pub routing: std::sync::RwLock<crate::config::RoutingCfg>,
    /// halogen 机位（SSH 控制面）热改层：config.yaml 打底，settings 表 `halctl.overlay` 覆盖。
    /// 用户的机器不一定叫 102、内网与端口也可能不同，所以地址必须可改而不用手编 YAML。
    pub halctl: std::sync::RwLock<crate::config::HalctlCfg>,
    /// gufo 机位热改层（与 halctl 同构，`guctl.overlay`）。
    pub guctl: std::sync::RwLock<crate::config::HalctlCfg>,
    /// 菜单栏「快速启停」的闸门（D22）：true = 暂停接单，推理入口直接 503，
    /// 控制面（管理端点/UI）照常可用，进程不退。
    pub paused: std::sync::atomic::AtomicBool,
    /// 统一转发端口（UI 重设计：状态胶囊双击原位编辑）。settings 表 `server.port`
    /// 覆盖 config 文件；运行时改动经 port_watch 通知 serve 循环重绑。
    pub port_now: std::sync::atomic::AtomicU64,
    pub port_watch: Arc<tokio::sync::watch::Sender<u16>>,
    /// Per-backend circuit breakers (created lazily, keyed by backend id).
    pub breakers:
        std::sync::RwLock<std::collections::HashMap<i64, Arc<circuit_breaker::CircuitBreaker>>>,
    /// 引擎原生速率聚合（halogen 差分基线 + TTL 快照缓存，监控页/浮窗共享）。
    pub engine_rates: crate::services::engine_rates::EngineRates,
    /// 系统资源监控（本机 macOS + 102 SSH 硬件指标，TTL 缓存，切片 12）。
    pub sysmon: crate::services::sysmon::SysMon,
    /// 网关→GUI 回调通道（#46）：代理请求挂号时把代理名递给 GUI（速率线自动弹出）。
    /// GUI 启动时装入发送端；headless（无 GUI）为 None，网关侧静默跳过。
    pub agent_hit_tx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
    /// 视频任务亲和表（切片 B / 12）：命名空间 job id → 后端。**内存表，重启即失**
    /// （计划拍板：任务生命周期短，不做持久化）。
    pub video_jobs: Arc<video_proxy::JobTable>,
}

/// Get-or-create the breaker for a backend id (honours routing.circuit_breaker).
pub fn breaker_for(st: &State, backend_id: i64) -> Arc<circuit_breaker::CircuitBreaker> {
    {
        let map = st.breakers.read().expect("breakers lock poisoned");
        if let Some(b) = map.get(&backend_id) {
            return b.clone();
        }
    }
    let mut map = st.breakers.write().expect("breakers lock poisoned");
    map.entry(backend_id)
        .or_insert_with(|| {
            Arc::new(circuit_breaker::CircuitBreaker::new(
                circuit_breaker::CircuitBreakerConfig::from(
                    &st.routing
                        .read()
                        .expect("routing lock poisoned")
                        .circuit_breaker,
                ),
            ))
        })
        .clone()
}

/// Reload the parameter override table from the store (called at boot and
/// after any admin/command mutation).
pub fn reload_params(st: &Arc<State>) {
    let table = params::ParamTable::from_rows(st.store.list_param_overrides().unwrap_or_default());
    *st.params.write().expect("params lock poisoned") = table;
}

/// 参数变更后的一站式处理（M5.5-C）：重载参数表 + 响应缓存整体失效。
/// 口径：缓存键只含客户端可见参数，参数中心的改动不在键里，
/// 所以参数一变就把存量条目清光（§5.5.4 「清相关条目」的安全超集）。
pub fn on_params_changed(st: &Arc<State>) {
    reload_params(st);
    let cleared = st.cache.clear();
    if cleared > 0 {
        st.log.log(log::Event {
            level: log::Level::Info,
            code: "CA103 cache.invalidated",
            message: "response cache invalidated by parameter change",
            request_id: None,
            backend: None,
            model: None,
            extra: serde_json::json!({ "cleared": cleared }),
        });
    }
}

/// 重载缓存策略表叠加（策略变更后）。
pub fn reload_cache_policies(st: &Arc<State>) {
    let ov = State::initial_cache_policies(&st.store);
    *st.cache_policies
        .write()
        .expect("cache_policies lock poisoned") = ov;
}

/// 统一转发端口的最终口径：settings 表覆盖（胶囊改过）> config 文件。
///
/// State 构造与启动时的速率发现文件（`ext_install::write_endpoint_file`）都必须走
/// 这里：从前启动写文件直接取 `config.server.port`，于是胶囊改过端口后一重启，
/// 发现文件被写回 yaml 里的旧端口，各家代理的速率扩展集体取不到数。
pub fn effective_listen_port(store: &Store, config: &Config) -> u16 {
    store
        .get_setting("server.port")
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p > 0)
        .unwrap_or(config.server.port)
}

impl State {
    pub fn new(store: Arc<Store>, config: Arc<Config>) -> Arc<Self> {
        let initial_params =
            params::ParamTable::from_rows(store.list_param_overrides().unwrap_or_default());
        let initial_routing = Self::initial_routing(&store, &config);
        let (initial_halctl, initial_guctl) = Self::initial_halctl(&store, &config);
        let initial_cache_policies = Self::initial_cache_policies(&store);
        let initial_cache = Self::initial_cache(&store, &config);
        let log = log::LogSink::new(store.clone());
        let monitor = monitor::ActiveRegistry::new();
        // 端口优先级：settings 表覆盖（胶囊改过）> config 文件。
        let listen_port = effective_listen_port(&store, &config);
        let (port_tx, _) = tokio::sync::watch::channel(listen_port);
        Arc::new(Self {
            store,
            config: config.clone(),
            started_at: Instant::now(),
            forwarder: Arc::new(forwarder::Forwarder::new(&config.timeouts)),
            rotate: AtomicU64::new(0),
            gemini_shadow: Arc::new(backends::gemini_shadow::GeminiShadowStore::new()),
            params: std::sync::RwLock::new(initial_params),
            log,
            monitor,
            metrics: std::sync::Arc::new(metrics::Metrics::default()),
            cache: std::sync::Arc::new(initial_cache),
            cache_policies: std::sync::RwLock::new(initial_cache_policies),
            telemetry: std::sync::RwLock::new(std::collections::HashMap::new()),
            routing: std::sync::RwLock::new(initial_routing),
            halctl: std::sync::RwLock::new(initial_halctl),
            guctl: std::sync::RwLock::new(initial_guctl),
            paused: std::sync::atomic::AtomicBool::new(false),
            port_now: AtomicU64::new(listen_port as u64),
            port_watch: Arc::new(port_tx),
            breakers: std::sync::RwLock::new(std::collections::HashMap::new()),
            engine_rates: crate::services::engine_rates::EngineRates::new(),
            sysmon: crate::services::sysmon::SysMon::new(),
            agent_hit_tx: std::sync::Mutex::new(None),
            video_jobs: Arc::new(video_proxy::JobTable::default()),
        })
    }

    /// 当前监听端口（热覆盖后的真值）。
    pub fn listen_port(&self) -> u16 {
        self.port_now.load(std::sync::atomic::Ordering::Relaxed) as u16
    }

    /// 改端口：更新原子值并广播给 serve 循环（持久化由调用方负责）。
    pub fn broadcast_port(&self, port: u16) {
        self.port_now
            .store(port as u64, std::sync::atomic::Ordering::Relaxed);
        let _ = self.port_watch.send(port);
    }

    /// config.routing 叠加 settings 表里的持久化覆盖（Settings 页写这层）。
    fn initial_routing(store: &Store, config: &Config) -> crate::config::RoutingCfg {
        let mut rc = config.routing.clone();
        if let Ok(Some(raw)) = store.get_setting("routing.overlay") {
            if let Ok(overlay) = serde_json::from_str::<crate::config::RoutingCfg>(&raw) {
                rc = overlay;
            }
        }
        rc
    }

    /// halogen / gufo 机位：config.yaml 打底，settings 表 `halctl.overlay` / `guctl.overlay` 覆盖。
    /// 覆盖层整节替换（与 routing.overlay 同口径）：只存用户真改过的那一节，缺省即出厂事实。
    fn initial_halctl(
        store: &Store,
        config: &Config,
    ) -> (crate::config::HalctlCfg, crate::config::HalctlCfg) {
        let overlay = |key: &str, base: &crate::config::HalctlCfg| -> crate::config::HalctlCfg {
            match store.get_setting(key) {
                Ok(Some(raw)) => serde_json::from_str::<crate::config::HalctlCfg>(&raw)
                    .unwrap_or_else(|_| base.clone()),
                _ => base.clone(),
            }
        };
        (
            overlay("halctl.overlay", &config.halctl),
            overlay("guctl.overlay", &config.guctl),
        )
    }

    /// 响应缓存分层（M5.5-B）：persist 且 disk_gb>0 才启用磁盘冷层（D9）。
    fn initial_cache(store: &Store, config: &Config) -> cache::ResponseCache {
        // 分层预算走 settings 覆盖层（`cache.*`，下次重启生效的口径见 cache.rs）。
        let rc = &config.cache.response;
        let tier = cache::tier_from_store(store, rc);
        let disk_on = tier.persist && tier.disk_gb > 0;
        if disk_on {
            cache::ResponseCache::new_with_disk(
                tier.memory_mb,
                config.cache_dir(),
                tier.disk_gb as u64,
            )
        } else {
            cache::ResponseCache::new(tier.memory_mb)
        }
    }

    /// cache_policies 表 → 叠加层（scope = "global" | "model:<名>"）。
    fn initial_cache_policies(store: &Store) -> cache::PolicyOverlay {
        let mut ov = cache::PolicyOverlay::default();
        for (scope, policy) in store.list_cache_policies().unwrap_or_default() {
            match scope.strip_prefix("model:") {
                Some(m) => {
                    ov.models.insert(m.to_string(), policy);
                }
                None => {
                    if scope == "global" {
                        ov.global = Some(policy);
                    }
                }
            }
        }
        ov
    }

    /// 当前策略叠加的拷贝（每请求一次，量小无所谓）。
    pub fn cache_overlays(&self) -> cache::PolicyOverlay {
        self.cache_policies
            .read()
            .expect("cache_policies lock poisoned")
            .clone()
    }

    /// 暂停/恢复接单（幂等：状态没变返回 false，调用方据此决定是否记日志）。
    pub fn set_paused(&self, paused: bool) -> bool {
        let previous = self
            .paused
            .swap(paused, std::sync::atomic::Ordering::Relaxed);
        previous != paused
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn uptime_ms(&self) -> u128 {
        self.started_at.elapsed().as_millis()
    }
}

/// Full router (control plane + inference) — reused by the server and by
/// integration tests that drive the gateway in-process over a real socket.
pub fn app(state: Arc<State>) -> axum::Router {
    routes::router(state)
}

/// Uniform JSON error shape; M2 reshapes it per entry protocol.
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg)
    }
}

impl From<crate::store::StoreError> for ApiError {
    fn from(err: crate::store::StoreError) -> Self {
        ApiError::internal(err.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": { "type": self.code, "message": self.message } })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 胶囊改过端口后 settings 表才是真值：State 与启动时的速率发现文件都必须吃
    /// 这个口径。从前启动写 `rate-ep.json` 直接取 `config.server.port`，于是改过
    /// 端口后一重启，发现文件被写回 yaml 旧端口，各家代理的速率扩展集体断供。
    #[test]
    fn effective_port_prefers_settings_over_config() {
        let store = Store::in_memory().expect("in-memory store");
        let config = Config::default();
        let base = config.server.port;
        assert_eq!(effective_listen_port(&store, &config), base);
        store
            .set_setting("server.port", "8899")
            .expect("set setting");
        assert_eq!(effective_listen_port(&store, &config), 8899);
        // 脏值（0 / 非数字）不当真值用，退回 config。
        store.set_setting("server.port", "0").expect("set setting");
        assert_eq!(effective_listen_port(&store, &config), base);
        store
            .set_setting("server.port", "not-a-port")
            .expect("set setting");
        assert_eq!(effective_listen_port(&store, &config), base);
    }
}
