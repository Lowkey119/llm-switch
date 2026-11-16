//! 客户端扩展的 IPC 面：状态、一键安装/更新、一键卸载、每端提示。
//!
//! 装的是同一个「速率扩展」——每端用它自己的官方扩展机制（pi 扩展 / statusLine /
//! TUI 插件 / 技能 / 官方 CLI），数值一律走网关速率出口 `/admin/rates/line?app=<代理>`，
//! 与浮窗速率线同源。机制逐家查证见 [`crate::services::ext_install`] 模块头。

use crate::gateway::State;
use crate::services::ext_install::{self, InstallResult};
use crate::takeover::{ClientApp, TakeoverEnv};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State as TauriState;

/// 评测环境（各家配置目录的环境变量覆盖与路径自定义，与接管同一套口径）。
fn env(st: &State) -> TakeoverEnv {
    TakeoverEnv::from_config(&st.config, &st.store)
}

/// 代理名 → 可接管端；只认有扩展机制的那几家（其余拒绝，不猜默认值）。
fn target(s: &str) -> Result<ClientApp, String> {
    let app = ClientApp::parse(s)?;
    if ext_install::items(app).is_empty() {
        return Err(format!("该代理没有可扩展面：{}", app.as_str()));
    }
    Ok(app)
}

/// 该端扩展状态 + 该端机制的一句话说明。
#[tauri::command]
pub async fn ext_status(st: TauriState<'_, Arc<State>>, app: String) -> Result<Value, String> {
    let app = target(&app)?;
    let env = env(&st);
    let data = st.config.data_dir();
    let rows = ext_install::status(app, &env, &data)?;
    Ok(json!({
        "app": app.as_str(),
        "rows": rows,
        "hint": hint(app),
    }))
}

/// 一键安装（或更新）该端扩展：先备份再覆盖，返回逐项结果。
#[tauri::command]
pub async fn ext_install(
    st: TauriState<'_, Arc<State>>,
    app: String,
) -> Result<Vec<InstallResult>, String> {
    let app = target(&app)?;
    let env = env(&st);
    let data = st.config.data_dir();
    let backups = backup_dir(&st, app);
    tokio::task::spawn_blocking(move || ext_install::install_all(app, &env, &data, &backups))
        .await
        .map_err(|e| format!("扩展安装任务崩溃: {e}"))?
}

/// 一键卸载：删掉我们装的东西（先备份），配置登记反向撤销；不是我们装的一律不碰。
#[tauri::command]
pub async fn ext_uninstall(
    st: TauriState<'_, Arc<State>>,
    app: String,
) -> Result<Vec<InstallResult>, String> {
    let app = target(&app)?;
    let env = env(&st);
    let data = st.config.data_dir();
    let backups = backup_dir(&st, app);
    tokio::task::spawn_blocking(move || ext_install::uninstall_all(app, &env, &data, &backups))
        .await
        .map_err(|e| format!("扩展卸载任务崩溃: {e}"))?
}

/// 备份目录：`<data_dir>/<app>-ext-backups/`，与 skill-backups 同风格。
fn backup_dir(st: &State, app: ClientApp) -> std::path::PathBuf {
    st.config
        .data_dir()
        .join(format!("{}-ext-backups", app.as_str()))
}

/// 每端机制的人话说明（UI 直接显示，别让用户去猜「装完要干什么」）。
fn hint(app: ClientApp) -> &'static str {
    match app {
        ClientApp::Pi => {
            "pi 启动时自动扫描扩展目录：装完重启 pi，速率条出现在输入行上方（/llm-switch-rate on|off 切换）"
        }
        ClientApp::Claude => {
            "状态线即时生效（1 秒刷新），无需重启 claude；速率口径与浮窗单行完全一致"
        }
        ClientApp::CodeBuddy => {
            "CodeBuddy 的 statusLine 与 Claude Code 同一套协议：装完即时生效，1 秒刷新"
        }
        ClientApp::Codex => {
            "codex 的 /statusline 只能选内置条目，自定义状态线仍是实验特性——所以这里装的是个人技能：\
             直接问「现在多快」它会跑脚本原样报数；想在旁边常驻，另开终端跑 \
             `python3 ~/.codex/llm-switch-rate.py --app codex --watch`"
        }
        ClientApp::Gemini => {
            "装完在 gemini 里 /extensions reload，然后 /rate 取数；gemini 的扩展面没有实时展示位，\
             常驻速率看 llm-switch 浮窗速率线"
        }
        ClientApp::OpenCode => {
            "TUI 插件在启动时装载：装完重启 opencode，速率出现在页脚（会话页右侧也有）"
        }
        ClientApp::DevEco => {
            "TUI 插件在启动时装载：装完重启 deveco，速率出现在页脚（与 opencode 同内核，同一份插件）"
        }
        ClientApp::Copilot => {
            "Copilot CLI 没有实时展示位：装完直接问「现在多快」，插件技能会跑脚本原样报数"
        }
    }
}
