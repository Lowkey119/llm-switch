//! SQLite access layer. One pooled connection behind a mutex is plenty for a
//! single-user desktop proxy; hot paths (usage writes) batch at the caller.

pub mod schema;

use crate::types::{
    now_ms, BackendRow, BackendUpsert, DailyRollup, EffectiveMeta, LogQuery, LogRow, MediaUsageRow,
    MetaSource, ParamOverride, PromptItem, ScannedModel, SkillAppRow, SkillRecord, SkillRepoRow,
    UsageRecord, UsageRecordQuery, UsageSummary,
};
use rusqlite::{params, types::Value as SqlValue, Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid input: {0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// 摘掉模型时顺带清掉了多少条配置（要说得清，别默默删）。
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct RemoveOutcome {
    pub dropped_params: usize,
    pub dropped_cache_policies: usize,
    /// 连带清掉的全局元数据声明（0 = 别的后端还在用，保留）。
    pub dropped_meta: usize,
    /// UI 重设计决策：手动删掉最后一个模型 → 后端连带删除。
    pub backend_deleted: bool,
}

/// 删后端时顺带清掉了多少条配置（口径同 [`RemoveOutcome`]：默默删是不行的）。
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct DeleteOutcome {
    /// 后端层参数覆盖（`scope='backend'`，ref_id = 后端名）。
    pub dropped_params: usize,
    /// 连带清掉的全局元数据声明（独占模型随之消失；0 = 别的后端还在用）。
    pub dropped_meta: usize,
}

/// 刷新（探活+重扫）的 diff 战报：新增/移除/未变。
///
/// 「移除」只把模型从对外清单（`backends.models`）**摘下**，不销毁任何东西：
/// 行、别名、参数覆盖、缓存策略、元数据声明全部保留；上游恢复该模型后按友好名
/// 自动挂回，配置与开关随行复活（见 `sync_backend_models`）。实报 bug：上游
/// 列表抖一次（如引擎重启中返回不完整清单）就把用户配置永久删了。
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct SyncOutcome {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub kept: usize,
}

/// 改名的战果报告（迁移了几条配置要说得清，不能默默搬完）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct RenameOutcome {
    pub public_name: String,
    pub upstream_name: String,
    pub migrated_params: usize,
    pub migrated_cache_policies: usize,
}

/// 写入声明的战果（钳过界要把实际值与上限一并告诉前端提示）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelMetaSet {
    /// 实际落库值（0 = 自动，行已删）。
    pub ctx: i64,
    pub output: i64,
    /// 三态声明落库值：0 = 自动，1 = 强制开，2 = 强制关。
    pub reasoning: i64,
    pub vlm: i64,
    /// 后端允许范围（None = 后端侧无从得知，未拦）。
    pub cap_ctx: Option<i64>,
    pub cap_output: Option<i64>,
    /// 超界被钳（提示用）。
    pub clamped_ctx: bool,
    pub clamped_output: bool,
}

/// ctx 先验 TTL 缓存条目：(生成时刻, 最近真报输入)；None = 库里没有，TTL 内不再查。
type InPriorEntry = (std::time::Instant, Option<i64>);

pub struct Store {
    conn: Mutex<Connection>,
    /// 有效元数据的 TTL 缓存（转发面快路径；配置面变更主动失效，见
    /// [`Self::effective_model_meta_cached`]）。
    meta_cache: Mutex<
        Option<(
            std::time::Instant,
            std::collections::BTreeMap<String, EffectiveMeta>,
        )>,
    >,
    /// (backend, model) → 最近真报输入的 TTL 缓存（ctx 先验快路径，见
    /// [`Self::last_real_input_cached`]）。
    in_prior_cache: Mutex<std::collections::HashMap<(String, String), InPriorEntry>>,
}

/// 策略表复合 scope 拆解："global" → (global, "")；"model:x"/"backend:x" → (层, 名)。
fn split_policy_scope(scope: &str) -> (&str, String) {
    if let Some(rest) = scope.strip_prefix("model:") {
        return ("model", rest.to_string());
    }
    if let Some(rest) = scope.strip_prefix("backend:") {
        return ("backend", rest.to_string());
    }
    ("global", String::new())
}

/// 参数覆盖存的数值：裸数字，或 clamp 形状 `{"v":..,"min":..,"max":..}`；非数值 = 未设置。
fn param_number(raw: &str) -> Option<i64> {
    let v: Value = serde_json::from_str(raw).ok()?;
    match v {
        Value::Number(n) => n.as_i64(),
        Value::Object(_) => v.get("v").and_then(Value::as_i64),
        _ => None,
    }
}

/// 已知值里取最小（都不认识 = None）。
fn min_known(vals: [Option<i64>; 2]) -> Option<i64> {
    vals.into_iter().flatten().min()
}

/// 区间内单客户端聚合行。
#[derive(Debug, Clone)]
pub struct ClientAgg {
    pub client: String,
    pub requests: i64,
    pub input: i64,
    pub output: i64,
}

/// 区间内单模型的 token 聚合行（计价输入）。
///
/// 片4：按（模型 × 后端引擎）分组——同一模型可能同时被本地与在线后端服务，
/// 分档不同必须拆行。`engine` = 提供该请求的后端引擎（NULL = 后端未填/已删）。
#[derive(Debug, Clone)]
pub struct ModelAgg {
    pub model: String,
    pub engine: Option<String>,
    pub requests: i64,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_creation: i64,
}

/// 列表接口回显的掩码；编辑时原样送回 = 保持库里已有的秘密不动。
const MASK: &str = "***";

/// copilot（Ollama 门面下游）可见清单的 settings 键（`None` = 缺省全暴露）。
/// 门面读取、矩阵写入、面板命令共用这一份，别再各写字符串。
pub(crate) const COPILOT_ALLOW_KEY: &str = "copilot.allow";

/// 单个「后端 × 模型」的探测事实：ctx / output / reasoning / vlm（None = 上游没说）。
type ProbeFacts = (Option<i64>, Option<i64>, Option<bool>, Option<bool>);

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        schema::migrate(&conn)?;
        // 自愈（2026-11-20）：停用后端清灯修复之前的旧库可能留着假 healthy
        // （停用后周期探活不再跑，绿灯永远挂着），启动时统一归 unknown。
        conn.execute(
            "UPDATE backends SET health_state = 'unknown', last_error = NULL
             WHERE enabled = 0 AND health_state != 'unknown'",
            [],
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            meta_cache: Mutex::new(None),
            in_prior_cache: Mutex::new(std::collections::HashMap::new()),
        })
    }

    // ---------------------------------------------------------------- backends

    pub fn upsert_backend(&self, b: &BackendUpsert) -> Result<i64> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let now = now_ms();
        let models = serde_json::to_string(&b.models)?;
        let aliases = b.aliases.to_string();
        let caps = b.capabilities.to_string();
        let params_json = b.params.to_string();

        let existing: Option<(i64, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT id, api_key, webui_password FROM backends WHERE name = ?1",
                params![b.name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;

        // 掩码哨兵：列表接口只会回显 "***"，编辑表单原样送回就表示「这条秘密别动」——
        // 否则编辑一次就把用户存的 API Key / 控制台密码洗成一串星号。
        let prev_key = existing.as_ref().and_then(|(_, k, _)| k.clone());
        let prev_pass = existing.as_ref().and_then(|(_, _, p)| p.clone());
        let api_key = match b.api_key.as_deref() {
            Some(MASK) => prev_key,
            other => other.map(str::to_string),
        };
        let webui_password = match b.webui_password.as_deref() {
            Some(MASK) => prev_pass,
            other => other.map(str::to_string),
        };

        let id = match existing {
            Some((id, _, _)) => {
                conn.execute(
                    "UPDATE backends SET base_url=?1, protocol=?2, api_key=?3, weight=?4, enabled=?5,
                        engine=?6, webui_url=?7, health_path=?8, models=?9, aliases=?10,
                        capabilities=?11, params=?12, webui_user=?13, webui_password=?14,
                        updated_at=?15
                     WHERE id=?16",
                    params![
                        b.base_url,
                        b.protocol.as_str(),
                        api_key,
                        b.weight,
                        b.enabled as i64,
                        b.engine,
                        b.webui_url,
                        b.health_path,
                        models,
                        aliases,
                        caps,
                        params_json,
                        b.webui_user.as_deref(),
                        webui_password.as_deref(),
                        now,
                        id
                    ],
                )?;
                id
            }
            None => {
                conn.execute(
                    "INSERT INTO backends (name, base_url, protocol, api_key, weight, enabled, engine,
                        webui_url, health_path, models, aliases, capabilities, params,
                        webui_user, webui_password, created_at, updated_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
                    params![
                        b.name,
                        b.base_url,
                        b.protocol.as_str(),
                        api_key,
                        b.weight,
                        b.enabled as i64,
                        b.engine,
                        b.webui_url,
                        b.health_path,
                        models,
                        aliases,
                        caps,
                        params_json,
                        b.webui_user.as_deref(),
                        webui_password.as_deref(),
                        now,
                        now
                    ],
                )?;
                conn.last_insert_rowid()
            }
        };

        // 模型表增量同步（M12-A）：按 (backend_id, public_name) 更新，**保留 enabled
        // 与 accept_upstream**。旧实现是「整表删掉重插」，于是每次启动的 yaml 种子
        // 都会把用户关掉的模型又打开——开关形同虚设。
        for name in &b.models {
            let upstream = b
                .aliases
                .as_object()
                .and_then(|m| m.get(name))
                .and_then(|v| v.as_str())
                .unwrap_or(name);
            conn.execute(
                "INSERT INTO backend_models (backend_id, public_name, upstream_name, enabled, accept_upstream)
                 VALUES (?1, ?2, ?3, 1, 0)
                 ON CONFLICT(backend_id, public_name)
                 DO UPDATE SET upstream_name = excluded.upstream_name",
                params![id, name, upstream],
            )?;
        }
        // 列表外的行**不删**（摘挂不销毁）：`models` 列即服务真值，行留下只是
        // 游离暂存——暴露开关、探测值、参数覆盖随行保留，编辑/刷新时摘掉的模型
        // 重新出现在清单里即经 ON CONFLICT 整套复活。改名走 rename_model，不经这条路。
        self.invalidate_meta_cache();
        Ok(id)
    }

    /// 控制台反代按 id 解析目标后端。
    pub fn backend_by_id(&self, id: i64) -> Result<Option<BackendRow>> {
        let all = self.list_backends()?;
        Ok(all.into_iter().find(|b| b.id == id))
    }

    /// 按名取单个后端。保存/导入后端时用它取回那几列**只有探测、引擎预设与
    /// yaml 种子会写、前端表单里根本没有**的字段（health_path / capabilities /
    /// params），否则整行 UPDATE 会把它们洗成 NULL——见 `commands/backend.rs`。
    pub fn backend_by_name(&self, name: &str) -> Result<Option<BackendRow>> {
        let all = self.list_backends()?;
        Ok(all.into_iter().find(|b| b.name == name))
    }

    /// 就地更新控制台入口配置（URL / 凭据 / 启停脚本，空串按 NULL 存），不触碰其余列。
    ///
    /// 掩码哨兵：编辑表单只回显 `"***"`，原样送回表示保持既有密码不动——否则编辑
    /// 一次地址就把用户存的密码洗成一串星号。脚本路径没有掩码概念（前端回显明文），
    /// 留空即清除。三样东西同属控制台页那张「入口设置」卡，故一次写完。
    pub fn set_backend_console(
        &self,
        name: &str,
        webui_url: Option<&str>,
        webui_user: Option<&str>,
        webui_password: Option<&str>,
        start_script: Option<&str>,
        stop_script: Option<&str>,
    ) -> Result<bool> {
        let norm = |v: Option<&str>| {
            v.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let conn = self.conn.lock().expect("store lock poisoned");
        // 掩码哨兵要「保持既有密码」就得先读回来再写回（同一把锁内，读写之间无并发写）。
        let keep_pass = webui_password == Some("***");
        let webui_password = if keep_pass {
            conn.query_row(
                "SELECT webui_password FROM backends WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()?
        } else {
            norm(webui_password)
        };
        let n = conn.execute(
            "UPDATE backends SET webui_url=?1, webui_user=?2, webui_password=?3,
                    start_script=?4, stop_script=?5, updated_at=?6
             WHERE name=?7",
            params![
                norm(webui_url),
                norm(webui_user),
                webui_password,
                norm(start_script),
                norm(stop_script),
                crate::types::now_ms(),
                name
            ],
        )?;
        Ok(n > 0)
    }

    /// 引擎切换落库：`engine` 与 `alt_engine` 互换 + capabilities 重写为目标引擎预设。
    /// capabilities 列存的是「声明增量」（生效集 = engine 预设 ⊕ 声明，见 gateway::capability），
    /// 配对后端的声明值往往是播种时整份写入的源引擎预设——不重写的话切到 gufo 后
    /// halogen 的 ngram 组会被声明并回来，能力口径就不跟切换了。
    pub fn swap_backend_engine(
        &self,
        name: &str,
        new_engine: &str,
        new_alt: Option<&str>,
        new_caps: &Value,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let n = conn.execute(
            "UPDATE backends SET engine=?1, alt_engine=?2, capabilities=?3, updated_at=?4
             WHERE name=?5",
            params![new_engine, new_alt, new_caps.to_string(), now_ms(), name],
        )?;
        Ok(n > 0)
    }

    pub fn list_backends(&self) -> Result<Vec<BackendRow>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, base_url, protocol, api_key, weight, enabled, engine, webui_url,
                    health_path, models, aliases, capabilities, params, health_state,
                    consecutive_failures, last_checked_at, last_error, webui_user, webui_password,
                    start_script, stop_script, alt_engine, direct
             FROM backends ORDER BY name",
        )?;
        let rows = stmt.query_map([], |r| {
            let models: String = r.get(10)?;
            let aliases: String = r.get(11)?;
            let caps: String = r.get(12)?;
            let params_json: String = r.get(13)?;
            let name: String = r.get(1)?;
            Ok(BackendRow {
                id: r.get(0)?,
                name: name.clone(),
                base_url: r.get(2)?,
                protocol: r.get(3)?,
                api_key: r.get(4)?,
                weight: r.get(5)?,
                enabled: r.get::<_, i64>(6)? != 0,
                direct: r.get::<_, i64>(23)? != 0,
                key_var: crate::takeover::Endpoint::direct_key_var(&name),
                engine: r.get(7)?,
                webui_url: r.get(8)?,
                health_path: r.get(9)?,
                models: serde_json::from_str(&models).unwrap_or_default(),
                aliases: serde_json::from_str(&aliases).unwrap_or(Value::Null),
                model_flags: Vec::new(),
                capabilities: serde_json::from_str(&caps).unwrap_or(Value::Null),
                params: serde_json::from_str(&params_json).unwrap_or(Value::Null),
                health_state: r.get(14)?,
                consecutive_failures: r.get(15)?,
                last_checked_at: r.get(16)?,
                last_error: r.get(17)?,
                webui_user: r.get(18)?,
                webui_password: r.get(19)?,
                start_script: r.get(20)?,
                stop_script: r.get(21)?,
                alt_engine: r.get(22)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        // 一次取全部模型开关再按后端归组（每请求都会走这里，别按后端 N+1 查）。
        let mut stmt = conn.prepare(
            "SELECT backend_id, public_name, upstream_name, enabled, accept_upstream, kind
             FROM backend_models ORDER BY backend_id, public_name",
        )?;
        let mut by_backend: std::collections::HashMap<i64, Vec<crate::types::ModelFlag>> =
            Default::default();
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                crate::types::ModelFlag {
                    public_name: r.get(1)?,
                    upstream_name: r.get(2)?,
                    exposed: r.get::<_, i64>(3)? != 0,
                    accept_upstream: r.get::<_, i64>(4)? != 0,
                    kind: r.get::<_, Option<String>>(5)?,
                },
            ))
        })?;
        for row in rows {
            let (bid, flag) = row?;
            by_backend.entry(bid).or_default().push(flag);
        }
        for b in out.iter_mut() {
            b.model_flags = by_backend.remove(&b.id).unwrap_or_default();
        }
        Ok(out)
    }

    /// 自动探测回填：仅当 engine 为空（用户未指定）时写入 engine + 能力声明。
    pub fn fill_backend_engine(
        &self,
        name: &str,
        engine: &str,
        capabilities: &serde_json::Value,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let n = conn.execute(
            "UPDATE backends SET engine = ?2, capabilities = ?3 WHERE name = ?1 AND engine IS NULL",
            params![name, engine, capabilities.to_string()],
        )?;
        Ok(n > 0)
    }

    pub fn set_backend_enabled(&self, name: &str, enabled: bool) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let changed = conn.execute(
            "UPDATE backends SET enabled = ?1, updated_at = ?2 WHERE name = ?3",
            params![enabled as i64, now_ms(), name],
        )?;
        if !enabled {
            // 停用即清健康灯：周期探活只跑启用后端，不清的话启用时留下的
            // healthy 会永远挂着说谎（真机实锤：gufo 停用后同地址由 halogen
            // 应答，绿灯不灭）。
            conn.execute(
                "UPDATE backends SET health_state = 'unknown', consecutive_failures = 0, last_error = NULL WHERE name = ?1",
                params![name],
            )?;
        }
        self.invalidate_meta_cache();
        Ok(changed > 0)
    }

    /// 直连开关（三态里的 网关/直连 两档；停用走 `set_backend_enabled`，不双轨）。
    pub fn set_backend_direct(&self, name: &str, direct: bool) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let changed = conn.execute(
            "UPDATE backends SET direct = ?1, updated_at = ?2 WHERE name = ?3",
            params![direct as i64, now_ms(), name],
        )?;
        self.invalidate_meta_cache();
        Ok(changed > 0)
    }

    /// 改「后端 × 暴露名」的对外开关（M12-A）。关掉 = 对客户端彻底隐身。
    pub fn set_model_exposed(
        &self,
        backend: &str,
        public_name: &str,
        exposed: bool,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let changed = conn.execute(
            "UPDATE backend_models SET enabled = ?1
             WHERE public_name = ?2
               AND backend_id = (SELECT id FROM backends WHERE name = ?3)",
            params![exposed as i64, public_name, backend],
        )?;
        self.invalidate_meta_cache();
        Ok(changed > 0)
    }

    /// 改名后的过渡开关：是否**同时**接受上游原名（客户端配置一时改不过来时用）。
    pub fn set_model_accept_upstream(
        &self,
        backend: &str,
        public_name: &str,
        accept: bool,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let changed = conn.execute(
            "UPDATE backend_models SET accept_upstream = ?1
             WHERE public_name = ?2
               AND backend_id = (SELECT id FROM backends WHERE name = ?3)",
            params![accept as i64, public_name, backend],
        )?;
        Ok(changed > 0)
    }

    /// 工种人工声明（11a）：`Some(kind)` = 强制归该工种；`None` = 清除声明，
    /// 回落判定链（/health > 名字建议 > chat）。工种字符串合法性由调用方
    /// （commands 层经 `model_kind::parse`）把关，这里只存。
    pub fn set_model_kind(
        &self,
        backend: &str,
        public_name: &str,
        kind: Option<&str>,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let changed = conn.execute(
            "UPDATE backend_models SET kind = ?1
             WHERE public_name = ?2
               AND backend_id = (SELECT id FROM backends WHERE name = ?3)",
            params![kind, public_name, backend],
        )?;
        Ok(changed > 0)
    }

    // ------------------------------------------------------- 模型元数据（ctx/output）

    /// 全量解析模型元数据（`/v1/models`、UI、接管写入共用这一份口径）。
    ///
    /// 分层规则：
    /// · 每个启用后端的设定 = min(后端层同键参数, 该后端探测值)，只认已知的一侧；
    /// · 跨后端取最小 = 该模型的「后端允许范围」（cap）；
    /// · 人工非 0 声明优先，但按 cap 钳到边界（决策：设置不得超出后端允许范围）；
    /// · 自动（0/缺省）直接用 cap；两边都无 = None（未知不写不猜）。
    pub fn effective_model_meta(
        &self,
    ) -> Result<std::collections::BTreeMap<String, EffectiveMeta>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut out: std::collections::BTreeMap<String, EffectiveMeta> = Default::default();

        // ① 人工声明（行存在 ⇔ 任一字段写了非默认值）。
        let mut manual: std::collections::BTreeMap<String, (i64, i64, i64, i64)> =
            Default::default();
        {
            let mut stmt =
                conn.prepare("SELECT public_name, ctx, output, reasoning, vlm FROM model_meta")?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?;
            for row in rows {
                let (m, c, o, re, vl) = row?;
                manual.insert(m, (c, o, re, vl));
            }
        }

        // ② 启用后端的模型清单（服务关系的真值 = backends.models）。
        let mut backends: Vec<(String, std::collections::BTreeSet<String>)> = Vec::new();
        {
            let mut stmt = conn.prepare("SELECT name, models FROM backends WHERE enabled = 1")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (name, raw) = row?;
                let list: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
                backends.push((name, list.into_iter().collect()));
            }
        }

        // ③ 后端层参数（ctx / max_tokens；0 或无效形状 = 未设置，继承语义）。
        let mut params: std::collections::HashMap<String, [Option<i64>; 2]> = Default::default();
        {
            let mut stmt = conn.prepare(
                "SELECT ref_id, key, value FROM param_overrides
                 WHERE scope = 'backend' AND key IN ('ctx', 'max_tokens')",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (backend, key, raw) = row?;
                let idx = if key == "ctx" { 0 } else { 1 };
                let slot = params.entry(backend).or_default();
                if slot[idx].is_none() {
                    slot[idx] = param_number(&raw).filter(|v| *v > 0);
                }
            }
        }

        // ④ 扫描探测值：只认启用后端上开着的模型行。
        let mut probes: std::collections::HashMap<(String, String), ProbeFacts> =
            Default::default();
        {
            let mut stmt = conn.prepare(
                "SELECT b.name, bm.public_name, bm.probe_ctx, bm.probe_output,
                        bm.probe_reasoning, bm.probe_vlm
                 FROM backend_models bm JOIN backends b ON b.id = bm.backend_id
                 WHERE b.enabled = 1 AND bm.enabled = 1",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, Option<i64>>(5)?,
                ))
            })?;
            for row in rows {
                let (b, m, c, o, re, vl) = row?;
                probes.insert(
                    (b, m),
                    (
                        c.filter(|v| *v > 0),
                        o.filter(|v| *v > 0),
                        re.map(|v| v != 0),
                        vl.map(|v| v != 0),
                    ),
                );
            }
        }

        // ⑤ 逐后端取小 → 跨后端取小 = cap；布尔能力位跨后端折叠（任一后端明说关 = 关，
        //    否则有明说开 = 开，全未知 = 未知；量化裁剪会丢视觉，保守方向同 ctx 取小）。
        let mut cap_ctx: std::collections::BTreeMap<String, i64> = Default::default();
        let mut cap_out: std::collections::BTreeMap<String, i64> = Default::default();
        let mut cap_re: std::collections::BTreeMap<String, bool> = Default::default();
        let mut cap_vl: std::collections::BTreeMap<String, bool> = Default::default();
        let fold_bool = |cap: &mut std::collections::BTreeMap<String, bool>, m: &str, v: bool| {
            cap.entry(m.to_string())
                .and_modify(|cur| *cur = *cur && v)
                .or_insert(v);
        };
        for (bname, models) in &backends {
            let slot = params.get(bname);
            for m in models {
                let probe = probes.get(&(bname.clone(), m.clone()));
                if let Some(v) = min_known([slot.and_then(|s| s[0]), probe.and_then(|p| p.0)]) {
                    cap_ctx
                        .entry(m.clone())
                        .and_modify(|cur| *cur = (*cur).min(v))
                        .or_insert(v);
                }
                if let Some(v) = min_known([slot.and_then(|s| s[1]), probe.and_then(|p| p.1)]) {
                    cap_out
                        .entry(m.clone())
                        .and_modify(|cur| *cur = (*cur).min(v))
                        .or_insert(v);
                }
                if let Some(v) = probe.and_then(|p| p.2) {
                    fold_bool(&mut cap_re, m, v);
                }
                if let Some(v) = probe.and_then(|p| p.3) {
                    fold_bool(&mut cap_vl, m, v);
                }
            }
        }

        // ⑥ 拼装：人工声明钳到 cap，自动用 cap，无则未知。
        let names: std::collections::BTreeSet<&str> = manual
            .keys()
            .map(String::as_str)
            .chain(cap_ctx.keys().map(String::as_str))
            .chain(cap_out.keys().map(String::as_str))
            .chain(cap_re.keys().map(String::as_str))
            .chain(cap_vl.keys().map(String::as_str))
            .collect();
        // 三态解析：人工 1/2 压倒一切；自动（0）用探测折叠值；全未知 = 不写。
        let resolve_bool = |man: i64, cap: Option<bool>| -> (Option<bool>, MetaSource) {
            match man {
                1 => (Some(true), MetaSource::Manual),
                2 => (Some(false), MetaSource::Manual),
                _ => match cap {
                    Some(v) => (Some(v), MetaSource::Backend),
                    None => (None, MetaSource::Unknown),
                },
            }
        };
        for m in names {
            let (man_ctx, man_out, man_re, man_vl) = manual.get(m).copied().unwrap_or((0, 0, 0, 0));
            let cap_c = cap_ctx.get(m).copied();
            let cap_o = cap_out.get(m).copied();
            let (ctx, ctx_source) = if man_ctx > 0 {
                (
                    Some(cap_c.map_or(man_ctx, |c| man_ctx.min(c))),
                    MetaSource::Manual,
                )
            } else if cap_c.is_some() {
                (cap_c, MetaSource::Backend)
            } else {
                (None, MetaSource::Unknown)
            };
            let (output, output_source) = if man_out > 0 {
                (
                    Some(cap_o.map_or(man_out, |c| man_out.min(c))),
                    MetaSource::Manual,
                )
            } else if cap_o.is_some() {
                (cap_o, MetaSource::Backend)
            } else {
                (None, MetaSource::Unknown)
            };
            let (reasoning, reasoning_source) = resolve_bool(man_re, cap_re.get(m).copied());
            let (vlm, vlm_source) = resolve_bool(man_vl, cap_vl.get(m).copied());
            out.insert(
                m.to_string(),
                EffectiveMeta {
                    ctx,
                    output,
                    ctx_source,
                    output_source,
                    reasoning,
                    reasoning_source,
                    vlm,
                    vlm_source,
                    manual_ctx: man_ctx,
                    manual_output: man_out,
                    manual_reasoning: man_re,
                    manual_vlm: man_vl,
                    cap_ctx: cap_c,
                    cap_output: cap_o,
                },
            );
        }
        Ok(out)
    }

    /// 转发面快路径：TTL 内的有效元数据（VLM 剥图判定、`/v1/models` 等高频消费共用）。
    ///
    /// 失效策略双保险：配置面的每个写路径（人工改参 / 探测落值 / 清单与启停变更）
    /// **主动清缓存**，即时生效；TTL 只兜「未来新增写路径忘了挂钩子」的底。
    /// 锁序纪律：本方法从不持 meta_cache 去拿 conn（先算后写缓存），不会互锁。
    pub fn effective_model_meta_cached(
        &self,
    ) -> Result<std::collections::BTreeMap<String, EffectiveMeta>> {
        const TTL: std::time::Duration = std::time::Duration::from_secs(5);
        {
            let guard = self.meta_cache.lock().expect("meta cache lock poisoned");
            if let Some((born, map)) = guard.as_ref() {
                if born.elapsed() < TTL {
                    return Ok(map.clone());
                }
            }
        }
        let fresh = self.effective_model_meta()?;
        *self.meta_cache.lock().expect("meta cache lock poisoned") =
            Some((std::time::Instant::now(), fresh.clone()));
        Ok(fresh)
    }

    /// 配置面变更时失效元数据缓存（与上面的 TTL 互为双保险）。
    pub fn invalidate_meta_cache(&self) {
        *self.meta_cache.lock().expect("meta cache lock poisoned") = None;
    }

    /// ctx 先验快路径（BUG-8 三修）：该 (backend, model) 最近一次**上游真报**的输入 token。
    ///
    /// `metrics.last_real_in` 是纯内存态，llm-switch 一重启就清零，冷启动头几个请求
    /// 只能退回字节启发式（对整会话重发的 JSON 载荷系统性虚高，估超窗口就画出假满窗）。
    /// 持久真值一直在 `usage_records` 里：`estimated = 0` 的行是上游 usage 原话，不是我们估的。
    /// 速率浮条每秒轮询，不能每流查库 → 5s TTL 缓存（照 [`Self::effective_model_meta_cached`]
    /// 范式：先算后写缓存，从不持缓存锁去拿 conn，不会互锁）。查不到/查失败都缓存 None，
    /// TTL 内不再重复查——「没有」也是一种要缓存的答案。
    pub fn last_real_input_cached(&self, backend: &str, model: &str) -> Option<u64> {
        const TTL: std::time::Duration = std::time::Duration::from_secs(5);
        let key = (backend.to_string(), model.to_string());
        {
            let guard = self
                .in_prior_cache
                .lock()
                .expect("prior cache lock poisoned");
            if let Some((born, v)) = guard.get(&key) {
                if born.elapsed() < TTL {
                    return v.map(|n| n as u64);
                }
            }
        }
        let fresh: Option<i64> = {
            let conn = self.conn.lock().expect("store lock poisoned");
            conn.query_row(
                "SELECT input_tokens FROM usage_records
                 WHERE backend = ?1 AND model = ?2 AND ok = 1 AND estimated = 0
                   AND input_tokens > 0
                 ORDER BY ts DESC LIMIT 1",
                params![backend, model],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
            .flatten()
        };
        let mut guard = self
            .in_prior_cache
            .lock()
            .expect("prior cache lock poisoned");
        // 键随 (backend, model) 组合增长，硬上限兜病态膨胀（清全表即可，TTL 本就秒级）。
        if guard.len() > 512 {
            guard.clear();
        }
        guard.insert(key, (std::time::Instant::now(), fresh));
        fresh.map(|n| n as u64)
    }

    /// 写人工声明：ctx/output 非 0 = 强制覆盖（超后端允许范围钳到边界，R6）；
    /// reasoning/vlm 三态：0 = 回自动（跟随探测），1 = 强制开，2 = 强制关。
    /// 四项全回默认 = 删行。
    pub fn set_model_meta(
        &self,
        public_name: &str,
        ctx: i64,
        output: i64,
        reasoning: i64,
        vlm: i64,
    ) -> Result<ModelMetaSet> {
        if ctx < 0 || output < 0 {
            return Err(StoreError::Invalid(
                "0 = 自动（按后端设定走），声明值必须是非负数".into(),
            ));
        }
        if ![0, 1, 2].contains(&reasoning) || ![0, 1, 2].contains(&vlm) {
            return Err(StoreError::Invalid(
                "reasoning/vlm 只能是 0（自动）/ 1（强制开）/ 2（强制关）".into(),
            ));
        }
        if !self.any_backend_serves(public_name)? {
            return Err(StoreError::Invalid(format!(
                "没有任何后端提供模型 `{public_name}`"
            )));
        }
        let cur = self.effective_model_meta()?.get(public_name).copied();
        let (cap_ctx, cap_output) = match cur {
            Some(m) => (m.cap_ctx, m.cap_output),
            None => (None, None),
        };
        let w_ctx = if ctx > 0 {
            cap_ctx.map_or(ctx, |c| ctx.min(c))
        } else {
            0
        };
        let w_output = if output > 0 {
            cap_output.map_or(output, |c| output.min(c))
        } else {
            0
        };
        let conn = self.conn.lock().expect("store lock poisoned");
        // 行存在条件：任一声明非默认，或挂了手工单价（价格也住在这张表里，不能连带删掉）。
        let has_price: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM model_meta WHERE public_name = ?1
                 AND (price_input IS NOT NULL OR price_output IS NOT NULL
                      OR price_cache_read IS NOT NULL OR price_cache_creation IS NOT NULL)",
                params![public_name],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if w_ctx == 0 && w_output == 0 && reasoning == 0 && vlm == 0 && !has_price {
            conn.execute(
                "DELETE FROM model_meta WHERE public_name = ?1",
                params![public_name],
            )?;
        } else {
            conn.execute(
                "INSERT INTO model_meta (public_name, ctx, output, reasoning, vlm, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(public_name)
                 DO UPDATE SET ctx = excluded.ctx, output = excluded.output,
                               reasoning = excluded.reasoning, vlm = excluded.vlm,
                               updated_at = excluded.updated_at",
                params![public_name, w_ctx, w_output, reasoning, vlm, now_ms()],
            )?;
        }
        self.invalidate_meta_cache();
        Ok(ModelMetaSet {
            ctx: w_ctx,
            output: w_output,
            reasoning,
            vlm,
            cap_ctx,
            cap_output,
            clamped_ctx: w_ctx != ctx,
            clamped_output: w_output != output,
        })
    }

    // ------------------------------------------------------- 模型手工单价（片4）

    /// 全量手工单价表：任一维非 NULL = 该模型已设价（USD / 百万 token）。
    ///
    /// 单价纯手工（D25）：没有内置定价表、没有前缀匹配，挂在哪就在哪生效；
    /// 只对在线后端（engine: custom/未填）的请求计价，本地引擎按分档直接跳过。
    pub fn model_prices(
        &self,
    ) -> Result<std::collections::BTreeMap<String, crate::services::pricing::Price>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT public_name, price_input, price_output, price_cache_read, price_cache_creation
             FROM model_meta
             WHERE price_input IS NOT NULL OR price_output IS NOT NULL
                OR price_cache_read IS NOT NULL OR price_cache_creation IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<f64>>(1)?,
                r.get::<_, Option<f64>>(2)?,
                r.get::<_, Option<f64>>(3)?,
                r.get::<_, Option<f64>>(4)?,
            ))
        })?;
        let mut out: std::collections::BTreeMap<String, crate::services::pricing::Price> =
            Default::default();
        for row in rows {
            let (m, i, o, cr, cc) = row?;
            out.insert(
                m,
                crate::services::pricing::Price {
                    input: i.unwrap_or(0.0),
                    output: o.unwrap_or(0.0),
                    cache_read: cr.unwrap_or(0.0),
                    cache_creation: cc.unwrap_or(0.0),
                },
            );
        }
        Ok(out)
    }

    /// 写/清手工单价：`Some` = 设价（任一维非 0，其余维补 0）；`None` = 清除。
    ///
    /// 清除后若其余声明也全回默认则删行（与 `set_model_meta` 的行存在语义一致）。
    pub fn set_model_price(
        &self,
        public_name: &str,
        price: Option<crate::services::pricing::Price>,
    ) -> Result<bool> {
        if !self.any_backend_serves(public_name)? {
            return Err(StoreError::Invalid(format!(
                "没有任何后端提供模型 `{public_name}`"
            )));
        }
        let conn = self.conn.lock().expect("store lock poisoned");
        let priced = match price {
            Some(p) => {
                if p.input < 0.0 || p.output < 0.0 || p.cache_read < 0.0 || p.cache_creation < 0.0 {
                    return Err(StoreError::Invalid("单价不能为负".into()));
                }
                conn.execute(
                    "INSERT INTO model_meta (public_name, ctx, output, reasoning, vlm,
                                             price_input, price_output, price_cache_read, price_cache_creation,
                                             updated_at)
                     VALUES (?1, 0, 0, 0, 0, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(public_name)
                     DO UPDATE SET price_input = excluded.price_input,
                                   price_output = excluded.price_output,
                                   price_cache_read = excluded.price_cache_read,
                                   price_cache_creation = excluded.price_cache_creation,
                                   updated_at = excluded.updated_at",
                    params![public_name, p.input, p.output, p.cache_read, p.cache_creation, now_ms()],
                )?;
                true
            }
            None => {
                conn.execute(
                    "UPDATE model_meta SET price_input = NULL, price_output = NULL,
                                            price_cache_read = NULL, price_cache_creation = NULL,
                                            updated_at = ?2
                     WHERE public_name = ?1",
                    params![public_name, now_ms()],
                )?;
                conn.execute(
                    "DELETE FROM model_meta WHERE public_name = ?1
                     AND ctx = 0 AND output = 0 AND reasoning = 0 AND vlm = 0",
                    params![public_name],
                )?;
                false
            }
        };
        self.invalidate_meta_cache();
        Ok(priced)
    }

    /// 任意后端（含停用的）是否还以该名提供模型（声明/清理的找对象口径）。
    fn any_backend_serves(&self, public_name: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Self::serves_in(&conn, None, public_name)
    }

    /// SQL 层公共查询：`exclude_id` 之外的后端里，谁的 models 清单含该名。
    fn serves_in(conn: &Connection, exclude_id: Option<i64>, public_name: &str) -> Result<bool> {
        let mut stmt =
            conn.prepare("SELECT id, models FROM backends WHERE id != COALESCE(?1, -1)")?;
        let rows = stmt.query_map(params![exclude_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (_, raw) = row?;
            if serde_json::from_str::<Vec<String>>(&raw)
                .unwrap_or_default()
                .iter()
                .any(|m| m == public_name)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 摘除路径共用的孤儿清理：没有任何后端还在用该名 → 删掉全局声明。
    fn drop_orphan_meta(conn: &Connection, exclude_id: i64, public_name: &str) -> Result<usize> {
        if Self::serves_in(conn, Some(exclude_id), public_name)? {
            return Ok(0);
        }
        Ok(conn.execute(
            "DELETE FROM model_meta WHERE public_name = ?1",
            params![public_name],
        )?)
    }

    /// 给某个模型换暴露名（缺省暴露名 = 上游名，改名只为让对外名字好看/统一）。
    ///
    /// 一个事务里同时办四件事，漏一件都会留下幽灵配置：
    /// ① `backends.models` 列表替换；② `aliases` 表跟着调整（新名等于上游名时不留别名）；
    /// ③ `backend_models` 行改名；④ 参数覆盖与缓存策略的 ref_id 迁移。
    pub fn rename_model(&self, backend: &str, from: &str, to: &str) -> Result<RenameOutcome> {
        let to = to.trim();
        if to.is_empty() {
            return Err(StoreError::Invalid("暴露名不能为空".into()));
        }
        if to == from {
            return Err(StoreError::Invalid("新旧暴露名相同".into()));
        }
        let mut guard = self.conn.lock().expect("store lock poisoned");
        let tx = guard.transaction()?;

        let row: Option<(i64, String, String)> = tx
            .query_row(
                "SELECT id, models, aliases FROM backends WHERE name = ?1",
                params![backend],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (id, models_raw, aliases_raw) =
            row.ok_or_else(|| StoreError::Invalid(format!("后端 `{backend}` 不存在")))?;

        let mut models: Vec<String> = serde_json::from_str(&models_raw).unwrap_or_default();
        if !models.iter().any(|m| m == from) {
            return Err(StoreError::Invalid(format!(
                "后端 `{backend}` 上没有模型 `{from}`"
            )));
        }
        if models.iter().any(|m| m == to) {
            return Err(StoreError::Invalid(format!(
                "暴露名 `{to}` 已被该后端上的其它模型占用"
            )));
        }

        let mut aliases: serde_json::Value =
            serde_json::from_str(&aliases_raw).unwrap_or_else(|_| serde_json::json!({}));
        if !aliases.is_object() {
            aliases = serde_json::json!({});
        }
        let upstream = aliases
            .get(from)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| from.to_string());

        for m in models.iter_mut() {
            if m == from {
                *m = to.to_string();
            }
        }
        if let Some(obj) = aliases.as_object_mut() {
            obj.remove(from);
            if to != upstream.as_str() {
                obj.insert(to.to_string(), serde_json::Value::String(upstream.clone()));
            } else {
                obj.remove(to);
            }
        }
        tx.execute(
            "UPDATE backends SET models = ?1, aliases = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                serde_json::to_string(&models)?,
                serde_json::to_string(&aliases)?,
                now_ms(),
                id
            ],
        )?;
        tx.execute(
            "UPDATE backend_models SET public_name = ?1, upstream_name = ?2
             WHERE backend_id = ?3 AND public_name = ?4",
            params![to, upstream, id, from],
        )?;

        // 全局声明（model_meta 按 public_name 跨后端共享）跟着搬：
        // 别的后端还暴露旧名 → 复制到新名、旧名保留；没人再用旧名 → 整行移动。
        if let Some((m_ctx, m_output, m_re, m_vl)) = tx
            .query_row(
                "SELECT ctx, output, reasoning, vlm FROM model_meta WHERE public_name = ?1",
                params![from],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?
        {
            let still_served = Self::serves_in(&tx, Some(id), from)?;
            tx.execute(
                "INSERT INTO model_meta (public_name, ctx, output, reasoning, vlm, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(public_name)
                 DO UPDATE SET ctx = excluded.ctx, output = excluded.output,
                               reasoning = excluded.reasoning, vlm = excluded.vlm,
                               updated_at = excluded.updated_at",
                params![to, m_ctx, m_output, m_re, m_vl, now_ms()],
            )?;
            if !still_served {
                tx.execute(
                    "DELETE FROM model_meta WHERE public_name = ?1",
                    params![from],
                )?;
            }
        }

        // 参数覆盖跟着搬（模型作用域的 ref_id 形状是 "<后端>:<模型>"）。
        let old_ref = format!("{backend}:{from}");
        let new_ref = format!("{backend}:{to}");
        // 目标位置上若已有同键覆盖，以「被改名这一份」为准：先清掉挡路的旧行，
        // 否则下面 UPDATE 会撞 UNIQUE(scope, ref_id, key)。
        tx.execute(
            "DELETE FROM param_overrides
             WHERE scope = 'model' AND ref_id = ?1
               AND key IN (SELECT key FROM param_overrides WHERE scope = 'model' AND ref_id = ?2)",
            params![new_ref, old_ref],
        )?;
        let migrated_params = tx.execute(
            "UPDATE param_overrides SET ref_id = ?1, updated_at = ?2
             WHERE scope = 'model' AND ref_id = ?3",
            params![new_ref, now_ms(), old_ref],
        )?;

        // 缓存策略的模型行同理（表里 scope='model'、ref_id 就是模型名）。
        let has_policy: i64 = tx.query_row(
            "SELECT COUNT(*) FROM cache_policies WHERE scope = 'model' AND ref_id = ?1",
            params![from],
            |r| r.get(0),
        )?;
        let mut migrated_cache_policies = 0usize;
        if has_policy > 0 {
            tx.execute(
                "DELETE FROM cache_policies WHERE scope = 'model' AND ref_id = ?1",
                params![to],
            )?;
            migrated_cache_policies = tx.execute(
                "UPDATE cache_policies SET ref_id = ?1, updated_at = ?2
                 WHERE scope = 'model' AND ref_id = ?3",
                params![to, now_ms(), from],
            )?;
        }

        tx.commit()?;
        self.invalidate_meta_cache();
        Ok(RenameOutcome {
            public_name: to.to_string(),
            upstream_name: upstream,
            migrated_params,
            migrated_cache_policies,
        })
    }

    /// 从某个后端摘掉一个模型（清单上的垃圾桶）。
    ///
    /// 连带清掉它的参数覆盖与缓存策略：留着就是一堆看不见的孤儿配置，
    /// 将来排查参数时格外咬人（这也是删除前要报明「连带删几条」的原因）。
    pub fn remove_model(&self, backend: &str, public_name: &str) -> Result<RemoveOutcome> {
        let mut guard = self.conn.lock().expect("store lock poisoned");
        let tx = guard.transaction()?;
        let row: Option<(i64, String, String)> = tx
            .query_row(
                "SELECT id, models, aliases FROM backends WHERE name = ?1",
                params![backend],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (id, models_raw, aliases_raw) =
            row.ok_or_else(|| StoreError::Invalid(format!("后端 `{backend}` 不存在")))?;
        let models: Vec<String> = serde_json::from_str(&models_raw).unwrap_or_default();
        if !models.iter().any(|m| m == public_name) {
            return Err(StoreError::Invalid(format!(
                "后端 `{backend}` 上没有模型 `{public_name}`"
            )));
        }
        let left: Vec<String> = models
            .iter()
            .filter(|m| m.as_str() != public_name)
            .cloned()
            .collect();
        let mut aliases: serde_json::Value =
            serde_json::from_str(&aliases_raw).unwrap_or_else(|_| serde_json::json!({}));
        if let Some(obj) = aliases.as_object_mut() {
            obj.remove(public_name);
        }
        tx.execute(
            "UPDATE backends SET models = ?1, aliases = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                serde_json::to_string(&left)?,
                serde_json::to_string(&aliases)?,
                now_ms(),
                id
            ],
        )?;
        tx.execute(
            "DELETE FROM backend_models WHERE backend_id = ?1 AND public_name = ?2",
            params![id, public_name],
        )?;
        let dropped_params = tx.execute(
            "DELETE FROM param_overrides WHERE scope = 'model' AND ref_id = ?1",
            params![format!("{backend}:{public_name}")],
        )?;
        let dropped_cache_policies = tx.execute(
            "DELETE FROM cache_policies WHERE scope = 'model' AND ref_id = ?1",
            params![public_name],
        )?;
        // 全局声明的孤儿清理：没有别的后端还在用这个名字才删（有人用就留着）。
        let dropped_meta = Self::drop_orphan_meta(&tx, id, public_name)?;
        // 级联删除（UI 重设计决策 3）：用户手动删掉最后一个模型 → 后端随之消失。
        // 护栏：只有这条用户主动路径会走到这里；新建未扫模型的空后端不受影响。
        let backend_deleted = left.is_empty();
        if backend_deleted {
            tx.execute(
                "DELETE FROM backend_models WHERE backend_id = ?1",
                params![id],
            )?;
            tx.execute("DELETE FROM backends WHERE id = ?1", params![id])?;
        }
        tx.commit()?;
        self.invalidate_meta_cache();
        Ok(RemoveOutcome {
            dropped_params,
            dropped_cache_policies,
            dropped_meta,
            backend_deleted,
        })
    }

    /// 刷新后端时以上游为准合并模型清单（UI 重设计决策：⟳ 二次确认背后的真操作）。
    ///
    /// **摘挂不销毁**（用户实报：上游列表抖一次，参数覆盖就被永久删掉）：
    /// · 新增：上游有、本地没有 → 反查别名定公开名（曾改名的友好名优先复活，
    ///   没有映射才用上游原名）追加进清单；
    /// · 移除：本地有、上游没了 → 只从 `backends.models` 摘下（即刻不再对外
    ///   服务），行 / 别名 / 参数覆盖 / 缓存策略 / 元数据声明**全部保留**——
    ///   服务真值是列（`exposed_models` / `resolve_upstream` 都按列走），游离行
    ///   对外不可见，等于配置暂存；
    /// · 挂回：上游恢复该 id → 反查别名得到原公开名，`ON CONFLICT` 命中保留行，
    ///   暴露开关 / 探测值 / ref_id=`<后端>:<模型>` 的参数覆盖整套自动复活。
    ///
    /// 扫描同时带回 ctx/output 真值（[`ScannedModel`]）：
    /// · 写进 `backend_models` 行（缺行补插、开关与过渡开关保留）；
    /// · 探测值 **COALESCE**——新值有就更新、没有就保留，避免一次拿不到 ctx 的
    ///   扫描把既有真值抹掉（没有的真值不给，但也别反向清空）。
    pub fn sync_backend_models(
        &self,
        backend: &str,
        discovered: &[ScannedModel],
    ) -> Result<SyncOutcome> {
        let mut guard = self.conn.lock().expect("store lock poisoned");
        let tx = guard.transaction()?;
        let row: Option<(i64, String, String)> = tx
            .query_row(
                "SELECT id, models, aliases FROM backends WHERE name = ?1",
                params![backend],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (id, models_raw, aliases_raw) =
            row.ok_or_else(|| StoreError::Invalid(format!("后端 `{backend}` 不存在")))?;
        let models: Vec<String> = serde_json::from_str(&models_raw).unwrap_or_default();
        let aliases: serde_json::Value =
            serde_json::from_str(&aliases_raw).unwrap_or_else(|_| serde_json::json!({}));
        let upstream_of = |public: &str| -> String {
            aliases
                .get(public)
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| public.to_string())
        };
        // 探测真值按上游名索引（ctx/output <=0 视为没给；布尔三态原样传递）。
        let probe_of = |upstream: &str| -> (Option<i64>, Option<i64>, Option<i64>, Option<i64>) {
            match discovered.iter().find(|s| s.id == upstream) {
                Some(s) => (
                    s.ctx.filter(|v| *v > 0),
                    s.output.filter(|v| *v > 0),
                    s.reasoning.map(|v| v as i64),
                    s.vlm.map(|v| v as i64),
                ),
                None => (None, None, None, None),
            }
        };
        let known: std::collections::BTreeSet<String> =
            models.iter().map(|m| upstream_of(m)).collect();
        let disc: std::collections::BTreeSet<String> =
            discovered.iter().map(|s| s.id.clone()).collect();

        let added_ids: Vec<String> = discovered
            .iter()
            .filter(|s| !known.contains(&s.id))
            .map(|s| s.id.clone())
            .collect();
        let removed: Vec<String> = models
            .iter()
            .filter(|m| !disc.contains(upstream_of(m).as_str()))
            .cloned()
            .collect();

        // 新增项的公开名：先拿别名反查（被摘下的友好名 —— 别名随移除保留 —— 优先
        // 复活，曾改名的模型抖动后回来仍用原名原配置）；查不到才用上游原名。
        let in_column: std::collections::BTreeSet<&String> = models.iter().collect();
        let mut added: Vec<String> = Vec::new();
        for id in &added_ids {
            let friendly: Vec<String> = aliases
                .as_object()
                .map(|obj| {
                    obj.iter()
                        .filter(|(_, v)| v.as_str() == Some(id.as_str()))
                        .map(|(k, _)| k.clone())
                        .collect()
                })
                .unwrap_or_default();
            if friendly.is_empty() {
                if !in_column.contains(id) {
                    added.push(id.clone());
                }
            } else {
                added.extend(friendly.into_iter().filter(|p| !in_column.contains(p)));
            }
        }

        let mut next: Vec<String> = models
            .iter()
            .filter(|m| !removed.contains(m))
            .cloned()
            .chain(added.iter().cloned())
            .collect();
        next.sort();
        // 别名不动：被摘模型的「友好名 → 上游 id」是挂回时认亲的凭据，随移除
        // 删掉就只能以裸上游名复活（改名与配置全丢）；非对象形态规整成 {}。
        let next_aliases = if aliases.is_object() {
            aliases.clone()
        } else {
            serde_json::json!({})
        };

        // 行同步：缺行补插（开关默认开），已有行只更新上游名与探测值（COALESCE 保留旧真值）。
        for public in &next {
            let upstream = upstream_of(public);
            let (p_ctx, p_output, p_re, p_vl) = probe_of(&upstream);
            tx.execute(
                "INSERT INTO backend_models
                     (backend_id, public_name, upstream_name, enabled, accept_upstream,
                      probe_ctx, probe_output, probe_reasoning, probe_vlm)
                 VALUES (?1, ?2, ?3, 1, 0, ?4, ?5, ?6, ?7)
                 ON CONFLICT(backend_id, public_name) DO UPDATE SET
                     upstream_name = excluded.upstream_name,
                     probe_ctx = COALESCE(excluded.probe_ctx, backend_models.probe_ctx),
                     probe_output = COALESCE(excluded.probe_output, backend_models.probe_output),
                     probe_reasoning = COALESCE(excluded.probe_reasoning, backend_models.probe_reasoning),
                     probe_vlm = COALESCE(excluded.probe_vlm, backend_models.probe_vlm)",
                params![id, public, upstream, p_ctx, p_output, p_re, p_vl],
            )?;
        }

        // 被摘模型（removed）：只出列，不删行 —— 行上的暴露开关、过渡开关、探测值
        // 与 ref_id=`<后端>:<模型>` 的参数覆盖 / 缓存策略 / 元数据声明全部原地保留，
        // 上游恢复后经 ON CONFLICT 命中同一行整套挂回（用户实报的丢配置 bug）。
        if !removed.is_empty() || !added.is_empty() {
            tx.execute(
                "UPDATE backends SET models = ?1, aliases = ?2, updated_at = ?3 WHERE id = ?4",
                params![
                    serde_json::to_string(&next)?,
                    serde_json::to_string(&next_aliases)?,
                    now_ms(),
                    id
                ],
            )?;
        }
        tx.commit()?;
        self.invalidate_meta_cache();
        Ok(SyncOutcome {
            kept: models.len() - removed.len(),
            added,
            removed,
        })
    }

    /// 保存后端时把这次扫描顺带带回的 ctx/output 真值落列（新增后端不用再点
    /// 一次 ⟳ 才看得到 ctx）。
    ///
    /// 与 [`Self::sync_backend_models`] 的分工：那个以扫描为准（改清单、报 diff），
    /// 这里以**用户保存的清单为准**——扫描结果只做真值来源：
    /// · 只更新已在册的行（扫描回来是上游名，公开名经别名也能对上），清单外的 id 忽略；
    /// · 探测值 COALESCE：新值有就更新、没给就保留，拿不到不清空既有真值。
    pub fn set_probe_values(&self, backend: &str, scanned: &[ScannedModel]) -> Result<usize> {
        if scanned.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().expect("store lock poisoned");
        let row: Option<(i64, String, String)> = conn
            .query_row(
                "SELECT id, models, aliases FROM backends WHERE name = ?1",
                params![backend],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        // 后端还没落库：真值没有去处，等 ⟳ 刷新再落。
        let Some((id, models_raw, aliases_raw)) = row else {
            return Ok(0);
        };
        let models: Vec<String> = serde_json::from_str(&models_raw).unwrap_or_default();
        let aliases: serde_json::Value =
            serde_json::from_str(&aliases_raw).unwrap_or_else(|_| serde_json::json!({}));
        let mut applied = 0usize;
        for s in scanned {
            let (p_ctx, p_output) = (s.ctx.filter(|v| *v > 0), s.output.filter(|v| *v > 0));
            let (p_re, p_vl) = (s.reasoning.map(|v| v as i64), s.vlm.map(|v| v as i64));
            if p_ctx.is_none() && p_output.is_none() && p_re.is_none() && p_vl.is_none() {
                continue;
            }
            for public in models.iter().filter(|p| {
                let up = aliases
                    .get(p.as_str())
                    .and_then(|v| v.as_str())
                    .unwrap_or(p.as_str());
                up == s.id.as_str() || *p == &s.id
            }) {
                applied += conn.execute(
                    "UPDATE backend_models SET probe_ctx = COALESCE(?3, probe_ctx),
                         probe_output = COALESCE(?4, probe_output),
                         probe_reasoning = COALESCE(?5, probe_reasoning),
                         probe_vlm = COALESCE(?6, probe_vlm)
                     WHERE backend_id = ?1 AND public_name = ?2",
                    params![id, public, p_ctx, p_output, p_re, p_vl],
                )?;
            }
        }
        self.invalidate_meta_cache();
        Ok(applied)
    }

    /// 删掉一个后端，连带清掉只属于它的配置；后端不存在返回 `None`。
    ///
    /// 与 `remove_model` 的分工：那是「摘一个模型」（行保留、上游回来自动挂回生效），
    /// 这里是整台后端退场——模型行、别名、参数覆盖、缓存策略一并作废。
    ///
    /// 模型层参数覆盖按 **`<后端名>:` 前缀整批清**（不用 LIKE：名字里的 `%`/`_` 会
    /// 变成通配符）。前缀比逐个 `models` 名清更彻底——上游清单抖动时被摘下、名字早已
    /// 不在 `models` 里的模型，它的覆盖留着同样是孤儿。
    pub fn delete_backend(&self, name: &str) -> Result<Option<DeleteOutcome>> {
        let mut guard = self.conn.lock().expect("store lock poisoned");
        let tx = guard.transaction()?;
        let row: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, models FROM backends WHERE name = ?1",
                params![name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, models_raw)) = row else {
            return Ok(None);
        };
        tx.execute(
            "DELETE FROM backend_models WHERE backend_id = ?1",
            params![id],
        )?;
        tx.execute("DELETE FROM backends WHERE id = ?1", params![id])?;
        // 后端层覆盖：ref_id 就是后端名，跟着一起退场。
        let mut dropped_params = tx.execute(
            "DELETE FROM param_overrides WHERE scope = 'backend' AND ref_id = ?1",
            params![name],
        )?;
        // 模型层覆盖：ref_id 形如 "<后端名>:<模型名>"，只有这台后端可能写过它。
        dropped_params += tx.execute(
            "DELETE FROM param_overrides
              WHERE scope = 'model' AND substr(ref_id, 1, length(?1) + 1) = ?2",
            params![name, format!("{name}:")],
        )?;
        // 全局声明与缓存策略：这台后端独占的模型，声明随之消失；别人还在用的留着
        // （`drop_orphan_meta` 内部同口径复查一次，免得「谁算独占」有两份定义）。
        let mut dropped_meta = 0;
        for m in serde_json::from_str::<Vec<String>>(&models_raw).unwrap_or_default() {
            if !Self::serves_in(&tx, Some(id), &m)? {
                dropped_meta += Self::drop_orphan_meta(&tx, id, &m)?;
                tx.execute(
                    "DELETE FROM cache_policies WHERE scope = 'model' AND ref_id = ?1",
                    params![m],
                )?;
            }
        }
        tx.commit()?;
        self.invalidate_meta_cache();
        Ok(Some(DeleteOutcome {
            dropped_params,
            dropped_meta,
        }))
    }

    pub fn record_health(&self, name: &str, healthy: bool, error: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "UPDATE backends SET health_state = ?1, consecutive_failures = ?2, last_checked_at = ?3, last_error = ?4
             WHERE name = ?5",
            params![
                if healthy { "healthy" } else { "down" },
                if healthy { 0_i64 } else { 1_i64 },
                now_ms(),
                error,
                name
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------- usage

    pub fn record_usage(&self, u: &UsageRecord) -> Result<()> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "INSERT INTO usage_records (request_id, ts, backend, model, entry_protocol, client, status,
                    ok, stream, ttft_ms, duration_ms, input_tokens, output_tokens, cache_read_tokens,
                    cache_creation_tokens, decode_tps, estimated, cache_status, retry_trace, error,
                    media_images, media_seconds)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
            params![
                u.request_id,
                u.ts,
                u.backend,
                u.model,
                u.entry_protocol,
                u.client,
                u.status,
                u.ok as i64,
                u.stream as i64,
                u.ttft_ms,
                u.duration_ms,
                u.input_tokens,
                u.output_tokens,
                u.cache_read_tokens,
                u.cache_creation_tokens,
                u.decode_tps,
                u.estimated as i64,
                u.cache_status,
                u.retry_trace,
                u.error,
                u.media_images,
                u.media_seconds
            ],
        )?;
        Ok(())
    }

    /// 媒体计量聚合（11d 用量页）：按 模型×后端 汇总媒体行的请求数、图像张数、音频秒数。
    /// 媒体行识别：entry_protocol 存的是媒体端点路径（`/v1/` 前缀，聊天行是协议名）。
    /// SUM 忽略 NULL：全 NULL 的列出 NULL = 取不到（UI 显示 —，不造假数字）。
    pub fn usage_media(&self, from: i64, to: i64) -> Result<Vec<MediaUsageRow>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT model, backend, COUNT(*), SUM(media_images), SUM(media_seconds)
             FROM usage_records
             WHERE ts BETWEEN ?1 AND ?2 AND entry_protocol LIKE '/v1/%'
             GROUP BY model, backend
             ORDER BY COUNT(*) DESC, model",
        )?;
        let rows = stmt.query_map(params![from, to], |r| {
            Ok(MediaUsageRow {
                model: r.get(0)?,
                backend: r.get(1)?,
                requests: r.get(2)?,
                images: r.get(3)?,
                seconds: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn usage_summary(&self, from: i64, to: i64) -> Result<UsageSummary> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let row = conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(ok), 0),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0),
                    AVG(ttft_ms),
                    AVG(decode_tps)
             FROM usage_records WHERE ts BETWEEN ?1 AND ?2",
            params![from, to],
            |r| {
                Ok(UsageSummary {
                    requests: r.get(0)?,
                    ok_requests: r.get(1)?,
                    input_tokens: r.get(2)?,
                    output_tokens: r.get(3)?,
                    cache_read_tokens: r.get(4)?,
                    avg_ttft_ms: r.get(5)?,
                    avg_decode_tps: r.get(6)?,
                })
            },
        )?;
        Ok(row)
    }

    /// Mean TTFT over the last 20 successful streamed calls of a backend —
    /// pragmatic stand-in for EWMA in least_busy scoring (M3).
    pub fn recent_avg_ttft(&self, backend: &str) -> Result<Option<f64>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let v: Option<Option<f64>> = conn
            .query_row(
                "SELECT AVG(ttft_ms) FROM (
                    SELECT ttft_ms FROM usage_records
                    WHERE backend = ?1 AND ok = 1 AND ttft_ms IS NOT NULL
                    ORDER BY ts DESC LIMIT 20
                 )",
                params![backend],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.flatten())
    }

    /// 明细分页（M4-D）：倒序，可按后端/模型/入口/request_id/时间过滤。
    pub fn usage_records_query(&self, f: &UsageRecordQuery) -> Result<Vec<UsageRecord>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut sql = String::from(
            "SELECT request_id, ts, backend, model, entry_protocol, client, status, ok, stream,
                    ttft_ms, duration_ms, input_tokens, output_tokens, cache_read_tokens,
                    cache_creation_tokens, decode_tps, estimated, cache_status, retry_trace, error,
                    media_images, media_seconds
             FROM usage_records WHERE 1=1",
        );
        let mut args: Vec<SqlValue> = Vec::new();
        let mut push = |clause: &str, v: SqlValue| {
            sql.push_str(" AND ");
            sql.push_str(clause);
            args.push(v);
        };
        if let Some(b) = &f.backend {
            push("backend = ?N", SqlValue::Text(b.clone()));
        }
        if let Some(m) = &f.model {
            push("model = ?N", SqlValue::Text(m.clone()));
        }
        if let Some(e) = &f.entry_protocol {
            push("entry_protocol = ?N", SqlValue::Text(e.clone()));
        }
        if let Some(r) = &f.request_id {
            push("request_id = ?N", SqlValue::Text(r.clone()));
        }
        if let Some(from) = f.from {
            push("ts >= ?N", SqlValue::Integer(from));
        }
        if let Some(to) = f.to {
            push("ts <= ?N", SqlValue::Integer(to));
        }
        if f.only_errors {
            sql.push_str(" AND ok = 0");
        }
        sql.push_str(" ORDER BY ts DESC, id DESC LIMIT ?N OFFSET ?N");
        args.push(SqlValue::Integer(f.limit.clamp(1, 1000)));
        args.push(SqlValue::Integer(f.offset.max(0)));
        // 占位符按序换成 ?1..?n。
        let mut idx = 0usize;
        let sql: String = {
            let mut out = String::new();
            let mut chars = sql.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '?' && chars.peek() == Some(&'N') {
                    chars.next();
                    idx += 1;
                    out.push('?');
                    out.push_str(&idx.to_string());
                } else {
                    out.push(c);
                }
            }
            out
        };
        let refs: Vec<&dyn rusqlite::ToSql> =
            args.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), |r| {
            Ok(UsageRecord {
                request_id: r.get(0)?,
                ts: r.get(1)?,
                backend: r.get(2)?,
                model: r.get(3)?,
                entry_protocol: r.get(4)?,
                client: r.get(5)?,
                status: r.get(6)?,
                ok: r.get::<_, i64>(7)? != 0,
                stream: r.get::<_, i64>(8)? != 0,
                ttft_ms: r.get(9)?,
                duration_ms: r.get(10)?,
                input_tokens: r.get(11)?,
                output_tokens: r.get(12)?,
                cache_read_tokens: r.get(13)?,
                cache_creation_tokens: r.get(14)?,
                decode_tps: r.get(15)?,
                estimated: r.get::<_, i64>(16)? != 0,
                cache_status: r.get(17)?,
                retry_trace: r.get(18)?,
                error: r.get(19)?,
                media_images: r.get(20)?,
                media_seconds: r.get(21)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// M8-C：删除早于水位线的明细（日聚合永久保留）。
    pub fn prune_usage_records(&self, before_ms: i64) -> Result<u64> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let n = conn.execute(
            "DELETE FROM usage_records WHERE ts < ?1",
            params![before_ms],
        )?;
        Ok(n as u64)
    }

    /// M8-B：TTFT P50/P95（升序取样，本地量级全量排序足够快）。
    pub fn usage_ttft_percentiles(&self, from: i64, to: i64) -> Result<(Option<f64>, Option<f64>)> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT ttft_ms FROM usage_records
             WHERE ts BETWEEN ?1 AND ?2 AND ttft_ms IS NOT NULL ORDER BY ttft_ms",
        )?;
        let rows = stmt.query_map(params![from, to], |r| r.get::<_, Option<f64>>(0))?;
        let mut vals: Vec<f64> = Vec::new();
        for r in rows {
            if let Some(v) = r? {
                vals.push(v);
            }
        }
        if vals.is_empty() {
            return Ok((None, None));
        }
        // 最近秩法：rank = ceil(q·n)，取第 rank 个（1 基）。
        let pick = |q: f64| -> f64 {
            let i = (q * vals.len() as f64).ceil() as usize;
            vals[i.max(1).min(vals.len()) - 1]
        };
        Ok((Some(pick(0.50)), Some(pick(0.95))))
    }

    /// M8-B：响应缓存状态计数（hit/miss/bypass/未参与）。
    pub fn usage_cache_counts(
        &self,
        from: i64,
        to: i64,
    ) -> Result<std::collections::BTreeMap<String, i64>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT COALESCE(cache_status,'none'), COUNT(*) FROM usage_records
             WHERE ts BETWEEN ?1 AND ?2 GROUP BY 1",
        )?;
        let rows = stmt.query_map(params![from, to], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut out = std::collections::BTreeMap::new();
        for r in rows {
            let (k, v) = r?;
            out.insert(k, v);
        }
        Ok(out)
    }

    /// M8-B：按客户端聚合（请求数与 token 合计）。
    pub fn usage_by_client(&self, from: i64, to: i64) -> Result<Vec<ClientAgg>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT COALESCE(client,'未知'), COUNT(*), SUM(input_tokens), SUM(output_tokens)
             FROM usage_records WHERE ts BETWEEN ?1 AND ?2 GROUP BY 1 ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map(params![from, to], |r| {
            Ok(ClientAgg {
                client: r.get(0)?,
                requests: r.get(1)?,
                input: r.get(2)?,
                output: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// M8-B：日×维度序列（dim 白名单：backend|model|client）。
    pub fn usage_series(
        &self,
        from: i64,
        to: i64,
        dim: &str,
    ) -> Result<Vec<(String, String, i64, i64)>> {
        let col = match dim {
            "backend" | "model" | "client" => dim,
            _ => return Err(StoreError::Invalid(format!("非法维度: {dim}"))),
        };
        let sql = format!(
            "SELECT date(ts/1000,'unixepoch','localtime'), COALESCE({col},''), COUNT(*),
                    SUM(input_tokens)+SUM(output_tokens)
             FROM usage_records WHERE ts BETWEEN ?1 AND ?2
             GROUP BY 1, 2 ORDER BY 1",
        );
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![from, to], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// 区间内按（模型 × 后端引擎）聚合 token 四类计数（片4 成本汇总底座）。
    ///
    /// LEFT JOIN backends：后端被删/未填引擎 → engine 为 NULL，按在线分档（不猜本地）。
    pub fn usage_model_totals(&self, from: i64, to: i64) -> Result<Vec<ModelAgg>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT COALESCE(u.model,''), b.engine, COUNT(*), SUM(input_tokens), SUM(output_tokens),
                    SUM(cache_read_tokens), SUM(cache_creation_tokens)
             FROM usage_records u LEFT JOIN backends b ON b.name = u.backend
             WHERE u.ts BETWEEN ?1 AND ?2
             GROUP BY COALESCE(u.model,''), b.engine
             ORDER BY COUNT(*) DESC",
        )?;
        let rows = stmt.query_map(params![from, to], |r| {
            Ok(ModelAgg {
                model: r.get(0)?,
                engine: r.get(1)?,
                requests: r.get(2)?,
                input: r.get(3)?,
                output: r.get(4)?,
                cache_read: r.get(5)?,
                cache_creation: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// 增量日聚合（M4-B）：把游标之后的 usage_records 汇入 usage_daily，回写游标。
    /// 由维护循环与启动钩子调用；UTC 日界（date(ts/1000,'unixepoch')）。
    pub fn rollup_usage_daily(&self) -> Result<u64> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let cursor: i64 = conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM settings WHERE key = 'rollup.usage_daily.cursor'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let scanned: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usage_records WHERE id > ?1",
            rusqlite::params![cursor],
            |r| r.get(0),
        )?;
        if scanned == 0 {
            return Ok(0);
        }
        conn.execute_batch("BEGIN")?;
        let res = (|| -> rusqlite::Result<()> {
            conn.execute(
                r#"
                INSERT INTO usage_daily (day, backend, model, requests, errors, input_tokens, output_tokens, cache_read_tokens, ttft_sum, ttft_count, tps_sum, tps_count)
                SELECT date(ts/1000,'unixepoch'), COALESCE(backend,''), COALESCE(model,''),
                       COUNT(*), SUM(CASE WHEN ok=0 THEN 1 ELSE 0 END),
                       SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens),
                       COALESCE(SUM(ttft_ms),0), COUNT(ttft_ms),
                       COALESCE(SUM(decode_tps),0.0), COUNT(decode_tps)
                FROM usage_records WHERE id > ?1
                GROUP BY 1,2,3
                ON CONFLICT(day,backend,model) DO UPDATE SET
                    requests = requests + excluded.requests,
                    errors = errors + excluded.errors,
                    input_tokens = input_tokens + excluded.input_tokens,
                    output_tokens = output_tokens + excluded.output_tokens,
                    cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens,
                    ttft_sum = ttft_sum + excluded.ttft_sum,
                    ttft_count = ttft_count + excluded.ttft_count,
                    tps_sum = tps_sum + excluded.tps_sum,
                    tps_count = tps_count + excluded.tps_count
                "#,
                rusqlite::params![cursor],
            )?;
            let max_id: i64 = conn.query_row(
                "SELECT COALESCE(MAX(id), ?1) FROM usage_records",
                rusqlite::params![cursor],
                |r| r.get(0),
            )?;
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('rollup.usage_daily.cursor', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![max_id.to_string()],
            )?;
            Ok(())
        })();
        match res {
            Ok(()) => {
                conn.execute_batch("COMMIT")?;
                Ok(scanned as u64)
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e.into())
            }
        }
    }

    /// 最近 `days` 天的日聚合（跨 backend/model 汇总到日粒度）。
    pub fn usage_daily_list(&self, days: i64) -> Result<Vec<DailyRollup>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT day, SUM(requests), SUM(errors), SUM(input_tokens), SUM(output_tokens),
                    SUM(ttft_sum), SUM(ttft_count)
             FROM usage_daily
             WHERE day >= date('now', ?1)
             GROUP BY day ORDER BY day",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![format!("-{} days", days.max(1) - 1)],
            |r| {
                Ok(DailyRollup {
                    day: r.get(0)?,
                    requests: r.get(1)?,
                    errors: r.get(2)?,
                    input_tokens: r.get(3)?,
                    output_tokens: r.get(4)?,
                    avg_ttft_ms: {
                        let s: i64 = r.get(5)?;
                        let c: i64 = r.get(6)?;
                        if c > 0 {
                            Some((s / c) as f64)
                        } else {
                            None
                        }
                    },
                })
            },
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Most recent non-null retry trace (diagnostics/tests).
    pub fn latest_retry_trace(&self) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn
            .query_row(
                "SELECT retry_trace FROM usage_records WHERE retry_trace IS NOT NULL ORDER BY ts DESC, id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn usage_by_model(&self, from: i64, to: i64) -> Result<Vec<(String, i64, i64)>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT COALESCE(model, '?'), COUNT(*), COALESCE(SUM(input_tokens + output_tokens), 0)
             FROM usage_records WHERE ts BETWEEN ?1 AND ?2
             GROUP BY model ORDER BY 3 DESC LIMIT 50",
        )?;
        let mut out = Vec::new();
        let rows = stmt.query_map(params![from, to], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    // -------------------------------------------------------------------- logs

    pub fn insert_logs(&self, rows: &[LogRow]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut n = 0usize;
        for row in rows {
            n += conn.execute(
                "INSERT INTO logs (ts, level, code, message, request_id, backend, model, extra)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    row.ts,
                    row.level,
                    row.code,
                    row.message,
                    row.request_id,
                    row.backend,
                    row.model,
                    row.extra.to_string()
                ],
            )?;
        }
        Ok(n)
    }

    pub fn query_logs(&self, q: &LogQuery) -> Result<Vec<LogRow>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut sql = String::from(
            "SELECT id, ts, level, code, message, request_id, backend, model, extra FROM logs WHERE 1=1",
        );
        let mut args: Vec<SqlValue> = Vec::new();

        if let Some(from) = q.from {
            sql.push_str(" AND ts >= ?");
            args.push(SqlValue::Integer(from));
        }
        if let Some(to) = q.to {
            sql.push_str(" AND ts <= ?");
            args.push(SqlValue::Integer(to));
        }
        if let Some(level) = &q.level {
            sql.push_str(" AND level = ?");
            args.push(SqlValue::Text(level.clone()));
        }
        if let Some(backend) = &q.backend {
            sql.push_str(" AND backend = ?");
            args.push(SqlValue::Text(backend.clone()));
        }
        if let Some(model) = &q.model {
            sql.push_str(" AND model = ?");
            args.push(SqlValue::Text(model.clone()));
        }
        if let Some(rid) = &q.request_id {
            sql.push_str(" AND request_id = ?");
            args.push(SqlValue::Text(rid.clone()));
        }
        if q.only_errors {
            sql.push_str(" AND level IN ('error','warn')");
        }
        if let Some(text) = &q.text {
            sql.push_str(" AND (message LIKE ? OR code LIKE ?)");
            let pat = format!("%{text}%");
            args.push(SqlValue::Text(pat.clone()));
            args.push(SqlValue::Text(pat));
        }
        sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");
        args.push(SqlValue::Integer(q.limit.max(1)));
        args.push(SqlValue::Integer(q.offset.max(0)));

        let refs: Vec<&dyn rusqlite::ToSql> =
            args.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let mut stmt = conn.prepare(&sql)?;
        let mut out = Vec::new();
        let rows = stmt.query_map(refs.as_slice(), |r| {
            let extra: String = r.get(8)?;
            Ok(LogRow {
                id: r.get(0)?,
                ts: r.get(1)?,
                level: r.get(2)?,
                code: r.get(3)?,
                message: r.get(4)?,
                request_id: r.get(5)?,
                backend: r.get(6)?,
                model: r.get(7)?,
                extra: serde_json::from_str(&extra).unwrap_or(Value::Null),
            })
        })?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// 日志页过滤器选项：logs 表里真实出现过的后端 / 模型名（DISTINCT）。
    /// 取日志真值而不是配置表——历史行的后端可能早已删除，但它的日志还在、
    /// 还要能被过滤到；col 只由本文件以固定字面量调用，无注入面。
    pub fn log_facets(&self) -> Result<(Vec<String>, Vec<String>)> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let distinct = |col: &str| -> Result<Vec<String>> {
            let sql = format!(
                "SELECT DISTINCT {col} FROM logs WHERE {col} IS NOT NULL AND {col} != '' ORDER BY {col}"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.map(|r| r.map_err(Into::into)).collect()
        };
        Ok((distinct("backend")?, distinct("model")?))
    }

    pub fn purge_logs_before(&self, ts: i64) -> Result<usize> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn.execute("DELETE FROM logs WHERE ts < ?1", params![ts])?)
    }

    pub fn log_table_bytes(&self) -> Result<i64> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let pages: i64 = conn.query_row(
            "SELECT pgcount * pgsize FROM pragma_page_count(), pragma_page_size()",
            [],
            |r| r.get(0),
        )?;
        Ok(pages)
    }

    // ---------------------------------------------------------------- settings

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// copilot（Ollama 门面下游）的虚拟分配清单；`None` = 缺省全暴露。
    /// 存 settings 表是因为 copilot 没有我们能写的配置文件 —— 门面 `/api/tags`
    /// 按这份清单过滤（UI 重设计：代理分配按钮组的 copilot 芯片）。
    pub fn copilot_allow(&self) -> Option<Vec<String>> {
        self.get_setting(COPILOT_ALLOW_KEY)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<Option<Vec<String>>>(&raw).ok())
            .flatten()
    }

    /// 代理启停（代理 tab，docs/plans/2026-10-09-代理管理tab.md）：停用 = 从代理
    /// 列表与两张矩阵里消失。只动这份清单——不碰配置文件、不清分配，启用即恢复。
    pub fn agents_disabled(&self) -> Vec<String> {
        self.get_setting("agents.disabled")
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
            .unwrap_or_default()
    }

    pub fn set_agent_enabled(&self, app: &str, enabled: bool) -> Result<()> {
        let mut set: std::collections::BTreeSet<String> =
            self.agents_disabled().into_iter().collect();
        if enabled {
            set.remove(app);
        } else {
            set.insert(app.to_string());
        }
        self.set_setting("agents.disabled", &serde_json::to_string(&set)?)
    }

    /// 某端配置文件的路径自定义（内置默认路径 → 自定义路径，代理 tab 片三）。
    /// 一份文件一条：默认路径就是身份键，`None` = 恢复默认。
    pub fn path_overrides(&self, app: &str) -> std::collections::BTreeMap<String, String> {
        self.get_setting(&format!("agent.path_overrides.{app}"))
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn set_path_override(&self, app: &str, default: &str, custom: Option<&str>) -> Result<()> {
        let mut map = self.path_overrides(app);
        match custom {
            Some(c) => {
                map.insert(default.to_string(), c.to_string());
            }
            None => {
                map.remove(default);
            }
        }
        self.set_setting(
            &format!("agent.path_overrides.{app}"),
            &serde_json::to_string(&map)?,
        )
    }

    /// 后端页②区的模型历史统计（UI 重设计）：一次拿齐三项，避免三次锁表。
    /// · 历史 prefill 均值 = AVG(输入 token / TTFT)，近 7 天成功请求；
    /// · 实际 ctx = 最近一次成功请求的输入 token（≈当前会话上下文占用）；
    /// · 「其他」指示灯 = 近 7 天有非接管矩阵客户端署名的请求（保守口径：
    ///   无署名不计入，宁可不亮不误亮）。
    pub fn model_usage_stats(
        &self,
        model: &str,
        since_ms: i64,
    ) -> Result<(Option<f64>, Option<i64>, bool)> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let avg_prefill: Option<f64> = conn
            .query_row(
                "SELECT AVG(input_tokens * 1000.0 / ttft_ms) FROM usage_records
                 WHERE model = ?1 AND ok = 1 AND ttft_ms IS NOT NULL AND ttft_ms > 0
                   AND input_tokens > 0 AND ts >= ?2",
                params![model, since_ms],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let ctx_used: Option<i64> = conn
            .query_row(
                "SELECT input_tokens FROM usage_records
                 WHERE model = ?1 AND ok = 1 AND input_tokens > 0
                 ORDER BY ts DESC LIMIT 1",
                params![model],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let others: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM usage_records
                 WHERE model = ?1 AND ts >= ?2 AND client IS NOT NULL AND client <> ''
                   AND lower(client) NOT LIKE '%claude%'
                   AND lower(client) NOT LIKE '%codex%'
                   AND lower(client) NOT LIKE '%opencode%'
                   AND lower(client) NOT LIKE '%deveco%'
                   AND lower(client) NOT LIKE '%gemini%'
                   AND lower(client) <> 'pi'",
                params![model, since_ms],
                |r| r.get(0),
            )
            .unwrap_or(0);
        Ok((avg_prefill, ctx_used, others > 0))
    }

    // ---------------------------------------------------------- param overrides

    /// All parameter override rows, ordered for deterministic reloads.
    pub fn list_param_overrides(&self) -> Result<Vec<ParamOverride>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, scope, ref_id, key, value, policy FROM param_overrides \
             ORDER BY scope, ref_id, key",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ParamOverride {
                id: r.get(0)?,
                scope: r.get(1)?,
                ref_id: r.get(2)?,
                key: r.get(3)?,
                value: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(Value::Null),
                policy: r.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Insert-or-replace one override; returns the row id.
    pub fn upsert_param_override(&self, o: &ParamOverride) -> Result<i64> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "INSERT INTO param_overrides (scope, ref_id, key, value, policy, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(scope, ref_id, key) DO UPDATE SET \
               value = excluded.value, policy = excluded.policy, updated_at = excluded.updated_at",
            params![
                o.scope,
                o.ref_id,
                o.key,
                o.value.to_string(),
                o.policy,
                now_ms()
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_param_override(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn.execute("DELETE FROM param_overrides WHERE id = ?1", params![id])? > 0)
    }

    // ---------------------------------------------------------- prompt items
    // 提示词速取面板（计划 2026-11-18）：条目按 tab 归属，tab 内按 sort 排序。

    /// 全部提示词条目。排序口径（用户：「高频用的自动排前面」）：
    /// 同一 tab 内先按取用次数降序、次数相同按最后取用降序，两者都相同才回到
    /// 手工 `sort`。搜索态跨 tab 时同一口径适用，因为 `tab` 在前只影响分组。
    pub fn list_prompt_items(&self) -> Result<Vec<PromptItem>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, tab, content, sort, use_count, last_used_at FROM prompt_items \
             ORDER BY tab, use_count DESC, last_used_at DESC, sort, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PromptItem {
                id: r.get(0)?,
                tab: r.get(1)?,
                content: r.get(2)?,
                sort: r.get(3)?,
                use_count: r.get(4)?,
                last_used_at: r.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// 记一次取用：次数 +1、刷新最后取用时间。只影响排序，返是否命中行。
    pub fn bump_prompt_item(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn.execute(
            "UPDATE prompt_items SET use_count = use_count + 1, last_used_at = ?2 WHERE id = ?1",
            params![id, now_ms()],
        )? > 0)
    }

    /// 新增一条到该 tab 末尾（sort = 现有最大值 + 1），返回行 id。
    pub fn create_prompt_item(&self, tab: &str, content: &str) -> Result<i64> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let next: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort), -1) + 1 FROM prompt_items WHERE tab = ?1",
                params![tab],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let now = now_ms();
        conn.execute(
            "INSERT INTO prompt_items (tab, content, sort, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![tab, content, next, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    /// 改内容（不动 tab 与 sort）；返回是否命中行。
    pub fn update_prompt_item(&self, id: i64, content: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn.execute(
            "UPDATE prompt_items SET content = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, content, now_ms()],
        )? > 0)
    }

    pub fn delete_prompt_item(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn.execute("DELETE FROM prompt_items WHERE id = ?1", params![id])? > 0)
    }

    // -------------------------------------------------------- cache policies
    // 表在 V1 就有（scope/ref_id/settings，UNIQUE(scope,ref_id)）；对外接口用
    // 复合 scope 字符串 "global" | "model:<名>"，backend 层故意不暴露：
    // 缓存判定发生在选后端之前，后端维度对响应缓存无意义。

    /// 列出响应缓存策略覆盖行（scope → 策略 JSON）。
    pub fn list_cache_policies(&self) -> Result<Vec<(String, Value)>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn
            .prepare("SELECT scope, ref_id, settings FROM cache_policies ORDER BY scope, ref_id")?;
        let rows = stmt.query_map([], |r| {
            let scope: String = r.get(0)?;
            let ref_id: String = r.get(1)?;
            let settings: String = r.get(2)?;
            let full = if scope == "global" {
                "global".to_string()
            } else {
                format!("{scope}:{ref_id}")
            };
            Ok((full, serde_json::from_str(&settings).unwrap_or(Value::Null)))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn upsert_cache_policy(&self, scope: &str, policy: &Value) -> Result<()> {
        let (layer, ref_id) = split_policy_scope(scope);
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "INSERT INTO cache_policies (scope, ref_id, settings, updated_at) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(scope, ref_id) DO UPDATE SET settings = excluded.settings, updated_at = excluded.updated_at",
            params![layer, ref_id, policy.to_string(), now_ms()],
        )?;
        Ok(())
    }

    pub fn delete_cache_policy(&self, scope: &str) -> Result<bool> {
        let (layer, ref_id) = split_policy_scope(scope);
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn.execute(
            "DELETE FROM cache_policies WHERE scope = ?1 AND ref_id = ?2",
            params![layer, ref_id],
        )? > 0)
    }

    // --------------------------------------------------------------- affinity

    /// Note that `backend_id` served `prefix_hash` right now (§5.5.7).
    pub fn touch_affinity(&self, prefix_hash: &str, backend_id: i64) -> Result<()> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "INSERT INTO cache_affinity (prefix_hash, backend_id, confidence, last_seen)
             VALUES (?1, ?2, 1.0, ?3)
             ON CONFLICT(prefix_hash, backend_id) DO UPDATE SET confidence = 1.0, last_seen = excluded.last_seen",
            params![prefix_hash, backend_id, now_ms()],
        )?;
        Ok(())
    }

    /// Backends believed to hold `prefix_hash`, most recent first. Rows older
    /// than `ttl_secs` decay linearly toward 0 and then drop out.
    pub fn affinity_candidates(&self, prefix_hash: &str, ttl_secs: i64) -> Result<Vec<(i64, f64)>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let now = now_ms();
        let horizon = ttl_secs.max(1) * 1000;
        let mut stmt = conn.prepare(
            "SELECT backend_id, confidence, last_seen FROM cache_affinity WHERE prefix_hash = ?1",
        )?;
        let mut out = Vec::new();
        let rows = stmt.query_map(params![prefix_hash], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (backend_id, confidence, last_seen) = row?;
            let age = (now - last_seen).max(0) as f64;
            let weight = confidence * (1.0 - (age / horizon as f64)).max(0.0);
            if weight > 0.0 {
                out.push((backend_id, weight));
            }
        }
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }

    /// Drop affinity rows last seen before `cutoff_ms` (callers decide the
    /// retention window, typically `now - affinity.ttl_secs * 1000`).
    pub fn prune_affinity(&self, cutoff_ms: i64) -> Result<usize> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn.execute(
            "DELETE FROM cache_affinity WHERE last_seen < ?1",
            params![cutoff_ms],
        )?)
    }

    // ---------------------------------------------------------------- skills (M6)

    /// `skills` 表一行（SSOT 目录名即主键）。
    pub fn list_skills(&self) -> Result<Vec<SkillRecord>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT dir, name, description, repo, branch, version, latest, size, managed, updated_at
             FROM skills ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SkillRecord {
                dir: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                repo: r.get(3)?,
                branch: r.get(4)?,
                version: r.get(5)?,
                latest: r.get(6)?,
                size: r.get(7)?,
                managed: r.get::<_, i64>(8)? != 0,
                updated_at: r.get(9)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn get_skill(&self, dir: &str) -> Result<Option<SkillRecord>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn
            .query_row(
                "SELECT dir, name, description, repo, branch, version, latest, size, managed, updated_at
                 FROM skills WHERE dir = ?1",
                params![dir],
                |r| {
                    Ok(SkillRecord {
                        dir: r.get(0)?,
                        name: r.get(1)?,
                        description: r.get(2)?,
                        repo: r.get(3)?,
                        branch: r.get(4)?,
                        version: r.get(5)?,
                        latest: r.get(6)?,
                        size: r.get(7)?,
                        managed: r.get::<_, i64>(8)? != 0,
                        updated_at: r.get(9)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn upsert_skill(&self, rec: &SkillRecord) -> Result<()> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "INSERT INTO skills (dir, name, description, repo, branch, version, latest, size, managed, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(dir) DO UPDATE SET
                 name=excluded.name, description=excluded.description, repo=excluded.repo,
                 branch=excluded.branch, version=excluded.version, latest=excluded.latest,
                 size=excluded.size, managed=excluded.managed, updated_at=excluded.updated_at",
            params![
                rec.dir,
                rec.name,
                rec.description,
                rec.repo,
                rec.branch,
                rec.version,
                rec.latest,
                rec.size,
                rec.managed as i64,
                rec.updated_at
            ],
        )?;
        Ok(())
    }

    /// 删除技能行并连带其 skill_apps 开关行（同连接同调用内完成，不留孤儿）。
    pub fn delete_skill(&self, dir: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute("DELETE FROM skill_apps WHERE skill_dir = ?1", params![dir])?;
        conn.execute("DELETE FROM skills WHERE dir = ?1", params![dir])?;
        Ok(())
    }

    /// 启用开关行：`dir=None` 返回全部（resync 用），否则限定技能。
    pub fn list_skill_apps(&self, dir: Option<&str>) -> Result<Vec<SkillAppRow>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let sql = "SELECT skill_dir, app_id, enabled, sync_mode FROM skill_apps";
        let mut out = Vec::new();
        match dir {
            None => {
                let mut stmt = conn.prepare(sql)?;
                let rows = stmt.query_map([], Self::row_to_skill_app)?;
                for row in rows {
                    out.push(row?);
                }
            }
            Some(d) => {
                let mut stmt = conn.prepare(&format!("{sql} WHERE skill_dir = ?1"))?;
                let rows = stmt.query_map(params![d], Self::row_to_skill_app)?;
                for row in rows {
                    out.push(row?);
                }
            }
        }
        Ok(out)
    }

    fn row_to_skill_app(r: &rusqlite::Row<'_>) -> rusqlite::Result<SkillAppRow> {
        Ok(SkillAppRow {
            skill_dir: r.get(0)?,
            app_id: r.get(1)?,
            enabled: r.get::<_, i64>(2)? != 0,
            sync_mode: r.get(3)?,
        })
    }

    pub fn set_skill_app(
        &self,
        dir: &str,
        app: &str,
        enabled: bool,
        sync_mode: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "INSERT INTO skill_apps (skill_dir, app_id, enabled, sync_mode) VALUES (?1,?2,?3,?4)
             ON CONFLICT(skill_dir, app_id) DO UPDATE SET
                 enabled=excluded.enabled, sync_mode=excluded.sync_mode",
            params![dir, app, enabled as i64, sync_mode],
        )?;
        Ok(())
    }

    pub fn delete_skill_app(&self, dir: &str, app: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "DELETE FROM skill_apps WHERE skill_dir = ?1 AND app_id = ?2",
            params![dir, app],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ skill_repos (M6-B)

    pub fn list_skill_repos(&self) -> Result<Vec<SkillRepoRow>> {
        let conn = self.conn.lock().expect("store lock poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, owner, name, branch, enabled FROM skill_repos ORDER BY owner, name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(SkillRepoRow {
                id: r.get(0)?,
                owner: r.get(1)?,
                name: r.get(2)?,
                branch: r.get(3)?,
                enabled: r.get::<_, i64>(4)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// 同 (owner, name) 重复添加视为更新分支/启用态。
    pub fn add_skill_repo(&self, owner: &str, name: &str, branch: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store lock poisoned");
        conn.execute(
            "INSERT INTO skill_repos (owner, name, branch) VALUES (?1, ?2, ?3)
             ON CONFLICT(owner, name) DO UPDATE SET branch=excluded.branch, enabled=1",
            params![owner, name, branch],
        )?;
        Ok(())
    }

    pub fn delete_skill_repo(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn.execute("DELETE FROM skill_repos WHERE id = ?1", params![id])? > 0)
    }

    pub fn set_skill_repo_enabled(&self, id: i64, enabled: bool) -> Result<bool> {
        let conn = self.conn.lock().expect("store lock poisoned");
        Ok(conn.execute(
            "UPDATE skill_repos SET enabled = ?2 WHERE id = ?1",
            params![id, enabled as i64],
        )? > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Protocol;

    /// 全局声明行是否存在（行存在 ⇔ 任一字段写了非默认值）。
    fn has_meta(store: &Store, public_name: &str) -> bool {
        let conn = store.conn.lock().expect("store lock poisoned");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM model_meta WHERE public_name = ?1",
                params![public_name],
                |r| r.get(0),
            )
            .unwrap();
        n > 0
    }

    fn backend(name: &str) -> BackendUpsert {
        BackendUpsert {
            name: name.into(),
            base_url: "http://127.0.0.1:1234".into(),
            protocol: Protocol::OpenaiChat,
            weight: 2,
            enabled: true,
            models: vec!["m-one".into(), "m-two".into()],
            capabilities: serde_json::json!({"sampling": ["temperature"]}),
            ..Default::default()
        }
    }

    #[test]
    fn mask_sentinel_keeps_existing_secrets() {
        let store = Store::in_memory().unwrap();
        let mut seed = backend("lab");
        seed.api_key = Some("sk-real".into());
        seed.webui_password = Some("pw".into());
        store.upsert_backend(&seed).unwrap();

        // 编辑表单只能回显 "***"，原样送回必须保住库里原有的秘密。
        let mut edit = backend("lab");
        edit.api_key = Some(MASK.into());
        edit.webui_password = Some(MASK.into());
        edit.models = vec!["m-one".into()];
        store.upsert_backend(&edit).unwrap();
        let row = store.list_backends().unwrap().remove(0);
        assert_eq!(row.api_key.as_deref(), Some("sk-real"));
        assert_eq!(row.webui_password.as_deref(), Some("pw"));
        assert_eq!(row.models, vec!["m-one".to_string()]);

        // 真正清空（None）才会抹掉。
        store.upsert_backend(&backend("lab")).unwrap();
        let row = store.list_backends().unwrap().remove(0);
        assert_eq!(row.api_key, None);
        assert_eq!(row.webui_password, None);
    }

    #[test]
    fn backend_crud_roundtrip() {
        let store = Store::in_memory().unwrap();
        let id = store.upsert_backend(&backend("lab")).unwrap();
        assert!(id > 0);

        let list = store.list_backends().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            list[0].models,
            vec!["m-one".to_string(), "m-two".to_string()]
        );
        assert_eq!(list[0].weight, 2);
        assert!(list[0].enabled);

        // Upsert with the same name updates instead of duplicating.
        let mut updated = backend("lab");
        updated.weight = 5;
        updated.enabled = false;
        let id2 = store.upsert_backend(&updated).unwrap();
        assert_eq!(id, id2);
        assert_eq!(store.list_backends().unwrap()[0].weight, 5);

        assert!(store.set_backend_enabled("lab", true).unwrap());
        assert!(store.list_backends().unwrap()[0].enabled);
        assert!(store.delete_backend("lab").unwrap().is_some());
        assert!(store.list_backends().unwrap().is_empty());
    }

    /// 停用即清健康灯：周期探活只跑启用后端，停用不清就会让旧的 healthy 永远挂着。
    #[test]
    fn disable_backend_resets_health_state() {
        let store = Store::in_memory().unwrap();
        store.upsert_backend(&backend("lab")).unwrap();
        store.record_health("lab", true, None).unwrap();
        assert_eq!(store.list_backends().unwrap()[0].health_state, "healthy");

        store.set_backend_enabled("lab", false).unwrap();
        let row = &store.list_backends().unwrap()[0];
        assert!(!row.enabled);
        assert_eq!(row.health_state, "unknown", "停用后健康灯必须归零");
    }

    /// 直连开关独立于启用开关：三态里 停用/网关/直连 各写各的列，互不串台。
    #[test]
    fn set_backend_direct_toggles_column_without_touching_enabled() {
        let store = Store::in_memory().unwrap();
        store.upsert_backend(&backend("lab")).unwrap();
        assert!(!store.list_backends().unwrap()[0].direct, "缺省 = 网关");

        assert!(store.set_backend_direct("lab", true).unwrap());
        let row = &store.list_backends().unwrap()[0];
        assert!(row.direct);
        assert!(row.enabled, "开直连不该动启用");

        assert!(store.set_backend_direct("lab", false).unwrap());
        assert!(!store.list_backends().unwrap()[0].direct);
        assert!(
            !store.set_backend_direct("missing", true).unwrap(),
            "不存在的后端返回 false"
        );
    }

    /// 删后端必须连带清掉只属于它的配置，否则就是一堆再也点不到的孤儿行
    /// （参数中心照旧列着、改着，却对任何请求都不再生效——排查时格外咬人）。
    #[test]
    fn delete_backend_clears_only_its_own_config() {
        let store = Store::in_memory().unwrap();
        store.upsert_backend(&backend("lab")).unwrap();
        store.upsert_backend(&backend("peer")).unwrap();
        // 后端名带下划线：前缀匹配不能把 `lab_x` 的覆盖当成本家的清掉。
        let mut wide = backend("lab_x");
        wide.models = vec!["m-three".into()];
        store.upsert_backend(&wide).unwrap();

        // 后端层覆盖：ref_id = 后端名。
        ov(&store, "backend", "lab", "temperature", 0.2);
        ov(&store, "backend", "peer", "temperature", 0.9);
        ov(&store, "backend", "lab_x", "temperature", 0.5);
        // 模型层覆盖：ref_id = "<后端名>:<模型名>"。
        ov(&store, "model", "lab:m-one", "top_p", 0.8);
        ov(&store, "model", "peer:m-two", "top_p", 0.7);
        // m-one 被两台后端同时提供：全局声明是共有的，谁都不能替对方删。
        store.set_model_meta("m-one", 8192, 4096, 0, 0).unwrap();

        let out = store.delete_backend("lab").unwrap().unwrap();
        assert_eq!(out.dropped_params, 2, "后端层 1 条 + 模型层 1 条");
        assert_eq!(out.dropped_meta, 0, "peer 还在提供 m-one，声明不该动");

        let left: Vec<String> = store
            .list_param_overrides()
            .unwrap()
            .into_iter()
            .map(|o| format!("{}|{}", o.scope, o.ref_id))
            .collect();
        assert!(
            !left
                .iter()
                .any(|k| k == "backend|lab" || k == "model|lab:m-one"),
            "lab 自己的覆盖必须清干净：{left:?}"
        );
        assert!(
            left.contains(&"backend|peer".to_string())
                && left.contains(&"model|peer:m-two".to_string())
                && left.contains(&"backend|lab_x".to_string()),
            "别人的配置一个字都不能动：{left:?}"
        );
        // peer 仍在提供 m-one → 全局声明留着。
        assert!(has_meta(&store, "m-one"));
        // lab_x 与 lab 同前缀不同名，它的覆盖与声明都不该被 lab 的删除牵连。
        assert!(store.delete_backend("lab_x").unwrap().is_some());
        assert!(store.delete_backend("peer").unwrap().is_some());
        // 最后一台提供 m-one 的后端退场：声明随之消失，不留孤儿行。
        assert!(!has_meta(&store, "m-one"));
        assert!(store.list_param_overrides().unwrap().is_empty());

        // 不存在的后端：报 None，不是「删成功了」。
        assert!(store.delete_backend("lab").unwrap().is_none());
    }

    /// 写一条覆盖（id 由库里自增，这里只关心行本身）。
    fn ov(store: &Store, scope: &str, ref_id: &str, key: &str, v: f64) {
        store
            .upsert_param_override(&ParamOverride {
                id: 0,
                scope: scope.into(),
                ref_id: ref_id.into(),
                key: key.into(),
                value: serde_json::json!(v),
                policy: "fill".into(),
            })
            .unwrap();
    }

    /// 启停脚本与控制台入口同卡保存；脚本留空即清除，密码掩码哨兵语义不变。
    #[test]
    fn console_entry_roundtrips_scripts_and_keeps_masked_password() {
        let store = Store::in_memory().unwrap();
        store.upsert_backend(&backend("lab")).unwrap();

        assert!(store
            .set_backend_console("lab", Some("http://h:1"), None, None, Some("~/s.sh"), None)
            .unwrap());
        let row = store.list_backends().unwrap().remove(0);
        assert_eq!(row.start_script.as_deref(), Some("~/s.sh"));
        assert!(row.stop_script.is_none());

        // 密码掩码哨兵：编辑脚本时原样送回 "***"，密码不能被洗掉。
        store
            .set_backend_console("lab", None, None, Some("pw"), None, None)
            .unwrap();
        assert!(store
            .set_backend_console(
                "lab",
                None,
                None,
                Some(MASK),
                Some("/new.sh"),
                Some("/off.sh")
            )
            .unwrap());
        let row = store.list_backends().unwrap().remove(0);
        assert_eq!(
            row.webui_password.as_deref(),
            Some("pw"),
            "密码被掩码洗掉了"
        );
        assert_eq!(row.start_script.as_deref(), Some("/new.sh"));
        assert_eq!(row.stop_script.as_deref(), Some("/off.sh"));

        // 脚本留空 = 清除。
        assert!(store
            .set_backend_console("lab", None, None, None, Some("  "), Some(""))
            .unwrap());
        let row = store.list_backends().unwrap().remove(0);
        assert!(row.start_script.is_none() && row.stop_script.is_none());

        // 后端不存在：报 false，不是「保存成功了」。
        assert!(!store
            .set_backend_console("ghost", None, None, None, None, None)
            .unwrap());
    }

    /// 引擎切换 = engine/alt_engine 互换 + capabilities 重写为目标预设；
    /// 后续 upsert（编辑后端/播种）不得洗掉 alt_engine（UPDATE 列清单不含它）。
    #[test]
    fn swap_backend_engine_swaps_pair_and_survives_upsert() {
        let store = Store::in_memory().unwrap();
        store.upsert_backend(&backend("lan")).unwrap();
        // 播种行没有配对（alt_engine NULL）——无配对的后端不显示开关。
        assert!(store.list_backends().unwrap()[0].alt_engine.is_none());

        let gufo_caps = serde_json::json!({"speculative": {"algorithm": ["mtp"], "max_tokens": 7}});
        assert!(store
            .swap_backend_engine("lan", "gufo", Some("halogen"), &gufo_caps)
            .unwrap());
        let row = store.list_backends().unwrap().remove(0);
        assert_eq!(row.engine.as_deref(), Some("gufo"));
        assert_eq!(row.alt_engine.as_deref(), Some("halogen"));
        assert_eq!(
            row.capabilities["speculative"]["max_tokens"],
            serde_json::json!(7)
        );

        // 换回去。
        let hal_caps = serde_json::json!({"speculative": {"algorithm": ["ngram", "mtp"]}});
        assert!(store
            .swap_backend_engine("lan", "halogen", Some("gufo"), &hal_caps)
            .unwrap());
        let row = store.list_backends().unwrap().remove(0);
        assert_eq!(row.engine.as_deref(), Some("halogen"));
        assert_eq!(row.alt_engine.as_deref(), Some("gufo"));

        // 编辑后端走 upsert：UPDATE 不碰 alt_engine，配对必须活下来。
        store.upsert_backend(&backend("lan")).unwrap();
        let row = store.list_backends().unwrap().remove(0);
        assert_eq!(
            row.alt_engine.as_deref(),
            Some("gufo"),
            "upsert 洗掉了 alt_engine"
        );
    }

    #[test]
    fn usage_aggregates() {
        let store = Store::in_memory().unwrap();
        let mk = |model: &str, out: i64, ttft: i64| UsageRecord {
            request_id: format!("req-{model}-{out}"),
            ts: now_ms(),
            model: Some(model.into()),
            backend: Some("lab".into()),
            entry_protocol: Some("anthropic".into()),
            status: 200,
            ok: true,
            stream: true,
            ttft_ms: Some(ttft),
            duration_ms: Some(ttft + 900),
            input_tokens: 100,
            output_tokens: out,
            decode_tps: Some(42.0),
            ..Default::default()
        };
        store.record_usage(&mk("m-a", 10, 100)).unwrap();
        store.record_usage(&mk("m-a", 20, 300)).unwrap();
        store.record_usage(&mk("m-b", 30, 500)).unwrap();

        let s = store.usage_summary(0, now_ms() + 1000).unwrap();
        assert_eq!(s.requests, 3);
        assert_eq!(s.output_tokens, 60);
        assert_eq!(s.avg_ttft_ms.unwrap().round(), 300.0);

        let by_model = store.usage_by_model(0, now_ms() + 1000).unwrap();
        assert_eq!(by_model.len(), 2);
        assert_eq!(by_model[0].0, "m-a", "m-a has more total tokens");
    }

    #[test]
    fn usage_media_aggregates_metering() {
        let s = Store::in_memory().unwrap();
        let now = crate::types::now_ms();
        let mk = |model: &str, ep: &str, images: Option<i64>, seconds: Option<f64>| UsageRecord {
            request_id: format!("r-{model}-{ep}"),
            ts: now,
            backend: Some("lab".into()),
            model: Some(model.into()),
            entry_protocol: Some(ep.into()),
            status: 200,
            ok: true,
            media_images: images,
            media_seconds: seconds,
            ..Default::default()
        };
        s.record_usage(&mk("flux", "/v1/images/generations", Some(2), None))
            .unwrap();
        s.record_usage(&mk("flux", "/v1/images/generations", Some(3), None))
            .unwrap();
        s.record_usage(&mk("tts", "/v1/audio/speech", None, None))
            .unwrap();
        s.record_usage(&mk("asr", "/v1/audio/transcriptions", None, Some(61.5)))
            .unwrap();
        // 聊天行（entry_protocol 是协议名）绝不混进媒体聚合。
        s.record_usage(&mk("chat", "openai", None, None)).unwrap();

        let rows = s.usage_media(now - 1000, now + 1000).unwrap();
        assert_eq!(rows.len(), 3, "只聚合媒体行");
        let flux = rows
            .iter()
            .find(|r| r.model.as_deref() == Some("flux"))
            .unwrap();
        assert_eq!(flux.requests, 2);
        assert_eq!(flux.images, Some(5), "张数按请求累加");
        let tts = rows
            .iter()
            .find(|r| r.model.as_deref() == Some("tts"))
            .unwrap();
        assert_eq!(tts.images, None, "取不到 → None（UI 显示 —），不造假数字");
        assert_eq!(tts.seconds, None);
        let asr = rows
            .iter()
            .find(|r| r.model.as_deref() == Some("asr"))
            .unwrap();
        assert_eq!(asr.seconds, Some(61.5));
    }

    #[test]
    fn log_filters_and_purge() {
        let store = Store::in_memory().unwrap();
        let row = |lvl: &str, msg: &str, backend: &str| LogRow {
            id: 0,
            ts: now_ms(),
            level: lvl.into(),
            code: Some("RT100 route.pick".into()),
            message: msg.into(),
            request_id: Some("req-1".into()),
            backend: Some(backend.into()),
            model: None,
            extra: serde_json::json!({}),
        };
        store
            .insert_logs(&[
                row("info", "picked backend", "lab"),
                row("error", "upstream 502", "lab"),
            ])
            .unwrap();

        let all = store
            .query_logs(&LogQuery {
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(all.len(), 2);

        let errs = store
            .query_logs(&LogQuery {
                only_errors: true,
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].level, "error");

        let searched = store
            .query_logs(&LogQuery {
                text: Some("picked".into()),
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(searched.len(), 1);

        assert_eq!(store.purge_logs_before(now_ms() + 1000).unwrap(), 2);
        assert!(store.query_logs(&LogQuery::default()).unwrap().is_empty());
    }

    /// 队列⑤：日志按后端 / 模型过滤；过滤器选项（facets）来自日志真值 DISTINCT，
    /// 空值行不进选项、也不被过滤误伤。
    #[test]
    fn log_filters_by_backend_model_and_lists_facets() {
        let store = Store::in_memory().unwrap();
        let row = |backend: &str, model: &str| LogRow {
            id: 0,
            ts: now_ms(),
            level: "info".into(),
            code: None,
            message: "req".into(),
            request_id: None,
            backend: Some(backend.into()),
            model: Some(model.into()),
            extra: serde_json::json!({}),
        };
        store
            .insert_logs(&[
                row("lab", "m-a"),
                row("lab", "m-b"),
                row("b2", "m-a"),
                // 无后端 / 无模型的行：两个下拉都选不到它，也不该被过滤出来
                LogRow {
                    backend: None,
                    model: None,
                    ..row("x", "y")
                },
            ])
            .unwrap();
        let q = |backend: Option<&str>, model: Option<&str>| {
            store
                .query_logs(&LogQuery {
                    backend: backend.map(String::from),
                    model: model.map(String::from),
                    limit: 10,
                    ..Default::default()
                })
                .unwrap()
        };
        assert_eq!(q(Some("lab"), None).len(), 2);
        assert_eq!(q(None, Some("m-a")).len(), 2);
        assert_eq!(q(Some("lab"), Some("m-b")).len(), 1);
        // facets：字典序，空值不列
        let (backends, models) = store.log_facets().unwrap();
        assert_eq!(backends, vec!["b2".to_string(), "lab".to_string()]);
        assert_eq!(models, vec!["m-a".to_string(), "m-b".to_string()]);
    }

    #[test]
    fn affinity_decays_and_prunes() {
        let store = Store::in_memory().unwrap();
        store.touch_affinity("hash-a", 7).unwrap();
        let cands = store.affinity_candidates("hash-a", 1800).unwrap();
        assert_eq!(cands.len(), 1);
        assert!(cands[0].1 > 0.99);

        assert!(store
            .affinity_candidates("missing", 1800)
            .unwrap()
            .is_empty());
        // Nothing is older than "now minus a day".
        assert_eq!(store.prune_affinity(now_ms() - 86_400_000).unwrap(), 0);
        // Everything recorded before now is gone.
        assert_eq!(store.prune_affinity(now_ms() + 1).unwrap(), 1);
        assert!(store
            .affinity_candidates("hash-a", 1800)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn settings_roundtrip() {
        let store = Store::in_memory().unwrap();
        assert_eq!(store.get_setting("theme").unwrap(), None);
        store.set_setting("theme", "dark").unwrap();
        store.set_setting("theme", "light").unwrap();
        assert_eq!(
            store.get_setting("theme").unwrap().as_deref(),
            Some("light")
        );
    }

    #[test]
    fn usage_aggregations_m8b() {
        let s = Store::in_memory().unwrap();
        let now = crate::types::now_ms();
        // 10 条流式请求：ttft 1..=10 ms，缓存 3 hit / 2 miss，两个客户端。
        for i in 1..=10i64 {
            let u = UsageRecord {
                request_id: format!("r{i}"),
                ts: now,
                backend: Some("b1".into()),
                model: Some("gpt-4o".into()),
                entry_protocol: Some("openai".into()),
                client: Some(if i % 2 == 0 { "cli-a" } else { "cli-b" }.into()),
                status: 200,
                ok: true,
                stream: true,
                ttft_ms: Some(i),
                duration_ms: Some(100),
                input_tokens: 10,
                output_tokens: 20,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                decode_tps: Some(50.0),
                estimated: false,
                cache_status: match i {
                    1..=3 => Some("hit".into()),
                    4..=5 => Some("miss".into()),
                    _ => None,
                },
                retry_trace: None,
                error: None,
                media_images: None,
                media_seconds: None,
            };
            s.record_usage(&u).unwrap();
        }
        let (p50, p95) = s.usage_ttft_percentiles(now - 1000, now + 1000).unwrap();
        assert_eq!(p50, Some(5.0));
        assert_eq!(p95, Some(10.0));
        let c = s.usage_cache_counts(now - 1000, now + 1000).unwrap();
        assert_eq!(c["hit"], 3);
        assert_eq!(c["miss"], 2);
        assert_eq!(c["none"], 5);
        let cli = s.usage_by_client(now - 1000, now + 1000).unwrap();
        assert_eq!(cli.len(), 2);
        assert_eq!(cli[0].requests + cli[1].requests, 10);
        let ser = s.usage_series(now - 1000, now + 1000, "backend").unwrap();
        assert_eq!(ser[0].2, 10);
        assert!(s
            .usage_series(now - 1000, now + 1000, "evil; DROP")
            .is_err());
    }

    #[test]
    fn last_real_input_cached_takes_latest_true_report() {
        // ctx 先验（BUG-8 三修）：只认该 (backend, model) 最近一次上游真报输入；
        // 估算行（estimated=1）、失败行（ok=0）、别的后端行统统不作数。
        let s = Store::in_memory().unwrap();
        let now = crate::types::now_ms();
        let mk = |backend: &str, model: &str, ts: i64, input: i64, estimated: bool, ok: bool| {
            UsageRecord {
                request_id: format!("r-{model}-{ts}"),
                ts,
                backend: Some(backend.into()),
                model: Some(model.into()),
                entry_protocol: None,
                client: None,
                status: 200,
                ok,
                stream: true,
                ttft_ms: Some(10),
                duration_ms: Some(100),
                input_tokens: input,
                output_tokens: 5,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                decode_tps: None,
                estimated,
                cache_status: None,
                retry_trace: None,
                error: None,
                media_images: None,
                media_seconds: None,
            }
        };
        s.record_usage(&mk("b1", "m1", now - 3000, 111, false, true))
            .unwrap();
        // 比 444 更新，但是估算行 → 必须跳过（取错就把启发式虚高当先验）。
        s.record_usage(&mk("b1", "m1", now - 1000, 999, true, true))
            .unwrap();
        s.record_usage(&mk("b1", "m1", now - 500, 222, false, false))
            .unwrap();
        s.record_usage(&mk("b2", "m1", now, 333, false, true))
            .unwrap();
        s.record_usage(&mk("b1", "m1", now - 1500, 444, false, true))
            .unwrap();
        assert_eq!(s.last_real_input_cached("b1", "m1"), Some(444));
        assert_eq!(s.last_real_input_cached("b1", "no-such"), None);
    }

    // ── M12-A：模型级暴露开关与暴露名 ──────────────────────────────

    fn flags(store: &Store, backend: &str) -> Vec<(String, bool, bool)> {
        store
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == backend)
            .unwrap()
            .model_flags
            .into_iter()
            .map(|f| (f.public_name, f.exposed, f.accept_upstream))
            .collect()
    }

    #[test]
    fn model_flags_survive_repeated_upsert() {
        // 关键回归：yaml 种子每次启动都 upsert 一遍，旧实现整表重插会把用户
        // 关掉的模型又打开，开关形同虚设。
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        assert!(s.set_model_exposed("lab", "m-two", false).unwrap());
        s.upsert_backend(&backend("lab")).unwrap();
        let got = flags(&s, "lab");
        assert_eq!(
            got,
            vec![
                ("m-one".to_string(), true, false),
                ("m-two".to_string(), false, false)
            ],
            "重新 upsert 不该把开关洗回暴露"
        );
    }

    #[test]
    fn upsert_parks_models_that_are_gone() {
        // 摘挂不销毁：编辑保存的清单少一个模型 → 只从列摘下（列是服务真值），
        // 行保留 = 开关与配置继续暂存，重新出现即整套复活。
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        let mut slim = backend("lab");
        slim.models = vec!["m-one".into()];
        s.upsert_backend(&slim).unwrap();
        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert_eq!(row.models, vec!["m-one".to_string()], "出列 = 不再服务");
        assert_eq!(
            flags(&s, "lab"),
            vec![
                ("m-one".to_string(), true, false),
                ("m-two".to_string(), true, false)
            ],
            "游离行保留（不删）"
        );
    }

    #[test]
    fn rename_model_moves_everything_it_owns() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.upsert_param_override(&crate::types::ParamOverride {
            id: 0,
            scope: "model".into(),
            ref_id: "lab:m-one".into(),
            key: "temperature".into(),
            value: serde_json::json!(0.2),
            policy: "fill".into(),
        })
        .unwrap();
        s.upsert_cache_policy("model:m-one", &serde_json::json!({"ttl_secs": 60}))
            .unwrap();
        s.set_model_exposed("lab", "m-one", false).unwrap();

        let out = s.rename_model("lab", "m-one", "fast").unwrap();
        assert_eq!(out.upstream_name, "m-one");
        assert_eq!(out.migrated_params, 1);
        assert_eq!(out.migrated_cache_policies, 1);

        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert_eq!(row.models, vec!["fast".to_string(), "m-two".to_string()]);
        assert_eq!(row.aliases["fast"], serde_json::json!("m-one"));
        // 开关跟着名字走：改名前是隐藏的，改名后仍然隐藏
        assert_eq!(
            row.model_flags
                .iter()
                .find(|f| f.public_name == "fast")
                .map(|f| (f.upstream_name.as_str(), f.exposed)),
            Some(("m-one", false))
        );
        // 参数覆盖与缓存策略都换到了新作用域
        let pol: Vec<(String, serde_json::Value)> =
            s.list_cache_policies().unwrap().into_iter().collect();
        assert!(
            pol.iter().any(|(scope, _)| scope == "model:fast"),
            "{pol:?}"
        );
        assert!(!pol.iter().any(|(scope, _)| scope == "model:m-one"));
    }

    #[test]
    fn rename_rejects_unknown_and_duplicate_names() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        assert!(s.rename_model("lab", "ghost", "x").is_err());
        assert!(s.rename_model("lab", "m-one", "m-two").is_err());
        assert!(s.rename_model("lab", "m-one", "  ").is_err());
        assert!(s.rename_model("lab", "m-one", "m-one").is_err());
        assert!(s.rename_model("ghost", "m-one", "x").is_err());
    }

    #[test]
    fn rename_to_upstream_name_clears_alias() {
        // 改回去（暴露名 == 上游名）时不该留下自指向的别名。
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.rename_model("lab", "m-one", "fast").unwrap();
        assert_eq!(
            s.rename_model("lab", "fast", "m-one")
                .unwrap()
                .upstream_name,
            "m-one"
        );
        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert!(row.aliases.get("m-one").is_none(), "{:?}", row.aliases);
    }

    #[test]
    fn remove_model_drops_its_config_too() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.upsert_param_override(&crate::types::ParamOverride {
            id: 0,
            scope: "model".into(),
            ref_id: "lab:m-two".into(),
            key: "temperature".into(),
            value: serde_json::json!(0.9),
            policy: "fill".into(),
        })
        .unwrap();
        let out = s.remove_model("lab", "m-two").unwrap();
        assert_eq!(out.dropped_params, 1);
        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert_eq!(row.models, vec!["m-one".to_string()]);
        assert!(s.remove_model("lab", "m-two").is_err());
    }

    // ── 模型元数据（ctx/output）─────────────────────────────────────────

    /// 后端层同键参数（scope=backend，ref_id=后端名）。
    fn backend_param(s: &Store, backend: &str, key: &str, v: i64) {
        s.upsert_param_override(&crate::types::ParamOverride {
            id: 0,
            scope: "backend".into(),
            ref_id: backend.into(),
            key: key.into(),
            value: serde_json::json!(v),
            policy: "fill".into(),
        })
        .unwrap();
    }

    #[test]
    fn meta_auto_chain_param_then_probe_then_min_across_backends() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();

        // ① 全空 = 未知不猜。
        assert_eq!(
            s.effective_model_meta().unwrap().get("m-one"),
            None,
            "没任何真值就不该有记录"
        );

        // ② 探测值（sync 写入）= 后端真值。
        s.sync_backend_models(
            "lab",
            &[crate::types::ScannedModel {
                id: "m-one".into(),
                ctx: Some(131_072),
                output: Some(8_192),
                reasoning: None,
                vlm: None,
            }],
        )
        .unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.ctx, Some(131_072));
        assert_eq!(m.ctx_source, crate::types::MetaSource::Backend);

        // ③ 后端层参数压过探测值。
        backend_param(&s, "lab", "ctx", 65_536);
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.ctx, Some(65_536));

        // ④ 跨后端取最小：新后端只给探测 32k。
        let mut other = backend("lab");
        other.name = "lab2".into();
        other.models = vec!["m-one".into()];
        s.upsert_backend(&other).unwrap();
        s.sync_backend_models(
            "lab2",
            &[crate::types::ScannedModel {
                id: "m-one".into(),
                ctx: Some(32_768),
                output: None,
                reasoning: None,
                vlm: None,
            }],
        )
        .unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.ctx, Some(32_768), "跨后端取最小");
        assert_eq!(m.output, Some(8_192), "只有 lab 给了 output，原样");
    }

    #[test]
    fn meta_manual_wins_and_clamps_to_backend_cap() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.sync_backend_models(
            "lab",
            &[
                crate::types::ScannedModel {
                    id: "m-one".into(),
                    ctx: Some(131_072),
                    output: None,
                    reasoning: None,
                    vlm: None,
                },
                // m-two 上游没给真值 → cap 未知，声明不拦。
                crate::types::ScannedModel {
                    id: "m-two".into(),
                    ctx: None,
                    output: None,
                    reasoning: None,
                    vlm: None,
                },
            ],
        )
        .unwrap();

        // 手工非 0 = 强制覆盖：声明小于后端上限，按声明走。
        let out = s.set_model_meta("m-one", 32_000, 0, 0, 0).unwrap();
        assert_eq!(out.ctx, 32_000);
        assert!(!out.clamped_ctx);
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.ctx, Some(32_000));
        assert_eq!(m.ctx_source, crate::types::MetaSource::Manual);

        // 超后端上限 = 钳到边界并报告。
        let out = s.set_model_meta("m-one", 999_999, 0, 0, 0).unwrap();
        assert_eq!(out.ctx, 131_072);
        assert!(out.clamped_ctx);
        assert_eq!(out.cap_ctx, Some(131_072));
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.ctx, Some(131_072));

        // 后端无从得知（cap=None）时不限制。
        let out = s.set_model_meta("m-two", 42_000, 7_777, 0, 0).unwrap();
        assert_eq!((out.ctx, out.output), (42_000, 7_777));
        assert!(!out.clamped_ctx && !out.clamped_output);

        // 0 = 回到自动（删行），探测值接管。
        let out = s.set_model_meta("m-one", 0, 0, 0, 0).unwrap();
        assert_eq!((out.ctx, out.output), (0, 0));
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(
            m.ctx_source,
            crate::types::MetaSource::Backend,
            "删行后回到自动"
        );

        // 负数与不存在的模型是硬错误。
        assert!(s.set_model_meta("m-one", -1, 0, 0, 0).is_err());
        assert!(s.set_model_meta("ghost", 1, 0, 0, 0).is_err());
    }

    #[test]
    fn meta_cache_serves_fast_and_invalidates_on_writes() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.sync_backend_models(
            "lab",
            &[crate::types::ScannedModel {
                id: "m-one".into(),
                ctx: Some(131_072),
                output: None,
                reasoning: None,
                vlm: None,
            }],
        )
        .unwrap();
        // 冷路径 = 现算；TTL 内热路径 = 同一份缓存值。
        assert_eq!(
            s.effective_model_meta_cached().unwrap()["m-one"].ctx,
            Some(131_072)
        );
        assert_eq!(
            s.effective_model_meta_cached().unwrap()["m-one"].ctx,
            Some(131_072)
        );
        // 配置面写操作必须立刻反映（缓存被主动打掉）——漏失效就会拿旧值剥错图。
        s.set_model_meta("m-one", 64_000, 0, 0, 0).unwrap();
        let m = s.effective_model_meta_cached().unwrap()["m-one"];
        assert_eq!(m.ctx, Some(64_000));
        assert_eq!(m.ctx_source, crate::types::MetaSource::Manual);
        // 清零人工声明 → 立刻回到后端真值（再一次证明失效即时）。
        s.set_model_meta("m-one", 0, 0, 0, 0).unwrap();
        assert_eq!(
            s.effective_model_meta_cached().unwrap()["m-one"].ctx,
            Some(131_072)
        );
    }

    #[test]
    fn meta_bool_dims_fold_across_backends_and_manual_tristate() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.sync_backend_models(
            "lab",
            &[crate::types::ScannedModel {
                id: "m-one".into(),
                ctx: None,
                output: None,
                reasoning: Some(true),
                vlm: Some(true),
            }],
        )
        .unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.reasoning, Some(true));
        assert_eq!(m.reasoning_source, crate::types::MetaSource::Backend);
        assert_eq!(m.vlm, Some(true));

        // 第二个后端明说 VLM 关（量化裁剪丢视觉）：跨后端折叠取保守 = 关。
        let mut other = backend("lab");
        other.name = "lab2".into();
        s.upsert_backend(&other).unwrap();
        s.set_probe_values(
            "lab2",
            &[crate::types::ScannedModel {
                id: "m-one".into(),
                ctx: None,
                output: None,
                reasoning: None,
                vlm: Some(false),
            }],
        )
        .unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.vlm, Some(false), "任一后端明说关 = 关");
        assert_eq!(m.reasoning, Some(true), "另一侧未知不抹已知");

        // 人工三态压倒折叠：1 = 强制开。
        s.set_model_meta("m-one", 0, 0, 0, 1).unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.vlm, Some(true));
        assert_eq!(m.vlm_source, crate::types::MetaSource::Manual);
        // 2 = 强制关（盖过探测的 true）。
        s.set_model_meta("m-one", 0, 0, 2, 2).unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.reasoning, Some(false));
        assert_eq!(m.vlm, Some(false));
        // 全 0 = 回到自动：行删掉，折叠真值接管。
        s.set_model_meta("m-one", 0, 0, 0, 0).unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.vlm, Some(false));
        assert_eq!(m.vlm_source, crate::types::MetaSource::Backend);
        // 非法三态拒绝。
        assert!(s.set_model_meta("m-one", 0, 0, 3, 0).is_err());
    }

    #[test]
    fn meta_probe_never_overwrites_manual_and_coalesces() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.set_model_meta("m-one", 64_000, 0, 0, 0).unwrap();

        // 再怎么探测，人工声明不动。
        s.sync_backend_models(
            "lab",
            &[crate::types::ScannedModel {
                id: "m-one".into(),
                ctx: Some(131_072),
                output: Some(8_192),
                reasoning: None,
                vlm: None,
            }],
        )
        .unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(
            (m.ctx, m.ctx_source),
            (Some(64_000), crate::types::MetaSource::Manual)
        );

        // 这次扫描没带回 ctx（None）：既有探测真值保留，不清空。
        s.set_model_meta("m-one", 0, 0, 0, 0).unwrap();
        s.sync_backend_models(
            "lab",
            &[crate::types::ScannedModel {
                id: "m-one".into(),
                ctx: None,
                output: Some(4_096),
                reasoning: None,
                vlm: None,
            }],
        )
        .unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.ctx, Some(131_072), "COALESCE：拿不到就不清空");
        assert_eq!(m.output, Some(4_096), "新值有就更新");
    }

    #[test]
    fn meta_follows_rename_and_dies_with_last_user() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.set_model_meta("m-one", 50_000, 4_000, 0, 0).unwrap();

        // 两个后端共享同一个对外名（同模型双路）。
        let mut other = backend("lab");
        other.name = "lab2".into();
        other.models = vec!["m-one".into()];
        s.upsert_backend(&other).unwrap();

        // 在 lab 上改名：lab2 还暴露旧名 → 旧名保留 + 新名复制。
        s.rename_model("lab", "m-one", "fast").unwrap();
        let meta = s.effective_model_meta().unwrap();
        assert_eq!(meta["m-one"].manual_ctx, 50_000, "lab2 还在用，留着");
        assert_eq!(meta["fast"].manual_ctx, 50_000, "新名复制一份");
        assert_eq!(meta["fast"].manual_output, 4_000);

        // 摘掉 lab2 的旧名：没人再用 m-one → 声明随之消失。
        let out = s.remove_model("lab2", "m-one").unwrap();
        assert_eq!(out.dropped_meta, 1, "最后一位用户摘掉 = 孤儿清理");
        let meta = s.effective_model_meta().unwrap();
        assert!(!meta.contains_key("m-one"), "无人再用，声明删掉");
        assert_eq!(meta["fast"].manual_ctx, 50_000, "还在用的不受影响");

        // lab 单独改名（无人共享）：整行移动，不留旧名。
        s.rename_model("lab", "fast", "faster").unwrap();
        let meta = s.effective_model_meta().unwrap();
        assert!(!meta.contains_key("fast"));
        assert_eq!(meta["faster"].manual_ctx, 50_000);
    }

    #[test]
    fn meta_sync_diff_reports_and_survives_upsert_seed() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.set_model_meta("m-one", 50_000, 0, 0, 0).unwrap();

        // 刷新后上游少了 m-two：只从清单摘下（diff 战报照报），行不删、配置暂存。
        let out = s
            .sync_backend_models(
                "lab",
                &[crate::types::ScannedModel {
                    id: "m-one".into(),
                    ctx: None,
                    output: None,
                    reasoning: None,
                    vlm: None,
                }],
            )
            .unwrap();
        assert_eq!(out.removed, vec!["m-two".to_string()]);
        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert!(
            row.model_flags.iter().any(|f| f.public_name == "m-two"),
            "游离行保留"
        );

        // yaml 种子 upsert 不该动人工声明，也不该清探测列。
        let mut seed = backend("lab");
        seed.models = vec!["m-one".into()];
        s.upsert_backend(&seed).unwrap();
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!(m.manual_ctx, 50_000);
    }

    /// 一条上游扫描结果（测试里只用 id，真值列全空）。
    fn scan(id: &str) -> crate::types::ScannedModel {
        crate::types::ScannedModel {
            id: id.into(),
            ctx: None,
            output: None,
            reasoning: None,
            vlm: None,
        }
    }

    /// 摘挂不销毁（用户实报 bug 的回归钉子）：上游列表抖一次（如引擎重启中返回
    /// 不完整清单），被摘模型的行 / 别名 / 参数覆盖全部暂存；上游恢复后按友好名
    /// 挂回，配置与过渡开关整套复活，游离期间②③通路不服务流量。
    #[test]
    fn sync_detach_parks_config_and_resurrects_friendly_name() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        // m-two 改名 → fast（别名 fast → m-two、行改名、参数 ref 跟着走）。
        s.rename_model("lab", "m-two", "fast").unwrap();
        s.upsert_param_override(&crate::types::ParamOverride {
            id: 0,
            scope: "model".into(),
            ref_id: "lab:fast".into(),
            key: "temperature".into(),
            value: serde_json::json!(0.7),
            policy: "fill".into(),
        })
        .unwrap();
        s.set_model_accept_upstream("lab", "fast", true).unwrap();

        // 上游抖动：这次只报出 m-one → fast 摘下（只出列，不销毁任何东西）。
        let out = s.sync_backend_models("lab", &[scan("m-one")]).unwrap();
        assert_eq!(out.removed, vec!["fast".to_string()]);
        assert!(out.added.is_empty());
        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert_eq!(row.models, vec!["m-one".to_string()]);
        assert!(
            row.aliases.get("fast").is_some(),
            "别名保留 = 挂回认亲的凭据"
        );
        assert!(
            row.model_flags.iter().any(|f| f.public_name == "fast"),
            "行保留 = 开关与探测值的载体"
        );
        assert!(
            s.list_param_overrides()
                .unwrap()
                .iter()
                .any(|o| o.ref_id == "lab:fast"),
            "参数覆盖暂存，不随摘除销毁"
        );
        // 游离期间不服务流量：②别名、③上游原名（过渡开关开着）都不认，硬打干净 404。
        assert_eq!(row.resolve_upstream("fast"), None, "② 游离别名不路由");
        assert_eq!(
            row.resolve_upstream("m-two"),
            None,
            "③ 游离行的过渡名不路由"
        );

        // 上游恢复（这次扫全 m-one + m-two）→ m-two 反查别名按友好名挂回。
        let out = s
            .sync_backend_models("lab", &[scan("m-one"), scan("m-two")])
            .unwrap();
        assert_eq!(
            out.added,
            vec!["fast".to_string()],
            "友好名复活，不以裸上游名回列"
        );
        assert!(out.removed.is_empty());
        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert_eq!(row.models, vec!["fast".to_string(), "m-one".to_string()]);
        assert_eq!(row.resolve_upstream("fast"), Some("m-two".to_string()));
        assert_eq!(
            row.resolve_upstream("m-two"),
            Some("m-two".to_string()),
            "过渡开关随行复活"
        );
        assert!(
            s.list_param_overrides()
                .unwrap()
                .iter()
                .any(|o| o.ref_id == "lab:fast"),
            "参数覆盖复活"
        );
    }

    /// 编辑路径同语义：对话框里摘掉的模型只出列，行与配置暂存；重新勾选即整套复活
    /// （用户关掉的开关不会复活成开）。
    #[test]
    fn upsert_edit_detach_keeps_row_and_config() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.upsert_param_override(&crate::types::ParamOverride {
            id: 0,
            scope: "model".into(),
            ref_id: "lab:m-two".into(),
            key: "temperature".into(),
            value: serde_json::json!(0.7),
            policy: "fill".into(),
        })
        .unwrap();
        s.set_model_exposed("lab", "m-two", false).unwrap();

        // 编辑保存时清单里没有 m-two：列更新，行与配置原地暂存。
        let mut edit = backend("lab");
        edit.models = vec!["m-one".into()];
        s.upsert_backend(&edit).unwrap();
        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert_eq!(row.models, vec!["m-one".to_string()]);
        assert!(
            row.model_flags
                .iter()
                .any(|f| f.public_name == "m-two" && !f.exposed),
            "行与开关保留"
        );
        assert!(
            s.list_param_overrides()
                .unwrap()
                .iter()
                .any(|o| o.ref_id == "lab:m-two"),
            "参数覆盖暂存"
        );
        assert_eq!(row.resolve_upstream("m-two"), None, "游离不路由");

        // 重新勾选保存：ON CONFLICT 命中暂存行，开关与参数整套复活。
        s.upsert_backend(&backend("lab")).unwrap();
        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert_eq!(row.models, vec!["m-one".to_string(), "m-two".to_string()]);
        assert!(!row.is_exposed("m-two"), "用户关掉的开关不复活成开");
        assert!(
            s.list_param_overrides()
                .unwrap()
                .iter()
                .any(|o| o.ref_id == "lab:m-two"),
            "参数覆盖复活"
        );
    }

    #[test]
    fn save_path_probe_fills_values_but_never_the_list() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        // m-two 改名成 fast（别名 fast → m-two），扫描回来的仍是上游名。
        s.rename_model("lab", "m-two", "fast").unwrap();

        // 清单外的 id 忽略；公开名经别名也认；0 不算真值。
        let applied = s
            .set_probe_values(
                "lab",
                &[
                    crate::types::ScannedModel {
                        id: "m-one".into(),
                        ctx: Some(131_072),
                        output: Some(8_192),
                        reasoning: None,
                        vlm: None,
                    },
                    crate::types::ScannedModel {
                        id: "m-two".into(),
                        ctx: Some(65_536),
                        output: None,
                        reasoning: None,
                        vlm: None,
                    },
                    crate::types::ScannedModel {
                        id: "ghost".into(),
                        ctx: Some(1),
                        output: None,
                        reasoning: None,
                        vlm: None,
                    },
                    crate::types::ScannedModel {
                        id: "fast".into(),
                        ctx: Some(0),
                        output: None,
                        reasoning: None,
                        vlm: None,
                    },
                ],
            )
            .unwrap();
        assert_eq!(applied, 2, "只落在册的 m-one 与改名后的 fast");
        let meta = s.effective_model_meta().unwrap();
        assert_eq!(meta["m-one"].ctx, Some(131_072));
        assert_eq!(meta["fast"].ctx, Some(65_536));

        // 清单一个没动（ghost 没被加进来）。
        let row = s
            .list_backends()
            .unwrap()
            .into_iter()
            .find(|b| b.name == "lab")
            .unwrap();
        assert_eq!(
            row.models,
            vec!["m-one".to_string(), "fast".to_string()],
            "清单原位替换，一个没动"
        );

        // COALESCE：这次没带回 ctx → 既有真值保留；有新 output → 更新。
        let applied = s
            .set_probe_values(
                "lab",
                &[crate::types::ScannedModel {
                    id: "m-one".into(),
                    ctx: None,
                    output: Some(4_096),
                    reasoning: None,
                    vlm: None,
                }],
            )
            .unwrap();
        assert_eq!(applied, 1);
        let m = s.effective_model_meta().unwrap()["m-one"];
        assert_eq!((m.ctx, m.output), (Some(131_072), Some(4_096)));

        // 后端不存在：静默 0（保存流程不该被这一步卡死）。
        assert_eq!(
            s.set_probe_values("ghost-backend", &[crate::types::ScannedModel::default()])
                .unwrap(),
            0
        );
        assert_eq!(s.set_probe_values("lab", &[]).unwrap(), 0);
    }

    /// 工种人工声明（11a）：声明落库、清除回落 None、未知行不命中。
    #[test]
    fn set_model_kind_declaration_and_clear() {
        let s = Store::in_memory().unwrap();
        s.upsert_backend(&backend("lab")).unwrap();
        s.sync_backend_models("lab", &[scan("m-one")]).unwrap();
        // 初始未声明。
        let flag = |s: &Store| -> Option<String> {
            s.list_backends()
                .unwrap()
                .into_iter()
                .find(|b| b.name == "lab")
                .unwrap()
                .model_flags
                .into_iter()
                .find(|f| f.public_name == "m-one")
                .and_then(|f| f.kind)
        };
        assert_eq!(flag(&s), None);
        // 声明 → 落库；清除 → 回 None（回落判定链）。
        assert!(s.set_model_kind("lab", "m-one", Some("image_gen")).unwrap());
        assert_eq!(flag(&s), Some("image_gen".to_string()));
        assert!(s.set_model_kind("lab", "m-one", None).unwrap());
        assert_eq!(flag(&s), None);
        // 未知模型 / 未知后端：UPDATE 不命中，返回 false 不报错。
        assert!(!s.set_model_kind("lab", "ghost", Some("tts")).unwrap());
        assert!(!s
            .set_model_kind("ghost-backend", "m-one", Some("tts"))
            .unwrap());
    }

    #[test]
    fn prompt_items_crud_and_tab_ordering() {
        let s = Store::in_memory().unwrap();
        assert!(s.list_prompt_items().unwrap().is_empty());

        let a = s.create_prompt_item("common", "第一条").unwrap();
        let b = s.create_prompt_item("common", "第二条").unwrap();
        s.create_prompt_item("codex", "codex 快捷命令").unwrap();
        let rows = s.list_prompt_items().unwrap();
        // tab 定序（BINARY 序：codex < common），tab 内按 sort 追加序。
        assert_eq!(
            rows.iter().map(|r| r.content.as_str()).collect::<Vec<_>>(),
            vec!["codex 快捷命令", "第一条", "第二条"]
        );
        assert_eq!(rows[0].sort, 0, "各 tab 的 sort 独立起算");
        assert_eq!(rows[1].sort, 0);
        assert_eq!(rows[2].sort, 1);

        assert!(s.update_prompt_item(b, "改过的第二条").unwrap());
        assert!(!s.update_prompt_item(9999, "不存在").unwrap());
        assert!(s
            .list_prompt_items()
            .unwrap()
            .iter()
            .any(|r| r.id == b && r.content == "改过的第二条"));

        assert!(s.delete_prompt_item(a).unwrap());
        assert!(!s.delete_prompt_item(a).unwrap());
        assert_eq!(s.list_prompt_items().unwrap().len(), 2);
    }
}
