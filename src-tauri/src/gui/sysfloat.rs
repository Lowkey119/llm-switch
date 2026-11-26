//! 资源悬浮小窗（主计划 §10① / 切片 13）：无边框、置顶的实时监控小窗，
//! 显示两台机器的 RAM/VRAM/CPU/GPU/温度/功率。展示借鉴 unsloth 悬浮监控窗——
//! 只借鉴思路，未复制任何代码（AGPL 红线）。
//!
//! 与速率线（rateline）同一套窗口 flags 与位置记忆手法：位置存 settings
//! （物理像素），显隐走主线程（macOS 上从 IPC 线程直接 show 会静默失效）。
//! 「钉住」= 保持置顶；取消钉住后窗口退为普通层级，会被别的窗口盖住。
//!
//! 数据不在这里——前端直接吃 `get_sys_stats`（sysmon 的 TTL 缓存合流，
//! 多窗口共享一次 SSH），本模块只管窗口的生死、位置、尺寸与偏好。

use crate::gateway::State;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};

/// 窗口标签（float-main.tsx 按它分流前端组件）。
pub const LABEL: &str = "sysfloat";
/// 可见性偏好（重启后恢复）；「1」/「0」。
const VIS_KEY: &str = "sysfloat.visible";
const POS_KEY: &str = "sysfloat.pos";
const SIZE_KEY: &str = "sysfloat.size";
const METRICS_KEY: &str = "sysfloat.metrics";
const PIN_KEY: &str = "sysfloat.pin";
/// 默认尺寸（逻辑像素）；前端量完内容用 fit 收。
const WIDTH: f64 = 264.0;
const HEIGHT: f64 = 220.0;
/// 默认显示的指标（设置页勾选，全量清单在前端 METRIC_KEYS 同源）。
const DEFAULT_METRICS: [&str; 6] = ["cpu", "mem", "vram", "gpu", "temp", "power"];
/// 拖动落位的静默窗口：松手后静默这么久才把最终位置写库。
const SETTLE_MS: u64 = 600;
/// 贴边吸附阈值（逻辑像素），与速率线/浮窗同值——三处手感一致。
const SNAP_THRESHOLD: f64 = 28.0;
/// 吸附自身会触发 Moved，用这把锁挡住自激递归（速率线同款）。
static SNAPPING: AtomicBool = AtomicBool::new(false);
/// 圆钮直径（逻辑像素），与浮窗/速率线的圆钮同款尺寸（.float-orb CSS 同值）。
const ORB: f64 = 54.0;

/// 圆钮态：`(是否收成圆钮, 停靠侧 -1 左 / 1 右)`。收成圆钮后不再重复裁决，
/// 点圆钮展开才回到面板态（与浮窗手感一致）。
fn orb_state() -> &'static Mutex<(bool, i8)> {
    static O: std::sync::OnceLock<Mutex<(bool, i8)>> = std::sync::OnceLock::new();
    O.get_or_init(|| Mutex::new((false, 0)))
}

fn is_orb() -> bool {
    orb_state().lock().expect("sysfloat orb state poisoned").0
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 待落库的拖动：`(时间戳, x, y)`。Moved 每帧刷新时间戳并 spawn 一次延时落库，
/// 只有静默到点、且没被更新的移动作废的那次真写——节流窗口会把松手前的
/// 最后一帧吞掉，重启就回到半路坐标（rateline schedule_snap 同款手法）。
fn pending_move() -> &'static Mutex<Option<(u64, i32, i32)>> {
    static P: std::sync::OnceLock<Mutex<Option<(u64, i32, i32)>>> = std::sync::OnceLock::new();
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
            tracing::warn!(target: "llm_switch::sysfloat", error = %e, key, "偏好落库失败");
        }
    }
}

/// 指标显隐清单（非法值过滤；空清单 = 全不显，是用户的选择，尊重）。
pub fn metrics(app: &AppHandle) -> Vec<String> {
    setting(app, METRICS_KEY)
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_else(|| DEFAULT_METRICS.iter().map(|s| s.to_string()).collect())
}

pub fn set_metrics(app: &AppHandle, list: &[String]) -> Vec<String> {
    if let Ok(raw) = serde_json::to_string(list) {
        set_setting(app, METRICS_KEY, &raw);
    }
    metrics(app)
}

/// 钉住（置顶）状态，缺省钉住——悬浮监视窗不置顶就失去意义。
pub fn pin(app: &AppHandle) -> bool {
    setting(app, PIN_KEY).map(|v| v != "0").unwrap_or(true)
}

pub fn set_pin(app: &AppHandle, on: bool) -> bool {
    set_setting(app, PIN_KEY, if on { "1" } else { "0" });
    if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.set_always_on_top(on);
    }
    on
}

/// 偏好上该不该显示（启动恢复与设置页初始态都看它——窗口对象可能还没建）。
/// 窗口已存在时以**实时可见性**为准（用户 2026-11-25 拍板）：用户用 Cmd+W 之类别的路子
/// 收掉小窗时偏好还留着 1，顶栏绿点会赖着不走。
pub fn visible_pref(app: &AppHandle) -> bool {
    if let Some(win) = app.get_webview_window(LABEL) {
        if let Ok(vis) = win.is_visible() {
            return vis;
        }
    }
    setting(app, VIS_KEY).map(|v| v == "1").unwrap_or(false)
}

/// 托盘开关：开着就收、收起就放（偏好即口径，与设置页开关同源）。
pub fn toggle(app: &AppHandle) -> bool {
    let vis = !visible_pref(app);
    set_visible(app, vis);
    vis
}

/// 设定可见性并持久化偏好。macOS 上窗口显隐必须在主线程（floatwin 踩过的坑）。
pub fn set_visible(app: &AppHandle, visible: bool) -> bool {
    set_setting(app, VIS_KEY, if visible { "1" } else { "0" });
    let handle = app.clone();
    if app
        .run_on_main_thread(move || apply(&handle, visible))
        .is_err()
    {
        apply(app, visible);
    }
    visible
}

fn apply(app: &AppHandle, visible: bool) {
    if visible {
        match app.get_webview_window(LABEL) {
            Some(win) => {
                let _ = win.show();
                let _ = win.set_focus();
            }
            None => {
                if let Err(e) = build(app) {
                    tracing::warn!(target: "llm_switch::sysfloat", error = %e, "资源小窗创建失败");
                }
            }
        }
    } else if let Some(win) = app.get_webview_window(LABEL) {
        save_geometry(app);
        let _ = win.hide();
    }
    // 与浮窗同一口径：可见性一变就广播，顶栏绿点不等轮询（用户 2026-11-25）。
    let _ = app.emit("floats://vis", ());
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let (w0, h0) = saved(app, SIZE_KEY).unwrap_or((WIDTH, HEIGHT));
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("float.html".into()))
        .title("llm-switch 资源小窗")
        .decorations(false)
        .transparent(true)
        .always_on_top(pin(app))
        .skip_taskbar(true)
        .resizable(false)
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
    win.on_window_event(move |ev| {
        if let WindowEvent::Moved(pos) = ev {
            remember_move(&handle, *pos);
            judge_edge(&handle);
        }
    });
    let _ = win.show();
    Ok(())
}

/// 落位：有存档用存档；默认贴主屏工作区右下角（浮窗在右上、速率线在左下，互不打架）。
fn place(app: &AppHandle, win: &tauri::WebviewWindow) {
    if let Some((x, y)) = saved_pos(app) {
        let _ = win.set_position(PhysicalPosition::new(x, y));
        return;
    }
    let Some((wx, wy, ww, wh, scale)) = crate::gui::floatwin::primary_work_area(app) else {
        return;
    };
    let Ok(size) = win.inner_size() else {
        return;
    };
    // 边距要大于吸附阈值：默认落位是悬浮观感，停靠靠用户自己拖出来。
    let margin = 36.0 * scale;
    let x = wx + ww - size.width as f64 - margin;
    let y = wy + wh - size.height as f64 - margin;
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
    if w < 80.0 || h < 40.0 {
        return None;
    }
    Some((w, h))
}

/// 拖动落位：Moved 每帧只记账 + 排一次延时，静默到点的那次才写库（见 pending_move）。
/// 初始摆位（place 里的 set_position）也会走到这里——默认落位同样被记住，
/// 与 rateline 行为一致。
fn remember_move(app: &AppHandle, pos: PhysicalPosition<i32>) {
    let stamp = now_ms();
    {
        let mut guard = pending_move().lock().expect("sysfloat move clock poisoned");
        *guard = Some((stamp, pos.x, pos.y));
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
        let due = {
            let mut guard = pending_move().lock().expect("sysfloat move clock poisoned");
            match *guard {
                Some((s, x, y)) if s == stamp => {
                    *guard = None;
                    Some((x, y))
                }
                // 期间又动了：这一发作废，让最后一次移动自己来写。
                _ => None,
            }
        };
        if let Some((x, y)) = due {
            set_setting(&handle, POS_KEY, &format!("{},{}", x, y));
            // 落位后顺手贴边：离工作区顶/底够近就吸齐（速率线同款停靠）。
            snap_to_edge(&handle);
        }
    });
}

/// 贴边裁决（拖动中每帧，浮窗同款时序）：左右贴边 → 收成圆钮；上下贴边由
/// snap_to_edge 在松手静默后贴合。已收成圆钮就不再裁决（点圆钮才展开）。
/// 接缝（多屏拼接的内缝）没做压除——小窗横向位置由用户自己摆，代价可接受。
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
    *orb_state().lock().expect("sysfloat orb state poisoned") = (true, side);
    SNAPPING.store(true, Ordering::SeqCst);
    let orb = (ORB * scale) as u32;
    let x = if side < 0 { wx } else { wx + ww - orb as f64 };
    let y = (pos.y as f64).clamp(wy, (wy + wh - orb as f64).max(wy));
    let _ = win.set_size(PhysicalSize::new(orb, orb));
    let _ = win.set_position(PhysicalPosition::new(x as i32, y as i32));
    set_setting(app, POS_KEY, &format!("{},{}", x as i32, y as i32));
    let _ = app.emit(
        "sysfloat://shape",
        serde_json::json!({ "orb": true, "side": side }),
    );
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_millis(120)).await;
        SNAPPING.store(false, Ordering::SeqCst);
    });
}

/// 点圆钮展开：恢复面板尺寸（库存尺寸，前端量完内容还会 fit 精修），
/// 位置跟圆钮左上角走、钳回工作区。
pub fn expand(app: &AppHandle) {
    {
        let mut guard = orb_state().lock().expect("sysfloat orb state poisoned");
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
        "sysfloat://shape",
        serde_json::json!({ "orb": false, "side": 0 }),
    );
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_millis(120)).await;
        SNAPPING.store(false, Ordering::SeqCst);
    });
}

/// 当前形态（前端挂载时先问一次，之后吃 sysfloat://shape 广播）。
pub fn shape() -> serde_json::Value {
    let (orb, side) = *orb_state().lock().expect("sysfloat orb state poisoned");
    serde_json::json!({ "orb": orb, "side": side })
}

/// 贴到所在屏工作区的顶边或底边（离得够近才吸，左右不管——小窗的横向位置
/// 是用户自己摆的，与速率线同一套停靠口径）。
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
        return; // 已经贴齐，别再动（也免得自激）
    }
    SNAPPING.store(true, Ordering::SeqCst);
    let landed = PhysicalPosition::new(pos.x, ny as i32);
    let _ = win.set_position(landed);
    // 吸附后的这一笔必须强制落库（settle 会被下面的 Moved 再排一次，但那是
    // 同一个值；这里先写死，重启才不回吸附前的半路坐标）。
    set_setting(app, POS_KEY, &format!("{},{}", landed.x, landed.y));
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(Duration::from_millis(120)).await;
        SNAPPING.store(false, Ordering::SeqCst);
    });
}

/// 收起前把位置与尺寸存死，下次放出原样摆回。
fn save_geometry(app: &AppHandle) {
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
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

/// 前端量完内容回报尺寸（逻辑像素）；左上角不动，只收大小。圆钮态不收——
/// 54px 是圆钮自己的尺寸，前端已挡，这里再兜一道。
pub fn fit(win: &tauri::WebviewWindow, width: f64, height: f64) {
    if is_orb() {
        return;
    }
    let scale = win.scale_factor().unwrap_or(1.0);
    let w = (width.max(120.0) * scale) as u32;
    let h = (height.max(60.0) * scale) as u32;
    let _ = win.set_size(PhysicalSize::new(w, h));
    // ResizeObserver 会反复量到同一个尺寸；与库存一致就不必再写。
    let app = win.app_handle();
    let raw = format!("{},{}", width.max(120.0), height.max(60.0));
    if setting(app, SIZE_KEY).as_deref() != Some(raw.as_str()) {
        set_setting(app, SIZE_KEY, &raw);
    }
    // 缩放/勾选让窗口变了大小：贴着顶/底停靠的那一侧要跟着重新吸齐
    // （贴底时变高会把下缘顶出屏幕，吸附把它拉回来）。
    snap_to_edge(app);
}

/// 启动时按偏好恢复（setup 里 rateline::restore 旁边调用）。
pub fn restore(app: &AppHandle) {
    if visible_pref(app) {
        set_visible(app, true);
    }
}
