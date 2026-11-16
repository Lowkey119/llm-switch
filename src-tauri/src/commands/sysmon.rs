//! 系统资源监控 IPC（切片 12）：本机 macOS + 102 硬件指标。
//! 数据源与红线见 services/sysmon.rs——拿不到的指标是 null，UI 显示 —。

use crate::gateway::State;
use crate::gui::sysfloat;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State as TauriState, WebviewWindow};

/// 「系统与引擎」卡快照：`{ local: {...}, remote: {...} }`，TTL 缓存（本机 2s / 102 5s）。
#[tauri::command]
pub async fn get_sys_stats(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    Ok(st
        .sysmon
        .snapshot(&st.config.halctl, &st.config.data_dir())
        .await)
}

// —— 资源悬浮小窗（切片 13，gui/sysfloat.rs 的接线）————————————————————

/// 小窗当前该不该显示（设置页初始态；偏好驱动，窗口对象可能还没建）。
#[tauri::command]
pub fn sysfloat_get_visible(app: AppHandle) -> bool {
    sysfloat::visible_pref(&app)
}

/// 开/关资源小窗（偏好持久化，重启恢复）。
#[tauri::command]
pub fn sysfloat_set_visible(app: AppHandle, visible: bool) -> bool {
    sysfloat::set_visible(&app, visible)
}

/// 指标显隐清单（设置页勾选；小窗启动时读一次）。
#[tauri::command]
pub fn sysfloat_metrics_get(app: AppHandle) -> Vec<String> {
    sysfloat::metrics(&app)
}

#[tauri::command]
pub fn sysfloat_metrics_set(app: AppHandle, list: Vec<String>) -> Vec<String> {
    sysfloat::set_metrics(&app, &list)
}

/// 钉住（置顶）状态与开关。
#[tauri::command]
pub fn sysfloat_pin_get(app: AppHandle) -> bool {
    sysfloat::pin(&app)
}

/// 资源小窗形态（面板/圆钮）：前端挂载时先问一次，之后吃 sysfloat://shape 广播。
#[tauri::command]
pub fn sysfloat_shape_get() -> Value {
    sysfloat::shape()
}

/// 点圆钮展开回面板。
#[tauri::command]
pub fn sysfloat_expand(app: AppHandle) -> Value {
    sysfloat::expand(&app);
    sysfloat::shape()
}

#[tauri::command]
pub fn sysfloat_pin_set(app: AppHandle, on: bool) -> bool {
    sysfloat::set_pin(&app, on)
}

/// 小窗前端量完内容回报尺寸（逻辑像素）；左上角不动。
#[tauri::command]
pub fn sysfloat_fit(window: WebviewWindow, width: f64, height: f64) {
    sysfloat::fit(&window, width, height);
}
