//! 槽位动作执行（C2，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）。
//!
//! 五个动作各一条执行路径，统一返回 `Result<(), String>`：调用方（`hid.rs` 手势出口）
//! 只负责「失败记一条、不静默」。键盘注入类动作必须在主线程调用（同 `inject.rs` 铁律），
//! 线程调度由调用方决定，这里不自己挪线程。
//!
//! 失败语义照 SayAll 口径：CGEvent post 没有回执，成功只能报「已投递」，不假装确认。

use crate::services::remote::actions::Action;
use crate::services::remote::{axfocus, inject, perms};

/// 执行一个槽动作。`Err` = 动作没做成（原因直接给人看）。
pub fn execute(action: &Action) -> Result<(), String> {
    match action {
        Action::Shortcut { shortcut } => inject::inject_shortcut(*shortcut),
        Action::Text { content } => inject_text(content),
        Action::OpenApp { bundle_id, args } => open_app(bundle_id, args),
        Action::AgentCli { app, args, cwd } => agent_cli(app, args, cwd),
        Action::FocusInput => axfocus::focus_frontmost_input(),
        Action::RunCommand { command } => run_command(command),
        // 宏在 `hid.rs` 就按 id 解析成步骤、逐步回主线程投递，这里收到未解析的引用＝调用方漏了。
        Action::Macro { id } => Err(format!("宏 {id} 未解析（内部错误）")),
    }
}

/// 注入文本到当前聚焦输入框。
///
/// 走 `inject::inject_text`（`CGEventKeyboardSetUnicodeString`，等价输入法上屏）而非
/// 计划原文的「剪贴板粘贴」：不碰用户剪贴板、不改输入法状态，任何输入法下都工作。
/// SecureInput（密码框等）会静默吃掉注入，这里提前查并明确报错，不许按了没反应。
fn inject_text(content: &str) -> Result<(), String> {
    if perms::secure_input_enabled() {
        return Err("系统处于安全输入状态（SecureInput），文本注入被系统拒绝".to_string());
    }
    inject::inject_text(content)
}

/// 打开/唤起 App：走 LaunchServices 原生语义——未运行即启动、已运行唤前，
/// 绝不重复起第二实例（`createsNewApplicationInstance = false`）。
/// 应用列表与「浏览…」选择器在 C7，本片按 bundle id 直填。
fn open_app(bundle_id: &str, args: &str) -> Result<(), String> {
    use objc2_app_kit::{NSWorkspace, NSWorkspaceOpenConfiguration};
    use objc2_foundation::NSString;

    let bundle_id = bundle_id.trim();
    if bundle_id.is_empty() {
        return Err("未填写 App 的 bundle id".to_string());
    }
    let ws = NSWorkspace::sharedWorkspace();
    let Some(url) = ws.URLForApplicationWithBundleIdentifier(&NSString::from_str(bundle_id)) else {
        return Err(format!("系统里没注册 bundle id 为 {bundle_id} 的 App"));
    };
    // 带启动参数时走 `open -b … --args`：`open` 原生支持传参，且默认就是「唤起已有实例、
    // 不另起第二实例」，与下面 NSWorkspace 的口径一致。无参仍走 NSWorkspace（既有已验证路径）。
    let args = split_args(args);
    if !args.is_empty() {
        let mut cmd = std::process::Command::new("open");
        cmd.arg("-b").arg(bundle_id).arg("--args").args(&args);
        cmd.spawn().map_err(|e| format!("打开 App 投递失败：{e}"))?;
        tracing::info!(target: "llm_switch::remote", bundle_id, n = args.len(), "遥控器动作：已投递打开 App（带启动参数）");
        return Ok(());
    }
    let conf = NSWorkspaceOpenConfiguration::new();
    conf.setActivates(true);
    conf.setCreatesNewApplicationInstance(false);
    // 完成回调传 None：启动是异步的，成败由系统日志与后续按键表现说话（同「不假装确认」口径）。
    ws.openApplicationAtURL_configuration_completionHandler(&url, &conf, None);
    tracing::info!(target: "llm_switch::remote", bundle_id, "遥控器动作：已投递打开 App");
    Ok(())
}

/// 登录 shell 异步跑一行命令：不阻塞按键、不等退出、无输出回显（细则见 C8）。
///
/// shell 解析与 `ext_install::run_login_shell` 同源（nvm/homebrew 的 PATH 才可用）。
/// 另起一个收尸线程等它退出：一是避免攒僵尸进程，二是把退出码记进日志留痕（C8 要求可追溯）。
fn run_command(command: &str) -> Result<(), String> {
    use std::process::{Command, Stdio};

    let command = command.trim();
    if command.is_empty() {
        return Err("未填写命令".to_string());
    }
    let shell = crate::services::ext_install::login_shell();
    let mut child = Command::new(&shell)
        .arg("-lc")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("命令投递失败：{e}"))?;
    let pid = child.id();
    tracing::info!(target: "llm_switch::remote", pid, command, "遥控器动作：命令已投递（登录 shell，不等退出、不回显）");
    std::thread::spawn(move || match child.wait() {
        Ok(code) => {
            tracing::info!(target: "llm_switch::remote", pid, ?code, "遥控器动作：命令已退出")
        }
        Err(e) => {
            tracing::warn!(target: "llm_switch::remote", pid, err = %e, "遥控器动作：等命令退出失败")
        }
    });
    Ok(())
}

/// 按 shell 引号规则把一行参数切成若干 arg（支持单/双引号，不支持转义与变量展开）：
/// `--model "gpt-5 codex" --yolo` → ["--model", "gpt-5 codex", "--yolo"]。
fn split_args(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for ch in raw.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    cur.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => {
                    quote = Some(ch);
                    has = true;
                }
                c if c.is_whitespace() => {
                    if has {
                        out.push(std::mem::take(&mut cur));
                        has = false;
                    }
                }
                c => {
                    cur.push(c);
                    has = true;
                }
            },
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// 单引号包一层给 shell（内部单引号按 `'"'"'` 规则转义）——路径与目录都走这里，
/// 参数串由用户自己写，原样交给 shell（本就是他自己的机器与自己的 CLI）。
fn sh_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

/// AppleScript 字符串字面量转义（\ 与 "）。
fn osa_quote(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out
}

/// 在终端里起一个代理 CLI（可带参数与工作目录）。
///
/// 为什么必须进终端：`codex` / `claude` 这类交互式 CLI 要 TTY，无终端跑法（`RunCommand`）
/// 起不来也看不见。装了 iTerm 用 iTerm，否则用系统自带 Terminal。
/// 可执行路径来自安装扫描的**登录 shell** 探测（GUI 进程 PATH 不含 nvm/homebrew）。
fn agent_cli(app: &str, args: &str, cwd: &str) -> Result<(), String> {
    use crate::takeover::ClientApp;

    let app = ClientApp::parse(app)?;
    let det = crate::services::agents_scan::cached(app)
        .ok_or_else(|| format!("{} 还没探测过（稍等或去代理页重扫一轮）", app.label()))?;
    let bin = det
        .bin_path
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "系统里没探测到 {} 的可执行文件（先在代理页装好/重扫）",
                app.label()
            )
        })?;
    let cwd_raw = cwd.trim();
    let dir = if cwd_raw.is_empty() {
        std::env::var("HOME").map_err(|_| "取不到主目录，请给动作填工作目录".to_string())?
    } else {
        let p = std::path::PathBuf::from(shellexpand_tilde(cwd_raw));
        if !p.is_dir() {
            return Err(format!("工作目录不存在：{cwd_raw}"));
        }
        p.to_string_lossy().to_string()
    };
    let cmdline = format!(
        "cd {} && {} {}",
        sh_quote(&dir),
        sh_quote(&bin),
        args.trim()
    );
    let script = if std::path::Path::new("/Applications/iTerm.app").exists() {
        format!(
            "tell application \"iTerm\"\nactivate\nset w to (create window with default profile)\ntell current session of w to write text \"{}\"\nend tell",
            osa_quote(&cmdline)
        )
    } else {
        format!(
            "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
            osa_quote(&cmdline)
        )
    };
    std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .spawn()
        .map_err(|e| format!("终端投递失败：{e}"))?;
    tracing::info!(target: "llm_switch::remote", app = app.as_str(), dir, "遥控器动作：已投递代理 CLI 到终端");
    Ok(())
}

/// `~` 展开成主目录（用户手填目录常写 `~/proj`）。
fn shellexpand_tilde(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    raw.to_string()
}
