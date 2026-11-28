//! 快速面板 tab 配置（`<data_dir>/quickpanel.json`）：结构、校验与原子写盘。
//!
//! 纯逻辑，无 cfg、无 FFI（默认 `cargo test` 就编到并全测）。口径见计划
//! `docs/plans/archive/2026-11-25-快速面板编辑复用动作面板.md`：
//! - **JSON 是内容的唯一真相**（原 `prompt_items` 表那条线退役）；
//! - 不再回落内置默认 tab：文件缺失/`tabs` 为空就是空面板，内容由用户自己填；
//! - 写盘一律「校验通过 → 临时文件 → rename 原子替换」，校验或写失败绝不落半个文件。
//!
//! item 四种形状互斥（同一时刻只能是一种）：
//! 文本 `{t,n}`｜宏引用 `{macro}`｜单动作 `{action}`｜命令 `{t,cmd,confirm}`。
//! 面板侧 OK 语义由 tab 的 `type` 决定，item 形状只决定「怎么执行」，两者不交叉校验。

use crate::services::remote::actions::Action;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 可编辑的 tab 类型（注入文本 / 执行命令）。
pub const EDITABLE_TYPES: [&str; 2] = ["chars", "actions"];
/// 只读 tab 类型：条目由面板运行时给（扫盘、配置预览），编辑无意义。
pub const READONLY_TYPES: [&str; 3] = ["prompts", "skills", "config"];
/// `type` 白名单 = 可编辑 + 只读。
pub const TYPES: [&str; 5] = ["chars", "actions", "prompts", "skills", "config"];

/// tab 与条目的硬上限：手滑贴一大坨也不该把面板卡死。
pub const MAX_TABS: usize = 64;
pub const MAX_ITEMS: usize = 512;

/// tab 里的一条内容。字段全 default，缺啥算啥；`label` 是旧 actions 形状的别名。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QpItem {
    /// 文本内容；命令条目里是它的显示名。旧形状 `{label,cmd}` 由 alias 落进来。
    #[serde(default, alias = "label", skip_serializing_if = "String::is_empty")]
    pub t: String,
    /// 备注（字符类 tab 的「打断」「清屏」）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub n: String,
    /// 引用宏库里的动作（`macro` 是 Rust 关键字，字段名另起，serde 名仍是 "macro"）。
    #[serde(default, rename = "macro", skip_serializing_if = "Option::is_none")]
    pub macro_ref: Option<String>,
    /// 单个槽位动作（快捷键 / 打开应用 / 代理 CLI），serde 形状与键位档里的动作一致。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    /// shell 命令（`quickpanel_run_action` 那条路）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cmd: String,
    /// 命令条目要不要面板两步确认（危险命令）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub confirm: bool,
}

/// item 的执行种类（互斥，由 `kind` 判出来）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// 引用宏库里的一条动作。
    Macro,
    /// 单个槽位动作。
    Action,
    /// shell 命令。
    Command,
    /// 纯文本（注入）。
    Text,
    /// 什么都有 = 什么都没填，非法。
    Empty,
}

impl QpItem {
    /// 判执行种类。同时填了多种按 `macro > action > cmd > t` 取优先（前端只可能填一种，
    /// 这里兜住手改文件的人，不报错——报错会让整份配置读不出来，代价太大）。
    pub fn kind(&self) -> ItemKind {
        if self
            .macro_ref
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            return ItemKind::Macro;
        }
        if self.action.is_some() {
            return ItemKind::Action;
        }
        if !self.cmd.trim().is_empty() {
            return ItemKind::Command;
        }
        if !self.t.trim().is_empty() {
            return ItemKind::Text;
        }
        ItemKind::Empty
    }

    /// 条目显示名（面板列表用；命令/引用没填 `t` 时给个兜底读数，不摆空行）。
    pub fn title(&self) -> String {
        match self.kind() {
            ItemKind::Macro => format!("动作 #{}", self.macro_ref.as_deref().unwrap_or("")),
            ItemKind::Action => self
                .action
                .as_ref()
                .map(|a| a.label())
                .unwrap_or_else(|| "动作".to_string()),
            ItemKind::Command => self.t.clone(),
            ItemKind::Text => self.t.clone(),
            ItemKind::Empty => String::new(),
        }
    }
}

/// 一个 tab。`type` 决定 OK 语义，`items` 是 chars/actions 的内容。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QpTab {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub tab_type: String,
    /// `"agent"` = 面板把本 tab 换成当前注入目标的代理。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic: Option<String>,
    /// 前面画虚线分隔（纯视觉）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sep: bool,
    #[serde(default)]
    pub items: Vec<QpItem>,
}

impl QpTab {
    /// 该 tab 的条目能不能编辑（skills/config 只读）。
    pub fn editable(&self) -> bool {
        EDITABLE_TYPES.contains(&self.tab_type.as_str())
    }
}

/// 落盘的文件形状（`{"tabs":[...]}`；读侧容忍裸数组，写侧一律这个形状）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QpConfig {
    #[serde(default)]
    pub tabs: Vec<QpTab>,
}

/// 校验 + 就地归一（key/label 去空白）。返回给命令层的错必须能直接给人看。
pub fn validate(config: &mut QpConfig) -> Result<(), String> {
    if config.tabs.len() > MAX_TABS {
        return Err(format!("tab 太多（上限 {MAX_TABS} 个）"));
    }
    let mut keys: Vec<String> = Vec::with_capacity(config.tabs.len());
    for (i, tab) in config.tabs.iter_mut().enumerate() {
        tab.key = tab.key.trim().to_string();
        tab.label = tab.label.trim().to_string();
        if tab.key.is_empty() {
            return Err(format!("第 {} 个 tab 缺 key", i + 1));
        }
        if keys.contains(&tab.key) {
            return Err(format!("tab key「{}」重复", tab.key));
        }
        keys.push(tab.key.clone());
        if tab.label.is_empty() {
            return Err(format!("tab「{}」缺名称", tab.key));
        }
        if !TYPES.contains(&tab.tab_type.as_str()) {
            return Err(format!(
                "tab「{}」类型「{}」不认识（可用：{}）",
                tab.key,
                tab.tab_type,
                TYPES.join(" / ")
            ));
        }
        if tab.items.len() > MAX_ITEMS {
            return Err(format!("tab「{}」条目太多（上限 {MAX_ITEMS} 条）", tab.key));
        }
        for (j, item) in tab.items.iter().enumerate() {
            if item.kind() == ItemKind::Empty {
                return Err(format!("tab「{}」第 {} 条是空的", tab.key, j + 1));
            }
        }
    }
    Ok(())
}

/// 读配置。文件不存在 = 空配置（不再回落内置默认）；解析失败 = 空配置 + 错误文案，
/// 让面板把「你的 JSON 坏了」显示出来，而不是静默当空面板。
pub fn load(path: &Path) -> (Vec<QpTab>, Option<String>) {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return (Vec::new(), None),
    };
    // 容忍两种写法：`{"tabs":[...]}` 与裸数组 `[...]`（早期文件与手改的人都会写后者）。
    let parsed: Result<QpConfig, serde_json::Error> = match serde_json::from_str(&raw) {
        Ok(c) => Ok(c),
        Err(_) => serde_json::from_str::<Vec<QpTab>>(&raw).map(|tabs| QpConfig { tabs }),
    };
    match parsed {
        Ok(mut c) => match validate(&mut c) {
            Ok(()) => (c.tabs, None),
            Err(e) => (Vec::new(), Some(e)),
        },
        Err(e) => (Vec::new(), Some(format!("JSON 解析失败：{e}"))),
    }
}

/// 写盘：先校验（含就地归一），再写临时文件 + `rename` 原子替换。
/// 失败时原文件保持不动——半份配置 = 面板整个空掉，比不保存糟糕得多。
/// 肉眼看不见的字符：控制符（C0/C1）+ 零宽/格式符。
/// `char::is_control()` 只覆盖 Cc，零宽空格那批是 Cf——看着一样、字节不一样，
/// 不去掉就会让「明明重复」的两条各自留下（用户 2026-11-25 要的就是严格去重）。
fn invisible(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{2060}' | '\u{feff}'
        )
}

/// 去重用的归一化文本（**单一真相**，宏库去重同源复用，不另写第二套）：
/// 剔零宽/控制符 + 去首尾空白；**不改内容本身**（计划 2026-11-26「备注字段与去重」）。
pub fn normalize_text(s: &str) -> String {
    s.chars()
        .filter(|c| !invisible(*c))
        .collect::<String>()
        .trim()
        .to_string()
}

/// 同一 tab 内的去重比较键。种类分开算：引用动作 "3" 与文本 "3" 是两回事，不许互相顶掉。
fn dedupe_key(it: &QpItem) -> String {
    let body = normalize_text(&it.t);
    match it.kind() {
        ItemKind::Macro => format!("macro:{}", it.macro_ref.clone().unwrap_or_default().trim()),
        ItemKind::Action => format!(
            "action:{}",
            serde_json::to_string(&it.action).unwrap_or_default()
        ),
        ItemKind::Command => format!("cmd:{body}"),
        ItemKind::Text => format!("text:{body}"),
        ItemKind::Empty => format!("empty:{body}"),
    }
}

/// 同一 tab 严格去重：重复的只留第一条，后面的剪掉（跨 tab 不管，两个 tab 各要一份是合理的）。
/// 只在写盘前跑一次；比较键见 `dedupe_key`，**原文一个字都不改**，只删重复条目。
pub fn dedupe(tabs: &mut [QpTab]) {
    for tab in tabs.iter_mut() {
        let mut seen: Vec<String> = Vec::with_capacity(tab.items.len());
        tab.items.retain(|it| {
            let k = dedupe_key(it);
            if seen.contains(&k) {
                false
            } else {
                seen.push(k);
                true
            }
        });
    }
}

pub fn save(path: &Path, mut config: QpConfig) -> Result<QpConfig, String> {
    validate(&mut config)?;
    dedupe(&mut config.tabs);
    let text = serde_json::to_string_pretty(&QpConfig {
        tabs: config.tabs.clone(),
    })
    .map_err(|e| format!("序列化失败：{e}"))?;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(d) = dir {
        std::fs::create_dir_all(d).map_err(|e| format!("建目录失败：{e}"))?;
    }
    let tmp = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("quickpanel.json"),
        std::process::id()
    ));
    std::fs::write(&tmp, text).map_err(|e| format!("写临时文件失败：{e}"))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("替换配置失败：{e}"));
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lls-qp-{}-{}-{}",
            tag,
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tab(key: &str, ty: &str, items: Vec<QpItem>) -> QpTab {
        QpTab {
            key: key.into(),
            label: key.into(),
            tab_type: ty.into(),
            dynamic: None,
            sep: false,
            items,
        }
    }

    fn text(t: &str) -> QpItem {
        QpItem {
            t: t.into(),
            ..Default::default()
        }
    }

    #[test]
    fn 同一tab_严格去重_跨tab不管() {
        let mut tabs = vec![
            tab(
                "a",
                "chars",
                vec![
                    text("  继续 "),
                    text("继续\u{200b}"), // 零宽空格：看着一样，字节不一样
                    text("继续"),
                    text("换个"),
                    QpItem {
                        cmd: "ls".into(),
                        ..Default::default()
                    },
                    QpItem {
                        macro_ref: Some("3".into()),
                        ..Default::default()
                    },
                    QpItem {
                        macro_ref: Some("3".into()),
                        t: "别名".into(),
                        ..Default::default()
                    },
                ],
            ),
            tab("b", "chars", vec![text("继续")]),
        ];
        dedupe(&mut tabs);
        // 三条「继续」只留第一条；命令与引用各自保留；引用同 id 的第二条（只多个别名）算重复。
        assert_eq!(tabs[0].items.len(), 4);
        assert_eq!(tabs[0].items[0].t, "  继续 ");
        assert_eq!(tabs[1].items.len(), 1); // 别的 tab 里同样内容不受牵连
    }

    #[test]
    fn item_kind_按优先级判且互斥() {
        assert_eq!(text("^C").kind(), ItemKind::Text);
        assert_eq!(
            QpItem {
                cmd: "ls".into(),
                ..Default::default()
            }
            .kind(),
            ItemKind::Command
        );
        assert_eq!(
            QpItem {
                macro_ref: Some("3".into()),
                cmd: "ls".into(),
                ..Default::default()
            }
            .kind(),
            ItemKind::Macro
        );
        assert_eq!(QpItem::default().kind(), ItemKind::Empty);
        // 空白内容不算内容（手滑存了个空格 = 空条目，照样拒）
        assert_eq!(text("   ").kind(), ItemKind::Empty);
    }

    #[test]
    fn 旧_label_形状读进来落成_t() {
        let item: QpItem =
            serde_json::from_str(r#"{"label":"审查改动","cmd":"git diff"}"#).unwrap();
        assert_eq!(item.t, "审查改动");
        assert_eq!(item.kind(), ItemKind::Command);
    }

    #[test]
    fn 合法配置往返一致() {
        let dir = tmp("roundtrip");
        let path = dir.join("quickpanel.json");
        let cfg = QpConfig {
            tabs: vec![
                tab(
                    "hot",
                    "chars",
                    vec![
                        text("^C"),
                        QpItem {
                            macro_ref: Some("7".into()),
                            ..Default::default()
                        },
                    ],
                ),
                QpTab {
                    key: "mine".into(),
                    label: "我的 skills".into(),
                    tab_type: "skills".into(),
                    dynamic: None,
                    sep: true,
                    items: vec![],
                },
            ],
        };
        let saved = save(&path, cfg.clone()).unwrap();
        assert_eq!(saved, cfg);
        let (tabs, err) = load(&path);
        assert_eq!(err, None);
        assert_eq!(tabs, cfg.tabs);
        assert!(!tabs[1].editable(), "skills 必须只读");
    }

    #[test]
    fn 裸数组写法也能读() {
        let dir = tmp("bare");
        let path = dir.join("quickpanel.json");
        std::fs::write(&path, r#"[{"key":"a","label":"A","type":"chars"}]"#).unwrap();
        let (tabs, err) = load(&path);
        assert_eq!(err, None);
        assert_eq!(tabs.len(), 1);
    }

    #[test]
    fn 校验挡住空_key_重复_key_空_label_坏类型_空条目() {
        let bad = |mut c: QpConfig| validate(&mut c).unwrap_err();
        assert!(bad(QpConfig {
            tabs: vec![tab("  ", "chars", vec![])],
        })
        .contains("key"));
        assert!(bad(QpConfig {
            tabs: vec![tab("a", "chars", vec![]), tab("a", "chars", vec![])],
        })
        .contains("重复"));
        assert!(bad(QpConfig {
            tabs: vec![QpTab {
                key: "a".into(),
                label: "".into(),
                tab_type: "chars".into(),
                dynamic: None,
                sep: false,
                items: vec![]
            }],
        })
        .contains("名称"));
        assert!(bad(QpConfig {
            tabs: vec![tab("a", "nope", vec![])],
        })
        .contains("类型"));
        assert!(bad(QpConfig {
            tabs: vec![tab("a", "chars", vec![QpItem::default()])],
        })
        .contains("空的"));
    }

    #[test]
    fn 坏_json_读成空配置并带错误文案() {
        let dir = tmp("broken");
        let path = dir.join("quickpanel.json");
        std::fs::write(&path, "{ 这不是 json").unwrap();
        let (tabs, err) = load(&path);
        assert!(tabs.is_empty());
        assert!(err.unwrap().contains("解析失败"));
    }

    #[test]
    fn 文件不存在是空配置不是错误() {
        let (tabs, err) = load(&tmp("missing").join("quickpanel.json"));
        assert!(tabs.is_empty());
        assert_eq!(err, None);
    }

    #[test]
    fn 校验失败时原文件一字不动() {
        let dir = tmp("atomic");
        let path = dir.join("quickpanel.json");
        save(
            &path,
            QpConfig {
                tabs: vec![tab("ok", "chars", vec![text("x")])],
            },
        )
        .unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        // 空条目 → 校验拒，旧文件必须还在且没被临时文件顶掉
        assert!(save(
            &path,
            QpConfig {
                tabs: vec![tab("bad", "chars", vec![QpItem::default()])]
            }
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "临时文件必须清掉"
        );
    }
}
