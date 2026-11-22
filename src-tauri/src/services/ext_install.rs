//! 客户端扩展的一键安装 / 卸载：源码随包内嵌（`include_str!`），状态按逐字节比对，不进库。
//!
//! ## 装的是同一个「速率扩展」
//!
//! 所有代理装的速率扩展都是 llm-switch 速率出口的**瘦客户端**（`assets/rate-extensions/`）：
//! 数值一律取网关 `GET /admin/rates/line?app=<代理>`（[`crate::gateway::rates::line_json`]），
//! 与浮窗单行速率线同源同口径（引擎原生速率优先 → 退回字节 × 纠偏系数的校准值；上下文
//! 出口已钳到窗口）。各家自己估速率的老路（transcript / 账本 / nettop / 逐 chunk 计数）
//! 全部退役——一份计算只留网关那一处，代理侧只取数与排版。
//!
//! ## 每端用它自己的官方扩展机制（2026-11 逐家查证）
//!
//! | 端 | 有没有实时展示位 | 装法（官方） | 卸法（官方） |
//! |---|---|---|---|
//! | pi | 有：扩展 widget | 写 `<PI_CODING_AGENT_DIR\|~/.pi/agent>/extensions/*.ts`（pi 自动扫描） | 删文件 |
//! | claude | 有：statusLine | 写脚本 + `settings.json` 的 `statusLine.command`（`refreshInterval: 1`） | 删脚本 + 撤 statusLine |
//! | codebuddy | 有：statusLine（与 claude 同一套 stdin 协议） | 同上，落 `<CODEBUDDY_CONFIG_DIR\|~/.codebuddy>` | 同上 |
//! | opencode | 有：TUI 插件槽 | 写 `plugins/llm-switch-rate/tui.ts`（V2 自动发现 `tui.ts`，无需登记） | 删插件文件 |
//! | deveco | 有：TUI 插件槽（opencode 同内核，**同一份源码**） | 写 `plugin/llm-switch-rate.ts` + 登记 `deveco.jsonc` 的 `plugin` | 删文件 + 撤登记 |
//! | codex | **无**：`/statusline` 只能选内置条目，自定义状态线是实验特性（`tui_extensions_statusline`，under development） | 写脚本 + 个人技能 `skills/llm-switch-rate/SKILL.md`（问速率时由模型跑脚本） | 删两处 |
//! | gemini | **无**：扩展只带命令/hooks/MCP/skills | 写脚本 + `gemini extensions install <目录>` | `gemini extensions uninstall llm-switch-rate` + 删脚本 |
//! | copilot | **无**：插件只带 agents/skills/hooks/MCP | 写脚本 + `copilot plugin install <目录>` | `copilot plugin uninstall llm-switch-rate` + 删脚本 |
//!
//! 没有实时展示位的三家不是漏做：它们的插件面里没有任何「每秒刷一行」的挂载点，硬塞一个
//! 常驻进程进 CLI 是污染。这三家给的是**命令形态**（技能 / 扩展命令），外加 llm-switch
//! 自己的浮窗速率线（按代理过滤，见 `gui::rateline`）。
//!
//! ## 通用规矩
//!
//! - 三态状态：`installed`（与本版逐字节一致 / 登记在位）· `missing` · `outdated`（内容或
//!   登记与本版不符，含手改）。
//! - 覆盖或卸载前先备份到 `<data_dir>/<app>-ext-backups/`，与 skill-backups 同风格；
//!   符号链接只摘链接不写穿（写穿会改到链接目标，那不是安装）。
//! - 配置文件（settings.json / deveco.jsonc / 旧 tui.json 的残留登记）只做**外科手术**：
//!   只加/撤我们自己那一项，其余键与注释原样不动；解析不了就报错，绝不覆写。
//! - 状态不进库：与内嵌源码比对即得，升级 llm-switch 后重装即对齐。

use crate::takeover::{ClientApp, TakeoverEnv};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

// ── 内嵌源码 ────────────────────────────────────────────────────────────────

/// 各家共用的速率脚本（瘦客户端；`--statusline` / `--watch` / 单行三种用法）。
const RATE_PY: &str = include_str!("../../../assets/rate-extensions/llm-switch-rate.py");
/// pi 扩展（输入行上方 widget）。
const RATE_PI_TS: &str = include_str!("../../../assets/rate-extensions/pi-llm-switch-rate.ts");
/// opencode / DevEco 的 TUI 插件（页脚槽）：两端**同一份源码**，双内核入口
/// （V2 `setup` + V1 `tui`），落盘名两端都是 `tui` 系（见 `OPENCODE_ITEMS` / `DEVECO_ITEMS`）。
const RATE_TUI_TS: &str = include_str!("../../../assets/rate-extensions/tui-llm-switch-rate.ts");
/// pi 的大小模型路由（与速率无关，随包一起装的既有扩展）。
const PI_ROUTER_TS: &str = include_str!("../../../assets/pi-extensions/llm-switch-router.ts");

const GEMINI_MANIFEST: &str =
    include_str!("../../../assets/rate-extensions/gemini/gemini-extension.json");
const GEMINI_CMD: &str = include_str!("../../../assets/rate-extensions/gemini/commands/rate.toml");
const CODEX_SKILL: &str = include_str!("../../../assets/rate-extensions/codex/SKILL.md");
const COPILOT_MANIFEST: &str = include_str!("../../../assets/rate-extensions/copilot/plugin.json");
const COPILOT_SKILL: &str =
    include_str!("../../../assets/rate-extensions/copilot/skills/llm-switch-rate/SKILL.md");

/// 速率扩展的包名（CLI 侧的标识，卸载与探测都靠它）。
const PKG: &str = "llm-switch-rate";

// ── 扩展包 ──────────────────────────────────────────────────────────────────

/// llm-switch 能替代理装的两类东西。**装的是两类完全不同的东西**：
///
/// - [`Pack::Rate`]：llm-switch 自己的速率扩展，源码随包内嵌（`include_str!`），
///   逐字节比对即得三态。
/// - [`Pack::Ponytail`]：第三方 ponytail 插件，源是一个 git 仓，由
///   [`crate::services::ponytail`] 托管一份检出，各端走自家官方插件机制装。
///
/// 两包共用同一套机器（备份、`retire`、登录 shell 免交互、三态语义），只是清单与
/// 包名不同——这是泛化成「包」而不是另起一个平行模块的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pack {
    Rate,
    Ponytail,
}

impl Pack {
    /// 宽松解析：认不出一律按速率扩展（老前端不传 `pack` 就是这个形状）。
    pub fn parse(s: &str) -> Pack {
        match s.trim().to_ascii_lowercase().as_str() {
            "ponytail" => Pack::Ponytail,
            _ => Pack::Rate,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Pack::Rate => "rate",
            Pack::Ponytail => "ponytail",
        }
    }

    /// CLI 侧的包名：探测命中与卸载都用它。
    pub fn pkg(self) -> &'static str {
        match self {
            Pack::Rate => PKG,
            Pack::Ponytail => crate::services::ponytail::PKG,
        }
    }
}

// ── 安装项 ──────────────────────────────────────────────────────────────────

/// 一个安装项的形态。
#[derive(Debug, Clone, Copy)]
pub enum Kind {
    /// 落盘文件（相对该端配置目录）。
    File {
        rel: &'static str,
        source: &'static str,
    },
    /// 状态线登记：`settings.json` 的 `statusLine` 指向本端的速率脚本。
    StatusLine,
    /// TUI 插件登记：配置文件的 `plugin` 数组里带上本插件路径。
    Register {
        /// 配置文件名（相对该端配置目录）。
        file: &'static str,
        /// 被登记的插件路径（相对该端配置目录）。
        plugin: &'static str,
        /// 登记时是否写成 `file://` URI（deveco 用裸路径 `uri: false`；
        /// 旧 opencode tui.json 用 URI，现在只在撤残留登记时用到）。
        uri: bool,
    },
    /// 官方 CLI 安装：先把插件目录落到数据目录，再在登录 shell 里跑装/卸/探测命令。
    /// 命令模板里的 `{dir}` 由安装器替换成暂存目录（已带引号），`{pkg}` 是包名。
    Cli {
        stage: &'static [(&'static str, &'static str)],
        install: &'static str,
        uninstall: &'static str,
        probe: &'static str,
    },
    /// 整仓插件安装（ponytail 专用）：源不是内嵌字符串，而是 llm-switch 托管的 git
    /// 检出目录。`{dir}` 换成该检出目录（已带引号），`{pkg}` 是包名。
    ///
    /// 与 [`Kind::Cli`] 的区别只有「源从哪来」：备份、超时、免交互、探测判定全部同一套。
    Repo {
        install: &'static str,
        uninstall: &'static str,
        probe: &'static str,
    },
}

/// 一个安装项：展示名 + 一句话说明 + 形态。
pub struct Item {
    pub name: &'static str,
    pub desc: &'static str,
    pub kind: Kind,
}

/// 该端配置目录（与各端自身读配置的口径一致，`TakeoverEnv::dir` 收口）。
/// 安装扫描（`services::agents_scan`）也用它报 copilot 这类「没有接管配置文件」端的落点。
pub(crate) fn config_dir(app: ClientApp, env: &TakeoverEnv) -> PathBuf {
    match app {
        ClientApp::Pi => env
            .dir("PI_CODING_AGENT_DIR", ".pi/agent")
            .join("extensions"),
        ClientApp::Claude => env.dir("CLAUDE_CONFIG_DIR", ".claude"),
        ClientApp::CodeBuddy => env.dir("CODEBUDDY_CONFIG_DIR", ".codebuddy"),
        ClientApp::Codex => env.dir("CODEX_HOME", ".codex"),
        ClientApp::Gemini => env.home.join(".gemini"),
        ClientApp::OpenCode => env.dir("XDG_CONFIG_HOME", ".config").join("opencode"),
        ClientApp::DevEco => env.dir("XDG_CONFIG_HOME", ".config").join("deveco"),
        // copilot 是网关门面 + Copilot CLI 的状态目录（~/.copilot：chats / installed-plugins）。
        ClientApp::Copilot => env.home.join(".copilot"),
    }
}

/// 该端某个扩展包的安装清单。
pub fn items(app: ClientApp, pack: Pack) -> &'static [Item] {
    if pack == Pack::Ponytail {
        return crate::services::ponytail::items(app);
    }
    match app {
        ClientApp::Pi => &PI_ITEMS,
        ClientApp::Claude => &CLAUDE_ITEMS,
        ClientApp::CodeBuddy => &CODEBUDDY_ITEMS,
        ClientApp::Codex => &CODEX_ITEMS,
        ClientApp::Gemini => &GEMINI_ITEMS,
        ClientApp::OpenCode => &OPENCODE_ITEMS,
        ClientApp::DevEco => &DEVECO_ITEMS,
        ClientApp::Copilot => &COPILOT_ITEMS,
    }
}

/// 上一代速率扩展的落盘名：装与卸都顺手清掉（先备份），免得新旧两条线同时刷屏。
fn legacy(app: ClientApp) -> &'static [&'static str] {
    match app {
        ClientApp::Pi => &["gen-rate.ts"],
        ClientApp::Claude => &["cc-rate.py"],
        ClientApp::Codex => &["codex-rate.py"],
        ClientApp::CodeBuddy => &["cb-rate.py"],
        // opencode 旧版是 tui.js（V1 登记制时代）；deveco 旧版是 plugin/llm-switch-rate.js。
        // 都换成同一份 tui 系源码后，旧文件装与卸都清掉。
        ClientApp::OpenCode => &["kernel-rate.py", "plugins/llm-switch-rate/tui.js"],
        ClientApp::DevEco => &["kernel-rate.py", "plugin/llm-switch-rate.js"],
        _ => &[],
    }
}

const PI_ITEMS: [Item; 2] = [
    Item {
        name: "llm-switch-rate.ts",
        desc: "速率条：输入行上方显示 llm-switch 算好的 decode / prefill / ctx",
        kind: Kind::File {
            rel: "llm-switch-rate.ts",
            source: RATE_PI_TS,
        },
    },
    Item {
        name: "llm-switch-router.ts",
        desc: "大小模型路由：注册 llm-switch/auto（读矩阵的主/小槽）",
        kind: Kind::File {
            rel: "llm-switch-router.ts",
            source: PI_ROUTER_TS,
        },
    },
];

const CLAUDE_ITEMS: [Item; 2] = [
    Item {
        name: "llm-switch-rate.py",
        desc: "速率脚本：状态线脚本落盘即生效（refreshInterval=1）",
        kind: Kind::File {
            rel: "llm-switch-rate.py",
            source: RATE_PY,
        },
    },
    Item {
        name: "settings.json · statusLine",
        desc: "状态线登记：statusLine.command 指向本端速率脚本",
        kind: Kind::StatusLine,
    },
];

const CODEBUDDY_ITEMS: [Item; 2] = [
    Item {
        name: "llm-switch-rate.py",
        desc: "速率脚本：CodeBuddy 的 statusLine 与 Claude Code 同一套 stdin 协议",
        kind: Kind::File {
            rel: "llm-switch-rate.py",
            source: RATE_PY,
        },
    },
    Item {
        name: "settings.json · statusLine",
        desc: "状态线登记：statusLine.command 指向本端速率脚本",
        kind: Kind::StatusLine,
    },
];

const CODEX_ITEMS: [Item; 2] = [
    Item {
        name: "llm-switch-rate.py",
        desc: "速率脚本：单行输出可嵌 tmux/prompt，--watch 放独立小窗",
        kind: Kind::File {
            rel: "llm-switch-rate.py",
            source: RATE_PY,
        },
    },
    Item {
        name: "skills/llm-switch-rate/SKILL.md",
        desc: "个人技能：问「现在多快」时由 codex 跑脚本原样报数",
        kind: Kind::File {
            rel: "skills/llm-switch-rate/SKILL.md",
            source: CODEX_SKILL,
        },
    },
];

const GEMINI_ITEMS: [Item; 2] = [
    Item {
        name: "llm-switch-rate.py",
        desc: "速率脚本：扩展命令 /rate 调它（落 ~/.gemini）",
        kind: Kind::File {
            rel: "llm-switch-rate.py",
            source: RATE_PY,
        },
    },
    Item {
        name: "gemini extensions · llm-switch-rate",
        desc: "官方扩展安装：gemini extensions install / uninstall",
        kind: Kind::Cli {
            stage: &[
                ("gemini-extension.json", GEMINI_MANIFEST),
                ("commands/rate.toml", GEMINI_CMD),
            ],
            // 装扩展要过两道信任确认（工作区 + 扩展源目录），登录 shell 里问不到、一问就挂：
            // `GEMINI_CLI_TRUST_WORKSPACE=1` 免掉工作区那道，`printf 'y'` 顶掉源目录那道，
            // `--consent` / `--skip-settings` 免掉安全确认与安装期问答。四条都是非交互必需。
            install: "printf 'y\\n' | GEMINI_CLI_TRUST_WORKSPACE=1 gemini extensions install \"{dir}\" --consent --skip-settings",
            uninstall: "gemini extensions uninstall {pkg}",
            probe: "gemini extensions list",
        },
    },
];

const OPENCODE_ITEMS: [Item; 1] = [Item {
    name: "plugins/llm-switch-rate/tui.ts",
    desc: "TUI 插件：页脚显示 llm-switch 算好的速率（V2 自动发现，无需登记）",
    kind: Kind::File {
        rel: "plugins/llm-switch-rate/tui.ts",
        source: RATE_TUI_TS,
    },
}];

const DEVECO_ITEMS: [Item; 2] = [
    Item {
        name: "plugin/llm-switch-rate.ts",
        desc: "TUI 插件：页脚显示 llm-switch 算好的速率（与 opencode 同一份源码）",
        kind: Kind::File {
            rel: "plugin/llm-switch-rate.ts",
            source: RATE_TUI_TS,
        },
    },
    Item {
        name: "deveco.jsonc · plugin",
        desc: "插件登记：deveco.jsonc 的 plugin 数组带上本插件",
        kind: Kind::Register {
            file: "deveco.jsonc",
            plugin: "plugin/llm-switch-rate.ts",
            uri: false,
        },
    },
];

const COPILOT_ITEMS: [Item; 2] = [
    Item {
        name: "llm-switch-rate.py",
        desc: "速率脚本：插件技能调它（落 ~/.copilot）",
        kind: Kind::File {
            rel: "llm-switch-rate.py",
            source: RATE_PY,
        },
    },
    Item {
        name: "copilot plugin · llm-switch-rate",
        desc: "官方插件安装：copilot plugin install / uninstall",
        kind: Kind::Cli {
            stage: &[
                ("plugin.json", COPILOT_MANIFEST),
                ("skills/llm-switch-rate/SKILL.md", COPILOT_SKILL),
            ],
            install: "copilot plugin install \"{dir}\"",
            uninstall: "copilot plugin uninstall {pkg}",
            probe: "copilot plugin list",
        },
    },
];

// ── 状态 ────────────────────────────────────────────────────────────────────

/// 一个安装项的状态（UI 列表行）。
#[derive(Debug, Serialize)]
pub struct ExtStatus {
    pub name: &'static str,
    pub desc: &'static str,
    /// `installed` 与本版一致 / `missing` 未装 / `outdated` 已装但与本版不符（旧版或手改）。
    pub state: &'static str,
    pub path: String,
}

/// 该端全部安装项的状态。
pub fn status(
    app: ClientApp,
    pack: Pack,
    env: &TakeoverEnv,
    data_dir: &Path,
) -> Result<Vec<ExtStatus>, String> {
    let dir = config_dir(app, env);
    let mut out = Vec::new();
    for it in items(app, pack) {
        let (state, path) = item_state(app, it, &dir, data_dir, pack)?;
        out.push(ExtStatus {
            name: it.name,
            desc: it.desc,
            state,
            path,
        });
    }
    Ok(out)
}

fn item_state(
    app: ClientApp,
    it: &Item,
    dir: &Path,
    data_dir: &Path,
    pack: Pack,
) -> Result<(&'static str, String), String> {
    match it.kind {
        Kind::File { rel, source } => {
            let path = dir.join(rel);
            let state = match fs::read(&path) {
                Ok(cur) if cur == source.as_bytes() => "installed",
                Ok(_) => "outdated",
                Err(_) => "missing",
            };
            Ok((state, path.display().to_string()))
        }
        Kind::StatusLine => {
            let path = dir.join("settings.json");
            let want = statusline_command(app, dir);
            let cur = fs::read_to_string(&path)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                .and_then(|v| v["statusLine"]["command"].as_str().map(|s| s.to_string()));
            Ok((
                match cur {
                    Some(c) if c == want => "installed",
                    Some(_) => "outdated",
                    None => "missing",
                },
                path.display().to_string(),
            ))
        }
        Kind::Register { file, plugin, uri } => {
            let path = dir.join(file);
            let entry = register_entry(dir, plugin, uri);
            let text = fs::read_to_string(&path).unwrap_or_default();
            Ok((
                match plugin_entry(&text, &entry) {
                    Some(true) => "installed",
                    Some(false) => "outdated",
                    None if !path.exists() => "missing",
                    None => "outdated",
                },
                path.display().to_string(),
            ))
        }
        Kind::Cli { probe, .. } => {
            let out = run_login_shell(probe, PROBE_TIMEOUT).unwrap_or_default();
            Ok((
                if out.contains(PKG) {
                    "installed"
                } else {
                    "missing"
                },
                staged_dir(app, Pack::Rate, data_dir).display().to_string(),
            ))
        }
        // 整仓插件只回答「装没装」：各家 `plugin list` 的输出格式不统一，硬解析版本
        // 就是猜。版本差在代理页的「上游 / 本地检出」两行上说清，不伪装成逐端版本。
        Kind::Repo { probe, .. } => {
            let out = run_login_shell(probe, PROBE_TIMEOUT).unwrap_or_default();
            Ok((
                if out.contains(pack.pkg()) {
                    "installed"
                } else {
                    "missing"
                },
                crate::services::ponytail::checkout_dir(data_dir)
                    .display()
                    .to_string(),
            ))
        }
    }
}

// ── 安装 ────────────────────────────────────────────────────────────────────

/// 一个安装项的结果行。
#[derive(Debug, Serialize)]
pub struct InstallResult {
    pub name: &'static str,
    /// `installed` 新装 / `updated` 覆盖旧内容 / `unchanged` 已是本版 / `removed` 卸载。
    pub action: &'static str,
    /// 覆盖或卸载前的备份路径（新装与一致时为 null）。
    pub backup: Option<String>,
}

/// 一键安装（或更新）该端某个扩展包的全部项。
pub fn install_all(
    app: ClientApp,
    pack: Pack,
    env: &TakeoverEnv,
    data_dir: &Path,
    backup_dir: &Path,
) -> Result<Vec<InstallResult>, String> {
    let dir = config_dir(app, env);
    let mut out = Vec::new();
    // 旧代速率扩展的残留与旧 tui.json 登记只属于速率包；ponytail 不往配置目录落文件，
    // 也不该替它建配置目录（代理未装时建出来就是个空壳）。
    if pack == Pack::Rate {
        fs::create_dir_all(&dir).map_err(|e| format!("创建扩展目录失败 {}: {e}", dir.display()))?;
        for old in legacy(app) {
            if let Some(b) = retire(&dir.join(old), backup_dir, old)? {
                out.push(InstallResult {
                    name: old,
                    action: "removed",
                    backup: Some(b),
                });
            }
        }
        if let Some(row) = retire_stale_tui_json(app, &dir, backup_dir) {
            out.push(row);
        }
    }
    for it in items(app, pack) {
        out.push(install_item(app, it, &dir, data_dir, backup_dir, pack)?);
    }
    Ok(out)
}

fn install_item(
    app: ClientApp,
    it: &Item,
    dir: &Path,
    data_dir: &Path,
    backup_dir: &Path,
    pack: Pack,
) -> Result<InstallResult, String> {
    let r = |action: &'static str, backup: Option<String>| InstallResult {
        name: it.name,
        action,
        backup,
    };
    match it.kind {
        Kind::File { rel, source } => {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
            }
            let cur = fs::read(&path).ok(); // 读穿链接：拿到的是目标内容
            if cur.as_deref() == Some(source.as_bytes()) {
                return Ok(r("unchanged", None));
            }
            let backup = replace_file(&path, source.as_bytes(), backup_dir, it.name)?;
            Ok(r(
                if cur.is_some() {
                    "updated"
                } else {
                    "installed"
                },
                backup,
            ))
        }
        Kind::StatusLine => patch_statusline(app, dir, backup_dir, true).map(|(a, b)| r(a, b)),
        Kind::Register { file, plugin, uri } => {
            let entry = register_entry(dir, plugin, uri);
            patch_plugin_entry(&dir.join(file), &entry, backup_dir, it.name, true)
                .map(|(a, b)| r(a, b))
        }
        Kind::Cli {
            stage,
            install,
            probe,
            ..
        } => {
            let stage_dir = staged_dir(app, Pack::Rate, data_dir);
            write_stage(&stage_dir, stage)?;
            let quoted = format!("\"{}\"", stage_dir.display());
            let cmd = install.replace("{dir}", &quoted).replace("{pkg}", PKG);
            let already = run_login_shell(probe, PROBE_TIMEOUT)
                .unwrap_or_default()
                .contains(PKG);
            run_login_shell(&cmd, CLI_TIMEOUT)?;
            let now = run_login_shell(probe, PROBE_TIMEOUT)
                .unwrap_or_default()
                .contains(PKG);
            if !now && !already {
                return Err(format!("{PKG} 安装命令跑完仍未在列表里，看运行日志"));
            }
            Ok(r(if already { "unchanged" } else { "installed" }, None))
        }
        Kind::Repo {
            install,
            probe,
            uninstall: _,
        } => {
            // 源是托管检出：没检出先浅克隆（首次装 ponytail 就是这里拉的）。
            let repo = crate::services::ponytail::checkout_dir(data_dir);
            crate::services::ponytail::ensure_checkout(&repo)?;
            let quoted = format!("\"{}\"", repo.display());
            let cmd = install
                .replace("{dir}", &quoted)
                .replace("{pkg}", pack.pkg());
            let already = run_login_shell(probe, PROBE_TIMEOUT)
                .unwrap_or_default()
                .contains(pack.pkg());
            run_login_shell(&cmd, CLI_TIMEOUT)?;
            let now = run_login_shell(probe, PROBE_TIMEOUT)
                .unwrap_or_default()
                .contains(pack.pkg());
            if !now && !already {
                return Err(format!(
                    "{} 安装命令跑完仍未在列表里，看运行日志",
                    pack.pkg()
                ));
            }
            Ok(r(if already { "unchanged" } else { "installed" }, None))
        }
    }
}

/// 一键卸载：删掉我们装的东西（先备份），配置登记反向撤销。
pub fn uninstall_all(
    app: ClientApp,
    pack: Pack,
    env: &TakeoverEnv,
    data_dir: &Path,
    backup_dir: &Path,
) -> Result<Vec<InstallResult>, String> {
    let dir = config_dir(app, env);
    let mut out = Vec::new();
    for it in items(app, pack).iter().rev() {
        let r = |action: &'static str, backup: Option<String>| InstallResult {
            name: it.name,
            action,
            backup,
        };
        match it.kind {
            Kind::File { rel, .. } => {
                let path = dir.join(rel);
                match retire(&path, backup_dir, it.name)? {
                    Some(b) => out.push(r("removed", Some(b))),
                    None => out.push(r("unchanged", None)),
                }
            }
            Kind::StatusLine => {
                let (action, backup) = patch_statusline(app, &dir, backup_dir, false)?;
                out.push(r(action, backup));
            }
            Kind::Register { file, plugin, uri } => {
                let entry = register_entry(&dir, plugin, uri);
                let (action, backup) =
                    patch_plugin_entry(&dir.join(file), &entry, backup_dir, it.name, false)?;
                out.push(r(action, backup));
            }
            Kind::Cli {
                uninstall, probe, ..
            } => {
                let before = run_login_shell(probe, PROBE_TIMEOUT)
                    .unwrap_or_default()
                    .contains(PKG);
                if before {
                    run_login_shell(&uninstall.replace("{pkg}", PKG), CLI_TIMEOUT)?;
                }
                out.push(r(if before { "removed" } else { "unchanged" }, None));
            }
            Kind::Repo {
                uninstall, probe, ..
            } => {
                let before = run_login_shell(probe, PROBE_TIMEOUT)
                    .unwrap_or_default()
                    .contains(pack.pkg());
                if before {
                    let repo = crate::services::ponytail::checkout_dir(data_dir);
                    let cmd = uninstall
                        .replace("{dir}", &format!("\"{}\"", repo.display()))
                        .replace("{pkg}", pack.pkg());
                    run_login_shell(&cmd, CLI_TIMEOUT)?;
                }
                out.push(r(if before { "removed" } else { "unchanged" }, None));
            }
        }
    }
    // 旧代残留与暂存目录都是速率包的东西，卸 ponytail 不得顺手拆掉速率扩展。
    if pack == Pack::Rate {
        for old in legacy(app) {
            if let Some(b) = retire(&dir.join(old), backup_dir, old)? {
                out.push(InstallResult {
                    name: old,
                    action: "removed",
                    backup: Some(b),
                });
            }
        }
        if let Some(row) = retire_stale_tui_json(app, &dir, backup_dir) {
            out.push(row);
        }
        // CLI 插件的暂存目录不再需要（卸载后重装会重写）。
        let _ = fs::remove_dir_all(staged_dir(app, Pack::Rate, data_dir));
    }
    Ok(out)
}

// ── 落盘与备份 ──────────────────────────────────────────────────────────────

/// 覆盖写一个文件：符号链接只摘链接（写穿会改到链接目标），实体文件先备份。
fn replace_file(
    path: &Path,
    bytes: &[u8],
    backup_dir: &Path,
    tag: &str,
) -> Result<Option<String>, String> {
    let cur = fs::read(path).ok();
    match fs::symlink_metadata(path) {
        Ok(md) if md.file_type().is_symlink() => {
            fs::remove_file(path).map_err(|e| format!("移除旧链接失败 {}: {e}", path.display()))?;
        }
        Ok(_) => {
            if let Some(prev) = cur.as_ref().filter(|c| !c.is_empty()) {
                let b = backup_path(backup_dir, tag)?;
                fs::write(&b, prev).map_err(|e| format!("备份旧扩展失败 {}: {e}", b.display()))?;
                return write_and_ok(path, bytes, Some(b.display().to_string()));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(format!("检查扩展失败 {}: {err}", path.display())),
    }
    write_and_ok(path, bytes, None)
}

fn write_and_ok(
    path: &Path,
    bytes: &[u8],
    backup: Option<String>,
) -> Result<Option<String>, String> {
    fs::write(path, bytes).map_err(|e| format!("写入扩展失败 {}: {e}", path.display()))?;
    Ok(backup)
}

/// 删除一个我们装过的文件（先备份）。不存在 → `None`（没装过，不报错）。
fn retire(path: &Path, backup_dir: &Path, tag: &str) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("检查扩展失败 {}: {e}", path.display())),
        Ok(_) => {}
    }
    let backup = match fs::read(path) {
        Ok(prev) if !prev.is_empty() => {
            let b = backup_path(backup_dir, tag)?;
            fs::write(&b, &prev).map_err(|e| format!("备份失败 {}: {e}", b.display()))?;
            Some(b.display().to_string())
        }
        _ => None,
    };
    let md = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if md.is_dir() {
        fs::remove_dir_all(path).map_err(|e| format!("删除目录失败 {}: {e}", path.display()))?;
    } else {
        fs::remove_file(path).map_err(|e| format!("删除失败 {}: {e}", path.display()))?;
    }
    Ok(backup)
}

fn backup_path(backup_dir: &Path, tag: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(backup_dir)
        .map_err(|e| format!("创建备份目录失败 {}: {e}", backup_dir.display()))?;
    let safe = tag.replace('/', "_");
    Ok(backup_dir.join(format!("{}-{}.bak", safe, epoch_secs())))
}

/// 备份文件名的时间戳（无 chrono 依赖：epoch 秒即够排序与去重）。
fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// CLI 插件的暂存目录（`gemini extensions install` / `copilot plugin install` 吃本地目录）。
/// 按包分开：卸 ponytail 不能顺手删掉速率扩展的暂存。
fn staged_dir(app: ClientApp, pack: Pack, data_dir: &Path) -> PathBuf {
    data_dir
        .join(format!("{}-ext-stage", app.as_str()))
        .join(pack.pkg())
}

fn write_stage(dir: &Path, files: &[(&'static str, &'static str)]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建暂存目录失败 {}: {e}", dir.display()))?;
    for (rel, src) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&path, src).map_err(|e| format!("写暂存插件失败 {}: {e}", path.display()))?;
    }
    Ok(())
}

// ── 配置文件的外科手术 ──────────────────────────────────────────────────────

/// 状态线命令：脚本绝对路径 + 本端代理名 + `--statusline`。
fn statusline_command(app: ClientApp, dir: &Path) -> String {
    format!(
        "python3 {} --app {} --statusline",
        dir.join("llm-switch-rate.py").display(),
        app.as_str()
    )
}

/// 改 `settings.json` 的 `statusLine`（on = 指向我们，off = 撤掉我们那一条）。
///
/// 只动 `statusLine` 一个键，其余原样保留；文件解析不了直接报错，不覆写用户配置。
fn patch_statusline(
    app: ClientApp,
    dir: &Path,
    backup_dir: &Path,
    on: bool,
) -> Result<(&'static str, Option<String>), String> {
    let path = dir.join("settings.json");
    let mut root: serde_json::Value = match fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t)
            .map_err(|e| format!("{} 不是合法 JSON：{e}", path.display()))?,
        Err(_) => serde_json::json!({}),
    };
    if !root.is_object() {
        return Err(format!("{} 顶层不是对象，拒绝改写", path.display()));
    }
    let want = statusline_command(app, dir);
    let cur = root["statusLine"]["command"].as_str().map(str::to_string);
    let ours = cur.as_deref() == Some(want.as_str());
    if on {
        if ours && root["statusLine"]["refreshInterval"].as_i64() == Some(1) {
            return Ok(("unchanged", None));
        }
        let backup = backup_json(&path, backup_dir, "settings.json")?;
        root["statusLine"] = serde_json::json!({
            "type": "command",
            "command": want,
            "refreshInterval": 1,
        });
        write_json(&path, &root)?;
        return Ok((
            if cur.is_some() {
                "updated"
            } else {
                "installed"
            },
            backup,
        ));
    }
    if cur.is_none() {
        return Ok(("unchanged", None));
    }
    if !ours {
        // 状态线指向别处（用户自己的脚本）：不动它。
        return Ok(("unchanged", None));
    }
    let backup = backup_json(&path, backup_dir, "settings.json")?;
    root.as_object_mut().map(|m| m.remove("statusLine"));
    write_json(&path, &root)?;
    Ok(("removed", backup))
}

/// 登记项：deveco 用裸路径（旧 opencode tui.json 用 `file://` URI，现仅撤残留时涉及）。
fn register_entry(dir: &Path, plugin: &str, uri: bool) -> String {
    let abs = dir.join(plugin);
    if uri {
        format!("file://{}", abs.display())
    } else {
        abs.display().to_string()
    }
}

/// `plugin` 数组里有没有这一项 → `Some(true)` 在位 / `Some(false)` 数组在但没我们 /
/// `None` 连数组都没有。文本级判定（JSONC 也能读），写回同样走文本级增删，
/// 这样注释与排版一个字都不动。
fn plugin_entry(text: &str, entry: &str) -> Option<bool> {
    let (_, arr) = split_plugin_array(text)?;
    Some(arr.contains(entry))
}

/// 找到 `"plugin"` 键的数组：返回 (数组前的文本, 数组内容含方括号)。
fn split_plugin_array(text: &str) -> Option<(usize, String)> {
    let key = text.find("\"plugin\"")?;
    let colon = text[key..].find(':')? + key;
    let open = text[colon..].find('[')? + colon;
    let mut depth = 0usize;
    for (i, ch) in text[open..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open, text[open..=open + i].to_string()));
                }
            }
            _ => {}
        }
    }
    None
}

/// 增（on）/ 撤（off）`plugin` 数组里我们自己那一项。
fn patch_plugin_entry(
    path: &Path,
    entry: &str,
    backup_dir: &Path,
    tag: &str,
    on: bool,
) -> Result<(&'static str, Option<String>), String> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let had_file = path.exists();
    let (open, arr) = split_plugin_array(&text).unwrap_or_default();
    let has = arr.contains(entry);
    if on && has {
        return Ok(("unchanged", None));
    }
    if !on && !has {
        return Ok(("unchanged", None));
    }
    let backup = if had_file {
        backup_dir_backup(path, backup_dir, tag)?
    } else {
        None
    };
    let next = if on {
        if arr.is_empty() {
            // 没有 plugin 键：在顶层对象开头插一条。
            let brace = text.find('{').map(|i| i + 1).unwrap_or(0);
            let mut s = String::new();
            s.push_str(&text[..brace]);
            s.push_str(&format!("\n  \"plugin\": [\"{entry}\"],"));
            s.push_str(&text[brace..]);
            s
        } else {
            let close = arr.rfind(']').unwrap_or(arr.len() - 1);
            let mut s = String::new();
            s.push_str(&text[..open]);
            s.push_str(&arr[..close]);
            s.push_str(&format!(", \"{entry}\""));
            s.push_str(&arr[close..]);
            s.push_str(&text[open + arr.len()..]);
            s
        }
    } else {
        let removed = arr.replace(entry, "");
        let cleaned = strip_empty_elements(
            &removed
                .replace("[, ", "[")
                .replace("[ ,", "[")
                .replace("[,", "["),
        );
        let mut s = String::new();
        s.push_str(&text[..open]);
        s.push_str(&cleaned);
        s.push_str(&text[open + arr.len()..]);
        s
    };
    fs::write(path, next).map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    Ok((if on { "installed" } else { "removed" }, backup))
}

/// 摘掉数组文本里的空串元素（`""`）。
///
/// 文本级删条目时（`arr.replace(entry, ""`）会把我们那一项换成空串留在原地——
/// DevEco 会把 `""` 当插件路径去解析，可能把整批插件带挂。这里连同紧邻的一侧逗号
/// 一起摘掉：只吃 `""` 与其相邻逗号，注释与排版其余一个字不动。
fn strip_empty_elements(s: &str) -> String {
    let mut out = s.to_string();
    while let Some(i) = out.find("\"\"") {
        let mut start = i;
        let mut end = i + 2;
        let head = out[..start].trim_end();
        if head.ends_with(',') {
            // 空元素前面有逗号：连逗号一起摘（摘后前一个元素与后一个元素仍以残留逗号相隔）。
            start = head.len() - 1;
        } else {
            let tail = out[end..].trim_start();
            if tail.starts_with(',') {
                // 空元素是首元素：连它后面那个逗号一起摘。
                end = out.len() - tail.len() + 1;
            }
        }
        out.replace_range(start..end, "");
    }
    out
}

/// OpenCode 旧版把插件登记在 `tui.json`（V2 内核已不读该文件）：装与卸都顺手撤掉
/// 我们那一条残留登记。只摘我们那一项，`theme` 等用户配置一个字不动；
/// 没有残留时什么都不做（不建文件、不产生结果行）。
fn retire_stale_tui_json(app: ClientApp, dir: &Path, backup_dir: &Path) -> Option<InstallResult> {
    if app != ClientApp::OpenCode {
        return None;
    }
    let entry = register_entry(dir, "plugins/llm-switch-rate/tui.js", true);
    let (action, backup) =
        patch_plugin_entry(&dir.join("tui.json"), &entry, backup_dir, "tui.json", false).ok()?;
    if action == "removed" {
        Some(InstallResult {
            name: "tui.json · 旧登记",
            action: "removed",
            backup,
        })
    } else {
        None
    }
}

/// 备份一个配置文件到备份目录。
fn backup_dir_backup(path: &Path, backup_dir: &Path, tag: &str) -> Result<Option<String>, String> {
    let prev = fs::read(path).ok().filter(|b| !b.is_empty());
    match prev {
        Some(prev) => {
            let b = backup_path(backup_dir, tag)?;
            fs::write(&b, &prev).map_err(|e| format!("备份失败 {}: {e}", b.display()))?;
            Ok(Some(b.display().to_string()))
        }
        None => Ok(None),
    }
}

fn backup_json(path: &Path, backup_dir: &Path, tag: &str) -> Result<Option<String>, String> {
    backup_dir_backup(path, backup_dir, tag)
}

fn write_json(path: &Path, v: &serde_json::Value) -> Result<(), String> {
    let s = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    fs::write(path, format!("{s}\n")).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

// ── 端点发现文件（代理侧扩展唯一的取数入口）─────────────────────────────────

/// 发现文件名：代理侧的速率扩展读它找到网关地址与令牌。
pub const ENDPOINT_FILE: &str = "rate-ep.json";

/// 写端点发现文件（纯函数：只写 `data_dir`，无副作用，测试可放心调用）。
///
/// 各家扩展跑在客户端进程里，拿不到 llm-switch 的运行时端口（胶囊可以热改），
/// 所以端口与令牌的真值由这里落一份只读快照；扩展只读，不改。
///
/// 从前这里内嵌「往默认 `~/.llm-switch/` 补写一份」的副作用，于是单测拿测试值
/// （8787/tk）把真实发现文件污染掉、八家扩展集体断供——补写逻辑拆到
/// `write_endpoint_file_for_app`，运行时走那边，测试只走这里。
pub fn write_endpoint_file(
    data_dir: &Path,
    host: &str,
    port: u16,
    token: Option<&str>,
) -> Result<PathBuf, String> {
    let text = endpoint_file_text(host, port, token);
    write_ep_at(data_dir, &text)
}

/// 运行时入口（启动与改端口两处调用）：写 `data_dir` 之外，当默认 `~/.llm-switch/`
/// 已存在且与 `data_dir` 不同再补一份——脚本端只认 `LLM_SWITCH_DATA_DIR` 与默认目录，
/// 不补这一份它们就集体取不到数；目录不存在则不替用户新建（不拿发现文件去撞用户挑好的布局）。
pub fn write_endpoint_file_for_app(
    data_dir: &Path,
    host: &str,
    port: u16,
    token: Option<&str>,
) -> Result<PathBuf, String> {
    let text = endpoint_file_text(host, port, token);
    let path = write_ep_at(data_dir, &text)?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".llm-switch");
    mirror_endpoint_file(data_dir, &home, &text);
    Ok(path)
}

/// 补写默认目录那份。home 显式传参以便单测用临时目录验证，绝不碰真实家目录。
fn mirror_endpoint_file(data_dir: &Path, home: &Path, text: &str) {
    if home != data_dir && home.is_dir() {
        if let Err(e) = write_ep_at(home, text) {
            tracing::warn!(target: "llm_switch::ext", "默认目录的速率发现文件补写失败：{e}");
        }
    }
}

fn endpoint_file_text(host: &str, port: u16, token: Option<&str>) -> String {
    let host = match host {
        "" | "0.0.0.0" | "::" | "*" => "127.0.0.1".to_string(),
        other => other.to_string(),
    };
    let body = serde_json::json!({
        "base": format!("http://{host}:{port}"),
        "token": token.unwrap_or_default(),
        "port": port,
        "version": env!("CARGO_PKG_VERSION"),
    });
    format!("{}\n", body)
}

fn write_ep_at(dir: &Path, text: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败 {}: {e}", dir.display()))?;
    let path = dir.join(ENDPOINT_FILE);
    fs::write(&path, text).map_err(|e| format!("写 {ENDPOINT_FILE} 失败: {e}"))?;
    Ok(path)
}

// ── 登录 shell ──────────────────────────────────────────────────────────────

/// 官方 CLI 探测的超时：这些命令是 Node CLI，冷启动就几秒，但不许把状态页顶死。
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
/// 装 / 卸的超时：装扩展要拉依赖、跑钩子，给充；再久就当挂了。
const CLI_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(240);
/// 在登录 shell 里跑一条固定命令（GUI 进程的 PATH 不含 nvm/homebrew，装不上 CLI）。
/// 命令一律是编译期常量模板，只允许 `{dir}` / `{pkg}` 两个占位（值由本模块生成）。
/// stdin 接 /dev/null，超时按 pid 收尾——绝不无限等一个 CLI 的确认提示。
pub(crate) fn run_login_shell(cmd: &str, timeout: std::time::Duration) -> Result<String, String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string());
    let mut child = std::process::Command::new(&shell)
        .arg("-lc")
        .arg(cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("登录 shell 跑 {cmd} 失败: {e}"))?;
    let pid = child.id();
    // 输出先在线程里抽干（管道塞满会把子进程顶住），退出码用 try_wait 轮询卡超时：
    // `output()` 没有超时，而 CLI 一旦问问（装扩展的信任确认就是）能挂到天荒地老，
    // 把安装任务顶死在这里。
    let (otx, orx) = std::sync::mpsc::channel();
    if let Some(mut out) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut out, &mut s);
            let _ = otx.send(s);
        });
    }
    let (etx, erx) = std::sync::mpsc::channel();
    if let Some(mut err) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut err, &mut s);
            let _ = etx.send(s);
        });
    }
    let started = std::time::Instant::now();
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) => {
                if started.elapsed() > timeout {
                    // 超时就按 pid 收尾（子进程还在等一个问不到的确认）。
                    let _ = std::process::Command::new("kill")
                        .arg(pid.to_string())
                        .status();
                    return Err(format!(
                        "`{cmd}` 超时 {}s 未返回（多半在等一个问不到的确认）",
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            Err(e) => return Err(format!("等 `{cmd}` 退出失败: {e}")),
        }
    };
    let text = orx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or_default();
    let err = erx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or_default();
    if !ok && text.trim().is_empty() {
        return Err(format!("`{cmd}` 失败: {}", err.trim()));
    }
    Ok(format!("{text}{err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pick<'a>(rows: &'a [InstallResult], name: &str) -> &'a InstallResult {
        rows.iter()
            .find(|r| r.name == name)
            .expect("结果里应有该项")
    }

    fn env_in(tmp: &Path) -> TakeoverEnv {
        TakeoverEnv::with_roots(tmp.to_path_buf(), tmp.join("backups"))
    }

    // 以下四个包装把「按速率包跑」收口在一处：既有断言讲的是速率扩展的落盘与登记，
    // 逐条拖一个 `Pack::Rate` 只会把测试淹掉。ponytail 包的清单与版本判定在
    // `services/ponytail.rs` 自己的测试里跑。
    fn items(app: ClientApp) -> &'static [Item] {
        super::items(app, Pack::Rate)
    }
    fn status(app: ClientApp, env: &TakeoverEnv, data: &Path) -> Result<Vec<ExtStatus>, String> {
        super::status(app, Pack::Rate, env, data)
    }
    fn install_all(
        app: ClientApp,
        env: &TakeoverEnv,
        data: &Path,
        backups: &Path,
    ) -> Result<Vec<InstallResult>, String> {
        super::install_all(app, Pack::Rate, env, data, backups)
    }
    fn uninstall_all(
        app: ClientApp,
        env: &TakeoverEnv,
        data: &Path,
        backups: &Path,
    ) -> Result<Vec<InstallResult>, String> {
        super::uninstall_all(app, Pack::Rate, env, data, backups)
    }

    /// 每个可接管的代理（含 copilot 门面）都有清单，且首项就是速率扩展。
    #[test]
    fn every_agent_has_the_rate_extension() {
        let mut apps: Vec<ClientApp> = ClientApp::FILE_CLIENTS.to_vec();
        apps.push(ClientApp::Copilot);
        for app in apps {
            let its = items(app);
            assert!(!its.is_empty(), "{:?} 应有安装清单", app);
            let src = match its[0].kind {
                Kind::File { source, .. } => source,
                _ => panic!("{:?} 首项应是速率扩展文件", app),
            };
            assert!(
                src.contains("admin/rates/line"),
                "{:?} 的速率扩展必须从网关速率出口取数",
                app
            );
        }
    }

    #[test]
    fn status_without_install_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_in(tmp.path());
        for app in [ClientApp::Pi, ClientApp::Claude, ClientApp::OpenCode] {
            let rows = status(app, &env, &tmp.path().join("data")).unwrap();
            assert_eq!(rows.len(), items(app).len());
            assert!(
                rows.iter().all(|r| r.state == "missing"),
                "{:?} 未装应报 missing: {rows:?}",
                app
            );
        }
    }

    #[test]
    fn install_is_idempotent_and_backfills_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_in(tmp.path());
        let data = tmp.path().join("data");
        let backups = tmp.path().join("backups");
        let first = install_all(ClientApp::Pi, &env, &data, &backups).unwrap();
        assert!(first.iter().all(|r| r.action == "installed"));
        assert_eq!(
            fs::read(config_dir(ClientApp::Pi, &env).join("llm-switch-rate.ts")).unwrap(),
            RATE_PI_TS.as_bytes()
        );
        let rows = status(ClientApp::Pi, &env, &data).unwrap();
        assert!(rows.iter().all(|r| r.state == "installed"), "{rows:?}");
        let second = install_all(ClientApp::Pi, &env, &data, &backups).unwrap();
        assert!(second.iter().all(|r| r.action == "unchanged"));
        assert!(!backups.exists(), "无差异不该产生备份");
    }

    /// claude / codebuddy：脚本落盘 + settings.json 的 statusLine 登记，卸载反向撤销。
    #[test]
    fn statusline_agents_patch_and_revert_settings() {
        for app in [ClientApp::Claude, ClientApp::CodeBuddy] {
            let tmp = tempfile::tempdir().unwrap();
            let env = env_in(tmp.path());
            let data = tmp.path().join("data");
            let backups = tmp.path().join("backups");
            let dir = config_dir(app, &env);
            fs::create_dir_all(&dir).unwrap();
            // 用户已有的设置必须一个字不丢。
            fs::write(
                dir.join("settings.json"),
                serde_json::json!({ "theme": "dark", "env": { "X": "1" } }).to_string(),
            )
            .unwrap();

            install_all(app, &env, &data, &backups).unwrap();
            let v: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(dir.join("settings.json")).unwrap())
                    .unwrap();
            assert_eq!(v["theme"], "dark", "别人的键不能动");
            assert_eq!(v["env"]["X"], "1");
            assert_eq!(v["statusLine"]["command"], statusline_command(app, &dir));
            assert_eq!(v["statusLine"]["refreshInterval"], 1);
            assert_eq!(
                status(app, &env, &data).unwrap()[1].state,
                "installed",
                "登记在位应报 installed"
            );

            // 手改状态线 → outdated（与源码逐字节比对同一套三态）。
            fs::write(
                dir.join("settings.json"),
                serde_json::json!({ "statusLine": { "command": "echo mine" } }).to_string(),
            )
            .unwrap();
            assert_eq!(status(app, &env, &data).unwrap()[1].state, "outdated");

            uninstall_all(app, &env, &data, &backups).unwrap();
            let v: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(dir.join("settings.json")).unwrap())
                    .unwrap();
            assert_eq!(
                v["statusLine"]["command"], "echo mine",
                "状态线已被用户改去别处：卸载不该拆掉它"
            );
            assert!(!dir.join("llm-switch-rate.py").exists(), "脚本应删掉");

            // 重新装回我们的登记，再卸载才应反向撤掉。
            install_all(app, &env, &data, &backups).unwrap();
            uninstall_all(app, &env, &data, &backups).unwrap();
            let v: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(dir.join("settings.json")).unwrap())
                    .unwrap();
            assert!(v.get("statusLine").is_none(), "卸载应撤掉我们自己的状态线");
        }
    }

    /// 状态线指向用户自己的脚本时，卸载不许顺手拆掉它。
    #[test]
    fn statusline_owned_by_user_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_in(tmp.path());
        let dir = config_dir(ClientApp::Claude, &env);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("settings.json"),
            serde_json::json!({ "statusLine": { "command": "my-own.sh" } }).to_string(),
        )
        .unwrap();
        uninstall_all(
            ClientApp::Claude,
            &env,
            &tmp.path().join("data"),
            &tmp.path().join("b"),
        )
        .unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(
            v["statusLine"]["command"], "my-own.sh",
            "不是我们装的就不该动"
        );
    }

    /// TUI 插件登记：写文件 + 配置文件 plugin 数组增撤，注释与其余键不动；
    /// 撤登记不许在数组里剩空串元素（`""` 会被 DevEco 当插件路径解析，可能整批挂掉）。
    #[test]
    fn tui_register_roundtrip_keeps_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_in(tmp.path());
        let data = tmp.path().join("data");
        let backups = tmp.path().join("backups");
        let dir = config_dir(ClientApp::DevEco, &env);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("deveco.jsonc"),
            "{\n  // 用户的注释\n  \"model\": \"x/y\",\n  \"plugin\": [\"/other/one.ts\"]\n}\n",
        )
        .unwrap();

        install_all(ClientApp::DevEco, &env, &data, &backups).unwrap();
        let text = fs::read_to_string(dir.join("deveco.jsonc")).unwrap();
        assert!(text.contains("用户的注释"), "注释必须原样留着");
        assert!(text.contains("/other/one.ts"), "别人的插件条目不能丢");
        assert!(text.contains("plugin/llm-switch-rate.ts"));
        assert!(dir.join("plugin/llm-switch-rate.ts").exists());
        assert_eq!(
            status(ClientApp::DevEco, &env, &data).unwrap()[1].state,
            "installed"
        );

        uninstall_all(ClientApp::DevEco, &env, &data, &backups).unwrap();
        let text = fs::read_to_string(dir.join("deveco.jsonc")).unwrap();
        assert!(!text.contains("llm-switch-rate"), "卸载应撤掉登记: {text}");
        assert!(text.contains("/other/one.ts"));
        assert!(!text.contains("\"\""), "撤登记不许剩空串元素: {text}");
        assert!(!dir.join("plugin/llm-switch-rate.ts").exists());
    }

    /// opencode（V2）：只落 `plugins/llm-switch-rate/tui.ts`，靠自动发现装载——
    /// 不再登记 tui.json（V2 内核根本不读它），也不替用户建这个文件。
    #[test]
    fn opencode_installs_auto_discovered_tui_ts() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_in(tmp.path());
        let data = tmp.path().join("data");
        let backups = tmp.path().join("backups");
        let dir = config_dir(ClientApp::OpenCode, &env);

        install_all(ClientApp::OpenCode, &env, &data, &backups).unwrap();
        let path = dir.join("plugins/llm-switch-rate/tui.ts");
        assert_eq!(fs::read(&path).unwrap(), RATE_TUI_TS.as_bytes());
        assert!(!dir.join("tui.json").exists(), "不该替用户建 tui.json");
        assert!(status(ClientApp::OpenCode, &env, &data).unwrap()[0].state == "installed");

        uninstall_all(ClientApp::OpenCode, &env, &data, &backups).unwrap();
        assert!(!path.exists(), "卸载应删掉插件文件");
        assert!(!dir.join("tui.json").exists());
    }

    /// 旧版登记在 tui.json 的残留（file:// URI 指向已退役的 tui.js）：
    /// 装与卸都顺手撤掉那一条，theme 等用户配置与别人的插件条目原样保留。
    #[test]
    fn stale_tui_json_registration_is_retired_surgically() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_in(tmp.path());
        let data = tmp.path().join("data");
        let backups = tmp.path().join("backups");
        let dir = config_dir(ClientApp::OpenCode, &env);
        fs::create_dir_all(&dir).unwrap();
        let stale = format!(
            "file://{}",
            dir.join("plugins/llm-switch-rate/tui.js").display()
        );
        fs::write(
            dir.join("tui.json"),
            format!(
                "{{\n  // 用户的主题\n  \"theme\": {{ \"name\": \"everforest\" }},\n  \"plugin\": [\"{stale}\", \"/other/plugin.ts\"]\n}}\n"
            ),
        )
        .unwrap();

        install_all(ClientApp::OpenCode, &env, &data, &backups).unwrap();
        let text = fs::read_to_string(dir.join("tui.json")).unwrap();
        assert!(!text.contains(&stale), "我们的残留登记应被撤掉: {text}");
        assert!(text.contains("用户的主题"), "注释保留");
        assert!(text.contains("everforest"), "主题保留");
        assert!(text.contains("/other/plugin.ts"), "别人的插件条目保留");

        uninstall_all(ClientApp::OpenCode, &env, &data, &backups).unwrap();
        let text = fs::read_to_string(dir.join("tui.json")).unwrap();
        assert!(
            !text.contains("llm-switch-rate"),
            "卸载同样不留残留: {text}"
        );
        assert!(text.contains("everforest"));
    }

    /// 上一代各家速率脚本：装与卸都顺手清掉并留备份。
    #[test]
    fn legacy_scripts_are_retired_with_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_in(tmp.path());
        let dir = config_dir(ClientApp::Codex, &env);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("codex-rate.py"), b"old self-measured script").unwrap();
        let out = install_all(
            ClientApp::Codex,
            &env,
            &tmp.path().join("data"),
            &tmp.path().join("backups"),
        )
        .unwrap();
        assert_eq!(pick(&out, "codex-rate.py").action, "removed");
        assert!(pick(&out, "codex-rate.py").backup.is_some());
        assert!(!dir.join("codex-rate.py").exists());
        assert!(dir.join("skills/llm-switch-rate/SKILL.md").exists());
    }

    /// 挂住的 CLI 必须被超时掐掉——装扩展的信任确认就是这种会永远等下去的问。
    #[test]
    fn login_shell_timeout_kills_hanging_command() {
        let t = std::time::Instant::now();
        let err = run_login_shell("sleep 30", std::time::Duration::from_millis(400))
            .expect_err("挂住的命令必须报错而不是挂住调用方");
        assert!(err.contains("超时"), "要说清是超时：{err}");
        assert!(
            t.elapsed() < std::time::Duration::from_secs(10),
            "不许真等 30s"
        );
    }

    /// 固定命令跑通时 stdout 与 stderr 都要带回来（CLI 常把结果打到 stderr）。
    #[test]
    fn login_shell_collects_both_streams() {
        let out = run_login_shell(
            "echo to-stdout; echo to-stderr >&2",
            std::time::Duration::from_secs(30),
        )
        .unwrap();
        assert!(
            out.contains("to-stdout") && out.contains("to-stderr"),
            "{out}"
        );
    }

    #[test]
    fn endpoint_file_is_what_scripts_read() {
        let tmp = tempfile::tempdir().unwrap();
        let p = write_endpoint_file(tmp.path(), "0.0.0.0", 8787, Some("tk")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["base"], "http://127.0.0.1:8787", "通配地址写回环");
        assert_eq!(v["token"], "tk");
    }

    /// 单测曾借 `write_endpoint_file` 的补写分支把测试值写进真实 `~/.llm-switch/`，
    /// 污染线上发现文件（见 docs/fix/rate-ep-test-pollution.md）。补写逻辑拆到
    /// `mirror_endpoint_file` 并显式传 home，这里用临时目录验证两条口径：
    /// 默认目录不存在不替用户新建；存在才补写。
    #[test]
    fn mirror_endpoint_file_only_hits_existing_home() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let home = tmp.path().join("home/.llm-switch");
        fs::create_dir_all(&data).unwrap();
        let text = endpoint_file_text("0.0.0.0", 9999, None);
        mirror_endpoint_file(&data, &home, &text);
        assert!(!home.exists(), "默认目录不存在时不得替用户新建");
        fs::create_dir_all(&home).unwrap();
        mirror_endpoint_file(&data, &home, &text);
        assert!(home.join(ENDPOINT_FILE).exists(), "默认目录已存在时应补写");
    }
}
