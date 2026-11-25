//! 提示词速取面板 IPC：prompt_items 的查/增/改/删（浮窗与快捷键见 gui::promptfloat）。
//!
//! 归属口径：`tab` = `"common"`（通用库）或某代理的 `ClientApp::as_str()`，
//! 命令层把关，坏 tab 进不了库；内容原样存文本，注入时逐字进剪贴板。

use crate::gateway::State;
use crate::gui::promptfloat;
use crate::takeover::ClientApp;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State as TauriState};

/// tab 必须是 "common" 或已登记代理（唯一真相 ClientApp，不另立名单）。
fn validate_tab(tab: &str) -> Result<(), String> {
    if tab == "common" || ClientApp::ALL.iter().any(|c| c.as_str() == tab) {
        Ok(())
    } else {
        Err(format!("未知归属 `{tab}`（应为 common 或已知代理）"))
    }
}

/// 内容非空且有上限（面板侧编辑窗是自由文本，8000 字足够任何提示词）。
fn validate_content(content: &str) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("内容不能为空".into());
    }
    if content.chars().count() > 8000 {
        return Err("内容过长（上限 8000 字）".into());
    }
    Ok(())
}

#[tauri::command]
pub fn prompt_list(st: TauriState<Arc<State>>) -> Result<Value, String> {
    let rows = st.store.list_prompt_items().map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn prompt_create(
    st: TauriState<Arc<State>>,
    tab: String,
    content: String,
) -> Result<i64, String> {
    validate_tab(&tab)?;
    validate_content(&content)?;
    st.store
        .create_prompt_item(&tab, &content)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn prompt_update(st: TauriState<Arc<State>>, id: i64, content: String) -> Result<bool, String> {
    validate_content(&content)?;
    st.store
        .update_prompt_item(id, &content)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn prompt_delete(st: TauriState<Arc<State>>, id: i64) -> Result<bool, String> {
    st.store.delete_prompt_item(id).map_err(|e| e.to_string())
}

/// 记一次取用（高频条目以次排序靠前的唯一数据源）。
/// 取用失败不能影响注入本身，所以调用方 `.catch` 掉就行，这里只负责落库。
#[tauri::command]
pub fn prompt_use(st: TauriState<Arc<State>>, id: i64) -> Result<bool, String> {
    st.store.bump_prompt_item(id).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 窗口与快捷键

/// 前端 Esc / 注入后收窗走这里（显隐必须回主线程，见 promptfloat）。
#[tauri::command]
pub fn prompt_set_visible(app: AppHandle, visible: bool) {
    promptfloat::set_visible(&app, visible);
}

/// 面板形态：圆钮/贴边 + **当前可见态**。顶栏那颗快捷开关要靠可见态决定图标与
/// `aria-pressed`，不然只有“显示”没有“收起”，用户只能靠 Esc。
#[tauri::command]
pub fn prompt_shape(app: AppHandle) -> Value {
    let mut v = promptfloat::shape();
    let visible = app
        .get_webview_window(promptfloat::LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    v["visible"] = serde_json::json!(visible);
    v
}

/// 点圆钮展开回面板态。
#[tauri::command]
pub fn prompt_expand(app: AppHandle) {
    promptfloat::expand(&app);
}

/// 编辑态临时加宽/还原（前端开/关多行编辑器时各发一次）。
#[tauri::command]
pub fn prompt_widen(app: AppHandle, extra: i32) {
    promptfloat::widen(&app, extra);
}

/// 当前唤起快捷键（accelerator 串，设置页回显用）。
#[tauri::command]
pub fn prompt_hotkey_get(app: AppHandle) -> String {
    promptfloat::hotkey(&app)
}

/// 换唤起快捷键：注册成功才落库；被占用则旧键原位装回并返回人话错误。
#[tauri::command]
pub fn prompt_hotkey_set(app: AppHandle, accelerator: String) -> Result<String, String> {
    promptfloat::set_hotkey(&app, &accelerator)
}

/// 注入高亮条目：剪贴板已由前端写好，这里收窗并把内容粘进唤起前的 App。
/// 注入方式查询：`insert`（光标处插入，默认）/ `replace`（先全选再粘贴，整段替换）。
#[tauri::command]
pub fn prompt_mode_get(app: AppHandle) -> String {
    promptfloat::inject_mode(&app).to_string()
}

/// 设注入方式（代理页「提示词与命令」管理面板用）；非法值由后端把关。
#[tauri::command]
pub fn prompt_mode_set(app: AppHandle, mode: String) -> Result<String, String> {
    promptfloat::set_inject_mode(&app, &mode).map(str::to_string)
}

#[tauri::command]
pub fn prompt_inject(app: AppHandle) {
    promptfloat::inject(&app);
}

/// 面板显示前缓存的注入目标（App 名 + 该终端里跑着的代理）。
/// 前端拿它显示“将注入 Ghostty”、并把 tab 切到激活代理。
#[tauri::command]
pub fn prompt_target() -> Option<promptfloat::Target> {
    promptfloat::target()
}

/// 辅助功能是否授权给**当前运行的这个构建**（模拟粘贴的前置条件）。
/// 为什么必须实时探：mac 的授权认签名身份，ad-hoc 签的包每次重编都是新身份，
/// 旧条目勾着也不作数——静态提示会把人带偏（用户实报「删了重勾还是注入失败」）。
#[tauri::command]
pub fn prompt_a11y_status() -> bool {
    promptfloat::can_simulate()
}

/// 用户主动点「授权」：让系统弹一次辅助功能授权框（注入路径上永远不弹）。
/// 返回当前是否已授权；没授权时弹框，用户勾完再点一次就是 true。
#[tauri::command]
pub fn prompt_a11y_ask() -> bool {
    promptfloat::ask_permission()
}

/// 打开系统辅助功能设置面板（引导用户授权；仅 mac 有意义）。
#[tauri::command]
pub fn prompt_a11y_open() {
    promptfloat::open_a11y_settings();
}

/// 清掉本 App 在「辅助功能」里的授权条目，让用户给**当前构建**重新勾。
///
/// 为什么必须有：macOS 发行包是 ad-hoc 签名（无 Developer ID），TCC 认的身份就是
/// cdhash——每次重新构建换一个身份，新构建在 TCC 眼里是新 App。旧条目还留在列表里
/// 勾着（看着像「早给了权限」），新构建却只能一遍遍弹窗。只 reset 自己的 bundle id，
/// 不碰别的 App。
#[tauri::command]
pub fn prompt_a11y_reset(app: AppHandle) -> Result<(), String> {
    let bundle = app.config().identifier.clone();
    let status = std::process::Command::new("tccutil")
        .args(["reset", "Accessibility", bundle.as_str()])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("tccutil reset 未生效".to_string())
    }
}
