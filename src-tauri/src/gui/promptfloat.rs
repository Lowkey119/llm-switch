//! 提示词速取浮动面板（计划 2026-11-18）：全局快捷键唤起的置顶速取面板。
//!
//! 与资源小窗（sysfloat）同一套窗口 flags 与位置记忆手法：位置/尺寸存 settings、
//! 显隐走主线程（macOS 上从 IPC 线程直接 show 会静默失效）、左右贴边收成圆钮。
//! 差别有两处：① 面板尺寸由用户拖拽决定（resizable + 前端右下角把手），改动记住；
//! ② 启动默认不出现——唤起全靠全局快捷键（可自定义，存 `promptfloat.hotkey`），
//! 所以这里没有 visible 偏好，托盘项也不带勾标（与 float-toggle 同款手感）。
//!
//! 条目 CRUD 在 `commands::prompt`；注入通道（剪贴板 + 模拟粘贴）也在命令层。

use crate::gateway::State;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};

/// 窗口标签（float-main.tsx 按它分流前端组件）。
pub const LABEL: &str = "prompt";
const POS_KEY: &str = "promptfloat.pos";
const SIZE_KEY: &str = "promptfloat.size";
/// 唤起快捷键（tauri accelerator 串）。默认 Option/Alt+Shift+P（计划建议默认键，
/// 三端避开系统级组合）。
pub const HOTKEY_KEY: &str = "promptfloat.hotkey";
/// 注入方式：`insert` = 在光标处插入（默认）；`replace` = 先全选再粘贴（整段替换）。
/// 面板设置页可切，存在同一张 settings 表里，与快捷键同口径。
pub const MODE_KEY: &str = "promptfloat.inject_mode";
pub const MODE_DEFAULT: &str = "insert";
pub const HOTKEY_DEFAULT: &str = "Alt+Shift+P";
/// 默认尺寸（逻辑像素）；用户拖拽后以库存为准。
const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 420.0;
/// 拖动/缩放落位的静默窗口：松手后静默这么久才把最终值写库。
const SETTLE_MS: u64 = 600;
/// 贴边吸附阈值（逻辑像素），与浮窗/速率线/资源小窗同值——四处手感一致。
const SNAP_THRESHOLD: f64 = 28.0;
/// 面板可见期间盯焦点的开关（收窗即停）。
static WATCHING: AtomicBool = AtomicBool::new(false);

static SNAPPING: AtomicBool = AtomicBool::new(false);
/// 圆钮直径（逻辑像素），与其余浮窗圆钮同款（.float-orb CSS 同值）。
const ORB: f64 = 54.0;

/// 圆钮态：`(是否收成圆钮, 停靠侧 -1 左 / 1 右)`。收成圆钮后不再重复裁决，
/// 点圆钮展开才回到面板态（与 sysfloat 手感一致）。
fn orb_state() -> &'static Mutex<(bool, i8)> {
    static O: std::sync::OnceLock<Mutex<(bool, i8)>> = std::sync::OnceLock::new();
    O.get_or_init(|| Mutex::new((false, 0)))
}

fn is_orb() -> bool {
    orb_state().lock().expect("prompt orb state poisoned").0
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 待落库的拖动：`(时间戳, x, y)`。Moved 每帧刷新时间戳并 spawn 一次延时落库，
/// 只有静默到点、且没被更新的移动作废的那次真写（sysfloat 同款手法）。
fn pending_move() -> &'static Mutex<Option<(u64, i32, i32)>> {
    static P: std::sync::OnceLock<Mutex<Option<(u64, i32, i32)>>> = std::sync::OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

/// 待落库的缩放：`(时间戳, w, h)`（逻辑像素）。
fn pending_size() -> &'static Mutex<Option<(u64, f64, f64)>> {
    static P: std::sync::OnceLock<Mutex<Option<(u64, f64, f64)>>> = std::sync::OnceLock::new();
    P.get_or_init(|| Mutex::new(None))
}

fn store_of(app: &AppHandle) -> Option<std::sync::Arc<crate::store::Store>> {
    app.try_state::<std::sync::Arc<State>>()
        .map(|st| st.store.clone())
}

fn setting(app: &AppHandle, key: &str) -> Option<String> {
    store_of(app)?.get_setting(key).ok().flatten()
}

fn set_setting(app: &AppHandle, key: &str, val: &str) {
    if let Some(store) = store_of(app) {
        if let Err(e) = store.set_setting(key, val) {
            tracing::warn!(target: "llm_switch::promptfloat", error = %e, key, "偏好落库失败");
        }
    }
}

// ------------------------------------------------------------------ 快捷键

/// 当前注入方式（只认 `insert` / `replace`，其余一律回到默认，防脏值）。
pub fn inject_mode(app: &AppHandle) -> &'static str {
    match setting(app, MODE_KEY).as_deref() {
        Some("replace") => "replace",
        _ => MODE_DEFAULT,
    }
}

/// 设注入方式；非法值直接报错，不让面板与库里对不上。
pub fn set_inject_mode(app: &AppHandle, mode: &str) -> Result<&'static str, String> {
    // 返回 'static：从字面量里挑，不把入参的生命周期带出去。
    let ok: &'static str = match mode {
        "insert" => "insert",
        "replace" => "replace",
        _ => return Err("注入方式只支持 insert / replace".into()),
    };
    set_setting(app, MODE_KEY, ok);
    Ok(ok)
}

/// 当前唤起快捷键（没设过用默认）。
pub fn hotkey(app: &AppHandle) -> String {
    setting(app, HOTKEY_KEY).unwrap_or_else(|| HOTKEY_DEFAULT.to_string())
}

/// 换键：先验串、再解旧装新；装新失败（多半被占用）把旧键原位装回，
/// 返回人话错误给设置页 toast。成功返回生效的键串。
pub fn set_hotkey(app: &AppHandle, accel: &str) -> Result<String, String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let accel = accel.trim().to_string();
    if accel.is_empty() {
        return Err("快捷键不能为空".into());
    }
    accel
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map_err(|_| format!("无法识别的快捷键 `{accel}`"))?;
    let old = hotkey(app);
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    match gs.register(accel.as_str()) {
        Ok(()) => {
            set_setting(app, HOTKEY_KEY, &accel);
            Ok(accel)
        }
        Err(e) => {
            // 新键装不上：旧键原位装回，别把用户留在无快捷键态。
            let _ = gs.register(old.as_str());
            Err(format!("快捷键 {accel} 注册失败（可能已被占用）：{e}"))
        }
    }
}

/// 启动注册（setup 里调用）：库存键坏了（手改库/被占用）退回默认键再试一次。
pub fn register_hotkey(app: &AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let key = hotkey(app);
    if app.global_shortcut().register(key.as_str()).is_ok() {
        return;
    }
    if key != HOTKEY_DEFAULT {
        tracing::warn!(
            target: "llm_switch::promptfloat",
            key = %key,
            "唤起快捷键注册失败，退回默认键"
        );
        if app.global_shortcut().register(HOTKEY_DEFAULT).is_ok() {
            set_setting(app, HOTKEY_KEY, HOTKEY_DEFAULT);
            return;
        }
    }
    tracing::warn!(
        target: "llm_switch::promptfloat",
        "提示词面板唤起快捷键注册失败（面板仍可从托盘唤起）"
    );
}

// ------------------------------------------------------------------ 显隐

/// 唤起/收起切换（快捷键与托盘共用）。is_visible 走 AppKit，整段收进主线程。
pub fn toggle(app: &AppHandle) {
    let handle = app.clone();
    if app
        .run_on_main_thread(move || {
            let vis = handle
                .get_webview_window(LABEL)
                .and_then(|w| w.is_visible().ok())
                .unwrap_or(false);
            apply(&handle, !vis);
        })
        .is_err()
    {
        let vis = app
            .get_webview_window(LABEL)
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false);
        apply(app, !vis);
    }
}

/// 设定可见性（前端 Esc / 注入后收窗走这里）。macOS 上窗口显隐必须在主线程。
pub fn set_visible(app: &AppHandle, visible: bool) {
    let handle = app.clone();
    if app
        .run_on_main_thread(move || apply(&handle, visible))
        .is_err()
    {
        apply(app, visible);
    }
}

fn apply(app: &AppHandle, visible: bool) {
    // 显隐联动（计划 2026-11-26）：面板可见时遥控器改用面板专用键位表，收起即恢复日常表。
    // 放在唯一显隐收口处，所有开关路径（快捷键/托盘/Esc/注入后收窗）自动覆盖。
    // state 可能尚未就绪（启动早期），拿不到就跳过——不影响浮窗本身显隐。
    let ch = app.state::<std::sync::Arc<crate::services::remote::hid::ButtonChannel>>();
    ch.set_panel_visible(visible);
    if visible {
        // 唤起瞬间先拍一次注入目标（在面板抢焦点之前），之后面板可见期间持续盯：
        // 用户切了焦点，顶行提示、自动 tab 与注入缓存要跟着换人（用户 2026-11-19）。
        note_front();
        start_focus_watch(app);
        // 面板可能已经开着（圆钮展开 / 托盘再唤）：把当前目标推一次，顶行不会停在旧值。
        let _ = app.emit("prompt://target", target());
        match app.get_webview_window(LABEL) {
            Some(win) => {
                let _ = win.show();
                let _ = win.set_focus();
            }
            None => {
                if let Err(e) = build(app) {
                    tracing::warn!(target: "llm_switch::promptfloat", error = %e, "提示词面板创建失败");
                }
            }
        }
    } else if let Some(win) = app.get_webview_window(LABEL) {
        stop_focus_watch();
        save_geometry(app);
        let _ = win.hide();
    }
    // 与浮窗同一口径：可见性一变就广播，顶栏绿点不等轮询（用户 2026-11-25）。
    let _ = app.emit("floats://vis", ());
}

/// 面板可见期间盯焦点（收窗即停，不给后台加轮询）：用户口径——面板开着时切了
/// 应用焦点，注入目标就跟着切。note_front 只在 pid 真变了才干活。
fn start_focus_watch(app: &AppHandle) {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return; // 已经盯着了（面板反复 show/hide 不重复起轮询）
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        while WATCHING.load(Ordering::SeqCst) {
            // 目标变了就主动广播：面板失焦时前端不轮询，不推就永远停在旧目标
            //（用户实报：面板开着切了应用，顶行与注入目标没跟着换）。
            if note_front() {
                let _ = handle.emit("prompt://target", target());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });
}

fn stop_focus_watch() {
    WATCHING.store(false, Ordering::SeqCst);
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let (w0, h0) = saved(app, SIZE_KEY).unwrap_or((WIDTH, HEIGHT));
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("float.html".into()))
        .title("llm-switch 提示词速取")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        // 面板宽高由用户拖右下角把手调整（前端 startResizeDragging），要记住。
        .resizable(true)
        .maximizable(false)
        .minimizable(false)
        .shadow(false)
        // 先建后摆：可见再挪会看见一次跳位。
        .visible(false)
        .inner_size(w0, h0)
        .visible_on_all_workspaces(true)
        .build()?;
    place(app, &win);
    let handle = app.clone();
    win.on_window_event(move |ev| match ev {
        WindowEvent::Moved(pos) => {
            remember_move(&handle, *pos);
            judge_edge(&handle);
        }
        WindowEvent::Resized(size) => remember_resize(&handle, *size),
        _ => {}
    });
    let _ = win.show();
    let _ = win.set_focus();
    Ok(())
}

/// 落位：有存档用存档；默认主屏工作区居中偏上（速取面板看中央最顺手）。
fn place(app: &AppHandle, win: &tauri::WebviewWindow) {
    if let Some((x, y)) = saved_pos(app) {
        let _ = win.set_position(PhysicalPosition::new(x, y));
        return;
    }
    let Some((wx, wy, ww, wh, _)) = crate::gui::floatwin::primary_work_area(app) else {
        return;
    };
    let Ok(size) = win.inner_size() else {
        return;
    };
    let x = wx + (ww - size.width as f64) / 2.0;
    let y = wy + (wh - size.height as f64) / 3.0;
    let _ = win.set_position(PhysicalPosition::new(x.max(wx) as i32, y.max(wy) as i32));
}

fn saved_pos(app: &AppHandle) -> Option<(i32, i32)> {
    let raw = setting(app, POS_KEY)?;
    let mut it = raw.split(',');
    let x = it.next()?.trim().parse().ok()?;
    let y = it.next()?.trim().parse().ok()?;
    Some((x, y))
}

fn saved(app: &AppHandle, key: &str) -> Option<(f64, f64)> {
    let raw = setting(app, key)?;
    let mut it = raw.split(',');
    let w = it.next()?.trim().parse().ok()?;
    let h = it.next()?.trim().parse().ok()?;
    if w < 240.0 || h < 160.0 {
        return None;
    }
    Some((w, h))
}

/// 拖动落位：Moved 每帧只记账 + 排一次延时，静默到点的那次才写库。
fn remember_move(app: &AppHandle, pos: PhysicalPosition<i32>) {
    let stamp = now_ms();
    {
        let mut guard = pending_move().lock().expect("prompt move clock poisoned");
        *guard = Some((stamp, pos.x, pos.y));
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
        let due = {
            let mut guard = pending_move().lock().expect("prompt move clock poisoned");
            match *guard {
                Some((s, x, y)) if s == stamp => {
                    *guard = None;
                    Some((x, y))
                }
                _ => None,
            }
        };
        if let Some((x, y)) = due {
            set_setting(&handle, POS_KEY, &format!("{},{}", x, y));
            snap_to_edge(&handle);
        }
    });
}

/// 缩放置记：Resized 每帧记账 + 延时落库（逻辑像素）。圆钮态不收——54px 是
/// 圆钮自己的尺寸，收进去下次展开就回不到用户尺寸了。
fn remember_resize(app: &AppHandle, size: PhysicalSize<u32>) {
    let scale = app
        .get_webview_window(LABEL)
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0);
    let (w, h) = (size.width as f64 / scale, size.height as f64 / scale);
    let stamp = now_ms();
    {
        let mut guard = pending_size().lock().expect("prompt size clock poisoned");
        *guard = Some((stamp, w, h));
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
        let due = {
            let mut guard = pending_size().lock().expect("prompt size clock poisoned");
            match *guard {
                Some((s, w, h)) if s == stamp => {
                    *guard = None;
                    Some((w, h))
                }
                _ => None,
            }
        };
        if let Some((w, h)) = due {
            if is_orb() || SNAPPING.load(Ordering::SeqCst) {
                return;
            }
            set_setting(&handle, SIZE_KEY, &format!("{w},{h}"));
        }
    });
}

/// 贴边裁决（拖动中每帧，sysfloat 同款时序）：左右贴边 → 收成圆钮。
/// 已收成圆钮就不再裁决（点圆钮才展开）。
fn judge_edge(app: &AppHandle) {
    if SNAPPING.load(Ordering::SeqCst) || is_orb() {
        return;
    }
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let Some((wx, wy, ww, wh, scale)) = crate::gui::floatwin::work_area_of(&win) else {
        return;
    };
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.inner_size()) else {
        return;
    };
    let th = SNAP_THRESHOLD * scale;
    let d_top = (pos.y as f64 - wy).max(0.0);
    let d_bottom = (wy + wh - (pos.y as f64 + size.height as f64)).max(0.0);
    let d_left = (pos.x as f64 - wx).max(0.0);
    let d_right = (wx + ww - (pos.x as f64 + size.width as f64)).max(0.0);
    let edge = crate::gui::floatwin::rank_edges(
        d_top,
        d_bottom,
        d_left,
        d_right,
        size.width as f64 <= ww,
        th,
    );
    match edge {
        crate::gui::floatwin::Edge::Left => collapse(app, -1),
        crate::gui::floatwin::Edge::Right => collapse(app, 1),
        _ => {}
    }
}

/// 编辑态临时加宽/还原：多行编辑器放在列表右侧（用户标注的位置）。
///
/// 以左上角为锚只改宽度，不动位置也不动 `SIZE_KEY` 里的偏好尺寸（那存的是
/// 常态尺寸，不能把临时加宽写回去）；收编辑器时传回负值原路缩回。
pub fn widen(app: &AppHandle, extra: i32) {
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let Ok(size) = win.outer_size() else { return };
    let scale = win.scale_factor().unwrap_or(1.0);
    let floor = (320.0 * scale) as i64;
    let target = (size.width as i64 + extra as i64).max(floor) as u32;
    if target == size.width {
        return;
    }
    let _ = win.set_size(PhysicalSize::new(target, size.height));
}

/// 收成圆钮：改尺寸、贴左右边、落库位置，并广播形态让前端换画圆钮。
fn collapse(app: &AppHandle, side: i8) {
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let Some((wx, wy, ww, wh, scale)) = crate::gui::floatwin::work_area_of(&win) else {
        return;
    };
    let Ok(pos) = win.outer_position() else {
        return;
    };
    *orb_state().lock().expect("prompt orb state poisoned") = (true, side);
    SNAPPING.store(true, Ordering::SeqCst);
    let orb = (ORB * scale) as u32;
    let x = if side < 0 { wx } else { wx + ww - orb as f64 };
    let y = (pos.y as f64).clamp(wy, (wy + wh - orb as f64).max(wy));
    let _ = win.set_size(PhysicalSize::new(orb, orb));
    let _ = win.set_position(PhysicalPosition::new(x as i32, y as i32));
    set_setting(app, POS_KEY, &format!("{},{}", x as i32, y as i32));
    let _ = app.emit(
        "prompt://shape",
        serde_json::json!({ "orb": true, "side": side }),
    );
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_millis(120)).await;
        SNAPPING.store(false, Ordering::SeqCst);
    });
}

/// 点圆钮展开：恢复面板尺寸（库存尺寸），位置跟圆钮左上角走、钳回工作区。
pub fn expand(app: &AppHandle) {
    {
        let mut guard = orb_state().lock().expect("prompt orb state poisoned");
        if !guard.0 {
            return;
        }
        *guard = (false, 0);
    }
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    SNAPPING.store(true, Ordering::SeqCst);
    let scale = win.scale_factor().unwrap_or(1.0);
    let (w0, h0) = saved(app, SIZE_KEY).unwrap_or((WIDTH, HEIGHT));
    let _ = win.set_size(PhysicalSize::new((w0 * scale) as u32, (h0 * scale) as u32));
    if let (Ok(pos), Some((wx, wy, ww, wh, _))) = (
        win.outer_position(),
        crate::gui::floatwin::work_area_of(&win),
    ) {
        let x = (pos.x as f64).clamp(wx, (wx + ww - w0 * scale).max(wx));
        let y = (pos.y as f64).clamp(wy, (wy + wh - h0 * scale).max(wy));
        let _ = win.set_position(PhysicalPosition::new(x as i32, y as i32));
        set_setting(app, POS_KEY, &format!("{},{}", x as i32, y as i32));
    }
    let _ = app.emit(
        "prompt://shape",
        serde_json::json!({ "orb": false, "side": 0 }),
    );
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_millis(120)).await;
        SNAPPING.store(false, Ordering::SeqCst);
    });
}

/// 当前形态（前端挂载时先问一次，之后吃 prompt://shape 广播）。
pub fn shape() -> serde_json::Value {
    let (orb, side) = *orb_state().lock().expect("prompt orb state poisoned");
    serde_json::json!({ "orb": orb, "side": side })
}

/// 贴到所在屏工作区的顶边或底边（离得够近才吸；左右交给 judge_edge 收圆钮）。
fn snap_to_edge(app: &AppHandle) {
    if SNAPPING.load(Ordering::SeqCst) || is_orb() {
        return;
    }
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let Some((_wx, wy, _ww, wh, scale)) = crate::gui::floatwin::work_area_of(&win) else {
        return;
    };
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else {
        return;
    };
    let th = SNAP_THRESHOLD * scale;
    let y = pos.y as f64;
    let h = size.height as f64;
    let ny = if (y - wy).abs() <= th {
        wy
    } else if (wy + wh - (y + h)).abs() <= th {
        wy + wh - h
    } else {
        return;
    };
    if (ny - y).abs() < 0.5 {
        return;
    }
    SNAPPING.store(true, Ordering::SeqCst);
    let landed = PhysicalPosition::new(pos.x, ny as i32);
    let _ = win.set_position(landed);
    set_setting(app, POS_KEY, &format!("{},{}", landed.x, landed.y));
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_millis(120)).await;
        SNAPPING.store(false, Ordering::SeqCst);
    });
}

// ------------------------------------------------------------------ 注入（计划 P0-3）

/// 注入目标缓存：`(面板可见期间最新一个前台 pid, 该 App 里跑着的代理, 该 App 有无输入区)`。
///
/// 第三个字段是「有输入编辑框」门控（用户 2026-11-25）：唤起瞬间缓存的前台 App
/// 必须有可写输入区才配当首要注入对象——没有就按「无注入目标」处理，
/// 免得用户对着一个没输入框的 App 按 OK 却毫无反应。
type Front = (i32, Option<&'static str>, bool);

/// 面板的注入目标：收窗后把焦点还给它，粘贴才落对窗口；
/// 代理键还给面板决定开哪个 tab。
/// Win/Linux 收窗后焦点自然回到原窗口，暂不追踪（P1-11 逐端实测）。
#[cfg(target_os = "macos")]
fn last_front() -> &'static Mutex<Option<Front>> {
    static L: std::sync::OnceLock<Mutex<Option<Front>>> = std::sync::OnceLock::new();
    L.get_or_init(|| Mutex::new(None))
}

/// 面板顶行与自动切 tab 用的目标快照。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Target {
    /// 前台 App 名（Ghostty / Terminal / Code …）。
    pub name: String,
    /// 该 App 进程树里跑着的代理（`ClientApp::as_str()`，认不出 = None）。
    pub agent: Option<String>,
}

/// 当前缓存的注入目标；没记住、或记到的 App 没有输入区就 None（前端显示「无注入目标」）。
pub fn target() -> Option<Target> {
    #[cfg(target_os = "macos")]
    {
        let cached = last_front().lock().ok().and_then(|g| *g)?;
        let (pid, agent, has_input) = cached;
        if !has_input {
            return None;
        }
        Some(Target {
            name: app_name(pid).unwrap_or_else(|| format!("pid {pid}")),
            agent: agent.map(str::to_string),
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// 目标 pid（注入时把焦点还给谁）；没记住或没输入区就 None。
#[cfg(target_os = "macos")]
fn target_pid() -> Option<i32> {
    last_front()
        .lock()
        .expect("prompt front clock poisoned")
        .filter(|(_, _, has_input)| *has_input)
        .map(|(p, _, _)| p)
}

/// 某个 pid 的 App 名（面板显示「将注入 Ghostty」用；App 已退出则 None）。
#[cfg(target_os = "macos")]
fn app_name(pid: i32) -> Option<String> {
    use objc2_app_kit::NSRunningApplication;
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|a| a.localizedName().map(|s| s.to_string()))
}

/// 该 App 的进程树里跑着哪个代理：终端类 App 本身不是代理，代理是它的子孙
/// （Terminal → zsh → claude）。按已知可执行名逐层往下找，最多四层。
/// 只比进程名的末段（codex 这类在 mac 上是全路径）；tmux/screen 里代理挂在
/// server 进程下、不挂在终端下，这种认不出 → None（面板不切 tab）。
/// shortcut: 只按子孙进程名匹配；要覆盖 tmux 得读 tty 或窗口标题，真遇到再加。
#[cfg(target_os = "macos")]
fn agent_of(pid: i32) -> Option<&'static str> {
    use crate::services::agents_scan::bin_name;
    use crate::takeover::ClientApp;
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut sys = System::new().ok()?;
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let base_name = |proc: &sysinfo::Process| {
        let raw = proc.name().to_string_lossy().into_owned();
        raw.rsplit('/').next().unwrap_or(&raw).to_string()
    };
    let hit = |name: &str| {
        ClientApp::ALL
            .iter()
            .map(|c| (bin_name(*c), c.as_str()))
            .find(|(bin, _)| !bin.is_empty() && *bin == name)
            .map(|(_, key)| key)
    };
    // 根：前台 App 自己，再加它的「同族 helper」。Orca 这类终端把真终端跑在
    // launchd 直接托管的 helper 里（父进程是 1），不是前台 App 的子孙，光往下找碰不到。
    // 同族判据用 `同名` 或 `同名-` 前缀：orca → orca-terminal-host 算，
    // code（VS Code）→ codex 不算，不会把隔壁代理误认进来。
    let front = sys
        .process(Pid::from_u32(pid as u32))
        .map(|p| base_name(p).to_lowercase())
        .unwrap_or_default();
    let mut frontier: Vec<u32> = vec![pid as u32];
    if !front.is_empty() {
        for proc in sys.processes().values() {
            let p = proc.pid().as_u32();
            if p == pid as u32 {
                continue;
            }
            let name = base_name(proc).to_lowercase();
            if name == front || name.starts_with(&format!("{front}-")) {
                frontier.push(p);
            }
        }
    }
    // 深度给到 8：ghostty → login → ghost-complete → zsh → pi 实测已经四层，
    // 再套一层 shell / node 包装就到限了。
    for _ in 0..8 {
        let mut next = Vec::new();
        for proc in sys.processes().values() {
            let Some(parent) = proc.parent() else {
                continue;
            };
            if !frontier.contains(&parent.as_u32()) {
                continue;
            }
            if let Some(key) = hit(&base_name(proc)) {
                return Some(key);
            }
            next.push(proc.pid().as_u32());
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    None
}

/// 用户主动点「授权」时才允许弹系统框：拿得到就是已经授权，拿不到时
/// `prompt: true` 的构造会让系统弹一次「允许辅助功能」对话框（带「打开系统设置」）。
/// 与 `can_simulate` 的区别就在这个 prompt 开关：注入路径上永远不许弹。
#[cfg(target_os = "macos")]
pub fn ask_permission() -> bool {
    enigo::Enigo::new(&enigo::Settings {
        open_prompt_to_get_permissions: true,
        ..Default::default()
    })
    .is_ok()
}

/// 非 mac 没有这回事。
#[cfg(not(target_os = "macos"))]
pub fn ask_permission() -> bool {
    true
}

/// 当前进程能不能模拟按键 = 辅助功能有没有授权给**正在运行的这个构建**。
///
/// 为什么用 enigo 探而不是直接问系统：本仓 `unsafe_code = "forbid"`，`AXIsProcessTrusted`
/// 这类 C 调用编不进来。探针刻意用「不弹系统授权框」的构造——enigo 默认构造会弹，
/// 那正是「每次注入都弹系统设置」的元凶（见 `no_prompt_settings`）。
pub fn can_simulate() -> bool {
    #[cfg(target_os = "macos")]
    {
        enigo::Enigo::new(&no_prompt_settings()).is_ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

#[cfg(target_os = "macos")]
fn note_front() -> bool {
    let Some(pid) = frontmost_pid() else {
        return false;
    };
    // 自己（llm-switch）不算：从主窗按钮或托盘唤起时前台就是自己，记下来会把
    // 内容粘回面板自己，所以那种情况保留上一个外部 App。
    if pid == std::process::id() as i32 {
        return false;
    }
    let mut guard = last_front().lock().expect("prompt front clock poisoned");
    // 焦点没换 App 就什么都不做：代理识别要扫进程表，不能 250ms 扫一次。
    if guard.as_ref().is_some_and(|(p, _, _)| *p == pid) {
        return false;
    }
    // 输入区探测只在换 App 时做一次（AX 遍历有预算，不许 250ms 轮询一次）。
    *guard = Some((
        pid,
        agent_of(pid),
        crate::services::remote::axfocus::front_has_input(),
    ));
    tracing::debug!(target: "llm_switch::promptfloat", pid, agent = ?guard.as_ref().and_then(|(_, a, _)| *a), "注入目标切换");
    true
}

/// 当前前台 App 的 pid（无前台 App 时 None）。
#[cfg(target_os = "macos")]
fn frontmost_pid() -> Option<i32> {
    use objc2_app_kit::NSWorkspace;
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|a| a.processIdentifier())
}

/// 把一段 AppKit / CoreGraphics 调用挪到主线程执行并等它的结果。
///
/// 为什么非挪不可（真因，别再改回去）：`CGEventPost` 在非主线程会撞上 libdispatch
/// 的主队列断言。现场统一日志的最后一行就是它（紧跟 HIDAnalytics 的 iohidpostevent）：
///
/// ```text
/// BUG IN CLIENT OF LIBDISPATCH: Assertion failed: Block was expected to execute
/// on queue [com.apple.main-thread]
/// ```
///
/// `dispatch_assert_queue_fail` 触发的正是 SIGTRAP：进程当场没，Rust 侧连 panic 都
/// 来不及打，DiagnosticReports 里也翻不到东西——正是用户实报的「一回车就退出整个
/// 应用」。注入跑在 `async_runtime::spawn` 的 tokio 工作线程上，正好踩中；而独立
/// 复现器从主线程发事件，所以永远复现不出来。
#[cfg(target_os = "macos")]
pub fn on_main<T: Send + 'static>(
    app: &AppHandle,
    timeout: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<T>(1);
    if app
        .run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .is_err()
    {
        return None;
    }
    // 主线程卡住时不能把 tokio 工作线程一起吊死：超时报错，调用方走兜底。
    rx.recv_timeout(timeout).ok()
}

#[cfg(not(target_os = "macos"))]
fn note_front() -> bool {
    false
}

/// 把焦点还给唤起前的 App（一次性，用完即弃，防止粘到已退出的进程）。
/// 返回拿到的目标 pid，调用方据此等它真成为前台再粘贴。
#[cfg(target_os = "macos")]
fn reactivate_front(app: &AppHandle) -> Option<i32> {
    // 只看不取：注入失败时目标要留着，用户补完授权还能直接回车重试。
    let Some(pid) = target_pid() else {
        tracing::warn!(target: "llm_switch::promptfloat", "焦点归还跳过：没记住唤起前的 App");
        return None;
    };
    // 激活是 AppKit 调用，照旧走主线程（见 on_main 的说明）。
    let reached = on_main(app, Duration::from_millis(600), move || {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        let Some(target) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        else {
            return Ok::<bool, &'static str>(false);
        };
        // 不用 ActivateIgnoringOtherApps：macOS 14 起已废弃且无效。
        Ok(target.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows))
    });
    match reached {
        // App 已退出：没得等，直接让调用方走 no-target 兜底。
        Some(Ok(false)) => {
            tracing::warn!(target: "llm_switch::promptfloat", pid, "焦点归还失败：App 已退出");
            None
        }
        // 请求递到了但系统没立刻认：照样等一轮前台，超时有 focus-lost 兜底。
        Some(_) => Some(pid),
        None => {
            tracing::warn!(target: "llm_switch::promptfloat", pid, "焦点归还失败：主线程未响应");
            None
        }
    }
}

/// 非 mac：收窗后焦点自然回到原窗口，没有可等的目标（返回哨兵让等待直接放行）。
#[cfg(not(target_os = "macos"))]
fn reactivate_front(_app: &AppHandle) -> Option<i32> {
    Some(-1)
}

/// 等目标真的成了前台再粘贴。激活是异步的系统行为，盲等固定毫秒要么粘空、
/// 要么粘进正在消失的面板自己（原实现等 120ms 就是这个坑）。
#[cfg(target_os = "macos")]
async fn wait_until_front(app: &AppHandle, pid: i32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if on_main(app, Duration::from_millis(300), frontmost_pid) == Some(Some(pid)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

#[cfg(not(target_os = "macos"))]
async fn wait_until_front(_app: &AppHandle, _pid: i32, _timeout: Duration) -> bool {
    tokio::time::sleep(Duration::from_millis(120)).await;
    true
}

/// ⌘V 里 V 的虚拟键码（ANSI 主键盘区）。
///
/// 刻意不用 `Key::Unicode('v')`：enigo 0.6.1 会走 `get_layoutdependent_keycode` →
/// `keycode_to_string`，那里对 `TISGetInputSourceProperty` 取回的 layout_data 只有
/// `debug_assert`（release 直接编掉），中文等 CJK 输入法下拿到 NULL 照样喂给
/// `UCKeyTranslate` → 整个进程当场没了（用户实报「一回车就退出整个应用」，
/// 日志正好停在 `key(Unicode('v'))` 之后、`raw(keycode:)` 之前）。
/// 非 ANSI 布局上 ⌘+9 号键可能不触发粘贴，那种情况走「已复制，请手动粘贴」兜底。
#[cfg(target_os = "macos")]
const V_KEYCODE: u32 = 9;

/// ⌘A 里 A 的虚拟键码（ANSI 主键盘区）——「整段替换」注入方式先全选用。
#[cfg(target_os = "macos")]
const A_KEYCODE: u32 = 0;

/// 向当前前台 App 模拟一次系统粘贴（mac 为 Cmd+V，其余 Ctrl+V）。
/// 要求辅助功能授权，未授权时事件被系统静默丢弃——设置页有静态提示引导授权。
///
/// mac 必须把发事件这件事挪到主线程（见 `on_main`）：在 tokio 工作线程上
/// `CGEventPost` 会撞上 libdispatch 主队列断言，进程当场 SIGTRAP。
fn simulate_paste(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        on_main(app, Duration::from_millis(800), paste_key_sequence)
            .unwrap_or_else(|| Err("主线程未响应，粘贴请求未执行".to_string()))
    }
    #[cfg(not(target_os = "macos"))]
    paste_key_sequence()
}

/// 「整段替换」注入的第一步：先全选目标输入框。与粘贴分两次主线程调用，
/// 中间在异步侧留一拍——不在主线程 sleep，不卡事件循环。
fn simulate_select_all(app: &AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        on_main(app, Duration::from_millis(800), || {
            modifier_click(A_KEYCODE)
        })
        .unwrap_or_else(|| Err("主线程未响应，全选请求未执行".to_string()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        modifier_click(0)
    }
}

/// 按住修饰键敲一个物理键（mac ⌘、其余 Ctrl）；⌘A / ⌘V 共用。
/// enigo 构造参数：`open_prompt_to_get_permissions` 默认是 **true**，于是每次
/// `Enigo::new` 都会调 `AXIsProcessTrustedWithOptions(prompt: true)`——没授权时
/// 每注入一次就弹一次「llm-switch 想使用辅助功能」（用户实报「永远弹系统设置」）。
/// 授权与否由面板自己说清楚（`can_simulate` + UI 状态行），系统框一律不许它弹。
#[cfg(target_os = "macos")]
fn no_prompt_settings() -> enigo::Settings {
    enigo::Settings {
        open_prompt_to_get_permissions: false,
        ..Default::default()
    }
}

fn modifier_click(keycode: u32) -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard};
    #[cfg(target_os = "macos")]
    let mut enigo = Enigo::new(&no_prompt_settings()).map_err(|e| e.to_string())?;
    #[cfg(not(target_os = "macos"))]
    let mut enigo = Enigo::new(&Default::default()).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let cmd = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let cmd = Key::Control;
    enigo
        .key(cmd, Direction::Press)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Other(keycode), Direction::Press)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Other(keycode), Direction::Release)
        .map_err(|e| e.to_string())?;
    enigo
        .key(cmd, Direction::Release)
        .map_err(|e| e.to_string())
}

/// 真正按下 ⌘V / Ctrl+V 的那一段；mac 下只允许由 `simulate_paste` 在主线程调。
fn paste_key_sequence() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return modifier_click(V_KEYCODE);
    #[cfg(not(target_os = "macos"))]
    {
        use enigo::{Direction, Enigo, Key, Keyboard, Settings};
        let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
        enigo
            .key(Key::Control, Direction::Press)
            .map_err(|e| e.to_string())?;
        enigo
            .key(Key::Unicode('v'), Direction::Press)
            .map_err(|e| e.to_string())?;
        enigo
            .key(Key::Unicode('v'), Direction::Release)
            .map_err(|e| e.to_string())?;
        enigo
            .key(Key::Control, Direction::Release)
            .map_err(|e| e.to_string())
    }
}

/// 注入失败的兜底广播：前端收到就重开面板并提示手动粘贴，绝不静默吞掉内容。
/// `target` 带上目标 App 名，面板才能说清「没送进 Ghostty」而不是含糊一句。
fn emit_inject_failed(app: &AppHandle, reason: &'static str) {
    let _ = app.emit(
        "prompt://inject-failed",
        serde_json::json!({ "reason": reason, "target": target().map(|t| t.name) }),
    );
}

/// 注入：收窗 → 焦点还给唤起前的 App → 模拟系统粘贴。
/// 剪贴板由前端在调用本命令前写好（用户手势上下文里 navigator.clipboard 才稳）。
///
/// 注入语义是**在目标输入框的光标处插入**：只发一次系统粘贴，不发 ⌘A、不发回车，
/// 所光标处原有内容不会被清空也不会被换行。唯一例外是目标此刻有选区——
/// 任何 App 的系统粘贴都是替换选区，这不是本面板能改变的语义。
/// 另外：注入会用条目内容覆盖剪贴板（用户原有剪贴板内容不恢复）。
pub fn inject(app: &AppHandle) {
    // 没授权就别收窗：收了窗等于把用户选的内容悄悄丢掉。这里直接回失败，面板留着
    // 显示「辅助功能未授权」与兜底文案。
    if !can_simulate() {
        tracing::warn!(target: "llm_switch::promptfloat", "注入中止：辅助功能未授权给当前构建");
        emit_inject_failed(app, "no-permission");
        return;
    }
    set_visible(app, false);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // 收窗是异步的系统行为，留一拍让窗口真消失，否则粘贴可能落进面板自己。
        tokio::time::sleep(Duration::from_millis(150)).await;
        let Some(pid) = reactivate_front(&handle) else {
            tracing::warn!(target: "llm_switch::promptfloat", "注入中止：没有可归还焦点的目标");
            emit_inject_failed(&handle, "no-target");
            return;
        };
        if !wait_until_front(&handle, pid, Duration::from_millis(1_200)).await {
            tracing::warn!(target: "llm_switch::promptfloat", pid, "注入中止：目标未在时限内成为前台");
            emit_inject_failed(&handle, "focus-lost");
            return;
        }
        // 注入方式由面板设置页决定：replace 先全选，粘贴就是整段替换。
        let replace = inject_mode(&handle) == "replace";
        if replace {
            if let Err(e) = simulate_select_all(&handle) {
                tracing::warn!(target: "llm_switch::promptfloat", error = %e, "全选失败（检查辅助功能授权）");
                emit_inject_failed(&handle, "paste-blocked");
                return;
            }
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
        tracing::debug!(target: "llm_switch::promptfloat", pid, mode = if replace { "replace" } else { "insert" }, "注入落点");
        if let Err(e) = simulate_paste(&handle) {
            tracing::warn!(target: "llm_switch::promptfloat", error = %e, "模拟粘贴失败（检查辅助功能授权）");
            emit_inject_failed(&handle, "paste-blocked");
        }
    });
}

/// 收窗并把焦点还给唤起前的 App，**等它真成了前台才返回 true**。
///
/// 宏 / 单动作类条目执行前必须先过这一关：CGEvent 只会打在**前台**窗口上，
/// 面板不收起就等于打在自己身上（与上面 `inject` 同一条纪律：收窗是异步的，
/// 盲等固定毫秒会粘空或打进正在消失的面板自己）。单独抽出来是因为宏执行
/// 不粘贴、不走 `inject`，但同样需要「先离场、再动手」。
pub async fn dismiss_to_target(app: &AppHandle) -> bool {
    set_visible(app, false);
    // 收窗是异步的系统行为，留一拍让窗口真消失。
    tokio::time::sleep(Duration::from_millis(150)).await;
    let Some(pid) = reactivate_front(app) else {
        tracing::warn!(target: "llm_switch::promptfloat", "宏执行中止：没有可归还焦点的目标");
        return false;
    };
    wait_until_front(app, pid, Duration::from_millis(1_200)).await
}

/// 打开系统「设置 → 隐私与安全性 → 辅助功能」面板（仅 mac 有意义）。
pub fn open_a11y_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

/// 收起前把位置与尺寸存死，下次放出原样摆回。
fn save_geometry(app: &AppHandle) {
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    if is_orb() {
        return; // 圆钮态的位置是圆钮的，别覆盖面板存档
    }
    if let Ok(p) = win.outer_position() {
        set_setting(app, POS_KEY, &format!("{},{}", p.x, p.y));
    }
    if let (Ok(size), Ok(scale)) = (win.inner_size(), win.scale_factor()) {
        set_setting(
            app,
            SIZE_KEY,
            &format!(
                "{},{}",
                size.width as f64 / scale,
                size.height as f64 / scale
            ),
        );
    }
}
