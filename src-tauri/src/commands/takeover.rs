//! 客户端接管 IPC（M9-A）：状态 / 接管 / 回滚 / 备份 / 设置。
//!
//! 语义与 REST `/admin/takeover/*` 完全一致（同一层服务，两个门面）。

use crate::gateway::State;
use crate::takeover::{self, ClientApp, TakeoverEnv};
use serde_json::Value;
use std::sync::Arc;
use tauri::State as TauriState;

fn ctx(st: &State) -> (TakeoverEnv, takeover::Endpoint) {
    (
        TakeoverEnv::from_config(&st.config, &st.store),
        takeover::Endpoint::build(&st.config, &st.store),
    )
}

#[tauri::command]
pub fn takeover_status(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let (env, ep) = ctx(&st);
    let clients = takeover::status(&env, &st.store, &ep)?;
    Ok(serde_json::json!({
        "endpoint": { "base_root": ep.base_root, "api_root": ep.api_root, "models": ep.models },
        // home 给前端做两件事：路径显示折叠成 ~/…，越界（家目录外）风险确认。
        "home": env.home.display().to_string(),
        "clients": clients,
    }))
}

#[tauri::command]
pub fn takeover_apply(st: TauriState<'_, Arc<State>>, app: String) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    let (env, ep) = ctx(&st);
    let report = takeover::apply(&env, &st.store, &ep, app)?;
    // 接管会立一份原件备份：写完顺手把该端旧备份收到最近 3 份（原件受保护）。
    takeover::prune_backups(&env, &st.store, 3);
    if report["changed"] == serde_json::json!(true) {
        st.log.log(crate::gateway::log::Event {
            level: crate::gateway::log::Level::Info,
            code: "TK001 takeover.applied",
            message: "client configuration taken over by local gateway",
            request_id: None,
            backend: None,
            model: None,
            extra: report.clone(),
        });
    }
    Ok(report)
}

#[tauri::command]
pub fn takeover_revert(st: TauriState<'_, Arc<State>>, app: String) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    let (env, _) = ctx(&st);
    let report = takeover::revert(&env, &st.store, app)?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK002 takeover.reverted",
        message: "client configuration restored from takeover backup",
        request_id: None,
        backend: None,
        model: None,
        extra: report.clone(),
    });
    Ok(report)
}

#[tauri::command]
pub fn takeover_backups(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let (env, _) = ctx(&st);
    let backups = takeover::list_backups(&env)?;
    Ok(serde_json::json!({ "backups": backups }))
}

#[tauri::command]
pub fn takeover_restore_backup(
    st: TauriState<'_, Arc<State>>,
    id: String,
) -> Result<Value, String> {
    let (env, _) = ctx(&st);
    let report = takeover::restore_backup(&env, &st.store, &id)?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK003 takeover.restored",
        message: "takeover backup restored",
        request_id: None,
        backend: None,
        model: None,
        extra: report.clone(),
    });
    Ok(report)
}

#[tauri::command]
pub fn takeover_delete_backup(st: TauriState<'_, Arc<State>>, id: String) -> Result<Value, String> {
    let (env, _) = ctx(&st);
    takeover::delete_backup(&env, &id)?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK004 takeover.backup_deleted",
        message: "takeover backup deleted",
        request_id: None,
        backend: None,
        model: None,
        extra: serde_json::json!({ "backup_id": id }),
    });
    Ok(serde_json::json!({ "deleted": true }))
}

#[tauri::command]
pub fn get_takeover_settings(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let (_, ep) = ctx(&st);
    Ok(serde_json::json!({ "codex_wire_api": ep.codex_wire_api }))
}

#[tauri::command]
pub fn set_takeover_settings(
    st: TauriState<'_, Arc<State>>,
    value: String,
) -> Result<Value, String> {
    let value = value.trim().to_ascii_lowercase();
    if value != "chat" && value != "responses" {
        return Err("codex_wire_api 只能是 chat 或 responses".into());
    }
    st.store
        .set_setting(takeover::CODEX_WIRE_API_SETTING, &value)
        .map_err(|e| e.to_string())?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK005 takeover.settings_changed",
        message: "takeover settings updated",
        request_id: None,
        backend: None,
        model: None,
        extra: serde_json::json!({ "codex_wire_api": value }),
    });
    Ok(serde_json::json!({ "codex_wire_api": value }))
}

// ── 配置文件编辑器（M12-D）─────────────────────────────────────────
// 与 REST /admin/takeover/:app/file(s) 同一层语义：路径只来自接管清单，
// 写前三道闸（格式校验 / 自动快照 / 并发拦截）都在 takeover::editor 里。

/// 某客户端可编辑的配置文件清单。
#[tauri::command]
pub fn takeover_files(st: TauriState<'_, Arc<State>>, app: String) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    let files = takeover::editor::list(&st, app)?;
    Ok(serde_json::json!({ "files": files }))
}

/// 读取某个配置文件（默认打码；`reveal=true` 才给真实密钥，也只有那时允许保存）。
#[tauri::command]
pub fn takeover_read_file(
    st: TauriState<'_, Arc<State>>,
    app: String,
    index: usize,
    reveal: Option<bool>,
) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    let doc = takeover::editor::read(&st, app, index, reveal.unwrap_or(false))?;
    serde_json::to_value(doc).map_err(|e| e.to_string())
}

/// 写回：校验 → 并发拦截 → 快照 → 落盘。
#[tauri::command]
pub fn takeover_write_file(
    st: TauriState<'_, Arc<State>>,
    app: String,
    index: usize,
    content: String,
    base_sha: String,
) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    let report = takeover::editor::write(&st, app, index, &content, &base_sha)?;
    // 手改也立一份备份：同样把该端备份收到最近 3 份（接管原件受保护）。
    let (env, _) = ctx(&st);
    takeover::prune_backups(&env, &st.store, 3);
    Ok(report)
}

/// 某端模型现状（文件真值 + 期望 + 可选清单）。
#[tauri::command]
pub fn takeover_model_get(st: TauriState<'_, Arc<State>>, app: String) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    let (env, ep) = ctx(&st);
    takeover::model_view(&env, &st.store, &ep, app)
}

/// 切模型（默认档 + 档位表），立刻落盘并进日志。
#[tauri::command]
pub fn takeover_model_set(
    st: TauriState<'_, Arc<State>>,
    app: String,
    default: Option<String>,
    tiers: Option<std::collections::BTreeMap<String, String>>,
    slots: Option<std::collections::BTreeMap<String, String>>,
) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    let mut sel = takeover::Selection {
        default,
        tiers: tiers.unwrap_or_default(),
        ..Default::default()
    };
    // pi 主/小槽与 codex 副槽走同一张面板：键 = 槽位名，空值 = 清空该槽。
    // 带过来就代表用户显式动过手，manage_* 必须置真，否则 set_model 会拿文件真值把槽位洗回去。
    let slot_map = slots.unwrap_or_default();
    let touched: std::collections::BTreeSet<String> = slot_map.keys().cloned().collect();
    for (k, v) in &slot_map {
        let v = Some(v.clone()).filter(|s| !s.trim().is_empty());
        match (app, k.as_str()) {
            (ClientApp::Pi, "main") => {
                sel.pi_main = v;
                sel.manage_pi = true;
            }
            (ClientApp::Pi, "small") => {
                sel.pi_small = v;
                sel.manage_pi = true;
            }
            (ClientApp::Codex, "sub") => {
                sel.codex_sub = v;
                sel.manage_codex = true;
            }
            _ => {}
        }
    }
    let (env, ep) = ctx(&st);
    // 面板只送「动过手」的槽：没送的槽以文件真值回填，否则改小模型会把主模型洗掉
    // （键缺席 = 没动过；空串 = 显式清空，上面已映射成 None，不会被回填）。
    if sel.manage_pi || sel.manage_codex {
        let truth = takeover::read_selection(&env, app)?;
        takeover::merge_untouched_slots(&mut sel, &truth, &touched);
    }
    let report = takeover::set_model(&env, &st.store, &ep, app, sel)?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK006 takeover.model_set",
        message: "client default model switched through gateway",
        request_id: None,
        backend: None,
        model: report["selection"]["default"].as_str(),
        extra: report.clone(),
    });
    Ok(report)
}

/// claude 语义开关（P0-a）：现状一律现场读 settings.json，不查库。
#[tauri::command]
pub fn claude_switches_get(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let (env, _) = ctx(&st);
    takeover::claude_switches::read_switches(&env)
}

/// 改一个开关：布尔开关传 true/false，数值键传数字字符串（空串清除）。
#[tauri::command]
pub fn claude_switch_set(
    st: TauriState<'_, Arc<State>>,
    key: String,
    value: Value,
) -> Result<Value, String> {
    let (env, ep) = ctx(&st);
    takeover::claude_switches::set_switch(&env, &st.store, &ep, &key, &value)
}

/// 模型矩阵（M12-G）：一次给全，格子真值来自各家配置文件。
#[tauri::command]
pub fn matrix_get(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let (env, ep) = ctx(&st);
    takeover::matrix::matrix(&env, &st.store, &ep)
}

/// 客户端列清单（两张矩阵同源 D24）：模型分配矩阵与 Skills 矩阵的列
/// 都从这一份下发，标签/顺序天然一致，新增端只在 `ClientApp` 加一次。
/// `enabled` = 代理启停态（代理 tab 片三）：停用的端由各消费方滤掉列。
/// 只下发**文件客户端**（FILE_CLIENTS）：copilot 门面没有技能目录/安装/接管，
/// 技能矩阵与代理管理都不该出现它（模型矩阵走 matrix_get，另算）。
#[tauri::command]
pub fn client_list(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let disabled: std::collections::BTreeSet<String> =
        st.store.agents_disabled().into_iter().collect();
    // 列源走 ALL（含 copilot 门面）：技能矩阵与代理页共用这一份列（D24），而 copilot
    // 现在两头都要出现——CLI 装了就有代理管理位，技能也真能纳管（~/.copilot/skills）。
    // 停用语义照旧：agents_disabled 里有的下发 enabled=false，两张矩阵一起隐列。
    let clients: Vec<Value> = ClientApp::ALL
        .iter()
        .map(|a| {
            serde_json::json!({
                "app": a.as_str(),
                "label": a.label(),
                "enabled": !disabled.contains(a.as_str()),
            })
        })
        .collect();
    Ok(Value::Array(clients))
}

/// 某端配置文件路径自定义：`default` = 内置默认路径（身份键，白名单校验），
/// `path` = 新位置（绝对路径或 `~/…`）；填回默认等价于恢复。
#[tauri::command]
pub fn takeover_path_set(
    st: TauriState<'_, Arc<State>>,
    app: String,
    default: String,
    path: String,
) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    let (env, ep) = ctx(&st);
    let report = takeover::set_path_override(&st.store, &env, &ep, app, &default, Some(&path))?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK008 takeover.path_override_changed",
        message: "config file path override changed",
        request_id: None,
        backend: None,
        model: None,
        extra: report.clone(),
    });
    Ok(report)
}

/// 恢复某端一份配置文件的内置默认路径。
#[tauri::command]
pub fn takeover_path_reset(
    st: TauriState<'_, Arc<State>>,
    app: String,
    default: String,
) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    let (env, ep) = ctx(&st);
    let report = takeover::set_path_override(&st.store, &env, &ep, app, &default, None)?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK008 takeover.path_override_changed",
        message: "config file path override reset to default",
        request_id: None,
        backend: None,
        model: None,
        extra: report.clone(),
    });
    Ok(report)
}

/// 代理启停（代理 tab 片三）：停用 = 从代理列表与两张矩阵消失。
/// 只动清单——不碰配置文件（接管态原样留着）、不清分配，启用即无损恢复。
#[tauri::command]
pub fn agents_set_enabled(
    st: TauriState<'_, Arc<State>>,
    app: String,
    enabled: bool,
) -> Result<Value, String> {
    let app = ClientApp::parse(&app)?;
    st.store
        .set_agent_enabled(app.as_str(), enabled)
        .map_err(|e| e.to_string())?;
    let report = serde_json::json!({ "app": app.as_str(), "enabled": enabled });
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK009 takeover.agent_toggled",
        message: "agent enabled/disabled (display only, no file changes)",
        request_id: None,
        backend: None,
        model: None,
        extra: report.clone(),
    });
    Ok(report)
}

/// 改一个格子：add / remove / default（claude 的 add 可指名档位槽）。
#[tauri::command]
pub fn matrix_cell(
    st: TauriState<'_, Arc<State>>,
    app: String,
    model: String,
    op: String,
    slot: Option<String>,
) -> Result<Value, String> {
    use takeover::matrix::CellOp;
    let app = ClientApp::parse(&app)?;
    let op = match CellOp::parse(&op)? {
        CellOp::Add { .. } => match slot.as_deref() {
            // UI 重设计：六档浮层的「main」= claude 默认档（矩阵里就是 default 位）。
            // pi 例外：它的 main 是真槽位（主模型），不是默认档——pi 默认档自理。
            Some("main") if app != ClientApp::Pi => CellOp::SetDefault,
            other => CellOp::Add {
                slot: other.map(|s| s.to_string()),
            },
        },
        // remove 也带槽位：一个模型现在能同时占多个槽，摘哪一个必须说清楚。
        CellOp::Remove { .. } => CellOp::Remove {
            slot: slot.map(|s| s.to_string()),
        },
        other => other,
    };
    let (env, ep) = ctx(&st);
    let report = takeover::matrix::apply_cell(&env, &st.store, &ep, app, &model, op)?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK007 takeover.matrix_cell",
        message: "model assignment changed through the matrix",
        request_id: None,
        backend: None,
        model: Some(&model),
        extra: report.clone(),
    });
    Ok(report)
}

/// 预设配置现状（一键还原面板）：读 `~/myllm/agent-configs/manifest.json`，
/// 逐条给出源/目标存在性与差异。
#[tauri::command]
pub fn preset_status(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let (env, _) = ctx(&st);
    takeover::preset::status(&env)
}

/// 一键还原预设配置：`app` 为空 = 全量还原，否则只还原该代理。
/// 覆盖前自动快照现行文件（快照目录随结果返回）。
#[tauri::command]
pub fn preset_restore(
    st: TauriState<'_, Arc<State>>,
    app: Option<String>,
) -> Result<Value, String> {
    let (env, _) = ctx(&st);
    let report = takeover::preset::restore(&env, app.as_deref())?;
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK008 takeover.preset_restore",
        message: "agent configs restored from preset directory",
        request_id: None,
        backend: None,
        model: None,
        extra: report.clone(),
    });
    Ok(report)
}
