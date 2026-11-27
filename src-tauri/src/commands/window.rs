//! 主窗口窗口级命令（不是浮窗，也不属于某个业务页）。
//!
//! 目前只有一件事：按当前页设主窗口的**最小尺寸下限**（计划 2026-11-26「主窗口完整显示」）。
//! 口径：普通页走响应式栅格，conf 下限（1080×640）以上就该完整铺下；
//! **遥控器页例外**——三栏键位 + 实物图 + 语音坞是固定比例，不参与自适应，
//! 于是把它的自然内容尺寸报上来当窗口下限，窗口拖不到那么小，内容就不会被裁掉。

use tauri::{AppHandle, LogicalSize, Manager};

/// 设主窗口最小尺寸（逻辑像素）。宽或高传 0 = 交回 conf 的默认下限（清掉动态下限）。
#[tauri::command]
pub fn main_min_size(app: AppHandle, width: f64, height: f64) -> bool {
    let Some(win) = app.get_webview_window("main") else {
        return false;
    };
    let min = if width > 0.0 && height > 0.0 {
        Some(LogicalSize { width, height })
    } else {
        // 传 0 走 None 会把下限整个取消（比 conf 还小），这里退回 conf 值更稳。
        let c = app.config().app.windows.first();
        Some(LogicalSize {
            width: c.as_ref().and_then(|w| w.min_width).unwrap_or(1080.0),
            height: c.as_ref().and_then(|w| w.min_height).unwrap_or(640.0),
        })
    };
    // set_min_size 会顺带把当前小于下限的窗口顶大到下限——正是「拖不动」的效果。
    win.set_min_size(min).is_ok()
}
