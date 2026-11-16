//! 推理引擎远程控制通道（SSH 双地址连到 102 那台机器），halogen 与 gufo 双机位共用。
//!
//! 为什么全走 SSH：引擎绑 127.0.0.1（或容器端口映射），启停/日志/镜像操作本来就得
//! 在 102 上执行；一条通道统一「雷电 10.0.0.102 优先、局域网 192.168.31.168 兜底」
//! 的故障转移，控制台页的读（health/metrics/cache/logs）与写（启停/换版本）同构，
//! 零新增暴露面。
//!
//! 安全：进入远端 shell 的字符串要么是本文件的固定模板，要么先过白名单
//!（版本号 `^x.y.z$`、日志等级 info|debug|trace|raw、动作 start|stop|restart），
//! 杜绝注入。SSH 一律 BatchMode（绝不弹交互提示），ConnectTimeout 快速失败换轨。
//!
//! 编排留在服务旁边：版本切换的编排逻辑在 102 的 `hal-ver.sh` / `gu-ver.sh`
//!（CLI 同权可用），参数读写编排在 `hal_env.py` / `gu_env.py`，
//! 这里只负责把它们接进 IPC——与 hal.sh/gufo.sh 同一套部署自描述风格。
//!
//! 双机位：`Channel` 描述一个引擎位的脚本名 / 容器名 / ghcr 仓 / 备份键 / 端口推导。
//! halogen 与 gufo 同机互斥（分占 8731，内存只够一家）， morals 一致所以实现只有一份。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::process::Command;

use crate::config::HalctlCfg;

/// 上次连通的地址下标（0=雷电，1=局域网）；连不上自动换轨，连上后记住，
/// 避免雷电拔线后每次轮询都先白等一轮超时。halogen/gufo 共享（同一台机器）。
static LAST_OK: AtomicUsize = AtomicUsize::new(0);

/// 一个引擎机位的编排面：部署目录里的脚本名 / 容器名 / 官方 ghcr 仓 / 备份键。
pub struct Channel {
    /// podman 容器名。
    pub container: &'static str,
    /// 启停脚本（hal.sh / gufo.sh）。
    pub shell: &'static str,
    /// 启停参数：[start, stop, restart]；halogen 的 restart 就是裸调 `bash hal.sh`。
    pub shell_args: [&'static str; 3],
    /// 版本管理脚本（hal-ver.sh / gu-ver.sh）。
    pub ver: &'static str,
    /// 参数读写器（hal_env.py / gu_env.py）。
    pub env: &'static str,
    /// 中文日志翻译器（hal_log_cn.py / gu_log_cn.py）。
    pub log: &'static str,
    /// 参数文件里的端口键名 + 错价默认。halogen 读 halogen.env 的 HALOGEN_HOST_PORT，
    /// gufo 读 gufo.conf 的 API_PORT（写侧带引号，读侧剥引号）。
    pub port_key: &'static str,
    pub port_file: &'static str,
    /// ghcr 官方镜像仓库（remote tags 列表用）。
    pub repo: &'static str,
    /// .ver_backup.json 里的备份键（hal / gu 两键共存一个文件，互不覆盖）。
    pub backup_key: &'static str,
}

impl Channel {
    /// `bash {proj}/{shell}[ args]` 片段。
    fn shell_cmd(&self, proj: &str, args: &str) -> String {
        if args.is_empty() {
            format!("bash {}/{}", proj, self.shell)
        } else {
            format!("bash {}/{} {}", proj, self.shell, args)
        }
    }
}

/// halogen 机位（/mnt/haloen）。
pub const HAL: Channel = Channel {
    container: "halogen",
    shell: "hal.sh",
    shell_args: ["start", "stop", ""],
    ver: "hal-ver.sh",
    env: "hal_env.py",
    log: "hal_log_cn.py",
    port_key: "HALOGEN_HOST_PORT",
    port_file: "halogen.env",
    repo: "peonist-ai/halogen-flash-server",
    backup_key: "hal",
};

/// gufo 机位（/mnt/data/gufo）。
pub const GU: Channel = Channel {
    container: "gufo",
    shell: "gufo.sh",
    shell_args: ["start", "stop", "restart"],
    ver: "gu-ver.sh",
    env: "gu_env.py",
    log: "gu_log_cn.py",
    port_key: "API_PORT",
    port_file: "gufo.conf",
    repo: "gufo-org/toolboxes/gufo-runtime",
    backup_key: "gu",
};

/// 通道名 → 机位（IPC 参数 ch：halogen→HAL、gufo→GU）。取值必须注册，拒绝静默兜底。
pub fn channel_of(ch: &str) -> Result<&'static Channel, String> {
    match ch.to_ascii_lowercase().as_str() {
        "halogen" | "hal" | "" => Ok(&HAL),
        "gufo" | "gu" => Ok(&GU),
        other => Err(format!("未知控制面通道：{other}（应为 halogen 或 gufo）")),
    }
}

/// 远端命令结果。
pub struct Remote {
    pub host: String,
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

impl Remote {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    /// stdout+stderr 合并的紧凑输出（给 UI 回显用）。
    pub fn out(&self) -> String {
        let s = format!("{}{}", self.stdout, self.stderr);
        let t = s.trim();
        if t.is_empty() {
            "(无输出)".to_string()
        } else {
            t.to_string()
        }
    }
}

async fn ssh_once(
    cfg: &HalctlCfg,
    host: &str,
    cmd: &str,
    timeout: Duration,
) -> Result<Remote, String> {
    let target = format!("{}@{}", cfg.user, host);
    let fut = async {
        let out = Command::new("ssh")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=5")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg("ServerAliveInterval=15")
            .arg("-o")
            .arg("ServerAliveCountMax=8")
            .arg(&target)
            .arg(cmd)
            .output()
            .await
            .map_err(|e| format!("启动 ssh 失败：{e}"))?;
        Ok::<Remote, String>(Remote {
            host: host.to_string(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            code: out.status.code().unwrap_or(-1),
        })
    };
    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| format!("SSH 超时（{}s）", timeout.as_secs()))?
}

/// 在 102 上执行一条命令：优先上次连通的地址，连不上（ssh 退出码 255 / 连接超时）
/// 自动换另一个地址；两个都失败才报错。远端命令自身的非零退出是真实结果，不换轨。
pub async fn run(cfg: &HalctlCfg, cmd: &str, timeout: Duration) -> Result<Remote, String> {
    if cfg.hosts.is_empty() {
        return Err("halctl.hosts 未配置".to_string());
    }
    let n = cfg.hosts.len();
    let start = LAST_OK.load(Ordering::Relaxed) % n;
    let mut last_err = String::new();
    for i in 0..n {
        let idx = (start + i) % n;
        let host = &cfg.hosts[idx];
        match ssh_once(cfg, host, cmd, timeout).await {
            // 255 = ssh 自身失败（连不上/认证失败），属于链路问题 → 换轨
            Ok(r) if r.code == 255 => last_err = format!("{host}: {}", r.stderr.trim()),
            Ok(r) => {
                LAST_OK.store(idx, Ordering::Relaxed);
                return Ok(r);
            }
            Err(e) => last_err = format!("{host}: {e}"),
        }
    }
    Err(format!("两个地址都连不上（{last_err}）"))
}

/// 版本号白名单：恰好三段、每段非空纯数字。tag 会进远端 shell，必须先过这道闸。
pub fn valid_tag(tag: &str) -> bool {
    let parts: Vec<&str> = tag.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// 端口跟引擎参数文件的端口键走（与 hal.sh / gufo.sh 同一套推导），缺省 8731。
/// gufo 写侧给端口值带引号（conf 会被 source），读侧 tr 剥掉。
fn port_prefix(ch: &Channel, proj: &str) -> String {
    format!(
        "P=$(grep -E '^{key}=' {proj}/{file} 2>/dev/null | tail -1 | cut -d= -f2- | tr -d '\\\"'); \
         P=${{P:-8731}}; ",
        key = ch.port_key,
        proj = proj,
        file = ch.port_file
    )
}

/// status 的远端命令：先取容器状态，**只有本引擎在跑才 curl 端口**。
/// 端口是两引擎共用的 8731——halogen 在跑时 curl 8731 拿到的是 halogen 的 health/metrics，
/// gufo 面板会把它当成自己的 KV 池/速率显示（实报串台）。停着就整段不输出——
/// section 缺标记返回空 → health/cache=null、metrics={}，面板如实显示 —。
fn status_cmd(ch: &Channel, proj: &str) -> String {
    format!(
        "{pre}S=$(podman ps --filter name={c} --format '{{{{.Status}}}}' | head -1); \
         echo __PS__; echo \"$S\"; \
         if [ -n \"$S\" ]; then \
         echo __HEALTH__; curl -s -m 4 http://127.0.0.1:$P/health || true; echo; \
         echo __CACHE__; curl -s -m 4 http://127.0.0.1:$P/cache || true; echo; \
         echo __METRICS__; curl -s -m 4 http://127.0.0.1:$P/metrics | grep -E '^(llamacpp|halogen):' || true; \
         fi",
        pre = port_prefix(ch, proj),
        c = ch.container
    )
}

/// 控制台快照：容器状态 +（仅当本引擎在跑）/health + /cache + /metrics，一次 SSH 往返全拿。
///
/// health/cache 原样透传（前端按需取字段，引擎加字段不用改这里）；
/// metrics 把 `llamacpp:` / `halogen:` 计数器解析成 name→f64 映射
///（gufo 全部走 llamacpp: 前缀，grep 同式命中）。
pub async fn status(cfg: &HalctlCfg, ch: &'static Channel) -> Result<Value, String> {
    let cmd = status_cmd(ch, &cfg.proj);
    let r = run(cfg, &cmd, Duration::from_secs(25)).await?;
    let ps = section(&r.stdout, "__PS__");
    let mut health = parse_json(section(&r.stdout, "__HEALTH__"));
    let cache = parse_json(section(&r.stdout, "__CACHE__"));
    let metrics = parse_prometheus(section(&r.stdout, "__METRICS__"));
    // 巨型目录字段对面板无意义，砍掉省得每次轮询搬运
    if let Some(h) = health.as_mut().and_then(|v| v.as_object_mut()) {
        h.remove("structured_output");
        h.remove("endpoints");
        h.remove("modes");
        h.remove("chat_template");
    }
    Ok(json!({
        "reachable": true,
        "host": r.host,
        "running": !ps.trim().is_empty(),
        "container": ps.trim(),
        "health": health,
        "cache": cache,
        "metrics": metrics,
    }))
}

/// 日志尾读：复用 102 上现成的中文翻译器（hal_log_cn.py / gu_log_cn.py，stdin→stdout），
/// 前端只负责展示——渲染逻辑只有一份，CLI（hal.sh log / gufo.sh log）与控制台永远同口径。
pub async fn logs(
    cfg: &HalctlCfg,
    ch: &'static Channel,
    tail: u32,
    level: &str,
) -> Result<Value, String> {
    let tail = tail.clamp(10, 2000);
    let cmd = match level {
        "raw" => format!("podman logs -t --tail {tail} {}", ch.container),
        lv => {
            let lv = match lv {
                "debug" | "trace" => lv,
                _ => "info",
            };
            format!(
                "podman logs -t --tail {tail} {c} | python3 {proj}/{log} {lv}",
                c = ch.container,
                proj = cfg.proj,
                log = ch.log
            )
        }
    };
    let r = run(cfg, &cmd, Duration::from_secs(30)).await?;
    let lines: Vec<&str> = r.stdout.lines().collect();
    // 失败原因必须带回去：gufo 停止时容器会被删掉，`podman logs gufo` 直接报
    // 「no such container」，而错误走 stderr。只回 stdout 的话前端只能显示
    // 「该等级下暂无日志行」，把真实原因藏得干干净净（用户口径：gufo 日志不显示）。
    let error = if r.ok() {
        None
    } else {
        let t = r.out();
        let t = if t.trim().is_empty() {
            format!("读取 {} 日志失败（远端无输出）", ch.container)
        } else {
            t.trim().to_string()
        };
        Some(if t.contains("no such container") {
            format!(
                "{} 容器不存在——引擎停止时容器会被删除，日志只在容器存在时有；先启动引擎再看",
                ch.container
            )
        } else {
            t
        })
    };
    Ok(json!({ "lines": lines, "host": r.host, "error": error }))
}

/// 启停控制。start/restart 会阻塞到引擎就绪（冷启动约 1~5 分钟，各启动脚本内部
/// 等待上限 900s），所以超时给足；UI 期间禁用按钮并显示进度文案。
pub async fn control(cfg: &HalctlCfg, ch: &'static Channel, action: &str) -> Result<Value, String> {
    let (args, secs) = match action {
        "stop" => (ch.shell_args[1], 180u64),
        "start" => (ch.shell_args[0], 960),
        "restart" => (ch.shell_args[2], 960),
        _ => return Err("动作需为 start/stop/restart".to_string()),
    };
    let r = run(
        cfg,
        &ch.shell_cmd(&cfg.proj, args),
        Duration::from_secs(secs),
    )
    .await?;
    Ok(json!({ "ok": r.ok(), "output": r.out(), "host": r.host }))
}

/// 版本信息：当前 tag + 本地镜像列表 + 永久备份槽（来自 102）+ 官方 tag 列表（Mac 直连 ghcr）。
pub async fn versions(cfg: &HalctlCfg, ch: &'static Channel) -> Result<Value, String> {
    let show_cmd = format!("bash {}/{} show", cfg.proj, ch.ver);
    let local_fut = run(cfg, &show_cmd, Duration::from_secs(30));
    let (local, remote) = tokio::join!(local_fut, remote_tags(ch.repo));
    let r = local?;
    let mut it = r.stdout.splitn(3, "__SEP__");
    let current = it.next().unwrap_or("").trim().to_string();
    let locals: Vec<String> = it
        .next()
        .unwrap_or("")
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let backup_raw = it.next().unwrap_or("").trim();
    let backup = parse_json(backup_raw).and_then(|b| b.get(ch.backup_key).cloned());
    let (remote_tags, remote_err) = match remote {
        Ok(t) => (t, None),
        Err(e) => (vec![], Some(e)),
    };
    Ok(json!({
        "current": current,
        "local": locals,
        "backup": backup,
        "remote": remote_tags,
        "remote_error": remote_err,
        "host": r.host,
    }))
}

/// 切到指定版本（更新与恢复共用；恢复=切回备份槽 tag）。缺镜像先 pull，可能数分钟。
pub async fn switch_version(
    cfg: &HalctlCfg,
    ch: &'static Channel,
    tag: &str,
) -> Result<Value, String> {
    if !valid_tag(tag) {
        return Err("版本号格式不合法（需 x.y.z）".to_string());
    }
    let cmd = format!("bash {}/{} set {}", cfg.proj, ch.ver, tag);
    let r = run(cfg, &cmd, Duration::from_secs(2700)).await?;
    Ok(json!({ "ok": r.ok(), "output": r.out(), "host": r.host }))
}

/// 把当前版本钉进永久备份槽。
pub async fn backup_set(cfg: &HalctlCfg, ch: &'static Channel) -> Result<Value, String> {
    let r = run(
        cfg,
        &format!("bash {}/{} backup", cfg.proj, ch.ver),
        Duration::from_secs(30),
    )
    .await?;
    Ok(json!({ "ok": r.ok(), "output": r.out() }))
}

/// 从备份槽恢复：目标 tag 由 102 端从文件读取，不信任前端传参。
pub async fn restore(cfg: &HalctlCfg, ch: &'static Channel) -> Result<Value, String> {
    let r = run(
        cfg,
        &format!("bash {}/{} restore", cfg.proj, ch.ver),
        Duration::from_secs(2700),
    )
    .await?;
    Ok(json!({ "ok": r.ok(), "output": r.out() }))
}

/// 删除一个本地镜像（备份引擎版本，腾磁盘用）。
///
/// 这里只挡版本号格式；「启动脚本正指向的版本」与「运行中容器在用的镜像」由 102 端
/// 的 `del` 再挡一道——只有那边看得见 podman 现场。删的是 tag，同一镜像的其它 tag
/// 不受影响（`podman rmi` 对多 tag 镜像只解绑本 tag）。
pub async fn image_delete(
    cfg: &HalctlCfg,
    ch: &'static Channel,
    tag: &str,
) -> Result<Value, String> {
    if !valid_tag(tag) {
        return Err("版本号格式不合法（需 x.y.z）".to_string());
    }
    let cmd = format!("bash {}/{} del {}", cfg.proj, ch.ver, tag);
    let r = run(cfg, &cmd, Duration::from_secs(180)).await?;
    Ok(json!({ "ok": r.ok(), "output": r.out(), "host": r.host }))
}

// ── 参数与环境变量（halogen.env / gufo.conf + 命名预设）─────────────────
//
// 编排逻辑在 102 的 hal_env.py / gu_env.py（白名单/校验/写盘/预设存储都在那，
// CLI 同权），这里只搬运：spec+show+preset-list 一次往返全拿；写路径经 heredoc 传 JSON。
// serde_json 紧凑输出保证单行，heredoc 定界符单独成行 → 值里带引号/换义都安全。

fn env_cmd(ch: &Channel, proj: &str, sub: &str, payload: &Value) -> String {
    format!(
        "python3 {}/{} {} <<'HALJSON'\n{}\nHALJSON",
        proj, ch.env, sub, payload
    )
}

/// 参数面板快照：规格（分组白名单）+ 当前值 + 预设列表，一次 SSH。
pub async fn env(cfg: &HalctlCfg, ch: &'static Channel) -> Result<Value, String> {
    let e = ch.env;
    let cmd = format!(
        "python3 {p}/{e} spec; echo __SEP__; python3 {p}/{e} show; echo __SEP__; python3 {p}/{e} preset-list",
        p = cfg.proj
    );
    let r = run(cfg, &cmd, Duration::from_secs(25)).await?;
    if !r.ok() {
        return Err(r.out());
    }
    let mut it = r.stdout.splitn(3, "__SEP__");
    let spec: Value = serde_json::from_str(it.next().unwrap_or("").trim()).unwrap_or(json!([]));
    let values: Value = serde_json::from_str(it.next().unwrap_or("").trim()).unwrap_or(json!({}));
    let presets: Value = serde_json::from_str(it.next().unwrap_or("").trim()).unwrap_or(json!([]));
    Ok(json!({ "spec": spec, "values": values, "presets": presets, "host": r.host }))
}

/// 保存参数编辑（空值=移除键跟随默认）；写 halogen.env / gufo.conf，重启后生效。
pub async fn env_save(
    cfg: &HalctlCfg,
    ch: &'static Channel,
    values: Value,
) -> Result<Value, String> {
    let r = run(
        cfg,
        &env_cmd(ch, &cfg.proj, "write", &json!({ "values": values })),
        Duration::from_secs(30),
    )
    .await?;
    Ok(json!({ "ok": r.ok(), "output": r.out() }))
}

/// 把当前编辑存成命名预设（预设不含 API 密钥）。
pub async fn env_preset_save(
    cfg: &HalctlCfg,
    ch: &'static Channel,
    name: &str,
    values: Value,
) -> Result<Value, String> {
    let r = run(
        cfg,
        &env_cmd(
            ch,
            &cfg.proj,
            "preset-save",
            &json!({ "name": name, "values": values }),
        ),
        Duration::from_secs(30),
    )
    .await?;
    Ok(json!({ "ok": r.ok(), "output": r.out() }))
}

/// 删除命名预设。
pub async fn env_preset_del(
    cfg: &HalctlCfg,
    ch: &'static Channel,
    name: &str,
) -> Result<Value, String> {
    let r = run(
        cfg,
        &env_cmd(ch, &cfg.proj, "preset-del", &json!({ "name": name })),
        Duration::from_secs(30),
    )
    .await?;
    Ok(json!({ "ok": r.ok(), "output": r.out() }))
}

/// 一键应用预设：写参数后若在跑则重启（应用=立即生效的语义）；停止状态只写不启。
pub async fn env_preset_apply(
    cfg: &HalctlCfg,
    ch: &'static Channel,
    name: &str,
) -> Result<Value, String> {
    let r = run(
        cfg,
        &env_cmd(ch, &cfg.proj, "preset-apply", &json!({ "name": name })),
        Duration::from_secs(60),
    )
    .await?;
    if !r.ok() {
        return Err(r.out());
    }
    let mut log = r.out();
    let ps = run(
        cfg,
        &format!(
            "podman ps --filter name={c} --format '{{{{.Names}}}}' | grep -cx {c} || true",
            c = ch.container
        ),
        Duration::from_secs(20),
    )
    .await?;
    if ps.stdout.trim() == "1" {
        log.push_str("\n重启加载新参数…\n");
        let rs = run(
            cfg,
            &ch.shell_cmd(&cfg.proj, ch.shell_args[2]),
            Duration::from_secs(960),
        )
        .await?;
        log.push_str(&rs.out());
        if !rs.ok() {
            return Ok(json!({ "ok": false, "output": log }));
        }
    } else {
        log.push_str(&format!("\n{} 未运行，仅写入参数", ch.container));
    }
    Ok(json!({ "ok": true, "output": log }))
}

/// ghcr 匿名取官方镜像 tags（semver 过滤 + 升序）。Mac 直连，不占 SSH 通道。
/// halogen = peonist-ai/halogen-flash-server；gufo = gufo-org/toolboxes/gufo-runtime。
/// **必须跟 `Link: rel="next"` 翻页**：ghcr 每页约 100 个 tag，只取首页时
/// 新 tag 全在后面的页里（实报：0.18.23 已发布却显示「线上最新 0.18.22」）。
async fn remote_tags(repo: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let tok: Value = client
        .get("https://ghcr.io/token")
        .query(&[
            ("service", "ghcr.io"),
            ("scope", &format!("repository:{repo}:pull")),
        ])
        .send()
        .await
        .map_err(|e| format!("ghcr 令牌请求失败：{e}"))?
        .json()
        .await
        .map_err(|e| format!("ghcr 令牌解析失败：{e}"))?;
    let token = tok.get("token").and_then(|t| t.as_str()).unwrap_or("");
    if token.is_empty() {
        return Err("ghcr 匿名令牌为空".to_string());
    }
    let mut tags: Vec<String> = Vec::new();
    let mut url = format!("https://ghcr.io/v2/{repo}/tags/list");
    // 翻页保险上限：每页约 100，20 页 = 2000 个 tag，远超真实规模；
    // Link 链若异常成环也不会无限抓。
    for _ in 0..20 {
        let resp = client
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| format!("ghcr tags 请求失败：{e}"))?;
        let next = next_link(resp.headers());
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("ghcr tags 解析失败：{e}"))?;
        if let Some(arr) = body.get("tags").and_then(|t| t.as_array()) {
            for t in arr.iter().filter_map(|t| t.as_str()) {
                if valid_tag(t) && !tags.iter().any(|x| x == t) {
                    tags.push(t.to_string());
                }
            }
        }
        let Some(next) = next else { break };
        // Link 里是相对路径（`/v2/...`），防御性兼容绝对 URL。
        url = if next.starts_with("http") {
            next
        } else {
            format!("https://ghcr.io{next}")
        };
    }
    tags.sort_by_key(|t| semver_key(t));
    Ok(tags)
}

/// 从 Link 响应头取 `rel="next"` 的 URL：`</v2/repo/tags/list?last=…>; rel="next"`。
fn next_link(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let raw = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    for part in raw.split(',') {
        let Some((url, rest)) = part.split_once(';') else {
            continue;
        };
        if rest.contains("rel=\"next\"") || rest.contains("rel=next") {
            let u = url.trim().trim_start_matches('<').trim_end_matches('>');
            if u.is_empty() {
                return None;
            }
            return Some(u.to_string());
        }
    }
    None
}

fn semver_key(tag: &str) -> (u64, u64, u64) {
    let mut it = tag.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

/// 按标记切段：取 `__NAME__` 之后到下一个 `__` 标记行（或结尾）的内容。
fn section<'a>(text: &'a str, marker: &str) -> &'a str {
    let Some(i) = text.find(marker) else {
        return "";
    };
    let rest = &text[i + marker.len()..];
    let end = rest.find("\n__").unwrap_or(rest.len());
    rest[..end].trim()
}

fn parse_json(s: &str) -> Option<Value> {
    if s.is_empty() {
        return None;
    }
    serde_json::from_str(s).ok()
}

/// Prometheus 文本 → name→f64（只收 llamacpp:/halogen: 前缀，跳过注释与直方图桶）。
fn parse_prometheus(text: &str) -> Value {
    let mut map = serde_json::Map::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            if let Ok(n) = v.parse::<f64>() {
                map.insert(k.to_string(), json!(n));
            }
        }
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 串台回归钉：status 的 health/cache/metrics 探测必须整体在「容器在跑」守卫之后，
    /// 否则 gufo 停着时会把共用 8731 上 halogen 的数据当成自己的（实报 bug）。
    #[test]
    fn status_cmd_guards_port_probes_behind_running() {
        for ch in [&HAL, &GU] {
            let cmd = status_cmd(ch, "/proj");
            let guard = cmd.find("if [ -n \"$S\" ]").expect("running guard present");
            for marker in ["__HEALTH__", "__CACHE__", "__METRICS__"] {
                let at = cmd.find(marker).unwrap();
                assert!(at > guard, "{marker} must sit inside the running guard");
            }
            assert!(cmd.contains(&format!("name={}", ch.container)));
        }
    }

    #[test]
    fn tag_whitelist_blocks_injection() {
        assert!(valid_tag("0.18.9"));
        assert!(valid_tag("1.2.3"));
        assert!(!valid_tag("0.18"));
        assert!(!valid_tag("0.18.9; rm -rf /"));
        assert!(!valid_tag("0.18.9\""));
        assert!(!valid_tag(""));
        assert!(!valid_tag("0..9"));
    }

    #[test]
    fn next_link_parses_ghcr_pagination() {
        use reqwest::header::{HeaderMap, HeaderValue, LINK};
        let mut h = HeaderMap::new();
        h.insert(
            LINK,
            HeaderValue::from_static(
                "</v2/peonist-ai/halogen-flash-server/tags/list?last=0.18.22&n=0>; rel=\"next\"",
            ),
        );
        assert_eq!(
            next_link(&h).as_deref(),
            Some("/v2/peonist-ai/halogen-flash-server/tags/list?last=0.18.22&n=0")
        );
        // 只有 rel=next 才算；preload/其它 rel 一律忽略。
        let mut other = HeaderMap::new();
        other.insert(LINK, HeaderValue::from_static("</x>; rel=\"preload\""));
        assert_eq!(next_link(&other), None);
        assert_eq!(next_link(&HeaderMap::new()), None);
    }

    /// 真机冒烟（手动跑）：钉死 ghcr 翻页——0.18.23 发布后首页（100 个原始 tag，
    /// 过滤掉带后缀的剩 98）里没有它，只取首页的旧实现永远报「线上最新 0.18.22」
    /// （用户实报）。断言用 semver 比较，不随后续发布过期。
    #[tokio::test]
    #[ignore = "需要联网访问 ghcr"]
    async fn live_remote_tags_follows_pagination() {
        let t = remote_tags("peonist-ai/halogen-flash-server")
            .await
            .expect("remote_tags 失败");
        println!(
            "tag 总数={} 尾部={:?}",
            t.len(),
            &t[t.len().saturating_sub(3)..]
        );
        let last = t.last().map(|s| s.as_str()).unwrap_or_default();
        assert!(
            semver_key(last) > semver_key("0.18.22"),
            "翻页未生效：线上最新仍停在 {last}（旧 bug 的病征）"
        );
    }

    #[test]
    fn semver_sorts_numerically() {
        assert!(semver_key("0.18.10") > semver_key("0.18.9"));
        assert!(semver_key("1.0.0") > semver_key("0.99.99"));
    }

    #[test]
    fn sections_split_on_markers() {
        let text = "junk\n__PS__\nUp 3 hours\n__HEALTH__\n{\"status\":\"ok\"}\n__CACHE__\n";
        assert_eq!(section(text, "__PS__"), "Up 3 hours");
        assert_eq!(section(text, "__HEALTH__"), "{\"status\":\"ok\"}");
        assert_eq!(section(text, "__CACHE__"), "");
        assert_eq!(section(text, "__NOPE__"), "");
    }

    #[test]
    fn prometheus_parses_counters() {
        let text = "# HELP x\n# TYPE x counter\nllamacpp:prompt_tokens_total 1186142\nhalogen:cache_hits_total 275\nbroken line x\n";
        let v = parse_prometheus(text);
        assert_eq!(v["llamacpp:prompt_tokens_total"], json!(1186142.0));
        assert_eq!(v["halogen:cache_hits_total"], json!(275.0));
        assert!(v.get("broken").is_none());
    }

    #[test]
    fn halogen_engine_parses() {
        assert_eq!(
            crate::engine::Engine::parse("halogen"),
            Some(crate::engine::Engine::Halogen)
        );
        assert_eq!(crate::engine::Engine::Halogen.as_str(), "halogen");
        assert!(crate::engine::Engine::Halogen.is_local());
    }

    #[test]
    fn gufo_engine_parses() {
        assert_eq!(
            crate::engine::Engine::parse("gufo"),
            Some(crate::engine::Engine::Gufo)
        );
        assert_eq!(crate::engine::Engine::Gufo.as_str(), "gufo");
        assert!(crate::engine::Engine::Gufo.is_local());
        assert_eq!(
            crate::engine::Engine::Gufo.default_protocol(),
            crate::types::Protocol::OpenaiChat
        );
        assert!(crate::engine::Engine::Gufo.default_webui_path().is_none());
        // 无裸串风险：未知 engine 字符串必须落 None，而不是静默 Custom（在线计费口径）
        assert_eq!(crate::engine::Engine::parse("guffo"), None);
    }

    #[test]
    fn channel_lookup_is_strict() {
        assert!(channel_of("halogen").is_ok());
        assert!(channel_of("").is_ok());
        assert!(channel_of("gufo").is_ok());
        // 未注册通道明报错，不静默兜底（乱串打到错误机位比报错更糟）
        assert!(channel_of("qwenox").is_err());
        assert!(channel_of("drop table").is_err());
        // 两个机位的关键事实各自独立：脚本名/容器/仓/备份键互不串
        let hal = channel_of("halogen").unwrap();
        let gu = channel_of("gufo").unwrap();
        assert_eq!(hal.container, "halogen");
        assert_eq!(gu.container, "gufo");
        assert_eq!(hal.ver, "hal-ver.sh");
        assert_eq!(gu.ver, "gu-ver.sh");
        assert_eq!(hal.backup_key, "hal");
        assert_eq!(gu.backup_key, "gu");
    }

    /// 真机冒烟（手动跑）：cargo test --features gui --lib halctl -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "需要能 SSH 到 102"]
    async fn live_status_and_logs() {
        let hal = HalctlCfg::default();
        let gu = crate::config::Config::default().guctl;
        let s = status(&hal, &HAL).await.expect("status 失败");
        println!(
            "host={} running={} model={}",
            s["host"], s["running"], s["health"]["model"]
        );
        let l = logs(&hal, &HAL, 500, "info").await.expect("logs 失败");
        let n = l["lines"].as_array().map(|a| a.len()).unwrap_or(0);
        println!("日志行数={n}");
        assert!(n > 0, "日志应有内容");
        let v = versions(&hal, &HAL).await.expect("versions 失败");
        println!(
            "current={} local={} remote={}",
            v["current"],
            v["local"].as_array().map(|a| a.len()).unwrap_or(0),
            v["remote"].as_array().map(|a| a.len()).unwrap_or(0)
        );
        // 版本只断言 x.y.z 格式（写死版本号会在每一次正常升级后过期——本机 0.18.9→0.18.21 即一例）
        assert!(valid_tag(v["current"].as_str().unwrap_or_default()));
        let e = env(&hal, &HAL).await.expect("env 失败");
        println!(
            "参数组={} 键={} 预设={}",
            e["spec"].as_array().map(|a| a.len()).unwrap_or(0),
            e["values"].as_object().map(|o| o.len()).unwrap_or(0),
            e["presets"].as_array().map(|a| a.len()).unwrap_or(0)
        );
        assert!(e["spec"].as_array().map(|a| !a.is_empty()).unwrap_or(false));

        // gufo 机位：当前未运行也应快照成功（不假走 halogen 的端口/日志脚本）
        let gs = status(&gu, &GU).await.expect("gufo status 失败");
        println!("gufo host={} running={}", gs["host"], gs["running"]);
        let gl = logs(&gu, &GU, 200, "info").await.expect("gufo logs 失败");
        println!(
            "gufo 日志行数={}",
            gl["lines"].as_array().map(|a| a.len()).unwrap_or(0)
        );
        let gv = versions(&gu, &GU).await.expect("gufo versions 失败");
        println!(
            "gufo current={} remote={}（首项 {}）",
            gv["current"],
            gv["remote"].as_array().map(|a| a.len()).unwrap_or(0),
            gv["remote"]
                .as_array()
                .and_then(|a| a.first())
                .cloned()
                .unwrap_or_default()
        );
        let ge = env(&gu, &GU).await.expect("gufo env 失败");
        println!(
            "gufo 参数键={}",
            ge["values"].as_object().map(|o| o.len()).unwrap_or(0)
        );
    }
}
