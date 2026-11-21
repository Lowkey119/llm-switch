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

use crate::services::remote::buttons::RemoteButton;
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

/// 快捷键组合：mods 位图（bit0 ctrl bit1 shift bit2 alt bit3 meta）+ CGEvent 虚拟键码。
/// 裸单键 = `mods = 0`。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
pub struct Shortcut {
    pub mods: u8,
    pub key: u16,
}

/// 逐键配置：使能位 + 三槽 + 连按。缺键 = 全默认（启用、未绑、不连按）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyConfig {
    /// false = 禁用（旧形状里的 "disabled"）。
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub click: Option<Shortcut>,
    #[serde(default)]
    pub double: Option<Shortcut>,
    #[serde(default)]
    pub long: Option<Shortcut>,
    /// 允许连续快速按（只作用于原生透传的连发语义，与三槽手势判定互不干扰）。
    #[serde(default)]
    pub rapid: bool,
}

fn enabled_by_default() -> bool {
    true
}

// 手写 Default：派生会把 enabled 置 false（=禁用），与「缺键=启用」口径相反，
// set_rapid/set_shortcut 走 or_default 时会把键误禁（S-A 单测抓到）。
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
    pub fn slot(&self, slot: Slot) -> Option<Shortcut> {
        match slot {
            Slot::Click => self.click,
            Slot::Double => self.double,
            Slot::Long => self.long,
        }
    }

    pub fn set_slot(&mut self, slot: Slot, shortcut: Option<Shortcut>) {
        match slot {
            Slot::Click => self.click = shortcut,
            Slot::Double => self.double = shortcut,
            Slot::Long => self.long = shortcut,
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

fn default_format_version() -> u32 {
    1
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

    /// 该键某槽绑定的快捷键；None = 未绑（原生）。
    pub fn shortcut(&self, button: RemoteButton, slot: Slot) -> Option<Shortcut> {
        self.keys.get(button.id()).and_then(|k| k.slot(slot))
    }

    pub fn set_shortcut(&mut self, button: RemoteButton, slot: Slot, shortcut: Option<Shortcut>) {
        self.entry(button).set_slot(slot, shortcut);
    }

    /// 该键三槽是否有任一绑定（独占模式「手势驱动」判据：有绑定则按下不注原生、
    /// 等手势执行——计划 §四 替换语义；shared 模式不看它，原生照常透传）。
    pub fn has_shortcuts(&self, button: RemoteButton) -> bool {
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
    pub fn find_conflict(&self) -> Option<Conflict> {
        let mut seen: HashMap<Shortcut, (String, Slot)> = HashMap::new();
        for (id, kc) in &self.keys {
            for slot in Slot::ALL {
                let Some(sc) = kc.slot(slot) else { continue };
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
                cfg.set_shortcut(b, slot, Some(Shortcut { mods, key }));
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
    if let Ok(old) = serde_json::from_str::<OldConfig>(raw) {
        return migrate(old);
    }
    serde_json::from_str::<ButtonConfig>(raw).unwrap_or_default()
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

/// 读取：文件为准；文件缺失/坏档先从旧 settings 表迁移并落盘物化，都没有回默认。
/// 坏文件不静默吞：回退后下一次保存会覆盖它。
pub fn load(data_dir: &Path, store: &Store, profile_id: &str) -> ButtonConfig {
    if let Ok(raw) = std::fs::read_to_string(config_path(data_dir, profile_id)) {
        if let Ok(cfg) = serde_json::from_str::<ButtonConfig>(&raw) {
            return cfg;
        }
    }
    // 档案拆分（如小米 1 代从 rc00x 里分出去）后，用户已配的键位还躺在旧仓里：
    // 沿用 `config_from` 指的那份并就地物化到新仓，别让拆档案变成清用户配置。
    if let Some(from) = crate::services::remote::profile::by_id(profile_id).config_from {
        if let Ok(raw) = std::fs::read_to_string(config_path(data_dir, from)) {
            if let Ok(cfg) = serde_json::from_str::<ButtonConfig>(&raw) {
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

/// 预存方案文件（用户 2026-11-20 拍板）：`remote/schemes.<profile>.json`，
/// 命名方案 → 整套映射；UI 下拉切换，切换由前端二次确认后走 apply。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SchemeFile {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub schemes: BTreeMap<String, ButtonConfig>,
}

pub fn schemes_path(data_dir: &Path, profile_id: &str) -> PathBuf {
    config_dir(data_dir).join(format!("schemes.{profile_id}.json"))
}

fn load_schemes(data_dir: &Path, profile_id: &str) -> SchemeFile {
    std::fs::read_to_string(schemes_path(data_dir, profile_id))
        .ok()
        .and_then(|raw| serde_json::from_str::<SchemeFile>(&raw).ok())
        .unwrap_or_default()
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

fn write_schemes(data_dir: &Path, profile_id: &str, file: &SchemeFile) -> Result<(), String> {
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
    write_schemes(data_dir, profile_id, &file)
}

/// 取方案（apply 用）；不存在回 None，由命令层报错。
pub fn get_scheme(data_dir: &Path, profile_id: &str, name: &str) -> Option<ButtonConfig> {
    load_schemes(data_dir, profile_id).schemes.remove(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(mods: u8, key: u16) -> Shortcut {
        Shortcut { mods, key }
    }

    #[test]
    fn defaults_are_native_passthrough() {
        let cfg = ButtonConfig::all_default();
        for b in RemoteButton::ALL {
            assert_eq!(cfg.action(b), ButtonAction::Native, "{b:?}");
            assert!(!cfg.allows_rapid(b), "{b:?}");
            for slot in Slot::ALL {
                assert_eq!(cfg.shortcut(b, slot), None, "{b:?} {slot:?}");
            }
        }
        assert!(!cfg.has_non_native());
        assert_eq!(cfg.find_conflict(), None);
    }

    #[test]
    fn set_and_query() {
        let mut cfg = ButtonConfig::default();
        cfg.set_action(RemoteButton::Back, ButtonAction::Disabled);
        cfg.set_rapid(RemoteButton::Up, true);
        cfg.set_shortcut(RemoteButton::Ok, Slot::Click, Some(sc(4, 0)));
        assert_eq!(cfg.action(RemoteButton::Back), ButtonAction::Disabled);
        assert_eq!(cfg.action(RemoteButton::Ok), ButtonAction::Native);
        assert!(cfg.has_non_native());
        assert!(cfg.allows_rapid(RemoteButton::Up));
        assert!(!cfg.allows_rapid(RemoteButton::Down));
        assert_eq!(cfg.shortcut(RemoteButton::Ok, Slot::Click), Some(sc(4, 0)));
        assert_eq!(cfg.shortcut(RemoteButton::Ok, Slot::Double), None);
    }

    #[test]
    fn serde_shape_is_stable() {
        let mut cfg = ButtonConfig::default();
        cfg.set_action(RemoteButton::VolumeUp, ButtonAction::Disabled);
        cfg.set_rapid(RemoteButton::Back, true);
        cfg.set_shortcut(RemoteButton::Power, Slot::Long, Some(sc(0, 53))); // 裸 Esc
        let raw = serde_json::to_string(&cfg).unwrap();
        assert!(raw.contains("\"volume_up\""));
        assert!(raw.contains("\"enabled\":false"));
        let back: ButtonConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, cfg);
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
            assert_eq!(cfg.shortcut(RemoteButton::Back, slot), None);
        }
    }

    #[test]
    fn new_shape_is_not_mistaken_for_old() {
        let mut cfg = ButtonConfig::default();
        cfg.set_shortcut(RemoteButton::Up, Slot::Double, Some(sc(2, 48)));
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
        cfg.set_shortcut(RemoteButton::Up, Slot::Click, Some(sc(4, 0)));
        cfg.set_shortcut(RemoteButton::Ok, Slot::Double, Some(sc(4, 0)));
        let c = cfg.find_conflict().expect("应报冲突");
        assert_eq!(c.shortcut, sc(4, 0));
        // BTreeMap 字典序：ok 在 up 前。
        assert_eq!(c.first.0, "ok");
        assert_eq!(c.second.0, "up");
    }

    #[test]
    fn conflict_within_same_key_across_slots() {
        let mut cfg = ButtonConfig::default();
        cfg.set_shortcut(RemoteButton::Up, Slot::Click, Some(sc(0, 53)));
        cfg.set_shortcut(RemoteButton::Up, Slot::Long, Some(sc(0, 53)));
        assert!(cfg.find_conflict().is_some());
    }

    #[test]
    fn distinct_shortcuts_have_no_conflict() {
        let mut cfg = ButtonConfig::default();
        cfg.set_shortcut(RemoteButton::Up, Slot::Click, Some(sc(0, 53)));
        cfg.set_shortcut(RemoteButton::Down, Slot::Click, Some(sc(0, 52)));
        cfg.set_shortcut(RemoteButton::Up, Slot::Double, Some(sc(1, 53)));
        assert_eq!(cfg.find_conflict(), None);
    }

    #[test]
    fn has_shortcuts_detects_any_slot() {
        let mut cfg = ButtonConfig::default();
        assert!(!cfg.has_shortcuts(RemoteButton::Up));
        cfg.set_shortcut(RemoteButton::Up, Slot::Long, Some(sc(0, 53)));
        assert!(cfg.has_shortcuts(RemoteButton::Up));
        assert!(!cfg.has_shortcuts(RemoteButton::Down));
    }

    #[test]
    fn file_roundtrip_is_pretty_and_per_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db")).unwrap();
        let mut cfg = ButtonConfig::default();
        cfg.set_shortcut(RemoteButton::Ok, Slot::Click, Some(sc(8, 5)));
        save(tmp.path(), "xiaomi_rc00x", &cfg).unwrap();
        let raw = std::fs::read_to_string(config_path(tmp.path(), "xiaomi_rc00x")).unwrap();
        assert!(raw.contains('\n'), "pretty JSON——人可读可 diff");
        assert!(raw.contains("\"format_version\": 1"));
        assert_eq!(load(tmp.path(), &store, "xiaomi_rc00x"), cfg);
        // 分仓：别的档案没有这份文件，也不会读到 xiaomi 的绑定。
        assert_eq!(load(tmp.path(), &store, "g9n9n"), ButtonConfig::default());
    }

    #[test]
    fn split_profile_inherits_the_source_config_bucket() {
        // 小米 1 代从 rc00x 里分出去后，用户已配的键位还躺在旧仓：
        // 新仓没有文件时沿用 config_from，并就地物化，别让用户重配一遍。
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("t.db")).unwrap();
        let mut cfg = ButtonConfig::default();
        cfg.set_shortcut(RemoteButton::Back, Slot::Click, Some(sc(0, 51)));
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
        assert!(cfg.has_shortcuts(RemoteButton::Power));
        // 别的档案不套用小米键表。
        assert_eq!(default_mapping("g9n9n"), ButtonConfig::default());
    }

    #[test]
    fn schemes_roundtrip_and_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(scheme_names(tmp.path(), "xiaomi_rc00x").is_empty());
        let mut cfg = ButtonConfig::default();
        cfg.set_shortcut(RemoteButton::Ok, Slot::Click, Some(sc(8, 5)));
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
    fn scheme_rename_moves_config_and_guard() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = ButtonConfig::default();
        cfg.set_shortcut(RemoteButton::Ok, Slot::Click, Some(sc(8, 5)));
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
