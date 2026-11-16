//! Tauri IPC surface. Logic lives in `services`/`store`; these are thin
//! serialization shells so the same behavior is testable without a window.

use crate::config::Config;
use crate::gateway::State;
use crate::services::backend::{self, ProbeOutcome};
use crate::services::script;
use crate::store::Store;
use crate::types::{now_ms, BackendUpsert, Protocol};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State as TauriState;

fn mask_keys(mut rows: Vec<crate::types::BackendRow>) -> Vec<crate::types::BackendRow> {
    for r in &mut rows {
        r.api_key = r.api_key.as_ref().map(|_| "***".to_string());
        r.webui_password = r.webui_password.as_ref().map(|_| "***".to_string());
    }
    rows
}

/// 持久化：把 DB 后端镜像回 config.yaml（委托 `config::persist_backends_to_yaml`）。
/// 根因与口径见该函数注释。这里只是薄封装，让本模块调用点保持短。
pub(crate) fn persist_config_yaml(config: &Arc<Config>, store: &Arc<Store>) -> Result<(), String> {
    crate::config::persist_backends_to_yaml(config, store)
}

#[tauri::command]
pub fn list_backends(st: TauriState<Arc<State>>) -> Result<Value, String> {
    let rows = mask_keys(st.store.list_backends().map_err(|e| e.to_string())?);
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

/// 控制台左列用：每个后端此刻在飞的请求数。
///
/// 口径 = 网关 monitor 的活跃流注册表（`count_by_backend`），即「此刻正在处理的上游
/// 请求」，与历史用量无关。前端据此点亮/压暗左列条目：>0 点亮（有数据流），=0 压暗。
///
/// 返回 `{ "后端名": 在飞请求数, ... }`；无活跃流时为空对象 `{}`。
#[tauri::command]
pub fn console_flow(st: TauriState<Arc<State>>) -> Result<Value, String> {
    serde_json::to_value(st.monitor.count_by_backend()).map_err(|e| e.to_string())
}

#[derive(Debug, Deserialize)]
pub struct BackendInput {
    pub name: String,
    pub base_url: String,
    /// **编辑态才带**：被编辑那一行原来的名字。
    ///
    /// 这个字段是「新增」和「覆盖」的分水岭。`Store::upsert_backend` 按名字整行
    /// UPDATE，所以以前新增时随手填了个已存在的名字，结果是**静默覆盖**那条后端：
    /// 列表里不出现新后端、原名那条的 base_url 被换掉、模型清单被表单里的空表洗空
    /// （实报：用户以为「加不上后端」）。带上原来的名字才认作编辑，否则同名字即报错。
    #[serde(default)]
    pub original_name: Option<String>,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub weight: i64,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    /// 「扫描模型」顺带带回的 ctx/output 真值：保存时一并落探测列，
    /// 新增后端不必再点一次 ⟳ 才看得到 ctx（只落值，不动清单）。
    #[serde(default)]
    pub probe: Vec<crate::types::ScannedModel>,
    #[serde(default)]
    pub aliases: Value,
    #[serde(default)]
    pub webui_url: Option<String>,
    #[serde(default)]
    pub webui_user: Option<String>,
    #[serde(default)]
    pub webui_password: Option<String>,
}

fn yes() -> bool {
    true
}

/// 「这次保存是新增还是编辑」的判据（纯函数）。
///
/// 名字是后端的主键语义（`Store::upsert_backend` 按名字整行 UPDATE），所以：
/// - 带了 `original_name` = 编辑既有行（UI 里编辑态名称锁死，正常等于 name）；
/// - 没带 = 新增，撞上已有名字就必须**当场拒绝**，否则会把那条后端静默覆盖：
///   新后端不出现、老后端的 base_url 被换、模型清单被表单里的空表洗空。
fn check_target(name: &str, original_name: Option<&str>, exists: bool) -> Result<(), String> {
    if let Some(orig) = original_name.map(str::trim).filter(|o| !o.is_empty()) {
        if orig != name {
            return Err(format!(
                "不支持改名（`{orig}` → `{name}`）：后端按名称定位，改名请先删除再新建"
            ));
        }
        return Ok(());
    }
    if exists {
        return Err(format!(
            "后端 `{name}` 已存在：要改它请点该行右侧的 ✎ 编辑，新增请换个名字"
        ));
    }
    Ok(())
}

/// 表单入参归一化 + 校验（纯函数，便于单测）。
///
/// 以前空名、`localhost:8000`（漏了 scheme）、带前后空白的 URL 都能直接落库：
/// 落进去就是一条**永远探不活**的死行，看起来就是「后端加了但不正常」。
/// 校验放在落库前，报错直接指着该改的那个字段说。
fn normalize_input(input: &BackendInput) -> Result<(String, String), String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("后端名不能为空".into());
    }
    if name.chars().any(char::is_control) {
        return Err(format!("后端名不能含换行/控制字符：`{name:?}`"));
    }
    let base = input.base_url.trim().to_string();
    if base.is_empty() {
        return Err("Base URL 不能为空".into());
    }
    let lower = base.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(format!(
            "Base URL 要带 http:// 或 https:// 前缀：`{base}`（例：http://127.0.0.1:8000 或 http://127.0.0.1:8000/v1）"
        ));
    }
    // 主机名不能空：`http://` / `http:///v1` 这类只写了 scheme 的也算无效。
    // 这一条必须排在去尾斜杠**之前**：`http://` 先被 trim_end_matches('/') 削成
    // `http:`，scheme 校验会先炸，报错指成「缺前缀」，把真正的原因盖掉了。
    let host = base
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or(""))
        .unwrap_or("");
    if host.is_empty() {
        return Err(format!("Base URL 缺主机名：`{base}`"));
    }
    // 去尾斜杠留到所有校验之后：`http://127.0.0.1:8000/v1/` → `.../v1`。
    let base = base.trim_end_matches('/').to_string();
    Ok((name, base))
}

/// `BackendInput`（前端表单）→ `BackendUpsert`（整行 UPDATE 的入参）。
///
/// **保留列（P1）**：`Store::upsert_backend` 是整行 UPDATE，而 `health_path` /
/// `capabilities` / `params` 这三列只有探测、引擎预设与 yaml 种子会写——前端的
/// `BackendInput` 里根本没有它们。不从既有行回填就等于「保存一次后端就把健康检查
/// 路径与能力表清空」（json!({}) 也一样，那是拿空对象盖掉真值）。
fn build_upsert(input: BackendInput, prev: Option<&crate::types::BackendRow>) -> BackendUpsert {
    // 新增行没有旧值可继承：给**空对象**而不是 JSON `null`。
    // 库里的三列都是 `TEXT NOT NULL DEFAULT '{}'`，前端表单也没这三列——写 null 等于
    // 拿 "null" 这串字符串盖掉默认值，于是新增的后端能力表/参数/别名在 DB、在
    // config.yaml 里全是 null（后续读方按对象取用就拿到空，语义上等于「没有声明」，
    // 但落盘口径和 schema 不一致，导出/对比时一片 null）。语义上 {} 与 null 等价，
    // 落盘口径统一用 {}。
    let fresh = || json!({});
    BackendUpsert {
        name: input.name,
        base_url: input.base_url,
        protocol: Protocol::parse(&input.protocol).unwrap_or_default(),
        api_key: input.api_key.filter(|k| !k.is_empty()),
        weight: if input.weight <= 0 { 1 } else { input.weight },
        enabled: input.enabled,
        engine: input.engine,
        webui_url: input.webui_url.filter(|s| !s.trim().is_empty()),
        webui_user: input.webui_user.filter(|s| !s.trim().is_empty()),
        webui_password: input.webui_password.filter(|s| !s.is_empty()),
        health_path: prev.and_then(|p| p.health_path.clone()),
        models: input.models,
        aliases: if input.aliases.is_object() {
            input.aliases
        } else {
            fresh()
        },
        capabilities: prev.map(|p| p.capabilities.clone()).unwrap_or_else(fresh),
        params: prev.map(|p| p.params.clone()).unwrap_or_else(fresh),
    }
}

#[tauri::command]
pub fn upsert_backend(st: TauriState<Arc<State>>, mut input: BackendInput) -> Result<i64, String> {
    // 先摘出来：下面 upsert 会把 input 逐字段搬走，而真值要落列还得用后端名。
    let probe = std::mem::take(&mut input.probe);
    let (name, base_url) = normalize_input(&input)?;
    let prev = st.store.backend_by_name(&name).map_err(|e| e.to_string())?;
    check_target(&name, input.original_name.as_deref(), prev.is_some())?;
    input.name = name;
    input.base_url = base_url;
    let upsert = build_upsert(input, prev.as_ref());
    let id = st
        .store
        .upsert_backend(&upsert)
        .map_err(|e| e.to_string())?;
    if !probe.is_empty() {
        // 落真值失败不回滚保存（清单已写好），⟳ 刷新还能再落一次。
        if let Err(e) = st.store.set_probe_values(&upsert.name, &probe) {
            tracing::warn!(target: "llm_switch::backend", backend = %upsert.name, "probe values persist failed: {e}");
        } else {
            resync_takeovers(st.inner());
        }
    }
    // 持久化：改后端要回写 config.yaml，否则启动时 seed_backends 会复活/覆盖。
    if let Err(e) = persist_config_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::backend", backend = %upsert.name, "persist config.yaml failed: {e}");
    }
    Ok(id)
}

/// 元数据变化（探测落真值 / 人工改参）后的统一收尾：把受管端的接管配置
/// 自动重写（无差异不写盘），每次实际写入记一条 TK005。
pub(crate) fn resync_takeovers(st: &State) -> Vec<serde_json::Value> {
    let env = crate::takeover::TakeoverEnv::from_config(&st.config, &st.store);
    let reports = crate::takeover::resync_managed(&env, &st.store, &st.config);
    for r in &reports {
        if r.get("error").is_some() {
            tracing::warn!(target: "llm_switch::takeover", report = %r, "元数据变更后自动重写接管配置失败");
            continue;
        }
        st.log.log(crate::gateway::log::Event {
            level: crate::gateway::log::Level::Info,
            code: "TK005 takeover.resynced",
            message: "taken-over client config rewritten after model metadata change",
            request_id: None,
            backend: None,
            model: None,
            extra: r.clone(),
        });
    }
    reports
}

/// 后端启用/停用。halogen 与 gufo 同端口互斥（内存只够一家，共用 8731）：
/// 开一家就自动关另一家，返回 `{ changed, also_disabled: [...] }` 让 UI 说清动了谁。
/// 不这么做的话两张卡会同时在监控页挂着，而流量全归到没在跑的那台名下。
#[tauri::command]
pub fn set_backend_enabled(
    st: TauriState<Arc<State>>,
    name: String,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    let changed = st
        .store
        .set_backend_enabled(&name, enabled)
        .map_err(|e| e.to_string())?;
    let mut also_disabled: Vec<String> = Vec::new();
    if enabled {
        let rows = st.store.list_backends().map_err(|e| e.to_string())?;
        let mine = rows.iter().find(|b| b.name == name);
        if crate::engine::is_exclusive_engine(mine.and_then(|b| b.engine.as_deref())) {
            for b in &rows {
                if b.name == name || !b.enabled {
                    continue;
                }
                if crate::engine::is_exclusive_engine(b.engine.as_deref()) {
                    st.store
                        .set_backend_enabled(&b.name, false)
                        .map_err(|e| e.to_string())?;
                    also_disabled.push(b.name.clone());
                }
            }
        }
    }
    if let Err(e) = persist_config_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::commands", %name, persist = "enabled", "{e}");
    }
    Ok(serde_json::json!({
        "changed": changed,
        "also_disabled": also_disabled,
    }))
}

/// 模型暴露开关（M12-A）：关掉即对客户端隐身。
#[tauri::command]
pub fn set_model_exposed(
    st: TauriState<Arc<State>>,
    name: String,
    model: String,
    exposed: bool,
) -> Result<bool, String> {
    let hit = st
        .store
        .set_model_exposed(&name, &model, exposed)
        .map_err(|e| e.to_string())?;
    // 与 REST 同一套日志码：GUI 里点的开关同样要在日志页留痕
    // （踩过：桌面端隐藏了一批模型后客户端集体 404，日志页却查无此事）。
    if hit {
        crate::gateway::routes::model_log(
            &st,
            if exposed {
                "MD200 model.exposed"
            } else {
                "MD200 model.hidden"
            },
            format!("模型 `{}` 暴露状态改为 {exposed}", model),
            &name,
            json!({ "model": model, "exposed": exposed, "via": "ipc" }),
        );
    }
    Ok(hit)
}

/// 改名后的过渡开关：是否同时接受上游原名。
#[tauri::command]
pub fn set_model_accept(
    st: TauriState<Arc<State>>,
    name: String,
    model: String,
    accept: bool,
) -> Result<bool, String> {
    st.store
        .set_model_accept_upstream(&name, &model, accept)
        .map_err(|e| e.to_string())
}

/// 模型元数据全量（UI 就地声明入口的数据源）：有效值 + 人工声明 + 后端允许上限 + 手工单价。
#[tauri::command]
pub fn model_meta_get(st: TauriState<Arc<State>>) -> Result<serde_json::Value, String> {
    let map = st.store.effective_model_meta().map_err(|e| e.to_string())?;
    let prices = st.store.model_prices().map_err(|e| e.to_string())?;
    let rows: Vec<serde_json::Value> = map
        .into_iter()
        .map(|(model, meta)| {
            let mut v = serde_json::to_value(meta).unwrap_or_else(|_| json!({}));
            if let Some(obj) = v.as_object_mut() {
                // 手工单价（片4）：null = 未设 = 在线也不计价。
                let p = prices.get(&model);
                obj.insert("model".into(), json!(model));
                obj.insert(
                    "price".into(),
                    match p {
                        Some(p) => json!({
                            "input": p.input,
                            "output": p.output,
                            "cache_read": p.cache_read,
                            "cache_creation": p.cache_creation,
                        }),
                        None => serde_json::Value::Null,
                    },
                );
            }
            v
        })
        .collect();
    Ok(json!(rows))
}

/// 写模型元数据声明：ctx/output 非 0 = 强制覆盖（超后端允许范围钳到边界）；
/// reasoning/vlm 三态：0 = 自动（跟随探测），1 = 强制开，2 = 强制关。
/// 落库后自动重写受管端的接管配置（元数据变了，客户端里的真值不能留着旧的）。
#[tauri::command]
pub fn model_meta_set(
    st: TauriState<Arc<State>>,
    model: String,
    ctx: i64,
    output: i64,
    reasoning: Option<i64>,
    vlm: Option<i64>,
) -> Result<serde_json::Value, String> {
    let res = st
        .store
        .set_model_meta(
            &model,
            ctx,
            output,
            reasoning.unwrap_or(0),
            vlm.unwrap_or(0),
        )
        .map_err(|e| e.to_string())?;
    let resynced = resync_takeovers(st.inner());
    let mut clamped = Vec::new();
    if res.clamped_ctx {
        clamped.push(format!(
            "ctx {ctx} → {}（后端允许上限 {}）",
            res.ctx,
            res.cap_ctx.unwrap_or_default()
        ));
    }
    if res.clamped_output {
        clamped.push(format!(
            "output {output} → {}（后端允许上限 {}）",
            res.output,
            res.cap_output.unwrap_or_default()
        ));
    }
    let message = if !clamped.is_empty() {
        format!("超出后端允许范围，已钳到边界：{}", clamped.join("；"))
    } else if res.ctx == 0 && res.output == 0 && res.reasoning == 0 && res.vlm == 0 {
        "已恢复自动（取后端设定值）".to_string()
    } else {
        "已保存".to_string()
    };
    let mut v = serde_json::to_value(&res).map_err(|e| e.to_string())?;
    v["message"] = json!(message);
    v["resynced"] = json!(resynced);
    Ok(v)
}

/// 写/清模型手工单价（片4，USD / 百万 token）：四维全空 = 清除。
///
/// 单价只影响计价，不改接管配置，无需 resync。
#[tauri::command]
pub fn model_price_set(
    st: TauriState<Arc<State>>,
    model: String,
    input: Option<f64>,
    output: Option<f64>,
    cache_read: Option<f64>,
    cache_creation: Option<f64>,
) -> Result<serde_json::Value, String> {
    let all_none =
        input.is_none() && output.is_none() && cache_read.is_none() && cache_creation.is_none();
    let price = (!all_none).then_some(crate::services::pricing::Price {
        input: input.unwrap_or(0.0),
        output: output.unwrap_or(0.0),
        cache_read: cache_read.unwrap_or(0.0),
        cache_creation: cache_creation.unwrap_or(0.0),
    });
    let priced = st
        .store
        .set_model_price(&model, price)
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "ok": true,
        "priced": priced,
        "message": if priced { "单价已保存（仅在线后端计价）" } else { "单价已清除" },
    }))
}

/// 换暴露名（缺省 = 上游 id）；参数覆盖与缓存策略同事务迁移。
#[tauri::command]
pub fn rename_model(
    st: TauriState<Arc<State>>,
    name: String,
    from: String,
    to: String,
) -> Result<crate::store::RenameOutcome, String> {
    let outcome = st
        .store
        .rename_model(&name, &from, &to)
        .map_err(|e| e.to_string())?;
    // 持久化：改名（暴露名）要回写 config.yaml，否则重启后旧名复活。
    if let Err(e) = persist_config_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::backend", backend = %name, "persist config.yaml failed: {e}");
    }
    Ok(outcome)
}

/// 从该后端摘掉一个模型（连带清掉它的参数覆盖与缓存策略）。
#[tauri::command]
pub fn remove_model(
    st: TauriState<Arc<State>>,
    name: String,
    model: String,
) -> Result<crate::store::RemoveOutcome, String> {
    let outcome = st
        .store
        .remove_model(&name, &model)
        .map_err(|e| e.to_string())?;
    // 持久化：摘模型要回写 config.yaml，否则重启后模型被 yaml 原样灌回。
    if let Err(e) = persist_config_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::backend", backend = %name, "persist config.yaml failed: {e}");
    }
    Ok(outcome)
}

#[tauri::command]
pub fn delete_backend(
    st: TauriState<Arc<State>>,
    name: String,
) -> Result<crate::store::DeleteOutcome, String> {
    // halogen 是速率标定与 halogen/gufo 控制台的锚点，删掉就把那两块打空——一律拒绝，
    // 想停走「停用」或切引擎（UI 那边同步换成锁图标 + 说明弹窗，不让人撞墙试）。
    let row = st
        .store
        .backend_by_name(&name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("没有名为 `{name}` 的后端"))?;
    if crate::engine::is_protected_halogen(&row.name, row.engine.as_deref()) {
        return Err(crate::engine::HALOGEN_PROTECTED.to_string());
    }
    let outcome = st
        .store
        .delete_backend(&name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("没有名为 `{name}` 的后端"))?;
    // 持久化：删后端要回写 config.yaml，否则下次启动 seed_backends 会复活它。
    if let Err(e) = persist_config_yaml(&st.config, &st.store) {
        tracing::warn!(target: "llm_switch::backend", backend = %name, "persist config.yaml failed: {e}");
    }
    Ok(outcome)
}

/// 控制台页「启动 / 停止」按钮：跑该后端配置的本机脚本（见 `services::script`）。
///
/// 只经 IPC 暴露、不给 REST：让远端触发本机脚本的暴露面，比现有的「改一条配置」
/// 大得多，而收益只是省一次点按钮。
#[tauri::command]
pub async fn run_backend_script(
    st: TauriState<'_, Arc<State>>,
    name: String,
    action: String,
) -> Result<serde_json::Value, String> {
    // 动作走白名单再取后端：它是分支键，别让任意字符串流进日志与输出。
    let row = st
        .store
        .backend_by_name(&name)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("没有名为 `{name}` 的后端"))?;
    let out = match action.as_str() {
        "start" => script::start(&st.config.data_dir(), &row).await?,
        "stop" => script::stop(&row).await?,
        other => return Err(format!("未知动作 `{other}`（只认 start / stop）")),
    };
    st.log.log(crate::gateway::log::Event {
        level: if out.ok {
            crate::gateway::log::Level::Info
        } else {
            crate::gateway::log::Level::Warn
        },
        code: if action == "start" {
            "BE330 backend.script_started"
        } else {
            "BE331 backend.script_stopped"
        },
        message: if action == "start" {
            "backend start script run"
        } else {
            "backend stop script run"
        },
        request_id: None,
        backend: Some(name.as_str()),
        model: None,
        extra: serde_json::json!({
            "action": action,
            "ok": out.ok,
            "pid": out.pid,
            "exit_code": out.exit_code,
        }),
    });
    serde_json::to_value(out).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn probe_backend(
    st: TauriState<'_, Arc<State>>,
    name: String,
) -> Result<ProbeOutcome, String> {
    // Owned clone so the future does not borrow the tauri State guard.
    let store = st.store.clone();
    backend::probe(&store, &name).await
}

/// 「扫描模型」按钮背后：只读探一把，不写库（勾选与保存由前端负责）。
#[tauri::command]
pub async fn probe_models(
    input: crate::services::discover::DiscoverInput,
) -> Result<serde_json::Value, String> {
    let found = crate::services::discover::discover(&input).await?;
    serde_json::to_value(found).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct ShellStatus {
    pub version: String,
    pub listen: String,
    pub uptime_ms: u128,
    pub backends: usize,
    pub today_requests: i64,
    pub today_tokens: i64,
    /// 菜单栏快速启停的当前态（true = 暂停接单）。
    pub paused: bool,
    /// 网关访问令牌（未设 = null）：控制台 iframe 发不出自定义请求头，
    /// 只能把令牌拼进 `/console/:id?access_token=…` 的 URL（见 gateway/auth）。
    pub access_token: Option<String>,
}

#[tauri::command]
pub fn get_status(st: TauriState<Arc<State>>) -> Result<ShellStatus, String> {
    let day_start = now_ms() - (now_ms() % 86_400_000);
    let usage = st
        .store
        .usage_summary(day_start, now_ms())
        .unwrap_or_default();
    Ok(ShellStatus {
        version: crate::VERSION.to_string(),
        listen: format!("{}:{}", st.config.server.host, st.listen_port()),
        uptime_ms: st.uptime_ms(),
        backends: st.store.list_backends().map(|v| v.len()).unwrap_or(0),
        today_requests: usage.requests,
        today_tokens: usage.input_tokens + usage.output_tokens,
        paused: st.is_paused(),
        access_token: st.config.server.token().map(str::to_string),
    })
}

/// 暂停/恢复接单（与菜单栏同一个开关）。返回切换后的状态。
#[tauri::command]
pub fn set_paused(st: TauriState<'_, Arc<State>>, paused: bool) -> Result<bool, String> {
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
        st.log.log(crate::gateway::log::Event {
            level: crate::gateway::log::Level::Info,
            code,
            message,
            request_id: None,
            backend: None,
            model: None,
            extra: serde_json::json!({ "via": "ipc" }),
        });
    }
    Ok(st.is_paused())
}

/// 单独更新控制台「入口设置」卡：地址 / 凭据 / 启停脚本（空串=清除）。
#[tauri::command]
pub fn set_backend_console(
    st: TauriState<Arc<State>>,
    name: String,
    webui_url: Option<String>,
    webui_user: Option<String>,
    webui_password: Option<String>,
    start_script: Option<String>,
    stop_script: Option<String>,
) -> Result<bool, String> {
    st.store
        .set_backend_console(
            &name,
            webui_url.as_deref(),
            webui_user.as_deref(),
            webui_password.as_deref(),
            start_script.as_deref(),
            stop_script.as_deref(),
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> BackendInput {
        BackendInput {
            name: name.into(),
            base_url: "http://new".into(),
            original_name: None,
            protocol: "openai_chat".into(),
            api_key: None,
            weight: 0,
            enabled: true,
            engine: None,
            models: vec!["m".into()],
            probe: vec![],
            aliases: Value::Null,
            webui_url: None,
            webui_user: None,
            webui_password: None,
        }
    }

    /// P1 回归：前端表单没有的三列必须原样带回去，否则保存一次后端
    /// 就把探测写出来的健康检查路径与能力表清成 NULL。
    #[test]
    fn upsert_keeps_probe_written_columns() {
        let mut prev = crate::types::backend_row_for_test("lab");
        prev.health_path = Some("/healthz".into());
        prev.capabilities = serde_json::json!({"vlm": true});
        prev.params = serde_json::json!({"temperature": 0.2});

        let up = build_upsert(input("lab"), Some(&prev));
        assert_eq!(up.health_path.as_deref(), Some("/healthz"));
        assert_eq!(up.capabilities, serde_json::json!({"vlm": true}));
        assert_eq!(up.params, serde_json::json!({"temperature": 0.2}));
        // 表单里那一份仍然是真值，别被旧行盖掉。
        assert_eq!(up.base_url, "http://new");
        assert_eq!(up.models, vec!["m".to_string()]);
        // 权重 0 归一到 1（老毛病顺带钉住）。
        assert_eq!(up.weight, 1);
    }

    /// 新后端没有旧行：三列落 schema 默认值 `{}`（不是被 "null" 盖掉默认值）。
    #[test]
    fn upsert_of_new_backend_uses_schema_defaults() {
        let up = build_upsert(input("fresh"), None);
        assert_eq!(up.health_path, None);
        assert_eq!(up.capabilities, serde_json::json!({}));
        assert_eq!(up.params, serde_json::json!({}));
        // 表单里没带别名（JSON null）→ 落空对象，别把 "null" 写进 NOT NULL 列。
        assert_eq!(up.aliases, serde_json::json!({}));
    }

    /// 新增撞名必须拒 —— 这是「加不上后端」最常见的真因：用户以为在新增，
    /// 后端却把同名老后端整行覆盖了（列表里没新行、老行被洗）。
    #[test]
    fn duplicate_name_on_create_is_rejected() {
        let e = check_target("omlx", None, true).unwrap_err();
        assert!(e.contains("已存在"), "{e}");
        assert!(e.contains("编辑"), "{e}");
        // 没人占这个名字：放行。
        assert!(check_target("omlx2", None, false).is_ok());
        // 编辑态带了原名（UI 名称锁死，正常等于 name）：放行。
        assert!(check_target("omlx", Some("omlx"), true).is_ok());
        // 改名不走这条路（名称即定位键）。
        let e = check_target("omlx-b", Some("omlx"), true).unwrap_err();
        assert!(e.contains("不支持改名"), "{e}");
    }

    #[test]
    fn normalize_input_trims_and_requires_scheme() {
        let mut i = input("  lab  ");
        i.base_url = "  http://127.0.0.1:8000/v1/  ".into();
        let (name, base) = normalize_input(&i).unwrap();
        assert_eq!(name, "lab");
        assert_eq!(base, "http://127.0.0.1:8000/v1");

        // 漏 scheme 的老写法（用户就是会这么敲）以前能落库，落进去是一条永远
        // 探不活的死行 —— 现在在保存这一步就拦下，并指出该补什么。
        i.base_url = "localhost:8000".into();
        let e = normalize_input(&i).unwrap_err();
        assert!(e.contains("http://"), "{e}");

        i.base_url = "http://".into();
        assert!(normalize_input(&i).unwrap_err().contains("主机名"));
        // 去尾斜杠必须排在 scheme / 主机名校验之后，否则这两条会被扫成
        // `http:` 然报成「缺前缀」，把真正的原因盖掉。
        i.base_url = "http:///v1".into();
        assert!(normalize_input(&i).unwrap_err().contains("主机名"));

        i.base_url = "http://ok".into();
        i.name = "   ".into();
        assert!(normalize_input(&i).unwrap_err().contains("不能为空"));
    }

    /// 空串字段一律收敛成 None/空，别把 "" 存进库。
    #[test]
    fn upsert_normalises_blank_strings() {
        let mut i = input("lab");
        i.api_key = Some(String::new());
        i.webui_url = Some("   ".into());
        i.webui_user = Some(String::new());
        i.webui_password = Some(String::new());
        let up = build_upsert(i, None);
        assert_eq!(up.api_key, None);
        assert_eq!(up.webui_url, None);
        assert_eq!(up.webui_user, None);
        assert_eq!(up.webui_password, None);
    }
}
