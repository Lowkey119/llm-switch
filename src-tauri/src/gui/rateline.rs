//! 单行速率线：一个代理一个窗口，各自可拖、各自记位置。
//!
//! 为什么不是一个浮窗切模式：同时开着 pi 和 codex 是常态，速率线要各贴各的终端，
//! 一个窗口两种形态只能轮流显示一家。多窗口才是这个需求的形状——窗口标签就是
//! 归属（`rate-pi`），前端读自己的标签决定画谁，数据仍与多行浮窗同源。
//!
//! 位置自己存 settings（`rateline.pos.<app>`，物理像素），不听 window-state
//! 插件的：那插件按固定标签恢复，而我们的标签是动态的，两套位置打架不值一次。
//!
//! 与浮窗同一套窗口 flags（无边框/透明/置顶/不进任务栏），几何一律物理像素。

use crate::gateway::State;
use crate::gui::floatwin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};

/// 窗口标签前缀；后缀就是归属代理名（`rate-pi`）。
pub const LABEL_PREFIX: &str = "rate-";
/// 启用清单与位置在 settings 里的键前缀。
const ENABLED_KEY: &str = "rateline.enabled";
const POS_KEY: &str = "rateline.pos.";
/// 每条线的窗口尺寸（逻辑像素 `宽,高`）：关闭时存、打开时拿来建窗，
/// 免得每次放出都先闪一下默认的 360×26 再被前端量着收回去。
const SIZE_KEY: &str = "rateline.size.";
/// 首次落位时多条速率线彼此错开的间距（逻辑像素，一行文字再留点缝）。
const STACK_STEP: f64 = 30.0;
/// 拖动中存位置的节流窗口：每 500ms 最多写一次库。
const SAVE_THROTTLE_MS: u64 = 500;
/// 总开关的键：关掉只是把窗口收起来，启用清单（代理页那些勾选）一个字不动。
const MASTER_KEY: &str = "rateline.master";
/// 自动弹出开关（#46）：代理一有请求就点亮它家的速率线，缺省开。
const AUTO_KEY: &str = "rateline.auto";
/// 空闲自动收起阈值：某代理连续这么久没有在飞请求，就把它家的线藏起来。
const IDLE_HIDE: Duration = Duration::from_secs(60);
/// 拖动静默多久算「松手」，到点才做吸附。中途 set_position 会在系统拖拽里留下
/// 永久偏移（浮窗为此删光了计时器），速率线只需要「松手时贴齐」，静默判定更稳。
const SETTLE_MS: u64 = 260;
/// 吸附阈值（逻辑像素），与浮窗的 EDGE_THRESHOLD 同值，两处手感一致。
const SNAP_THRESHOLD: f64 = 28.0;
/// 吸附自身也会触发 Moved，用这把锁挡住自激递归。
static SNAPPING: AtomicBool = AtomicBool::new(false);

fn label(app: &str) -> String {
    format!("{LABEL_PREFIX}{app}")
}

/// 从标签还原代理名（前端也做同样的事，两边口径一致）。
pub fn app_of_label(label: &str) -> Option<&str> {
    label.strip_prefix(LABEL_PREFIX)
}

fn last_save() -> &'static Mutex<std::collections::HashMap<String, u64>> {
    static M: OnceLock<Mutex<std::collections::HashMap<String, u64>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

fn last_move() -> &'static Mutex<std::collections::HashMap<String, u64>> {
    static M: OnceLock<Mutex<std::collections::HashMap<String, u64>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// 最近一次落库的尺寸，用来挡住「内容没变但前端照样回报同一尺寸」的重复写。
fn last_size() -> &'static Mutex<std::collections::HashMap<String, String>> {
    static M: OnceLock<Mutex<std::collections::HashMap<String, String>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// 每条线最近一次「它家在飞」的时刻（空闲自动收起的计时基准）。
fn last_busy() -> &'static Mutex<std::collections::HashMap<String, std::time::Instant>> {
    static M: OnceLock<Mutex<std::collections::HashMap<String, std::time::Instant>>> =
        OnceLock::new();
    M.get_or_init(Default::default)
}

fn now_ms() -> u64 {
    crate::types::now_ms() as u64
}

fn store_of(app: &AppHandle) -> Option<std::sync::Arc<crate::store::Store>> {
    app.try_state::<std::sync::Arc<State>>()
        .map(|st| st.store.clone())
}

/// 旧偏好迁移：单窗口时代把唯一的浮窗切成单行（`float.rateline`），那时多行面板
/// 会被顶掉。现在速率线是独立窗口，老偏好只翻译成「给那一家开一条线」，
/// 然后把旧键删掉——否则每次启动都白搬一遍。
fn migrate_legacy(store: &crate::store::Store) {
    let Some(raw) = store.get_setting("float.rateline").ok().flatten() else {
        return;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    if v.get("single").and_then(|x| x.as_bool()).unwrap_or(false) {
        if let Some(target) = v.get("app").and_then(|x| x.as_str()) {
            let mut apps = enabled(store);
            if !apps.iter().any(|a| a == target) {
                apps.push(target.to_string());
                set_enabled(store, &apps);
            }
        }
    }
    let _ = store.set_setting("float.rateline", "");
}

/// 当前开着速率线的代理（按启用清单的顺序）。
pub fn list(app: &AppHandle) -> Vec<String> {
    let Some(store) = store_of(app) else {
        return Vec::new();
    };
    enabled(&store)
}

fn enabled(store: &crate::store::Store) -> Vec<String> {
    store
        .get_setting(ENABLED_KEY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

fn set_enabled(store: &crate::store::Store, apps: &[String]) {
    if let Ok(raw) = serde_json::to_string(apps) {
        if let Err(e) = store.set_setting(ENABLED_KEY, &raw) {
            tracing::warn!(target: "llm_switch::rateline", error = %e, "启用清单落库失败");
        }
    }
}

/// 总开关是否开着（缺省开：装了速率线就该看得见）。
pub fn master_on(app: &AppHandle) -> bool {
    store_of(app)
        .and_then(|s| s.get_setting(MASTER_KEY).ok().flatten())
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// 托盘上的总开关：关掉把所有速率线窗口收起来（启用清单不动，代理页的勾选还在），
/// 打开则把清单里那些逐条点回来。
pub fn set_master(app: &AppHandle, on: bool) -> bool {
    if let Some(store) = store_of(app) {
        if let Err(e) = store.set_setting(MASTER_KEY, if on { "1" } else { "0" }) {
            tracing::warn!(target: "llm_switch::rateline", error = %e, "总开关落库失败");
        }
    }
    if on {
        restore(app);
    } else {
        for target in enabled_all(app) {
            // 收起前把位置和尺寸都存死：下次放出照原样摆回去。
            save_geometry(app, &target);
            if let Some(win) = app.get_webview_window(&label(&target)) {
                let _ = win.hide();
            }
        }
    }
    on
}

/// 自动弹出开关是否开着（#46，缺省开：代理有请求就该看见）。
pub fn auto_on(app: &AppHandle) -> bool {
    store_of(app)
        .and_then(|s| s.get_setting(AUTO_KEY).ok().flatten())
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// 托盘开关：翻自动弹出偏好并回传新值（勾标随刷新重建）。
pub fn set_auto(app: &AppHandle, on: bool) -> bool {
    if let Some(store) = store_of(app) {
        if let Err(e) = store.set_setting(AUTO_KEY, if on { "1" } else { "0" }) {
            tracing::warn!(target: "llm_switch::rateline", error = %e, "自动弹出开关落库失败");
        }
    }
    on
}

/// 网关挂号回调（#46）：代理有请求自动点亮它家的速率线。三道闸——自动开关关着
/// 不动；总开关收起（用户明确整批藏过）不越权弹窗；已经看得见就不折腾。
pub fn auto_open(app: &AppHandle, target: &str) {
    if !auto_on(app) || !master_on(app) || is_open(app, target) {
        return;
    }
    open(app, target);
}

/// 这条线**此刻真的看得见**吗：总开关开着、在启用清单里、窗口也确实可见。
///
/// 托盘的勾标必须用它。只看启用清单会出现「勾着却什么都看不见」——总开关收起时
/// 清单一个字没动，逐家却全打着勾，用户点一下反而像没反应。
pub fn is_open(app: &AppHandle, target: &str) -> bool {
    if !master_on(app) || !enabled_all(app).iter().any(|a| a == target) {
        return false;
    }
    match app.get_webview_window(&label(target)) {
        Some(win) => win.is_visible().unwrap_or(false),
        // 清单里有、窗口却没建起来（建窗失败或已被关）：看不见就是没开。
        None => false,
    }
}

/// 启用清单（不看总开关）。
fn enabled_all(app: &AppHandle) -> Vec<String> {
    store_of(app).map(|s| enabled(&s)).unwrap_or_default()
}

/// 空闲自动收起（monitor 每秒巡检）：某代理连续 IDLE_HIDE 秒没有在飞请求，
/// 就把它家的线藏起来——只 hide 不 close，启用清单与位置都不动，
/// 下轮流量 auto_open 再点亮。自动弹出开关关着时完全不碰：用户自己开的窗用户自己管。
pub fn idle_sweep(app: &AppHandle, busy: &std::collections::HashSet<String>) {
    if !auto_on(app) || !master_on(app) {
        return;
    }
    let now = std::time::Instant::now();
    let mut clock = last_busy().lock().expect("rateline busy clock poisoned");
    let apps = enabled_all(app);
    for target in &apps {
        if busy.contains(target) {
            clock.insert(target.clone(), now);
            continue;
        }
        // 从没记过时（手动开的线或刚进清单）：首轮先记时，下轮起才计空闲。
        let Some(at) = clock.get(target).copied() else {
            clock.insert(target.clone(), now);
            continue;
        };
        if now.duration_since(at) < IDLE_HIDE {
            continue;
        }
        if let Some(win) = app.get_webview_window(&label(target)) {
            if win.is_visible().unwrap_or(false) {
                // 与总开关收起同口径：存死几何再藏，下轮放出原样摆回。
                save_geometry(app, target);
                let _ = win.hide();
            }
        }
    }
    // 已移出清单的代理别在时钟里发霉。
    clock.retain(|t, _| apps.iter().any(|a| a == t));
}

/// 打开某代理的速率线（已开则置顶）。
pub fn open(app: &AppHandle, target: &str) -> bool {
    let Some(store) = store_of(app) else {
        return false;
    };
    let mut apps = enabled(&store);
    if !apps.iter().any(|a| a == target) {
        apps.push(target.to_string());
        set_enabled(&store, &apps);
    }
    let idx = apps.iter().position(|a| a == target).unwrap_or(0);
    // 总开关关着时代理页单独点亮一家：把总开关一起打开并整批恢复。
    // 只放出自家那一条会留下「清单里三家只回来一条、另外两条还藏着」的半开状态。
    if !master_on(app) {
        return set_master(app, true);
    }
    if let Some(win) = app.get_webview_window(&label(target)) {
        let _ = win.show();
        return true;
    }
    if let Err(e) = build(app, target, idx) {
        tracing::warn!(target: "llm_switch::rateline", error = %e, app = target, "速率线窗口创建失败");
        return false;
    }
    true
}

/// 关闭某代理的速率线并把它从启用清单里摘掉。
pub fn close(app: &AppHandle, target: &str) -> bool {
    if let Some(store) = store_of(app) {
        let apps: Vec<String> = enabled(&store)
            .into_iter()
            .filter(|a| a != target)
            .collect();
        set_enabled(&store, &apps);
    }
    match app.get_webview_window(&label(target)) {
        Some(win) => {
            save_geometry(app, target);
            let _ = win.close();
            true
        }
        None => false,
    }
}

fn build(app: &AppHandle, target: &str, idx: usize) -> tauri::Result<()> {
    let (w0, h0) = saved_size(app, target).unwrap_or((360.0, 26.0));
    let win = WebviewWindowBuilder::new(app, label(target), WebviewUrl::App("float.html".into()))
        .title(format!("llm-switch 速率线 · {target}"))
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
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
    place(app, &win, target, idx);
    let handle = app.clone();
    let name = target.to_string();
    win.on_window_event(move |ev| {
        if let WindowEvent::Moved(pos) = ev {
            remember_move(&handle, &name, *pos, false);
            schedule_snap(&handle, &name);
        }
    });
    let _ = win.show();
    Ok(())
}

/// 落位：有存档用存档，没有就贴主屏工作区下沿、按序号往上错开。
fn place(app: &AppHandle, win: &tauri::WebviewWindow, target: &str, idx: usize) {
    if let Some((x, y)) = saved_pos(app, target) {
        let _ = win.set_position(PhysicalPosition::new(x, y));
        return;
    }
    let Some((wx, wy, _ww, wh, wscale)) = floatwin::primary_work_area(app) else {
        return;
    };
    let h = 26.0 * wscale;
    let x = wx + 24.0 * wscale;
    let y = wy + wh - h - (idx as f64 * STACK_STEP * wscale);
    let _ = win.set_position(PhysicalPosition::new(x as i32, y.max(wy) as i32));
}

/// 存下来的窗口尺寸（逻辑像素）。
fn saved_size(app: &AppHandle, target: &str) -> Option<(f64, f64)> {
    let store = store_of(app)?;
    let raw = store
        .get_setting(&format!("{SIZE_KEY}{target}"))
        .ok()
        .flatten()?;
    let mut it = raw.split(',');
    let w = it.next()?.trim().parse().ok()?;
    let h = it.next()?.trim().parse().ok()?;
    if w < 40.0 || h < 10.0 {
        return None;
    }
    Some((w, h))
}

fn saved_pos(app: &AppHandle, target: &str) -> Option<(i32, i32)> {
    let store = store_of(app)?;
    let raw = store
        .get_setting(&format!("{POS_KEY}{target}"))
        .ok()
        .flatten()?;
    let mut it = raw.split(',');
    let x = it.next()?.trim().parse().ok()?;
    let y = it.next()?.trim().parse().ok()?;
    Some((x, y))
}

/// 拖动一停就记位置（节流：拖动每帧都来，别把库写爆）。吸附后的那一次必须强制写，
/// 否则重启会回到吸附前的旧坐标。
fn remember_move(app: &AppHandle, target: &str, pos: PhysicalPosition<i32>, force: bool) {
    let now = now_ms();
    {
        let mut guard = last_save().lock().expect("rateline save clock poisoned");
        if !force && guard.get(target).copied().unwrap_or(0) + SAVE_THROTTLE_MS > now {
            return;
        }
        guard.insert(target.to_string(), now);
    }
    if let Some(store) = store_of(app) {
        let raw = format!("{},{}", pos.x, pos.y);
        if let Err(e) = store.set_setting(&format!("{POS_KEY}{target}"), &raw) {
            tracing::warn!(target: "llm_switch::rateline", error = %e, app = target, "速率线位置落库失败");
        }
    }
}

/// 排一次「松手后吸附」：拖动期间每帧都重排，只有静默到点的那次真动手。
fn schedule_snap(app: &AppHandle, target: &str) {
    if SNAPPING.load(Ordering::SeqCst) {
        return;
    }
    let stamp = now_ms();
    last_move()
        .lock()
        .expect("rateline move clock poisoned")
        .insert(target.to_string(), stamp);
    let handle = app.clone();
    let name = target.to_string();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(SETTLE_MS)).await;
        let fresh = last_move()
            .lock()
            .expect("rateline move clock poisoned")
            .get(&name)
            .copied()
            .unwrap_or(0);
        // 期间用户又动了：这一发作废，让最后一次移动自己来吸。
        if fresh != stamp {
            return;
        }
        snap_to_edge(&handle, &name);
    });
}

/// 贴到所在屏工作区的顶边或底边（离得够近才吸，左右不管——速率线要停在终端
/// 状态线旁边，横向位置是用户自己摆的）。
fn snap_to_edge(app: &AppHandle, target: &str) {
    let Some(win) = app.get_webview_window(&label(target)) else {
        return;
    };
    let Some((_wx, wy, _ww, wh, scale)) = floatwin::work_area_of(&win) else {
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
    remember_move(app, target, landed, true);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        let _ = handle;
        SNAPPING.store(false, Ordering::SeqCst);
    });
}

/// 存下窗口当前的位置与尺寸（逻辑像素）。总开关收起、单条关闭时各存一次，
/// 下次放出原样恢复。
fn save_geometry(app: &AppHandle, target: &str) {
    let Some(store) = store_of(app) else { return };
    let Some(win) = app.get_webview_window(&label(target)) else {
        return;
    };
    if let Ok(p) = win.outer_position() {
        let _ = store.set_setting(&format!("{POS_KEY}{target}"), &format!("{},{}", p.x, p.y));
    }
    if let (Ok(size), Ok(scale)) = (win.inner_size(), win.scale_factor()) {
        let raw = format!(
            "{},{}",
            size.width as f64 / scale,
            size.height as f64 / scale
        );
        let _ = store.set_setting(&format!("{SIZE_KEY}{target}"), &raw);
    }
}

/// 前端量完内容回报尺寸（逻辑像素）；左上角不动，只收大小。
/// 顺手记进 settings：缩放或模型名长度变了，下次放出就是新的尺寸。
pub fn fit(win: &tauri::WebviewWindow, width: f64, height: f64) {
    let scale = win.scale_factor().unwrap_or(1.0);
    let w = (width.max(80.0) * scale) as u32;
    let h = (height.max(16.0) * scale) as u32;
    let _ = win.set_size(PhysicalSize::new(w, h));
    if let (Some(store), Some(target)) = (
        win.app_handle()
            .try_state::<std::sync::Arc<State>>()
            .map(|s| s.store.clone()),
        app_of_label(win.label()),
    ) {
        let raw = format!("{},{}", width.max(80.0), height.max(16.0));
        {
            let mut guard = last_size().lock().expect("rateline size clock poisoned");
            if guard.get(target).map(|s| s.as_str()) == Some(raw.as_str()) {
                return;
            }
            guard.insert(target.to_string(), raw.clone());
        }
        if let Err(e) = store.set_setting(&format!("{SIZE_KEY}{target}"), &raw) {
            tracing::warn!(target: "llm_switch::rateline", error = %e, app = target, "速率线尺寸落库失败");
        }
    }
}

/// 启动时把上次开着的速率线逐个点回来。
pub fn restore(app: &AppHandle) {
    let Some(store) = store_of(app) else {
        return;
    };
    migrate_legacy(&store);
    if !master_on(app) {
        return;
    }
    for (idx, target) in enabled(&store).into_iter().enumerate() {
        // 总开关关掉时窗口只是 hide 着，对象还在。这里必须 show 而不是「存在就跳过」，
        // 否则关一次就再也打不开了（踩过：托盘开关只能关不能开）。
        if let Some(win) = app.get_webview_window(&label(&target)) {
            let _ = win.show();
            continue;
        }
        if let Err(e) = build(app, &target, idx) {
            tracing::warn!(target: "llm_switch::rateline", error = %e, app = target, "恢复速率线失败");
        }
    }
}
