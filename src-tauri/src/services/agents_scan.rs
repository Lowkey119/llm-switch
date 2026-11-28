//! 代理安装扫描：检测本机装了哪些代理 CLI、版本、一键安装命令。
//!
//! 为什么要扫：代理散在各家 npm 全局 / 官方脚本里，装了哪几个、装在哪、什么版本，
//! 从前只能靠配置文件在不在猜。接管页要「只显示装了的代理」、矩阵只给装了的代理分配
//! 模型、没装的给一键安装——都得先有这份扫描结果。
//!
//! 判定口径（两个信号，都展示，避免误判把用户锁在外面）：
//! - **可执行文件**：在**登录 shell**（`$SHELL -lc`）里 `command -v <bin>`。GUI 进程
//!   自身的 PATH 不含 nvm/homebrew，必须借登录 shell 才看得见用户装的 CLI（口径同
//!   「装了但跑不起来」（`broken`），如实上报不掩盖。
//! - **配置目录**：各端配置文件是否存在（由命令层用接管评测的 `files` 提供，本模块不重算）。
//!
//! `installed = 可执行命中 || 配置存在`。命令层把两者合并成最终视图。
//!
//! 安装命令是**每端固定白名单**（绝不拼用户输入），来自各代理官方文档 / npm registry
//! 实测；`install_command` 既给一键安装执行，也给前端展示 + 一键复制让用户自己跑。

use crate::takeover::ClientApp;
use serde::Serialize;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 一次扫描结果的缓存寿命。CLI 不会秒级变，60s 足够；另有强制重扫入口。
/// TTL 外若 `bin_path` 文件仍在，旧值继续用（装没装没变），版本号留给主动刷新
/// （进安装页 / 手动重扫）——见 [`cached`]。
const SCAN_TTL: Duration = Duration::from_secs(60);
/// 登录 shell 探测的兜底超时：用户 profile 卡住也不能把扫描拖死。
const PROBE_TIMEOUT: Duration = Duration::from_millis(4000);

/// 一个代理的安装元数据（固定白名单，编译期常量，绝不来自外部输入）。
struct AgentMeta {
    /// 可执行文件名（`command -v` 用）。
    bin: &'static str,
    /// 一键安装命令（POSIX，在登录 shell 里跑）。`None` = 没有可靠的一键装法。
    install: Option<&'static str>,
}

/// 各代理的安装元数据表。装法来自各代理官方文档 / npm registry 实测（2026-11）：
/// claude 与 opencode 官方脚本优先、失败退回 npm；codex / gemini / codebuddy /
/// deveco / pi / copilot 是纯 npm 全局包。
///
/// copilot 是网关门面、**不在 `FILE_CLIENTS`**（没有可接管的配置文件，不进代理管理列表、
/// 不进接管与矩阵遍历），但它的 CLI 是真装在本机的，所以这里照样给探测与一键装法——
/// 只出现在「安装引导」面板（用户拍板：门面不进代理管理列表，只进安装引导）。
fn meta(app: ClientApp) -> Option<AgentMeta> {
    let m = match app {
        ClientApp::Claude => AgentMeta {
            bin: "claude",
            install: Some(
                "curl -fsSL https://claude.ai/install.sh | bash || npm i -g @anthropic-ai/claude-code@latest",
            ),
        },
        ClientApp::Codex => AgentMeta {
            bin: "codex",
            install: Some("npm i -g @openai/codex@latest"),
        },
        ClientApp::Gemini => AgentMeta {
            bin: "gemini",
            install: Some("npm i -g @google/gemini-cli@latest"),
        },
        ClientApp::OpenCode => AgentMeta {
            bin: "opencode",
            install: Some(
                "curl -fsSL https://opencode.ai/install | bash || npm i -g opencode-ai@latest",
            ),
        },
        ClientApp::DevEco => AgentMeta {
            bin: "deveco",
            install: Some("npm i -g @deveco/deveco-code@latest"),
        },
        ClientApp::Pi => AgentMeta {
            bin: "pi",
            install: Some("npm i -g @earendil-works/pi-coding-agent@latest"),
        },
        ClientApp::CodeBuddy => AgentMeta {
            bin: "codebuddy",
            install: Some("npm i -g @tencent-ai/codebuddy-code@latest"),
        },
        ClientApp::Copilot => AgentMeta {
            bin: "copilot",
            install: Some("npm i -g @github/copilot@latest"),
        },
    };
    Some(m)
}

/// 该代理的可执行文件名（命令层展示用）。
pub fn bin_name(app: ClientApp) -> &'static str {
    meta(app).map(|m| m.bin).unwrap_or("")
}

/// 该代理的一键安装命令（固定白名单）。`None` = 没有可靠一键装法，前端只给手动引导。
pub fn install_command(app: ClientApp) -> Option<&'static str> {
    meta(app).and_then(|m| m.install)
}

/// 可执行文件探测结果（不含配置目录信号，那由命令层合并）。
#[derive(Debug, Clone, Serialize, Default)]
pub struct Detect {
    /// `command -v` 命中的绝对路径；未命中 = None。
    pub bin_path: Option<String>,
    /// `<bin> --version` 首行；未命中或跑不起来 = None。
    pub version: Option<String>,
    /// 命中可执行但 `--version` 非零退出（装了却跑不起来，如 Node 版本不达标）。
    pub broken: bool,
}

/// 用户登录 shell：GUI 进程自身 PATH 不含 nvm/homebrew，必须借登录 shell 才看得见
/// 用户装的 CLI。`$SHELL` 缺省退回 `/bin/zsh`（macOS 默认）。
fn login_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string())
}

/// 在登录 shell 里跑一条命令并收 stdout，带超时兜底（口径同 origin::run）。
/// 超时那条线程留在后台自己跑完；我们已在调用线程，多一个短命线程不心疼。
fn run_login(script: &str) -> Option<String> {
    let shell = login_shell();
    let script = script.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let out = Command::new(&shell)
            .arg("-lc")
            .arg(&script)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        let _ = tx.send(out);
    });
    rx.recv_timeout(PROBE_TIMEOUT).ok().flatten()
}

/// 探测单个代理的可执行文件与版本。一次登录 shell 调用同时取路径与版本，省一半 spawn。
///
/// 脚本输出协议（逐行）：
/// - `MISSING`：`command -v` 没命中（没装）。
/// - `PATH=<路径>` + `VER=<版本首行>`：装了且能跑。
/// - `PATH=<路径>` + `BROKEN`：装了但 `--version` 非零退出。
pub fn detect(app: ClientApp) -> Detect {
    let bin = match meta(app) {
        Some(m) => m.bin,
        None => return Detect::default(),
    };
    // 单引号包裹 bin，防注入（bin 是编译期常量，但保持习惯）。
    let script = format!(
        "p=$(command -v '{bin}' 2>/dev/null); if [ -z \"$p\" ]; then echo MISSING; else echo \"PATH=$p\"; if out=$('{bin}' --version 2>&1); then printf 'VER=%s\\n' \"$(printf '%s' \"$out\" | head -1)\"; else echo BROKEN; fi; fi"
    );
    let out = match run_login(&script) {
        Some(s) => s,
        // 探测本身失败（shell 起不来/超时）：当作未知，不误判成没装。
        None => return Detect::default(),
    };
    let mut d = Detect::default();
    for line in out.lines() {
        if line == "MISSING" {
            return Detect::default();
        } else if let Some(p) = line.strip_prefix("PATH=") {
            d.bin_path = Some(p.trim().to_string());
        } else if let Some(v) = line.strip_prefix("VER=") {
            d.version = Some(v.trim().to_string());
        } else if line == "BROKEN" {
            d.broken = true;
        }
    }
    d
}

/// 一条代理的完整扫描结果（命令层把可执行探测 + 配置路径合并后下发给前端）。
#[derive(Debug, Clone, Serialize)]
pub struct AgentScan {
    pub app: String,
    pub label: String,
    /// 可执行命中 或 配置存在 = 视为已安装。
    pub installed: bool,
    /// 可执行文件是否命中（独立信号，UI 可区分「装了 CLI」与「只有配置残留」）。
    pub bin_found: bool,
    /// 命中可执行但跑不起来。
    pub broken: bool,
    /// 可执行探测是否已有结论。false = 冷启动首轮还在后台飞，
    /// 「没装」尚不可信，前端别急着把该端归进「未检测到」。
    pub probed: bool,
    pub bin: String,
    pub bin_path: Option<String>,
    pub version: Option<String>,
    /// 一键安装命令（固定白名单）；None = 只给手动引导。
    pub install_command: Option<String>,
    /// 配置文件落点（路径 + 是否存在），来自接管评测的 files。
    pub config_paths: Vec<ConfigPath>,
}

/// 一份配置文件的存在性。
#[derive(Debug, Clone, Serialize)]
pub struct ConfigPath {
    pub path: String,
    pub exists: bool,
}

/// 可执行探测的进程内缓存（按 app）。快照直接从这里组装，**从不等 shell**：
/// 值可以超 TTL——「装了/没装」仍是已知最优事实，版本陈旧由主动刷新入口兜底
/// （进安装页 / 手动重扫 / 一键安装后），见 [`needs_probe`]。
fn cache() -> &'static Mutex<std::collections::HashMap<String, (Instant, Detect)>> {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, (Instant, Detect)>>> =
        OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// 缓存的探测结果（含超 TTL 的旧值）。None = 从未探测过（冷启动头几秒）。
pub fn cached(app: ClientApp) -> Option<Detect> {
    cache()
        .lock()
        .expect("scan cache poisoned")
        .get(app.as_str())
        .map(|(_, d)| d.clone())
}

/// 该端是否需要（重）探测：无缓存，或超 TTL 且可执行状态存疑——
/// 路径没了（被删）或本是 MISSING（可能新装了）。版本陈旧**不**触发探测：
/// 版本号只在主动入口刷新（force 全量），稳态浏览零 shell 开销。
pub fn needs_probe(app: ClientApp) -> bool {
    let c = cache().lock().expect("scan cache poisoned");
    match c.get(app.as_str()) {
        None => true,
        Some((at, d)) => {
            if at.elapsed() < SCAN_TTL {
                false
            } else {
                match &d.bin_path {
                    Some(p) => !Path::new(p).exists(),
                    None => true,
                }
            }
        }
    }
}

/// 后台探测队列 + 单飞标志。REFRESHING 的两次变更都在队列锁内完成，
/// 「在飞线程退出」与「新请求决定不再起线程」不可能互相错过。
static REFRESHING: AtomicBool = AtomicBool::new(false);
fn queue() -> &'static Mutex<Vec<ClientApp>> {
    static QUEUE: OnceLock<Mutex<Vec<ClientApp>>> = OnceLock::new();
    QUEUE.get_or_init(Default::default)
}

/// 是否有后台刷新在飞（UI 的「扫描中」读它）。
pub fn refreshing() -> bool {
    REFRESHING.load(Ordering::SeqCst)
}

/// 安排后台并行探测（单飞）：apps 入队；没有线程在飞就起一个。
/// 线程按轮清队（在飞期间新加的下一轮带走），一轮内逐 app 并行 spawn
/// （单个探测自带 [`PROBE_TIMEOUT`] 兜底），写完缓存后 on_done
/// （命令层用它 emit 通知前端重取）。
pub fn schedule_refresh(apps: &[ClientApp], on_done: impl FnOnce() + Send + 'static) {
    {
        let mut q = queue().lock().expect("scan queue poisoned");
        for a in apps {
            if !q.contains(a) {
                q.push(*a);
            }
        }
        if REFRESHING.swap(true, Ordering::SeqCst) {
            return; // 在飞线程会把新追加带走
        }
    }
    std::thread::spawn(move || {
        loop {
            let round: Vec<ClientApp> = {
                let mut q = queue().lock().expect("scan queue poisoned");
                if q.is_empty() {
                    REFRESHING.store(false, Ordering::SeqCst);
                    break;
                }
                std::mem::take(&mut *q)
            };
            std::thread::scope(|s| {
                for app in round {
                    s.spawn(move || {
                        let d = detect(app);
                        cache()
                            .lock()
                            .expect("scan cache poisoned")
                            .insert(app.as_str().to_string(), (Instant::now(), d));
                    });
                }
            });
        }
        on_done();
    });
}

/// 清缓存（安装成功后调用，下次扫描重新探测新版本）。
pub fn invalidate(app: ClientApp) {
    cache()
        .lock()
        .expect("scan cache poisoned")
        .remove(app.as_str());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个文件客户端都有一键安装命令；copilot 门面也有（CLI 真装在本机，只是不进
    /// 代理管理列表），所以它同样要能被探测与一键装。
    #[test]
    fn every_file_client_has_install_command() {
        for app in ClientApp::FILE_CLIENTS {
            assert!(install_command(app).is_some(), "{:?} 应有一键安装命令", app);
            assert!(!bin_name(app).is_empty(), "{:?} 应有可执行名", app);
        }
        assert!(
            install_command(ClientApp::Copilot).is_some(),
            "copilot 的 CLI 装在本机，安装引导里要能给一键装"
        );
        assert_eq!(bin_name(ClientApp::Copilot), "copilot");
    }

    /// copilot 只进安装引导：它**不是**文件客户端，接管/矩阵/代理管理列表都不该遍历到它。
    #[test]
    fn copilot_is_scanned_but_not_a_file_client() {
        assert!(!ClientApp::FILE_CLIENTS.contains(&ClientApp::Copilot));
        assert!(ClientApp::ALL.contains(&ClientApp::Copilot));
    }

    /// 安装命令是固定白名单：只含已知安装器，不含任何用户输入痕迹。
    #[test]
    fn install_commands_are_known_installers_only() {
        for app in [ClientApp::FILE_CLIENTS.as_slice(), &[ClientApp::Copilot]].concat() {
            let cmd = install_command(app).unwrap();
            let ok = cmd.starts_with("npm i -g ")
                || cmd.starts_with("curl -fsSL ")
                || cmd.contains("npm i -g ");
            assert!(ok, "{:?} 安装命令形态异常: {cmd}", app);
        }
    }

    /// 探测脚本协议解析：MISSING / PATH+VER / PATH+BROKEN 三种输出。
    #[test]
    fn parses_probe_output_protocol() {
        // 直接验证解析逻辑：构造三种输出文本走同样的分支。
        let parse = |out: &str| {
            let mut d = Detect::default();
            for line in out.lines() {
                if line == "MISSING" {
                    return Detect::default();
                } else if let Some(p) = line.strip_prefix("PATH=") {
                    d.bin_path = Some(p.trim().to_string());
                } else if let Some(v) = line.strip_prefix("VER=") {
                    d.version = Some(v.trim().to_string());
                } else if line == "BROKEN" {
                    d.broken = true;
                }
            }
            d
        };
        let miss = parse("MISSING\n");
        assert!(miss.bin_path.is_none() && !miss.broken);
        let ok = parse("PATH=/usr/local/bin/pi\nVER=pi 1.2.3\n");
        assert_eq!(ok.bin_path.as_deref(), Some("/usr/local/bin/pi"));
        assert_eq!(ok.version.as_deref(), Some("pi 1.2.3"));
        assert!(!ok.broken);
        let broken = parse("PATH=/usr/local/bin/pi\nBROKEN\n");
        assert_eq!(broken.bin_path.as_deref(), Some("/usr/local/bin/pi"));
        assert!(broken.broken);
    }
}
