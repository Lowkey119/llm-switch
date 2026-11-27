//! 代理安装扫描 / 一键安装 IPC。
//!
//! - `agents_scan`：逐文件客户端给出「装没装、版本、可执行路径、配置文件落点、一键安装命令」。
//!   可执行探测走 [`crate::services::agents_scan`]（登录 shell），配置落点复用接管评测的
//!   `files`（含路径自定义、各家内核差异），两处合并成一份视图。
//!   命令本身只读缓存快照，探测在后台线程并行做，刷完 emit `agents://scan`。
//! - `agent_install`：在登录 shell 里跑该端**固定白名单**安装命令，走通用后台任务队列
//!   （`spawn_task`）回报进度，前端复用任务托盘 / 就地进度条。装完清扫描缓存。
//!
//! 安全：安装命令是编译期常量（见 agents_scan::meta），绝不拼任何用户输入；执行只在登录
//! shell 里跑这条固定命令。

use crate::commands::task::spawn_task;
use crate::gateway::State;
use crate::services::agents_scan::{self, AgentScan, ConfigPath};
use crate::takeover::{self, ClientApp, Selection, TakeoverEnv};
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State as TauriState};

fn ctx(st: &State) -> (TakeoverEnv, takeover::Endpoint) {
    (
        TakeoverEnv::from_config(&st.config, &st.store),
        takeover::Endpoint::build(&st.config, &st.store),
    )
}

/// 扫描全部代理的安装态——**快照语义，永不阻塞等 shell**。
///
/// 遍历 = `FILE_CLIENTS` + copilot 门面：copilot 没有可接管的配置文件，**不进代理管理
/// 列表**（那里按 `facade` 过滤），但它的 CLI 是真装在本机的，安装引导里要能扫到、能一键装。
///
/// 可执行部分直接读缓存：装没装的增量判断（路径在不在）在后台线程做，需要重探的
/// 端丢进 [`agents_scan::schedule_refresh`] 并行刷新，刷完 emit `agents://scan`，
/// 前端监听重取即可——页面首帧永远秒回（冷启动返回空快照 + `scanning=true`）。
/// `force = Some(true)`（手动重扫）：全部端强制重探版本号。
/// `only = Some(app)`：强制重探只针对该端（各代理详情自己的「检测新版本」入口），
/// 返回仍是全量快照，前端共用同一份缓存。
#[tauri::command]
pub fn agents_scan(
    st: TauriState<'_, Arc<State>>,
    handle: AppHandle,
    force: Option<bool>,
    only: Option<String>,
) -> Result<Value, String> {
    let force = force.unwrap_or(false);
    let only = only.as_deref().map(ClientApp::parse).transpose()?;
    let (env, ep) = ctx(&st);
    let mut out = Vec::with_capacity(ClientApp::ALL.len());
    let mut to_probe: Vec<ClientApp> = Vec::new();
    for app in ClientApp::ALL {
        let forced = force || only == Some(app);
        if forced || agents_scan::needs_probe(app) {
            to_probe.push(app);
        }
        // 配置落点复用接管评测：路径自定义、DevEco 内核、gemini 双文件都已在里面收口。
        // copilot 门面没有配置文件（evaluate_one 对它显式 Err），退回它的状态目录存在性。
        let config_paths: Vec<ConfigPath> = match app {
            ClientApp::Copilot => {
                let dir = crate::services::ext_install::config_dir(app, &env);
                vec![ConfigPath {
                    path: dir.display().to_string(),
                    exists: dir.is_dir(),
                }]
            }
            _ => takeover::evaluate_one(&env, &ep, app, &Selection::default())
                .map(|ev| {
                    ev.files
                        .iter()
                        .map(|f| ConfigPath {
                            path: f.path.clone(),
                            exists: f.exists,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        let config_exists = config_paths.iter().any(|c| c.exists);
        // 可执行信号取缓存；没缓存（冷启动首轮在飞）就按「未知」出快照，
        // 等 emit 到货后前端自然刷新，绝不在这里等 shell。
        let detect = agents_scan::cached(app);
        let probed = detect.is_some();
        let detect = detect.unwrap_or_default();
        out.push(AgentScan {
            app: app.as_str().to_string(),
            label: app.label().to_string(),
            installed: detect.bin_path.is_some() || config_exists,
            bin_found: detect.bin_path.is_some(),
            broken: detect.broken,
            probed,
            bin: agents_scan::bin_name(app).to_string(),
            bin_path: detect.bin_path,
            version: detect.version,
            install_command: agents_scan::install_command(app).map(str::to_string),
            config_paths,
        });
    }
    if !to_probe.is_empty() {
        agents_scan::schedule_refresh(&to_probe, move || {
            let _ = handle.emit("agents://scan", ());
        });
    }
    Ok(serde_json::json!({
        "scans": out,
        // 前端据此显示「扫描中」并在 agents://scan 到货后重取。
        "scanning": agents_scan::refreshing(),
    }))
}

/// 一键安装某代理：登录 shell 里跑固定白名单命令，进度走后台任务队列，返回任务 id。
#[tauri::command]
pub fn agent_install(app: String, handle: AppHandle) -> Result<String, String> {
    let app = ClientApp::parse(&app)?;
    let cmd = agents_scan::install_command(app)
        .ok_or_else(|| format!("{} 没有可用的一键安装命令", app.label()))?
        .to_string();
    let label = format!("安装 {}", app.label());
    // 装成功后清该端扫描缓存，下次扫描立刻看到新版本而不是等 TTL。
    let invalidate = app;
    let id = spawn_task(
        handle,
        "agent-install",
        &label,
        move |progress| async move {
            progress.set(0.05, Some("启动安装…"));
            let shell = std::env::var("SHELL")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "/bin/zsh".to_string());
            use tokio::io::{AsyncBufReadExt, BufReader};
            use tokio::process::Command;
            let mut child = Command::new(&shell)
                .arg("-lc")
                .arg(&cmd)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("启动安装失败: {e}"))?;
            // 合并 stdout/stderr 逐行回报：npm/curl 没有干净百分比，用「行数推进 + 末行文案」
            // 给一条会动的不定进度（封顶 0.95，收尾由任务队列置 1.0）。
            let stdout = child.stdout.take().expect("stdout piped");
            let stderr = child.stderr.take().expect("stderr piped");
            let out = tokio::task::spawn(async move {
                let mut lines: Vec<String> = Vec::new();
                let mut a = BufReader::new(stdout).lines();
                while let Ok(Some(l)) = a.next_line().await {
                    lines.push(l);
                }
                lines
            });
            let mut err_lines: Vec<String> = Vec::new();
            let mut b = BufReader::new(stderr).lines();
            let mut seen = 0u32;
            while let Ok(Some(l)) = b.next_line().await {
                seen += 1;
                let pct = (0.1 + (seen as f64) * 0.02).min(0.95);
                progress.set(pct, Some(l.trim()));
                err_lines.push(l);
            }
            let out_lines = out.await.unwrap_or_default();
            let status = child
                .wait()
                .await
                .map_err(|e| format!("安装进程异常: {e}"))?;
            if status.success() {
                agents_scan::invalidate(invalidate);
                let tail = err_lines
                    .last()
                    .cloned()
                    .or_else(|| out_lines.last().cloned())
                    .unwrap_or_else(|| "完成".to_string());
                Ok(tail)
            } else {
                // 失败：把 stderr（退回 stdout）末尾几行带回，便于用户排错。
                let tail: Vec<String> = if !err_lines.is_empty() {
                    err_lines
                } else {
                    out_lines
                }
                .into_iter()
                .rev()
                .take(5)
                .collect();
                Err(format!(
                    "安装失败（退出码 {}）：{}",
                    status.code().unwrap_or(-1),
                    tail.join(" ⏎ ")
                ))
            }
        },
    );
    Ok(id)
}
