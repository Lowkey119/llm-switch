//! M9 客户端配置接管（takeover）：把客户端的「连接键」改写到本地网关，附备份与回滚。
//!
//! - **只写自己拥有的连接键**（地址 + 凭据，个别客户端外加「选我这个 provider」的开关键），
//!   其余键、注释一概不动用户的资产；
//! - **写前整文件快照**到 `<data_dir>/takeover-backups/<app>-<毫秒>/`，回滚按备份逐字节还原；
//!   原本不存在的文件记 `existed=false`，回滚时删掉我们创建的文件；
//! - **原件只备一份**：备份发生在「进入接管态」的那一刻（接管前的原件），
//!   接管态内的重写（切模型 / resync / 手工同步）不再新增备份；回滚取
//!   `State.backup_id` 记的那份，老数据没记账才退回取该端最旧的一份；
//! - **解析不了的用户文件绝不覆盖**（宁可报错让用户先修），空文件同样拒绝；
//! - 幂等：接管态已等于目标态时不写盘、不产生备份（有差异时才写，但
//!   也不因此再备一份，见上一条「原件只备一份」）。
//!
//! 已知取舍：JSON 文件重写按 serde_json 默认的字母序重排键（不开 `preserve_order`，
//! 免得影响缓存键的规范序列化），内容不丢、键顺序可能变；备份/回滚是逐字节的，故可接受。

pub mod claude;
pub mod claude_switches;
pub mod codebuddy;
pub mod codex;
pub mod editor;
pub mod gemini;
pub mod matrix;
pub mod opencode;
pub mod pi;
pub mod preset;

use crate::config::Config;
use crate::store::Store;
use crate::types::now_ms;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 写进各客户端配置里的 provider 标识（用户在自家配置里能认出我们写了什么）。
pub const PROVIDER_ID: &str = "llm-switch";
/// 网关未设 `server.access_token` 时写入的占位凭据（各客户端都要求「必须有 key」）。
pub const PLACEHOLDER_TOKEN: &str = "llm-switch-local";
/// Codex wire_api 的可配键（settings 表）：`chat` | `responses`。
pub const CODEX_WIRE_API_SETTING: &str = "takeover.codex.wire_api";
/// 每端的模型选择（默认档 + 档位表）存在这张 settings 前缀下。
pub const MODEL_SETTING_PREFIX: &str = "takeover.model.";

/// 被接管的客户端。有配置文件可接管的七端与 Skills 各端一致（D18 起，D24 加
/// codebuddy）；copilot 是网关自己的 Ollama 门面下游（真值在 settings，没有
/// 外部配置文件），只在模型矩阵里出现——见 [`ClientApp::FILE_CLIENTS`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientApp {
    Claude,
    Codex,
    OpenCode,
    DevEco,
    Gemini,
    Pi,
    CodeBuddy,
    /// 网关的 Ollama 门面下游（非文件客户端）：可见清单存 settings，
    /// 没有可评估/接管的配置文件，代理管理与技能矩阵都不列它。
    Copilot,
}

impl ClientApp {
    /// 有配置文件、可接管的真客户端（= Skills 各端）。代理管理、技能矩阵、
    /// 接管评估/回滚这些「文件客户端」的遍历一律走这份，不掺 copilot 门面。
    pub const FILE_CLIENTS: [ClientApp; 7] = [
        ClientApp::Claude,
        ClientApp::Codex,
        ClientApp::OpenCode,
        ClientApp::DevEco,
        ClientApp::Gemini,
        ClientApp::Pi,
        ClientApp::CodeBuddy,
    ];

    /// 全部消费端（文件七端 + copilot 门面）。模型矩阵按这份下发列。
    pub const ALL: [ClientApp; 8] = [
        ClientApp::Claude,
        ClientApp::Codex,
        ClientApp::OpenCode,
        ClientApp::DevEco,
        ClientApp::Gemini,
        ClientApp::Pi,
        ClientApp::CodeBuddy,
        ClientApp::Copilot,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ClientApp::Claude => "claude",
            ClientApp::Codex => "codex",
            ClientApp::OpenCode => "opencode",
            ClientApp::DevEco => "deveco",
            ClientApp::Gemini => "gemini",
            ClientApp::Pi => "pi",
            ClientApp::CodeBuddy => "codebuddy",
            ClientApp::Copilot => "copilot",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ClientApp::Claude => "Claude Code",
            ClientApp::Codex => "Codex CLI",
            ClientApp::OpenCode => "OpenCode",
            ClientApp::DevEco => "DevEco Code",
            ClientApp::Gemini => "Gemini CLI",
            ClientApp::Pi => "Pi",
            ClientApp::CodeBuddy => "CodeBuddy",
            // 标签取中性名：这一枚在代码里同时指两件事——网关的 Ollama 形状门面
            // （可见清单存 settings）与你机器上装的 GitHub Copilot CLI（~/.copilot，
            // 有 skills、有 plugin）。写死 "(Ollama)" 会让技能那一列说不通。
            ClientApp::Copilot => "Copilot",
        }
    }

    pub fn parse(raw: &str) -> Result<ClientApp, String> {
        ClientApp::ALL
            .into_iter()
            .find(|a| a.as_str() == raw.trim().to_ascii_lowercase())
            .ok_or_else(|| format!("未知的客户端: {raw}"))
    }

    fn state_key(self) -> String {
        format!("takeover.state.{}", self.as_str())
    }
}

/// 接管动作的工作目录。测试必须用 [`TakeoverEnv::with_roots`] 指临时目录，
/// 否则会动真实的 `~/.claude` 等目录（与 `SkillEnv::with_roots` 同一条铁律）。
#[derive(Debug, Clone)]
pub struct TakeoverEnv {
    pub home: PathBuf,
    pub backups: PathBuf,
    /// 配置文件路径自定义（内置默认路径 → 自定义路径），构造时从 settings 装载。
    /// 各端的 `*_path()` 一律经 [`TakeoverEnv::file`] 收口，读、写、编辑器天然同侧。
    overrides: std::collections::BTreeMap<String, String>,
    /// true = 忽略进程环境变量覆盖（`with_roots` 测试隔离专用）。
    /// 不隔离时 `dir()` 会优先读 `PI_CODING_AGENT_DIR` 等真实环境变量，
    /// 开发 shell 里一设这些变量，测试就穿透 tmp 读写真实目录（门禁红三片的真因）。
    isolate: bool,
}

impl TakeoverEnv {
    pub fn from_config(config: &Config, store: &Store) -> TakeoverEnv {
        let mut overrides = std::collections::BTreeMap::new();
        for app in ClientApp::ALL {
            overrides.extend(store.path_overrides(app.as_str()));
        }
        TakeoverEnv {
            home: crate::config::dirs_home(),
            backups: config.data_dir().join("takeover-backups"),
            overrides,
            isolate: false,
        }
    }

    /// 测试专用：根目录全部显式给定，绝不碰真实目录。
    pub fn with_roots(home: PathBuf, backups: PathBuf) -> TakeoverEnv {
        TakeoverEnv {
            home,
            backups,
            overrides: Default::default(),
            isolate: true,
        }
    }

    /// 剥掉路径自定义，得到纯内置路径（用于校验「默认路径」是否在白名单里）。
    pub fn plain(&self) -> TakeoverEnv {
        TakeoverEnv {
            home: self.home.clone(),
            backups: self.backups.clone(),
            overrides: Default::default(),
            isolate: self.isolate,
        }
    }

    /// 路径自定义的唯一咽喉：默认路径 → 自定义路径（无覆盖原样返回）。
    pub fn file(&self, default: impl AsRef<Path>) -> PathBuf {
        let d = default.as_ref();
        match self.overrides.get(&d.display().to_string()) {
            Some(custom) => PathBuf::from(custom),
            None => d.to_path_buf(),
        }
    }

    /// 反查生效路径来自哪条默认（身份键）；无覆盖返回 `None`。
    pub fn default_of(&self, effective: &Path) -> Option<String> {
        let eff = effective.display().to_string();
        self.overrides
            .iter()
            .find(|(_, custom)| **custom == eff)
            .map(|(default, _)| default.clone())
    }

    pub fn ensure_dirs(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.backups)
            .map_err(|e| format!("创建接管备份目录失败 {}: {e}", self.backups.display()))
    }

    /// 目录解析：环境变量覆盖（绝对路径）优先，否则 home 下的默认相对目录。
    /// 与各客户端自身的解析口径一致（`CLAUDE_CONFIG_DIR` / `CODEX_HOME` /
    /// `XDG_CONFIG_HOME` / `PI_CODING_AGENT_DIR`）。
    pub fn dir(&self, var: &str, default_rel: &str) -> PathBuf {
        if self.isolate {
            return self.home.join(default_rel);
        }
        match std::env::var_os(var) {
            Some(v) if !v.is_empty() => {
                let p = PathBuf::from(v);
                if p.is_absolute() {
                    p
                } else {
                    self.home.join(p)
                }
            }
            _ => self.home.join(default_rel),
        }
    }
}

/// 一次接管要下发的目标端点。
#[derive(Debug, Clone)]
pub struct Endpoint {
    /// 网关根地址（不带 `/v1`）：Claude / Gemini 用它。
    pub base_root: String,
    /// OpenAI 兼容根地址（带 `/v1`）：Codex / OpenCode / Pi 用它。
    pub api_root: String,
    pub token: String,
    /// 网关当前可服务的模型名（Pi / OpenCode 的 provider 需要模型清单）。
    pub models: Vec<String>,
    /// 各模型的 ctx/output 元数据（接管写入用；未知的模型不在表里 = 不写该字段）。
    pub meta: std::collections::BTreeMap<String, crate::types::EffectiveMeta>,
    /// Codex 的 wire_api（`chat` | `responses`）。
    pub codex_wire_api: String,
    /// 直连伪端点标记：为真时 `models` 只是这一个后端的模型，
    /// 写盘前槽位一律钳进这份清单（配置文件只连一个 URL，别家模型必 503）。
    pub direct: bool,
}

impl Endpoint {
    /// 接管配置里写的网关根（与 `build` 同一套主机名归一，口径只留一处）。
    pub fn base_root(host: &str, port: u16) -> String {
        let h = match host {
            "" | "0.0.0.0" | "::" | "*" => "127.0.0.1",
            other => other,
        };
        format!("http://{h}:{port}")
    }

    /// 直连模式的 Key 环境变量名（计划 2026-11-18-backend-direct-mode 决策 3）：
    /// `LLM_SWITCH_<后端名大写、非字母数字转下划线>_KEY`。程序绝不把真实 Key
    /// 写进客户端配置文件，Key 位只写 `${VAR}` 引用，缺 Key 由用户 export。
    pub fn direct_key_var(backend_name: &str) -> String {
        let body: String = backend_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect();
        format!("LLM_SWITCH_{body}_KEY")
    }

    /// 从配置与库构造目标端点。监听在通配地址时，写回配置用回环。
    pub fn build(config: &Config, store: &Store) -> Endpoint {
        let base_root = Endpoint::base_root(&config.server.host, config.server.port);
        Endpoint {
            token: config
                .server
                .access_token
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| PLACEHOLDER_TOKEN.to_string()),
            api_root: format!("{base_root}/v1"),
            base_root: base_root.clone(),
            models: served_models(store),
            meta: store.effective_model_meta().unwrap_or_default(),
            codex_wire_api: store
                .get_setting(CODEX_WIRE_API_SETTING)
                .ok()
                .flatten()
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| v == "responses")
                .unwrap_or_else(|| "chat".to_string()),
            direct: false,
        }
    }
}

/// 网关当前**对外暴露**的模型名（启用后端 ∩ 模型暴露开关，字典序去重）。
///
/// 暴露开关一定要在这里生效：这份清单会被写进 opencode / pi 的配置文件，
/// 关掉的模型还留在客户端列表里，用户点了就是一个 404。
///
/// 口径 = `exposed_models()` = **列 ∩ 开关**：刷新摘下的游离行不在列里，
/// 一律不进客户端清单（游离只暂存配置，不服务流量）；老数据无开关行时
/// `is_exposed` 默认真，退回整列，与旧分支等价。
pub fn served_models(store: &Store) -> Vec<String> {
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    if let Ok(rows) = store.list_backends() {
        for b in rows {
            if !b.enabled {
                continue;
            }
            seen.extend(
                b.exposed_models()
                    .into_iter()
                    .filter(|m| !m.trim().is_empty()),
            );
        }
    }
    seen.into_iter().collect()
}

/// 每个暴露模型来自哪些启用后端（model → 后端名列表，均字典序）。
///
/// 与 `served_models` 完全同口径（enabled × exposed_models()），矩阵行标注与
/// 接管下拉分组共用这一份，杜绝两处反推分叉。
pub fn backends_of_models(store: &Store) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut backends_of: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    if let Ok(list) = store.list_backends() {
        for b in list {
            if !b.enabled {
                continue;
            }
            for m in b.exposed_models() {
                if m.trim().is_empty() {
                    continue;
                }
                let slot = backends_of.entry(m).or_default();
                if !slot.contains(&b.name) {
                    slot.push(b.name.clone());
                }
            }
        }
    }
    backends_of
}

/// 接管页模型下拉的后端分组：独占模型归自家后端组，多后端共享的模型
/// 归 `a + b` 组合组（每个模型恰好出现一次，optgroup 值不歧义）。
/// 返回 [(组名, 模型升序)]，组名字典序。
pub fn option_groups(store: &Store) -> Vec<(String, Vec<String>)> {
    let mut groups: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (model, bs) in backends_of_models(store) {
        groups.entry(bs.join(" + ")).or_default().push(model);
    }
    groups.into_iter().collect()
}

/// `option_groups` 的 JSON 形状（前端 `optgroup` 直接用）。
///
/// 接管页 `model_view` 与全站 `model_groups` 命令共用这一份拼装，两处不许各写一遍。
pub fn option_groups_json(store: &Store) -> Vec<Value> {
    option_groups(store)
        .into_iter()
        .map(|(backend, models)| json!({ "backend": backend, "models": models }))
        .collect()
}

/// 配置文件里的模型列表 = 已分配给这个端的模型 ∩ 对外暴露清单。
///
/// 顺序沿用暴露清单（字典序去重）：这样同样的输入永远渲染出同样的字节，
/// 幂等写盘不会因为集合迭代顺序抖动。**这就是「接管后清单里只该有已分配
/// 模型」的收口点**——各端的 evaluate 一律走这里，别再拿全量 `ep.models`
/// 或 `visible` 全集去写（那会把没分配给这个端的模型也塞进去）。
pub(crate) fn listed_models(
    assigned: &std::collections::BTreeSet<String>,
    exposed: &[String],
) -> Vec<String> {
    exposed
        .iter()
        .filter(|m| assigned.contains(*m))
        .cloned()
        .collect()
}

/// copilot（Ollama 门面下游）的可见清单怎么改。
///
/// copilot 没有我们能写的配置文件：真值在 settings 的 `copilot.allow`，门面
/// `/api/tags` 按它过滤。归一规则与门面读取同源：恰好 = 暴露全集时存回
/// `null`（缺省全暴露），省得白写一长串。矩阵格子与面板命令共用这一支。
pub fn copilot_assign(store: &Store, model: &str, add: bool) -> Result<Value, String> {
    let exposed = served_models(store);
    if !exposed.iter().any(|m| m == model) {
        return Err(format!("模型 {model} 未暴露，无法分配"));
    }
    let mut next: std::collections::BTreeSet<String> = store
        .copilot_allow()
        .map(std::collections::BTreeSet::from_iter)
        .unwrap_or_else(|| exposed.iter().cloned().collect());
    if add {
        next.insert(model.to_string());
    } else if !next.remove(model) {
        return Err(format!("copilot 本来就用不到 {model}"));
    }
    if next.is_empty() {
        return Err("copilot 至少要留一个可用模型".into());
    }
    // 恰好等于暴露全集 → 归一成缺省（null），与门面的零配置默认态同一个口径。
    let is_all = next.len() == exposed.len() && exposed.iter().all(|m| next.contains(m));
    let stored = if is_all {
        "null".to_string()
    } else {
        serde_json::to_string(&Vec::from_iter(next.iter().cloned())).map_err(|e| e.to_string())?
    };
    store
        .set_setting(crate::store::COPILOT_ALLOW_KEY, &stored)
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "allow": if is_all {
            Value::Null
        } else {
            Value::Array(next.iter().map(|s| Value::String(s.clone())).collect())
        }
    }))
}

/// 某客户端的模型选择：默认档 + 档位表（claude 才有档位）。
///
/// 存的是**期望值**；文件里的实际值是另一个口径（见 [`current_selection`]），
/// 两者不一致就说明有人在客户端那边动过，界面上要看得出来。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Selection {
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub tiers: std::collections::BTreeMap<String, String>,
    /// 是否由我们来托管档位表。只有矩阵/切模型明确动过手才置真——
    #[serde(default)]
    pub manage_tiers: bool,
    /// 该端的可用清单（暴露集合的子集）。`None` = 全暴露，零配置默认态。
    #[serde(default)]
    pub allow: Option<std::collections::BTreeSet<String>>,
    /// Pi 专属：主模型 / 小模型槽（存上游模型**全名**）。llm-switch 为 pi 完成大小模型
    /// 配置的落点——最终写进 pi 的 `models.json` provider 节点 `piSlots`，供 pi 自己的
    /// virtual-model 路由器读取（high→主模型，low→小模型）。
    #[serde(default)]
    pub pi_main: Option<String>,
    #[serde(default)]
    pub pi_small: Option<String>,
    /// 是否由我们托管 pi 的槽位。只有矩阵的 pi 格子动过手才置真——
    /// 否则面板/REST 只改默认档的一次写入会把槽位洗掉（同 `manage_tiers` 的理由）。
    #[serde(default)]
    pub manage_pi: bool,
    /// Codex 专属：子代理模型槽（存模型全名）。主模型就是 `default` 档（顶层
    /// `model`），副槽写进 `[agents] default_subagent_model`——codex 自主 spawn
    /// 的子代理用它（官方 subagents 文档；子代理继承父会话的 provider，
    /// 只换模型名，正好绕开 provider 覆盖被丢弃的历史 bug）。
    #[serde(default)]
    pub codex_sub: Option<String>,
    /// 是否由我们托管 codex 的副槽。只有矩阵的 codex 格子动过手才置真——
    /// 否则面板/REST 只改默认档的一次写入会把副槽洗掉（同 `manage_pi` 的理由）。
    #[serde(default)]
    pub manage_codex: bool,
}

impl Selection {
    /// 该端实际能看到的模型：全局暴露 ∩ 每端清单（清单缺省即全暴露）。
    pub fn visible(&self, exposed: &[String]) -> Vec<String> {
        match &self.allow {
            None => exposed.to_vec(),
            Some(allow) => exposed
                .iter()
                .filter(|m| allow.contains(*m))
                .cloned()
                .collect(),
        }
    }

    /// 已被这个端「占住」的模型：默认档 + 档位表 + pi 主/小槽 + codex 副槽。
    ///
    /// 这是槽位口径的分配——pi / codex / codebuddy 这几端的配置文件里没有
    /// 独立的清单概念，模型列表就该由占位决定（见 `listed_models`）。
    pub fn slot_models(&self) -> std::collections::BTreeSet<String> {
        let mut out = std::collections::BTreeSet::new();
        if let Some(d) = &self.default {
            out.insert(d.clone());
        }
        for v in self.tiers.values() {
            out.insert(v.clone());
        }
        for v in [&self.pi_main, &self.pi_small, &self.codex_sub]
            .into_iter()
            .flatten()
        {
            out.insert(v.clone());
        }
        out
    }

    /// 分配给这个端的模型 = 占位口径 ∪ 显式清单（`allow`）。
    ///
    /// 清单制端（opencode / DevEco）的分配方式就是那份清单，所以要并进来；
    /// 档位制端读不到清单（文件里没有），并进来也还是占位那几个。
    pub fn assigned_models(&self) -> std::collections::BTreeSet<String> {
        let mut out = self.slot_models();
        if let Some(allow) = &self.allow {
            out.extend(allow.iter().cloned());
        }
        out
    }

    /// Pi 两个固定槽位的键（顺序固定：主模型在前）。矩阵/面板据此画两个格。
    pub const PI_SLOT_KEYS: [&'static str; 2] = ["main", "small"];

    /// Pi 已占的槽：`[(main, 全名?), (small, 全名?)]`，未设为 `None`。
    pub fn pi_slots(&self) -> [(&'static str, Option<&str>); 2] {
        [
            ("main", self.pi_main.as_deref()),
            ("small", self.pi_small.as_deref()),
        ]
    }

    /// Codex 两个固定槽位的键（顺序固定：主模型在前）。矩阵/面板据此画两个格。
    pub const CODEX_SLOT_KEYS: [&'static str; 2] = ["main", "sub"];

    /// Codex 已占的槽：`[(main, 全名?), (sub, 全名?)]`，未设为 `None`。
    /// 主模型就是 `default` 档（写顶层 `model`），副槽是 `codex_sub`。
    pub fn codex_slots(&self) -> [(&'static str, Option<&str>); 2] {
        [
            ("main", self.default.as_deref()),
            ("sub", self.codex_sub.as_deref()),
        ]
    }
}

impl Selection {
    fn setting_key(app: ClientApp) -> String {
        format!("{MODEL_SETTING_PREFIX}{}", app.as_str())
    }

    pub fn load(store: &Store, app: ClientApp) -> Selection {
        store
            .get_setting(&Self::setting_key(app))
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, store: &Store, app: ClientApp) -> Result<(), String> {
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        store
            .set_setting(&Self::setting_key(app), &text)
            .map_err(|e| e.to_string())
    }

    pub fn is_empty(&self) -> bool {
        self.default.is_none()
            && self.tiers.is_empty()
            && self.pi_main.is_none()
            && self.pi_small.is_none()
            && self.codex_sub.is_none()
    }

    /// 「这个端现在一个模型都没分到」——自动取消接管的判据（用户 2026-11-17）。
    ///
    /// 与 `is_empty` 的区别：`is_empty` 只看占位（默认档 / 档位 / pi・codex 专属槽），
    /// 而清单制端（opencode / DevEco）的分配就是那份显式清单。注意
    /// `allow: None` 是「没特别限定 = 全暴露可见」，**不是**空分配；只有
    /// `Some(空集)` 才是一个都没分。档位制端读不到清单（`allow` 恒 None），
    /// 所以它只看占位那几个。
    pub fn is_unassigned(&self) -> bool {
        self.slot_models().is_empty() && self.allow.as_ref().is_none_or(|a| a.is_empty())
    }
}

/// 这个 URL 指的是**本地/自建端点**吗（回环、私网、`.local`/`.internal`）？
///
/// 接管语义要用它决定「哪些 provider 该隐藏」：指向本地端点的 provider 与网关
/// 是同一类东西（自建推理入口），接管后留着它们，客户端清单就还是花的；
/// 在线 API（公网域名）是用户自己的账号能力，一律不隐藏、不删除。
/// 判定只看主机名，不看路径——各家 base_url 的路径形状千奇百怪（`/v1`、`/api`…）。
pub fn is_local_endpoint(url: &str) -> bool {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("");
    let host = match authority.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    let host = host.trim_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return false;
    }
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        Ok(std::net::IpAddr::V6(v6)) => {
            let text = v6.to_string();
            v6.is_loopback()
                || v6.is_unspecified()
                || text.starts_with("fe80")
                || text.starts_with("fc")
                || text.starts_with("fd")
        }
        // 公网域名 = 在线 API
        Err(_) => false,
    }
}

/// 给模型 id 一个可读的短标签：剥掉尾部量化/格式后缀（`-mlx-8bit`、`-oq6e-mtp`…），
/// 仍过长再截断。矩阵与面板用它把长 model id 显示成槽位名（有意义的模型前缀）。
pub fn model_prefix(id: &str) -> String {
    /// 尾部段白名单：匹配即整段剥去，循环到不再匹配（多段后缀逐层剥）。
    const TAIL_TAGS: [&str; 13] = [
        "8bit", "mtp", "oq6e", "oq4", "oq6", "mlx", "a3b", "apex", "fp16", "next", "oq", "bf16",
        "int4",
    ];
    const MAX_LEN: usize = 16;
    let mut cur = id.trim().to_string();
    loop {
        match cur.rfind('-') {
            Some(pos) if TAIL_TAGS.contains(&cur[pos + 1..].to_ascii_lowercase().as_str()) => {
                cur.truncate(pos);
            }
            _ => break,
        }
    }
    if cur.chars().count() > MAX_LEN {
        let end = cur
            .char_indices()
            .nth(MAX_LEN)
            .map(|(i, _)| i)
            .unwrap_or(cur.len());
        format!("{}…", &cur[..end])
    } else {
        cur
    }
}

// ============================== 评测 ==============================

/// 一个文件的格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Toml,
    Text,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Json => "json",
            Format::Toml => "toml",
            Format::Text => "text",
        }
    }
}

/// 单个文件的现状。
#[derive(Debug, Clone, Default, Serialize)]
pub struct FileEval {
    pub path: String,
    pub format: &'static str,
    /// 磁盘上是否存在。
    pub exists: bool,
    /// 我们拥有的键是否已全部等于目标值（接管态）。
    pub managed: bool,
    /// 人话补充（例如「文件不存在，接管时会创建」）。
    pub note: String,
    /// 内置默认路径（路径自定义的身份键）；`overridden` 为真时与 `path` 不同。
    /// 由 `evaluate_one` 统一回填，各端不必关心。
    pub default_path: String,
    pub overridden: bool,
}

/// 一次待写入：已渲染好的完整新内容。
#[derive(Debug, Clone)]
pub struct Edit {
    pub path: PathBuf,
    pub content: String,
}

/// 一个客户端的评测结果：现状 + 待写差异。
#[derive(Debug, Default)]
pub struct Evaluation {
    pub files: Vec<FileEval>,
    pub edits: Vec<Edit>,
}

impl Evaluation {
    /// 接管态：至少有一个文件，且我们拥有的键全部到位。
    pub fn managed(&self) -> bool {
        !self.files.is_empty() && self.files.iter().all(|f| f.managed)
    }
}

/// 直连伪端点的槽位钳制：配置文件只连一个 URL，槽位指向别家模型必 503，
/// 所以写盘前把 default / 档位 / pi 主小 / codex 副槽全部钳进本后端模型清单，
/// 不在清单里的一律换成清单首个模型。库里存的 Selection 不动——离开直连
/// 写回网关时按原槽位恢复。清单为空（后端没报模型）时无所依从，原样返回。
pub(crate) fn clamp_to_direct(sel: &Selection, models: &[String]) -> Selection {
    let Some(first) = models.first() else {
        return sel.clone();
    };
    let hit = |m: &str| models.iter().any(|x| x == m);
    let fix = |v: &mut Option<String>| {
        if v.as_deref().is_some_and(|m| !hit(m)) {
            *v = Some(first.clone());
        }
    };
    let mut out = sel.clone();
    fix(&mut out.default);
    for v in out.tiers.values_mut() {
        if !hit(v) {
            *v = first.clone();
        }
    }
    fix(&mut out.pi_main);
    fix(&mut out.pi_small);
    fix(&mut out.codex_sub);
    out
}

pub(crate) fn evaluate_one(
    env: &TakeoverEnv,
    ep: &Endpoint,
    app: ClientApp,
    sel: &Selection,
) -> Result<Evaluation, String> {
    // 直连写盘检查（全代理统一）：七端都从这里过，钳制只挂这一处。
    let clamped;
    let sel = if ep.direct {
        clamped = clamp_to_direct(sel, &ep.models);
        &clamped
    } else {
        sel
    };
    let mut evaluation = match app {
        ClientApp::Claude => claude::evaluate(env, ep, sel),
        ClientApp::Codex => codex::evaluate(env, ep, sel),
        ClientApp::OpenCode => opencode::evaluate(env, ep, sel),
        ClientApp::DevEco => opencode::evaluate_in(env, ep, sel, &opencode::DEV_ECO),
        ClientApp::Gemini => gemini::evaluate(env, ep, sel),
        ClientApp::Pi => pi::evaluate(env, ep, sel),
        ClientApp::CodeBuddy => codebuddy::evaluate(env, ep, sel),
        // copilot 没有配置文件：真值在 settings，走矩阵/门面支路。
        ClientApp::Copilot => return Err("copilot 是网关门面，没有配置文件可评估".into()),
    }?;
    // 路径自定义：回填每份文件的「内置默认路径」（身份键），界面据此显示
    // 「已自定义」并支持一键恢复。生效路径本身已在各端 *_path() 经 env.file 收口。
    for f in &mut evaluation.files {
        let effective = f.path.clone();
        f.default_path = env
            .default_of(Path::new(&effective))
            .unwrap_or_else(|| effective.clone());
        f.overridden = f.default_path != effective;
    }
    Ok(evaluation)
}

/// 设定 / 清除某端一份配置文件的路径自定义（代理 tab 片三）。
///
/// 三道闸与接管同源：`default` 必须是该端「无覆盖评测」下的白名单路径；
/// 自定义路径必须展开成绝对路径（`~/` 折叠为 home）；同一端不许两份文件
/// 指向同一路径（否则评测、编辑器、回滚会互相踩）。回滚的安全闸仍在
/// `restore_backup`（家目录外的清单路径一律拒绝还原）。
pub fn set_path_override(
    store: &Store,
    env: &TakeoverEnv,
    ep: &Endpoint,
    app: ClientApp,
    default: &str,
    custom: Option<&str>,
) -> Result<Value, String> {
    let plain = env.plain();
    let known: Vec<String> = evaluate_one(&plain, ep, app, &Selection::default())?
        .files
        .iter()
        .map(|f| f.path.clone())
        .collect();
    if !known.iter().any(|k| k == default) {
        return Err(format!("默认路径不在该端的接管白名单里：{default}"));
    }
    let custom_norm = match custom.map(str::trim) {
        None | Some("") => None,
        Some(raw) => {
            let p = if raw == "~" {
                env.home.clone()
            } else if let Some(rest) = raw.strip_prefix("~/") {
                env.home.join(rest)
            } else {
                let p = PathBuf::from(raw);
                if !p.is_absolute() {
                    return Err("路径必须是绝对路径，或以 ~ 开头".into());
                }
                p
            };
            let p = p.display().to_string();
            // 填回默认值 = 等价于恢复默认，直接清掉覆盖。
            (p != default).then_some(p)
        }
    };
    if let Some(c) = &custom_norm {
        if known.iter().any(|k| k == c) {
            return Err(format!("目标路径已是该端另一份配置的默认位置：{c}"));
        }
        for (d, existing) in store.path_overrides(app.as_str()) {
            if d != default && existing == *c {
                return Err(format!("该端另一份文件已自定义到同一路径：{c}"));
            }
        }
    }
    store
        .set_path_override(app.as_str(), default, custom_norm.as_deref())
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "app": app.as_str(),
        "default": default,
        "path": custom_norm,
    }))
}

/// 接管状态表（一屏看完：每端现状 / 是否待写入 / 最近备份）。
#[derive(Debug, Serialize)]
pub struct ClientStatus {
    pub app: &'static str,
    pub label: &'static str,
    pub managed: bool,
    /// 存在待写入差异（点接管会真的写盘）。
    pub pending: bool,
    pub files: Vec<FileEval>,
    /// 上次成功接管时间（settings 记账），从未接管为 None。
    pub applied_at: Option<i64>,
    pub reverted_at: Option<i64>,
    pub latest_backup: Option<String>,
    /// 模型选择的现状（文件里的真值 + 我们的期望）。
    pub model: ModelView,
    /// 该端评测失败的原因（一端失败不拖垮整张表）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 模型选择的两个口径并存，是因为**真相在文件里**：
/// `current` 现场读配置文件，`desired` 是我们上次写下去的期望。
/// 两者不一致 = 有人在客户端那边（或手工编辑里）动过。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ModelView {
    pub current: Selection,
    pub desired: Selection,
    /// 该端不支持由我们定默认时的人话说明。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

static TAKEOVER_LOCK: Mutex<()> = Mutex::new(());

fn lock_takeover() -> std::sync::MutexGuard<'static, ()> {
    TAKEOVER_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn status(
    env: &TakeoverEnv,
    store: &Store,
    ep: &Endpoint,
) -> Result<Vec<ClientStatus>, String> {
    let backups = list_backups(env).unwrap_or_default();
    let mut out = Vec::with_capacity(ClientApp::FILE_CLIENTS.len());
    for app in ClientApp::FILE_CLIENTS {
        let state = read_state(store, app);
        let latest_backup = backups
            .iter()
            .find(|b| b.app == app.as_str())
            .map(|b| b.backup_id.clone());
        let desired = Selection::load(store, app);
        let (files, pending, error) = match evaluate_one(env, ep, app, &desired) {
            Ok(ev) => (ev.files.clone(), !ev.edits.is_empty(), None),
            Err(e) => (Vec::new(), false, Some(e)),
        };
        let current = read_selection(env, app).unwrap_or_default();
        let model = ModelView {
            current,
            desired,
            note: model_note(app),
        };
        out.push(ClientStatus {
            app: app.as_str(),
            label: app.label(),
            managed: error.is_none() && !files.is_empty() && files.iter().all(|f| f.managed),
            pending,
            files,
            applied_at: state.applied_at,
            reverted_at: state.reverted_at,
            latest_backup,
            model,
            error,
        });
    }
    Ok(out)
}

/// 接管一个客户端：先快照原文件，再写差异。已是接管态时不写盘、不留备份。
pub fn apply(
    env: &TakeoverEnv,
    store: &Store,
    ep: &Endpoint,
    app: ClientApp,
) -> Result<Value, String> {
    let _guard = lock_takeover();
    env.ensure_dirs()?;
    let evaluation = evaluate_one(env, ep, app, &Selection::load(store, app))?;
    if evaluation.edits.is_empty() {
        return Ok(json!({
            "app": app.as_str(),
            "changed": false,
            "written": [],
            "backup_id": Value::Null,
            "files": evaluation.files,
        }));
    }

    let touched: Vec<PathBuf> = evaluation
        .files
        .iter()
        .map(|f| PathBuf::from(&f.path))
        .collect();
    // 原件备份**只在进入接管态的那一刻**立一份：接管态内再写盘（切模型、
    // 元数据 resync、手工同步）沿用同一份，回滚永远还原它（见 `revert`）。
    // 接管前原件被删过就补不回来了——宁可没有备份，也不许把「已经改过的
    // 文件」当成原件备份下来。
    let state = read_state(store, app);
    let backup_id: Option<String> = if taken_over(store, app) {
        state.backup_id.clone()
    } else {
        Some(write_backup(env, app, &touched, &ep.base_root)?)
    };

    let mut written = Vec::new();
    for edit in &evaluation.edits {
        atomic_write_private(&edit.path, &edit.content)?;
        written.push(edit.path.display().to_string());
    }

    write_state(
        store,
        app,
        &State {
            applied_at: Some(now_ms()),
            reverted_at: None,
            endpoint: Some(ep.base_root.clone()),
            backup_id: backup_id.clone(),
        },
    )?;

    Ok(json!({
        "app": app.as_str(),
        "changed": true,
        "written": written,
        "backup_id": backup_id.map(Value::String).unwrap_or(Value::Null),
        "files": evaluation.files,
    }))
}

/// 模型元数据变化（探测落真值 / 人工改参）后，把所有**受管端**的配置自动重写。
///
/// 口径：只重写「接管成功过、未回滚、且端点没换」的端；无差异的端不写盘；
/// 有差异的端也不新增备份（接管态内的原件备份早就留过了）；一端失败只记录，
/// 不拖垮其它端。返回值 = 实际发生写入（或出错）的报告列表，供调用方记日志。
pub fn resync_managed(env: &TakeoverEnv, store: &Store, config: &Config) -> Vec<Value> {
    let ep = Endpoint::build(config, store);
    let mut out = Vec::new();
    for app in ClientApp::FILE_CLIENTS {
        let state = read_state(store, app);
        let managed = state.applied_at.is_some()
            && state.reverted_at.is_none()
            && state.endpoint.as_deref() == Some(ep.base_root.as_str());
        if !managed {
            continue;
        }
        match apply(env, store, &ep, app) {
            Ok(report) => {
                if report["changed"] == json!(true) {
                    out.push(report);
                }
            }
            Err(e) => out.push(json!({ "app": app.as_str(), "error": e })),
        }
    }
    out
}

/// 改端口后的接管联动重写（用户 2026-11-18：「改了暴露端口，所有接管的代理都要跟着更新」）。
///
/// 为什么不能直接用 `resync_managed`：那条路只认「库里记的 endpoint 已等于当前端点」
/// 的条目，而端口刚改完时库里记的还是旧端点，判定必然为假 → 一个都不重写，客户端
/// 继续往旧端口打（就是「改了个寂寞」）。这里反过来按**旧端点**认领接管态，再用新端点
/// 重写；不是我们写的（旧端点不是我们）一律不碰，与接管启动口径一致。
pub fn resync_after_endpoint_change(
    env: &TakeoverEnv,
    store: &Store,
    config: &Config,
    old_base_root: &str,
) -> Vec<Value> {
    let ep = Endpoint::build(config, store);
    if ep.base_root == old_base_root {
        return Vec::new();
    }
    let mut out = Vec::new();
    for app in ClientApp::FILE_CLIENTS {
        let state = read_state(store, app);
        let ours = state.applied_at.is_some()
            && state.reverted_at.is_none()
            && state.endpoint.as_deref() == Some(old_base_root);
        if !ours {
            continue;
        }
        match apply(env, store, &ep, app) {
            Ok(report) => out.push(report),
            Err(e) => out.push(json!({ "app": app.as_str(), "error": e })),
        }
    }
    out
}

/// 直连模式的伪端点（计划 2026-11-18-backend-direct-mode 决策 3/4）。
///
/// 复用 `apply` 的写盘链路：各客户端只消费 `base_root` / `api_root` / `token` 三值，
/// 直连时把三者换成后端真实地址与 Key 环境变量引用即可，模型清单与元数据照旧。
/// `base_root` 同时是状态表里认领接管态的键——进入直连按网关根认领，离开直连
/// 按后端根认领，与 `resync_after_endpoint_change` 的「按旧端点认领」同一口径。
///
/// 已知局限：`token` 写成 `${VAR}` 字面量，Claude/Gemini 的 env 位天然由 shell
/// 展开；codex/opencode/pi/codebuddy 是配置字段值，是否展开取决于各家实现，
/// 未逐一实测（计划验收 7 的真机验证覆盖）。
pub fn direct_endpoint(store: &Store, backend: &crate::types::BackendRow) -> Endpoint {
    let base_root = backend.base_url.trim().trim_end_matches('/').to_string();
    Endpoint {
        token: format!("${{{}}}", Endpoint::direct_key_var(&backend.name)),
        api_root: format!("{base_root}/v1"),
        base_root,
        // 直连只认这一个后端：模型清单必须收窄到它的暴露模型，
        // 全后端清单会让多槽代理（只连一个 URL）把槽位指到别家模型上 503。
        models: backend
            .exposed_models()
            .into_iter()
            .filter(|m| !m.trim().is_empty())
            .collect(),
        meta: store.effective_model_meta().unwrap_or_default(),
        codex_wire_api: store
            .get_setting(CODEX_WIRE_API_SETTING)
            .ok()
            .flatten()
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| v == "responses")
            .unwrap_or_else(|| "chat".to_string()),
        direct: true,
    }
}

/// 直连开关变更后的接管联动重写（计划决策 4：进入与离开都要重写，失败只告警不静默）。
///
/// 认领口径与 `resync_after_endpoint_change` 一致：只动「接管成功过、未回滚、
/// 且库里端点 == 切换前的根」的客户端——进入直连时旧根是网关，离开时旧根是后端
/// 真实地址。返回逐家报告（含 `error` 的即未写成，调用方 toast 列名单）。
pub fn resync_for_direct_change(
    env: &TakeoverEnv,
    store: &Store,
    config: &Config,
    backend: &crate::types::BackendRow,
    to_direct: bool,
) -> Vec<Value> {
    let gw = Endpoint::base_root(&config.server.host, config.server.port);
    let direct = direct_endpoint(store, backend);
    let (old_root, new_ep) = if to_direct {
        (gw, direct)
    } else {
        (direct.base_root.clone(), Endpoint::build(config, store))
    };
    if old_root == new_ep.base_root {
        return Vec::new();
    }
    let mut out = Vec::new();
    for app in ClientApp::FILE_CLIENTS {
        let state = read_state(store, app);
        let ours = state.applied_at.is_some()
            && state.reverted_at.is_none()
            && state.endpoint.as_deref() == Some(old_root.as_str());
        if !ours {
            continue;
        }
        match apply(env, store, &new_ep, app) {
            Ok(report) => out.push(report),
            Err(e) => {
                tracing::warn!(target: "llm_switch::takeover", app = app.as_str(), backend = %backend.name, "{e}");
                out.push(json!({ "app": app.as_str(), "error": e }));
            }
        }
    }
    out
}

/// 该端支持的档位名（只有 claude 有档位表；别家给了就忽略，不假装生效）。
pub fn tiers_of(app: ClientApp) -> &'static [&'static str] {
    match app {
        ClientApp::Claude => &claude::TIERS,
        ClientApp::CodeBuddy => &codebuddy::TIERS,
        _ => &[],
    }
}

pub(crate) fn model_note(app: ClientApp) -> Option<String> {
    match app {
        // pi 的默认模型在它自己的 models-store.json 里，我们不碰别人的状态文件；
        // 主/小模型两个槽是我们替 pi 写进 models.json 的 piSlots（矩阵里分配）。
        ClientApp::Pi => Some(
            "pi 的默认模型由 pi 自己管理；主/小模型两个槽写进 models.json 的 piSlots，面板与矩阵都能改，两个槽可以是同一个模型"
                .into(),
        ),
        ClientApp::Codex => Some(
            "codex 的主模型写顶层 model，子代理模型写 [agents] default_subagent_model，两个槽可分可同，面板与矩阵都能改"
                .into(),
        ),
        ClientApp::Gemini => Some("gemini 配置文件尚不存在时，接管才会创建".into()),
        ClientApp::DevEco => {
            Some("DevEco Code 的内核是 opencode，配置在 ~/.config/deveco/deveco.jsonc".into())
        }
        ClientApp::CodeBuddy => Some(
            "codebuddy 的连接键在 models.json（url/apiKey），默认档与 lite/reasoning 变体在 settings.json"
                .into(),
        ),
        ClientApp::Copilot => Some(
            "copilot 是网关的 Ollama 门面下游：可见清单存 settings（缺省全暴露），在矩阵里勾选"
                .into(),
        ),
        _ => None,
    }
}

/// 现场读配置文件，得到「这个客户端现在真的在用哪些模型」。
pub fn read_selection(env: &TakeoverEnv, app: ClientApp) -> Result<Selection, String> {
    match app {
        ClientApp::Claude => claude::read_selection(env),
        ClientApp::Codex => codex::read_selection(env),
        ClientApp::OpenCode => opencode::read_selection(env),
        ClientApp::DevEco => opencode::read_selection_in(env, &opencode::DEV_ECO),
        ClientApp::Gemini => gemini::read_selection(env),
        ClientApp::Pi => pi::read_selection(env),
        ClientApp::CodeBuddy => codebuddy::read_selection(env),
        // copilot 没有配置文件：真值在 settings 的可见清单（store.copilot_allow），
        // 走矩阵合成的支路。漏特判到这里就显式炸，别静默给错值。
        ClientApp::Copilot => Err("copilot 是网关门面，模型真值在 settings，不经配置文件".into()),
    }
}

/// 该端现在是否处于接管态：apply 成功过、之后没还原过。
/// 这个口径只允许住在这里——`apply` 决定要不要立原件备份、`connection_managed`
/// 判漂移、`set_model` 判要不要自动取消接管，三处共用，不许各自重拼一遍。
fn taken_over(store: &Store, app: ClientApp) -> bool {
    let state = read_state(store, app);
    state.applied_at.is_some() && state.reverted_at.is_none()
}

/// 连接键是否已到位（不含模型）。
///
/// 它不再是「能不能写模型」的闸门（用户 2026-11-17：分配即接管），只用来告诉
/// 矩阵/详情这一端现在是不是接管态：未接管时取消分配无东西可取消，得明确报错。
pub(crate) fn connection_managed(
    env: &TakeoverEnv,
    store: &Store,
    ep: &Endpoint,
    app: ClientApp,
) -> Result<bool, String> {
    // copilot 是网关自己的 Ollama 门面下游：没有外部配置文件可评估，恒为就绪。
    if app == ClientApp::Copilot {
        return Ok(true);
    }
    // 用**文件真值**当期望来评估：这样模型层面的差异（我们自己写的、
    // 或者用户手工改的）不会被误判成「没接管」，剩下的只有连接本身
    // （base_url / token / provider 节点）是否是我们的。
    let truth = read_selection(env, app).unwrap_or_default();
    if evaluate_one(env, ep, app, &truth)?.managed() {
        return Ok(true);
    }
    // 曾经 apply 且没还原过 = 连接是我们的，只是模型层面还有待同步的改动。
    // 这种情况不能叫「没接管」，否则矩阵会因为一点漂移把整列锁死。
    Ok(taken_over(store, app))
}

/// 某一端的模型面板所需的全部信息。
pub fn model_view(
    env: &TakeoverEnv,
    store: &Store,
    ep: &Endpoint,
    app: ClientApp,
) -> Result<Value, String> {
    Ok(json!({
        "app": app.as_str(),
        "current": read_selection(env, app)?,
        "desired": Selection::load(store, app),
        "options": served_models(store),
        // 下拉按后端分组用（接管页 ClientModelPicker）：[{backend, models}]，
        // 与 options 同口径；前端没拿到该字段时（老后端）回退平铺 options。
        "option_groups": option_groups_json(store),
        "tiers": tiers_of(app),
        // 面板可编辑的槽位：档位制 = 档位；pi = 主/小两槽；codex = 副槽
        // （主槽就是 default，面板上已经是第一个下拉）。
        "slots": match app {
            ClientApp::Pi => Selection::PI_SLOT_KEYS.to_vec(),
            ClientApp::Codex => Selection::CODEX_SLOT_KEYS[1..].to_vec(),
            _ => tiers_of(app).to_vec(),
        },
        "taken_over": connection_managed(env, store, ep, app)?,
        "note": model_note(app),
    }))
}

/// 面板只送「用户动过手」的槽（键缺席 = 没动过，空串 = 显式清空）：把没送的槽
/// 用文件真值回填，否则改一个槽会把同端另一个槽洗掉（分配小模型清空主模型的根因）。
/// `touched` = 面板这次送过的槽键集合；只在对应 `manage_*` 为真时回填。
pub fn merge_untouched_slots(
    sel: &mut Selection,
    truth: &Selection,
    touched: &std::collections::BTreeSet<String>,
) {
    if sel.manage_pi {
        if !touched.contains("main") {
            sel.pi_main = truth.pi_main.clone();
        }
        if !touched.contains("small") {
            sel.pi_small = truth.pi_small.clone();
        }
    }
    if sel.manage_codex && !touched.contains("sub") {
        sel.codex_sub = truth.codex_sub.clone();
    }
}

/// 设定模型选择并立刻落盘（走接管的同一套快照与原子写）。
pub fn set_model(
    env: &TakeoverEnv,
    store: &Store,
    ep: &Endpoint,
    app: ClientApp,
    mut sel: Selection,
) -> Result<Value, String> {
    // 闸门改了口径（用户 2026-11-17）：**分配即接管**。这里不再因为「还没接管」
    // 就拒写模型——末尾的 `apply` 自己会先快照原件、再写连接配置、再落接管态，
    // 与手动点「接管」是同一条路（不另写一套，原件备份的时序口径也跟着复用）。
    if tiers_of(app).is_empty() {
        sel.tiers.clear();
    } else {
        // 丢掉不认识的空档位，别让脏数据留在配置里
        sel.tiers.retain(|tier, value| {
            !value.trim().is_empty() && tiers_of(app).contains(&tier.as_str())
        });
    }
    if let Some(d) = &sel.default {
        if d.trim().is_empty() {
            sel.default = None;
        }
    }
    if sel.allow.is_none() {
        // 面板/REST 只送 default + tiers，别把矩阵勾出来的清单洗成「没特别限定」。
        // 清单为空的写入口径是「只写已分配的模型」（见 `assigned_models`），
        // 一旦被洗空，用户在矩阵里分配过的清单会在下一次 apply 里凭空消失。
        // 真值取库里的期望：文件那份可能还没被 apply 收敛，跟着它走等于把
        // 旧版本的全量清单又锚回库里。
        sel.allow = Selection::load(store, app).allow;
    }
    if app == ClientApp::Pi && !sel.manage_pi {
        // 没动过槽位的写入（面板只改默认档、REST 老客户端）：槽位以文件真值为准，
        // 别让一次无关写入把用户配好的大小模型洗掉（同 manage_tiers 的理由）。
        let truth = read_selection(env, app)?;
        sel.pi_main = truth.pi_main;
        sel.pi_small = truth.pi_small;
    }
    if app == ClientApp::Codex && !sel.manage_codex {
        // 同上：没动过副槽的写入，副槽以文件真值为准，别洗掉用户配好的子代理模型。
        let truth = read_selection(env, app)?;
        sel.codex_sub = truth.codex_sub;
    }
    sel.save(store, app)?;
    /* 分配被清空 = 自动取消接管（用户 2026-11-17）：还原接管前原件、清接管态，
    与手动点「取消接管」同一条路（复用 `revert`）。copilot 是网关门面，没有
    配置文件可还原，不参与本条。没接管过又写空 = 无事可做，绝不能让它落到
    下面的 `apply`——那会拿着空分配把用户接管上去。 */
    if sel.is_unassigned() {
        let released = app != ClientApp::Copilot && taken_over(store, app);
        let back = if released {
            revert(env, store, app)?
        } else {
            Value::Null
        };
        return Ok(json!({
            "app": app.as_str(),
            "selection": sel,
            "current": read_selection(env, app).unwrap_or_default(),
            "changed": released,
            "backup_id": back.get("backup_id").cloned().unwrap_or(Value::Null),
            "reverted": released,
        }));
    }
    let applied = apply(env, store, ep, app)?;
    Ok(json!({
        "app": app.as_str(),
        "selection": sel,
        "current": read_selection(env, app)?,
        "changed": applied.get("changed").cloned().unwrap_or(Value::Bool(false)),
        "backup_id": applied.get("backup_id").cloned().unwrap_or(Value::Null),
        "reverted": false,
    }))
}

/// 回滚一个客户端：还原**接管前的那一份原件备份**（逐字节）+ 取消接管态。
///
/// 取哪一份：优先 `State.backup_id`（接管时立的原件）；老数据没记过账，
/// 才退回取该端最旧的一份当原件（`manual-edit` 快照是接管后编辑时留的，
/// 不是原件，跳过）。
pub fn revert(env: &TakeoverEnv, store: &Store, app: ClientApp) -> Result<Value, String> {
    let state = read_state(store, app);
    let backups = list_backups(env)?;
    let mut mine: Vec<&BackupEntry> = backups.iter().filter(|b| b.app == app.as_str()).collect();
    if let Some(id) = &state.backup_id {
        if let Some(hit) = mine.iter().find(|b| &b.backup_id == id) {
            return restore_backup(env, store, &hit.backup_id);
        }
    }
    // `list_backups` 是新→旧排的，回滚兜底要最旧（最接近接管前）那份。
    mine.sort_by_key(|b| b.created_at);
    let oldest = mine
        .iter()
        .find(|b| b.endpoint != "manual-edit")
        .ok_or_else(|| format!("{} 没有接管备份，无需回滚", app.label()))?;
    restore_backup(env, store, &oldest.backup_id)
}

// ============================== 备份 ==============================

/// 备份里的一个文件条目。路径来自我们自己算出来的客户端配置路径，
/// 但 `manifest.json` 可能被手工挪动过，回滚前照样重新校验父目录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupFile {
    /// 原始绝对路径（回滚写回这里）。
    pub path: String,
    /// 备份内的落盘序号名（`0`、`1`…）。
    pub slot: usize,
    pub name: String,
    pub existed: bool,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    app: String,
    created_at: i64,
    endpoint: String,
    files: Vec<BackupFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupEntry {
    pub backup_id: String,
    pub app: String,
    pub created_at: i64,
    pub endpoint: String,
    pub files: Vec<String>,
}

fn backup_dir_for_id(env: &TakeoverEnv, backup_id: &str) -> Result<PathBuf, String> {
    if backup_id.contains("..")
        || backup_id.contains('/')
        || backup_id.contains('\\')
        || backup_id.trim().is_empty()
    {
        return Err(format!("非法的备份 id: {backup_id}"));
    }
    Ok(env.backups.join(backup_id))
}

/// 快照一批文件（不存在的记 existed=false）。返回备份 id。
///
/// 备份里装的是**原样的密钥文件**，所以目录收 0700、副本与清单收 0600——
/// 否则 `~/.llm-switch/` 下一份 0644 的副本就能被同机别的账号读走。
pub(crate) fn write_backup(
    env: &TakeoverEnv,
    app: ClientApp,
    paths: &[PathBuf],
    endpoint: &str,
) -> Result<String, String> {
    let created_at = now_ms();
    let backup_id = format!("{}-{created_at}", app.as_str());
    let dir = env.backups.join(&backup_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建备份目录失败: {e}"))?;
    tighten_mode(&dir, 0o700);
    tighten_mode(&env.backups, 0o700);

    let mut files = Vec::new();
    for (slot, path) in paths.iter().enumerate() {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        let (existed, sha, payload) = match std::fs::read(path) {
            Ok(bytes) => (true, Some(sha256_hex(&bytes)), Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (false, None, None),
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(format!("备份 {} 失败: {e}", path.display()));
            }
        };
        if let Some(bytes) = payload {
            let copy = dir.join(slot.to_string());
            std::fs::write(&copy, bytes).map_err(|e| format!("写入备份副本失败: {e}"))?;
            tighten_mode(&copy, 0o600);
        }
        files.push(BackupFile {
            path: path.display().to_string(),
            slot,
            name,
            existed,
            sha256: sha,
        });
    }

    let manifest = Manifest {
        app: app.as_str().to_string(),
        created_at,
        endpoint: endpoint.to_string(),
        files,
    };
    let manifest_path = dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写入备份清单失败: {e}"))?;
    tighten_mode(&manifest_path, 0o600);
    Ok(backup_id)
}

fn read_manifest(dir: &Path) -> Result<Manifest, String> {
    let raw =
        std::fs::read(dir.join("manifest.json")).map_err(|e| format!("读取备份清单失败: {e}"))?;
    serde_json::from_slice(&raw).map_err(|e| format!("备份清单损坏: {e}"))
}

pub fn list_backups(env: &TakeoverEnv) -> Result<Vec<BackupEntry>, String> {
    let entries = match std::fs::read_dir(&env.backups) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取备份目录失败: {e}")),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(id) = path.file_name().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        let manifest = match read_manifest(&path) {
            Ok(m) => m,
            // 半成品/外来目录：跳过而不是让整张备份列表报错。
            Err(_) => continue,
        };
        out.push(BackupEntry {
            backup_id: id,
            app: manifest.app,
            created_at: manifest.created_at,
            endpoint: manifest.endpoint,
            files: manifest.files.iter().map(|f| f.path.clone()).collect(),
        });
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    Ok(out)
}

/// 还原指定备份：存在的文件逐字节写回，原本不存在的（我们创建的）删掉。
pub fn restore_backup(env: &TakeoverEnv, store: &Store, backup_id: &str) -> Result<Value, String> {
    let _guard = lock_takeover();
    let dir = backup_dir_for_id(env, backup_id)?;
    let manifest = read_manifest(&dir)?;
    let app = ClientApp::parse(&manifest.app)?;

    let mut restored = Vec::new();
    let mut removed = Vec::new();
    for file in &manifest.files {
        let target = PathBuf::from(&file.path);
        if !path_under_home(env, &target) {
            return Err(format!("备份清单里的路径越界，已拒绝还原: {}", file.path));
        }
        if file.existed {
            let bytes = std::fs::read(dir.join(file.slot.to_string()))
                .map_err(|e| format!("读取备份副本失败: {e}"))?;
            let digest = sha256_hex(&bytes);
            if let Some(want) = &file.sha256 {
                if want != &digest {
                    return Err(format!(
                        "备份副本校验失败（{}）：期望 {want}，实际 {digest}，已拒绝还原",
                        file.name
                    ));
                }
            }
            // 逐字节写回：这里一旦经手 String，非 UTF-8 的备份副本就会被
            // lossy 换成 U+FFFD，校验通过却写坏文件。
            atomic_write_private_bytes(&target, &bytes)?;
            restored.push(file.path.clone());
        } else if target.exists() {
            std::fs::remove_file(&target)
                .map_err(|e| format!("删除接管时创建的文件失败 {}: {e}", target.display()))?;
            removed.push(file.path.clone());
        }
    }

    write_state(
        store,
        app,
        &State {
            applied_at: None,
            reverted_at: Some(now_ms()),
            endpoint: None,
            backup_id: Some(backup_id.to_string()),
        },
    )?;

    Ok(json!({
        "app": app.as_str(),
        "backup_id": backup_id,
        "restored": restored,
        "removed": removed,
    }))
}

pub fn delete_backup(env: &TakeoverEnv, backup_id: &str) -> Result<(), String> {
    let dir = backup_dir_for_id(env, backup_id)?;
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("备份不存在".into()),
        Err(e) => Err(format!("删除备份失败: {e}")),
    }
}

/// 每个代理只保留最近 `keep` 份备份，更旧的自动删掉（用户口径：备份列表别无限堆）。
///
/// 但**当前接管的原件**（该端 `State.backup_id` 记的那份）永远保留——否则接管期间
/// 多次手改把原件挤出前三，「放弃接管」就还原不回接管前那份（回滚优先取 State.backup_id）。
/// `list_backups` 已按 created_at 倒序，逐端计数、超出 `keep` 且非原件者删。
pub fn prune_backups(env: &TakeoverEnv, store: &Store, keep: usize) {
    let all = match list_backups(env) {
        Ok(v) => v,
        Err(_) => return,
    };
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for b in all {
        let n = seen.entry(b.app.clone()).or_insert(0);
        *n += 1;
        if *n <= keep {
            continue;
        }
        // 超出保留数：保护该端当前接管原件，其余删除。
        if let Ok(app) = ClientApp::parse(&b.app) {
            if read_state(store, app).backup_id.as_deref() == Some(b.backup_id.as_str()) {
                continue;
            }
        }
        let _ = delete_backup(env, &b.backup_id);
    }
}

/// 备份清单里的路径必须仍在 home 之下（清单是磁盘上的可变数据，按不可信输入对待）。
fn path_under_home(env: &TakeoverEnv, path: &Path) -> bool {
    let home = env.home.display().to_string();
    let p = path.display().to_string();
    p == home || p.starts_with(&format!("{home}/"))
}

// ============================== settings 记账 ==============================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub applied_at: Option<i64>,
    pub reverted_at: Option<i64>,
    pub endpoint: Option<String>,
    pub backup_id: Option<String>,
}

fn read_state(store: &Store, app: ClientApp) -> State {
    store
        .get_setting(&app.state_key())
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn write_state(store: &Store, app: ClientApp, state: &State) -> Result<(), String> {
    let raw = serde_json::to_string(state).map_err(|e| e.to_string())?;
    store
        .set_setting(&app.state_key(), &raw)
        .map_err(|e| e.to_string())
}

// ============================== 共享小工具（各客户端模块共用） ==============================

pub(crate) fn read_raw(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("读取 {} 失败: {e}", path.display())),
    }
}

pub(crate) fn parse_json(raw: &str, path: &Path) -> Result<Value, String> {
    if raw.trim().is_empty() {
        return Err(format!(
            "{} 是空文件，为避免覆盖你的内容已拒绝接管，请先补齐为合法 JSON",
            path.display()
        ));
    }
    serde_json::from_str(raw).map_err(|e| {
        format!(
            "{} 不是合法 JSON（{e}），已拒绝接管，请先修复",
            path.display()
        )
    })
}

pub(crate) fn render_json(value: &Value) -> String {
    let mut s = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    s.push('\n');
    s
}

/// JSON 文件评测：`wanted` 是合并后的完整文档，`managed` 由调用方判定。
pub(crate) fn finish_json(
    path: &Path,
    raw: Option<String>,
    wanted: Value,
    managed: bool,
    note: String,
) -> (FileEval, Option<Edit>) {
    let rendered = render_json(&wanted);
    let dirty = raw.as_deref() != Some(rendered.as_str());
    let eval = FileEval {
        path: path.display().to_string(),
        format: Format::Json.as_str(),
        exists: raw.is_some(),
        managed,
        note,
        ..Default::default()
    };
    let edit = dirty.then(|| Edit {
        path: path.to_path_buf(),
        content: rendered,
    });
    (eval, edit)
}

/// 原子写并把权限收到 0600（这些文件从此装着凭据）。
pub(crate) fn atomic_write_private(path: &Path, data: &str) -> Result<(), String> {
    atomic_write_private_bytes(path, data.as_bytes())
}

/// 原子写的字节形态。**还原备份必须走这条**：先把备份读成 `String` 再写，
/// 非 UTF-8 的字节会被 lossy 换成 U+FFFD——sha256 校验通过了，写回去的却是
/// 另一份文件，「逐字节还原」就成了空话。
pub(crate) fn atomic_write_private_bytes(path: &Path, data: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
    }
    let tmp = path.with_file_name(format!(
        "{}.tmp-{}-{:x}",
        path.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into()),
        std::process::id(),
        rand_suffix()
    ));
    std::fs::write(&tmp, data).map_err(|e| format!("写入临时文件失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("替换 {} 失败: {e}", path.display())
    })
}

/// 收紧权限（0700 目录 / 0600 凭据文件）。失败只记不拦：老系统或某些
/// 文件系统上 chmod 可能不支持，那不该让接管/备份整体失败。
fn tighten_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
}

/// 临时文件名后缀（同进程多次写入也要唯一）。
fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    (nanos.wrapping_mul(2862933555777941757)) >> 16
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

pub(crate) fn json_str_eq(root: &Value, pointer: &str, want: &str) -> bool {
    root.pointer(pointer).and_then(Value::as_str) == Some(want)
}

/// `.env` 原位补丁：我们的键在原行位置改写，缺失的追加到末尾，别人的行不动。
pub(crate) fn dotenv_patch(raw: &str, wanted: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut placed = vec![false; wanted.len()];
    for line in raw.lines() {
        let key = line
            .split_once('=')
            .map(|(k, _)| k.trim().to_string())
            .unwrap_or_default();
        match wanted.iter().position(|(k, _)| *k == key.as_str()) {
            Some(idx) => {
                placed[idx] = true;
                lines.push(format!("{}={}", wanted[idx].0, wanted[idx].1));
            }
            None => lines.push(line.to_string()),
        }
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    for (idx, (k, v)) in wanted.iter().enumerate() {
        if !placed[idx] {
            lines.push(format!("{k}={v}"));
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

pub(crate) fn dotenv_has(raw: &str, key: &str, want: &str) -> bool {
    raw.lines().any(|line| {
        line.split_once('=')
            .map(|(k, v)| k.trim() == key && v.trim() == want)
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerCfg;
    use crate::types::{BackendUpsert, Protocol};
    use tempfile::TempDir;

    fn store_with_model() -> Store {
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&BackendUpsert {
                name: "lab".into(),
                base_url: "http://127.0.0.1:8000".into(),
                protocol: Protocol::OpenaiChat,
                api_key: None,
                weight: 1,
                enabled: true,
                engine: Some("vllm".into()),
                webui_url: None,
                webui_user: None,
                webui_password: None,
                health_path: None,
                models: vec!["qwen3-30b".into()],
                aliases: json!({}),
                capabilities: json!({}),
                params: json!({}),
            })
            .unwrap();
        store
    }

    struct Fixture {
        /// 持有临时目录句柄：离开作用域才清理。
        _tmp: TempDir,
        env: TakeoverEnv,
        store: Store,
        ep: Endpoint,
    }

    /// 两张矩阵同源的地基（D24）：ClientApp 与 SkillApp 必须是同一批客户端、
    /// 同一批键，否则模型矩阵和 Skills 矩阵的列会悄悄分家。
    #[test]
    fn client_and_skill_apps_stay_in_sync() {
        use crate::services::skill::SkillApp;
        let client: std::collections::BTreeSet<_> =
            ClientApp::FILE_CLIENTS.iter().map(|a| a.as_str()).collect();
        let skill: std::collections::BTreeSet<_> =
            SkillApp::ALL.iter().map(|a| a.as_str()).collect();
        // 技能纳管端 = 文件七端 + copilot：门面虽然没有配置文件可接管，但 CLI 真吃
        // Personal 技能（~/.copilot/skills，`copilot skill --help` 官方口径），
        // 所以技能矩阵必须有它一列——装了就出现，别拿「门面」当理由漏掉它。
        let mut expect = client.clone();
        expect.insert("copilot");
        assert_eq!(
            expect, skill,
            "SkillApp 要比文件客户端多出 copilot（门面吃技能），其余一一对应"
        );
        assert_eq!(ClientApp::FILE_CLIENTS.len(), 7, "七端（D24 加 codebuddy）");
        assert_eq!(ClientApp::ALL.len(), 8, "文件七端 + copilot 门面");
    }

    fn fixture() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let env = TakeoverEnv::with_roots(
            tmp.path().join("home"),
            tmp.path().join("data").join("takeover-backups"),
        );
        std::fs::create_dir_all(env.home.join(".claude")).unwrap();
        let store = store_with_model();
        let config = Config {
            server: ServerCfg {
                port: 8787,
                ..Default::default()
            },
            data_dir: Some(tmp.path().join("data")),
            ..Default::default()
        };
        let ep = Endpoint::build(&config, &store);
        Fixture {
            _tmp: tmp,
            env,
            store,
            ep,
        }
    }

    #[test]
    fn endpoint_uses_loopback_and_serves_models() {
        let store = store_with_model();
        let wildcard = Config {
            server: ServerCfg {
                host: "0.0.0.0".into(),
                port: 9100,
                ..Default::default()
            },
            ..Default::default()
        };
        let ep = Endpoint::build(&wildcard, &store);
        assert_eq!(ep.base_root, "http://127.0.0.1:9100");
        assert_eq!(ep.api_root, "http://127.0.0.1:9100/v1");
        assert_eq!(ep.token, PLACEHOLDER_TOKEN);
        assert_eq!(ep.models, vec!["qwen3-30b".to_string()]);

        let guarded = Config {
            server: ServerCfg {
                access_token: Some("s3cret".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(Endpoint::build(&guarded, &store).token, "s3cret");
    }

    #[test]
    fn apply_backs_up_and_revert_restores_exact_bytes() {
        let f = fixture();
        let original = "{ \"env\": { \"ANTHROPIC_BASE_URL\": \"http://elsewhere\" } }\n";
        let target = claude::settings_path(&f.env);
        std::fs::write(&target, original).unwrap();

        let report = apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        assert_eq!(report["changed"], json!(true));
        assert!(report["backup_id"].is_string());
        let after = std::fs::read_to_string(&target).unwrap();
        assert!(after.contains("127.0.0.1:8787"), "{after}");

        let restored = revert(&f.env, &f.store, ClientApp::Claude).unwrap();
        assert_eq!(restored["restored"].as_array().unwrap().len(), 1);
        assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
        // 回滚后不再是接管态。
        let evaluation = claude::evaluate(&f.env, &f.ep, &Selection::default()).unwrap();
        assert!(!evaluation.managed());
        let state = read_state(&f.store, ClientApp::Claude);
        assert!(state.reverted_at.is_some() && state.applied_at.is_none());
    }

    /// 改暴露端口必须联动重写已接管的客户端配置（用户 2026-11-18）。
    /// 关键在认领口径：端口刚改完时库里记的还是**旧**端点，`resync_managed` 那条路
    /// 判定必然为假、一个都不重写；`resync_after_endpoint_change` 按旧端点认领。
    #[test]
    fn port_change_rewrites_taken_over_clients_only() {
        let f = fixture();
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        let target = claude::settings_path(&f.env);
        assert!(std::fs::read_to_string(&target)
            .unwrap()
            .contains("127.0.0.1:8787"));

        let cfg = Config {
            server: ServerCfg {
                port: 9999,
                ..Default::default()
            },
            data_dir: Some(f._tmp.path().join("data")),
            ..Default::default()
        };
        let reports = resync_after_endpoint_change(&f.env, &f.store, &cfg, &f.ep.base_root);
        assert_eq!(reports.len(), 1, "{reports:?}");
        let after = std::fs::read_to_string(&target).unwrap();
        assert!(after.contains("127.0.0.1:9999"), "{after}");
        assert!(!after.contains("8787"), "{after}");
        // 库里记的端点跟着走，下次改端口才认领得到新值。
        assert_eq!(
            read_state(&f.store, ClientApp::Claude).endpoint.as_deref(),
            Some("http://127.0.0.1:9999")
        );
        // 旧端点对不上（不是我们写的 / 已经回滚）就一个都不碰。
        assert!(
            resync_after_endpoint_change(&f.env, &f.store, &cfg, "http://127.0.0.1:1111")
                .is_empty()
        );
    }

    #[test]
    fn apply_creates_file_and_revert_removes_it() {
        let f = fixture();
        let target = claude::settings_path(&f.env);
        assert!(!target.exists());

        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        assert!(target.exists());

        revert(&f.env, &f.store, ClientApp::Claude).unwrap();
        assert!(!target.exists(), "接管时新建的文件回滚时应删除");
    }

    #[test]
    fn second_apply_changes_nothing_and_leaves_one_backup() {
        let f = fixture();
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        let again = apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        assert_eq!(again["changed"], json!(false));
        assert!(again["backup_id"].is_null());
        assert_eq!(list_backups(&f.env).unwrap().len(), 1);
    }

    /// 原件只备一份：备份发生在**进入接管态的那一刻**，里面是接管前的原件；
    /// 接管态内的重写（切模型 / resync / 手工同步）不再新增备份，回滚永远还原
    /// 那份原件，而不是「上一次写盘前」的中间态。
    #[test]
    fn takeover_backs_up_original_once_and_revert_restores_it() {
        let f = fixture();
        let original = "{ \"env\": { \"ANTHROPIC_BASE_URL\": \"http://elsewhere\" } }\n";
        let target = claude::settings_path(&f.env);
        std::fs::write(&target, original).unwrap();

        let first = apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        assert!(first["backup_id"].is_string());

        // 接管态内切模型：真写盘了，但不立新备份。
        set_model(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            Selection {
                default: Some("qwen3-30b".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let backups = list_backups(&f.env).unwrap();
        assert_eq!(backups.len(), 1, "接管态内的重写不该再备一份: {backups:?}");
        let mid = std::fs::read_to_string(&target).unwrap();
        assert!(mid != original && mid.contains("qwen3-30b"), "{mid}");

        // 回滚还原的是接管前那份原件，不是切模型前的中间态。
        revert(&f.env, &f.store, ClientApp::Claude).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
    }

    /// state 没了记账（老数据 / 手动清过）：回滚退回取该端**最旧**的一份当原件，
    /// 且跳过 `manual-edit` 快照——那是接管后编辑留下的，不是原件。
    #[test]
    fn revert_without_state_falls_back_to_oldest_takeover_backup() {
        let f = fixture();
        let original = "{ \"env\": { \"ANTHROPIC_BASE_URL\": \"http://elsewhere\" } }\n";
        let target = claude::settings_path(&f.env);
        std::fs::write(&target, original).unwrap();
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();

        // 接管之后又留了一份编辑快照（更新的那份）
        std::thread::sleep(std::time::Duration::from_millis(2));
        let edit_id = write_backup(
            &f.env,
            ClientApp::Claude,
            std::slice::from_ref(&target),
            "manual-edit",
        )
        .unwrap();
        assert_eq!(
            list_backups(&f.env).unwrap()[0].backup_id,
            edit_id,
            "快照在最前"
        );
        f.store
            .set_setting(&ClientApp::Claude.state_key(), "")
            .unwrap();

        revert(&f.env, &f.store, ClientApp::Claude).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
    }

    #[test]
    fn backups_list_newest_first_and_delete_works() {
        let f = fixture();
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        // 第一次接管留下 1 份备份；第二次无差异不再备份。
        let backups = list_backups(&f.env).unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].app, "claude");
        assert!(!backups[0].files.is_empty());

        let id = backups[0].backup_id.clone();
        delete_backup(&f.env, &id).unwrap();
        assert!(list_backups(&f.env).unwrap().is_empty());
        assert!(delete_backup(&f.env, &id).is_err());
    }

    #[test]
    fn revert_without_backup_is_a_clear_error() {
        let f = fixture();
        let err = revert(&f.env, &f.store, ClientApp::Gemini).unwrap_err();
        assert!(err.contains("无需回滚"), "{err}");
    }

    #[test]
    fn restore_refuses_paths_outside_home() {
        let f = fixture();
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        let id = list_backups(&f.env).unwrap()[0].backup_id.clone();
        let dir = f.env.backups.join(&id);
        let manifest_path = dir.join("manifest.json");
        let mut manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest["files"][0]["path"] = json!("/etc/hosts");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let err = restore_backup(&f.env, &f.store, &id).unwrap_err();
        assert!(err.contains("越界"), "{err}");
    }

    #[test]
    fn restore_detects_tampered_payload() {
        let f = fixture();
        let original = "{\"env\":{}}\n";
        std::fs::write(claude::settings_path(&f.env), original).unwrap();
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        let id = list_backups(&f.env).unwrap()[0].backup_id.clone();
        std::fs::write(f.env.backups.join(&id).join("0"), b"tampered").unwrap();
        let err = restore_backup(&f.env, &f.store, &id).unwrap_err();
        assert!(err.contains("校验失败"), "{err}");
    }

    /// 还原必须**逐字节**：旧实现把备份读成 `String` 再写回，非 UTF-8 字节会被
    /// lossy 换成 U+FFFD——sha256 校验通过，写回去的却是另一份文件。
    #[test]
    fn restore_is_byte_exact_even_for_non_utf8_files() {
        let f = fixture();
        let target = f.env.home.join("odd.conf");
        let original = vec![0xffu8, 0xfe, 0x7b, 0x00, 0xc3, 0x28, b'"'];
        std::fs::write(&target, &original).unwrap();

        let id = write_backup(
            &f.env,
            ClientApp::Claude,
            std::slice::from_ref(&target),
            "test",
        )
        .unwrap();
        std::fs::write(&target, b"corrupted in the meantime").unwrap();

        restore_backup(&f.env, &f.store, &id).unwrap();
        assert_eq!(
            std::fs::read(&target).unwrap(),
            original,
            "还原出来的字节必须与备份逐字节相同"
        );
    }

    /// 备份装的是原样的密钥文件：目录 0700、副本与清单 0600。
    #[cfg(unix)]
    #[test]
    fn backup_files_are_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let f = fixture();
        let target = f.env.home.join("secret.conf");
        std::fs::write(&target, "api_key = \"sk\"\n").unwrap();
        let id = write_backup(&f.env, ClientApp::Claude, &[target], "test").unwrap();
        let dir = f.env.backups.join(&id);
        assert_eq!(
            dir.metadata().unwrap().permissions().mode() & 0o777,
            0o700,
            "备份目录该是 0700"
        );
        for name in ["0", "manifest.json"] {
            let mode = dir.join(name).metadata().unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{name} 该是 0600，实得 {mode:o}");
        }
    }

    /// 手工造一份备份目录（created_at 可控，避开 write_backup 同毫秒撞 id）。
    fn mk_backup(env: &TakeoverEnv, app: &str, created_at: i64) -> String {
        let id = format!("{app}-{created_at}");
        let dir = env.backups.join(&id);
        std::fs::create_dir_all(&dir).unwrap();
        let m = serde_json::json!({
            "app": app, "created_at": created_at, "endpoint": "test", "files": []
        });
        std::fs::write(dir.join("manifest.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        id
    }

    fn claude_backups(env: &TakeoverEnv) -> Vec<String> {
        let mut v: Vec<String> = list_backups(env)
            .unwrap()
            .into_iter()
            .filter(|b| b.app == "claude")
            .map(|b| b.backup_id)
            .collect();
        v.sort();
        v
    }

    #[test]
    fn prune_backups_keeps_only_recent_per_app() {
        let f = fixture();
        for t in 1..=5 {
            mk_backup(&f.env, "claude", t);
        }
        assert_eq!(claude_backups(&f.env).len(), 5);
        prune_backups(&f.env, &f.store, 3);
        // 只留最近 3 份（created_at 3/4/5）。
        assert_eq!(
            claude_backups(&f.env),
            vec!["claude-3", "claude-4", "claude-5"]
        );
    }

    #[test]
    fn prune_backups_protects_takeover_original() {
        let f = fixture();
        let oldest = mk_backup(&f.env, "claude", 1);
        for t in 2..=6 {
            mk_backup(&f.env, "claude", t);
        }
        // 把最旧那份记为该端当前接管原件（State.backup_id）。
        let st = State {
            backup_id: Some(oldest.clone()),
            ..Default::default()
        };
        f.store
            .set_setting(
                ClientApp::Claude.state_key().as_str(),
                &serde_json::to_string(&st).unwrap(),
            )
            .unwrap();
        prune_backups(&f.env, &f.store, 3);
        let kept = claude_backups(&f.env);
        assert!(kept.contains(&oldest), "接管原件必须不被剪掉");
        // 最近 3 份（4/5/6）+ 受保护原件（1）= 4 份。
        assert_eq!(kept, vec!["claude-1", "claude-4", "claude-5", "claude-6"]);
    }

    #[test]
    fn status_reports_pending_then_managed() {
        let f = fixture();
        let rows = status(&f.env, &f.store, &f.ep).unwrap();
        assert_eq!(rows.len(), ClientApp::FILE_CLIENTS.len());
        assert!(
            rows.iter().all(|r| r.app != "copilot"),
            "copilot 是网关门面，不进接管状态列表"
        );
        let claude_row = rows.iter().find(|r| r.app == "claude").unwrap();
        assert!(claude_row.pending && !claude_row.managed);

        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        let rows = status(&f.env, &f.store, &f.ep).unwrap();
        let claude_row = rows.iter().find(|r| r.app == "claude").unwrap();
        assert!(!claude_row.pending && claude_row.managed);
        assert!(claude_row.applied_at.is_some());
        assert!(claude_row.latest_backup.is_some());
    }

    #[test]
    fn status_survives_a_broken_client_file() {
        let f = fixture();
        std::fs::write(claude::settings_path(&f.env), "{{{").unwrap();
        let rows = status(&f.env, &f.store, &f.ep).unwrap();
        let broken = rows.iter().find(|r| r.app == "claude").unwrap();
        assert!(broken.error.is_some());
        // 另一端照常评测，不被带崩。
        assert!(rows.iter().find(|r| r.app == "codex").unwrap().pending);
    }

    #[test]
    fn apply_refuses_broken_user_file_without_touching_it() {
        let f = fixture();
        let target = claude::settings_path(&f.env);
        std::fs::write(&target, "nope").unwrap();
        assert!(apply(&f.env, &f.store, &f.ep, ClientApp::Claude).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "nope");
        assert!(list_backups(&f.env).unwrap().is_empty());
    }

    #[test]
    fn backup_id_rejects_traversal() {
        let f = fixture();
        assert!(backup_dir_for_id(&f.env, "../../etc").is_err());
        assert!(backup_dir_for_id(&f.env, "").is_err());
        assert!(backup_dir_for_id(&f.env, "claude-1").is_ok());
    }

    #[test]
    fn dotenv_helpers() {
        let patched = dotenv_patch("A=1\nOURS=x\n", &[("OURS", "y"), ("NEW", "z")]);
        assert_eq!(patched, "A=1\nOURS=y\nNEW=z\n");
        assert!(dotenv_has(&patched, "NEW", "z"));
        assert!(!dotenv_has(&patched, "NEW", "nope"));
    }

    #[test]
    fn resync_managed_rewrites_taken_over_clients_on_meta_change() {
        let f = fixture();
        f.env.ensure_dirs().unwrap();
        let config = Config {
            server: ServerCfg {
                port: 8787,
                ..Default::default()
            },
            data_dir: Some(f._tmp.path().join("data")),
            ..Default::default()
        };

        // 未接管：元数据变了也不应碰任何配置文件。
        f.store
            .set_model_meta("qwen3-30b", 123_456, 0, 1, 1)
            .unwrap();
        assert!(resync_managed(&f.env, &f.store, &config).is_empty());

        // 接管 pi 前先给它占个主槽：清单只写**分配给它的模型**，没有分配就没有
        // 要维护的条目，谈不上「元数据变了自动重写」。
        Selection {
            pi_main: Some("qwen3-30b".into()),
            ..Default::default()
        }
        .save(&f.store, ClientApp::Pi)
        .unwrap();
        apply(&f.env, &f.store, &f.ep, ClientApp::Pi).unwrap();
        // 再改元数据：受管端自动重写，文件里能看到新真值。
        f.store
            .set_model_meta("qwen3-30b", 200_000, 0, 0, 0)
            .unwrap();
        let reports = resync_managed(&f.env, &f.store, &config);
        assert_eq!(reports.len(), 1, "{reports:?}");
        assert_eq!(reports[0]["app"], json!("pi"));
        let text =
            std::fs::read_to_string(f._tmp.path().join("home/.pi/agent/models.json")).unwrap();
        assert!(text.contains("200000"), "{text}");

        // 无变化再跑一次：幂等，不产生写入报告也不留新备份。
        assert!(resync_managed(&f.env, &f.store, &config).is_empty());
    }

    /// 路径自定义（代理 tab 片三）：env.file 收口读写，default_of 反查身份键，
    /// evaluate_one 统一回填 default_path / overridden 给界面。
    #[test]
    fn path_override_redirects_and_reverses() {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let default = home.join(".claude").join("settings.json");
        let custom = tmp.path().join("elsewhere").join("s.json");
        let mut env = TakeoverEnv::with_roots(home, tmp.path().join("b"));
        env.overrides
            .insert(default.display().to_string(), custom.display().to_string());

        assert_eq!(claude::settings_path(&env), custom, "读写同侧生效");
        assert_eq!(env.default_of(&custom), Some(default.display().to_string()));
        assert_eq!(env.default_of(&default), None, "默认路径本身不算被覆盖");
        assert_eq!(env.plain().file(&default), default, "plain = 纯内置路径");

        let store = Store::in_memory().unwrap();
        let ep = Endpoint::build(&Config::default(), &store);
        let ev = evaluate_one(&env, &ep, ClientApp::Claude, &Selection::default()).unwrap();
        let f = ev
            .files
            .iter()
            .find(|f| f.path == custom.display().to_string())
            .expect("覆盖后的路径应进评测清单");
        assert!(f.overridden);
        assert_eq!(f.default_path, default.display().to_string());
    }

    /// 覆盖与启停都落 settings 表：可写入、可往返、可清除。
    #[test]
    fn store_path_override_and_agent_toggle_roundtrip() {
        let store = Store::in_memory().unwrap();
        store
            .set_path_override("claude", "/h/.claude/settings.json", Some("/x/s.json"))
            .unwrap();
        let mut want = std::collections::BTreeMap::new();
        want.insert(
            "/h/.claude/settings.json".to_string(),
            "/x/s.json".to_string(),
        );
        assert_eq!(store.path_overrides("claude"), want);
        store
            .set_path_override("claude", "/h/.claude/settings.json", None)
            .unwrap();
        assert!(store.path_overrides("claude").is_empty());

        assert!(store.agents_disabled().is_empty());
        store.set_agent_enabled("codex", false).unwrap();
        store.set_agent_enabled("codex", false).unwrap(); // 幂等
        assert_eq!(store.agents_disabled(), vec!["codex".to_string()]);
        store.set_agent_enabled("codex", true).unwrap();
        assert!(store.agents_disabled().is_empty());
    }

    /// set_path_override 的三道闸：白名单身份、绝对路径、同端不许撞路径。
    #[test]
    fn set_path_override_enforces_whitelist_and_uniqueness() {
        let f = fixture();
        let known = f.env.home.join(".claude").join("settings.json");
        let known = known.to_str().unwrap();

        let err = set_path_override(
            &f.store,
            &f.env,
            &f.ep,
            ClientApp::Claude,
            "/nope/x.json",
            Some("/a/b.json"),
        )
        .unwrap_err();
        assert!(err.contains("白名单"), "未知默认路径必须被拒：{err}");

        let err = set_path_override(
            &f.store,
            &f.env,
            &f.ep,
            ClientApp::Claude,
            known,
            Some("rel/path.json"),
        )
        .unwrap_err();
        assert!(err.contains("绝对路径"), "相对路径必须被拒：{err}");

        // ~/ 折叠成 home 后落库
        set_path_override(
            &f.store,
            &f.env,
            &f.ep,
            ClientApp::Claude,
            known,
            Some("~/custom/settings.json"),
        )
        .unwrap();
        let folded = f
            .env
            .home
            .join("custom/settings.json")
            .display()
            .to_string();
        assert_eq!(
            f.store
                .path_overrides("claude")
                .get(known)
                .map(String::as_str),
            Some(folded.as_str())
        );

        // 同端两份文件不许指向同一路径
        let cb_models = f.env.home.join(".codebuddy").join("models.json");
        let cb_settings = f.env.home.join(".codebuddy").join("settings.json");
        set_path_override(
            &f.store,
            &f.env,
            &f.ep,
            ClientApp::CodeBuddy,
            cb_models.to_str().unwrap(),
            Some("/x/m.json"),
        )
        .unwrap();
        let err = set_path_override(
            &f.store,
            &f.env,
            &f.ep,
            ClientApp::CodeBuddy,
            cb_settings.to_str().unwrap(),
            Some("/x/m.json"),
        )
        .unwrap_err();
        assert!(err.contains("同一路径"), "撞路径必须被拒：{err}");

        // 填回默认 = 恢复默认（覆盖被清掉）
        set_path_override(
            &f.store,
            &f.env,
            &f.ep,
            ClientApp::Claude,
            known,
            Some(known),
        )
        .unwrap();
        assert!(f.store.path_overrides("claude").is_empty());
    }

    /// 直连切换必须联动重写已接管客户端（计划验收 2）：进入写后端真实地址 +
    /// Key 环境变量引用，离开写回网关地址；不是我们写的一律不碰。
    #[test]
    fn direct_change_rewrites_taken_over_clients_both_ways() {
        let f = fixture();
        let cfg = Config {
            server: ServerCfg {
                port: 8787,
                ..Default::default()
            },
            data_dir: Some(f._tmp.path().join("data")),
            ..Default::default()
        };
        apply(&f.env, &f.store, &f.ep, ClientApp::Claude).unwrap();
        let target = claude::settings_path(&f.env);
        assert!(std::fs::read_to_string(&target)
            .unwrap()
            .contains("127.0.0.1:8787"));

        let mut be = crate::types::backend_row_for_test("lab");
        be.base_url = "http://10.0.0.9:8000/".into();

        // 进入直连：认领旧根 = 网关根。
        let reports = resync_for_direct_change(&f.env, &f.store, &cfg, &be, true);
        assert_eq!(reports.len(), 1, "{reports:?}");
        let after = std::fs::read_to_string(&target).unwrap();
        assert!(after.contains("http://10.0.0.9:8000"), "{after}");
        assert!(!after.contains("8787"), "{after}");
        assert!(after.contains("LLM_SWITCH_LAB_KEY"), "{after}");
        assert_eq!(
            read_state(&f.store, ClientApp::Claude).endpoint.as_deref(),
            Some("http://10.0.0.9:8000"),
            "库里端点跟着走，离开直连才认领得到"
        );

        // 离开直连：认领旧根 = 后端真实地址，写回网关。
        let back = resync_for_direct_change(&f.env, &f.store, &cfg, &be, false);
        assert_eq!(back.len(), 1, "{back:?}");
        let restored = std::fs::read_to_string(&target).unwrap();
        assert!(restored.contains("127.0.0.1:8787"), "{restored}");
        assert!(!restored.contains("10.0.0.9"), "{restored}");

        // 认领口径：已回滚（不再是接管态）就一个都不碰。
        revert(&f.env, &f.store, ClientApp::Claude).unwrap();
        assert!(resync_for_direct_change(&f.env, &f.store, &cfg, &be, true).is_empty());
    }

    /// 直连伪端点：地址去尾斜杠、api_root 带 /v1、Key 位是环境变量引用。
    #[test]
    fn direct_endpoint_shapes_urls_and_token() {
        let f = fixture();
        let mut be = crate::types::backend_row_for_test("my lab");
        be.base_url = "http://10.0.0.9:8000/".into();
        let ep = direct_endpoint(&f.store, &be);
        assert_eq!(ep.base_root, "http://10.0.0.9:8000");
        assert_eq!(ep.api_root, "http://10.0.0.9:8000/v1");
        assert_eq!(ep.token, "${LLM_SWITCH_MY_LAB_KEY}");
    }

    /// 直连伪端点的模型清单只收本后端（fixture 的 lab 后端暴露 qwen3-30b），
    /// 别家后端的模型不得混进来——多槽代理只连一个 URL，混进来就是 503。
    #[test]
    fn direct_endpoint_models_are_only_this_backend() {
        let f = fixture();
        f.store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "other".into(),
                base_url: "http://10.9.9.9:1".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                models: vec!["foreign-model".into()],
                ..Default::default()
            })
            .unwrap();
        let mut be = crate::types::backend_row_for_test("lab");
        be.models = vec!["qwen3-30b".into()];
        let ep = direct_endpoint(&f.store, &be);
        assert_eq!(ep.models, vec!["qwen3-30b".to_string()]);
        assert!(ep.direct);
    }

    /// 直连写盘检查（全代理）：进入直连后每个已接管端的槽位都钳进本后端模型。
    /// claude 档位、pi 主小槽、codex 副槽一个都不许漏——配置文件只连一个 URL。
    #[test]
    fn direct_rewrite_clamps_slots_for_all_clients() {
        let f = fixture();
        let cfg = Config {
            server: ServerCfg {
                port: 8787,
                ..Default::default()
            },
            data_dir: Some(f._tmp.path().join("data")),
            ..Default::default()
        };
        // 每端槽位都指向别家模型 foreign。
        for app in [ClientApp::Claude, ClientApp::Pi, ClientApp::Codex] {
            let sel = Selection {
                default: Some("foreign".into()),
                tiers: [("opus", "foreign")]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                manage_tiers: true,
                pi_main: Some("foreign".into()),
                pi_small: Some("foreign".into()),
                codex_sub: Some("foreign".into()),
                ..Default::default()
            };
            sel.save(&f.store, app).unwrap();
            apply(&f.env, &f.store, &f.ep, app).unwrap();
        }
        let mut be = crate::types::backend_row_for_test("lab");
        be.base_url = "http://10.0.0.9:8000".into();
        be.models = vec!["qwen3-30b".into()];
        let reports = resync_for_direct_change(&f.env, &f.store, &cfg, &be, true);
        assert_eq!(reports.len(), 3, "{reports:?}");
        for app in [ClientApp::Claude, ClientApp::Pi, ClientApp::Codex] {
            let files = evaluate_one(
                &f.env,
                &direct_endpoint(&f.store, &be),
                app,
                &Selection::load(&f.store, app),
            )
            .unwrap();
            let text: String = files
                .edits
                .iter()
                .map(|e| e.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            let written = std::fs::read_to_string(files.files[0].path.clone()).unwrap_or_default();
            let blob = format!("{text}{written}");
            assert!(
                blob.contains("qwen3-30b"),
                "{app:?} 直连后应写本后端模型：{blob}"
            );
            assert!(
                !blob.contains("foreign"),
                "{app:?} 直连后不得残留别家模型：{blob}"
            );
        }
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    fn selection_roundtrips_through_settings() {
        let store = Store::in_memory().unwrap();
        let sel = Selection {
            default: Some("qwen-mini".into()),
            tiers: [("haiku", "qwen-mini"), ("opus", "big-one")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            ..Default::default()
        };
        sel.save(&store, ClientApp::Claude).unwrap();
        assert_eq!(Selection::load(&store, ClientApp::Claude), sel);
        // 另一端不受影响
        assert!(Selection::load(&store, ClientApp::Codex).is_empty());
    }

    #[test]
    fn served_models_respects_exposure_switch() {
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "b".into(),
                base_url: "http://x".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                weight: 1,
                models: vec!["show".into(), "hide".into()],
                ..Default::default()
            })
            .unwrap();
        // 默认全暴露
        assert_eq!(
            served_models(&store),
            vec!["hide".to_string(), "show".to_string()]
        );
        // 关掉一个：客户端清单里就不该再有它
        store
            .set_model_exposed("b", "hide", false)
            .expect("set_model_exposed 需要先就位");
        assert_eq!(served_models(&store), vec!["show".to_string()]);
    }

    /// 接管下拉的后端分组：独占模型归自家组；同名共享模型进 `a + b` 组合组，
    /// 每个模型恰好出现一次（optgroup 值不歧义）；停用后端不成组。
    #[test]
    fn option_groups_partition_models_by_backend() {
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "b".into(),
                base_url: "http://x".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                weight: 1,
                models: vec!["own-b".into(), "shared".into()],
                ..Default::default()
            })
            .unwrap();
        store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "c".into(),
                base_url: "http://y".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                weight: 1,
                models: vec!["own-c".into(), "shared".into()],
                ..Default::default()
            })
            .unwrap();
        store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "z-off".into(),
                base_url: "http://z".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: false,
                weight: 1,
                models: vec!["gone".into()],
                ..Default::default()
            })
            .unwrap();
        assert_eq!(
            option_groups(&store),
            vec![
                ("b".to_string(), vec!["own-b".to_string()]),
                ("b + c".to_string(), vec!["shared".to_string()]),
                ("c".to_string(), vec!["own-c".to_string()]),
            ]
        );
    }

    /// 刷新摘下的游离行（开关还开着、别名还在）绝不进客户端清单——否则写进
    /// opencode/pi 配置就是一个必然 404 的死模型（摘挂不销毁的封条测试）。
    #[test]
    fn served_models_skips_parked_models() {
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "b".into(),
                base_url: "http://x".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                weight: 1,
                models: vec!["show".into(), "parked".into()],
                ..Default::default()
            })
            .unwrap();
        // 上游只报出 show → parked 摘下（行保留、开关仍开）。
        store
            .sync_backend_models(
                "b",
                &[crate::types::ScannedModel {
                    id: "show".into(),
                    ctx: None,
                    output: None,
                    reasoning: None,
                    vlm: None,
                }],
            )
            .unwrap();
        let row = store.list_backends().unwrap().remove(0);
        assert!(row
            .model_flags
            .iter()
            .any(|f| f.public_name == "parked" && f.exposed));
        // 游离行被列排除：清单只剩 show。
        assert_eq!(served_models(&store), vec!["show".to_string()]);
        // 挂回（parked 上游恢复）→ 自动重回客户端清单。
        store
            .sync_backend_models(
                "b",
                &[
                    crate::types::ScannedModel {
                        id: "show".into(),
                        ctx: None,
                        output: None,
                        reasoning: None,
                        vlm: None,
                    },
                    crate::types::ScannedModel {
                        id: "parked".into(),
                        ctx: None,
                        output: None,
                        reasoning: None,
                        vlm: None,
                    },
                ],
            )
            .unwrap();
        assert_eq!(
            served_models(&store),
            vec!["parked".to_string(), "show".to_string()]
        );
    }

    /// 模型 id → 槽位显示名：剥掉尾部量化/格式段，仍过长再截断（可截短的有意义前缀）。
    #[test]
    fn model_prefix_strips_format_tails_and_truncates() {
        assert_eq!(model_prefix("ornith-35b-oq6e-mtp"), "ornith-35b");
        assert_eq!(model_prefix("ornith-9b-mlx-8bit"), "ornith-9b");
        assert_eq!(model_prefix("qwen3.8-flash-next"), "qwen3.8-flash");
        assert_eq!(model_prefix("qwen3.6-35b-oq4-mtp"), "qwen3.6-35b");
        // 没有可剥的尾巴就不动
        assert_eq!(model_prefix("m-one"), "m-one");
        // 超长（按字符数，不切坏 UTF-8）：截到 16 字符 + 省略号
        let long = "这是一个特别长的模型名字要截断到十六个字符为止";
        let got = model_prefix(long);
        assert_eq!(got.chars().count(), 17, "{got}");
        assert!(got.ends_with('…'));
    }

    /// Selection 的 pi 槽位字段经 settings 序列化往返不丢；is_empty 认得它们。
    #[test]
    fn pi_slot_fields_roundtrip_and_count_as_non_empty() {
        let store = Store::in_memory().unwrap();
        let sel = Selection {
            pi_main: Some("ornith-35b".into()),
            pi_small: Some("ornith-9b".into()),
            manage_pi: true,
            ..Default::default()
        };
        sel.save(&store, ClientApp::Pi).unwrap();
        let back = Selection::load(&store, ClientApp::Pi);
        assert_eq!(back.pi_main.as_deref(), Some("ornith-35b"));
        assert_eq!(back.pi_small.as_deref(), Some("ornith-9b"));
        assert!(back.manage_pi);
        assert!(!back.is_empty(), "只设了 pi 槽位也不算空");
        assert!(Selection::default().is_empty());
    }

    /// 面板只送动过的槽：没送的槽用文件真值回填（改小槽不洗主槽）；
    /// 键送过（含空串 = 显式清空）就不回填，None 保持。
    #[test]
    fn merge_untouched_slots_keeps_truth_only_for_untouched() {
        let touched: std::collections::BTreeSet<String> =
            ["small".to_string()].into_iter().collect();
        let truth = Selection {
            pi_main: Some("m-main".into()),
            pi_small: Some("m-old".into()),
            ..Default::default()
        };
        // 只动 small：main 没送 → 回填文件真值，不被洗掉。
        let mut sel = Selection {
            pi_small: Some("m-new".into()),
            manage_pi: true,
            ..Default::default()
        };
        merge_untouched_slots(&mut sel, &truth, &touched);
        assert_eq!(sel.pi_main.as_deref(), Some("m-main"));
        assert_eq!(sel.pi_small.as_deref(), Some("m-new"));

        // main 键送过且为空 = 显式清空：不回填。
        let touched2: std::collections::BTreeSet<String> =
            ["main".to_string(), "small".to_string()]
                .into_iter()
                .collect();
        let mut sel2 = Selection {
            pi_small: Some("m-new".into()),
            manage_pi: true,
            ..Default::default()
        };
        merge_untouched_slots(&mut sel2, &truth, &touched2);
        assert_eq!(sel2.pi_main, None, "显式清空不能被回填");

        // manage_pi 为假时不掺和（set_model 的整槽兜底分支负责）。
        let mut sel3 = Selection::default();
        merge_untouched_slots(&mut sel3, &truth, &touched);
        assert_eq!(sel3.pi_main, None);
    }

    /// Selection 的 codex 副槽字段同理：序列化往返不丢；is_empty 认得它。
    #[test]
    fn codex_sub_slot_roundtrips_and_counts_as_non_empty() {
        let store = Store::in_memory().unwrap();
        let sel = Selection {
            codex_sub: Some("qwen3.8-flash-next".into()),
            manage_codex: true,
            ..Default::default()
        };
        sel.save(&store, ClientApp::Codex).unwrap();
        let back = Selection::load(&store, ClientApp::Codex);
        assert_eq!(back.codex_sub.as_deref(), Some("qwen3.8-flash-next"));
        assert!(back.manage_codex);
        assert!(!back.is_empty(), "只设了 codex 副槽也不算空");
        assert_eq!(
            back.codex_slots(),
            [("main", None), ("sub", Some("qwen3.8-flash-next"))]
        );
    }

    /// 直连 Key 环境变量名派生（计划决策 3）：大写、非字母数字全转下划线。
    #[test]
    fn direct_key_var_derives_from_backend_name() {
        assert_eq!(
            Endpoint::direct_key_var("halogen"),
            "LLM_SWITCH_HALOGEN_KEY"
        );
        assert_eq!(Endpoint::direct_key_var("my lab"), "LLM_SWITCH_MY_LAB_KEY");
        assert_eq!(Endpoint::direct_key_var("my-lab"), "LLM_SWITCH_MY_LAB_KEY");
        // 中文与空格全转下划线：5 个字符 → 5 条下划线（用 repeat 拼期望，不靠肉眼数）。
        assert_eq!(
            Endpoint::direct_key_var("本地 模型"),
            format!("LLM_SWITCH_{}_KEY", "_".repeat(5))
        );
        assert_eq!(
            Endpoint::direct_key_var("MixCase-2"),
            "LLM_SWITCH_MIXCASE_2_KEY"
        );
    }
}
