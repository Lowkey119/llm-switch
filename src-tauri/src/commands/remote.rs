//! 遥控器 IPC 门面（计划 §六；受 gui feature 门，macOS 才有实现）。
//!
//! 命令名按计划定稿：remote_status / remote_scan / remote_connect / remote_disconnect。
//! 非 macOS 平台这些命令不注册（页面在非 macOS 整页降级，不发 IPC，计划 §五口径）。

use crate::gateway::State;
use crate::services::remote::actions::ButtonConfig;
use crate::services::remote::appfocus::AppFocusProfile;
use crate::services::remote::buttons::KeyInteraction;
use crate::services::remote::hid::ButtonChannel;
use crate::services::remote::identity::{RemoteDevice, RemoteSession, RemoteStatus};
use crate::services::remote::macros::Macro;
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

/// 当前键位配置所属档案：绑定设备的档案；未绑定回兜底档案。
/// 口径在 `hid::bound_profile`——前台 App 监听走同一份，两处不许各写一遍。
fn bound_profile(session: &RemoteSession, store: &crate::store::Store) -> String {
    crate::services::remote::hid::bound_profile(session, store)
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

/// 读快速面板专用键位表（遥控器页「面板」档编辑用；与生效表分离）。
/// 文件缺失 = 空表（全原生透传），不回落默认映射。
#[tauri::command]
pub fn remote_panel_buttons_get(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
) -> Result<ButtonConfig, String> {
    Ok(channel.panel_config_for(&bound_profile(&session, &st.store)))
}

/// 保存快速面板专用键位表（整体替换，写独立文件并即时生效）。
/// 冲突检查与日常表同口径：唯一拒绝条件，整份拒绝防互踩。
#[tauri::command]
pub fn remote_panel_buttons_set(
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
    channel.set_panel_config(&bound_profile(&session, &st.store), config)
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

/// 删除方案（前端弹警告二次确认后才走到这里）；指向它的 App 绑定一并清掉。
#[tauri::command]
pub fn remote_scheme_delete(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    name: String,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("方案名称不能为空".into());
    }
    channel.scheme_delete(&bound_profile(&session, &st.store), name)
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

/// 当前前台 App + 生效来源（C5）：卡头显示「默认 / 某 App → 某方案」，也是「绑到前台 App」的读数。
#[tauri::command]
pub fn remote_front_app_get(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
) -> Result<serde_json::Value, String> {
    let profile = bound_profile(&session, &st.store);
    Ok(serde_json::json!({
        "app": channel.front_app(),
        "source": channel.source_json(&profile),
    }))
}

/// App 绑定表（C5）：`bundle_id → 方案名`（精确匹配）。
#[tauri::command]
pub fn remote_app_bindings_get(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    Ok(channel.app_bindings(&bound_profile(&session, &st.store)))
}

/// 把某个 App 绑到方案（C5）；`scheme` 空 = 解绑。绑完立即重算当前生效来源。
#[tauri::command]
pub fn remote_app_binding_set(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    bundle_id: String,
    scheme: Option<String>,
) -> Result<(), String> {
    let scheme = scheme.as_deref().map(str::trim).filter(|s| !s.is_empty());
    channel.set_app_binding(&bound_profile(&session, &st.store), &bundle_id, scheme)
}

/// 宏列表（C3）：当前档案宏库里的全部宏，按名排序。
#[tauri::command]
pub fn remote_macros_get(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
) -> Result<Vec<Macro>, String> {
    Ok(channel.macro_list(&bound_profile(&session, &st.store)))
}

/// 存宏（同 id 覆盖 = 改名/改步骤都走这条）。`id` 留空 = 新建，回存好的整条（含新 id）。
/// 校验（名称非空、至少一步、延迟封顶、不许套宏）在 `macros::Macro::normalize`。
#[tauri::command]
pub fn remote_macro_save(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    def: Macro,
) -> Result<Macro, String> {
    channel.macro_save(&bound_profile(&session, &st.store), def)
}

/// 删宏，并跨「当前映射 + 全部方案」清掉引用它的槽位（回清掉的条数，页面据此提示）。
#[tauri::command]
pub fn remote_macro_delete(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    id: String,
) -> Result<usize, String> {
    channel.macro_delete(&bound_profile(&session, &st.store), id.trim())
}

/// 改动作列表次序（计划 2026-11-25 片③：动作列表整组挪位置）。
/// `ids` = 排完序的全量 id（第 k 条拿 order=k）；没点到的宏按原序接在后面。
/// 为什么要在后端存序而不是前端自己记：宏库是 id→map，顺序不落库就存不住，
/// 重启就弹回按名排（与 tab 列表、步骤列表的「顺序存在数据里」口径一致）。
#[tauri::command]
pub fn remote_macro_reorder(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    ids: Vec<String>,
) -> Result<(), String> {
    channel.macro_reorder(&bound_profile(&session, &st.store), &ids)
}

/// 键交互设置（C4）：双击窗 / 长按窗 / 按住模式 / 连发起始与间隔，按档案一份。
#[tauri::command]
pub fn remote_key_interaction_get(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
) -> Result<KeyInteraction, String> {
    Ok(channel.interaction_for(&bound_profile(&session, &st.store)))
}

/// 存键交互设置（C4）。越界按边界截（回的是截完的真值，页面显示的就是生效值），
/// 存完立刻生效，不必重开遥控器连接。
#[tauri::command]
pub fn remote_key_interaction_set(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    def: KeyInteraction,
) -> Result<KeyInteraction, String> {
    channel.set_interaction(&bound_profile(&session, &st.store), def)
}

/// 某个 App 的聚焦策略（C6）：打开它之后要不要补一次聚焦、等多久、用什么方式。
/// 没配回 None——打开就是纯打开，不抢焦点。
#[tauri::command]
pub fn remote_app_focus_get(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    bundle_id: String,
) -> Result<Option<AppFocusProfile>, String> {
    Ok(channel.app_focus_get(&bound_profile(&session, &st.store), &bundle_id))
}

/// 存某个 App 的聚焦策略（C6）。策略 `{"type":"none"}` = 取消聚焦（条目删除），
/// 回 `true` 表示确实动了条目；从没配过再取消回 `false`，不算失败。
#[tauri::command]
pub fn remote_app_focus_set(
    st: TauriState<'_, Arc<State>>,
    session: TauriState<'_, Arc<RemoteSession>>,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    def: AppFocusProfile,
) -> Result<bool, String> {
    channel.app_focus_set(&bound_profile(&session, &st.store), def)
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
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn remote_ime_list() -> Result<Vec<crate::services::remote::ime::ImeSource>, String> {
    Ok(crate::services::remote::ime::list())
}

/// 打开系统「键盘 → 输入法」设置面板。
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn remote_ime_open_settings() -> Result<(), String> {
    crate::services::remote::ime::open_settings();
    Ok(())
}

/// 已安装应用清单（C7「打开应用」选择器数据源）：进程内缓存，启动已在后台预热
/// （`gui` setup 调 `applist::warm`），`refresh = true` 手动作废重扫。
///
/// **异步命令**：同步命令跑在主线程，扫盘（上百份 Info.plist）会把整个界面
/// 卡住（用户 2026-11-23 实报「一点『打开应用』就转圈」）；挪到 blocking 线程后
/// 界面始终活，前端该转圈只转列表那一块。
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn remote_apps_get(
    refresh: bool,
) -> Result<Vec<crate::services::remote::applist::AppEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::services::remote::applist::apps(refresh))
        .await
        .map_err(|e| format!("应用清单读取失败：{e}"))
}

/// 可绑进动作的代理 CLI 清单（用户 2026-11-23 要求「项目支持的代理 CLI 带参进动作」）。
///
/// `app` 键名与接管页同源（`ClientApp::as_str`），可执行路径取安装扫描的**登录 shell**
/// 探测结果——GUI 进程自身 PATH 不含 nvm/homebrew，只有那份绝对路径才真能跑起来。
/// 纯读缓存，不等 shell（冷启动头几秒 `probed = false`，前端据此提示"稍等/去代理页重扫"）。
#[tauri::command]
pub fn remote_agents_cli() -> Result<Vec<AgentCliEntry>, String> {
    use crate::services::agents_scan;
    use crate::takeover::ClientApp;
    Ok(ClientApp::ALL
        .iter()
        .map(|a| {
            let d = agents_scan::cached(*a);
            AgentCliEntry {
                app: a.as_str().to_string(),
                label: a.label().to_string(),
                bin: a.as_str().to_string(),
                bin_path: d.as_ref().and_then(|x| x.bin_path.clone()),
                version: d.as_ref().and_then(|x| x.version.clone()),
                broken: d.as_ref().map(|x| x.broken).unwrap_or(false),
                probed: d.is_some(),
            }
        })
        .collect())
}

/// [`remote_agents_cli`] 的一行。
#[derive(serde::Serialize)]
pub struct AgentCliEntry {
    /// 键名（写进动作里的 `app`）。
    pub app: String,
    /// 给人看的名字（Codex CLI / Claude Code…）。
    pub label: String,
    pub bin: String,
    pub bin_path: Option<String>,
    pub version: Option<String>,
    /// 装了但 `--version` 跑不起来（如 Node 版本不达标）。
    pub broken: bool,
    /// 首轮探测是否已有结论（false = 冷启动还在后台跑，"没装"暂不可信）。
    pub probed: bool,
}

/// 某个 .app 的图标（PNG → base64）。按需取：整表预取几百个图标会把 IPC 撑爆。
///
/// 同步命令（主线程）是故意的：`NSWorkspace` 出图走 AppKit，放工作线程有「非主线程
/// 访问 AppKit」的崩溃风险。代价是主线程要为每张图付一次 TIFF→PNG，所以前端
/// **只给滚进视口的行取图**（见 `AppPicker.tsx` 的 IntersectionObserver）——
/// 整表几十上百行一起要，就是「点『打开应用』卡死转圈」的真因。
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn remote_app_icon(path: String) -> Result<String, String> {
    crate::services::remote::applist::icon_png(&path)
}

/// 「浏览…」：原生面板挑一个 .app（装在扫描目录之外的也能绑）。
/// 同步命令跑在主线程，NSOpenPanel 要的正是主线程；取消回 None，不算失败。
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn remote_app_pick() -> Result<Option<crate::services::remote::applist::AppEntry>, String> {
    crate::services::remote::applist::pick_app()
}

/// 系统授权批量检测（辅助功能/输入监控/语音识别/麦克风/蓝牙）。纯读，不弹请求窗。
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn remote_perms_check() -> Result<Vec<crate::services::remote::perms::Perm>, String> {
    Ok(crate::services::remote::perms::check())
}

/// 打开某授权项对应的系统设置面板（id 见 perms::check；ime 走输入法面板）。
#[cfg(target_os = "macos")]
#[tauri::command]
pub fn remote_perm_open(id: String) -> Result<(), String> {
    crate::services::remote::perms::open(&id)
}
