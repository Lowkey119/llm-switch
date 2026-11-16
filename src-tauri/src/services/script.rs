//! 后端启停脚本：控制台页「启动 / 停止」按钮背后。
//!
//! 配置里存的是**本机脚本路径**（`backends.start_script` / `stop_script`，用户在
//! 控制台页「入口设置」卡里填），执行走直接 exec——`Command::new(path)`，内核按
//! shebang 换解释器，**不拼任何 shell 字符串、不带任何参数**。路径是用户可编辑的
//! 配置，拼进 `sh -c` 就是一整块注入面；直接 exec 则参数表恒空，无从注入。
//!
//! 暴露面：命令只经 Tauri IPC 提供，**不开 REST**。把「让远端触发本机脚本」这件事
//! 放到网关端口上，比现有的「改一条配置」暴露面大得多，而收益只是省一次点按钮。
//!
//! 启动的进程怎么收尾：`start` 不等脚本跑完（它本来就该一直跑着），但也不能放着
//! 不管——子进程退出时没人 `wait` 就会留下僵尸。所以起一个后台任务替它收尸。

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::types::BackendRow;

/// 启动脚本在这个窗口内退出就判定「没起来」。正常的服务脚本会一直挂着，留 3 秒
/// 只为让「文件不存在 / 没可执行位 / 一启动就崩」这类**立刻可见**的失败当场说清——
/// 否则按钮只会回一句「已启动」，而实际什么都没发生。
const START_GRACE: Duration = Duration::from_secs(3);

/// 停止脚本的等待上限。停止一般几秒内完事；超时说明脚本自己卡住了，如实报给用户，
/// 但不让这一条 IPC 无限期挂着。
const STOP_TIMEOUT: Duration = Duration::from_secs(60);

/// 输出只回尾部这么多字节：服务脚本一启动就刷几百行，整段塞进 IPC 没人看得动。
const OUTPUT_TAIL: usize = 4096;

/// 一次脚本执行的战报。
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScriptOutcome {
    /// true = 如预期完成（启动：超时仍在跑，或脚本自己以 0 退出表示"已在运行"；
    /// 停止：退出码 0）。false = 启动没起来，或停止脚本报错。
    pub ok: bool,
    /// 启动仍在跑时的进程 id（已退出或停止动作 = None）。
    pub pid: Option<u32>,
    /// 脚本已退出时的退出码（仍在跑 = None）。
    pub exit_code: Option<i32>,
    /// 输出尾部：启动失败时来自日志文件，停止时来自 stdout+stderr 合并。
    pub output: String,
    /// 启动脚本的日志文件路径（完整输出在那里；停止动作不落文件）。
    pub log: Option<String>,
}

/// 启动后端：后台跑起 `start_script`，3 秒内没退就算成了。
pub async fn start(data_dir: &Path, row: &BackendRow) -> Result<ScriptOutcome, String> {
    let path = script_path(row.start_script.as_deref(), "启动")?;
    let log = log_path(data_dir, row.id, "start");
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("建脚本日志目录失败 {}: {e}", dir.display()))?;
    }
    // 输出直接落文件、不走管道：长跑的服务脚本会写很多字节，管道没人读就会写满、
    // 把它自己阻塞在 write 上——那正是「点了启动，服务假死」的经典成因。
    let file = std::fs::File::create(&log)
        .map_err(|e| format!("建脚本日志失败 {}: {e}", log.display()))?;
    let err_file = file
        .try_clone()
        .map_err(|e| format!("复制脚本日志句柄失败: {e}"))?;

    let mut cmd = command(&path)?;
    cmd.stdout(Stdio::from(file)).stderr(Stdio::from(err_file));
    let mut child = cmd.spawn().map_err(|e| spawn_err(&path, e))?;
    let pid = child.id();
    match tokio::time::timeout(START_GRACE, child.wait()).await {
        // 3 秒内就退了：把脚本自己说的话带回来，别只甩一句「失败」。
        Ok(Ok(status)) => Ok(ScriptOutcome {
            ok: status.success(),
            pid: None,
            exit_code: status.code(),
            output: tail(&log),
            log: Some(log.display().to_string()),
        }),
        Ok(Err(e)) => Err(format!("等待脚本退出时出错: {e}")),
        Err(_) => {
            // 仍在跑：交给后台任务收尸，免得它退了以后变成僵尸进程。
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            Ok(ScriptOutcome {
                ok: true,
                pid,
                exit_code: None,
                output: String::new(),
                log: Some(log.display().to_string()),
            })
        }
    }
}

/// 停止后端：同步跑完 `stop_script`，把它的输出原样带回来。
pub async fn stop(row: &BackendRow) -> Result<ScriptOutcome, String> {
    let path = script_path(row.stop_script.as_deref(), "停止")?;
    // 停止脚本输出少、且要原样回显给用户，所以走管道而不是落文件（启动相反）。
    let mut cmd = command(&path)?;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| spawn_err(&path, e))?;
    match tokio::time::timeout(STOP_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            let mut text = out.stdout;
            text.extend_from_slice(&out.stderr);
            let output = String::from_utf8_lossy(&text).trim().to_string();
            Ok(ScriptOutcome {
                ok: out.status.success(),
                pid: None,
                exit_code: out.status.code(),
                output: tail_bytes(&output),
                log: None,
            })
        }
        // 超时：`wait_with_output` 已被丢弃，脚本继续在后台跑（不替用户杀它——
        // 它卡在哪我们不知道，擅自杀掉可能留下半截状态）。如实说明。
        Err(_) => Err(format!(
            "停止脚本 {} 秒内没有返回，它可能卡住了（进程仍在后台跑）",
            STOP_TIMEOUT.as_secs()
        )),
        Ok(Err(e)) => Err(format!("等待停止脚本退出时出错: {e}")),
    }
}

/// 建一个只带脚本路径的命令：工作目录取脚本所在目录（脚本里写的相对路径按它解析，
/// 而不是按 GUI 应用启动时的 cwd——那在 macOS 上是 `/`，什么都找不到）。
fn command(path: &Path) -> Result<Command, String> {
    let mut cmd = Command::new(path);
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    // stdin 关掉：脚本若读输入会立刻拿到 EOF 而不是把 App 挂住等键盘。
    cmd.stdin(Stdio::null());
    Ok(cmd)
}

fn spawn_err(path: &Path, e: std::io::Error) -> String {
    let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
        "（脚本没有可执行位，先 `chmod +x` 一下）"
    } else {
        ""
    };
    format!("脚本启动失败 {}{hint}: {e}", path.display())
}

/// 取出并校验脚本路径：空 = 这个后端没配；`~/` 开头按家目录展开（用户在输入框里
/// 打的就是这个形态，不展开只会换来一句「文件不存在」）；必须存在且是普通文件。
fn script_path(raw: Option<&str>, what: &str) -> Result<PathBuf, String> {
    let raw = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("该后端没有配置{what}脚本"))?;
    let path = expand_home(raw);
    let meta = std::fs::metadata(&path)
        .map_err(|e| format!("{what}脚本不存在或读不到 {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{what}脚本不是普通文件: {}", path.display()));
    }
    Ok(path)
}

fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" {
        return crate::config::dirs_home();
    }
    match raw.strip_prefix("~/") {
        Some(rest) => crate::config::dirs_home().join(rest),
        None => PathBuf::from(raw),
    }
}

/// 启动脚本的日志落点：`<data_dir>/logs/backend-scripts/<后端 id>-<动作>.log`。
/// 用 id 而不是名字：名字可能带斜杠（LM Studio 就吐过带斜杠的模型 id），落到路径上
/// 会把目录切碎。
fn log_path(data_dir: &Path, backend_id: i64, action: &str) -> PathBuf {
    data_dir
        .join("logs")
        .join("backend-scripts")
        .join(format!("{backend_id}-{action}.log"))
}

/// 文件尾部：按字节定位读取，不把整个日志读进内存（跑了一夜的服务日志可能几百 MB）。
fn tail(path: &Path) -> String {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(OUTPUT_TAIL as u64);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return String::new();
    }
    tail_bytes(&String::from_utf8_lossy(&buf))
}

/// 字符串尾部截断。按字节截可能切在多字节字符中间，`from_utf8_lossy` 之后那处会
/// 显示成一个替换符——不影响阅读，比先完整解码再按字符截便宜。
fn tail_bytes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() <= OUTPUT_TAIL {
        return s.trim().to_string();
    }
    String::from_utf8_lossy(&bytes[bytes.len() - OUTPUT_TAIL..])
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::backend_row_for_test;

    fn row_with(script: &str) -> BackendRow {
        let mut r = backend_row_for_test("lab");
        r.start_script = Some(script.into());
        r
    }

    /// 用真实脚本跑一次启动：3 秒内退出的脚本必须如实回报退出码与输出。
    #[tokio::test]
    async fn start_reports_exit_and_output_for_quick_script() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("start.sh");
        std::fs::write(&script, "#!/bin/sh\necho hello-start\nexit 0\n").unwrap();
        set_exec(&script);

        let out = start(dir.path(), &row_with(script.to_str().unwrap()))
            .await
            .unwrap();
        assert!(out.ok, "退出码 0 应判成功: {out:?}");
        assert_eq!(out.exit_code, Some(0));
        assert!(out.output.contains("hello-start"), "{}", out.output);
        assert!(out.log.is_some());
    }

    /// 启动失败要带得回输出，且 ok=false（否则用户只看到「失败」两个字）。
    #[tokio::test]
    async fn start_surfaces_failing_script_output() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("boom.sh");
        std::fs::write(&script, "#!/bin/sh\necho nope >&2\nexit 7\n").unwrap();
        set_exec(&script);

        let out = start(dir.path(), &row_with(script.to_str().unwrap()))
            .await
            .unwrap();
        assert!(!out.ok);
        assert_eq!(out.exit_code, Some(7));
        assert!(out.output.contains("nope"), "{}", out.output);
    }

    /// 一直跑着的脚本要在宽限期后如实报「仍在跑」并带上 pid。
    #[tokio::test]
    async fn start_reports_running_for_long_lived_script() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("serve.sh");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        set_exec(&script);

        let out = start(dir.path(), &row_with(script.to_str().unwrap()))
            .await
            .unwrap();
        assert!(out.ok);
        assert!(out.pid.is_some(), "仍在跑时必须给出 pid: {out:?}");
        assert!(out.exit_code.is_none());
    }

    /// 停止脚本：退出码与输出都要带回来。
    #[tokio::test]
    async fn stop_reports_exit_code_and_output() {
        let mut r = backend_row_for_test("lab");
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("stop.sh");
        std::fs::write(&script, "#!/bin/sh\necho bye\nexit 3\n").unwrap();
        set_exec(&script);
        r.stop_script = Some(script.to_str().unwrap().into());

        let out = stop(&r).await.unwrap();
        assert!(!out.ok);
        assert_eq!(out.exit_code, Some(3));
        assert!(out.output.contains("bye"), "{}", out.output);
    }

    /// 没配脚本 / 路径不存在：说的是人话，且不 pretend 成功。
    #[tokio::test]
    async fn missing_script_is_an_actionable_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = start(dir.path(), &backend_row_for_test("lab"))
            .await
            .unwrap_err();
        assert!(err.contains("启动"), "{err}");

        let err = start(dir.path(), &row_with("~/definitely-not-here/x.sh"))
            .await
            .unwrap_err();
        assert!(err.contains("不存在"), "{err}");
    }

    /// `~/` 展开与尾部截断：这两件小事错了都会以「文件不存在 / 输出空白」的形式报给用户。
    #[test]
    fn expand_home_and_tail() {
        let home = crate::config::dirs_home();
        assert_eq!(expand_home("~/a/b.sh"), home.join("a/b.sh"));
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("/abs/x.sh"), PathBuf::from("/abs/x.sh"));

        assert_eq!(tail_bytes("abc"), "abc");
        let long = "x".repeat(OUTPUT_TAIL + 10);
        assert_eq!(tail_bytes(&long).len(), OUTPUT_TAIL);
    }

    #[cfg(unix)]
    fn set_exec(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(not(unix))]
    fn set_exec(_path: &Path) {}
}
