//! 托盘（顶部驻留图标）下拉菜单文案 i18n：中英双表，按系统语言取词。
//!
//! 探测顺序：`LLM_SWITCH_LANG=zh|en` 覆盖 → macOS 系统 UI 语言
//! （`defaults read -g AppleLanguages` 首项，`AppleLocale` 区域兜底）→
//! `plutil` 直读 `~/Library/Preferences/.GlobalPreferences.plist`（同上两项的
//! 文件级兜底：`defaults` 走 cfprefsd，受限环境会失手；实测本机 `defaults`
//! 在沙箱里报「不存在」而文件里明明有 `zh-Hans-CN`）→ POSIX
//! `LC_ALL`/`LC_MESSAGES`/`LANG` → **默认中文**（主界面恒为中文，探测失手
//! 时菜单不该先变英文）。进程内探测一次并缓存，托盘菜单每次重建（60s 周期 /
//! 动作后刷新）都从这里取词。检出结果打 `llm_switch::gui` debug 日志。
//!
//! 新菜单条目必须走本表，禁止在 `build_menu` 里再写死任何一种语言的字面量。

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Zh,
    En,
}

static LANG: OnceLock<Lang> = OnceLock::new();
/// 用户显式选择的语言（0=跟随系统自动探测，1=中，2=英）。
/// 顶栏「中/EN」按钮与启动时的 `ui.lang` 偏好都写这里；托盘菜单每次重建取词。
static OVERRIDE: AtomicU8 = AtomicU8::new(0);

/// 设置语言覆盖（None = 回到自动探测）。
pub fn set_override(l: Option<Lang>) {
    OVERRIDE.store(
        match l {
            None => 0,
            Some(Lang::Zh) => 1,
            Some(Lang::En) => 2,
        },
        Ordering::Relaxed,
    );
}

/// 当前语言（显式覆盖优先；否则进程内探测一次）。
pub fn lang() -> Lang {
    match OVERRIDE.load(Ordering::Relaxed) {
        1 => Lang::Zh,
        2 => Lang::En,
        _ => *LANG.get_or_init(|| {
            let l = detect();
            // 封口对账用：部署后到 drag-diag.log 里确认菜单到底检出了哪种语言。
            tracing::debug!(target: "llm_switch::gui", lang = ?l, "menu language detected");
            l
        }),
    }
}

impl Lang {
    /// 顶部摘要行：今日用量。
    pub fn stats(self, requests: i64, tokens_human: &str, cost_usd: f64) -> String {
        match self {
            Lang::Zh => format!("今日 {requests} 请求 · {tokens_human} tok · ${cost_usd:.4}"),
            Lang::En => format!("Today {requests} reqs · {tokens_human} tok · ${cost_usd:.4}"),
        }
    }

    /// 暂停接单（点击即暂停，条目随状态在 pause/resume 间切换）。
    pub fn pause(self) -> &'static str {
        match self {
            Lang::Zh => "暂停接单",
            Lang::En => "Pause intake",
        }
    }

    /// 恢复接单。
    pub fn resume(self) -> &'static str {
        match self {
            Lang::Zh => "恢复接单",
            Lang::En => "Resume intake",
        }
    }

    /// 「快速切后端」子菜单标题。
    pub fn quick_switch(self) -> &'static str {
        match self {
            Lang::Zh => "快速切后端",
            Lang::En => "Quick backend switch",
        }
    }

    /// 子菜单占位：没有任何后端。
    pub fn no_backends(self) -> &'static str {
        match self {
            Lang::Zh => "（没有后端）",
            Lang::En => "(no backends)",
        }
    }

    /// 「全部启用」。
    pub fn enable_all(self) -> &'static str {
        match self {
            Lang::Zh => "全部启用",
            Lang::En => "Enable all",
        }
    }

    /// 浮动面板显隐开关。
    pub fn float_toggle(self) -> &'static str {
        match self {
            Lang::Zh => "浮动面板 显示/隐藏",
            Lang::En => "Float panel show/hide",
        }
    }

    /// 资源小窗（硬件监控浮窗）显隐开关。
    pub fn sysfloat_toggle(self) -> &'static str {
        match self {
            Lang::Zh => "资源小窗 显示/隐藏",
            Lang::En => "Resource widget show/hide",
        }
    }

    /// 速率状态线二级菜单的标题（勾标跟总开关走，下面逐家单独开关）。
    pub fn rate_lines(self) -> &'static str {
        match self {
            Lang::Zh => "速率状态线",
            Lang::En => "Rate lines",
        }
    }

    /// 二级菜单第一项：整批放出 / 收起（动作项，不挂勾——勾只留给「此刻看得见」的逐家项）。
    pub fn rate_lines_show_all(self) -> &'static str {
        match self {
            Lang::Zh => "全部显示",
            Lang::En => "Show all",
        }
    }

    pub fn rate_lines_hide_all(self) -> &'static str {
        match self {
            Lang::Zh => "全部隐藏",
            Lang::En => "Hide all",
        }
    }

    /// 自动弹出开关（#46）：代理一有请求就点亮它家的速率线（总开关收起时不越权）。
    pub fn rate_lines_auto(self) -> &'static str {
        match self {
            Lang::Zh => "有请求自动弹出",
            Lang::En => "Auto-show on request",
        }
    }

    /// 显示主窗口（原来写死英文 "Show Window"，中文环境也出英文的实报项）。
    pub fn show_window(self) -> &'static str {
        match self {
            Lang::Zh => "显示主窗口",
            Lang::En => "Show main window",
        }
    }

    /// 退出（原来写死 "Quit llm-switch"）。
    pub fn quit(self) -> &'static str {
        match self {
            Lang::Zh => "退出 llm-switch",
            Lang::En => "Quit llm-switch",
        }
    }
}

fn detect() -> Lang {
    // ① 显式覆盖（部署/调试用）。
    if let Ok(v) = std::env::var("LLM_SWITCH_LANG") {
        if let Some(l) = classify(&v) {
            return l;
        }
    }
    // ② macOS 系统 UI 语言：AppleLanguages 首项最准（AppleLocale 只是区域）。
    if let Ok(out) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
    {
        if out.status.success() {
            if let Some(l) = first_entry(&String::from_utf8_lossy(&out.stdout)) {
                return l;
            }
        }
    }
    if let Ok(out) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
    {
        if out.status.success() {
            if let Some(l) = classify(&String::from_utf8_lossy(&out.stdout)) {
                return l;
            }
        }
    }
    // ②b 文件级兜底：defaults 走 cfprefsd，受限环境会失手（本机沙箱实测报
    //    「不存在」，但文件里有 zh-Hans-CN）；plutil 直读文件通常放行。
    if let Some(l) = lang_from_plist(&global_prefs_plist()) {
        return l;
    }
    // ③ POSIX 环境变量兜底。
    for k in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(k) {
            if let Some(l) = classify(&v) {
                return l;
            }
        }
    }
    // ④ 默认中文：主界面恒中文，探测失手也别把中文用户甩进英文菜单。
    Lang::Zh
}

/// 从任意语言标记里认 zh/en（`zh_CN`、`zh-Hans-CN`、`en_US.UTF-8`、`en`…）。
fn classify(tag: &str) -> Option<Lang> {
    let t = tag.trim().trim_matches('"').trim().to_ascii_lowercase();
    let primary = t.split(['-', '_']).next().unwrap_or("");
    if primary.starts_with("zh") {
        Some(Lang::Zh)
    } else if primary.starts_with("en") {
        Some(Lang::En)
    } else {
        None
    }
}

/// AppleLanguages 输出形如 `(\n    "zh-Hans",\n    en\n)`：取第一项认语言；
/// 第一项既非 zh 也非 en（如 `fr`）→ None，交给下一级兜底。
fn first_entry(s: &str) -> Option<Lang> {
    for line in s.lines() {
        let e = line.trim().trim_end_matches(',').trim();
        if e.is_empty() || e == "(" || e == ")" {
            continue;
        }
        return classify(e);
    }
    None
}

/// 全局偏好 plist 路径（HOME 缺失时返回的路径不存在，探测自然落空）。
fn global_prefs_plist() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push("Library/Preferences/.GlobalPreferences.plist");
    p
}

/// `plutil -extract <keypath> raw <plist>`：拿不到（文件缺/key 缺/plutil 挂）→ None。
fn plutil_extract(plist: &std::path::Path, keypath: &str) -> Option<String> {
    let out = std::process::Command::new("plutil")
        .args(["-extract", keypath, "raw"])
        .arg(plist)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// 从全局偏好 plist 文件认语言：AppleLanguages 首项 → AppleLocale 区域。
/// 与 `defaults` 探测同语义，只是绕开 cfprefsd 直读文件。
fn lang_from_plist(plist: &std::path::Path) -> Option<Lang> {
    if let Some(v) = plutil_extract(plist, "AppleLanguages.0") {
        if let Some(l) = classify(&v) {
            return Some(l);
        }
    }
    plutil_extract(plist, "AppleLocale").and_then(|v| classify(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_recognizes_zh_and_en_tags_only() {
        assert_eq!(classify("zh_CN"), Some(Lang::Zh));
        assert_eq!(classify("zh-Hans-CN"), Some(Lang::Zh));
        assert_eq!(classify("  \"zh-Hant\"  "), Some(Lang::Zh));
        assert_eq!(classify("en_US.UTF-8"), Some(Lang::En));
        assert_eq!(classify("en"), Some(Lang::En));
        assert_eq!(classify("fr_FR"), None, "法语不该被认成任何支持语言");
        assert_eq!(classify("junk"), None);
        assert_eq!(classify(""), None);
    }

    #[test]
    fn apple_languages_first_entry_decides() {
        let zh = "(\n    \"zh-Hans\",\n    en\n)";
        assert_eq!(first_entry(zh), Some(Lang::Zh));
        let en = "(\n    en\n)";
        assert_eq!(first_entry(en), Some(Lang::En));
        // 首项不认识 → None（不看后面的项，跟随系统首项的语义），交给兜底。
        assert_eq!(first_entry("(\n    fr\n)"), None);
        assert_eq!(first_entry(""), None);
    }

    #[test]
    fn plist_fallback_reads_languages_then_locale() {
        // plutil 直读文件的兜底：defaults 走 cfprefsd 失手时靠它救回系统语言。
        let dir = std::env::temp_dir().join(format!("llmswitch-i18n-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("gp.plist");
        let head = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">"#;

        // ① AppleLanguages 首项定语言。
        std::fs::write(
            &p,
            format!(
                "{head}<dict><key>AppleLanguages</key><array><string>zh-Hans-CN</string></array><key>AppleLocale</key><string>en_US</string></dict></plist>"
            ),
        )
        .unwrap();
        assert_eq!(lang_from_plist(&p), Some(Lang::Zh), "首项 zh 应压过 locale");

        // ② 没有 AppleLanguages → AppleLocale 区域兜底。
        std::fs::write(
            &p,
            format!("{head}<dict><key>AppleLocale</key><string>zh_CN</string></dict></plist>"),
        )
        .unwrap();
        assert_eq!(lang_from_plist(&p), Some(Lang::Zh));

        // ③ 都没有 / 文件不存在 → None（交给 POSIX 与默认中文）。
        std::fs::write(&p, format!("{head}<dict></dict></plist>")).unwrap();
        assert_eq!(lang_from_plist(&p), None);
        std::fs::remove_file(&p).unwrap();
        assert_eq!(lang_from_plist(&p), None, "文件不存在必须安静落空");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_menu_string_is_localized() {
        // 中英两表逐项互异且非空——防止再出现「只写一种语言字面量」的漏项。
        let pairs = [
            (Lang::Zh.pause(), Lang::En.pause()),
            (Lang::Zh.resume(), Lang::En.resume()),
            (Lang::Zh.quick_switch(), Lang::En.quick_switch()),
            (Lang::Zh.no_backends(), Lang::En.no_backends()),
            (Lang::Zh.enable_all(), Lang::En.enable_all()),
            (Lang::Zh.float_toggle(), Lang::En.float_toggle()),
            (Lang::Zh.show_window(), Lang::En.show_window()),
            (Lang::Zh.quit(), Lang::En.quit()),
        ];
        for (zh, en) in pairs {
            assert!(!zh.is_empty() && !en.is_empty(), "空文案: {zh:?}/{en:?}");
            assert_ne!(zh, en, "中英同串(未翻译): {zh:?}");
        }
        let zh = Lang::Zh.stats(3, "1.2k", 0.5);
        let en = Lang::En.stats(3, "1.2k", 0.5);
        assert!(zh.contains("今日") && zh.contains("3 请求"), "{zh}");
        assert!(en.contains("Today") && en.contains("3 reqs"), "{en}");
        assert_ne!(zh, en);
    }
}
