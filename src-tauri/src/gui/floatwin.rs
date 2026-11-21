//! 浮动面板窗口（M12-C2a/C2b）：无边框、透明、置顶的小窗，只显示「此刻在飞什么」。
//!
//! 行为对齐参考实现 `~/mytools/tokbar/main.swift`：
//! · 任意位置可拖动（前端 `startDragging`，不是局部拖拽把手）
//! · 双击展开态切换「多行 ⇄ 前 4 行」（多行上限 12 行、高度 ≤ 屏幕 70%），
//!   双击圆钮展开成面板并恢复折叠前的行数模式
//! · 拖到左右边缘 28pt 内 → 缩成贴在边上的圆形图标钮；上下边缘 → 保持形态贴合，
//!   贴合/缩钮都走 0.18s 缓动（tokbar `moveTo` 同款），动画期间挂判定锁
//! · 屏幕归属按重叠面积判定（tokbar `screenFrame` 同款），多屏拼接不误判
//! · 拖动进行中 fit/zoom 只记账不落位——原生拖拽按 delta 走，中途 set_position
//!   会变成窗口与光标的永久错位（「拖动不跟鼠标」）
//! · Cmd+滚轮 / Cmd+±/0 缩放（缩放值由前端持有并持久化，这里只负责随之改窗口尺寸）
//!
//! 前端是另一个入口页 `float.html`（Vite 多入口）：Tauri 的资源解析不吃
//! `index.html?view=float` 这种带查询串的 URL，窗口会建成一张白纸。
//!
//! 几何一律用**物理像素**算（`work_area` 本来就是物理的，逻辑常量乘 scale_factor），
//! 混用两套单位是这类代码最容易踩且最难看的坑。

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

/// 窗口标签（capabilities 与 window-state 黑名单都认这个名字）。
pub const LABEL: &str = "float";
/// 展开态默认尺寸（逻辑像素）；真实高度由前端量完内容再 `fit` 回来。
const WIDTH: f64 = 480.0;
const HEIGHT: f64 = 132.0;
/// 圆钮直径（逻辑像素）。
const BUTTON: f64 = 54.0;
/// 贴边判定阈值（逻辑像素），与 tokbar 的 edgeThreshold 一致。
const EDGE_THRESHOLD: f64 = 28.0;
/// 距屏幕边缘的默认间距。
const MARGIN: f64 = 18.0;
/// 贴合/缩钮动画时长（毫秒），与 tokbar `moveTo` 的 0.18s 一致。
const ANIM_MS: u64 = 180;
/// 拖动静默判定：距最后一次真实移动不足这个时长就算「用户还在动窗口」，
/// 这期间 fit/zoom 一律记账延后（原生拖拽是增量位移，拖动中的 set_position
/// 会变成窗口与光标的**永久**错位——「拖动不跟鼠标」的元凶）。
const SETTLE_MS: i64 = 400;

/// 浮窗形态：面板态 / 圆钮态 + 贴在哪条侧边。
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub expanded: bool,
    /// -1 左贴边，0 自由，1 右贴边
    pub edge: i8,
    /// 展开态的内容尺寸（逻辑像素），由前端测量后经 `fit` 更新。
    pub width: f64,
    pub height: f64,
    /// 展开态是否进入多行态（tokbar 的 `multiRow` 同款语义）：默认只前
    /// DEFAULT_ROWS 行，双击展开多行（内容自适应，上限 MAX_ROWS 行、
    /// 高度不超过屏幕 70%），再双击收回。**折叠成圆钮时不抹掉**——
    /// 双击圆钮展开时恢复折叠前的行数模式（tokbar「恢复折叠前的行数」）。
    pub show_all: bool,
}

impl Default for Shape {
    fn default() -> Self {
        Shape {
            expanded: true,
            edge: 0,
            width: WIDTH,
            height: HEIGHT,
            show_all: false,
        }
    }
}

fn shape() -> &'static Mutex<Shape> {
    static SHAPE: OnceLock<Mutex<Shape>> = OnceLock::new();
    SHAPE.get_or_init(|| Mutex::new(Shape::default()))
}
/// 我们自己挪窗口时置位，避免自激（set_position 也会触发 Moved）。
static SNAPPING: AtomicBool = AtomicBool::new(false);
/// 最近一次「用户真实移动窗口」的时刻（unix 毫秒）。自激 Moved 因 SNAPPING
/// 在 on_moved 就被拦下、不会刷新，所以这个时间戳近似等于「拖动进行中」。
static LAST_MOVE: AtomicI64 = AtomicI64::new(0);
/// 挂起的 fit/zoom 记账：拖动中来的一律记最新值，静默后再落一次。
fn pending_fit() -> &'static Mutex<Option<(f64, f64, bool)>> {
    static PENDING: OnceLock<Mutex<Option<(f64, f64, bool)>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 用户是否还在动窗口（拖动中的 Moved 每帧刷新时间戳）。
fn moving_recently() -> bool {
    now_ms() - LAST_MOVE.load(Ordering::SeqCst) < SETTLE_MS
}

pub fn toggle(app: &AppHandle) -> bool {
    let vis = is_visible(app);
    set_visible(app, !vis);
    !vis
}

/// 显式设定可见性。macOS 上窗口显隐必须在主线程，从 IPC 线程或定时器任务里直接
/// `show()` 会静默失效（踩过：日志说窗口建好了，屏幕上什么都没有）。
pub fn set_visible(app: &AppHandle, visible: bool) -> bool {
    tracing::info!(target: "llm_switch::float", visible, "set_visible 入口");
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
    tracing::info!(target: "llm_switch::float", visible, "apply 在主线程");
    if visible {
        // 大浮窗一出现就把整批单条速率线藏掉（用户 2026-11-19）：两扇同屏纯重复占位。
        super::rateline::hide_all(app);
        match app.get_webview_window(LABEL) {
            Some(win) => {
                let _ = win.show();
                let _ = win.set_focus();
            }
            None => {
                if let Err(e) = build(app) {
                    tracing::warn!(target: "llm_switch::float", error = %e, "浮窗创建失败");
                }
            }
        }
    } else if let Some(win) = app.get_webview_window(LABEL) {
        let _ = win.hide();
    }
}

pub fn is_visible(app: &AppHandle) -> bool {
    app.get_webview_window(LABEL)
        .map(|w| w.is_visible().unwrap_or(false))
        .unwrap_or(false)
}

fn build(app: &AppHandle) -> tauri::Result<()> {
    let win = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("float.html".into()))
        .title("llm-switch 浮窗")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .shadow(false)
        .visible(false)
        .inner_size(WIDTH, HEIGHT)
        .visible_on_all_workspaces(true)
        .build()?;
    place_top_right(app, &win);
    let _ = win.show();
    Ok(())
}

/// 摆到主屏工作区的右上角。
fn place_top_right(app: &AppHandle, win: &WebviewWindow) {
    let Some(wa) = WorkArea::primary(app) else {
        tracing::warn!(target: "llm_switch::float", "枚举不到显示器，浮窗留在默认位置");
        return;
    };
    let margin = MARGIN * wa.scale;
    let w = WIDTH * wa.scale;
    let h = HEIGHT * wa.scale;
    let x = wa.x + wa.w - w - margin;
    let y = wa.y + margin;
    tracing::info!(
        target: "llm_switch::float",
        work_area = format!("{}x{}@{},{}", wa.w, wa.h, wa.x, wa.y),
        scale = wa.scale,
        pos = format!("{},{}", x, y),
        "浮窗落位（主屏右上角）"
    );
    guard_place(win, x, y, w, h);
}

/// 贴边判定里的「接缝」位掩码：多屏拼接时，选中屏的某条边若正贴着**另一块屏
/// 的外框**，这条边就不是桌面外周、只是屏与屏之间的缝。从缝上过不该触发任何
/// 贴边动作——窗口刚跨过缝时接收屏的近侧边必然在阈值内，不压掉就会在桌面
/// 中间凭空缩钮/贴合（实测报告的「拖到两个屏幕相接处会有问题」）。
const SEAM_TOP: u8 = 1;
const SEAM_BOTTOM: u8 = 2;
const SEAM_LEFT: u8 = 4;
const SEAM_RIGHT: u8 = 8;

/// 工作区（物理像素）：优先窗口所在那块屏，多屏拼接时才不会算到隔壁屏去。
#[derive(Debug, Clone, Copy)]
struct WorkArea {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    scale: f64,
    /// 四条边里哪些是屏间接缝（SEAM_* 位掩码；接缝不参与贴边判定）。
    seams: u8,
}

impl WorkArea {
    /// 窗口归哪块屏，按**重叠面积最大**判定（tokbar `screenFrame` 同款）：
    /// 多屏拼接时 `current_monitor` 的中心点判定可能落在缝隙、或被另一块屏的
    /// 坐标系吞掉——选错屏 = 拿隔壁屏的工作区当贴边基准，边距全是假的，
    /// 拖动中会凭空判出「相接」把窗口拽走。只有重叠面积不会骗人。
    fn of_window(app: &AppHandle, win: &WebviewWindow) -> Option<WorkArea> {
        let pos = win.outer_position().ok()?;
        let size = win.outer_size().ok()?;
        let (fx, fy, fw, fh) = (
            pos.x as f64,
            pos.y as f64,
            size.width as f64,
            size.height as f64,
        );
        let mut best: Option<(f64, WorkArea, Frame)> = None;
        // 全部屏的外框：接缝判定用外框而不是工作区——菜单栏/坞只缩工作区，
        // 屏与屏真正相接的是外框。
        let mut frames: Vec<Frame> = Vec::new();
        // 候选屏清单（诊断线，debug 级）：排查「吸附到错误的边」时先看拓扑。
        let mut cands = String::new();
        for mon in app.available_monitors().ok()?.into_iter() {
            let f = frame_of(&mon);
            frames.push(f);
            let Some(wa) = Self::from_monitor(Some(mon)) else {
                continue;
            };
            let ox = (fx + fw).min(wa.x + wa.w) - fx.max(wa.x);
            let oy = (fy + fh).min(wa.y + wa.h) - fy.max(wa.y);
            let area = if ox > 0.0 && oy > 0.0 { ox * oy } else { 0.0 };
            if !cands.is_empty() {
                cands.push_str(" | ");
            }
            cands.push_str(&format!(
                "wa({},{},{},{}) x{}",
                wa.x, wa.y, wa.w, wa.h, area as i64
            ));
            if area > 0.0 && best.map(|(a, _, _)| area > a).unwrap_or(true) {
                best = Some((area, wa, f));
            }
        }
        let chosen = best.map(|(_, mut wa, f)| {
            wa.seams = seam_mask(f, &frames);
            wa
        });
        tracing::debug!(
            target: "llm_switch::float",
            win_x = fx, win_y = fy, win_w = fw, win_h = fh,
            candidates = %cands,
            chosen = ?chosen.map(|w| (w.x, w.y, w.w, w.h, w.scale)),
            seams = ?chosen.map(|w| w.seams),
            "screen 归属"
        );
        chosen
            // 一块都没交叠（窗口刚建、还没落位）才退回老办法
            .or_else(|| Self::from_monitor(win.current_monitor().ok().flatten()))
            .or_else(|| Self::primary(app))
    }

    /// 刚建出来、还没落位的窗口不能信 `current_monitor`：原点 (0,0) 可能属于
    /// 另一块屏（竖屏副屏就在这儿坑过一次——浮窗建到了看不见的屏上）。
    fn primary(app: &AppHandle) -> Option<WorkArea> {
        if let Some(wa) = Self::from_monitor(app.primary_monitor().ok().flatten()) {
            return Some(wa);
        }
        let first = app.available_monitors().ok()?.into_iter().next()?;
        Self::from_monitor(Some(first))
    }

    fn from_monitor(monitor: Option<tauri::Monitor>) -> Option<WorkArea> {
        let monitor = monitor?;
        let scale = monitor.scale_factor().max(1.0);
        let pos = monitor.work_area().position;
        let size = monitor.work_area().size;
        Some(WorkArea {
            x: pos.x as f64,
            y: pos.y as f64,
            w: size.width as f64,
            h: size.height as f64,
            scale,
            // 接缝只在 of_window 里对着真实窗口算；这里是兜底路径，置 0（无接缝）。
            seams: 0,
        })
    }
}

/// 屏幕外框 (x, y, w, h)，物理像素。接缝判定要比对的是外框而不是工作区：
/// 菜单栏/坞只缩工作区，屏与屏相接的是外框。
type Frame = (f64, f64, f64, f64);

fn frame_of(mon: &tauri::Monitor) -> Frame {
    let p = mon.position();
    let s = mon.size();
    (p.x as f64, p.y as f64, s.width as f64, s.height as f64)
}

/// `own` 的四条边里哪些正贴着 `others` 中某块屏的外框（±`TOL` 物理 px，且在
/// 垂直方向有投影交叠才算相接——只碰到一个角不算）。相接 = 桌面内缝，
/// 贴边判定要绕开（SEAM_* 位掩码）。
fn seam_mask(own: Frame, others: &[Frame]) -> u8 {
    const TOL: f64 = 8.0;
    let (x, y, w, h) = own;
    let mut m = 0u8;
    for &(ox, oy, ow, oh) in others {
        // 自己跟自己永远不算接缝（own 就在列表里）。
        if (ox - x).abs() < 1.0
            && (oy - y).abs() < 1.0
            && (ow - w).abs() < 1.0
            && (oh - h).abs() < 1.0
        {
            continue;
        }
        // 投影交叠（> 0 才算；刚好碰角 = 0，不算）。
        let xov = (x + w).min(ox + ow) - x.max(ox) > 0.0;
        let yov = (y + h).min(oy + oh) - y.max(oy) > 0.0;
        if yov && (x - (ox + ow)).abs() <= TOL {
            m |= SEAM_LEFT;
        }
        if yov && ((x + w) - ox).abs() <= TOL {
            m |= SEAM_RIGHT;
        }
        if xov && (y - (oy + oh)).abs() <= TOL {
            m |= SEAM_TOP;
        }
        if xov && ((y + h) - oy).abs() <= TOL {
            m |= SEAM_BOTTOM;
        }
    }
    m
}

/// 钳到 [lo, hi]；区间退化（hi < lo，即窗口比工作区还宽）时**原样返回**。
/// 退化时取 lo 会把整块窗口甩到工作区左边缘——诊断日志抓到过的 -576→148
/// 瞬移（「拖到左边很远就吸附」）就是这么来的：放不下就别动。
fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if hi < lo {
        v
    } else {
        v.max(lo).min(hi)
    }
}

/// 移动 + 改尺寸，并且给自激上锁：我们自己放的位子不再参与贴边判定。
fn guard_place(win: &WebviewWindow, x: f64, y: f64, w: f64, h: f64) {
    SNAPPING.store(true, Ordering::SeqCst);
    let _ = win.set_size(PhysicalSize::new(w.max(1.0) as u32, h.max(1.0) as u32));
    let _ = win.set_position(PhysicalPosition::new(x as i32, y as i32));
    let handle = win.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(260)).await;
        let _ = handle;
        SNAPPING.store(false, Ordering::SeqCst);
    });
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// ease-in-out（tokbar 的 NSAnimationContext 默认曲线同款形状）。
fn ease_in_out(t: f64) -> f64 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// 贴合/缩钮用 0.18s 缓动到位（tokbar `moveTo` 同款时长与锁语义）：
/// 全程持有 SNAPPING——动画期间新到的 Moved 一律不再裁决，避免连环出手；
/// 结束时终值精确落位再放锁。直接 set_position 是瞬移，拖动中瞬移 28pt
/// 就是肉眼可见的「拽离光标」，缓动后位移被摊进 180ms，不再跳。
fn animate_place(win: &WebviewWindow, x: f64, y: f64, w: f64, h: f64) {
    let Ok(p0) = win.outer_position() else {
        return guard_place(win, x, y, w, h);
    };
    let Ok(s0) = win.outer_size() else {
        return guard_place(win, x, y, w, h);
    };
    let (x0, y0, w0, h0) = (p0.x as f64, p0.y as f64, s0.width as f64, s0.height as f64);
    SNAPPING.store(true, Ordering::SeqCst);
    let handle = win.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        const STEPS: u32 = 12;
        let step_ms = (ANIM_MS / STEPS as u64).max(1);
        for i in 1..=STEPS {
            let e = ease_in_out(i as f64 / STEPS as f64);
            if let Some(win) = handle.get_webview_window(LABEL) {
                let _ = win.set_size(PhysicalSize::new(
                    lerp(w0, w, e).max(1.0) as u32,
                    lerp(h0, h, e).max(1.0) as u32,
                ));
                let _ = win.set_position(PhysicalPosition::new(
                    lerp(x0, x, e) as i32,
                    lerp(y0, y, e) as i32,
                ));
            }
            tokio::time::sleep(Duration::from_millis(step_ms)).await;
        }
        if let Some(win) = handle.get_webview_window(LABEL) {
            let _ = win.set_size(PhysicalSize::new(w.max(1.0) as u32, h.max(1.0) as u32));
            let _ = win.set_position(PhysicalPosition::new(x as i32, y as i32));
        }
        SNAPPING.store(false, Ordering::SeqCst);
    });
}

/// 某个窗口所在屏的工作区（物理像素 x/y/w/h + 缩放），速率线吸附用。
/// 多屏时必须按窗口所在那块屏算，否则副屏上的线会吸到主屏边上去。
pub fn work_area_of(win: &WebviewWindow) -> Option<(f64, f64, f64, f64, f64)> {
    let wa = WorkArea::of_window(win.app_handle(), win)?;
    Some((wa.x, wa.y, wa.w, wa.h, wa.scale))
}

/// 主屏工作区（物理像素 x/y/w/h + 缩放），速率线首次落位用。
pub fn primary_work_area(app: &AppHandle) -> Option<(f64, f64, f64, f64, f64)> {
    let wa = WorkArea::primary(app)?;
    Some((wa.x, wa.y, wa.w, wa.h, wa.scale))
}

fn current_geom(app: &AppHandle) -> Option<(WorkArea, f64, f64, f64, f64)> {
    let win = app.get_webview_window(LABEL)?;
    let wa = WorkArea::of_window(app, &win)?;
    let pos = win.outer_position().ok()?;
    let size = win.inner_size().ok()?;
    Some((
        wa,
        pos.x as f64,
        pos.y as f64,
        size.width as f64,
        size.height as f64,
    ))
}

fn shape_body(s: &Shape) -> serde_json::Value {
    serde_json::json!({
        "expanded": s.expanded,
        "edge": s.edge,
        "width": s.width,
        "height": s.height,
        "show_all": s.show_all,
    })
}

fn publish(app: &AppHandle) {
    let s = *shape().lock().unwrap();
    let _ = app.emit("float://shape", shape_body(&s));
}

pub fn shape_json() -> serde_json::Value {
    let s = *shape().lock().unwrap();
    shape_body(&s)
}

/// 双击（tokbar `handleDoubleClick` 同款语义）：
/// · 展开态 → 原地切换「多行 ⇄ 前 DEFAULT_ROWS 行」（`show_all` 翻转），
///   保持面板形态与位置；行数变化由前端量完内容后 `fit` 回来（直接落位，
///   tokbar 的 `relayoutForRows` 同样不带动画）。
/// · 圆钮态 → 展开成面板，**恢复折叠前的行数模式**（tokbar：multiRow 不动，
///   只翻 expanded）。
/// 收成圆钮只能靠拖到左右贴边（`judge_edge`），双击不再承担收起——两件事
/// 分开，手上不歧义。
pub fn double_click(app: &AppHandle) -> serde_json::Value {
    let expanded_now = shape().lock().unwrap().expanded;
    if expanded_now {
        toggle_show(app)
    } else {
        // show_all 由 set_shape 原样保留，不需要回传旧值。
        set_shape(app, true, 0, false);
        shape_json()
    }
}

/// 展开态内切换「显示全部 / 前 DEFAULT_ROWS 行」，位置与尺寸不变（高度由前端
/// 量完新行数后 `fit` 自动收）。圆钮态不切换行数，直接返回当前形状。
pub fn toggle_show(app: &AppHandle) -> serde_json::Value {
    let mut s = shape().lock().unwrap();
    if !s.expanded {
        return shape_json();
    }
    s.show_all = !s.show_all;
    drop(s);
    publish(app);
    shape_json()
}

/// 切换形态（前端与内部都用它，避免出现第二种改形状的写法）。
/// `show_all` 不在这里改——折叠/展开只换形态，行数模式原样保留
/// （tokbar 的 `multiRow` 跨折叠不清零）。`animated`：贴边裁决来的缩钮走
/// 0.18s 缓动（tokbar `layoutButton`→`moveTo` 同款），双击展开直接落位。
pub fn set_shape(app: &AppHandle, expanded: bool, edge: i8, animated: bool) {
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let Some((wa, x, y, w, h)) = current_geom(app) else {
        return;
    };
    let scale = wa.scale;
    let (nw, nh) = {
        let mut s = shape().lock().unwrap();
        s.expanded = expanded;
        s.edge = if expanded { 0 } else { edge };
        if expanded {
            // 同样受工作区封顶：恢复展开时若把窗口放到比屏还宽，左右贴边裁决
            // 会被 fits 判死，直到下一次 fit 才自愈——不给它开局。
            ((s.width * scale).min(wa.w * 0.95), s.height * scale)
        } else {
            (BUTTON * scale, BUTTON * scale)
        }
    };
    let nx = if !expanded {
        match edge {
            -1 => wa.x,
            1 => wa.x + wa.w - nw,
            _ => clamp(x + (w - nw) / 2.0, wa.x, wa.x + wa.w - nw),
        }
    } else {
        // 展开时以圆钮中心为锚点摊开，视觉上不会跳走
        clamp(x + (w - nw) / 2.0, wa.x, wa.x + wa.w - nw)
    };
    let ny = clamp(y + (h - nh) / 2.0, wa.y, wa.y + wa.h - nh);
    tracing::debug!(
        target: "llm_switch::float",
        expanded, edge, animated,
        from = format!("{x},{y},{w},{h}"),
        to = format!("{nx},{ny},{nw},{nh}"),
        "set_shape"
    );
    if animated {
        animate_place(&win, nx, ny, nw, nh);
    } else {
        guard_place(&win, nx, ny, nw, nh);
    }
    publish(app);
}

/// 前端量完内容后回报尺寸（逻辑像素），窗口贴着内容收紧，居中不跳位。
/// 拖动进行中先记账、静默后落（见 `fit_or_defer`）。
pub fn fit(app: &AppHandle, width: f64, height: f64) {
    fit_or_defer(app, width, height, false);
}

/// 缩放时随之改窗口尺寸（前端改了 CSS zoom 后调用），以中心为锚点。
pub fn zoom_resize(app: &AppHandle, width: f64, height: f64) {
    fit_or_defer(app, width, height, true);
}

/// fit/zoom 的统一入口：**用户还在动窗口时绝不碰位置**。
///
/// 原生拖拽是增量位移（系统按 delta 挪窗），拖动中任何一次 set_position
/// 都会在拖拽里留下永久偏移——窗口从此「远离光标」，而且每次干涉都累积。
/// 所以拖动中来的 fit 一律记最新一笔，静默 SETTLE_MS 后只落最后一次。
fn fit_or_defer(app: &AppHandle, width: f64, height: f64, center: bool) {
    let mut p = pending_fit().lock().unwrap();
    if !moving_recently() {
        p.take(); // 旧记账作废：直接落最新的这次测量
        drop(p);
        tracing::debug!(
            target: "llm_switch::float",
            width, height, center, "fit 直接落位"
        );
        return apply_fit(app, width, height, center);
    }
    *p = Some((width, height, center));
    drop(p);
    tracing::debug!(target: "llm_switch::float", width, height, center, "fit 拖动中挂起");
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if moving_recently() {
                continue;
            }
            // 持锁落位，和直接落位的路径互斥，保证「后到的测量后落」。
            let mut p = pending_fit().lock().unwrap();
            if let Some((w, h, c)) = p.take() {
                tracing::debug!(
                    target: "llm_switch::float",
                    width = w, height = h, center = c, "fit 静默后落位"
                );
                apply_fit(&handle, w, h, c);
            }
            return;
        }
    });
}

/// 落一次 fit/zoom：`center`=true 以中心为锚（缩放），否则原地收紧（贴内容）。
fn apply_fit(app: &AppHandle, width: f64, height: f64, center: bool) {
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let Some((wa, x, y, w, h)) = current_geom(app) else {
        return;
    };
    let scale = wa.scale;
    // 多行态别撑满整屏（tokbar `expandedHeight` 同款上限：屏幕工作区的 70%），
    // 行再多、zoom 再大，窗口也不许长到用不了。
    let max_logical = (wa.h * 0.7) / scale;
    // 宽度同样封顶：窗口比工作区还宽会把左右贴边裁决判死（fits 恒 false，
    // 表现为「贴边不缩钮」），是前端量出多大都必须在这里兜住的硬底线。
    let max_w_logical = (wa.w * 0.95) / scale;
    if center {
        let (nw, nh) = (
            width.max(60.0).min(max_w_logical) * scale,
            height.max(40.0).min(max_logical) * scale,
        );
        {
            let mut s = shape().lock().unwrap();
            if !s.expanded {
                return; // 圆钮态尺寸由 BUTTON 说了算
            }
            s.width = width.max(60.0);
            s.height = height.max(40.0).min(max_logical);
        }
        let nx = clamp(x + (w - nw) / 2.0, wa.x, wa.x + wa.w - nw);
        let ny = clamp(y + (h - nh) / 2.0, wa.y, wa.y + wa.h - nh);
        guard_place(&win, nx, ny, nw, nh);
    } else {
        let (nw, nh) = (
            width.max(120.0).min(max_w_logical) * scale,
            height.max(48.0).min(max_logical) * scale,
        );
        {
            let mut s = shape().lock().unwrap();
            if !s.expanded {
                return; // 圆钮态尺寸由 BUTTON 说了算
            }
            s.width = width.max(120.0);
            s.height = height.max(48.0).min(max_logical);
        }
        // 原地收紧：x/y 不动（越界才钳），所以没有“跳位”的观感。
        let nx = clamp(x, wa.x, wa.x + wa.w - nw);
        let ny = clamp(y, wa.y, wa.y + wa.h - nh);
        guard_place(&win, nx, ny, nw, nh);
    }
}

/// 贴边裁决结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    None,
}

/// 只认「离得最近的那条边」，并列时上下优先（tokbar 的 windowDidMove 同款裁决）。
///
/// 为什么值得单独拎出来当纯函数：角落同时贴顶又贴左时，如果先判左右就会平白
/// 缩成圆钮；面板比屏还宽时左右贴边毫无意义（贴哪边都溢出）。这两种情况
/// 光看代码想不清楚，必须有测试钉住。
pub fn rank_edges(
    d_top: f64,
    d_bottom: f64,
    d_left: f64,
    d_right: f64,
    fits_width: bool,
    threshold: f64,
) -> Edge {
    let inf = f64::MAX;
    let ranked = [
        (d_top, Edge::Top),
        (d_bottom, Edge::Bottom),
        (if fits_width { d_left } else { inf }, Edge::Left),
        (if fits_width { d_right } else { inf }, Edge::Right),
    ];
    let mut best = ranked[0];
    for cand in ranked.iter().skip(1) {
        if cand.0 < best.0 || (cand.0 == best.0 && (cand.1 as u8) < (best.1 as u8)) {
            best = *cand;
        }
    }
    if best.0 > threshold {
        Edge::None
    } else {
        best.1
    }
}

/// 拖动中**每来一帧 Moved 就立刻判定**贴边（tokbar `windowDidMove` 同款时序）：
/// 「随时检测面板的边是否与屏幕边相接」，相接（≤ EDGE_THRESHOLD）才出手。
///
/// 判定没有计时器、没有猜测：没有 Moved 就什么都不做；当前这一帧真判出
/// 「相接」才缩圆钮 / 贴合，其余帧一律不碰窗口。
///
/// 踩过（保留教训）：从前是「停下来 280ms 再判」，可按住不放慢慢拖时 macOS
/// 会在两帧 Moved 之间留出几百毫秒空档，计时器误以为已松手，当场
/// set_position / 缩圆钮——窗口被我们自己拽离光标（「拖动不跟鼠标」），
/// 半路犹豫一下它就自己动了（「直接就跑了」）。所以计时器整个删掉：
/// 判定只由真实移动事件触发，且只在相接时才碰窗口。
pub fn on_moved(app: &AppHandle) {
    if SNAPPING.load(Ordering::SeqCst) {
        return;
    }
    // 真实移动 = 用户在拖：记下时刻，fit/zoom 在此后的 SETTLE_MS 内只记账不落位。
    LAST_MOVE.store(now_ms(), Ordering::SeqCst);
    // 不在窗口事件回调里直接改窗口（tao 对事件处理中的 set_position 有限制），
    // 丢到异步运行时上执行；无睡眠，延迟只有微秒级，仍是「随时」判定。
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        judge_edge(&handle);
    });
}

/// 相接判定本体（拖动中每帧跑一次）：只认离得最近的那条边，相接才动作。
fn judge_edge(app: &AppHandle) {
    if SNAPPING.load(Ordering::SeqCst) {
        return;
    }
    let Some(win) = app.get_webview_window(LABEL) else {
        return;
    };
    let Some((wa, x, y, w, h)) = current_geom(app) else {
        return;
    };
    let th = EDGE_THRESHOLD * wa.scale;
    // 接缝不是边：多屏拼接处，选中屏朝向隔壁屏的那条边只是屏间内缝。窗口刚
    // 跨过缝时接收屏的近侧边必然在阈值内——不压掉就会在桌面中间凭空缩钮/
    // 贴合。压成无穷大，rank_edges 自然不会选它。
    let d_top = if wa.seams & SEAM_TOP != 0 {
        f64::INFINITY
    } else {
        (y - wa.y).max(0.0)
    };
    let d_bottom = if wa.seams & SEAM_BOTTOM != 0 {
        f64::INFINITY
    } else {
        (wa.y + wa.h - (y + h)).max(0.0)
    };
    let d_left = if wa.seams & SEAM_LEFT != 0 {
        f64::INFINITY
    } else {
        (x - wa.x).max(0.0)
    };
    let d_right = if wa.seams & SEAM_RIGHT != 0 {
        f64::INFINITY
    } else {
        (wa.x + wa.w - (x + w)).max(0.0)
    };
    let fits = w <= wa.w;
    let edge = rank_edges(d_top, d_bottom, d_left, d_right, fits, th);
    // 诊断线（默认 info 级不输出；排查拖拽问题时用 RUST_LOG=llm_switch::float=debug 打开）：
    // 每帧的窗口位置/尺寸、所选工作区、四边距离与阈值、裁决结果——
    // 「很远就吸附」这类问题必须拿真实数字说话，猜不出来。
    tracing::debug!(
        target: "llm_switch::float",
        x, y, w, h,
        wa_x = wa.x, wa_y = wa.y, wa_w = wa.w, wa_h = wa.h, wa_scale = wa.scale,
        seams = wa.seams,
        d_top, d_bottom, d_left, d_right, th, fits,
        edge = ?edge,
        "judge_edge"
    );
    match edge {
        Edge::Left | Edge::Right => {
            // 缩圆钮逐帧断言、直接落位（不走动画）：贴边拖动时系统拖拽每帧都在
            // 挪窗，动画/「已缩过就不再出手」都会让这一帧的缩钮被拖拽覆盖后
            // 再也没有下一帧来补——表现为「不缩钮了」。tokbar 也是每帧重放 layoutButton。
            let side: i8 = if edge == Edge::Left { -1 } else { 1 };
            set_shape(app, false, side, false);
        }
        Edge::Top | Edge::Bottom => {
            // 上下贴边：保持形态，只是贴合过去。0.5pt 内视为已贴合直接跳过
            // （tokbar 同款 no-op 判定）——否则贴住之后每一帧都重新上锁动画。
            let ny = if edge == Edge::Top {
                wa.y
            } else {
                wa.y + wa.h - h
            };
            let nx = clamp(x, wa.x, wa.x + wa.w - w);
            if (nx - x).abs() > 0.5 || (ny - y).abs() > 0.5 {
                animate_place(&win, nx, ny, w, h);
            }
        }
        Edge::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::rank_edges;

    #[test]
    fn far_from_every_edge_is_free() {
        assert_eq!(
            rank_edges(300.0, 300.0, 300.0, 300.0, true, 28.0),
            super::Edge::None
        );
    }

    #[test]
    fn nearest_side_wins() {
        assert_eq!(
            rank_edges(200.0, 200.0, 200.0, 10.0, true, 28.0),
            super::Edge::Right
        );
        assert_eq!(
            rank_edges(200.0, 200.0, 6.0, 200.0, true, 28.0),
            super::Edge::Left
        );
    }

    #[test]
    fn corner_prefers_vertical_over_collapsing() {
        // 角落：既贴顶又贴左。应当贴合顶部，而不是平白缩成圆钮。
        assert_eq!(
            rank_edges(0.0, 900.0, 0.0, 900.0, true, 28.0),
            super::Edge::Top
        );
        assert_eq!(
            rank_edges(900.0, 0.0, 0.0, 900.0, true, 28.0),
            super::Edge::Bottom
        );
    }

    #[test]
    fn oversized_panel_never_snaps_sideways() {
        // 面板比屏宽：左右贴边贴哪边都溢出，只认上下。
        assert_eq!(
            rank_edges(400.0, 400.0, 0.0, 0.0, false, 28.0),
            super::Edge::None
        );
        assert_eq!(
            rank_edges(400.0, 12.0, 0.0, 0.0, false, 28.0),
            super::Edge::Bottom
        );
    }

    #[test]
    fn negative_distances_are_clamped_before_ranking() {
        // 溢出屏幕时距离会是负数，钳到 0 才不会骗过排序（调用侧已 max(0.0)，这里复核语义）
        assert_eq!(rank_edges(0.0, 0.0, 0.0, 0.0, true, 28.0), super::Edge::Top);
    }

    #[test]
    fn degenerate_clamp_keeps_position() {
        // 窗口比工作区还宽时区间退化（hi < lo）：必须保持原位，不许钳到 lo。
        // 钳到 lo 会把整块窗口甩到左边缘——诊断日志抓到的 -576→148 瞬移
        // （「拖到左边很远就吸附」）就是这条退化路径，此测试是它的回归锁。
        assert_eq!(super::clamp(-576.0, 146.0, -274.0), -576.0);
        // 正常区间行为不变：越界才钳，界内不动。
        assert_eq!(super::clamp(100.0, 146.0, 3000.0), 146.0);
        assert_eq!(super::clamp(4000.0, 146.0, 3000.0), 3000.0);
        assert_eq!(super::clamp(1000.0, 146.0, 3000.0), 1000.0);
    }

    #[test]
    fn seam_mask_marks_only_real_adjacent_sides() {
        use super::{seam_mask, SEAM_BOTTOM, SEAM_LEFT, SEAM_RIGHT, SEAM_TOP};
        // 主屏 + 右侧竖屏（实机拓扑同款）。
        let main = (0.0, 0.0, 3840.0, 2160.0);
        let portrait = (3840.0, 0.0, 2160.0, 3780.0);
        assert_eq!(seam_mask(main, &[main, portrait]), SEAM_RIGHT);
        assert_eq!(seam_mask(portrait, &[main, portrait]), SEAM_LEFT);
        // 上下叠放：下屏顶边、上屏底边互为缝。
        let upper = (0.0, -2160.0, 3840.0, 2160.0);
        assert_eq!(seam_mask(main, &[upper]), SEAM_TOP);
        assert_eq!(seam_mask(upper, &[main]), SEAM_BOTTOM);
        // 孤立屏：无缝。
        assert_eq!(seam_mask((10000.0, 0.0, 2000.0, 2000.0), &[main]), 0);
        // 只碰一个角（对角排列，投影交叠为 0）：不算接缝。
        assert_eq!(seam_mask((3840.0, 2160.0, 2000.0, 2000.0), &[main]), 0);
    }

    #[test]
    fn suppressed_edges_never_win_ranking() {
        // 接缝边在 judge 里被压成 INFINITY：全体被压时是 None（不是 Top），
        // 只剩一条真边时归它。
        let inf = f64::INFINITY;
        assert_eq!(
            rank_edges(inf, inf, inf, inf, true, 28.0),
            super::Edge::None
        );
        assert_eq!(
            rank_edges(inf, inf, inf, 20.0, true, 28.0),
            super::Edge::Right
        );
    }

    #[test]
    fn ease_in_out_endpoints_and_monotonic() {
        use super::{ease_in_out, lerp};
        // 缓动必须端点归零/归一且单调——否则动画会先快后倒退，肉眼看出“抽搐”。
        assert!((ease_in_out(0.0)).abs() < 1e-9);
        assert!((ease_in_out(1.0) - 1.0).abs() < 1e-9);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-9);
        let mut prev = 0.0;
        for i in 1..=100 {
            let e = ease_in_out(i as f64 / 100.0);
            assert!(e >= prev, "ease_in_out 不单调：{e} < {prev}");
            prev = e;
        }
        // lerp 端点：t=0 起点、t=1 终点（终值精确落位靠它）。
        assert_eq!(lerp(10.0, 40.0, 0.0), 10.0);
        assert_eq!(lerp(10.0, 40.0, 1.0), 40.0);
        assert_eq!(lerp(10.0, 40.0, 0.5), 25.0);
    }
}
