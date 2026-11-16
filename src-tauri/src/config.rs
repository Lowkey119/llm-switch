//! Configuration loading and defaults (`configs/config.example.yaml` documents it).

use crate::engine::Engine;
use crate::store::Store;
use crate::types::{BackendRow, BackendUpsert, Protocol};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("invalid config: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerCfg {
    pub host: String,
    pub port: u16,
    /// 换血端口（无缝升级用）：`--swap` 起新实例时听这个口，与旧实例（port）
    /// 并行共存；旧实例被杀后新实例再改回 port 接管，客户端配置全程不动。
    pub swap_port: u16,
    /// Optional bearer token clients must present. `None` = open loopback proxy.
    pub access_token: Option<String>,
    /// Extra `Access-Control-Allow-Origin` values for the web client.
    pub cors_allow_origins: Vec<String>,
    /// Background health-probe & housekeeping cadence (0 disables).
    pub maintenance_interval_secs: u64,
    /// 入站请求体上限（MiB）。默认 64，远大于上游 Anthropic 的 32MB——代理不该比
    /// 上游更早把关，否则客户端只会收到一句指向错误方向的「Request too large」。
    pub max_body_mb: u32,
}

impl Default for ServerCfg {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8787,
            swap_port: 8899,
            access_token: None,
            cors_allow_origins: Vec::new(),
            maintenance_interval_secs: 30,
            max_body_mb: 64,
        }
    }
}

impl ServerCfg {
    /// 已配置的访问令牌（空白视为未设）。网关侧的校验在 `gateway::auth`。
    pub fn token(&self) -> Option<&str> {
        self.access_token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
    }

    /// `max_body_mb` 换算成字节；0 视为未配置（回落默认 64），上限钳到 512 MiB，
    /// 免得一个笔误把代理变成 OOM 制造机（请求体是全量缓冲进内存的）。
    pub fn max_body_bytes(&self) -> usize {
        (if self.max_body_mb == 0 {
            64
        } else {
            self.max_body_mb
        }
        .min(512)) as usize
            * 1024
            * 1024
    }
}

/// How the router picks a backend among the eligible ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RoutingPolicy {
    #[default]
    WeightedRoundRobin,
    RoundRobin,
    LeastBusy,
    Failover,
    /// Same score basis as LeastBusy but prefix/KV residency dominates (§5.5.7).
    CacheAware,
}

impl std::fmt::Display for RoutingPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RoutingPolicy::WeightedRoundRobin => "weighted_round_robin",
            RoutingPolicy::RoundRobin => "round_robin",
            RoutingPolicy::LeastBusy => "least_busy",
            RoutingPolicy::Failover => "failover",
            RoutingPolicy::CacheAware => "cache_aware",
        };
        f.write_str(s)
    }
}

/// Prefix/session affinity settings (§5.5.0 A).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AffinityCfg {
    pub enabled: bool,
    pub ttl_secs: i64,
    /// Below this many characters of leading context, pinning is not worth it.
    pub min_prefix_chars: usize,
}

impl Default for AffinityCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_secs: 1800,
            min_prefix_chars: 2000,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RoutingCfg {
    pub policy: RoutingPolicy,
    /// How many other backends to try after the first failure.
    pub retries: u32,
    /// Give a failing backend a cooling-off period before retrying it.
    pub cooldown_secs: i64,
    pub affinity: AffinityCfg,
    pub circuit_breaker: CircuitCfg,
}

/// 熔断器默认参数（M3）：连续 3 次失败开断，30s 后半开试探 1 个请求。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CircuitCfg {
    pub enabled: bool,
    pub failure_threshold: u32,
    /// 半开状态下连续成功多少次回到闭合。
    pub success_threshold: u32,
    /// 开断后多少秒进入半开。
    pub timeout_secs: u64,
    /// 错误率兜底（样本足够时才生效）。
    pub error_rate_threshold: f64,
    pub min_requests: u32,
}

impl Default for CircuitCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            failure_threshold: 3,
            success_threshold: 1,
            timeout_secs: 30,
            error_rate_threshold: 0.6,
            min_requests: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeoutCfg {
    pub connect_ms: u64,
    /// Fail before first byte.
    pub first_byte_ms: u64,
    /// Max gap between two streamed chunks.
    pub idle_stream_ms: u64,
    /// Hard ceiling for the whole exchange.
    pub total_ms: u64,
}

impl Default for TimeoutCfg {
    fn default() -> Self {
        Self {
            connect_ms: 5_000,
            first_byte_ms: 60_000,
            idle_stream_ms: 120_000,
            total_ms: 1_800_000,
        }
    }
}

/// Response cache (§5.5). Off by default: reusing replies is opt-in per model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResponseCacheCfg {
    pub enabled: bool,
    /// Only count messages (+tools) in the key, ignore sampling params.
    pub key_granularity: KeyGranularity,
    pub default_ttl_secs: i64,
    /// Hot tier budget in memory.
    pub memory_mb: u32,
    /// Cold tier on disk, **user-set** (D9). `0` disables the disk tier.
    pub disk_gb: u32,
    /// Persist entries across restarts by scanning the disk tier at startup.
    pub persist: bool,
    /// `instant` | `paced`
    pub replay: String,
    /// Refuse to cache requests carrying tool definitions.
    pub exclude_when_tools: bool,
    /// Maximum temperature still considered cacheable.
    pub max_temperature: f64,
    /// Per model overrides: name -> settings blob.
    pub per_model: BTreeMap<String, Value>,
}

impl Default for ResponseCacheCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            key_granularity: KeyGranularity::Strict,
            default_ttl_secs: 3600,
            memory_mb: 256,
            disk_gb: 8,
            persist: true,
            replay: "instant".into(),
            exclude_when_tools: true,
            max_temperature: 0.0,
            per_model: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeyGranularity {
    /// Key includes the effective parameters: changing a knob invalidates.
    #[default]
    Strict,
    /// Key ignores sampling parameters; suits temperature = 0 setups.
    Loose,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheCfg {
    pub response: ResponseCacheCfg,
    pub affinity: AffinityCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LogsCfg {
    /// Initial `EnvFilter` directive; hot-changeable.
    pub level: String,
    pub retention_days: i64,
    /// Rotate the structured log table once it grows past this many megabytes.
    pub max_mb: u32,
    /// Capture request/response bodies (masked, size-capped). Off by default.
    pub capture_bodies: bool,
    pub max_body_bytes: usize,
}

impl Default for LogsCfg {
    fn default() -> Self {
        Self {
            level: "info,llm_switch=info".into(),
            retention_days: 7,
            max_mb: 50,
            capture_bodies: false,
            max_body_bytes: 8192,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackendCfg {
    pub name: String,
    pub base_url: String,
    /// `anthropic` | `openai_chat` | `openai_responses` | `gemini`
    pub protocol: String,
    /// `vllm` | `sglang` | `llama.cpp` | `ollama` | `omlx` | `lmstudio` | `custom`
    pub engine: String,
    pub api_key: Option<String>,
    pub weight: i64,
    /// 启用态：`None`（yaml 没写）= **不覆盖 DB 运行态**——UI 的启用开关只写 DB
    /// 不回写 yaml，播种若拿默认 true 洗回去，用户关掉的后端每次启动都复活
    /// （实报 bug）。显式 `true`/`false` = 每次启动都以 yaml 为准（headless
    /// 用户管的就是这个字段）。见 `gateway::seed_backends`。
    pub enabled: Option<bool>,
    pub models: Vec<String>,
    pub aliases: BTreeMap<String, String>,
    /// Overrides the engine preset when present.
    pub capabilities: Option<Value>,
    pub params: Value,
    pub webui_url: Option<String>,
    /// Console login (optional, paired with webui_url).
    #[serde(default)]
    pub webui_user: Option<String>,
    #[serde(default)]
    pub webui_password: Option<String>,
    pub health_path: Option<String>,
}

impl Default for BackendCfg {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_url: String::new(),
            protocol: String::new(),
            engine: String::new(),
            api_key: None,
            weight: 1,
            enabled: None,
            models: Vec::new(),
            aliases: BTreeMap::new(),
            capabilities: None,
            params: Value::Null,
            webui_url: None,
            webui_user: None,
            webui_password: None,
            health_path: None,
        }
    }
}

impl BackendCfg {
    /// Resolve engine (falling back to the protocol) and fill preset gaps.
    pub fn resolve_engine(&self) -> Engine {
        if let Some(e) = Engine::parse(&self.engine) {
            if !matches!(e, Engine::Custom) || !self.engine.is_empty() {
                return e;
            }
        }
        Engine::Custom
    }

    pub fn resolve_protocol(&self) -> Result<Protocol, String> {
        if let Some(p) = Protocol::parse(&self.protocol) {
            return Ok(p);
        }
        if self.protocol.trim().is_empty() {
            return Ok(self.resolve_engine().default_protocol());
        }
        Err(format!(
            "backend {}: unknown protocol `{}`",
            self.name, self.protocol
        ))
    }

    pub fn capabilities(&self) -> Value {
        self.capabilities
            .clone()
            .unwrap_or_else(|| self.resolve_engine().capabilities())
    }

    pub fn webui_url(&self) -> Option<String> {
        if let Some(u) = &self.webui_url {
            return Some(u.clone());
        }
        let path = self.resolve_engine().default_webui_path()?;
        Some(format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        ))
    }

    pub fn to_upsert(&self) -> Result<BackendUpsert, String> {
        let protocol = self.resolve_protocol()?;
        Ok(BackendUpsert {
            name: self.name.clone(),
            base_url: self.base_url.trim_end_matches('/').to_string(),
            protocol,
            api_key: self.api_key.clone(),
            weight: self.weight.max(1),
            // None = 未在 yaml 指定：新后端默认启用；已有后端由 seed_backends
            // 按 DB 运行态覆盖，这里只是占位（见 seed_backends 注释）。
            enabled: self.enabled.unwrap_or(true),
            engine: Some(self.resolve_engine().as_str().to_string()),
            webui_url: self.webui_url(),
            webui_user: self.webui_user.clone(),
            webui_password: self.webui_password.clone(),
            health_path: self.health_path.clone(),
            models: self.models.clone(),
            aliases: serde_json::to_value(&self.aliases)
                .unwrap_or_else(|_| Value::Object(Default::default())),
            capabilities: self.capabilities(),
            params: self.params.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerCfg,
    pub routing: RoutingCfg,
    pub timeouts: TimeoutCfg,
    pub cache: CacheCfg,
    pub logs: LogsCfg,
    /// Global parameter defaults, lowest layer of the override stack (§5.3).
    pub params: Value,
    pub backends: Vec<BackendCfg>,
    /// Where the sqlite db, logs and cache live. Defaults to `~/.llm-switch`.
    pub data_dir: Option<PathBuf>,
    /// halogen 控制台（SSH 控制面）：双地址故障转移 + 远端部署目录。
    pub halctl: HalctlCfg,
    /// gufo 控制台（同机 SSH 控制面，仅部署目录不同；`guctl:` 节可覆盖）。
    pub guctl: HalctlCfg,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerCfg::default(),
            routing: RoutingCfg::default(),
            timeouts: TimeoutCfg::default(),
            cache: CacheCfg::default(),
            logs: LogsCfg::default(),
            params: Value::Null,
            backends: Vec::new(),
            data_dir: None,
            halctl: HalctlCfg::standard("/mnt/haloen"),
            guctl: HalctlCfg::standard("/mnt/data/gufo"),
        }
    }
}

/// halogen/gufo 远程控制配置。默认值即这台 102 机器的事实（雷电优先、局域网兜底），
/// config.yaml 的 `halctl:` / `guctl:` 节可覆盖（两者仅部署目录不同）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HalctlCfg {
    /// SSH 目标地址，按顺序尝试（雷电直连优先，局域网兜底）。
    pub hosts: Vec<String>,
    /// SSH 用户名。
    pub user: String,
    /// 102 上的部署目录（hal.sh / hal-ver.sh / halogen.env / run-halogen.sh 所在）。
    pub proj: String,
}

impl HalctlCfg {
    /// 102 机位的出厂事实：SSH 目标 + 部署目录。halctl/guctl 共用，仅 proj 不同。
    pub fn standard(proj: &str) -> Self {
        Self {
            hosts: vec!["10.0.0.102".into(), "192.168.31.168".into()],
            user: "lowkey".into(),
            proj: proj.into(),
        }
    }
}

impl Default for HalctlCfg {
    fn default() -> Self {
        Self::standard("/mnt/haloen")
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)?;
        let cfg: Config = serde_yaml::from_str(&raw)?;
        cfg.validate()?;
        Ok(cfg)
    }

    // 曾有一个 `load_or_default(path)`：解析失败就 eprintln 一句然后返回默认配置。
    // 零调用点，而且这个兜底本身就是隐患——配置写错了却「成功」启动，跑的是
    // 你以为不是的那套配置。要加载就走 `load()`，错误往上传。
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.server.port == 0 {
            return Err(ConfigError::Invalid("server.port must not be 0".into()));
        }
        // 监听面安全（P0）：非回环监听 + 未设令牌 = 把一整个能访问你所有后端的
        // 开放代理摆到网络上。FAQ 里写了「一定要设」，这里把它变成硬约束。
        if !is_loopback_host(&self.server.host) && self.server.token().is_none() {
            return Err(ConfigError::Invalid(format!(
                "server.host = '{}' requires server.access_token: otherwise every \
                 backend behind this gateway is exposed to the whole network \
                 (see docs/FAQ.md)",
                self.server.host
            )));
        }
        let mut names = std::collections::HashSet::new();
        for b in &self.backends {
            if b.name.trim().is_empty() {
                return Err(ConfigError::Invalid(
                    "backend name must not be empty".into(),
                ));
            }
            if !names.insert(b.name.clone()) {
                return Err(ConfigError::Invalid(format!(
                    "duplicate backend name `{}`",
                    b.name
                )));
            }
            if !(b.base_url.starts_with("http://") || b.base_url.starts_with("https://")) {
                return Err(ConfigError::Invalid(format!(
                    "backend {} base_url must start with http:// or https://",
                    b.name
                )));
            }
            b.resolve_protocol().map_err(ConfigError::Invalid)?;
        }
        if self.cache.response.disk_gb > 512 {
            return Err(ConfigError::Invalid(
                "cache.response.disk_gb above 512 is almost certainly a typo".into(),
            ));
        }
        Ok(())
    }

    pub fn data_dir(&self) -> PathBuf {
        self.data_dir
            .clone()
            .unwrap_or_else(|| dirs_home().join(".llm-switch"))
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir().join("llm-switch.db")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir().join("cache")
    }

    /// config.yaml 的路径（与 db_path/cache_dir 同目录）。
    pub fn yaml_path(&self) -> PathBuf {
        self.data_dir().join("config.yaml")
    }

    /// 把 backends 重建为 DB 行的镜像（enabled 一律 None = 不覆盖 DB 运行态）。
    ///
    /// 持久化的唯一落点：UI 增删改后端/模型后调用它，让 config.yaml 与 DB 一致，
    /// 这样启动时 seed_backends 就不会从陈旧的 yaml 复活被删的后端或旧模型列表。
    /// enabled 留 None 是刻意的：seed_backends 对 None 会保留 DB 运行态，
    /// 与 UI 的启用开关（只写 DB、不回写 yaml）同口径。
    pub fn mirror_from_rows(&mut self, rows: &[BackendRow]) {
        self.backends = rows.iter().map(row_to_cfg).collect();
    }

    /// 原子写 config.yaml：先写临时文件再 rename，避免写到一半崩了留半份。
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        let tmp = path.with_extension("yaml.tmp");
        {
            let mut f =
                std::fs::File::create(&tmp).map_err(|e| format!("创建临时文件失败：{e}"))?;
            f.write_all(
                serde_yaml::to_string(self)
                    .map_err(|e| format!("序列化失败：{e}"))?
                    .as_bytes(),
            )
            .map_err(|e| format!("写入临时文件失败：{e}"))?;
            f.flush().map_err(|e| format!("刷盘失败：{e}"))?;
        }
        std::fs::rename(&tmp, path).map_err(|e| format!("替换 config.yaml 失败：{e}"))?;
        Ok(())
    }
}

/// 把 DB 一行后端重建为 yaml 种子（seed_backends→to_upsert 会原样复现它）。
/// enabled 一律 None：seed_backends 对 None 保留 DB 运行态，与 UI 启用开关同口径。
fn row_to_cfg(r: &BackendRow) -> BackendCfg {
    BackendCfg {
        name: r.name.clone(),
        base_url: r.base_url.clone(),
        protocol: r.protocol.clone(),
        engine: non_empty_opt(&r.engine).unwrap_or_else(|| "custom".to_string()),
        api_key: None,
        weight: r.weight,
        enabled: None,
        models: r.models.clone(),
        aliases: value_to_alias_map(&r.aliases),
        capabilities: cap_option(&r.capabilities),
        params: r.params.clone(),
        webui_url: non_empty_opt(&r.webui_url),
        webui_user: non_empty_opt(&r.webui_user),
        webui_password: non_empty_opt(&r.webui_password),
        health_path: non_empty_opt(&r.health_path),
    }
}

/// 把 serde_json::Value 的友好名→上游 id 映射，规整成 BTreeMap（非对象 → 空）。
fn value_to_alias_map(v: &Value) -> BTreeMap<String, String> {
    v.as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// capabilities 为空对象 {} 时 seed_backends 会回退到引擎预设，保留它才有意义。
fn cap_option(v: &Value) -> Option<Value> {
    if v.is_null() || (v.is_object() && v.as_object().map(|o| o.is_empty()).unwrap_or(false)) {
        None
    } else {
        Some(v.clone())
    }
}

/// 空串 / None 都规整成 None（seed_backends 对空串视为「未指定」）。
fn non_empty_opt(v: &Option<String>) -> Option<String> {
    v.as_ref().filter(|s| !s.trim().is_empty()).cloned()
}

/// 持久化：把 DB 后端镜像回 config.yaml，使启动时 `seed_backends` 不再复活/覆盖。
///
/// 根因（实报 bug）：UI 的增删改只写 DB，config.yaml 从不更新；启动时
/// `seed_backends` 从 yaml 逐条 upsert，于是删掉的后端、改过的模型被重新灌回 DB。
/// 修复口径：DB 是真值，删后端 / 摘模型 / 改名后把当前 DB 行镜像回 yaml，让种子
/// 与真值一致，播种就只是「把 DB 原样写回」的幂等操作。
///
/// 注意：`enabled` 镜像为 `None`（不回写 yaml），与现有「启用开关只写 DB、播种按 DB
/// 运行态覆盖」的约定一致——否则关掉的后端又被 yaml 的默认 true 洗开。
pub fn persist_backends_to_yaml(config: &Config, store: &Store) -> Result<(), String> {
    let rows = store.list_backends().map_err(|e| e.to_string())?;
    let mut cfg = config.clone();
    cfg.mirror_from_rows(&rows);
    cfg.save_to(&config.yaml_path())
}

/// 是否回环监听地址：令牌未设时只允许绑在这些地址上（`Config::validate`）。
pub fn is_loopback_host(host: &str) -> bool {
    let h = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    h == "localhost" || h == "::1" || h.starts_with("127.")
}

/// 用户主目录（测试可用 HOME 变量隔离）。
pub fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_loopback_and_cache_off() {
        let cfg = Config::default();
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.server.port, 8787);
        assert!(
            !cfg.cache.response.enabled,
            "response cache must default to off"
        );
        assert!(cfg.cache.affinity.enabled, "affinity must default to on");
    }

    #[test]
    fn parses_example_shape_and_resolves_presets() {
        let yaml = r#"
server:
  port: 9000
backends:
  - name: lab-omlx
    base_url: http://127.0.0.1:8000/
    engine: omlx
    models: [qwen3-30b]
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.server.port, 9000);
        let b = &cfg.backends[0];
        assert_eq!(b.resolve_engine(), Engine::Omlx);
        assert_eq!(b.resolve_protocol().unwrap(), Protocol::OpenaiChat);
        assert_eq!(b.base_url.trim_end_matches('/'), "http://127.0.0.1:8000");
        assert_eq!(b.webui_url().as_deref(), Some("http://127.0.0.1:8000/"));
        assert!(Engine::Omlx.has_disk_kv_tier());
        let up = b.to_upsert().unwrap();
        assert_eq!(up.base_url, "http://127.0.0.1:8000");
        assert_eq!(up.weight, 1);
    }

    #[test]
    fn rejects_duplicate_names_and_bad_urls() {
        let dup = Config {
            backends: vec![
                BackendCfg {
                    name: "a".into(),
                    base_url: "http://x".into(),
                    ..Default::default()
                },
                BackendCfg {
                    name: "a".into(),
                    base_url: "http://y".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert!(dup.validate().is_err());

        let bad = Config {
            backends: vec![BackendCfg {
                name: "a".into(),
                base_url: "ftp://x".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(bad.validate().is_err());
    }

    /// 持久化回归（实报 bug）：UI 删后端 / 改模型只写 DB，config.yaml 从不更新，
    /// 启动时 seed_backends 从陈旧 yaml 逐条 upsert，于是删掉的后端、改过的模型
    /// 被重新灌回。修复：DB 行镜像回 yaml（mirror_from_rows + save_to），
    /// 重启加载后 yaml 与 DB 一致，播种不再复活。
    #[test]
    fn mirror_from_rows_survives_save_load_roundtrip() {
        use crate::types::backend_row_for_test;

        // 陈旧 yaml：两个后端，models 是旧清单。
        let stale = Config {
            backends: vec![
                BackendCfg {
                    name: "keep".into(),
                    base_url: "http://keep".into(),
                    protocol: "openai_chat".into(),
                    engine: "custom".into(),
                    models: vec!["old-a".into(), "old-b".into()],
                    ..Default::default()
                },
                BackendCfg {
                    name: "gone".into(),
                    base_url: "http://gone".into(),
                    protocol: "openai_chat".into(),
                    engine: "custom".into(),
                    models: vec!["x".into()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        // DB 现状："gone" 被删，"keep" 的模型改成 new-a/new-b（模拟 remove+rename）。
        let mut keep = backend_row_for_test("keep");
        keep.base_url = "http://keep".into();
        keep.protocol = "openai_chat".into();
        keep.engine = Some("custom".into());
        keep.models = vec!["new-a".into(), "new-b".into()];
        let rows = vec![keep];

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");

        let mut cfg = stale;
        cfg.mirror_from_rows(&rows);
        cfg.save_to(&path).unwrap();

        // 重新加载（模拟重启）：yaml 必须只剩 keep，且模型是新清单。
        let reloaded = Config::load(&path).unwrap();
        assert_eq!(reloaded.backends.len(), 1, "被删后端不得复活");
        assert_eq!(reloaded.backends[0].name, "keep");
        assert_eq!(
            reloaded.backends[0].models,
            vec!["new-a".to_string(), "new-b".to_string()],
            "改过的模型清单必须持久化"
        );
        // enabled 镜像为 None：seed_backends 对 None 保留 DB 运行态。
        assert!(
            reloaded.backends[0].enabled.is_none(),
            "enabled 不得回写 yaml（保留 DB 运行态）"
        );
    }
}
