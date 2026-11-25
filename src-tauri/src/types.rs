//! Shared domain types: protocols, backend records, usage and log rows.

use serde::{Deserialize, Serialize};

/// Wire protocol spoken by a client (entry) or an upstream backend (exit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// Anthropic Messages API (`POST /v1/messages`)
    Anthropic,
    /// OpenAI Chat Completions (`POST /v1/chat/completions`) — the common case,
    /// so it is also the default for a backend.
    #[default]
    OpenaiChat,
    /// OpenAI Responses API (`POST /v1/responses`)
    OpenaiResponses,
    /// Gemini native `generateContent` / `streamGenerateContent`
    Gemini,
    /// 云 ComfyUI worker（视频：`POST /prompt` → `/history/{job}` 轮询）。
    /// 无聊天适配（adapter_for 返回 None），只被视频任务代理消费（切片 B）。
    ComfyUI,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Anthropic => "anthropic",
            Protocol::OpenaiChat => "openai_chat",
            Protocol::OpenaiResponses => "openai_responses",
            Protocol::Gemini => "gemini",
            Protocol::ComfyUI => "comfyui",
        }
    }

    pub fn parse(s: &str) -> Option<Protocol> {
        match s {
            "anthropic" | "claude" => Some(Protocol::Anthropic),
            "openai_chat" | "openai" | "chat" => Some(Protocol::OpenaiChat),
            "openai_responses" | "responses" => Some(Protocol::OpenaiResponses),
            "gemini" | "google" => Some(Protocol::Gemini),
            "comfyui" | "comfy" => Some(Protocol::ComfyUI),
            _ => None,
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl BackendRow {
    /// 该暴露名是否对外可见（无 flag 记录 = 可见）。
    pub fn is_exposed(&self, public_name: &str) -> bool {
        match self
            .model_flags
            .iter()
            .find(|f| f.public_name == public_name)
        {
            Some(flag) => flag.exposed,
            None => true,
        }
    }

    /// 把客户端请求的模型名解析成上游模型名；不可见（未暴露或根本不存在）返回 None。
    ///
    /// 三条通路：①暴露名直接命中；②`aliases` 里的公开别名命中（等价于①，历史形态）；
    /// ③改名后的上游原名，仅当该模型打开了 `accept_upstream` 过渡开关。
    ///
    /// 「有开关行却不在 `models` 列」= 刷新摘下的游离配置（摘挂不销毁）：②③一律
    /// 不认，硬打仍是干净 404——游离只暂存配置，不服务流量；挂回列后走①照常。
    /// 老数据不变量是「行⊆列」（sync/upsert/rename 都同步两处），此判断轮不到
    /// 既有名字，只拦游离态。
    pub fn resolve_upstream(&self, requested: &str) -> Option<String> {
        if self.models.iter().any(|m| m == requested) {
            if !self.is_exposed(requested) {
                return None;
            }
            let aliased = self
                .aliases
                .get(requested)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            return Some(aliased.unwrap_or_else(|| requested.to_string()));
        }
        let has_row = self.model_flags.iter().any(|f| f.public_name == requested);
        // ② 纯别名（历史形态：别名键没写进 models 列表，也没有开关行）。
        if !has_row {
            if let Some(upstream) = self
                .aliases
                .get(requested)
                .and_then(serde_json::Value::as_str)
            {
                return Some(upstream.to_string());
            }
        }
        // ③ 改名后的上游原名：仅当该模型开着过渡开关 `accept_upstream`，且还在列里。
        if self.model_flags.iter().any(|f| {
            f.exposed
                && f.accept_upstream
                && f.upstream_name == requested
                && f.public_name != requested
                && self.models.iter().any(|m| m.as_str() == f.public_name)
        }) {
            return Some(requested.to_string());
        }
        None
    }

    /// 对外可见的模型名清单（`/v1/models` 用）。
    pub fn exposed_models(&self) -> Vec<String> {
        self.models
            .iter()
            .filter(|m| self.is_exposed(m))
            .cloned()
            .collect()
    }
}

/// Health state of a backend as tracked by the prober (M3 wires the transitions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Unknown,
    Healthy,
    Degraded,
    Down,
}

impl HealthState {
    pub fn as_str(self) -> &'static str {
        match self {
            HealthState::Unknown => "unknown",
            HealthState::Healthy => "healthy",
            HealthState::Degraded => "degraded",
            HealthState::Down => "down",
        }
    }
}

/// 单个「后端 × 暴露名」的对外开关（`backend_models` 表的投影，M12-A）。
///
/// 为什么开关挂在「后端 × 模型」而不是全局模型名：同一个上游模型可能由两台后端
/// 各自提供，用户想关的往往只是其中一台的那份。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelFlag {
    /// 对外暴露的名字（缺省等于上游名）。
    pub public_name: String,
    /// 真正发给上游的名字。
    pub upstream_name: String,
    /// 关掉 = 对客户端彻底隐身：`/v1/models` 不列、路由不选、硬打 404。
    pub exposed: bool,
    /// 改名后是否**同时**接受上游原名（客户端配置一时改不过来的过渡期用）。
    #[serde(default)]
    pub accept_upstream: bool,
    /// 工种人工声明列（11a 遗留）：**用户 2026-11-23 拍板取消人工覆盖后不再参与判定**，
    /// 请求期一律走 `gateway::model_kind::request_kind` 的名字启发式；列保留只作历史数据。
    /// 媒体工种永不进聊天择路与分配格子的口径不变。
    #[serde(default)]
    pub kind: Option<String>,
}

/// 扫描得到的单个模型：上游顺带给出的 ctx/output/reasoning/vlm 真值
/// （None = 没给，不猜）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScannedModel {
    pub id: String,
    pub ctx: Option<i64>,
    pub output: Option<i64>,
    /// 是否思考模型（上游明说才有值；没给 = None）。
    #[serde(default)]
    pub reasoning: Option<bool>,
    /// 是否有视觉输入（VLM；上游明说才有值；没给 = None）。
    #[serde(default)]
    pub vlm: Option<bool>,
}

/// 模型元数据一个字段的来源（分级：人工声明 > 后端设定（参数/探测）> 未知不写）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaSource {
    /// `model_meta` 里的人工非 0 声明（超后端上限时已被钳到边界）。
    Manual,
    /// 自动：后端层参数 → 扫描探测值（跨后端取最小）。
    Backend,
    /// 两边都没有：未知，消费端不写这个字段。
    #[default]
    Unknown,
}

/// 一个模型的元数据解析结果（`/v1/models`、UI、接管写入共用一份口径）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct EffectiveMeta {
    /// 有效上下文（人工值已按后端允许范围钳过；None = 未知不写）。
    pub ctx: Option<i64>,
    pub output: Option<i64>,
    pub ctx_source: MetaSource,
    pub output_source: MetaSource,
    /// 是否思考模型（None = 未知不写）。
    pub reasoning: Option<bool>,
    pub reasoning_source: MetaSource,
    /// 是否有视觉输入（VLM；None = 未知不写，网关按 Unknown 放行）。
    pub vlm: Option<bool>,
    pub vlm_source: MetaSource,
    /// 人工声明的原始值（0 = 自动）。
    pub manual_ctx: i64,
    pub manual_output: i64,
    /// 人工三态声明：0 = 自动（跟随探测），1 = 强制开，2 = 强制关。
    pub manual_reasoning: i64,
    pub manual_vlm: i64,
    /// 后端允许范围上限（None = 后端侧无从得知，不拦）。
    pub cap_ctx: Option<i64>,
    pub cap_output: Option<i64>,
}

/// One row of the `backends` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendRow {
    pub id: i64,
    pub name: String,
    pub base_url: String,
    pub protocol: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub weight: i64,
    pub enabled: bool,
    /// 直连模式（计划 2026-11-18-backend-direct-mode）：true = 已接管的代理直连本后端
    /// 真实地址，流量不经网关。与 enabled=false（停用）正交，三态 = 停用/网关/直连。
    #[serde(default)]
    pub direct: bool,
    /// 直连 Key 环境变量名（派生规则唯一入路 `takeover::Endpoint::direct_key_var`，
    /// 前端确认框/toast 只回显，不再各自拼字符串）。
    #[serde(default)]
    pub key_var: String,
    pub engine: Option<String>,
    /// 互斥配对引擎名（可空 = 无配对）。当前生效引擎就是 `engine` 本身，
    /// 切换 = 两列互换（见 `commands::halctl::halctl_use_engine`）。只存 DB 不进 yaml。
    #[serde(default)]
    pub alt_engine: Option<String>,
    pub webui_url: Option<String>,
    /// Console login (both optional): user+pass -> Basic, pass only -> Bearer.
    pub webui_user: Option<String>,
    pub webui_password: Option<String>,
    /// 本机启停脚本路径（可空 = 该后端不提供启停；执行见 `services::script`）。
    /// 与上面的控制台入口三元组同属控制台页「入口设置」卡。
    pub start_script: Option<String>,
    pub stop_script: Option<String>,
    pub health_path: Option<String>,
    /// JSON array of model names served by this backend.
    pub models: Vec<String>,
    /// JSON object mapping public alias -> upstream model name.
    pub aliases: serde_json::Value,
    /// 每个暴露名的开关；**没有记录 = 暴露且未改名**（老数据零迁移负担）。
    #[serde(default)]
    pub model_flags: Vec<ModelFlag>,
    /// Engine capability descriptor (see DEVELOPMENT-PLAN §5.1).
    pub capabilities: serde_json::Value,
    /// Per-backend parameter defaults (see DEVELOPMENT-PLAN §5.3).
    pub params: serde_json::Value,
    pub health_state: String,
    pub consecutive_failures: i64,
    pub last_checked_at: Option<i64>,
    pub last_error: Option<String>,
}

/// 一份可用的 `BackendRow`，给单测构造既存行用（测试想覆盖哪列就覆哪列）。
#[cfg(test)]
pub fn backend_row_for_test(name: &str) -> BackendRow {
    BackendRow {
        id: 1,
        name: name.to_string(),
        base_url: "http://x".into(),
        protocol: "openai_chat".into(),
        api_key: None,
        weight: 1,
        enabled: true,
        direct: false,
        key_var: String::new(),
        engine: None,
        alt_engine: None,
        webui_url: None,
        webui_user: None,
        webui_password: None,
        start_script: None,
        stop_script: None,
        health_path: None,
        models: vec![],
        aliases: serde_json::json!({}),
        model_flags: Vec::new(),
        capabilities: serde_json::Value::Null,
        params: serde_json::Value::Null,
        health_state: "unknown".into(),
        consecutive_failures: 0,
        last_checked_at: None,
        last_error: None,
    }
}

/// Input for creating/updating a backend (used by config seeding and IPC).
#[derive(Debug, Clone, Default)]
pub struct BackendUpsert {
    pub name: String,
    pub base_url: String,
    pub protocol: Protocol,
    pub api_key: Option<String>,
    pub weight: i64,
    pub enabled: bool,
    pub engine: Option<String>,
    pub webui_url: Option<String>,
    pub webui_user: Option<String>,
    pub webui_password: Option<String>,
    pub health_path: Option<String>,
    pub models: Vec<String>,
    pub aliases: serde_json::Value,
    pub capabilities: serde_json::Value,
    pub params: serde_json::Value,
}

/// Token/latency accounting for a single proxied request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageRecord {
    pub request_id: String,
    pub ts: i64,
    pub backend: Option<String>,
    pub model: Option<String>,
    pub entry_protocol: Option<String>,
    pub client: Option<String>,
    pub status: i64,
    pub ok: bool,
    pub stream: bool,
    pub ttft_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub decode_tps: Option<f64>,
    /// Tokens came from an estimate rather than upstream usage.
    pub estimated: bool,
    /// `hit` | `miss` | `bypass:<reason>` | `affinity:<backend>`
    pub cache_status: Option<String>,
    pub retry_trace: Option<String>,
    pub error: Option<String>,
    /// 媒体计量（11c）：生成图像张数；None = 取不到（UI 显示 —，不造假数字）。
    pub media_images: Option<i64>,
    /// 媒体计量（11c）：音频秒数（转写 duration 等）；None = 取不到（—）。
    pub media_seconds: Option<f64>,
}

/// Aggregated numbers over a time window (UTC day granularity handled in SQL).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    pub requests: i64,
    pub ok_requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub avg_ttft_ms: Option<f64>,
    pub avg_decode_tps: Option<f64>,
}

/// 媒体计量聚合行（11d 用量页「媒体计量」卡）：张数/秒数 NULL = 取不到（显示 —）。
#[derive(Debug, Clone, Serialize)]
pub struct MediaUsageRow {
    pub model: Option<String>,
    pub backend: Option<String>,
    pub requests: i64,
    pub images: Option<i64>,
    pub seconds: Option<f64>,
}

/// Filter for querying the structured log table.
#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub level: Option<String>,
    pub backend: Option<String>,
    pub model: Option<String>,
    pub request_id: Option<String>,
    /// Substring match against message + code.
    pub text: Option<String>,
    pub only_errors: bool,
    pub limit: i64,
    pub offset: i64,
}

/// 用量明细过滤（M4-D）。
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UsageRecordQuery {
    pub backend: Option<String>,
    pub model: Option<String>,
    pub entry_protocol: Option<String>,
    pub request_id: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    #[serde(default)]
    pub only_errors: bool,
    pub limit: i64,
    pub offset: i64,
}

/// 日粒度用量聚合（M4-B，usage_daily 跨 backend/model 汇总行）。
#[derive(Debug, Clone, Serialize)]
pub struct DailyRollup {
    pub day: String,
    pub requests: i64,
    pub errors: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub avg_ttft_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRow {
    pub id: i64,
    pub ts: i64,
    pub level: String,
    pub code: Option<String>,
    pub message: String,
    pub request_id: Option<String>,
    pub backend: Option<String>,
    pub model: Option<String>,
    pub extra: serde_json::Value,
}

/// Unix time in milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

/// One row of the layered parameter override table (M2 参数中心).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParamOverride {
    pub id: i64,
    /// `global` | `backend` | `model`
    pub scope: String,
    /// `''` for global, backend name for backend, `backend:model` for model.
    pub ref_id: String,
    pub key: String,
    pub value: serde_json::Value,
    /// `fill` | `override` | `clamp`
    pub policy: String,
}

/// One row of the `prompt_items` table（提示词速取面板，计划 2026-11-18）。
/// `tab` = `"common"`（通用库）或某代理的 `ClientApp::as_str()`。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PromptItem {
    pub id: i64,
    pub tab: String,
    pub content: String,
    /// tab 内手工顺序，新条目追加末尾。
    pub sort: i64,
    /// 取用次数（V14）：只用于排序，高频条目排前面。
    pub use_count: i64,
    /// 最后一次取用（毫秒）：同次数时新近的排前面。
    pub last_used_at: i64,
}

/// One row of the `skills` table (M6：SSOT 目录名即主键）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRecord {
    pub dir: String,
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub version: Option<String>,
    pub latest: Option<String>,
    pub size: i64,
    /// 1 = 经本工具安装；0 = 扫描到的本地/外部摆放目录。
    pub managed: bool,
    pub updated_at: i64,
}

/// One row of the `skill_apps` 启用开关表。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAppRow {
    pub skill_dir: String,
    pub app_id: String,
    pub enabled: bool,
    /// 实际采用的分发方式："symlink" | "copy"。
    pub sync_mode: String,
}

/// One row of the `skill_repos` 仓库清单表（M6-B）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRepoRow {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub branch: String,
    pub enabled: bool,
}
