//! 浮动面板的 IPC 出口（M12-C2a/C2b）。窗口的生死与形状都在 gui::floatwin 里，
//! 这里只做接线：前端要形状、要切换、要回报尺寸。

use crate::gui::{floatwin, rateline};
use serde_json::{json, Value};
use tauri::{AppHandle, WebviewWindow};

/// 开/关某个代理的单行速率线（一个代理一个窗口，彼此独立，可各自拖位置）。
#[tauri::command]
pub fn rateline_set(app: AppHandle, target: String, on: bool) -> Value {
    let ok = if on {
        rateline::open(&app, &target)
    } else {
        rateline::close(&app, &target)
    };
    json!({ "ok": ok, "apps": rateline::list(&app) })
}

/// 当前开着速率线的代理清单（各代理页开关的初始态）。
#[tauri::command]
pub fn rateline_list(app: AppHandle) -> Vec<String> {
    rateline::list(&app)
}

/// 速率线总开关的当前状态（托盘菜单与主窗都用它）。
#[tauri::command]
pub fn rateline_master_get(app: AppHandle) -> bool {
    rateline::master_on(&app)
}

/// 总开关：关掉收起所有速率线窗口，打开把代理页勾着的逐条点回来。
#[tauri::command]
pub fn rateline_master_set(app: AppHandle, on: bool) -> bool {
    rateline::set_master(&app, on)
}

/// 「代理有请求自动点亮速率线」开关（#46）：设置页与托盘共用同一个偏好。
#[tauri::command]
pub fn rateline_auto_get(app: AppHandle) -> bool {
    rateline::auto_on(&app)
}

/// 同上，保存后即时生效（网关挂号回调与空闲巡检都以此为准）。
#[tauri::command]
pub fn rateline_auto_set(app: AppHandle, on: bool) -> bool {
    rateline::set_auto(&app, on)
}

/// 速率线前端量完内容回报尺寸（逻辑像素）；窗口左上角不动。
#[tauri::command]
pub fn rateline_fit(window: WebviewWindow, width: f64, height: f64) {
    rateline::fit(&window, width, height);
}

/// 切换浮窗可见性，返回切换后的状态。
#[tauri::command]
pub fn float_toggle(app: AppHandle) -> bool {
    floatwin::toggle(&app)
}

/// 显式开/关浮窗，返回最终状态。
#[tauri::command]
pub fn float_set_visible(app: AppHandle, visible: bool) -> bool {
    floatwin::set_visible(&app, visible)
}

/// 当前是否可见（设置页初始态）。
#[tauri::command]
pub fn float_is_visible(app: AppHandle) -> bool {
    floatwin::is_visible(&app)
}

/// 当前形状：面板态还是圆钮态、贴哪条边、展开态尺寸。
#[tauri::command]
pub fn float_state() -> Value {
    floatwin::shape_json()
}

/// 双击切换形态，返回新形状。
#[tauri::command]
pub fn float_double_click(app: AppHandle) -> Value {
    floatwin::double_click(&app)
}

/// 前端量完内容回报尺寸（逻辑像素），窗口贴紧内容。
#[tauri::command]
pub fn float_fit(app: AppHandle, width: f64, height: f64) {
    floatwin::fit(&app, width, height);
}

/// 缩放后按新尺寸重摆（中心锚点）。
#[tauri::command]
pub fn float_zoom_resize(app: AppHandle, width: f64, height: f64) {
    floatwin::zoom_resize(&app, width, height);
}
