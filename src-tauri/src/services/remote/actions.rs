//! 按键动作表与持久化（纯逻辑，无 cfg、无 FFI；默认 `cargo test` 全测）。
//!
//! 三槽模型（计划 `2026-11-19-remote-three-slot-mapping` §二，切片 S-A）：
//! 每键 = 使能位 + 三槽（单击/双击/长按）快捷键绑定 + 连按开关。
//! - 槽未绑 = 原生透传（默认）；键禁用 = 什么都不做；
//! - 快捷键 = 该手势触发时注入的按键组合（shared 模式叠加、exclusive 模式替换，见计划 §四）；
//! - **无黑名单**：裸单键（Esc/Tab…，mods=0）全可绑（用户 2026-11-20 拍板）；
//! - 唯一拒绝条件 = 冲突：所有键 × 所有槽的快捷键组合全局唯一。
//!
//! 旧形状 `{bindings, rapid}` 读取时自动迁移，保存一律新形状。
//! 持久化（21b 改外部文件，用户 2026-11-20 拍板）：`<data_dir>/remote/buttons.<profile>.json`
//! 按档案分仓、pretty JSON——导入导出就是拷文件；旧 settings 表数据首读时自动迁移落盘。

use crate::services::remote::buttons::{native_event, NativeEvent, RemoteButton};
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// 键级动作（键使能位语义；快捷键绑定在槽上，不在本枚举）。
/// serde 名稳定，进持久化与 IPC。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonAction {
    /// 原生透传（默认）。
    Native,
    /// 禁用该键。
    Disabled,
}

/// 三槽：单击 / 双击 / 长按。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    Click,
    Double,
    Long,
}

impl Slot {
    pub const ALL: [Self; 3] = [Self::Click, Self::Double, Self::Long];

    /// 后端兜底报错文案用中文名（前端 S-D 片有自己的 i18n 文案，这里只是最后防线）。
    pub fn label(self) -> &'static str {
        match self {
            Self::Click => "单击",
            Self::Double => "双击",
            Self::Long => "长按",
        }
    }
}

/// 修饰键左右侧（C1，计划 `2026-11-21-SayAll对标` §三）：注入时并 `NX_DEVICEL*KEYMASK`，
/// 应用据此分辨左右 ⌘/⌥/⌃/⇧。None = 不区分侧（旧配置、组合混侧）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
}

/// 快捷键组合：mods 位图（bit0 ctrl bit1 shift bit2 alt bit3 meta）+ CGEvent 虚拟键码。
/// 裸单键 = `mods = 0`。
/// C1 扩展：`r#fn` = 组合带 Fn 位（只认 flagsChanged(Fn) 边沿录入，不从事件 flags 猜——
/// 遥控器方向键的 KeyDown 自带 Fn 伪位，猜了就会把 ⌘X 录成 Fn⌘X）；
/// `side` = 修饰键取左/右侧（单修饰键组合由录入键码判定，混侧为 None）。
/// 两字段 serde default：旧配置（只有 mods/key）原样读入，fn=false、side=None。
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub struct Shortcut {
    pub mods: u8,
    pub key: u16,
    /// 组合带 Fn 位（`fn` 是 Rust 关键字，serde 名仍是 "fn"）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub r#fn: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
}

/// 可独立成键的修饰键 kVK（C1「单修饰键绑定」，9 个）：
/// 54 右⌘ 55 左⌘ 56 左⇧ 58 左⌥ 59 左⌃ 60 右⇧ 61 右⌥ 62 右⌃ 63 Fn。
/// 57 是 CapsLock，不在内。键 → (mods 位, 侧)；Fn 无 mods 位（走 flags 的 Fn 位）。
pub const MOD_KEYCODES: [(u16, u8, Option<Side>); 9] = [
    (54, 8, Some(Side::Right)), // 右⌘
    (55, 8, Some(Side::Left)),  // 左⌘
    (56, 2, Some(Side::Left)),  // 左⇧
    (58, 4, Some(Side::Left)),  // 左⌥
    (59, 1, Some(Side::Left)),  // 左⌃
    (60, 2, Some(Side::Right)), // 右⇧
    (61, 4, Some(Side::Right)), // 右⌥
    (62, 1, Some(Side::Right)), // 右⌃
    (63, 0, None),              // Fn：无 mods 位，注入走 SecondaryFn flags
];

/// kVK → 修饰键条目（mods 位 + 侧）；非修饰键回 None。
pub fn mod_key(key: u16) -> Option<(u8, Option<Side>)> {
    MOD_KEYCODES
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|&(_, m, s)| (m, s))
}

impl Shortcut {
    /// 裸单修饰键绑定（mods=0 且键码是 9 键之一）：注入走 FlagsChanged 而非 KeyDown。
    pub fn is_mod_key(&self) -> bool {
        self.mods == 0 && mod_key(self.key).is_some()
    }
}

/// 槽位动作（C2，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）：三槽每槽一个动作。
///
/// 与 `ButtonAction` 是两件事：那是键级使能位（原生透传 / 禁用），这是槽级动作。
/// 名字撞车属历史遗留（C2 之前槽里只能放快捷键，动作 == 快捷键），而 `ButtonAction`
/// 的 serde 名早已进持久化与 IPC，改名代价大于收益，故保留原名不动。
///
/// serde = internally tagged union（`type` 字段）。旧形状（C2 之前槽值是裸
/// `{"mods":…,"key":…}`，无 `type`）读取时自动迁移成 `Action::Shortcut`，
/// 一条映射不丢（`待用户确认.md` §5 拍板 A）。执行口径在 `action_exec.rs`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// 注入一组键盘组合（C1 的 `Shortcut`，含 fn 位与左右侧）。
    Shortcut { shortcut: Shortcut },
    /// 打开/唤起指定 bundle id 的 App，可带启动参数（`args` 按 shell 引号规则分词，
    /// 空 = 无参）。`args` 带 `serde(default)`：老配置里没这个字段照常加载。
    OpenApp {
        bundle_id: String,
        #[serde(default)]
        args: String,
    },
    /// 在终端里起一个代理 CLI（带参数与工作目录，用户 2026-11-23 要求）。
    /// `app` 用接管页同一套键名（`ClientApp::as_str`），可执行路径取安装扫描的
    /// 登录 shell 探测结果。**必须进终端**：交互式 CLI 没有 TTY 起不来，
    /// `RunCommand` 那种无终端跑法跑 `codex` 只会没影。
    AgentCli {
        app: String,
        #[serde(default)]
        args: String,
        #[serde(default)]
        cwd: String,
    },
    /// 向当前聚焦输入框注入一段文本（等价输入法上屏，不碰剪贴板）。
    Text { content: String },
    /// 让前台 App 的输入区拿到键盘焦点（找不到即 no-op）。
    FocusInput,
    /// 登录 shell 异步跑一行命令（执行细则见 C8）。
    RunCommand { command: String },
    /// 播放一条宏（C3）：按 id 引用宏库 `remote/macros.<profile>.json`。
    /// 步骤**不副本进键位档**——宏改一次所有绑点同时生效，删除宏时跨全部方案清引用（`macros.rs`）。
    Macro { id: String },
}

impl Action {
    /// 参与全局唯一判定的只有快捷键动作（计划 §三：冲突检测只对 Shortcut）——
    /// 两条一样的命令、两个同样的「打开 App」都是用户有意为之，不算冲突。
    pub fn shortcut(&self) -> Option<Shortcut> {
        match self {
            Self::Shortcut { shortcut } => Some(*shortcut),
            _ => None,
        }
    }

    /// 空白值（文本 / 命令全空白、bundle id 空）按未绑处理：不许存一条按了没反应的动作。
    pub fn is_blank(&self) -> bool {
        let blank = |s: &str| s.trim().is_empty();
        match self {
            Self::Text { content } => blank(content),
            Self::RunCommand { command } => blank(command),
            Self::OpenApp { bundle_id, .. } => blank(bundle_id),
            Self::AgentCli { app, .. } => blank(app),
            Self::Macro { id } => blank(id),
            _ => false,
        }
    }

    /// 后端兜底文案（前端有自己的 i18n 文案，这里只是日志/报错的最后防线）。
    pub fn label(&self) -> String {
        match self {
            Self::Shortcut { .. } => "快捷键".to_string(),
            Self::OpenApp { bundle_id, args } => {
                if args.trim().is_empty() {
                    format!("打开 App {bundle_id}")
                } else {
                    format!("打开 App {bundle_id} {args}")
                }
            }
            Self::AgentCli { app, args, .. } => {
                if args.trim().is_empty() {
                    format!("代理 CLI {app}")
                } else {
                    format!("代理 CLI {app} {args}")
                }
            }
            Self::Text { content } => {
                let head: String = content.chars().take(12).collect();
                format!("注入文本「{head}」")
            }
            Self::FocusInput => "聚焦输入框".to_string(),
            Self::RunCommand { command } => format!("运行命令 {command}"),
            // 宏只有 id，名字在宏库里；执行侧解析到整条宏后会给更准的读数（`hid.rs`）。
            Self::Macro { id } => format!("宏 {id}"),
        }
    }
}

/// 槽值的线上形状：新 = 带 `type` 的 `Action`；旧 = 裸 `Shortcut`（读入即迁移）。
/// untagged 按声明序试：`Action` 要求 `type` 字段，旧形状缺它就落到 `Legacy`。
#[derive(Deserialize)]
#[serde(untagged)]
enum SlotWire {
    Action(Action),
    Legacy(Shortcut),
}

fn deserialize_slot<'de, D>(deserializer: D) -> Result<Option<Action>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(
        Option::<SlotWire>::deserialize(deserializer)?.map(|w| match w {
            SlotWire::Action(a) => a,
            SlotWire::Legacy(shortcut) => Action::Shortcut { shortcut },
        }),
    )
}

/// 逐键配置：使能位 + 三槽 + 连按。缺键 = 全默认（启用、未绑、不连按）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyConfig {
    /// false = 禁用（旧形状里的 "disabled"）。
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub click: Option<Action>,
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub double: Option<Action>,
    #[serde(default, deserialize_with = "deserialize_slot")]
    pub long: Option<Action>,
    /// 允许连续快速按（只作用于原生透传的连发语义，与三槽手势判定互不干扰）。
    #[serde(default)]
    pub rapid: bool,
}

fn enabled_by_default() -> bool {
    true
}

// 手写 Default：派生会把 enabled 置 false（=禁用），与「缺键=启用」口径相反，
// set_rapid/set_slot_action 走 or_default 时会把键误禁（S-A 单测抓到）。
impl Default for KeyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            click: None,
            double: None,
            long: None,
            rapid: false,
        }
    }
}

impl KeyConfig {
    pub fn slot(&self, slot: Slot) -> Option<&Action> {
        match slot {
            Slot::Click => self.click.as_ref(),
            Slot::Double => self.double.as_ref(),
            Slot::Long => self.long.as_ref(),
        }
    }

    pub fn set_slot(&mut self, slot: Slot, action: Option<Action>) {
        match slot {
            Slot::Click => self.click = action,
            Slot::Double => self.double = action,
            Slot::Long => self.long = action,
        }
    }
}

/// 冲突：两个（键，槽）位置绑了同一快捷键——这是保存的**唯一**拒绝条件。
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub shortcut: Shortcut,
    pub first: (String, Slot),
    pub second: (String, Slot),
}

/// 键位配置表：键 id → 逐键配置。缺键 = 默认；未知键 id 原样保留不影响已知键。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ButtonConfig {
    /// 文件格式版本（借鉴 SayAll 导出格式的做法）：形状变更时递增，导入方可拒收。
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub keys: BTreeMap<String, KeyConfig>,
}

/// 当前文件格式版本：C2 把槽值从裸 `Shortcut` 泛化成 tagged `Action`（旧档读入自动迁移）
/// 后递增到 2；读侧仍接受 1（迁移在 `deserialize_slot`），写侧一律最新。
fn default_format_version() -> u32 {
    2
}

// 手写 Default：派生会把 format_version 置 0（=坏档），新表必须是当前版本。
impl Default for ButtonConfig {
    fn default() -> Self {
        Self {
            format_version: 1,
            keys: BTreeMap::new(),
        }
    }
}

impl ButtonConfig {
    fn entry(&mut self, button: RemoteButton) -> &mut KeyConfig {
        self.keys.entry(button.id().to_string()).or_default()
    }

    /// 该键使能态；未配置 = 默认原生透传。（hid.rs 分发口径不变，S-B 才接手势×槽执行。）
    pub fn action(&self, button: RemoteButton) -> ButtonAction {
        match self.keys.get(button.id()) {
            Some(k) if !k.enabled => ButtonAction::Disabled,
            _ => ButtonAction::Native,
        }
    }

    pub fn set_action(&mut self, button: RemoteButton, action: ButtonAction) {
        self.entry(button).enabled = action == ButtonAction::Native;
    }

    /// 该键是否允许连续快速按；默认 false。
    pub fn allows_rapid(&self, button: RemoteButton) -> bool {
        self.keys.get(button.id()).is_some_and(|k| k.rapid)
    }

    pub fn set_rapid(&mut self, button: RemoteButton, allow: bool) {
        self.entry(button).rapid = allow;
    }

    /// 该键某槽绑定的动作；None = 未绑（原生透传）。
    pub fn slot_action(&self, button: RemoteButton, slot: Slot) -> Option<&Action> {
        self.keys.get(button.id()).and_then(|k| k.slot(slot))
    }

    /// 写槽动作。空白值（空文本 / 空命令 / 空 bundle id）归一成未绑——
    /// 存一条按了没反应的动作只会让人以为是坏了。
    pub fn set_slot_action(&mut self, button: RemoteButton, slot: Slot, action: Option<Action>) {
        self.entry(button)
            .set_slot(slot, action.filter(|a| !a.is_blank()));
    }

    /// 该键三槽是否有任一绑定（独占模式「手势驱动」判据：有绑定则按下不注原生、
    /// 等手势执行——计划 §四 替换语义；shared 模式不看它，原生照常透传）。
    /// C2 起判的是「任一动作」，不再只看快捷键。
    pub fn has_bindings(&self, button: RemoteButton) -> bool {
        self.keys
            .get(button.id())
            .is_some_and(|k| Slot::ALL.iter().any(|s| k.slot(*s).is_some()))
    }

    /// 是否存在禁用键（决定共享模式要不要养抑制 tap；
    /// 快捷键槽是叠加注入、不需要抑制，故不计入）。
    pub fn has_non_native(&self) -> bool {
        self.keys.values().any(|k| !k.enabled)
    }

    /// 全默认（native + 未绑 + 不连按）——IPC 空态与测试基准。
    pub fn all_default() -> Self {
        Self::default()
    }

    /// 全局唯一校验：所有键 × 所有槽，返回第一个冲突（BTreeMap 序稳定，报错可复现）。
    /// 无 OS 快捷键黑名单（用户 2026-11-20 拍板：所有允许的按键映射都要能绑）。
    /// C2 起只对快捷键动作判定：非快捷键动作（打开 App / 命令 / 文本）重复是合法配置。
    pub fn find_conflict(&self) -> Option<Conflict> {
        let mut seen: HashMap<Shortcut, (String, Slot)> = HashMap::new();
        for (id, kc) in &self.keys {
            for slot in Slot::ALL {
                let Some(sc) = kc.slot(slot).and_then(Action::shortcut) else {
                    continue;
                };
                if let Some(prev) = seen.insert(sc, (id.clone(), slot)) {
                    return Some(Conflict {
                        shortcut: sc,
                        first: prev,
                        second: (id.clone(), slot),
                    });
                }
            }
        }
        None
    }
}

/// 旧持久化键（settings 表，≤0.2.10x）：只作迁移来源，新保存一律走文件。
pub const CONFIG_KEY: &str = "remote.buttons.config";

/// 缺省映射（用户 2026-11-20 拍板：从 SayAll 最新版导出播种，键位定义进默认方案）。
/// 只写**与原生透传不同**的槽：方向键/Return 等原生等价、媒体动作（音量长按）、
/// pageUp/Down（本模型无 fn 位）、开指定 App 这类表达不了的项一律留未绑。
/// 仅小米档案播种——别的档案键表不同，不套用（同「无实物图不套图」的纪律）。
pub fn default_mapping(profile_id: &str) -> ButtonConfig {
    let mut cfg = ButtonConfig::default();
    // 默认映射跟着键表走，不认档案名：小米 1 代与 rc00x 共用同一张键表，
    // 默认值也该一样（按 id 判会让分出去的档案一上来全是空映射）。
    if crate::services::remote::profile::by_id(profile_id)
        .keys
        .is_empty()
    {
        return cfg;
    }
    let mut bind = |b: RemoteButton,
                    click: Option<(u8, u16)>,
                    double: Option<(u8, u16)>,
                    long: Option<(u8, u16)>| {
        for (slot, v) in [
            (Slot::Click, click),
            (Slot::Double, double),
            (Slot::Long, long),
        ] {
            if let Some((mods, key)) = v {
                cfg.set_slot_action(
                    b,
                    slot,
                    Some(Action::Shortcut {
                        shortcut: Shortcut {
                            mods,
                            key,
                            ..Default::default()
                        },
                    }),
                );
            }
        }
    };
    // mods：bit0 ctrl · bit1 shift · bit2 alt · bit3 cmd；key = kVK（与 shortcut.ts 同源）。
    bind(
        RemoteButton::Power,
        Some((0, 53)),
        Some((1, 18)),
        Some((1, 8)),
    ); // Esc · ⌃1 · ⌃C
    bind(RemoteButton::Up, None, None, Some((5, 126))); // 长按 ⌃⌥↑
    bind(RemoteButton::Down, None, None, Some((5, 125))); // 长按 ⌃⌥↓
    bind(RemoteButton::Left, None, Some((13, 18)), Some((5, 123))); // 双击 ⌃⌥⌘1 · 长按 ⌃⌥←
    bind(RemoteButton::Right, None, Some((13, 19)), Some((5, 124))); // 双击 ⌃⌥⌘2 · 长按 ⌃⌥→
    bind(RemoteButton::Ok, None, Some((0, 48)), Some((2, 36))); // 双击 Tab · 长按 ⇧↩
    bind(
        RemoteButton::Back,
        Some((0, 51)),
        Some((1, 40)),
        Some((8, 51)),
    ); // ⌫ · ⌃K · ⌘⌫
    bind(RemoteButton::Home, Some((8, 48)), None, Some((8, 9))); // ⌘Tab · 长按 ⌘V
    bind(RemoteButton::Menu, None, Some((2, 48)), Some((13, 21))); // 双击 ⇧Tab · 长按 ⌃⌥⌘4
    bind(RemoteButton::Tv, None, Some((8, 45)), Some((8, 6))); // 双击 ⌘N · 长按 ⌘C
    bind(RemoteButton::VolumeUp, None, Some((10, 24)), None); // 双击 ⌘⇧=
    bind(RemoteButton::VolumeDown, None, Some((8, 27)), None); // 双击 ⌘-
    cfg
}

/// 旧形状（≤v0.2.101）：`{bindings, rapid}`。只读来迁移，不再写回。
/// `deny_unknown_fields` 保证新形状 JSON（含 `keys`）不会被误判成旧形状。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OldConfig {
    #[serde(default)]
    bindings: BTreeMap<String, ButtonAction>,
    #[serde(default)]
    rapid: BTreeMap<String, bool>,
}

fn migrate(old: OldConfig) -> ButtonConfig {
    let mut cfg = ButtonConfig::default();
    for (id, action) in old.bindings {
        cfg.keys.entry(id).or_default().enabled = action == ButtonAction::Native;
    }
    for (id, rapid) in old.rapid {
        cfg.keys.entry(id).or_default().rapid = rapid;
    }
    cfg
}

/// 解析：先试旧形状（新形状会被它的 `deny_unknown_fields` 挡下），否则按新形状；
/// 都失败回默认——解析失败不静默吞：写回一条坏数据不如保持默认让页面重新保存覆盖。
pub fn parse(raw: &str) -> ButtonConfig {
    let cfg = if let Ok(old) = serde_json::from_str::<OldConfig>(raw) {
        migrate(old)
    } else {
        serde_json::from_str::<ButtonConfig>(raw).unwrap_or_default()
    };
    normalize(cfg)
}

/// 加载归一入口（**所有**加载路径都必须过这里，含预存方案）。
/// 历史上口径性修正都挂这里，避免"改了代码但老文件没归一"的第二次翻车。
/// 口径性修正都挂在这里（载入时执行，用户旧文件不必手改）。
///
/// **方向四键的单击槽若绑成「它自己且无修饰键」，等于没绑**（用户 2026-11-25 报
/// 「快速面板里上下键不跟手：有时没反应、有时跳两格」，真机键位档里
/// `up.click = {mods:0, key:126}` 就是 ↑ 自己）。那条绑定零收益，却买回三条代价：
///
/// 1. 单击槽有值 → `NavGate` 不锁 → 该键进手势判定；上下键通常还绑着双击，
///    单击就必须等完一个双击窗（默认 300ms）才出——手感就是「按下去半秒不动」。
/// 2. 共享模式下绑了槽的键要抑制原生再补注入，而方向键不在设备级禁用表里、
///    原生此刻已在系统里——抑制账本没吃掉就是原生一发 + 补注入一发 = **跳两格**。
/// 3. 走我们自己的连发路径，macOS 自带的按住自动重复没了。
///
/// 归一成未绑，三条一起消失（零延迟 + 保留系统连发 + 快速连按不再被误判成双击）。
/// 只动方向四键：`ok` 在共享模式被设备级禁用表吞掉，绑 Return **不是**原生等价，不能一并清。
pub fn normalize(mut cfg: ButtonConfig) -> ButtonConfig {
    for b in [
        RemoteButton::Up,
        RemoteButton::Down,
        RemoteButton::Left,
        RemoteButton::Right,
    ] {
        let code = match native_event(b) {
            NativeEvent::Keyboard(code) => code,
            _ => continue,
        };
        // 无修饰、无 fn 位、不分左右侧，且键码就是该键自己的原生键码 → 与原生透传完全等价。
        let same_as_native = matches!(
            cfg.slot_action(b, Slot::Click),
            Some(Action::Shortcut { shortcut })
                if shortcut.mods == 0
                    && !shortcut.r#fn
                    && shortcut.side.is_none()
                    && shortcut.key == code
        );
        if same_as_native {
            cfg.set_slot_action(b, Slot::Click, None);
        }
    }
    cfg
}

/// 键位配置目录（data_dir 下）。外部文件形态 = 导入导出直接拷文件。
pub fn config_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("remote")
}

/// 按档案分仓的键位配置文件：`remote/buttons.<profile_id>.json`
/// （不同设备键数不同，共用一张表必然互相踩——计划 2026-11-19 §五）。
pub fn config_path(data_dir: &Path, profile_id: &str) -> PathBuf {
    config_dir(data_dir).join(format!("buttons.{profile_id}.json"))
}

/// 快速面板专用键位表文件：`remote/buttons.panel.<profile_id>.json`。
/// 与日常表同结构、同分仓，独立文件——面板开着时遥控器改用这张（计划
/// `2026-11-26-快速面板专用遥控器映射表切换`）。**文件缺失 = 空表**：不迁移旧
/// settings、不回落默认映射，空表即所有键原生透传，面板现有导航手感不变。
pub fn panel_config_path(data_dir: &Path, profile_id: &str) -> PathBuf {
    config_dir(data_dir).join(format!("buttons.panel.{profile_id}.json"))
}

/// 读取：文件为准；文件缺失/坏档先从旧 settings 表迁移并落盘物化，都没有回默认。
/// 坏文件不静默吞：回退后下一次保存会覆盖它。
pub fn load(data_dir: &Path, store: &Store, profile_id: &str) -> ButtonConfig {
    if let Ok(raw) = std::fs::read_to_string(config_path(data_dir, profile_id)) {
        if let Ok(cfg) = serde_json::from_str::<ButtonConfig>(&raw) {
            return normalize(cfg);
        }
    }
    // 档案拆分（如小米 1 代从 rc00x 里分出去）后，用户已配的键位还躺在旧仓里：
    // 沿用 `config_from` 指的那份并就地物化到新仓，别让拆档案变成清用户配置。
    if let Some(from) = crate::services::remote::profile::by_id(profile_id).config_from {
        if let Ok(raw) = std::fs::read_to_string(config_path(data_dir, from)) {
            if let Ok(cfg) = serde_json::from_str::<ButtonConfig>(&raw) {
                let cfg = normalize(cfg);
                let _ = save(data_dir, profile_id, &cfg);
                return cfg;
            }
        }
    }
    let migrated = store
        .get_setting(CONFIG_KEY)
        .ok()
        .flatten()
        .map(|raw| parse(&raw))
        .unwrap_or_else(|| default_mapping(profile_id));
    let _ = save(data_dir, profile_id, &migrated);
    migrated
}

/// 保存：pretty JSON 写文件（人可读、可 diff、可手改）。
pub fn save(data_dir: &Path, profile_id: &str, config: &ButtonConfig) -> Result<(), String> {
    let path = config_path(data_dir, profile_id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("配置目录创建失败：{e}"))?;
    }
    let raw = serde_json::to_string_pretty(config).map_err(|e| format!("配置序列化失败：{e}"))?;
    std::fs::write(&path, raw).map_err(|e| format!("配置文件写入失败：{e}"))
}

/// 读面板表：文件在就读并归一；缺失/坏档 = 空表（全原生透传），不迁移、不回落默认。
/// 坏档不静默？这里与日常表不同——面板表是「可选的另一套」，坏档退回空表比拿一份
/// 错映射去接管遥控器安全；下一次保存会覆盖它。
pub fn load_panel(data_dir: &Path, profile_id: &str) -> ButtonConfig {
    std::fs::read_to_string(panel_config_path(data_dir, profile_id))
        .ok()
        .and_then(|raw| serde_json::from_str::<ButtonConfig>(&raw).ok())
        .map(normalize)
        .unwrap_or_default()
}

/// 写面板表（整体替换，写独立文件）。
pub fn save_panel(data_dir: &Path, profile_id: &str, config: &ButtonConfig) -> Result<(), String> {
    let path = panel_config_path(data_dir, profile_id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("配置目录创建失败：{e}"))?;
    }
    let raw = serde_json::to_string_pretty(config).map_err(|e| format!("配置序列化失败：{e}"))?;
    std::fs::write(&path, raw).map_err(|e| format!("配置文件写入失败：{e}"))
}

/// 预存方案文件（用户 2026-11-20 拍板）：`remote/schemes.<profile>.json`，
/// 命名方案 → 整套映射；UI 下拉切换，切换由前端二次确认后走 apply。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SchemeFile {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub schemes: BTreeMap<String, ButtonConfig>,
    /// App 绑定（C5）：`bundle_id → 方案名`。前台 App 命中即热加载该方案，
    /// **精确相等**才算命中（SayAll 公开口径，不做通配/前缀）。
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
}

pub fn schemes_path(data_dir: &Path, profile_id: &str) -> PathBuf {
    config_dir(data_dir).join(format!("schemes.{profile_id}.json"))
}

pub(crate) fn load_schemes(data_dir: &Path, profile_id: &str) -> SchemeFile {
    let mut file = std::fs::read_to_string(schemes_path(data_dir, profile_id))
        .ok()
        .and_then(|raw| serde_json::from_str::<SchemeFile>(&raw).ok())
        .unwrap_or_default();
    // 方案里存的是整份键位表，锁口径之前的老方案同样挂着遗留绑定——
    // 切过去就会「按一次动两下」，所以方案也要逐个归一。
    for cfg in file.schemes.values_mut() {
        *cfg = normalize(std::mem::take(cfg));
    }
    file
}

/// 方案名列表（下拉内容，字典序）。
pub fn scheme_names(data_dir: &Path, profile_id: &str) -> Vec<String> {
    load_schemes(data_dir, profile_id)
        .schemes
        .keys()
        .cloned()
        .collect()
}

/// 存方案（同名覆盖——「存当前为新方案」就是拿当前映射盖名字）。
pub fn save_scheme(
    data_dir: &Path,
    profile_id: &str,
    name: &str,
    config: &ButtonConfig,
) -> Result<(), String> {
    let mut file = load_schemes(data_dir, profile_id);
    file.schemes.insert(name.to_string(), config.clone());
    write_schemes(data_dir, profile_id, &file)
}

pub(crate) fn write_schemes(
    data_dir: &Path,
    profile_id: &str,
    file: &SchemeFile,
) -> Result<(), String> {
    let path = schemes_path(data_dir, profile_id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("配置目录创建失败：{e}"))?;
    }
    let raw = serde_json::to_string_pretty(file).map_err(|e| format!("方案序列化失败：{e}"))?;
    std::fs::write(&path, raw).map_err(|e| format!("方案文件写入失败：{e}"))
}

/// 改名方案：旧名不存在或新名已占用都报错（不静默覆盖别的方案）。
pub fn rename_scheme(
    data_dir: &Path,
    profile_id: &str,
    old: &str,
    new: &str,
) -> Result<(), String> {
    let mut file = load_schemes(data_dir, profile_id);
    let cfg = file
        .schemes
        .remove(old)
        .ok_or_else(|| format!("方案「{old}」不存在"))?;
    if file.schemes.contains_key(new) {
        return Err(format!("方案「{new}」已存在"));
    }
    file.schemes.insert(new.to_string(), cfg);
    // 跟着改名的是 App 绑定（C5）：不然改完名，绑这个方案的 App 就静默掉回默认映射。
    for scheme in file.bindings.values_mut() {
        if scheme == old {
            *scheme = new.to_string();
        }
    }
    write_schemes(data_dir, profile_id, &file)
}

/// 删除方案：不存在报错。跟着清掉指向它的 App 绑定（C5）——
/// 不然删完名，绑这个方案的 App 前台一切就静默拿不到映射。
pub fn delete_scheme(data_dir: &Path, profile_id: &str, name: &str) -> Result<(), String> {
    let mut file = load_schemes(data_dir, profile_id);
    if file.schemes.remove(name).is_none() {
        return Err(format!("方案「{name}」不存在"));
    }
    file.bindings.retain(|_, scheme| scheme != name);
    write_schemes(data_dir, profile_id, &file)
}

/// 取方案（apply 用）；不存在回 None，由命令层报错。
pub fn get_scheme(data_dir: &Path, profile_id: &str, name: &str) -> Option<ButtonConfig> {
    load_schemes(data_dir, profile_id).schemes.remove(name)
}

// ------------------------------------------------------------------ App 绑定（C5）

/// 当前 App 绑定表（`bundle_id → 方案名`）。
pub fn app_bindings(data_dir: &Path, profile_id: &str) -> BTreeMap<String, String> {
    load_schemes(data_dir, profile_id).bindings
}

/// 前台 App 该用哪个方案（C5）：精确匹配，没绑或前台为空回 None（= 用默认映射）。
/// 纯函数，匹配规则只在这一处，单测钉死「前缀不算命中」。
pub fn match_scheme(
    bindings: &BTreeMap<String, String>,
    bundle_id: Option<&str>,
) -> Option<String> {
    let id = bundle_id.map(str::trim).filter(|s| !s.is_empty())?;
    bindings.get(id).cloned()
}

/// 绑/解一个 App 到方案：`scheme = None` 解绑；绑到不存在的方案直接报错（不存悬空绑定）。
pub fn set_app_binding(
    data_dir: &Path,
    profile_id: &str,
    bundle_id: &str,
    scheme: Option<&str>,
) -> Result<(), String> {
    let bundle_id = bundle_id.trim();
    if bundle_id.is_empty() {
        return Err("未取到前台 App（切到一个 App 窗口再绑）".to_string());
    }
    let mut file = load_schemes(data_dir, profile_id);
    match scheme {
        None => {
            file.bindings.remove(bundle_id);
        }
        Some(s) => {
            let s = s.trim();
            if s.is_empty() {
                return Err("方案名称不能为空".to_string());
            }
            if !file.schemes.contains_key(s) {
                return Err(format!("方案「{s}」不存在，先存方案再绑"));
            }
            file.bindings.insert(bundle_id.to_string(), s.to_string());
        }
    }
    write_schemes(data_dir, profile_id, &file)
}

#[cfg(test)]
mod tests {
    /// 加载口必须归一锁定键：文件里挂着遗留「等价快捷键」就会按一次动两下
    /// （设备原生一份 + 手势注入一份，用户 2026-11-23 实报）。
    use super::*;

    fn sc(mods: u8, key: u16) -> Shortcut {
        Shortcut {
            mods,
            key,
            ..Default::default()
        }
    }

    /// 槽动作快捷构造（C2 起槽里放的是 Action，不是裸 Shortcut）。
    fn act(mods: u8, key: u16) -> Action {
        Action::Shortcut {
            shortcut: sc(mods, key),
        }
    }

    #[test]
    fn defaults_are_native_passthrough() {
        let cfg = ButtonConfig::all_default();
        for b in RemoteButton::ALL {
            assert_eq!(cfg.action(b), ButtonAction::Native, "{b:?}");
            assert!(!cfg.allows_rapid(b), "{b:?}");
            for slot in Slot::ALL {
                assert_eq!(cfg.slot_action(b, slot), None, "{b:?} {slot:?}");
            }
        }
        assert!(!cfg.has_non_native());
        assert_eq!(cfg.find_conflict(), None);
    }

    /// 方向键单击槽绑成「它自己且无修饰」必须归一成未绑（用户 2026-11-25 实报
    /// 快速面板上下键不跟手：绑了槽就进手势判定，要等一个双击窗，共享模式还会双发）。
    #[test]
    fn normalize_drops_navigation_click_bound_to_itself() {
        let mut cfg = ButtonConfig::default();
        // 126/125 = ↑/↓ 的 macOS 虚拟键码（与 buttons::native_event 同源）。
        cfg.set_slot_action(RemoteButton::Up, Slot::Click, Some(act(0, 126)));
        cfg.set_slot_action(RemoteButton::Down, Slot::Click, Some(act(0, 125)));
        // 带修饰的不是原生等价（⌥↑ 是真动作），不许清。
        cfg.set_slot_action(RemoteButton::Left, Slot::Click, Some(act(4, 123)));
        // ok 在共享模式被设备级禁用表吞掉，绑 Return 不是原生等价，也不许清。
        cfg.set_slot_action(RemoteButton::Ok, Slot::Click, Some(act(0, 36)));
        let out = normalize(cfg);
        assert_eq!(out.slot_action(RemoteButton::Up, Slot::Click), None);
        assert_eq!(out.slot_action(RemoteButton::Down, Slot::Click), None);
        assert_eq!(
            out.slot_action(RemoteButton::Left, Slot::Click).cloned(),
            Some(act(4, 123))
        );
        assert_eq!(
            out.slot_action(RemoteButton::Ok, Slot::Click).cloned(),
            Some(act(0, 36))
        );
    }

    #[test]
    fn set_and_query() {
        let mut cfg = ButtonConfig::default();
        cfg.set_action(RemoteButton::Back, ButtonAction::Disabled);
        cfg.set_rapid(RemoteButton::Up, true);
        cfg.set_slot_action(RemoteButton::Home, Slot::Click, Some(act(4, 0)));
        assert_eq!(cfg.action(RemoteButton::Back), ButtonAction::Disabled);
        assert_eq!(cfg.action(RemoteButton::Home), ButtonAction::Native);
        assert!(cfg.has_non_native());
        assert!(cfg.allows_rapid(RemoteButton::Up));
        assert!(!cfg.allows_rapid(RemoteButton::Down));
        assert_eq!(
            cfg.slot_action(RemoteButton::Home, Slot::Click).cloned(),
            Some(act(4, 0))
        );
        assert_eq!(cfg.slot_action(RemoteButton::Home, Slot::Double), None);
    }

    #[test]
    fn serde_shape_is_stable() {
        let mut cfg = ButtonConfig::default();
        cfg.set_action(RemoteButton::VolumeUp, ButtonAction::Disabled);
        cfg.set_rapid(RemoteButton::Back, true);
        cfg.set_slot_action(RemoteButton::Power, Slot::Long, Some(act(0, 53))); // 裸 Esc
        let raw = serde_json::to_string(&cfg).unwrap();
        assert!(raw.contains("\"volume_up\""));
        assert!(raw.contains("\"enabled\":false"));
        let back: ButtonConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn shortcut_serde_stays_compatible_with_old_configs() {
        // 旧配置只有 mods/key：读入后 fn=false、side=None；干净组合序列化不回这两字段。
        let old: Shortcut = serde_json::from_str(r#"{"mods":8,"key":5}"#).unwrap();
        assert_eq!(
            old,
            Shortcut {
                mods: 8,
                key: 5,
                r#fn: false,
                side: None
            }
        );
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"mods":8,"key":5}"#
        );
        // 新字段读回：fn 位与左右侧。
        let s: Shortcut =
            serde_json::from_str(r#"{"mods":8,"key":5,"fn":true,"side":"right"}"#).unwrap();
        assert!(s.r#fn);
        assert_eq!(s.side, Some(Side::Right));
    }

    #[test]
    fn macro_action_is_a_reference_not_a_copy() {
        // C3：槽里存的是宏 id 引用（步骤住宏库），所以它不参与快捷键冲突、空 id 归未绑。
        let m = Action::Macro {
            id: "m1".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"type":"macro","id":"m1"}"#
        );
        assert_eq!(m.shortcut(), None);
        assert!(!m.is_blank());
        assert!(Action::Macro { id: "  ".into() }.is_blank());
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Home, Slot::Click, Some(m.clone()));
        cfg.set_slot_action(RemoteButton::Back, Slot::Click, Some(m));
        assert_eq!(cfg.find_conflict(), None, "同一宏绑两键是合法配置");
        assert!(cfg.has_bindings(RemoteButton::Home));
        let raw = serde_json::to_string(&cfg).unwrap();
        assert_eq!(serde_json::from_str::<ButtonConfig>(&raw).unwrap(), cfg);
    }

    #[test]
    fn mod_key_table_matches_plan() {
        // 9 键逐条钉死（计划 §三 C1）：侧与 mods 位错一条，注入就注错键。
        assert_eq!(mod_key(54), Some((8, Some(Side::Right)))); // 右⌘
        assert_eq!(mod_key(55), Some((8, Some(Side::Left)))); // 左⌘
        assert_eq!(mod_key(56), Some((2, Some(Side::Left)))); // 左⇧
        assert_eq!(mod_key(60), Some((2, Some(Side::Right)))); // 右⇧
        assert_eq!(mod_key(58), Some((4, Some(Side::Left)))); // 左⌥
        assert_eq!(mod_key(61), Some((4, Some(Side::Right)))); // 右⌥
        assert_eq!(mod_key(59), Some((1, Some(Side::Left)))); // 左⌃
        assert_eq!(mod_key(62), Some((1, Some(Side::Right)))); // 右⌃
        assert_eq!(mod_key(63), Some((0, None))); // Fn
        assert_eq!(mod_key(57), None); // CapsLock 不在内
        assert_eq!(mod_key(5), None); // 普通键
                                      // 单修饰键判定：mods=0 且键码在表内。
        assert!(Shortcut {
            mods: 0,
            key: 54,
            r#fn: false,
            side: None
        }
        .is_mod_key());
        assert!(!Shortcut {
            mods: 8,
            key: 54,
            r#fn: false,
            side: None
        }
        .is_mod_key());
        assert!(!Shortcut {
            mods: 0,
            key: 5,
            r#fn: false,
            side: None
        }
        .is_mod_key());
    }

    #[test]
    fn sided_shortcuts_are_distinct_for_conflict() {
        // 左⌘ 与右⌘ 是两个不同的组合（验收：绑不同动作可分辨）。
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(
            RemoteButton::Home,
            Slot::Click,
            Some(Action::Shortcut {
                shortcut: Shortcut {
                    mods: 8,
                    key: 5,
                    r#fn: false,
                    side: Some(Side::Left),
                },
            }),
        );
        cfg.set_slot_action(
            RemoteButton::Menu,
            Slot::Click,
            Some(Action::Shortcut {
                shortcut: Shortcut {
                    mods: 8,
                    key: 5,
                    r#fn: false,
                    side: Some(Side::Right),
                },
            }),
        );
        assert_eq!(cfg.find_conflict(), None);
        // 同侧同键才冲突。
        cfg.set_slot_action(
            RemoteButton::Tv,
            Slot::Click,
            Some(Action::Shortcut {
                shortcut: Shortcut {
                    mods: 8,
                    key: 5,
                    r#fn: false,
                    side: Some(Side::Left),
                },
            }),
        );
        assert!(cfg.find_conflict().is_some());
    }

    #[test]
    fn legacy_bare_shortcut_slot_migrates_to_action() {
        // C2 契约迁移（待用户确认 §5 拍板 A）：旧档槽值是裸 {"mods":…,"key":…}，
        // 读入必须变成 Action::Shortcut，一条映射不丢。
        let raw = r#"{"keys":{"back":{"click":{"mods":8,"key":5},"long":{"mods":0,"key":53,"fn":true}}}}"#;
        let cfg: ButtonConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(
            cfg.slot_action(RemoteButton::Back, Slot::Click).cloned(),
            Some(act(8, 5))
        );
        assert_eq!(
            cfg.slot_action(RemoteButton::Back, Slot::Long).cloned(),
            Some(Action::Shortcut {
                shortcut: Shortcut {
                    mods: 0,
                    key: 53,
                    r#fn: true,
                    side: None
                }
            })
        );
        assert_eq!(cfg.slot_action(RemoteButton::Back, Slot::Double), None);
        // 保存一律新形状：再序列化带 type 标签，读回等价。
        let raw2 = serde_json::to_string(&cfg).unwrap();
        assert!(
            raw2.contains(r#""type":"shortcut""#),
            "新形状应带 type 标签：{raw2}"
        );
        assert_eq!(parse(&raw2), cfg);
    }

    #[test]
    fn every_action_type_roundtrips() {
        let cases = [
            act(3, 5),
            Action::OpenApp {
                bundle_id: "com.apple.Terminal".to_string(),
                args: String::new(),
            },
            Action::AgentCli {
                app: "codex".to_string(),
                args: "--yolo".to_string(),
                cwd: "/tmp".to_string(),
            },
            Action::Text {
                content: "你好，世界".to_string(),
            },
            Action::FocusInput,
            Action::RunCommand {
                command: "open -a Calculator".to_string(),
            },
        ];
        for a in cases {
            let raw = serde_json::to_string(&a).unwrap();
            let back: Action = serde_json::from_str(&raw).unwrap();
            assert_eq!(back, a, "{raw}");
        }
    }

    #[test]
    fn blank_actions_normalize_to_unbound() {
        // 空文本 / 空命令 / 空 bundle id 不许存成动作（按了没反应）。
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(
            RemoteButton::Home,
            Slot::Click,
            Some(Action::Text {
                content: "   ".to_string(),
            }),
        );
        cfg.set_slot_action(
            RemoteButton::Home,
            Slot::Double,
            Some(Action::RunCommand {
                command: "".to_string(),
            }),
        );
        cfg.set_slot_action(
            RemoteButton::Home,
            Slot::Long,
            Some(Action::OpenApp {
                bundle_id: "  ".to_string(),
                args: String::new(),
            }),
        );
        // 空 app 名的代理 CLI 同样按未绑处理（按了没反应的不许存）。
        cfg.set_slot_action(
            RemoteButton::Tv,
            Slot::Click,
            Some(Action::AgentCli {
                app: "   ".to_string(),
                args: "--yolo".to_string(),
                cwd: String::new(),
            }),
        );
        for slot in Slot::ALL {
            assert_eq!(cfg.slot_action(RemoteButton::Home, slot), None, "{slot:?}");
        }
        assert!(!cfg.has_bindings(RemoteButton::Home));
    }

    #[test]
    fn non_shortcut_actions_never_conflict() {
        // 冲突只对快捷键（计划 §三）：两个键都绑「打开同一个 App」是合法配置。
        let open = || Action::OpenApp {
            bundle_id: "com.doubao.app".to_string(),
            args: String::new(),
        };
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Home, Slot::Click, Some(open()));
        cfg.set_slot_action(RemoteButton::Menu, Slot::Click, Some(open()));
        cfg.set_slot_action(
            RemoteButton::Tv,
            Slot::Click,
            Some(Action::RunCommand {
                command: "echo hi".to_string(),
            }),
        );
        cfg.set_slot_action(
            RemoteButton::Power,
            Slot::Click,
            Some(Action::RunCommand {
                command: "echo hi".to_string(),
            }),
        );
        assert_eq!(cfg.find_conflict(), None);
        // 快捷键照样和快捷键撞（同一份表里两类动作混着放）。
        cfg.set_slot_action(RemoteButton::Home, Slot::Double, Some(act(8, 5)));
        cfg.set_slot_action(RemoteButton::Menu, Slot::Click, Some(act(8, 5)));
        assert!(cfg.find_conflict().is_some());
    }

    #[test]
    fn old_shape_migrates() {
        let raw = r#"{"bindings":{"back":"disabled","ok":"native"},"rapid":{"back":true}}"#;
        let cfg = parse(raw);
        assert_eq!(cfg.action(RemoteButton::Back), ButtonAction::Disabled);
        assert_eq!(cfg.action(RemoteButton::Ok), ButtonAction::Native);
        assert!(cfg.allows_rapid(RemoteButton::Back));
        assert!(!cfg.allows_rapid(RemoteButton::Ok));
        // 迁移后三槽全未绑（原生）。
        for slot in Slot::ALL {
            assert_eq!(cfg.slot_action(RemoteButton::Back, slot), None);
        }
    }

    #[test]
    fn new_shape_is_not_mistaken_for_old() {
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Up, Slot::Double, Some(act(2, 48)));
        let raw = serde_json::to_string(&cfg).unwrap();
        assert_eq!(parse(&raw), cfg);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        // 前向兼容：未来版本写过的键名不能炸老版本读取。
        let raw = r#"{"keys":{"no_such_key":{"enabled":false,"click":null,"double":null,"long":null,"rapid":true}}}"#;
        let cfg: ButtonConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.action(RemoteButton::Ok), ButtonAction::Native);
        assert_eq!(cfg.keys.len(), 1); // 未知键原样存着但不影响已知键
    }

    #[test]
    fn conflict_is_global_across_keys_and_slots() {
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Home, Slot::Click, Some(act(4, 0)));
        cfg.set_slot_action(RemoteButton::Menu, Slot::Double, Some(act(4, 0)));
        let c = cfg.find_conflict().expect("应报冲突");
        assert_eq!(c.shortcut, sc(4, 0));
        // BTreeMap 字典序：home 在 menu 前。
        assert_eq!(c.first.0, "home");
        assert_eq!(c.second.0, "menu");
    }

    #[test]
    fn conflict_within_same_key_across_slots() {
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Home, Slot::Click, Some(act(0, 53)));
        cfg.set_slot_action(RemoteButton::Home, Slot::Long, Some(act(0, 53)));
        assert!(cfg.find_conflict().is_some());
    }

    #[test]
    fn distinct_shortcuts_have_no_conflict() {
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Home, Slot::Click, Some(act(0, 53)));
        cfg.set_slot_action(RemoteButton::Menu, Slot::Click, Some(act(0, 52)));
        cfg.set_slot_action(RemoteButton::Home, Slot::Double, Some(act(1, 53)));
        assert_eq!(cfg.find_conflict(), None);
    }

    #[test]
    fn has_bindings_detects_any_slot() {
        let mut cfg = ButtonConfig::default();
        assert!(!cfg.has_bindings(RemoteButton::Up));
        cfg.set_slot_action(RemoteButton::Up, Slot::Long, Some(act(0, 53)));
        assert!(cfg.has_bindings(RemoteButton::Up));
        assert!(!cfg.has_bindings(RemoteButton::Down));
    }

    #[test]
    fn file_roundtrip_is_pretty_and_per_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db")).unwrap();
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Ok, Slot::Click, Some(act(8, 5)));
        save(tmp.path(), "xiaomi_rc00x", &cfg).unwrap();
        let raw = std::fs::read_to_string(config_path(tmp.path(), "xiaomi_rc00x")).unwrap();
        assert!(raw.contains('\n'), "pretty JSON——人可读可 diff");
        assert!(raw.contains("\"format_version\": 1"));
        assert_eq!(load(tmp.path(), &store, "xiaomi_rc00x"), cfg);
        // 分仓：别的档案没有这份文件，也不会读到 xiaomi 的绑定。
        assert_eq!(load(tmp.path(), &store, "g9n9n"), ButtonConfig::default());
    }

    #[test]
    fn panel_config_is_independent_and_empty_when_missing() {
        // 快速面板专用表：缺文件 = 空表（全原生透传），不迁移、不回落默认；
        // 与日常表独立存，写一张不碰另一张（计划 2026-11-26 映射表切换）。
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db")).unwrap();
        let mut daily = ButtonConfig::default();
        daily.set_slot_action(RemoteButton::Ok, Slot::Click, Some(act(8, 5)));
        save(tmp.path(), "xiaomi_rc00x", &daily).unwrap();
        // 面板表没有文件 → 空表。
        assert_eq!(
            load_panel(tmp.path(), "xiaomi_rc00x"),
            ButtonConfig::default()
        );
        // 写面板表 → 只落独立文件，日常表不变。
        let mut panel = ButtonConfig::default();
        panel.set_slot_action(RemoteButton::Back, Slot::Click, Some(act(0, 51)));
        save_panel(tmp.path(), "xiaomi_rc00x", &panel).unwrap();
        assert_eq!(load_panel(tmp.path(), "xiaomi_rc00x"), panel);
        assert_eq!(load(tmp.path(), &store, "xiaomi_rc00x"), daily);
        // 坏档 → 空表（不拿错映射去接管遥控器）。
        std::fs::write(panel_config_path(tmp.path(), "xiaomi_rc00x"), "{bad").unwrap();
        assert_eq!(
            load_panel(tmp.path(), "xiaomi_rc00x"),
            ButtonConfig::default()
        );
    }

    #[test]
    fn split_profile_inherits_the_source_config_bucket() {
        // 小米 1 代从 rc00x 里分出去后，用户已配的键位还躺在旧仓：
        // 新仓没有文件时沿用 config_from，并就地物化，别让用户重配一遍。
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db")).unwrap();
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Back, Slot::Click, Some(act(0, 51)));
        save(tmp.path(), "xiaomi_rc00x", &cfg).unwrap();
        assert_eq!(load(tmp.path(), &store, "xiaomi_gen1"), cfg);
        assert!(
            config_path(tmp.path(), "xiaomi_gen1").exists(),
            "沿用后物化到新仓"
        );
        // 默认映射也跟键表走：1 代与 rc00x 同键表，不该一上来全是空映射。
        assert_ne!(default_mapping("xiaomi_gen1"), ButtonConfig::default());
    }

    #[test]
    fn legacy_settings_migrates_to_file_once() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db")).unwrap();
        store
            .set_setting(CONFIG_KEY, r#"{"bindings":{"back":"disabled"}}"#)
            .unwrap();
        let cfg = load(tmp.path(), &store, "xiaomi_rc00x");
        assert_eq!(cfg.action(RemoteButton::Back), ButtonAction::Disabled);
        // 已物化落盘：此后文件为准（旧键即使还在也不再看它）。
        store.set_setting(CONFIG_KEY, r#"{"bindings":{}}"#).unwrap();
        assert_eq!(
            load(tmp.path(), &store, "xiaomi_rc00x").action(RemoteButton::Back),
            ButtonAction::Disabled
        );
    }

    #[test]
    fn seed_mapping_is_conflict_free() {
        let cfg = default_mapping("xiaomi_rc00x");
        // SayAll 播种必须过我们自己的全局唯一校验，否则首屏就是冲突态。
        assert_eq!(cfg.find_conflict(), None);
        assert!(cfg.has_bindings(RemoteButton::Power));
        // 别的档案不套用小米键表。
        assert_eq!(default_mapping("g9n9n"), ButtonConfig::default());
    }

    #[test]
    fn schemes_roundtrip_and_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(scheme_names(tmp.path(), "xiaomi_rc00x").is_empty());
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Ok, Slot::Click, Some(act(8, 5)));
        save_scheme(tmp.path(), "xiaomi_rc00x", "写代码", &cfg).unwrap();
        save_scheme(
            tmp.path(),
            "xiaomi_rc00x",
            "看视频",
            &ButtonConfig::default(),
        )
        .unwrap();
        // BTreeMap 按 UTF-8 字节序排（非拼音序）：写(E5) 在 看(E7) 前。
        assert_eq!(
            scheme_names(tmp.path(), "xiaomi_rc00x"),
            vec!["写代码", "看视频"]
        );
        assert_eq!(get_scheme(tmp.path(), "xiaomi_rc00x", "写代码"), Some(cfg));
        assert_eq!(get_scheme(tmp.path(), "xiaomi_rc00x", "没有这个"), None);
        // 同名覆盖：再存一次以最后一次为准。
        save_scheme(
            tmp.path(),
            "xiaomi_rc00x",
            "写代码",
            &ButtonConfig::default(),
        )
        .unwrap();
        assert_eq!(
            get_scheme(tmp.path(), "xiaomi_rc00x", "写代码"),
            Some(ButtonConfig::default())
        );
    }

    #[test]
    fn app_binding_is_exact_match_only() {
        let tmp = tempfile::tempdir().unwrap();
        let p = "xiaomi_rc00x";
        save_scheme(tmp.path(), p, "豆包", &ButtonConfig::default()).unwrap();
        set_app_binding(tmp.path(), p, "com.doubao.chat", Some("豆包")).unwrap();
        assert_eq!(
            app_bindings(tmp.path(), p)
                .get("com.doubao.chat")
                .map(String::as_str),
            Some("豆包")
        );
        // 精确相等才算命中：前缀、大小写、空前台一律不匹配（计划 §三：不做通配）。
        assert_eq!(
            match_scheme(&app_bindings(tmp.path(), p), Some("com.doubao.chat")),
            Some("豆包".to_string())
        );
        assert_eq!(
            match_scheme(&app_bindings(tmp.path(), p), Some("com.doubao.chat.pro")),
            None
        );
        assert_eq!(
            match_scheme(&app_bindings(tmp.path(), p), Some("COM.DOUBAO.CHAT")),
            None
        );
        assert_eq!(match_scheme(&app_bindings(tmp.path(), p), Some("  ")), None);
        assert_eq!(match_scheme(&app_bindings(tmp.path(), p), None), None);
        // 解绑 + 绑到不存在的方案（不许存悬空绑定）。
        set_app_binding(tmp.path(), p, "com.doubao.chat", None).unwrap();
        assert!(app_bindings(tmp.path(), p).is_empty());
        assert!(set_app_binding(tmp.path(), p, "com.x", Some("没这个方案")).is_err());
        assert!(set_app_binding(tmp.path(), p, "  ", Some("豆包")).is_err());
    }

    #[test]
    fn scheme_rename_follows_app_bindings() {
        // 改名后绑这个 App 的方案必须跟着走，否则切前台静默掉回默认映射。
        let tmp = tempfile::tempdir().unwrap();
        let p = "xiaomi_rc00x";
        save_scheme(tmp.path(), p, "旧名", &ButtonConfig::default()).unwrap();
        set_app_binding(tmp.path(), p, "com.doubao.chat", Some("旧名")).unwrap();
        rename_scheme(tmp.path(), p, "旧名", "新名").unwrap();
        assert_eq!(
            app_bindings(tmp.path(), p)
                .get("com.doubao.chat")
                .map(String::as_str),
            Some("新名")
        );
        assert_eq!(
            match_scheme(&app_bindings(tmp.path(), p), Some("com.doubao.chat")),
            Some("新名".to_string())
        );
    }

    #[test]
    fn scheme_delete_clears_bindings() {
        // 删方案必须跟着清绑定：不然绑这个方案的 App 前台一切，热加载指向一个没了的方案。
        let tmp = tempfile::tempdir().unwrap();
        let p = "xiaomi_rc00x";
        save_scheme(tmp.path(), p, "删我", &ButtonConfig::default()).unwrap();
        save_scheme(tmp.path(), p, "留我", &ButtonConfig::default()).unwrap();
        set_app_binding(tmp.path(), p, "com.doubao.chat", Some("删我")).unwrap();
        set_app_binding(tmp.path(), p, "com.x", Some("留我")).unwrap();
        delete_scheme(tmp.path(), p, "删我").unwrap();
        assert_eq!(scheme_names(tmp.path(), p), vec!["留我"]);
        assert_eq!(app_bindings(tmp.path(), p).get("com.doubao.chat"), None);
        assert_eq!(
            app_bindings(tmp.path(), p).get("com.x").map(String::as_str),
            Some("留我")
        );
        // 不存在的方案删除要报错，不许静默成功。
        assert!(delete_scheme(tmp.path(), p, "删我").is_err());
    }

    #[test]
    fn scheme_rename_moves_config_and_guard() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = ButtonConfig::default();
        cfg.set_slot_action(RemoteButton::Ok, Slot::Click, Some(act(8, 5)));
        save_scheme(tmp.path(), "xiaomi_rc00x", "旧名", &cfg).unwrap();
        rename_scheme(tmp.path(), "xiaomi_rc00x", "旧名", "新名").unwrap();
        assert_eq!(scheme_names(tmp.path(), "xiaomi_rc00x"), vec!["新名"]);
        assert_eq!(
            get_scheme(tmp.path(), "xiaomi_rc00x", "新名"),
            Some(cfg.clone())
        );
        // 旧名没了、新名占用都报错，且不许把原有方案弄丢。
        assert!(rename_scheme(tmp.path(), "xiaomi_rc00x", "旧名", "别的").is_err());
        save_scheme(tmp.path(), "xiaomi_rc00x", "别的", &ButtonConfig::default()).unwrap();
        assert!(rename_scheme(tmp.path(), "xiaomi_rc00x", "新名", "别的").is_err());
        assert_eq!(get_scheme(tmp.path(), "xiaomi_rc00x", "新名"), Some(cfg));
    }
}
