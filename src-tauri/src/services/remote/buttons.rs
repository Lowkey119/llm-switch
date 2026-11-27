//! 12 键 usage 表、原始报告解析、边沿检测、手势判定与连发/连按闸门（纯逻辑，无 cfg、无 FFI）。
//!
//! 默认 `cargo test` 就编到并全测（计划 §十：单测只证进程内逻辑，IOHID/真机必须现场验）。
//! 行为口径对齐旧仓参考实现（clean-room，只取事实不搬代码）：
//! - usage 表与报告格式：RC001/RC003 共享键盘接口，reportID=1，payload 为小端 u16 对，0 = 空槽；
//! - 连发：首发 350 ms，方向/音量间隔 100 ms、back 50 ms，其余键不连发；
//! - 连按闸门：250 ms 内再按视为连按，逐键 `allow_rapid_press` 默认 false；
//! - 原生事件表：共享模式设备级映射生效后的系统侧事件（抑制器按它匹配），
//!   也是独占模式下我们自行注入的等价事件。

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// 小米 RC001/RC003 实物 12 键（计划 §三事实基线）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteButton {
    Power,
    Up,
    Left,
    Ok,
    Right,
    Down,
    Back,
    VolumeUp,
    Home,
    VolumeDown,
    Menu,
    Tv,
    /// 语音键（mic，键盘页 usage 0x3E）。**不参与三槽快捷键映射**——
    /// 它的动作就是语音录入本身（计划 2026-11-19 §六.5），键位表里不给它槽位。
    Mic,
}

impl RemoteButton {
    /// 全键序（页面与遍历用，固定顺序，别依赖枚举序）。
    pub const ALL: [Self; 13] = [
        Self::Power,
        Self::Up,
        Self::Left,
        Self::Ok,
        Self::Right,
        Self::Down,
        Self::Back,
        Self::VolumeUp,
        Self::Home,
        Self::VolumeDown,
        Self::Menu,
        Self::Tv,
        Self::Mic,
    ];

    /// HID usage（键盘页内小端 u16 值；音量/返回是设备塞进键盘接口的非标准值，照实记）。
    ///
    /// 单一真相在 `profile::XIAOMI_KEYS`（档案键表）——这里只是旧 API 的转发，
    /// 免得 usage 在两个文件里各写一版对不上。
    pub fn hid_usage(self) -> u16 {
        crate::services::remote::profile::XIAOMI_KEYS
            .iter()
            .find(|(_, b)| *b == self)
            .map_or(0, |(u, _)| *u)
    }

    pub fn from_usage(usage: u16) -> Option<Self> {
        crate::services::remote::profile::XIAOMI_KEYS
            .iter()
            .find(|(u, _)| *u == usage)
            .map(|(_, b)| *b)
    }

    /// 持久化 / IPC 用的稳定 id（与 serde snake_case 同名）。
    pub fn id(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::Up => "up",
            Self::Left => "left",
            Self::Ok => "ok",
            Self::Right => "right",
            Self::Down => "down",
            Self::Back => "back",
            Self::VolumeUp => "volume_up",
            Self::Home => "home",
            Self::VolumeDown => "volume_down",
            Self::Menu => "menu",
            Self::Tv => "tv",
            Self::Mic => "mic",
        }
    }

    /// `id()` 的逆向：演示与自动化脚本按稳定 id 喂键。未知 id 返回 None，调用方明报错。
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|b| b.id() == id)
    }
}

/// 原生事件形状。
///
/// 共享模式：设备级映射生效后系统侧收到的就是它，抑制器按它匹配丢弃；
/// 独占模式：系统收不到任何事件，由我们按它自行注入。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NativeEvent {
    /// CGEvent 虚拟键码（Return/arrows/home/menu/F20…）。
    Keyboard(u16),
    /// NSEvent systemDefined（subtype 8）：音量 ±/播放/静音，data1 = type << 16 | 0x0A << 8。
    SystemKey(u8),
    /// 没有原生事件：back 键（usage 0xF1）在共享接口上不出事件，原生透传 = 无动作。
    None,
}

/// 键 → 原生事件。
///
/// power 记 90（F20）：设备级映射（0x7_0x66 → 0x7_0x6F）生效后系统侧就是 F20，
/// 这正是共享模式抑制器要匹配的值；独占模式同样注 F20（无害占位，绝不注电源事件）。
/// tv 记 10（ISO §）：历史值 50 从未对上硬件（旧仓实测教训），但抑制器仍要认它，见 [`native_events`]。
pub fn native_event(button: RemoteButton) -> NativeEvent {
    match button {
        RemoteButton::Ok => NativeEvent::Keyboard(36),
        RemoteButton::Tv => NativeEvent::Keyboard(10),
        RemoteButton::Home => NativeEvent::Keyboard(115),
        RemoteButton::Right => NativeEvent::Keyboard(124),
        RemoteButton::Left => NativeEvent::Keyboard(123),
        RemoteButton::Down => NativeEvent::Keyboard(125),
        RemoteButton::Up => NativeEvent::Keyboard(126),
        RemoteButton::Menu => NativeEvent::Keyboard(110),
        RemoteButton::Power => NativeEvent::Keyboard(90),
        RemoteButton::VolumeUp => NativeEvent::SystemKey(0),
        RemoteButton::VolumeDown => NativeEvent::SystemKey(1),
        // mic 在共享模式下由设备级禁用表（powermap）中和，系统侧不出事件；
        // 独占模式也不注任何等价事件——它的语义在语音会话里，不在按键上。
        RemoteButton::Back | RemoteButton::Mic => NativeEvent::None,
    }
}

/// 抑制匹配集：tv 额外认历史值 50（当年抑制器漏配真事件就是因为它），其余键单元素。
pub fn native_events(button: RemoteButton) -> Vec<NativeEvent> {
    let mut v = vec![native_event(button)];
    if button == RemoteButton::Tv {
        v.push(NativeEvent::Keyboard(50));
    }
    v
}

/// 报告必须有的 reportID（小米共享键盘接口；新档案走 `profile.report` 各自声明）。
pub const REPORT_ID: u8 = 1;

/// 原始报告 payload（不含 reportID 前缀，即 hidapi `read()` 返回的 `buf[1..]`）→ 按下 usage 集。
///
/// 格式（小端 u16 对，0 = 空槽）对键盘页与 Consumer 页同形，**期望 reportID 由档案给**
/// （`want`）：小米是 1，G9N9N 是另一路。reportID 不对返回 None（不是本档案的报告，
/// 调用方忽略）；长度奇数返回 None（坏帧，宁缺毋滥）。
pub fn parse_usages(want: u8, report_id: u8, payload: &[u8]) -> Option<Vec<u16>> {
    if report_id != want || !payload.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(payload.len() / 2);
    for pair in payload.as_chunks::<2>().0 {
        let usage = u16::from_le_bytes(*pair);
        if usage != 0 && !out.contains(&usage) {
            out.push(usage);
        }
    }
    Some(out)
}

/// usage 集 → 已知按键集（未知 usage 静默丢弃：设备可能塞别的）。
pub fn buttons_of(usages: &[u16]) -> Vec<RemoteButton> {
    usages
        .iter()
        .filter_map(|u| RemoteButton::from_usage(*u))
        .collect()
}

/// 边沿检测：把「当前按下的键集」快照差分成 (新按下, 释放)。
#[derive(Default)]
pub struct EdgeTracker {
    active: Vec<RemoteButton>,
}

impl EdgeTracker {
    pub fn apply(&mut self, pressed: &[RemoteButton]) -> (Vec<RemoteButton>, Vec<RemoteButton>) {
        let down: Vec<_> = pressed
            .iter()
            .copied()
            .filter(|b| !self.active.contains(b))
            .collect();
        let up: Vec<_> = self
            .active
            .iter()
            .copied()
            .filter(|b| !pressed.contains(b))
            .collect();
        self.active = pressed.to_vec();
        (down, up)
    }

    /// 断连/重开时清空，避免残影按键。
    pub fn clear(&mut self) {
        self.active.clear();
    }
}

/// 连发首发延迟：按住 350 ms 后才开始重复（旧仓 HIDRemoteTiming.repeatStart）。
pub const REPEAT_START: Duration = Duration::from_millis(350);

/// 该键的连发间隔；None = 不连发。方向/音量 100 ms，back 50 ms（旧仓口径）。
pub fn repeat_interval(button: RemoteButton) -> Option<Duration> {
    use RemoteButton::*;
    match button {
        Up | Down | Left | Right | VolumeUp | VolumeDown => Some(Duration::from_millis(100)),
        Back => Some(Duration::from_millis(50)),
        _ => None,
    }
}

/// 长按连发状态机（只在独占模式有意义：系统不会替我们重复注入）。
///
/// 调用方负责按下瞬间的首发注入；本闸门只负责排后续：首发在 press + 350 ms，
/// 之后每 repeat_interval 一发。`tick` 由读线程按节拍驱动（间隔被阻塞时一次补多发不在此列，
/// 每 tick 每键至多一发，防止卡顿后事件风暴）。
#[derive(Default)]
pub struct RepeatGate {
    holding: Vec<(RemoteButton, std::time::Instant)>,
}

impl RepeatGate {
    /// 记录按下（首发由调用方注入，这里只排首发到期时间）。
    pub fn press(&mut self, button: RemoteButton, now: std::time::Instant) {
        if repeat_interval(button).is_some() && !self.holding.iter().any(|(b, _)| *b == button) {
            self.holding.push((button, now + REPEAT_START));
        }
    }

    pub fn release(&mut self, button: RemoteButton, _now: std::time::Instant) {
        self.holding.retain(|(b, _)| *b != button);
    }

    /// 上下文变化（切 App / 断连）整体取消在途连发（C4）。
    pub fn clear(&mut self) {
        self.holding.clear();
    }

    /// 此刻到期的连发键（调用方逐键注入一发）。
    pub fn tick(&mut self, now: std::time::Instant) -> Vec<RemoteButton> {
        let mut out = Vec::new();
        for (b, next_due) in self.holding.iter_mut() {
            if *next_due <= now {
                out.push(*b);
                if let Some(iv) = repeat_interval(*b) {
                    *next_due = now + iv;
                }
            }
        }
        out
    }
}

/// 连按闸门窗口：250 ms 内再按视为连按。
pub const RAPID_WINDOW: Duration = Duration::from_millis(250);

/// 逐键连按闸门：`allow_rapid_press` 默认 false 时，窗口内的再按直接丢弃。
///
/// 只管注入路径（独占模式 / 共享模式的非原生动作）；共享模式原生直通不经闸门。
/// 被丢弃的再按不刷新窗口——窗口以最后一次被放行的按下为锚，防的是误触双发。
#[derive(Default)]
pub struct RapidGate {
    last_accepted: Vec<(RemoteButton, std::time::Instant)>,
}

impl RapidGate {
    /// true = 放行本次按下；false = 连按且该键不允许，丢弃。
    pub fn allow(
        &mut self,
        button: RemoteButton,
        now: std::time::Instant,
        allow_rapid: bool,
    ) -> bool {
        if !allow_rapid {
            if let Some((_, t)) = self.last_accepted.iter().find(|(b, _)| *b == button) {
                if now.duration_since(*t) < RAPID_WINDOW {
                    return false;
                }
            }
        }
        if let Some(slot) = self.last_accepted.iter_mut().find(|(b, _)| *b == button) {
            slot.1 = now;
        } else {
            self.last_accepted.push((button, now));
        }
        true
    }

    pub fn clear(&mut self) {
        self.last_accepted.clear();
    }
}

/// 手势归类结果。serde 名与三槽（click/double/long）对齐，前端直接吃。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    Click,
    Double,
    Long,
}

impl Gesture {
    /// 与 actions::Slot serde 名一致的槽名（IPC payload 用）。
    pub fn slot_id(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Double => "double",
            Self::Long => "long",
        }
    }
}

/// 双击窗默认值：两次按下沿相隔 ≤300ms 判双击（第二次按下即判定，不等松开）。
pub const DOUBLE_WINDOW: Duration = Duration::from_millis(300);
/// 长按阈值默认值：按住到点判长按（到点即判定，不等松开）。
pub const LONG_HOLD: Duration = Duration::from_millis(550);

/// 按住模式（C4，SayAll `repeatSingle`/`repeatLong` 口径）：按住不放时怎么办。
///
/// 默认 `Once` = 现有行为（一次手势判一次，按住不重复）。另两档是给「按住连调」
/// 这类场景：音量键绑了单击槽，按住就该一路加上去，而不是只加一格。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldMode {
    /// 只判一次手势，按住不重复。
    #[default]
    Once,
    /// 按住期间反复执行「单击」槽的动作。
    RepeatSingle,
    /// 按住期间反复执行「长按」槽的动作（长按判定本身仍到点即出、不重复）。
    RepeatLong,
}

/// 键交互时序（C4）：一个档案一份，全部可配。
///
/// 默认值即计划口径（双击 300 / 长按 550 / 首发 350 / 间隔 100）。窗口是身体差异，
/// 手快的人和手慢的人不该被同一组数字判成不同手势，所以这里只给区间封顶不给自由数：
/// 越界按边界截（手滑多打一个 0 不该让手势判定变得不可理喻）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyTiming {
    pub double_ms: u32,
    pub long_ms: u32,
    pub repeat_start_ms: u32,
    pub repeat_interval_ms: u32,
}

/// 各窗口的允许区间（ms）：下限防「双击窗比人类反应还短」，上限防「长按要按半天」。
const DOUBLE_RANGE: (u32, u32) = (120, 800);
const LONG_RANGE: (u32, u32) = (300, 1_500);
const REPEAT_START_RANGE: (u32, u32) = (150, 1_000);
const REPEAT_INTERVAL_RANGE: (u32, u32) = (30, 500);

impl Default for KeyTiming {
    fn default() -> Self {
        Self {
            double_ms: DOUBLE_WINDOW.as_millis() as u32,
            long_ms: LONG_HOLD.as_millis() as u32,
            repeat_start_ms: REPEAT_START.as_millis() as u32,
            repeat_interval_ms: 100,
        }
    }
}

impl KeyTiming {
    /// 就地归一到各自区间（回自身引用便于链式用）。
    pub fn normalize(&mut self) -> &mut Self {
        self.double_ms = self.double_ms.clamp(DOUBLE_RANGE.0, DOUBLE_RANGE.1);
        self.long_ms = self.long_ms.clamp(LONG_RANGE.0, LONG_RANGE.1);
        self.repeat_start_ms = self
            .repeat_start_ms
            .clamp(REPEAT_START_RANGE.0, REPEAT_START_RANGE.1);
        self.repeat_interval_ms = self
            .repeat_interval_ms
            .clamp(REPEAT_INTERVAL_RANGE.0, REPEAT_INTERVAL_RANGE.1);
        // 长按窗必须大于双击窗，否则「按住」永远抢在「双击」前面，双击再也判不出来。
        if self.long_ms <= self.double_ms {
            self.long_ms = self.double_ms + LONG_RANGE.0;
        }
        self
    }

    pub fn double(&self) -> Duration {
        Duration::from_millis(u64::from(self.double_ms))
    }

    pub fn long(&self) -> Duration {
        Duration::from_millis(u64::from(self.long_ms))
    }

    pub fn repeat_start(&self) -> Duration {
        Duration::from_millis(u64::from(self.repeat_start_ms))
    }

    pub fn repeat_interval(&self) -> Duration {
        Duration::from_millis(u64::from(self.repeat_interval_ms))
    }
}

/// 键交互设置（C4）：时序 + 按住模式，一个档案一份，页面一处配齐。
///
/// 按住模式做全局档而非逐键档：逐键要在已经很挤的键行里再加一列控件，
/// 而「按住连调」本来就是手感受，不是某个键的个性。要按键区分，
/// 判据是「对应槽位绑没绑动作」——没绑的键不会被这条连发接管。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyInteraction {
    #[serde(default)]
    pub timing: KeyTiming,
    #[serde(default)]
    pub hold: HoldMode,
}

impl KeyInteraction {
    pub fn normalize(&mut self) -> &mut Self {
        self.timing.normalize();
        self
    }

    /// 该键此刻该不该进按住连发。`slot_bound` = 要重复的那个槽是否绑了动作；
    /// 没绑动作的槽不接管（原生键的连发归 `RepeatGate`，不该在这里多出第二路）。
    pub fn hold_of(&self, slot_bound: bool) -> Option<HoldMode> {
        match self.hold {
            HoldMode::Once => None,
            m if slot_bound => Some(m),
            _ => None,
        }
    }
}

/// 按住连发闸门（C4）：按住超过首发延迟后，每 `repeat_interval` 催一发。
///
/// 与 `RepeatGate` 分工不同：那个是独占模式下替系统重复注入**原生键**，
/// 这个是把**槽位绑定的动作**按着重复跑（绑的是宏/打开 App/命令，原生注入帮不上）。
/// 只登记 `HoldMode != Once` 且对应槽确实绑了动作的键，没绑的键不该凭空多出连发。
#[derive(Default)]
pub struct HoldRepeatGate {
    /// (键, 重复哪个槽, 下一发到期时刻)
    holding: Vec<(RemoteButton, HoldMode, Instant)>,
}

impl HoldRepeatGate {
    /// 记录按下；`mode = Once` 或不该连发的键直接忽略。
    pub fn press(
        &mut self,
        button: RemoteButton,
        mode: HoldMode,
        timing: &KeyTiming,
        now: Instant,
    ) {
        if mode == HoldMode::Once || self.holding.iter().any(|(b, _, _)| *b == button) {
            return;
        }
        self.holding
            .push((button, mode, now + timing.repeat_start()));
    }

    pub fn release(&mut self, button: RemoteButton) {
        self.holding.retain(|(b, _, _)| *b != button);
    }

    /// 此刻到期的 (键, 槽)；调用方逐条执行对应槽动作。
    pub fn tick(&mut self, now: Instant, timing: &KeyTiming) -> Vec<(RemoteButton, HoldMode)> {
        let mut out = Vec::new();
        for (b, mode, due) in self.holding.iter_mut() {
            if *due <= now {
                out.push((*b, *mode));
                *due = now + timing.repeat_interval();
            }
        }
        out
    }

    /// 上下文变化（切 App / 断连 / 开始录制）整体取消在途判定：松手沿可能已经丢了，
    /// 不清就会有一个键在后台一直重复下去。
    pub fn clear(&mut self) {
        self.holding.clear();
    }
}

#[derive(Clone, Copy)]
enum KeyGesture {
    /// 首按按住中。long_fired = 长按已到点判出，松开时吞掉单击。
    /// `rec_double` / `rec_long` = 这一按要不要判双击 / 长按（口径见 [`GestureTracker::press`]）。
    Holding {
        t0: Instant,
        long_fired: bool,
        rec_double: bool,
        rec_long: bool,
    },
    /// 已松开，等可能的第二次按下（到 t0 + DOUBLE_WINDOW 为止）。
    AwaitSecond { t0: Instant },
    /// 双击的第二次按下按住中：吞到松开为止，不再归类。
    Consume,
    /// tick 已结算（单击窗到期），循环后统一清除的占位。
    Done,
}

/// 锁定键（方向四键 / OK）的**连按防误判**。
///
/// 这些键的单击已交原生（零延迟导航），双击槽是「原生之外额外加的一层」。问题在于：
/// 面板里用方向键快速移动焦点时，第二下必然落在双击窗内，于是导航被当成双击——
/// 用户 2026-11-23 实报「快速面板里按方向键变成了放大/缩小」（↑↓ 的双击槽上挂着 ⌘= / ⌘-）。
///
/// 口径：**连按到第 3 下，整串都不算双击**；双击只在「恰好两下、且后面没有第三下」时出。
/// 代价是锁定键的双击比原来晚一个窗（约 300ms）——这是把「导航」与「双击」分开的必要代价，
/// 非锁定键不受影响，仍按原样立即出双击。
#[derive(Default)]
pub struct NavGate {
    /// (键, 本串连按到第几下, 上一按时刻)
    runs: Vec<(RemoteButton, u8, Instant)>,
    /// 待出的双击 (键, 到期时刻)——到期时若没被第三按作废就执行。
    pending: Vec<(RemoteButton, Instant)>,
    /// 哪些键走这套判定（= 单击锁定键）。
    locked: Vec<RemoteButton>,
}

impl NavGate {
    /// 声明某键是否单击锁定（每拍同步一次，配置改了立刻跟上）。
    pub fn set_locked(&mut self, b: RemoteButton, locked: bool) {
        self.locked.retain(|k| *k != b);
        if locked {
            self.locked.push(b);
        }
    }

    pub fn is_locked(&self, b: RemoteButton) -> bool {
        self.locked.contains(&b)
    }

    /// 收到一次按下。true = 这一按把双击**挂起**（调用方别立即执行，等 [`tick`]）。
    pub fn press(&mut self, b: RemoteButton, now: Instant, window: Duration) -> bool {
        if !self.is_locked(b) {
            return false;
        }
        let prev = self
            .runs
            .iter()
            .find(|(k, _, _)| *k == b)
            .map(|(_, _, t)| *t);
        let chained = prev
            .map(|t| now.duration_since(t) <= window)
            .unwrap_or(false);
        let n = if chained {
            match self.runs.iter_mut().find(|(k, _, _)| *k == b) {
                Some((_, c, t)) => {
                    *c = c.saturating_add(1);
                    *t = now;
                    *c
                }
                None => 1,
            }
        } else {
            self.runs.retain(|(k, _, _)| *k != b);
            self.runs.push((b, 1, now));
            1
        };
        if n == 2 {
            // 恰好两下：挂起一个窗，窗内没有第三按才算双击。
            self.pending.retain(|(k, _)| *k != b);
            self.pending.push((b, now + window));
            return true;
        }
        // 第三下起整串作废（导航不是双击）；第一按也不留旧挂起。
        self.pending.retain(|(k, _)| *k != b);
        false
    }

    /// 到期的双击（调用方逐条执行）。
    pub fn tick(&mut self, now: Instant) -> Vec<RemoteButton> {
        let due: Vec<RemoteButton> = self
            .pending
            .iter()
            .filter(|(_, t)| *t <= now)
            .map(|(b, _)| *b)
            .collect();
        self.pending.retain(|(b, _)| !due.contains(b));
        due
    }

    /// 上下文变化（切 App / 断连 / 开始录制）：在途的挂起双击整体作废。
    pub fn clear(&mut self) {
        self.runs.clear();
        self.pending.clear();
    }
}

/// 手势判定器：把按下/松开边沿按时间窗归类为单击/双击/长按（计划 §三）。
///
/// RC001 固件只有按下/松开沿，没有手势事件——归类只能在软件做。
/// 判定本身零延迟；执行是否等判定由调用方决定（S-C）：纯原生键直通不延迟，
/// 绑了槽的键单击最多等 DOUBLE_WINDOW（双击判定的物理代价）。
/// `tick` 由 reader 线程按读超时节拍驱动（长按到点、单击窗到期靠它）。
#[derive(Default)]
pub struct GestureTracker {
    keys: Vec<(RemoteButton, KeyGesture)>,
    /// 当前生效的时序（C4）：页面改了时序不必重开连接，读线程每拍 `set_timing` 跟上。
    timing: KeyTiming,
}

impl GestureTracker {
    /// 换一套时序（C4）。已按住的手势沿用新窗判定——判定只看 `t0` 到 `now` 的时长，
    /// 换窗不会让已经判出的手势重复出。
    pub fn set_timing(&mut self, timing: KeyTiming) {
        self.timing = timing;
    }

    /// 按住连发接管这一按（C4）：之后的长按到点与松开都不再出判定，
    /// 免得连发已经跑了好几发、松手又补一发单击。没在按住就什么都不做。
    pub fn suppress(&mut self, b: RemoteButton) {
        if let Some((_, st)) = self.keys.iter_mut().find(|(k, _)| *k == b) {
            if let KeyGesture::Holding { t0, .. } = *st {
                *st = KeyGesture::Holding {
                    t0,
                    long_fired: true,
                    rec_double: true,
                    rec_long: true,
                };
            }
        }
    }

    /// 按下沿。Some = 该次按下立即出判定（未绑双击/长按的立即单击；或窗已过补判的单击）。
    ///
    /// **手势判定按绑定条件开启**（口径对齐 SayAll `BridgeAppModel` 的
    /// `recognizesDoubleClick` / `recognizesLongPress`）：没绑双击也没绑长按的键
    /// 不进双击窗，单击立即出。以前对所有键无条件开窗，白让「只绑了单击」的键
    /// 等一个双击窗（默认 300ms）——那部分延迟是本可以不要的。
    pub fn press(
        &mut self,
        b: RemoteButton,
        now: Instant,
        rec_double: bool,
        rec_long: bool,
    ) -> Option<Gesture> {
        let st = self.take(b);
        if !rec_double && !rec_long {
            return Some(Gesture::Click);
        }
        if let Some(KeyGesture::AwaitSecond { t0 }) = st {
            if now.duration_since(t0) <= self.timing.double() {
                self.push(b, KeyGesture::Consume);
                return Some(Gesture::Double);
            }
            // 窗已过但 tick 没赶上（防御）：先补记单击，本次按下另起新判定。
            self.push(
                b,
                KeyGesture::Holding {
                    t0: now,
                    long_fired: false,
                    rec_double,
                    rec_long,
                },
            );
            return Some(Gesture::Click);
        }
        // Holding 中再按 = 报告异常（EdgeTracker 保证同键不会），重置为新按下最稳。
        self.push(
            b,
            KeyGesture::Holding {
                t0: now,
                long_fired: false,
                rec_double,
                rec_long,
            },
        );
        None
    }

    /// 松开沿。Some = 此刻出判定（没绑双击的松开即单击；绑了则等第二个按下或窗到期）。
    pub fn release(&mut self, b: RemoteButton, now: Instant) -> Option<Gesture> {
        match self.take(b) {
            Some(KeyGesture::Holding {
                t0,
                long_fired,
                rec_double,
                ..
            }) => {
                let held = now.duration_since(t0);
                if long_fired {
                    None // 长按已吞单击
                } else if held >= self.timing.long() {
                    Some(Gesture::Long) // tick 没赶上的兜底
                } else if !rec_double || held >= self.timing.double() {
                    Some(Gesture::Click) // 没绑双击 → 不等；或双击窗已过
                } else {
                    self.push(b, KeyGesture::AwaitSecond { t0 });
                    None
                }
            }
            _ => None, // Consume / 无状态（重复松开帧）
        }
    }

    /// 节拍判定：长按到点、单击窗到期。返回此刻出判定的全部 (键, 手势)。
    pub fn tick(&mut self, now: Instant) -> Vec<(RemoteButton, Gesture)> {
        let mut out = Vec::new();
        for (b, st) in self.keys.iter_mut() {
            match *st {
                KeyGesture::Holding {
                    t0,
                    long_fired: false,
                    rec_long: true,
                    ..
                } if now.duration_since(t0) >= self.timing.long() => {
                    *st = KeyGesture::Holding {
                        t0,
                        long_fired: true,
                        rec_double: true,
                        rec_long: true,
                    };
                    out.push((*b, Gesture::Long));
                }
                KeyGesture::AwaitSecond { t0 }
                    if now.duration_since(t0) >= self.timing.double() =>
                {
                    out.push((*b, Gesture::Click));
                    *st = KeyGesture::Done;
                }
                _ => {}
            }
        }
        self.keys.retain(|(_, st)| !matches!(*st, KeyGesture::Done));
        out
    }

    /// 断连/切 App/开始录制时清空，避免残影手势（C4：上下文一变就整体取消在途判定）。
    pub fn clear(&mut self) {
        self.keys.clear();
    }

    fn take(&mut self, b: RemoteButton) -> Option<KeyGesture> {
        self.keys
            .iter()
            .position(|(k, _)| *k == b)
            .map(|i| self.keys.remove(i).1)
    }

    fn push(&mut self, b: RemoteButton, st: KeyGesture) {
        self.keys.push((b, st));
    }
}

#[cfg(test)]
mod tests {
    /// 手势判定按绑定条件开启（对齐 SayAll）：没绑双击也没绑长按的键不进双击窗。
    #[test]
    fn gesture_skips_double_window_when_nothing_bound() {
        let t0 = Instant::now();
        let mut g = GestureTracker::default();
        assert_eq!(
            g.press(RemoteButton::Home, t0, false, false),
            Some(Gesture::Click),
            "三槽只绑单击 → 按下立即出单击，零延迟"
        );
        assert!(
            g.tick(t0 + ms(400)).is_empty(),
            "已立即出判定，窗到期不许再补一发"
        );
        // 只绑长按（没绑双击）：松开即单击，不必等双击窗
        assert_eq!(
            g.press(RemoteButton::Home, t0 + ms(1000), false, true),
            None
        );
        assert_eq!(
            g.release(RemoteButton::Home, t0 + ms(1100)),
            Some(Gesture::Click)
        );
    }

    /// 锁定键连按防误判：两下挂起、窗内没第三按才出双击；第三按整串作废。
    #[test]
    fn nav_gate_defers_double_and_drops_navigation_burst() {
        let w = Duration::from_millis(300);
        let mut g = NavGate::default();
        g.set_locked(RemoteButton::Up, true);
        let t0 = Instant::now();
        assert!(!g.press(RemoteButton::Up, t0, w), "第一按不挂起");
        assert!(
            g.press(RemoteButton::Up, t0 + Duration::from_millis(120), w),
            "第二按应挂起"
        );
        assert_eq!(
            g.tick(t0 + Duration::from_millis(500)),
            vec![RemoteButton::Up],
            "窗内无第三按 → 双击"
        );
        // 导航连按：第三按落进窗内，整串都不算双击
        assert!(!g.press(RemoteButton::Up, t0 + Duration::from_secs(1), w));
        assert!(g.press(
            RemoteButton::Up,
            t0 + Duration::from_secs(1) + Duration::from_millis(100),
            w
        ));
        assert!(
            !g.press(
                RemoteButton::Up,
                t0 + Duration::from_secs(1) + Duration::from_millis(200),
                w
            ),
            "第三按应撤挂起"
        );
        assert!(
            g.tick(t0 + Duration::from_secs(3)).is_empty(),
            "导航串不许补出双击"
        );
        // 非锁定键不参与这套判定
        assert!(!g.press(RemoteButton::Home, t0, w));
    }

    use super::*;
    use std::time::Instant;

    #[test]
    fn usage_table_roundtrip() {
        assert_eq!(RemoteButton::ALL.len(), 13);
        for b in RemoteButton::ALL {
            assert_eq!(RemoteButton::from_usage(b.hid_usage()), Some(b), "{b:?}");
        }
        assert_eq!(RemoteButton::from_usage(0x00), None);
        assert_eq!(RemoteButton::Tv.hid_usage(), 0x35);
        assert_eq!(RemoteButton::Back.hid_usage(), 0xF1);
    }

    /// 演示驱动按 id 喂键：id 必须能逆向回键，未知 id 不能默默兑成某颗键。
    #[test]
    fn from_id_round_trips_id() {
        for b in RemoteButton::ALL {
            assert_eq!(RemoteButton::from_id(b.id()), Some(b), "{}", b.id());
        }
        assert_eq!(RemoteButton::from_id("nope"), None);
        assert_eq!(RemoteButton::from_id(""), None);
    }

    #[test]
    fn parse_report_le_u16_pairs() {
        // OK + 上 同时按下（三槽，第三槽空）。
        let payload = [0x28u8, 0x00, 0x52, 0x00, 0x00, 0x00];
        assert_eq!(parse_usages(1, 1, &payload), Some(vec![0x28, 0x52]));
        // reportID 不对 → 不是本档案的报告。
        assert_eq!(parse_usages(1, 2, &payload), None);
        // 奇数长度 = 坏帧。
        assert_eq!(parse_usages(1, 1, &[0x28, 0x00, 0x52]), None);
        // 全空槽 = 全释放。
        assert_eq!(parse_usages(1, 1, &[0, 0, 0, 0]), Some(vec![]));
        // 空 payload（纯释放帧）合法。
        assert_eq!(parse_usages(1, 1, &[]), Some(vec![]));
        // 重复 usage 去重。
        assert_eq!(parse_usages(1, 1, &[0x28, 0, 0x28, 0]), Some(vec![0x28]));
    }

    #[test]
    fn edge_down_up() {
        let mut t = EdgeTracker::default();
        let (d, u) = t.apply(&[RemoteButton::Ok]);
        assert_eq!(d, vec![RemoteButton::Ok]);
        assert!(u.is_empty());
        // 同帧重复不再算按下。
        let (d, u) = t.apply(&[RemoteButton::Ok]);
        assert!(d.is_empty() && u.is_empty());
        // 释放。
        let (d, u) = t.apply(&[]);
        assert!(d.is_empty());
        assert_eq!(u, vec![RemoteButton::Ok]);
        // 未知 usage 在 buttons_of 就被丢掉。
        assert_eq!(buttons_of(&[0x28, 0x99]), vec![RemoteButton::Ok]);
    }

    #[test]
    fn repeat_gate_timing() {
        let t0 = Instant::now();
        let mut g = RepeatGate::default();
        g.press(RemoteButton::Up, t0);
        assert!(g.tick(t0 + Duration::from_millis(349)).is_empty());
        assert_eq!(
            g.tick(t0 + Duration::from_millis(351)),
            vec![RemoteButton::Up]
        );
        assert!(g.tick(t0 + Duration::from_millis(400)).is_empty());
        assert_eq!(
            g.tick(t0 + Duration::from_millis(451)),
            vec![RemoteButton::Up]
        );
        g.release(RemoteButton::Up, t0 + Duration::from_millis(500));
        assert!(g.tick(t0 + Duration::from_millis(900)).is_empty());
    }

    #[test]
    fn repeat_interval_per_button() {
        assert_eq!(
            repeat_interval(RemoteButton::Up),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            repeat_interval(RemoteButton::VolumeDown),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            repeat_interval(RemoteButton::Back),
            Some(Duration::from_millis(50))
        );
        assert_eq!(repeat_interval(RemoteButton::Ok), None);
        assert_eq!(repeat_interval(RemoteButton::Power), None);
        // 不连发的键 press 后 tick 永远空。
        let t0 = Instant::now();
        let mut g = RepeatGate::default();
        g.press(RemoteButton::Ok, t0);
        assert!(g.tick(t0 + Duration::from_secs(5)).is_empty());
    }

    #[test]
    fn rapid_gate_debounce() {
        let t0 = Instant::now();
        let mut g = RapidGate::default();
        assert!(g.allow(RemoteButton::Ok, t0, false));
        // 窗口内再按：不允许 → 丢弃。
        assert!(!g.allow(RemoteButton::Ok, t0 + Duration::from_millis(100), false));
        // 被丢弃的再按不刷新窗口：距首发 300ms 已出窗 → 放行。
        assert!(g.allow(RemoteButton::Ok, t0 + Duration::from_millis(300), false));
        // 允许连按的键窗口内照放。
        assert!(g.allow(RemoteButton::Up, t0, true));
        assert!(g.allow(RemoteButton::Up, t0 + Duration::from_millis(10), true));
        // 逐键独立。
        assert!(g.allow(RemoteButton::Menu, t0, false));
        assert!(!g.allow(RemoteButton::Menu, t0 + Duration::from_millis(10), false));
        assert!(g.allow(RemoteButton::Home, t0 + Duration::from_millis(10), false));
    }

    #[test]
    fn native_event_table() {
        assert_eq!(native_event(RemoteButton::Ok), NativeEvent::Keyboard(36));
        assert_eq!(native_event(RemoteButton::Tv), NativeEvent::Keyboard(10));
        assert_eq!(native_event(RemoteButton::Power), NativeEvent::Keyboard(90));
        assert_eq!(native_event(RemoteButton::Back), NativeEvent::None);
        assert_eq!(
            native_event(RemoteButton::VolumeUp),
            NativeEvent::SystemKey(0)
        );
        // tv 的抑制集额外认历史值 50。
        assert_eq!(
            native_events(RemoteButton::Tv),
            vec![NativeEvent::Keyboard(10), NativeEvent::Keyboard(50)]
        );
        assert_eq!(native_events(RemoteButton::Ok).len(), 1);
    }

    fn ms(m: u64) -> Duration {
        Duration::from_millis(m)
    }

    #[test]
    fn gesture_click_emits_after_double_window() {
        let t0 = Instant::now();
        let mut g = GestureTracker::default();
        assert_eq!(g.press(RemoteButton::Ok, t0, true, true), None);
        assert_eq!(g.release(RemoteButton::Ok, t0 + ms(200)), None);
        // 窗未过不急着发单击——第二次按下还可能变双击。
        assert!(g.tick(t0 + ms(299)).is_empty());
        assert_eq!(
            g.tick(t0 + ms(301)),
            vec![(RemoteButton::Ok, Gesture::Click)]
        );
        assert!(g.tick(t0 + ms(900)).is_empty());
    }

    #[test]
    fn gesture_release_after_window_is_immediate_click() {
        // 松开已超双击窗（但未到长按阈值）：不必再等，立刻单击。
        let t0 = Instant::now();
        let mut g = GestureTracker::default();
        g.press(RemoteButton::Ok, t0, true, true);
        assert_eq!(
            g.release(RemoteButton::Ok, t0 + ms(400)),
            Some(Gesture::Click)
        );
        assert!(g.tick(t0 + ms(900)).is_empty());
    }

    #[test]
    fn gesture_double_swallows_first_click() {
        let t0 = Instant::now();
        let mut g = GestureTracker::default();
        g.press(RemoteButton::Ok, t0, true, true);
        assert_eq!(g.release(RemoteButton::Ok, t0 + ms(120)), None);
        // 第二次按下 ≤300ms：按下即判双击，不等松开。
        assert_eq!(
            g.press(RemoteButton::Ok, t0 + ms(250), true, true),
            Some(Gesture::Double)
        );
        assert_eq!(g.release(RemoteButton::Ok, t0 + ms(400)), None);
        // 第一次的单击被吞：任何时刻都不该再补发。
        assert!(g.tick(t0 + ms(900)).is_empty());
    }

    #[test]
    fn gesture_long_fires_at_threshold_without_release() {
        let t0 = Instant::now();
        let mut g = GestureTracker::default();
        g.press(RemoteButton::Ok, t0, true, true);
        assert!(g.tick(t0 + ms(549)).is_empty());
        assert_eq!(
            g.tick(t0 + ms(551)),
            vec![(RemoteButton::Ok, Gesture::Long)]
        );
        assert!(g.tick(t0 + ms(900)).is_empty()); // 只判一次
        assert_eq!(g.release(RemoteButton::Ok, t0 + ms(1000)), None); // 长按吞单击
    }

    #[test]
    fn gesture_long_defensive_without_tick() {
        // 中间一次 tick 都没赶上就松开且超阈值：判长按，不许误判成单击。
        let t0 = Instant::now();
        let mut g = GestureTracker::default();
        g.press(RemoteButton::Ok, t0, true, true);
        assert_eq!(
            g.release(RemoteButton::Ok, t0 + ms(600)),
            Some(Gesture::Long)
        );
    }

    #[test]
    fn gesture_second_press_after_window_is_new_press() {
        let t0 = Instant::now();
        let mut g = GestureTracker::default();
        g.press(RemoteButton::Ok, t0, true, true);
        g.release(RemoteButton::Ok, t0 + ms(100));
        // 窗已过而 tick 没来就来按下：先补记单击，本次另起新判定。
        assert_eq!(
            g.press(RemoteButton::Ok, t0 + ms(350), true, true),
            Some(Gesture::Click)
        );
        assert_eq!(g.release(RemoteButton::Ok, t0 + ms(550)), None);
        assert_eq!(
            g.tick(t0 + ms(660)),
            vec![(RemoteButton::Ok, Gesture::Click)]
        );
    }

    #[test]
    fn timing_windows_are_configurable_at_runtime() {
        // C4：双击窗/长按窗可配（页面改了不必重开连接）。收窄双击窗到 200ms 后，
        // 隔 400ms 的第二次按下是全新一次按下；长按也跟着推到 900ms。
        let t0 = Instant::now();
        let mut g = GestureTracker::default();
        g.set_timing(KeyTiming {
            double_ms: 200,
            long_ms: 900,
            ..Default::default()
        });
        g.press(RemoteButton::Ok, t0, true, true);
        assert_eq!(g.release(RemoteButton::Ok, t0 + ms(120)), None);
        assert_eq!(
            g.tick(t0 + ms(250)),
            vec![(RemoteButton::Ok, Gesture::Click)],
            "窗收窄到 200ms：250ms 时单击已结算"
        );
        assert_eq!(
            g.press(RemoteButton::Ok, t0 + ms(400), true, true),
            None,
            "不该判成双击"
        );
        assert!(g.tick(t0 + ms(1_299)).is_empty());
        assert_eq!(
            g.tick(t0 + ms(1_301)),
            vec![(RemoteButton::Ok, Gesture::Long)]
        );
    }

    #[test]
    fn timing_normalize_keeps_windows_sane() {
        // 越界按边界截；长按窗必须大于双击窗，否则按住永远抢在双击前面。
        let mut t = KeyTiming {
            double_ms: 10,
            long_ms: 9_999,
            repeat_start_ms: 0,
            repeat_interval_ms: 0,
        };
        t.normalize();
        assert_eq!(t.double_ms, 120);
        assert_eq!(t.long_ms, 1_500);
        assert_eq!(t.repeat_start_ms, 150);
        assert_eq!(t.repeat_interval_ms, 30);
        let mut bad = KeyTiming {
            double_ms: 600,
            long_ms: 400,
            ..Default::default()
        };
        bad.normalize();
        assert!(bad.long_ms > bad.double_ms, "长按窗被抬到双击窗之上");
        // 默认值就是计划口径。
        let d = KeyTiming::default();
        assert_eq!(
            (
                d.double_ms,
                d.long_ms,
                d.repeat_start_ms,
                d.repeat_interval_ms
            ),
            (300, 550, 350, 100)
        );
    }

    #[test]
    fn hold_repeat_starts_after_delay_then_keeps_beating() {
        // 首发在 press + 350ms，之后每 100ms 一发；松手即停。
        let t0 = Instant::now();
        let timing = KeyTiming::default();
        let mut gate = HoldRepeatGate::default();
        gate.press(RemoteButton::VolumeUp, HoldMode::RepeatSingle, &timing, t0);
        assert!(gate.tick(t0 + ms(349), &timing).is_empty());
        assert_eq!(
            gate.tick(t0 + ms(351), &timing),
            vec![(RemoteButton::VolumeUp, HoldMode::RepeatSingle)]
        );
        assert_eq!(gate.tick(t0 + ms(451), &timing).len(), 1);
        gate.release(RemoteButton::VolumeUp);
        assert!(gate.tick(t0 + ms(600), &timing).is_empty(), "松手就该停");
    }

    #[test]
    fn hold_repeat_skips_once_and_clear_kills_everything() {
        let t0 = Instant::now();
        let timing = KeyTiming::default();
        let mut gate = HoldRepeatGate::default();
        // Once = 现有行为：按住也不该多出任何一发。
        gate.press(RemoteButton::Ok, HoldMode::Once, &timing, t0);
        assert!(gate.tick(t0 + ms(5_000), &timing).is_empty());
        gate.press(RemoteButton::VolumeUp, HoldMode::RepeatLong, &timing, t0);
        // 同键重复按下沿不排第二路连发。
        gate.press(
            RemoteButton::VolumeUp,
            HoldMode::RepeatLong,
            &timing,
            t0 + ms(50),
        );
        assert_eq!(gate.tick(t0 + ms(360), &timing).len(), 1);
        // 上下文变化（切 App/断连）整体取消在途判定。
        gate.clear();
        assert!(gate.tick(t0 + ms(900), &timing).is_empty());
    }

    #[test]
    fn gesture_keys_are_independent() {
        let t0 = Instant::now();
        let mut g = GestureTracker::default();
        g.press(RemoteButton::Up, t0, true, true);
        g.press(RemoteButton::Down, t0 + ms(100), true, true);
        assert_eq!(g.release(RemoteButton::Up, t0 + ms(150)), None);
        // up 的单击窗到期与 down 的长按到点互不干扰（顺序 = 表内插入序，up 松开重排到了队尾）。
        // down 按在 t0+100，长按窗 550ms → t0+650 到点。
        assert_eq!(
            g.tick(t0 + ms(650)),
            vec![
                (RemoteButton::Down, Gesture::Long),
                (RemoteButton::Up, Gesture::Click)
            ]
        );
    }
}
