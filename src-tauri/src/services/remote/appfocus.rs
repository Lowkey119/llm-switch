//! App 聚焦策略（C6，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）。
//!
//! 「打开 App」只是把窗口拉起来，多数输入类 App（豆包、ChatGPT、终端）起来后光标
//! 并不在输入框里。SayAll 的 `openCustomApplication` 口径是：打开 → 激活 → 等一会儿
//! → 按该 App 的策略补一次聚焦。本片落这套骨架 + keyboardShortcut 策略，
//! AX 目标录制（role/identifier/placeholder）按计划视真机效果再排。
//!
//! 纯逻辑，无 cfg、无 FFI（默认 `cargo test` 就编到并全测），存储对齐 macros 口径：
//! `<data_dir>/remote/appfocus.<profile>.json`，按档案分仓。
//!
//! 焦点闸门：延迟聚焦是「过一会儿再抢一次焦点」，用户手速比延迟快得多——
//! 连开两个 App、或者延迟没到用户就自己点了输入框，旧请求必须作废，
//! 否则会出现「打着字光标突然跳走」。每次聚焦请求先 `claim()` 领一个序号，
//! 到点只认最大号，领到旧号的一律放弃（SayAll 的焦点请求序号 gate 口径）。

use crate::services::remote::actions::{config_dir, Shortcut};
use crate::services::remote::macros::MAX_DELAY_MS;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 聚焦策略。`Ax` = 用辅助功能找输入框（同 `FocusInput` 动作）；
/// `Shortcut` = 注入一组快捷键，让 App 自己把光标送进输入框（多数聊天 App 自带这种键）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FocusStrategy {
    #[default]
    None,
    Ax,
    Shortcut {
        shortcut: Shortcut,
    },
}

impl FocusStrategy {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// 日志/报错用的读数（后端兜底文案，前端有自己的 i18n）。
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "不聚焦",
            Self::Ax => "辅助功能聚焦",
            Self::Shortcut { .. } => "快捷键聚焦",
        }
    }
}

/// 一个 App 的聚焦策略：策略 + 延迟（等 App 自己把窗口和光标状态弄好）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppFocusProfile {
    pub bundle_id: String,
    #[serde(default)]
    pub strategy: FocusStrategy,
    /// 打开后等多久再聚焦。上限同宏步骤延迟（`MAX_DELAY_MS`），超了按上限截。
    #[serde(default)]
    pub delay_ms: u32,
}

impl AppFocusProfile {
    /// 校验 + 就地归一。返回给命令层的错必须能直接给人看。
    pub fn normalize(&mut self) -> Result<(), String> {
        self.bundle_id = self.bundle_id.trim().to_string();
        if self.bundle_id.is_empty() {
            return Err("未填写 App 的 bundle id".to_string());
        }
        self.delay_ms = self.delay_ms.min(MAX_DELAY_MS);
        Ok(())
    }
}

/// 策略文件（`remote/appfocus.<profile>.json`）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AppFocusFile {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub profiles: BTreeMap<String, AppFocusProfile>,
}

fn default_format_version() -> u32 {
    1
}

pub fn path(data_dir: &Path, profile_id: &str) -> PathBuf {
    config_dir(data_dir).join(format!("appfocus.{profile_id}.json"))
}

/// 读策略表：文件缺失/坏档回空表（与 macros 同口径，坏档不静默吞——下次保存覆盖它）。
pub fn load(data_dir: &Path, profile_id: &str) -> AppFocusFile {
    std::fs::read_to_string(path(data_dir, profile_id))
        .ok()
        .and_then(|raw| serde_json::from_str::<AppFocusFile>(&raw).ok())
        .unwrap_or_default()
}

fn write(data_dir: &Path, profile_id: &str, file: &AppFocusFile) -> Result<(), String> {
    let p = path(data_dir, profile_id);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("聚焦策略目录创建失败：{e}"))?;
    }
    let raw = serde_json::to_string_pretty(file).map_err(|e| format!("聚焦策略序列化失败：{e}"))?;
    std::fs::write(&p, raw).map_err(|e| format!("聚焦策略文件写入失败：{e}"))
}

/// 某个 App 的聚焦策略（没配 = 打开后不补聚焦）。
pub fn get(data_dir: &Path, profile_id: &str, bundle_id: &str) -> Option<AppFocusProfile> {
    load(data_dir, profile_id).profiles.remove(bundle_id.trim())
}

/// 全部策略（按 bundle id 排序，顺序稳定）。
pub fn list(data_dir: &Path, profile_id: &str) -> Vec<AppFocusProfile> {
    load(data_dir, profile_id).profiles.into_values().collect()
}

/// 存某个 App 的策略；策略为 `None` 即删条目（回 `false`），页面不必区分两种写法。
pub fn set(data_dir: &Path, profile_id: &str, mut p: AppFocusProfile) -> Result<bool, String> {
    p.normalize()?;
    let mut file = load(data_dir, profile_id);
    let kept = if p.strategy.is_none() {
        file.profiles.remove(&p.bundle_id).is_some()
    } else {
        file.profiles.insert(p.bundle_id.clone(), p);
        true
    };
    write(data_dir, profile_id, &file)?;
    Ok(kept)
}

// ------------------------------------------------------------------ 焦点闸门

/// 在途聚焦请求的最新序号。0 留给「从未领号」。
static FOCUS_CLAIM: AtomicU64 = AtomicU64::new(0);

/// 领一个聚焦序号（每次要延迟聚焦前先领）。
pub fn claim() -> u64 {
    FOCUS_CLAIM.fetch_add(1, Ordering::SeqCst) + 1
}

/// 这个序号还是最新的吗？不是就说明期间又有人要焦点，本次该作废。
pub fn is_current(id: u64) -> bool {
    FOCUS_CLAIM.load(Ordering::SeqCst) == id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lls-appfocus-{}-{}-{}",
            tag,
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sc(mods: u8, key: u16) -> Shortcut {
        Shortcut {
            mods,
            key,
            ..Default::default()
        }
    }

    fn prof(bundle: &str, strategy: FocusStrategy, delay: u32) -> AppFocusProfile {
        AppFocusProfile {
            bundle_id: bundle.to_string(),
            strategy,
            delay_ms: delay,
        }
    }

    #[test]
    fn serde_shape_is_stable() {
        let p = prof(
            "com.doubao.chat",
            FocusStrategy::Shortcut {
                shortcut: sc(8, 49),
            },
            600,
        );
        let raw = serde_json::to_string(&p).unwrap();
        assert!(
            raw.contains(r#""strategy":{"type":"shortcut","shortcut""#),
            "{raw}"
        );
        let back: AppFocusProfile = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, p);
        // 缺 strategy = None（老档/手改文件不该凭空聚焦）。
        let bare: AppFocusProfile =
            serde_json::from_str(r#"{"bundle_id":"x","delay_ms":10}"#).unwrap();
        assert!(bare.strategy.is_none());
    }

    #[test]
    fn normalize_trims_bundle_and_caps_delay() {
        // 延迟上限同宏步骤：手滑多打一个 0 按上限截，不整条存不下去。
        let mut p = prof("  com.doubao.chat  ", FocusStrategy::Ax, 99_999);
        p.normalize().unwrap();
        assert_eq!(p.bundle_id, "com.doubao.chat");
        assert_eq!(p.delay_ms, MAX_DELAY_MS);
        assert!(prof("   ", FocusStrategy::Ax, 0).normalize().is_err());
    }

    #[test]
    fn set_get_roundtrip_per_profile() {
        let dir = tmp("roundtrip");
        let profile = "xiaomi_rc00x";
        let p = prof("com.apple.Terminal", FocusStrategy::Ax, 300);
        assert!(set(&dir, profile, p.clone()).unwrap());
        assert_eq!(get(&dir, profile, "com.apple.Terminal").unwrap(), p);
        assert_eq!(list(&dir, profile).len(), 1);
        // 同 bundle 再存 = 覆盖（改策略/改延迟都走这条）。
        set(
            &dir,
            profile,
            prof(
                "com.apple.Terminal",
                FocusStrategy::Shortcut {
                    shortcut: sc(8, 49),
                },
                0,
            ),
        )
        .unwrap();
        assert_eq!(list(&dir, profile).len(), 1);
        assert!(matches!(
            get(&dir, profile, "com.apple.Terminal").unwrap().strategy,
            FocusStrategy::Shortcut { .. }
        ));
        // 按档案分仓：别的档案看不到这条。
        assert!(get(&dir, "other", "com.apple.Terminal").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn strategy_none_removes_the_entry() {
        let dir = tmp("unset");
        let profile = "xiaomi_rc00x";
        set(
            &dir,
            profile,
            prof("com.doubao.chat", FocusStrategy::Ax, 200),
        )
        .unwrap();
        // 取消策略 = 存一条 None，条目该消失（回 true 表示确实删掉了一条）。
        assert!(set(
            &dir,
            profile,
            prof("com.doubao.chat", FocusStrategy::None, 0)
        )
        .unwrap());
        assert!(get(&dir, profile, "com.doubao.chat").is_none());
        assert!(list(&dir, profile).is_empty());
        // 没配过再取消：不算失败，只回「没删到东西」。
        assert!(!set(&dir, profile, prof("com.没有", FocusStrategy::None, 0)).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_newest_focus_claim_survives() {
        // 焦点闸门：连开两个 App，前一个的延迟聚焦必须作废（否则光标从用户手里抢走）。
        let a = claim();
        let b = claim();
        assert_ne!(a, b);
        assert!(!is_current(a), "旧请求必须失效");
        assert!(is_current(b), "最新号必须有效");
        // 手动聚焦（FocusInput）也领号：新号一出现，前一个立刻作废。
        let c = claim();
        assert!(!is_current(b), "期间有人再要焦点，旧请求必须作废");
        assert!(is_current(c));
    }
}
