//! 连接归属：把「此刻是谁在跟网关说话」从内核 socket 表里挖出来。
//!
//! 为什么走 socket 而不是请求头：各家 CLI 没有统一的身份头（claude 有
//! `x-claude-code-session-id`，pi / codex / opencode / DevEco / CodeBuddy 什么都
//! 不发），从 body 猜身份更是毫无依据。但 TCP 是诚实的——网关 accept 到的每条
//! 连接都有对端端口，`lsof` 能把它反查成进程，进程 argv 里就写着它是谁。
//!
//! 成本与缓存：`lsof` 一次几十毫秒，绝不能每请求跑一次。keep-alive 下一条连接
//! 扛得住几十次请求，所以按**对端端口**缓存（连接活多久它就不变）；整张表另有
//! SCAN_TTL 的扫描冷却，进程名 → 代理名的判定再按 pid 缓存一层（进程不死不变）。
//! 查不到一律 `None`：浮窗退回「显示全部」，而不是猜一个把数据串到别人身上。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 整表扫描冷却：这段时间内新来的连接复用上一张表。
const SCAN_TTL: Duration = Duration::from_millis(1500);
/// 单连接归属的缓存寿命（对端端口被系统回收重用也就这个量级）。
const PEER_TTL: Duration = Duration::from_secs(120);
/// 撞空补试的**睡眠**间隔（第一次补试不睡，见 [`lookup_retry`]）。
///
/// 为什么补试而不是事后补判：归属在中间件里定，请求一旦挂号，`app` 就跟着那条在飞
/// 记录过完一生——事后才发现认不出，速率线这一代早就空过去了。补试只落在「认不出」
/// 这条路径上（认得出走缓存，零成本），代价可控。典型撞空：连接刚建、lsof 冷却期
/// （[`SCAN_TTL`]）里吃的是上一张表。
const RETRY_GAP_MS: u64 = 200;
/// **认不出**时的缓存寿命。认得出可以钉 120s（进程活着身份就不变），认不出不行：
/// 一次撞空（lsof 冷却期吃旧表、进程刚起来 argv 还没读全）就把这条连接/这个进程
/// 拉黑两分钟，速率线整代都显示「空闲」——用户看到的「刷新不及时」大半是它。
/// 短到能很快重试，又不至于每个请求都跑一遍外部命令。
const NEG_TTL: Duration = Duration::from_secs(2);
/// 外部命令的兜底超时：`lsof` 卡住也不能把请求拖死。
const CMD_TIMEOUT: Duration = Duration::from_millis(900);

/// 认得的代理名，与各客户端接管/扩展页的 `app` 键同一套口径。
///
/// 从 [`ClientApp::FILE_CLIENTS`] 派生（而非手写名单）：新增一个可接管代理只在
/// `ClientApp` 加一次，速率线名单自动跟上——从前手写漏了 gemini，它就永远认不出、
/// 不自动弹、也不在托盘里（「没覆盖项目支持的所有代理」的根因）。
pub static APPS: std::sync::LazyLock<Vec<&'static str>> = std::sync::LazyLock::new(|| {
    crate::takeover::ClientApp::FILE_CLIENTS
        .iter()
        .map(|a| a.as_str())
        .collect()
});

/// 一条连接背后的进程。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// 归一化后的代理名（见 [`APPS`]）。
    pub app: String,
    pub pid: u32,
    /// 该进程的控制终端（形如 `ttys009`），留给「吸附到终端」用；空 = 未知。
    pub tty: String,
}

/// 三层缓存的形状（提出来既让 clippy 别嫌类型套娃，读起来也省事）。
/// 对端端口 → 归属；认不出也占位，免得同一条连接反复查。
type PeerCache = HashMap<u16, (Instant, Option<Origin>)>;
/// `lsof` 一张表：对端端口 → (pid, 进程名)。
type PortTable = HashMap<u16, (u32, String)>;
type TableStamp = Option<(Instant, PortTable)>;
/// pid → (发现时刻, 代理名, tty)；进程活着就不变，省掉成堆的 `ps`。
type ProcEntry = (Instant, Option<String>, String);
type ProcCache = HashMap<u32, ProcEntry>;

fn peers() -> &'static Mutex<PeerCache> {
    static PEERS: OnceLock<Mutex<PeerCache>> = OnceLock::new();
    PEERS.get_or_init(Default::default)
}

/// `对端端口 -> (pid, comm)`：一次 `lsof` 建整张表。
fn table() -> &'static Mutex<TableStamp> {
    static TABLE: OnceLock<Mutex<TableStamp>> = OnceLock::new();
    TABLE.get_or_init(Default::default)
}

fn procs() -> &'static Mutex<ProcCache> {
    static PROCS: OnceLock<Mutex<ProcCache>> = OnceLock::new();
    PROCS.get_or_init(Default::default)
}

/// 反查对端进程。阻塞（起子进程），调用方负责放到 blocking 池上跑。
fn lookup(listen_port: u16, peer_port: u16) -> Option<Origin> {
    lookup_at(listen_port, peer_port, false)
}

/// 撞空后补试，专治「连接刚建立、上一张表里还没有它」。
///
/// 先**立刻**强制重扫（不睡）：最常见的撞空只是表太新，一次 `lsof` 就中，没理由让请求
/// 白等；仍空才短睡再扫一次（连接还在握手中，`lsof` 暂时看不见它）。
fn lookup_retry(listen_port: u16, peer_port: u16) -> Option<Origin> {
    lookup_at(listen_port, peer_port, true).or_else(|| {
        std::thread::sleep(Duration::from_millis(RETRY_GAP_MS));
        lookup_at(listen_port, peer_port, true)
    })
}

fn lookup_at(listen_port: u16, peer_port: u16, force: bool) -> Option<Origin> {
    let (pid, comm) = scan_table(listen_port, force).get(&peer_port).cloned()?;
    let (app, tty) = process(pid, &comm)?;
    Some(Origin { app, pid, tty })
}

/// `force = true` 跳过冷却：补试专用，把刚建立的连接扫进来。
fn scan_table(listen_port: u16, force: bool) -> PortTable {
    if !force {
        if let Some(fresh) = fresh_table() {
            return fresh;
        }
    }
    // 一波新连接同时未命中时只放一个去跑 lsof，其余直接吃旧表：网关一条 keep-alive
    // 连接扛几十次请求，让几十次请求排队等同一张表毫无意义。
    if SCANNING.swap(true, Ordering::SeqCst) {
        return table()
            .lock()
            .expect("origin table poisoned")
            .as_ref()
            .map(|(_, m)| m.clone())
            .unwrap_or_default();
    }
    let _guard = ScanGuard;
    // 先试绝对路径（.app 里的 PATH 未必有 /usr/sbin），再退回 PATH 查找。
    let filter = format!("-iTCP:{listen_port}");
    let args = [
        "-nP",
        filter.as_str(),
        "-sTCP:ESTABLISHED",
        "-Fpcn",
        "+c",
        "0",
    ];
    let map = match run("/usr/sbin/lsof", &args).or_else(|| run("lsof", &args)) {
        Some(out) => parse_lsof(&out, listen_port),
        None => HashMap::new(),
    };
    *table().lock().expect("origin table poisoned") = Some((Instant::now(), map.clone()));
    map
}

/// 冷却期内的现成表（None = 该重扫了）。
fn fresh_table() -> Option<PortTable> {
    let guard = table().lock().expect("origin table poisoned");
    guard
        .as_ref()
        .filter(|(at, _)| at.elapsed() < SCAN_TTL)
        .map(|(_, m)| m.clone())
}

/// 扫描互斥标记：同一时刻只让一个人去跑 `lsof`。
static SCANNING: AtomicBool = AtomicBool::new(false);

/// 释放 [`SCANNING`]：提前 return 也不能把闸门永久卡住。
struct ScanGuard;
impl Drop for ScanGuard {
    fn drop(&mut self) {
        SCANNING.store(false, Ordering::SeqCst);
    }
}

/// 解析 `lsof -Fpcn` 的机器可读输出：`p<pid>` / `c<comm>` 开一段，`n<本地->远端>` 收尾。
///
/// 同一条连接 lsof 会列**两端**各一行。本地端口等于监听端口的那行是我们自己，
/// 跳过；剩下的行里本地端口就是对端端口，映射到该行的 pid + 命令名。
pub fn parse_lsof(out: &str, listen_port: u16) -> HashMap<u16, (u32, String)> {
    let mut map = HashMap::new();
    let mut pid: Option<u32> = None;
    let mut comm = String::new();
    for line in out.lines() {
        if let Some(v) = line.strip_prefix('p') {
            pid = v.trim().parse().ok();
            comm.clear();
        } else if let Some(v) = line.strip_prefix('c') {
            comm = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix('n') {
            let (Some(p), Some(c)) = (pid, (!comm.is_empty()).then_some(comm.as_str())) else {
                continue;
            };
            // 形如 `127.0.0.1:50504->127.0.0.1:8787`；IPv6 带方括号，端口仍取最后一段。
            let local = v.split("->").next().unwrap_or("");
            if let Some(port) = local.rsplit(':').next().and_then(|p| p.parse().ok()) {
                if port != listen_port {
                    map.insert(port, (p, c.to_string()));
                }
            }
        }
    }
    map
}

/// pid → (代理名, tty)，带缓存。`comm` 是 lsof 给的进程名，够认直连的 CLI；
/// node/bun 起的 CLI 名字一律是 `node`，得再看 argv。
fn process(pid: u32, comm: &str) -> Option<(String, String)> {
    {
        let guard = procs().lock().expect("origin procs poisoned");
        if let Some((at, app, tty)) = guard.get(&pid) {
            let ttl = if app.is_some() { PEER_TTL } else { NEG_TTL };
            if at.elapsed() < ttl {
                return app.clone().map(|a| (a, tty.clone()));
            }
        }
    }
    let args = run("ps", &["-o", "args=", "-p", &pid.to_string()]).unwrap_or_default();
    let tty = run("ps", &["-o", "tty=", "-p", &pid.to_string()])
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    let app = classify(comm, &args);
    // 只记 comm 与判定结果：argv 里可能夹着用户提示词，不得入日志。
    tracing::debug!(
        target: "gateway",
        pid,
        comm,
        app = app.as_deref().unwrap_or("?"),
        "连接归属判定"
    );
    procs()
        .lock()
        .expect("origin procs poisoned")
        .insert(pid, (Instant::now(), app.clone(), tty.clone()));
    app.map(|a| (a, tty))
}

/// 自由文本标志：紧跟其后的那一段是用户/代理写的提示词，**绝不参与身份判定**。
///
/// 真实事故：Orca 用 `--prefill "..."` 把上一轮会话上下文喂给 claude，那段文字里
/// 出现了 `opencode`，整条 claude 连接就被判给了 opencode——速率线张冠李戴的根因。
const FREE_TEXT_FLAGS: &[&str] = &[
    "--prefill",
    "--prompt",
    "-p",
    "--message",
    "-m",
    "--system-prompt",
    "--append-system-prompt",
    "--instruction",
    "--instructions",
    "--context",
];

/// 包名/安装路径里的身份标记。只在**路径段**上比对，不拿整条命令行撞子串。
const HINTS: &[(&str, &str)] = &[
    ("@anthropic-ai/claude-code", "claude"),
    ("/codex-rs/", "codex"),
    ("@openai/codex", "codex"),
    ("opencode", "opencode"),
    ("deveco", "deveco"),
    ("codebuddy", "codebuddy"),
    // Gemini CLI 是 node 包（`@google/gemini-cli`），comm 常是 node，身份在 argv。
    ("@google/gemini-cli", "gemini"),
    ("copilot", "copilot"),
];

/// 进程名（comm）认代理。lsof 的 COMMAND 列只给 9 个字符（`claude.exe` → `claude.ex`），
/// 从前拿它做精确等值比对，截断后的名字永远认不出。这里先剥可执行后缀，再按前缀
/// 兜住截断；短名（pi）不做前缀兜底，免得 `pi` 去撞 `ping`。
fn comm_hit(comm: &str) -> Option<&'static str> {
    let raw = comm.rsplit('/').next().unwrap_or(comm).to_ascii_lowercase();
    APPS.iter()
        .find(|a| {
            if raw.starts_with(*a) {
                // 带可执行后缀的尾巴（`.exe` / 被截成 `.ex`）算同一家；`pin` 不能算 `pi`。
                let tail = &raw[a.len()..];
                tail.is_empty() || (tail.starts_with('.') && tail.len() <= 4)
            } else {
                // lsof 只给 9 个字符：comm 可能是真名被截短的前缀。
                raw.len() >= 6 && a.starts_with(raw.as_str())
            }
        })
        .copied()
}

/// 从 argv 里挑出「可执行/路径」候选，自由文本与纯标志一律丢掉。
///
/// `ps -o args=` 早把引号丢了，无法可靠还原参数边界，所以只信两类 token：argv[0]
/// （裸命令名，`claude --prefill ...` 就靠它）与含 `/` 的路径段。自由文本标志后面
/// 那一段直接跳过——那是提示词，不是身份。
fn argv_candidates(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for (i, tok) in args.split_whitespace().enumerate() {
        let t = tok.to_ascii_lowercase();
        if skip_next {
            skip_next = false;
            continue;
        }
        if FREE_TEXT_FLAGS.iter().any(|f| t == *f) {
            skip_next = true;
            continue;
        }
        if i > 0 && (t.starts_with('-') || !t.contains('/')) {
            continue;
        }
        out.push(t);
    }
    out
}

/// 单个 token 认代理：末节整段相等优先，其次包名标记。都不中返回 None。
///
/// 只看 argv[0] 不够：`#!/usr/bin/env node` 起的 CLI（pi 就是）comm 与 argv[0] 都是
/// `node`，身份写在后面那一段路径的末节上（`node /Users/x/.nvm/.../bin/pi`）——从前
/// pi 因此永远认不出，它那条约速率线一直空着。末节必须整段相等（脚本后缀剥掉再比），
/// 不能拿子串撞：`/usr/bin/ping` 里也含 `pi`。
fn token_hit(tok: &str) -> Option<&'static str> {
    let last = tok.rsplit('/').next().unwrap_or(tok);
    let base = [".mjs", ".cjs", ".js", ".ts", ".exe"]
        .iter()
        .find_map(|suf| last.strip_suffix(suf))
        .unwrap_or(last);
    if let Some(hit) = APPS.iter().find(|a| **a == base) {
        return Some(*hit);
    }
    HINTS
        .iter()
        .find(|(pat, _)| tok.contains(pat))
        .map(|(_, name)| *name)
}

/// 从进程名与 argv 里认代理。认不出返回 None（宁可少显示，不可张冠李戴）。
pub fn classify(comm: &str, args: &str) -> Option<String> {
    if let Some(hit) = comm_hit(comm) {
        return Some(hit.to_string());
    }
    let cands = argv_candidates(args);
    // argv[0] 先判：裸命令名（`claude --prefill ...`）是最强信号，命中就定，不再往后翻。
    if let Some(hit) = cands.first().and_then(|t| token_hit(t)) {
        return Some(hit.to_string());
    }
    cands
        .iter()
        .skip(1)
        .find_map(|t| token_hit(t))
        .map(|s| s.to_string())
}

/// 归属解析（异步入口）：命中缓存零成本，未命中才去跑外部命令。
pub async fn resolve(listen_port: u16, peer: SocketAddr) -> Option<Origin> {
    let port = peer.port();
    if let Some(hit) = cached(port) {
        return hit;
    }
    let mut found = tokio::task::spawn_blocking(move || lookup(listen_port, port))
        .await
        .ok()?;
    // 第一次没有不等于没有：强制重扫补试（只有撞空这条路才付这个钱）。
    // 只补试回环对端：远端连接在 lsof 里只有网关自己那一端，注定认不出是谁，
    // 不该让非本机客户端为一次必然落空的归属白等。
    if found.is_none() && peer.ip().is_loopback() {
        found = tokio::task::spawn_blocking(move || lookup_retry(listen_port, port))
            .await
            .ok()?;
    }
    cached_set(port, found.clone());
    found
}

fn cached(port: u16) -> Option<Option<Origin>> {
    let guard = peers().lock().expect("origin peers poisoned");
    guard
        .get(&port)
        .filter(|(at, o)| at.elapsed() < if o.is_some() { PEER_TTL } else { NEG_TTL })
        .map(|(_, o)| o.clone())
}

fn cached_set(port: u16, origin: Option<Origin>) {
    let mut guard = peers().lock().expect("origin peers poisoned");
    // 顺手清掉过期的，这张表随连接数增长，不能只进不出。
    guard.retain(|_, (at, o)| at.elapsed() < if o.is_some() { PEER_TTL } else { NEG_TTL });
    guard.insert(port, (Instant::now(), origin));
}

/// 跑外部命令并收 stdout。失败（不存在/超时/非零）一律 None，调用方自己退回兜底。
///
/// `Command::output()` 自己没有超时，所以丢到独立线程上用 `recv_timeout` 兜住
/// ——`lsof` 在负载高时会卡住，绝不能把网关的请求一起拖死。超时那条线程留在
/// 后台自己跑完（我们已经在 blocking 池上，多一个短命线程不心疼）。
fn run(prog: &str, args: &[&str]) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let prog = prog.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    std::thread::spawn(move || {
        let out = Command::new(&prog)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
        let _ = tx.send(out);
    });
    rx.recv_timeout(CMD_TIMEOUT).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_maps_client_side_only() {
        // 同一条连接的两端：我们的那行（本地端口 = 监听端口）必须被跳过。
        let out = "p61599\ncnode\nf27\nn127.0.0.1:50504->127.0.0.1:8787\n\
                   p80899\ncllm-switch\nf21\nn127.0.0.1:8787->127.0.0.1:50504\n";
        let map = parse_lsof(out, 8787);
        assert_eq!(map.get(&50504).map(|(_, c)| c.as_str()), Some("node"));
        assert_eq!(map.len(), 1, "服务端侧那行不该进表");
    }

    /// 真连接往返（手动跑，CI 不跑）：自己连自己，验证「对端端口 → pid」这条链
    /// 在真 `lsof` 上成立。解析器有单测钉格式，这条钉的是格式与现实相符。
    #[test]
    #[ignore = "起真连接并跑真 lsof：cargo test --lib gateway::origin -- --ignored"]
    fn live_loopback_roundtrip() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::net::TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        let table = scan_table(addr.port(), false);
        let found = table
            .get(&client.local_addr().unwrap().port())
            .expect("真连接应被 lsof 查到");
        assert_eq!(found.0, std::process::id(), "对端 pid 就是本进程");
        drop((client, server));
    }

    /// 速率线名单必须覆盖全部可接管代理（从 FILE_CLIENTS 派生，防手写漂移）。
    #[test]
    fn apps_covers_all_file_clients() {
        for app in crate::takeover::ClientApp::FILE_CLIENTS {
            assert!(
                APPS.contains(&app.as_str()),
                "速率线名单缺 {}",
                app.as_str()
            );
        }
    }

    /// gemini 是 node 包：comm=node 时靠 argv 认，comm=gemini 时直接命中。
    #[test]
    fn classify_recognizes_gemini() {
        assert_eq!(classify("gemini", "").as_deref(), Some("gemini"));
        assert_eq!(
            classify("node", "node /opt/@google/gemini-cli/dist/index.js").as_deref(),
            Some("gemini")
        );
        // basename 兼认 `.../bin/gemini`（剥脚本后缀整段相等）。
        assert_eq!(
            classify("node", "node /Users/x/.nvm/.../bin/gemini").as_deref(),
            Some("gemini")
        );
    }

    #[test]
    fn classify_prefers_comm_then_argv() {
        assert_eq!(classify("pi", "").as_deref(), Some("pi"));
        assert_eq!(classify("codex", "").as_deref(), Some("codex"));
        // node 起的 claude：argv 里才有身份
        assert_eq!(
            classify(
                "node",
                "node /opt/homebrew/@anthropic-ai/claude-code/cli.js"
            )
            .as_deref(),
            Some("claude")
        );
        // 子串不能乱撞：`/ping` 含 `pi`，但它不是 pi
        assert_eq!(classify("sh", "/usr/bin/ping 1.1.1.1").as_deref(), None);
        assert_eq!(classify("login", "").as_deref(), None);
        // `#!/usr/bin/env node` 起的 CLI：comm 与 argv[0] 都是 node，身份在后面的路径段
        assert_eq!(
            classify(
                "node",
                "node /Users/x/.nvm/versions/node/v22/bin/pi --continue"
            )
            .as_deref(),
            Some("pi")
        );
        assert_eq!(
            classify("node", "node /opt/pkg/dist/opencode.js run").as_deref(),
            Some("opencode")
        );
        // 路径里出现同名目录不算（末节必须整段相等）
        assert_eq!(
            classify("node", "node /srv/pi/server/main.js").as_deref(),
            None
        );
    }

    /// lsof 的 COMMAND 列被截到 9 字符（`claude.exe` → `claude.ex`），从前精确比对认不出。
    #[test]
    fn classify_tolerates_truncated_comm() {
        assert_eq!(classify("claude.ex", "").as_deref(), Some("claude"));
        assert_eq!(classify("claude.exe", "").as_deref(), Some("claude"));
        assert_eq!(classify("Codex", "").as_deref(), Some("codex"));
        // 短名不做前缀兜底：`pi` 不能把 `ping` 认成自己
        assert_eq!(classify("pi", "").as_deref(), Some("pi"));
        assert_eq!(classify("pin", "").as_deref(), None);
    }

    /// 真实现场：Orca 给 claude 的 `--prefill` 自由文本里写着 opencode，
    /// 整条连接曾被判给 opencode（速率线张冠李戴的根因）。
    #[test]
    fn classify_ignores_free_text_after_prefill() {
        let argv = "claude --plugin-dir /Users/x/Library/Application Support/orca/claude-agent-view/bed635 --dangerously-skip-permissions --prefill Continue work from the prior Orca session using opencode as provider";
        assert_eq!(classify("claude.ex", argv).as_deref(), Some("claude"));
        // comm 认不出时也不能被 --prefill 里的别家名字带走
        assert_eq!(classify("sh", argv).as_deref(), Some("claude"));
    }

    /// 提示词里提到别家代理不算身份；路径段才是身份。
    #[test]
    fn classify_ignores_prompt_mentioning_other_agents() {
        assert_eq!(
            classify(
                "node",
                "node /Users/x/.nvm/versions/node/v22/bin/pi --prompt 用 opencode 继续"
            )
            .as_deref(),
            Some("pi")
        );
        assert_eq!(
            classify(
                "node",
                "node /opt/homebrew/@anthropic-ai/claude-code/cli.js -p 让 codex 看一下"
            )
            .as_deref(),
            Some("claude")
        );
    }

    /// 各家真实命令行回归（argv 取自本机 ps）。
    #[test]
    fn classify_recognizes_real_command_lines() {
        assert_eq!(
            classify(
                "opencode",
                "/Users/x/.opencode/bin/opencode serve --service"
            )
            .as_deref(),
            Some("opencode")
        );
        assert_eq!(
            classify(
                "node",
                "/Users/x/.nvm/versions/node/v22/lib/node_modules/@openai/codex/node_modules/@openai/codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex"
            )
            .as_deref(),
            Some("codex")
        );
        assert_eq!(
            classify("node", "node /opt/pkg/dist/opencode.js run").as_deref(),
            Some("opencode")
        );
    }
}
