//! 推理引擎控制台的 IPC 薄壳：逻辑在 `services::halctl`，这里只做通道分派与序列化。
//! 命令只在 `--features gui` 下编译（commands 模块整体 gui 门控）。
//!
//! `ch` 参数选择控制面机位：halogen（缺省，老调用零破坏）或 gufo——同一台 102、
//! 同一套 SSH 故障转移，只是部署目录与脚本名不同（hal.sh/gufo.sh、hal-ver.sh/gu-ver.sh…）。
//! 通道未注册明报错，绝不静默兜底（乱串打到错误机位比报错更糟）。

use crate::config::{Config, HalctlCfg};
use crate::gateway::State;
use crate::services::halctl::{self, Channel};
use serde_json::Value;
use std::sync::Arc;
use tauri::State as TauriState;

/// ch 参数 → (该机位的配置节, 机位描述)。halogen 缺省。
fn dispatch(cfg: &Config, ch: Option<String>) -> Result<(&HalctlCfg, &'static Channel), String> {
    let c = halctl::channel_of(ch.as_deref().unwrap_or("halogen"))?;
    let ctl = if std::ptr::eq(c, &halctl::HAL) {
        &cfg.halctl
    } else {
        &cfg.guctl
    };
    Ok((ctl, c))
}

/// 控制台快照：容器状态 + health + cache + metrics（一次 SSH 往返）。
#[tauri::command]
pub async fn halctl_status(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::status(ctl, c).await
}

/// 启停控制：action ∈ start|stop|restart。start/restart 阻塞到就绪（数分钟）。
#[tauri::command]
pub async fn halctl_control(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
    action: String,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::control(ctl, c, &action).await
}

/// 日志尾读：level ∈ info|debug|trace|raw，复用远端中文渲染器（hal_log_cn.py /
/// gu_log_cn.py），两个引擎各用各的翻译器但同一接口。
#[tauri::command]
pub async fn halctl_logs(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
    tail: u32,
    level: String,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::logs(ctl, c, tail, &level).await
}

/// 版本信息：当前 tag + 本地镜像 + 备份槽 + 官方 tag 列表。
#[tauri::command]
pub async fn halctl_versions(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::versions(ctl, c).await
}

/// 切到指定版本（更新/恢复共用）。tag 白名单校验在 service 层。
#[tauri::command]
pub async fn halctl_switch(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
    tag: String,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::switch_version(ctl, c, &tag).await
}

/// 把当前版本钉进永久备份槽。
#[tauri::command]
pub async fn halctl_backup(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::backup_set(ctl, c).await
}

/// 从备份槽恢复（目标 tag 由 102 端读取，不信任前端）。
#[tauri::command]
pub async fn halctl_restore(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::restore(ctl, c).await
}

/// 删除本地镜像（备份引擎版本）。当前版本与运行中镜像由 102 端拒绝。
#[tauri::command]
pub async fn halctl_image_del(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
    tag: String,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::image_delete(ctl, c, &tag).await
}

/// 参数面板快照：规格 + 当前值 + 命名预设列表。
#[tauri::command]
pub async fn halctl_env(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::env(ctl, c).await
}

/// 保存参数（空值=移除键）；写 halogen.env / gufo.conf，重启后生效。
#[tauri::command]
pub async fn halctl_env_save(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
    values: Value,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::env_save(ctl, c, values).await
}

/// 把当前参数存成命名预设。
#[tauri::command]
pub async fn halctl_env_preset_save(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
    name: String,
    values: Value,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::env_preset_save(ctl, c, &name, values).await
}

/// 一键应用预设（写参数 + 若在跑则重启）。
#[tauri::command]
pub async fn halctl_env_preset_apply(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
    name: String,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::env_preset_apply(ctl, c, &name).await
}

/// 删除命名预设。
#[tauri::command]
pub async fn halctl_env_preset_del(
    st: TauriState<'_, Arc<State>>,
    ch: Option<String>,
    name: String,
) -> Result<Value, String> {
    let (ctl, c) = dispatch(&st.config, ch)?;
    halctl::env_preset_del(ctl, c, &name).await
}

/// 输出尾部截断（错误信息里带远端输出，太长没法看也没法存）。
fn tail_snip(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return s.to_string();
    }
    "…".to_string() + &chars[chars.len() - n..].iter().collect::<String>()
}

/// 互斥引擎切换（一个后端两种引擎实现）：**显式**先停运行中的引擎再起目标引擎，
/// 成功后 DB 里 engine/alt_engine 互换 + capabilities 重写为目标预设（能力口径跟切）。
/// 中途失败回滚：目标起不来且原引擎切换前在跑 → 重新拉起原引擎恢复现场，错误如实报。
/// 与脚本侧互斥守卫（gufo.sh/hal.sh 谁起谁先停对方）是双保险：控制台走显式编排，
/// 不靠「顺手停」；守卫兜住 CLI 直连的场合。
#[tauri::command]
pub async fn halctl_use_engine(
    st: TauriState<'_, Arc<State>>,
    backend: String,
    engine: String,
) -> Result<Value, String> {
    let row = st
        .store
        .list_backends()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.name == backend)
        .ok_or_else(|| format!("后端不存在：{backend}"))?;
    let cur = row.engine.as_deref().unwrap_or("halogen").to_string();
    let alt = row
        .alt_engine
        .clone()
        .ok_or_else(|| format!("后端 {backend} 没有配对的互斥引擎（alt_engine 为空）"))?;
    if engine == cur {
        return Ok(serde_json::json!({ "ok": true, "engine": cur, "changed": false }));
    }
    if engine != alt {
        return Err(format!(
            "目标引擎 {engine} 不在配对内（当前 {cur} / 配对 {alt}）"
        ));
    }
    let cfg = st.config.as_ref();
    let (cfg_from, ch_from) = dispatch(cfg, Some(cur.clone()))?;
    let (cfg_to, ch_to) = dispatch(cfg, Some(engine.clone()))?;

    // 预检：链路不可达就原样退出——盲停盲起只会把在跑的引擎白白折掉。
    let was_running = halctl::status(cfg_from, ch_from)
        .await
        .map(|s| s["running"].as_bool().unwrap_or(false))
        .map_err(|e| format!("查不到当前引擎状态，切换中止：{e}"))?;

    if was_running {
        let stop = halctl::control(cfg_from, ch_from, "stop").await?;
        if stop["ok"].as_bool() != Some(true) {
            return Err(format!(
                "停止原引擎 {cur} 失败，切换中止（未动目标引擎）：{}",
                tail_snip(stop["output"].as_str().unwrap_or(""), 400)
            ));
        }
    }

    let start_res = halctl::control(cfg_to, ch_to, "start").await;
    let start_ok = start_res
        .as_ref()
        .map(|v| v["ok"].as_bool() == Some(true))
        .unwrap_or(false);
    if !start_ok {
        let why = match &start_res {
            Ok(v) => tail_snip(v["output"].as_str().unwrap_or("启动失败"), 400),
            Err(e) => e.clone(),
        };
        // 回滚：切换前在跑才拉回原引擎（本来就没跑就别无中生有）。
        let mut rolled_back = false;
        if was_running {
            if let Ok(rb) = halctl::control(cfg_from, ch_from, "start").await {
                rolled_back = rb["ok"].as_bool() == Some(true);
            }
        }
        let note = if !was_running {
            "（切换前原引擎本就未运行，保持停止）".to_string()
        } else if rolled_back {
            format!("；已回滚到原引擎 {cur}")
        } else {
            format!("；回滚 {cur} 也失败——两个引擎都没在跑，需手动处理").to_string()
        };
        return Err(format!("目标引擎 {engine} 启动失败：{why}{note}"));
    }

    // 能力表跟切：capabilities 列重写为目标引擎预设（swap_backend_engine 注释有口径）。
    let caps = crate::engine::Engine::parse(&engine)
        .map(|e| e.capabilities())
        .unwrap_or_else(|| row.capabilities.clone());
    st.store
        .swap_backend_engine(&backend, &engine, Some(&cur), &caps)
        .map_err(|e| format!("引擎已切换但落库失败（重启后会回到 {cur}）：{e}"))?;
    // DB 是真值，镜像回 yaml（与后端增删改同口径），否则重启被播种洗回旧引擎。
    crate::config::persist_backends_to_yaml(cfg, &st.store)
        .map_err(|e| format!("引擎已切到 {engine} 但 config.yaml 回写失败：{e}"))?;
    Ok(serde_json::json!({
        "ok": true, "engine": engine, "changed": true, "was_running": was_running,
    }))
}
