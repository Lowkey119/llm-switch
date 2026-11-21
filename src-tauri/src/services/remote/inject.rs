//! 注入与原生事件抑制（计划 S4：CGEvent 注入 + 180 ms 抑制 + 合成事件标记）。
//!
//! 主线程铁律（promptfloat 的实锤教训）：`CGEventPost` 在非主线程会撞 libdispatch
//! 断言直接 SIGTRAP 带走进程，所以本模块所有注入**必须在主线程调用**——调用方
//! （hid.rs）统一经 `AppHandle::run_on_main_thread` 投递，这里不再自检。
//!
//! 合成事件标记：注入事件统一带 `EVENT_SOURCE_USER_DATA = EVENT_MARKER`，
//! 抑制器见标记即放行——否则自己注入的事件会被自己的账本吞掉（旧仓同款坑）。

use crate::services::remote::actions::Shortcut;
use crate::services::remote::buttons::{native_events, NativeEvent, RemoteButton};
use core_foundation::base::TCFType;
use core_foundation::runloop::CFRunLoop;
use core_foundation::string::CFString;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventType, CallbackResult, EventField,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use enigo::{Direction, Enigo, Key, Keyboard as _};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// 合成事件指纹（"LLSW"）。注入与抑制两侧共用，别和别的工具撞值。
pub const EVENT_MARKER: i64 = 0x4C4C_5357;

/// 录制闸门（用户 2026-11-20 拍板）：true = 槽位正在录快捷键，tap 拦下所有键盘输入
/// 转交前端录制器，一律不透传系统——按下的组合与遥控按键都不许触发其他响应。
/// Esc 也照录（用户 2026-11-20：Esc 要能映射，返回键常用映射就是它）；
/// 取消录制 = 再点一次该槽 / 点标题退出设置态 / 点别处失焦。
pub static RECORDING: AtomicBool = AtomicBool::new(false);

// 进程级 tap 的唯一句柄（tap 专用线程的 thread_local，避开 unsafe_code=forbid 的 Sync 墙）：
// macOS 会在回调超时或撞上安全输入时把 tap 静默禁用，回调里拿它原地重新 enable，
// 否则拦截从此哑掉。
thread_local! {
    static TAP: std::cell::RefCell<Option<CGEventTap<'static>>> =
        const { std::cell::RefCell::new(None) };
}

/// CGEventFlags → Shortcut mods 位图（bit0 ctrl · bit1 shift · bit2 alt · bit3 meta，
/// 与 actions::Shortcut / 前端 shortcut.ts 同一约定）。
pub fn flags_to_mods(flags: CGEventFlags) -> u8 {
    let mut mods = 0u8;
    if flags.contains(CGEventFlags::CGEventFlagControl) {
        mods |= 1;
    }
    if flags.contains(CGEventFlags::CGEventFlagShift) {
        mods |= 2;
    }
    if flags.contains(CGEventFlags::CGEventFlagAlternate) {
        mods |= 4;
    }
    if flags.contains(CGEventFlags::CGEventFlagCommand) {
        mods |= 8;
    }
    mods
}

/// Shortcut mods 位图 → CGEventFlags（flags_to_mods 的逆向，同一约定）。
pub fn mods_to_flags(mods: u8) -> CGEventFlags {
    let mut f = CGEventFlags::empty();
    if mods & 1 != 0 {
        f.insert(CGEventFlags::CGEventFlagControl);
    }
    if mods & 2 != 0 {
        f.insert(CGEventFlags::CGEventFlagShift);
    }
    if mods & 4 != 0 {
        f.insert(CGEventFlags::CGEventFlagAlternate);
    }
    if mods & 8 != 0 {
        f.insert(CGEventFlags::CGEventFlagCommand);
    }
    f
}

/// 注入快捷键组合（S-C，计划 §四）：带 flags 的 key down/up 各一，裸单键 = mods 0。
/// 事件同样打 EVENT_MARKER——抑制账本不会吞自己注入的组合。
/// **必须在主线程调用**（同 inject_native 铁律）。
pub fn inject_shortcut(sc: Shortcut) -> Result<(), String> {
    // 媒体键特判（用户 2026-11-21 拍板媒体控制预设）：kVK 72/73/74（Vol+/Vol-/Mute，
    // 与前端 KVK_LABEL 同源）走普通键盘事件不触发媒体功能，转 enigo systemDefined 通道。
    let media = match sc.key {
        72 => Some(0),
        73 => Some(1),
        74 => Some(2),
        _ => None,
    };
    if let Some(kind) = media {
        return inject_native(NativeEvent::SystemKey(kind));
    }
    let flags = mods_to_flags(sc.mods);
    for down in [true, false] {
        let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
            .map_err(|_| "事件源创建失败".to_string())?;
        let event = CGEvent::new_keyboard_event(src, sc.key, down)
            .map_err(|_| "键盘事件创建失败".to_string())?;
        event.set_flags(flags);
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, EVENT_MARKER);
        event.post(CGEventTapLocation::Session);
    }
    Ok(())
}

/// 注入一段文本到当前焦点输入框（语音定稿上屏，计划 §六.4）。
///
/// 走 enigo 的 fast_text（macOS 即 `CGEventKeyboardSetUnicodeString`）：等价输入法上屏，
/// 任何输入法下都工作、不碰剪贴板、不改用户输入法状态。**必须在主线程调用**（同 inject_native）。
pub fn inject_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let mut enigo = Enigo::new(&enigo::Settings::default()).map_err(|e| e.to_string())?;
    enigo
        .text(text)
        .map_err(|e| format!("文本注入失败：{e}（辅助功能未授权？）"))
}

/// 抑制窗口：登记后 500 ms 内匹配的原生事件丢弃。
/// 取 SayAll（remote-mic-app）实测值：原生 HID 投递可滞后报告 300 ms 以上、
/// 甚至晚于松键（其 KeyboardEventSuppressor.nativeDeliveryWindow）。旧值 180 ms
/// 会让迟到的菜单键 110 在账本过期后弹出上下文菜单——人家交过学费，直接抄。
pub const SUPPRESS_WINDOW: Duration = Duration::from_millis(500);

/// 注入一个原生等价事件。**必须在主线程调用**（见模块头铁律）。
pub fn inject_native(ev: NativeEvent) -> Result<(), String> {
    match ev {
        NativeEvent::None => Ok(()),
        NativeEvent::Keyboard(keycode) => {
            for down in [true, false] {
                let src = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
                    .map_err(|_| "事件源创建失败".to_string())?;
                let event = CGEvent::new_keyboard_event(src, keycode, down)
                    .map_err(|_| "键盘事件创建失败".to_string())?;
                event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, EVENT_MARKER);
                event.post(CGEventTapLocation::Session);
            }
            Ok(())
        }
        NativeEvent::SystemKey(kind) => {
            // 媒体键走 enigo 的 systemDefined 通道（enigo 自己会写 EVENT_SOURCE_USER_DATA）。
            let key = match kind {
                0 => Key::VolumeUp,
                1 => Key::VolumeDown,
                2 => Key::VolumeMute,
                _ => return Err(format!("暂不支持的媒体键类型 {kind}")),
            };
            let mut enigo = Enigo::new(&enigo::Settings {
                event_source_user_data: Some(EVENT_MARKER),
                // 注入路径永远不许弹系统授权框（promptfloat 同款口径）。
                open_prompt_to_get_permissions: false,
                ..Default::default()
            })
            .map_err(|e| format!("辅助功能不可用：{e}"))?;
            enigo
                .key(key, Direction::Click)
                .map_err(|e| format!("媒体键注入失败：{e}"))
        }
    }
}

/// 抑制账本：登记「该吞的原生键盘事件」。
///
/// 共享模式下把键设为 `disabled`（或 S7 起的自定义动作）时，原生事件仍会到系统，
/// 由 tap 按账本丢弃。每个键码登记两条：按下与配对的抬起各消一条（旧仓口径：
/// 已抑制 down 的配对 up 单独记账）。
///
/// 媒体键（systemDefined）core-graphics 0.25 的 tap 订阅不了（事件枚举没有该类型），
/// 共享模式禁用媒体键无法抑制——那是独占模式的场景，调用方负责把限制显示出去。
/// 按住态上限：菜单/电源长按时 repeat 全靠它续命，up 丢失时靠超时自愈。
/// 没人按住菜单键超过 3 秒（手势判定 500ms 就触发），取 3s 宽裕。
const HELD_CAP: Duration = Duration::from_secs(3);

#[derive(Default)]
pub struct SuppressLedger {
    armed: Vec<(u16, Instant)>,
    /// 已吞 down、配对 up 未到的键码：长按时系统以 ~12Hz 自动重复 KeyDown，
    /// 全部要吞到 up 到达（SayAll heldEventCounts 的教训——真机实锤：repeat
    /// 逐条漏过窗口就逐条弹一次上下文菜单）。
    held: Vec<(u16, Instant)>,
}

impl SuppressLedger {
    /// 登记某键的原生事件（按下瞬间调用，覆盖随后窗口内到达的原生事件）。
    pub fn arm_button(&mut self, button: RemoteButton, now: Instant) {
        for ev in native_events(button) {
            if let NativeEvent::Keyboard(keycode) = ev {
                self.armed.push((keycode, now));
                self.armed.push((keycode, now));
            }
        }
    }

    /// 判定一个原生键盘事件是否吞掉。
    ///
    /// - 按住态命中：up 吞掉并闭合配对；非 autorepeat 的 down = 实体键盘新按，
    ///   放行并清账（SayAll `new_keyboard_press` 口径）；autorepeat down 吞掉。
    /// - 账本命中：吞掉；down 同时开按住态。
    pub fn swallow(&mut self, keycode: u16, down: bool, autorepeat: bool, now: Instant) -> bool {
        self.armed
            .retain(|(_, t)| now.duration_since(*t) < SUPPRESS_WINDOW);
        self.held.retain(|(_, t)| now.duration_since(*t) < HELD_CAP);
        if let Some(i) = self.held.iter().position(|(c, _)| *c == keycode) {
            if !down {
                self.held.remove(i);
                // 配对 up 走按住态闭合时，armed 里那条备用记录同步消掉，
                // 否则窗口内下一次同码实体按下会被它多吞一次。
                if let Some(j) = self.armed.iter().position(|(c, _)| *c == keycode) {
                    self.armed.remove(j);
                }
                return true;
            }
            if !autorepeat {
                self.held.remove(i);
                return false;
            }
            return true;
        }
        if let Some(i) = self.armed.iter().position(|(c, _)| *c == keycode) {
            self.armed.remove(i);
            if down {
                self.held.push((keycode, now));
            }
            return true;
        }
        false
    }
}

/// 启动抑制 tap：在**专用线程**上创建、挂 runloop 并驻留运行。
///
/// 为什么不挂主线程（remote-mic-app 用真事故换来的铁律，Bugs/2026-10-24）：
/// active tap 的回调没返回时，全系统输入都排队等它；回调挂在主 runloop，
/// 主线程只要卡一下（同步查询、磁盘 IO），所有 app 都收不到输入，整机像死机。
///
/// 生命周期：进程级一次性启动，之后不再拆（账本空时全放行，成本可忽略）。
/// 创建失败 = 辅助功能未授权，错误经通道回报调用方显示；不许静默。
pub fn start_suppress_tap(
    ledger: Arc<Mutex<SuppressLedger>>,
    app: AppHandle,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("remote-tap".to_string())
        .spawn(move || {
            let mounted = mount_tap(ledger, app);
            // 挂载结果先回报再驻留：调用方（HID 线程/命令线程）拿得到成败。
            let _ = tx.send(mounted);
            CFRunLoop::run_current();
        })
        .map_err(|e| format!("tap 线程起不来：{e}"))?;
    rx.recv().map_err(|e| format!("tap 线程未回报：{e}"))?
}

/// 在 tap 专用线程上建 tap、挂进本线程 runloop 的公共模式、enable。
/// tap 与 source 都与进程同寿（Drop 会拆回调/摘 source），句柄存 TAP 供回调复活。
fn mount_tap(ledger: Arc<Mutex<SuppressLedger>>, app: AppHandle) -> Result<(), String> {
    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::FlagsChanged,
        ],
        move |_proxy, etype, event| {
            // 系统禁用（回调超时 / 安全输入插队）后原地复活，否则拦截静默失效。
            if matches!(
                etype,
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
            ) {
                TAP.with(|t| {
                    if let Some(tap) = t.borrow().as_ref() {
                        tap.enable();
                    }
                });
                return CallbackResult::Keep;
            }
            if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == EVENT_MARKER {
                return CallbackResult::Keep;
            }
            let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
            // 录制态（用户 2026-11-20 拍板「录制期间所有键盘事件都要拦截」）：
            // 全部键盘输入——含修饰键 flagsChanged、Esc 与遥控按键——都归录制器，
            // 不透传系统。KeyDown 转成 {mods,key} 发前端提交（Esc 照录，取消走失焦）。
            if RECORDING.load(Ordering::Relaxed) {
                if matches!(etype, CGEventType::KeyDown) {
                    let mods = flags_to_mods(event.get_flags());
                    let _ = app.emit(
                        "remote://captured",
                        serde_json::json!({ "mods": mods, "key": keycode }),
                    );
                }
                return CallbackResult::Drop;
            }
            if !matches!(etype, CGEventType::KeyDown | CGEventType::KeyUp) {
                return CallbackResult::Keep;
            }
            let hit = ledger.lock().expect("suppress ledger poisoned").swallow(
                keycode,
                matches!(etype, CGEventType::KeyDown),
                event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0,
                Instant::now(),
            );
            // 观测（2026-11-20 菜单键排查）：110/90 是账本仅有的常驻客户，
            // 逐事件记判定，漏吞/没挂上一眼可分。
            if matches!(keycode, 110 | 90) {
                tracing::info!(target: "remote::tap", keycode, ?etype, hit, "抑制 tap 事件");
            }
            if hit {
                CallbackResult::Drop
            } else {
                CallbackResult::Keep
            }
        },
    )
    .map_err(|_| "事件 tap 创建失败（辅助功能未授权）")?;
    // 病根修复（用户 2026-11-20：录制时 ctrl+1 照穿）：CGEventTapCreate 出来的 tap
    // 不挂 runloop 就永远收不到事件——旧代码只 enable + forget，回调一次都没进过。
    let loop_source = tap
        .mach_port()
        .create_runloop_source(0)
        .map_err(|_| "事件 tap 挂不上 runloop")?;
    // 病根修复第二轮（探针实锤：mount Ok 但回调从未被调用）：kCFRunLoopCommonModes
    // 是哨兵常量，CF 对它走指针同一性判断，同值新建字符串会被当成一个没人进入的
    // 普通模式，source 挂上也永远不被服务。改挂两个真实模式串（按字符串值匹配）：
    // 默认模式 + 事件跟踪模式，覆盖 runloop 实际会进入的全部场景。
    let default_mode = CFString::new("kCFRunLoopDefaultMode");
    let tracking_mode = CFString::new("NSTrackingRunLoopMode");
    let rl = CFRunLoop::get_current();
    rl.add_source(&loop_source, default_mode.as_concrete_TypeRef());
    rl.add_source(&loop_source, tracking_mode.as_concrete_TypeRef());
    // 挂上 source 才 enable（remote-mic-app 口径）：已 enable 却无人承接的 tap
    // 会把事件堆在端口里堵死系统输入。
    tap.enable();
    std::mem::forget(loop_source);
    TAP.with(|t| *t.borrow_mut() = Some(tap));
    tracing::info!(target: "remote::tap", "抑制 tap 挂载成功");
    Ok(())
}

// ---------------------------------------------------------------- 平台接口归位（S-X0）
//
// 行为与上面的自由函数逐字一致，只是多一个跨平台入口（platform::{Injector, Interceptor}），
// 让 Windows/Linux 能换实现而 hid.rs 一行不用改。

use crate::services::remote::platform::{Injector, Interceptor};

/// CGEvent 注入实现。
pub struct MacInjector;

impl Injector for MacInjector {
    fn inject_shortcut(&self, sc: Shortcut) -> Result<(), String> {
        inject_shortcut(sc)
    }
    fn inject_native(&self, ev: NativeEvent) -> Result<(), String> {
        inject_native(ev)
    }
    fn inject_text(&self, text: &str) -> Result<(), String> {
        inject_text(text)
    }
}

/// CGEventTap 抑制实现：账本 + 录制闸门 + 进程级 tap。
#[derive(Default)]
pub struct MacInterceptor {
    ledger: Arc<Mutex<SuppressLedger>>,
}

impl Interceptor for MacInterceptor {
    fn arm(&self, button: RemoteButton, now: Instant) {
        self.ledger
            .lock()
            .expect("suppress ledger poisoned")
            .arm_button(button, now);
    }
    fn set_recording(&self, active: bool) {
        RECORDING.store(active, Ordering::Relaxed);
    }
    fn recording(&self) -> bool {
        RECORDING.load(Ordering::Relaxed)
    }
    fn start(&self, app: AppHandle) -> Result<(), String> {
        start_suppress_tap(self.ledger.clone(), app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_map_to_shortcut_mods_bits() {
        assert_eq!(flags_to_mods(CGEventFlags::empty()), 0);
        let f = CGEventFlags::CGEventFlagControl | CGEventFlags::CGEventFlagCommand;
        assert_eq!(flags_to_mods(f), 1 | 8);
        let g = CGEventFlags::CGEventFlagShift | CGEventFlags::CGEventFlagAlternate;
        assert_eq!(flags_to_mods(g), 2 | 4);
    }

    #[test]
    fn mods_to_flags_roundtrips() {
        // 16 种 mods 组合（4 个修饰键位）全部往返一致，注入与捕获两侧永不各说各话。
        for m in 0u8..16 {
            assert_eq!(flags_to_mods(mods_to_flags(m)), m);
        }
    }

    #[test]
    fn ledger_consumes_down_and_up() {
        let mut l = SuppressLedger::default();
        let t0 = Instant::now();
        l.arm_button(RemoteButton::Ok, t0);
        // 按下与抬起各消一条。
        assert!(l.swallow(36, true, false, t0 + Duration::from_millis(5)));
        assert!(l.swallow(36, false, false, t0 + Duration::from_millis(10)));
        // 第三条没有——不吞无关事件。
        assert!(!l.swallow(36, true, false, t0 + Duration::from_millis(15)));
    }

    #[test]
    fn ledger_swallows_autorepeat_until_up() {
        let mut l = SuppressLedger::default();
        let t0 = Instant::now();
        l.arm_button(RemoteButton::Menu, t0);
        assert!(l.swallow(110, true, false, t0));
        // 长按：repeat KeyDown 越过抑制窗也要吞（按住态续命到 up）。
        assert!(l.swallow(110, true, true, t0 + Duration::from_millis(600)));
        assert!(l.swallow(110, false, false, t0 + Duration::from_millis(900)));
        // up 之后账本干净：实体键盘新按下放行。
        assert!(!l.swallow(110, true, false, t0 + Duration::from_millis(950)));
    }

    #[test]
    fn ledger_window_expiry() {
        let mut l = SuppressLedger::default();
        let t0 = Instant::now();
        l.arm_button(RemoteButton::Up, t0);
        assert!(!l.swallow(
            126,
            true,
            false,
            t0 + SUPPRESS_WINDOW + Duration::from_millis(1)
        ));
    }

    #[test]
    fn ledger_tv_matches_legacy_keycode() {
        let mut l = SuppressLedger::default();
        let t0 = Instant::now();
        l.arm_button(RemoteButton::Tv, t0);
        // 真事件 10 与历史值 50 都要能吞（各 down+up 两条）。
        assert!(l.swallow(10, true, false, t0 + Duration::from_millis(1)));
        assert!(l.swallow(50, true, false, t0 + Duration::from_millis(2)));
    }

    #[test]
    fn ledger_back_key_arms_nothing() {
        let mut l = SuppressLedger::default();
        l.arm_button(RemoteButton::Back, Instant::now());
        assert!(l.armed.is_empty(), "back 无原生事件，不该登记");
    }
}
