//! 提示词速取面板的预置内容。
//!
//! 面板建好时表是空的，用户面对空白列表不会自己一条条敲——预置一份能直接用的。
//! 口径三条：
//! 1. **只跑一次**：`settings` 里落 `promptseed.v1` 标记。用户删掉的条目不会被复活，
//!    第二次启动零写入。
//! 2. **代理 tab 只放该家 CLI 真实存在的斜杠命令**（取自本机安装的 CLI 与官方文档），
//!    不编造命令；正文类提示词一律放「常用语」。
//! 3. tab 键必须与 `ClientApp::as_str()` 一致，否则前端 tab 对不上号；
//!    `common` 与 `shell` 是前端自己声明的两个通用 tab。

use crate::store::{Result, Store};

/// 预置完成标记（有值 = 已铺过，不再写）。
pub const SEED_KEY: &str = "promptseed.v1";
/// 前端「常用语」tab 的键，与 PromptPanel.tsx 的 COMMON 同值。
pub const TAB_COMMON: &str = "common";
/// 前端「通用命令」tab 的键，与 PromptPanel.tsx 的 SHELL 同值。
pub const TAB_SHELL: &str = "shell";

/// `(tab, [内容])` 的预置表。顺序即面板里的显示顺序。
const SEEDS: &[(&str, &[&str])] = &[
    (
        TAB_COMMON,
        &[
            "只改必要文件，不要顺手重构",
            "改完跑一遍相关测试再回报",
            "用中文回报，先给结论再给理由",
            "有破坏性改动先说影响面，等我确认再动手",
            "证据不足就直说，不要猜",
            "LGTM，可以合并",
        ],
    ),
    (
        TAB_SHELL,
        &[
            "git status",
            "git diff --stat",
            "git add -A",
            "git log --oneline -15",
            "git stash",
            "列出你改过的文件",
            "跑一遍相关测试再回报",
            "先给结论再给理由",
        ],
    ),
    (
        "claude",
        &[
            "/compact",
            "/clear",
            "/model",
            "/permissions",
            "/config",
            "/init",
            "/memory",
            "/rewind",
        ],
    ),
    (
        "codex",
        &[
            "/status", "/model", "/new", "/diff", "/clear", "/log", "/help",
        ],
    ),
    (
        "opencode",
        &[
            "/models", "/clear", "/compact", "/undo", "/init", "/themes", "/share", "/help",
        ],
    ),
    (
        "pi",
        &[
            "/new",
            "/resume",
            "/settings",
            "/keybindings",
            "/hotkeys",
            "/agent",
            "/fork",
            "/reload",
        ],
    ),
    (
        "gemini",
        &[
            "/new",
            "/clear",
            "/compact",
            "/model",
            "/status",
            "/diff",
            "/review",
            "/doctor",
            "/memory",
            "/settings",
            "/mcp",
            "/help",
        ],
    ),
    (
        "codebuddy",
        &[
            "/new",
            "/clear",
            "/compact",
            "/model",
            "/status",
            "/cost",
            "/diff",
            "/review",
            "/doctor",
            "/permissions",
            "/memory",
            "/help",
        ],
    ),
    (
        "copilot",
        &[
            "/status",
            "/model",
            "/compact",
            "/clear",
            "/cost",
            "/usage",
            "/config",
            "/settings",
            "/init",
            "/memory",
            "/resume",
            "/help",
        ],
    ),
    (
        "deveco",
        &[
            "/status", "/build", "/model", "/config", "/init", "/skills", "/mcp", "/log", "/help",
        ],
    ),
];

/// 铺一次预置内容，返回写入条数（已铺过返回 0）。
///
/// 与 `seed_backends` 同一手法：启动时对齐一次，落库幂等，失败由调用方决定要不要停。
pub fn seed_prompt_items(store: &Store) -> Result<usize> {
    if store.get_setting(SEED_KEY)?.is_some() {
        return Ok(0);
    }
    let mut inserted = 0usize;
    for (tab, items) in SEEDS {
        for content in *items {
            store.create_prompt_item(tab, content)?;
            inserted += 1;
        }
    }
    store.set_setting(SEED_KEY, "1")?;
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时库：预置会写 settings 标记，必须与真库隔离。
    /// 返回顺序是 (库, 目录)——Rust 按声明顺序 drop，库先关、目录后删。
    fn temp_store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("临时目录");
        let store = Store::open(&dir.path().join("seed-test.db")).expect("临时库");
        (store, dir)
    }

    #[test]
    fn seed_runs_once_and_count_matches() {
        let (store, _dir) = temp_store();
        let total: usize = SEEDS.iter().map(|(_, items)| items.len()).sum();
        assert_eq!(seed_prompt_items(&store).unwrap(), total);
        // 第二次必须零写入：用户删掉的条目不会被复活。
        assert_eq!(seed_prompt_items(&store).unwrap(), 0);
        assert_eq!(store.list_prompt_items().unwrap().len(), total);
    }

    #[test]
    fn seed_rows_are_non_empty_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (tab, items) in SEEDS {
            assert!(!tab.is_empty(), "tab 不能为空");
            for content in *items {
                assert!(!content.trim().is_empty(), "内容不能是空白");
                assert!(
                    seen.insert((*tab, *content)),
                    "同 tab 内出现重复条目 {tab} / {content}"
                );
                // 斜杠命令必须是一条整命令：带空格就不是命令而是句子，预置里不许混。
                if content.starts_with('/') {
                    assert!(
                        !content.contains(' '),
                        "{tab} 的 {content} 看着像斜杠命令却带空格"
                    );
                }
            }
        }
    }
}
