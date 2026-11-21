//! 遥控器 IPC 门面（计划 §六；受 gui feature 门，macOS 才有实现）。
//!
//! 命令名按计划定稿：remote_status / remote_scan / remote_connect / remote_disconnect。
//! 非 macOS 平台这些命令不注册（页面在非 macOS 整页降级，不发 IPC，计划 §五口径）。

use crate::gateway::State;
use crate::services::remote::actions::ButtonConfig;
use crate::services::remote::hid::ButtonChannel;
use crate::services::remote::identity::{RemoteDevice, RemoteSession, RemoteStatus};
use crate::services::remote::voice::VoiceChannel;
use std::sync::Arc;
use tauri::State as TauriState;

/// 页头状态卡：绑定态 + 候选 + BLE 通道状态。纯读，不触发扫描。
#[tauri::command]
pub fn remote_status(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
) -> Result<RemoteStatus, String> {
    Ok(session.status(&st.store))
}

/// 短扫描一轮（约 2.5s），回白名单候选。
#[tauri::command]
pub async fn remote_scan(
    session: TauriState<'_, Arc<RemoteSession>>,
) -> Result<Vec<RemoteDevice>, String> {
    session.scan().await
}

/// 连接指定候选并读 DIS 型号；成功即落绑定。
#[tauri::command]
pub async fn remote_connect(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    id: String,
) -> Result<RemoteDevice, String> {
    session.connect(&st.store, &id).await
}

#[tauri::command]
pub async fn remote_disconnect(session: TauriState<'_, Arc<RemoteSession>>) -> Result<(), String> {
    session.disconnect().await
}

/// 按键配置 + HID 通道状态（页面按键映射区一次拿齐：config + hid{mode/error/last_button}）。
/// 配置按档案分仓（21b）：当前档案 = 绑定设备的档案，未绑定回小米（与绑定串同兜底）。
#[tauri::command]
pub fn remote_buttons_get(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
) -> Result<serde_json::Value, String> {
    Ok(channel.snapshot(&bound_profile(&session, &st.store)))
}

/// 当前键位配置所属档案：绑定设备的档案；未绑定 = 小米（历史默认）。
fn bound_profile(session: &RemoteSession, store: &crate::store::Store) -> String {
    session
        .status(store)
        .bound
        .as_ref()
        .and_then(|b| b.profile.clone())
        .unwrap_or_else(|| "xiaomi_rc00x".to_string())
}

/// 录制闸门：true = 槽位正在录快捷键，tap 拦下全部键盘输入（含遥控按键）转交前端，
/// 不透传不触发映射；false = 恢复。纯开关无返回，旧前端不调用即零影响。
#[tauri::command]
pub fn remote_recording_set(channel: TauriState<'_, Arc<ButtonChannel>>, active: bool) {
    channel.set_recording(active);
}

/// 演示/自动化专用：把一颗键的手势喂进与真键完全相同的事件出口（映射页高亮、
/// 运行态闪示与真机一致）。**只在 debug 构建可用**，只演 UI、不做动作注入。
#[tauri::command]
pub fn remote_simulate_key(
    channel: TauriState<'_, Arc<ButtonChannel>>,
    id: String,
    gesture: String,
) -> Result<(), String> {
    channel.inner().simulate_named(&id, &gesture)
}

/// 保存按键配置（整体替换，写当前档案的分仓文件并即时生效）。
/// 冲突是**唯一**拒绝条件（无黑名单，计划 §五）：整份拒绝，防两个窗口互踩。
#[tauri::command]
pub fn remote_buttons_set(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    config: ButtonConfig,
) -> Result<(), String> {
    if let Some(c) = config.find_conflict() {
        return Err(format!(
            "按键冲突：键 {}·{} 与键 {}·{} 绑定了同一快捷键",
            c.first.0,
            c.first.1.label(),
            c.second.0,
            c.second.1.label()
        ));
    }
    channel.set_config(&bound_profile(&session, &st.store), config)
}

/// 预存方案名列表（下拉内容，用户 2026-11-20 拍板）。
#[tauri::command]
pub fn remote_schemes_get(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
) -> Result<Vec<String>, String> {
    Ok(channel.scheme_names(&bound_profile(&session, &st.store)))
}

/// 把当前映射存为命名方案（同名覆盖）。名字去空白后不许为空。
#[tauri::command]
pub fn remote_scheme_save(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    name: String,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("方案名称不能为空".into());
    }
    channel.scheme_save(&bound_profile(&session, &st.store), name)
}

/// 应用方案：整体覆盖当前映射（前端弹警告二次确认后才走到这里）。
#[tauri::command]
pub fn remote_scheme_apply(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    name: String,
) -> Result<(), String> {
    channel.scheme_apply(&bound_profile(&session, &st.store), name.trim())
}

/// 改名方案（新名占用报错）。
#[tauri::command]
pub fn remote_scheme_rename(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    old: String,
    new: String,
) -> Result<(), String> {
    let (old, new) = (old.trim(), new.trim());
    if old.is_empty() || new.is_empty() {
        return Err("方案名称不能为空".into());
    }
    channel.scheme_rename(&bound_profile(&session, &st.store), old, new)
}

/// 导出方案为 JSON 文本（前端 blob 下载存盘）。
#[tauri::command]
pub fn remote_scheme_export(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    name: String,
) -> Result<String, String> {
    channel.scheme_export(&bound_profile(&session, &st.store), name.trim())
}

/// 从 JSON 文本导入方案（同名覆盖）。
#[tauri::command]
pub fn remote_scheme_import(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    name: String,
    content: String,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("方案名称不能为空".into());
    }
    channel.scheme_import(&bound_profile(&session, &st.store), name, &content)
}

/// 语音通道快照：状态 + 偏好 + 上次会话指标 + 可选输出设备（页面一次拿齐）。
#[tauri::command]
pub fn remote_voice_get(
    channel: TauriState<'_, Arc<VoiceChannel>>,
) -> Result<serde_json::Value, String> {
    Ok(channel.snapshot())
}

/// 保存语音偏好（出口三选一 + 路 2 设备 + 音量 + 路 3 模型），即时落库生效。
#[tauri::command]
pub fn remote_voice_set(
    st: TauriState<'_, Arc<State>>,
    channel: TauriState<'_, Arc<VoiceChannel>>,
    settings: serde_json::Value,
) -> Result<(), String> {
    channel.set_settings(&st.store, settings)
}

/// 本机已启用、可选择的输入法清单（路 2 组合框数据源）。
#[tauri::command]
pub fn remote_ime_list() -> Result<Vec<crate::services::remote::ime::ImeSource>, String> {
    Ok(crate::services::remote::ime::list())
}

/// 打开系统「键盘 → 输入法」设置面板。
#[tauri::command]
pub fn remote_ime_open_settings() -> Result<(), String> {
    crate::services::remote::ime::open_settings();
    Ok(())
}

/// 系统授权批量检测（辅助功能/输入监控/语音识别/麦克风/蓝牙）。纯读，不弹请求窗。
#[tauri::command]
pub fn remote_perms_check() -> Result<Vec<crate::services::remote::perms::Perm>, String> {
    Ok(crate::services::remote::perms::check())
}

/// 打开某授权项对应的系统设置面板（id 见 perms::check；ime 走输入法面板）。
#[tauri::command]
pub fn remote_perm_open(id: String) -> Result<(), String> {
    crate::services::remote::perms::open(&id)
}
