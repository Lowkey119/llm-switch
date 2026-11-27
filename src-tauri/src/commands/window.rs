//! 主窗口窗口级命令（不是浮窗，也不属于某个业务页）。
//!
//! 目前只有一件事：按当前页设主窗口的**最小尺寸下限**（计划 2026-11-26「主窗口完整显示」）。
//! 口径：普通页走响应式栅格，conf 下限（1080×640）以上就该完整铺下；
//! **遥控器页例外**——三栏键位 + 实物图 + 语音坞是固定比例，不参与自适应，
//! 于是把它的自然内容尺寸报上来当窗口下限，窗口拖不到那么小，内容就不会被裁掉。

use tauri::{AppHandle, LogicalSize, Manager};

/// 设主窗口最小尺寸（逻辑像素）。宽或高传 0 = 该轴交回 conf 的默认下限（清掉动态下限）。
#[tauri::command]
pub fn main_min_size(app: AppHandle, width: f64, height: f64) -> bool {
    let Some(win) = app.get_webview_window("main") else {
        return false;
    };
    // 按轴独立回退：前端可能只量得出一个轴的自然尺寸（如遥控器页宽不溢出、高仍要设防），
    // 另一轴传 0 交回 conf 值，不能把两轴一起清掉。
    let c = app.config().app.windows.first();
    let min = LogicalSize {
        width: if width > 0.0 {
            width
        } else {
            c.as_ref().and_then(|w| w.min_width).unwrap_or(1080.0)
        },
        height: if height > 0.0 {
            height
        } else {
            c.as_ref().and_then(|w| w.min_height).unwrap_or(640.0)
        },
    };
    // set_min_size 会顺带把当前小于下限的窗口顶大到下限——正是「拖不动」的效果。
    win.set_min_size(Some(min)).is_ok()
}
