//! 提示词速取面板 IPC：prompt_items 的查/增/改/删（浮窗与快捷键见 gui::promptfloat）。
//!
//! 归属口径：`tab` = `"common"`（通用库）或某代理的 `ClientApp::as_str()`，
//! 命令层把关，坏 tab 进不了库；内容原样存文本，注入时逐字进剪贴板。

use crate::gateway::State;
use crate::gui::promptfloat;
use crate::services::quickpanel::{self as qp, QpTab};
use crate::services::remote::actions::Action;
use crate::services::remote::hid::ButtonChannel;
use crate::takeover::ClientApp;
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State as TauriState};

/// tab 必须是 "common" 或已登记代理（唯一真相 ClientApp，不另立名单）。
fn validate_tab(tab: &str) -> Result<(), String> {
    if tab == "common" || ClientApp::ALL.iter().any(|c| c.as_str() == tab) {
        Ok(())
    } else {
        Err(format!("未知归属 `{tab}`（应为 common 或已知代理）"))
    }
}

/// 内容非空且有上限（面板侧编辑窗是自由文本，8000 字足够任何提示词）。
fn validate_content(content: &str) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("内容不能为空".into());
    }
    if content.chars().count() > 8000 {
        return Err("内容过长（上限 8000 字）".into());
    }
    Ok(())
}

#[tauri::command]
pub fn prompt_list(st: TauriState<Arc<State>>) -> Result<Value, String> {
    let rows = st.store.list_prompt_items().map_err(|e| e.to_string())?;
    serde_json::to_value(rows).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn prompt_create(
    st: TauriState<Arc<State>>,
    tab: String,
    content: String,
) -> Result<i64, String> {
    validate_tab(&tab)?;
    validate_content(&content)?;
    st.store
        .create_prompt_item(&tab, &content)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn prompt_update(st: TauriState<Arc<State>>, id: i64, content: String) -> Result<bool, String> {
    validate_content(&content)?;
    st.store
        .update_prompt_item(id, &content)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn prompt_delete(st: TauriState<Arc<State>>, id: i64) -> Result<bool, String> {
    st.store.delete_prompt_item(id).map_err(|e| e.to_string())
}

/// 记一次取用（高频条目以次排序靠前的唯一数据源）。
/// 取用失败不能影响注入本身，所以调用方 `.catch` 掉就行，这里只负责落库。
#[tauri::command]
pub fn prompt_use(st: TauriState<Arc<State>>, id: i64) -> Result<bool, String> {
    st.store.bump_prompt_item(id).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------- 窗口与快捷键

/// 前端 Esc / 注入后收窗走这里（显隐必须回主线程，见 promptfloat）。
#[tauri::command]
pub fn prompt_set_visible(app: AppHandle, visible: bool) {
    promptfloat::set_visible(&app, visible);
}

/// 面板形态：圆钮/贴边 + **当前可见态**。顶栏那颗快捷开关要靠可见态决定图标与
/// `aria-pressed`，不然只有“显示”没有“收起”，用户只能靠 Esc。
#[tauri::command]
pub fn prompt_shape(app: AppHandle) -> Value {
    let mut v = promptfloat::shape();
    let visible = app
        .get_webview_window(promptfloat::LABEL)
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    v["visible"] = serde_json::json!(visible);
    v
}

/// 点圆钮展开回面板态。
#[tauri::command]
pub fn prompt_expand(app: AppHandle) {
    promptfloat::expand(&app);
}

/// 编辑态临时加宽/还原（前端开/关多行编辑器时各发一次）。
#[tauri::command]
pub fn prompt_widen(app: AppHandle, extra: i32) {
    promptfloat::widen(&app, extra);
}

/// 当前唤起快捷键（accelerator 串，设置页回显用）。
#[tauri::command]
pub fn prompt_hotkey_get(app: AppHandle) -> String {
    promptfloat::hotkey(&app)
}

/// 换唤起快捷键：注册成功才落库；被占用则旧键原位装回并返回人话错误。
#[tauri::command]
pub fn prompt_hotkey_set(app: AppHandle, accelerator: String) -> Result<String, String> {
    promptfloat::set_hotkey(&app, &accelerator)
}

/// 注入高亮条目：剪贴板已由前端写好，这里收窗并把内容粘进唤起前的 App。
/// 注入方式查询：`insert`（光标处插入，默认）/ `replace`（先全选再粘贴，整段替换）。
#[tauri::command]
pub fn prompt_mode_get(app: AppHandle) -> String {
    promptfloat::inject_mode(&app).to_string()
}

/// 设注入方式（代理页「提示词与命令」管理面板用）；非法值由后端把关。
#[tauri::command]
pub fn prompt_mode_set(app: AppHandle, mode: String) -> Result<String, String> {
    promptfloat::set_inject_mode(&app, &mode).map(str::to_string)
}

#[tauri::command]
pub fn prompt_inject(app: AppHandle) {
    promptfloat::inject(&app);
}

/// 面板显示前缓存的注入目标（App 名 + 该终端里跑着的代理）。
/// 前端拿它显示“将注入 Ghostty”、并把 tab 切到激活代理。
#[tauri::command]
pub fn prompt_target() -> Option<promptfloat::Target> {
    promptfloat::target()
}

/// 辅助功能是否授权给**当前运行的这个构建**（模拟粘贴的前置条件）。
/// 为什么必须实时探：mac 的授权认签名身份，ad-hoc 签的包每次重编都是新身份，
/// 旧条目勾着也不作数——静态提示会把人带偏（用户实报「删了重勾还是注入失败」）。
#[tauri::command]
pub fn prompt_a11y_status() -> bool {
    promptfloat::can_simulate()
}

/// 用户主动点「授权」：让系统弹一次辅助功能授权框（注入路径上永远不弹）。
/// 返回当前是否已授权；没授权时弹框，用户勾完再点一次就是 true。
#[tauri::command]
pub fn prompt_a11y_ask() -> bool {
    promptfloat::ask_permission()
}

/// 打开系统辅助功能设置面板（引导用户授权；仅 mac 有意义）。
#[tauri::command]
pub fn prompt_a11y_open() {
    promptfloat::open_a11y_settings();
}

/// 清掉本 App 在「辅助功能」里的授权条目，让用户给**当前构建**重新勾。
///
/// 为什么必须有：macOS 发行包是 ad-hoc 签名（无 Developer ID），TCC 认的身份就是
/// cdhash——每次重新构建换一个身份，新构建在 TCC 眼里是新 App。旧条目还留在列表里
/// 勾着（看着像「早给了权限」），新构建却只能一遍遍弹窗。只 reset 自己的 bundle id，
/// 不碰别的 App。
#[tauri::command]
pub fn prompt_a11y_reset(app: AppHandle) -> Result<(), String> {
    let bundle = app.config().identifier.clone();
    let status = std::process::Command::new("tccutil")
        .args(["reset", "Accessibility", bundle.as_str()])
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("tccutil reset 未生效".to_string())
    }
}

// ==================== 快速面板 v2：用户自定义 tab 配置 + 动作执行 ====================

/// 内置默认 tab 配置（quickpanel.json 不存在时使用；不落盘，改文件即自定义）。
/// type 决定 OK 语义：prompts=注入 / chars=插入字符 / actions=执行命令 / skills=只读 / config=配置预览。
/// dynamic:"agent" 的 tab 由前端换成当前注入目标的代理；items 为 chars/actions 的内容。
fn default_tabs() -> serde_json::Value {
    let ctrl: Vec<serde_json::Value> = (b'A'..=b'Z')
        .map(|c| {
            let ch = (c as char).to_string();
            serde_json::json!({ "t": format!("^{ch}"), "n": ctrl_note(&ch) })
        })
        .collect();
    let plain = |chars: &[&str]| -> Vec<serde_json::Value> {
        chars
            .iter()
            .map(|s| serde_json::json!({ "t": s, "n": "" }))
            .collect()
    };
    const SYM: [&str; 24] = [
        "/", "@", "!", "?", ".", ",", "_", "-", "~", "|", ">", "<", "&", "$", "*", "#", "%", "+",
        "=", ":", "'", "\"", "\\", "`",
    ];
    const NUM: [&str; 16] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "(", ")", "[", "]", "{", "}",
    ];
    let sym = plain(&SYM);
    let num = plain(&NUM);
    serde_json::json!([
        { "key": "common", "label": "通用", "type": "prompts" },
        { "key": "shell", "label": "命令", "type": "prompts" },
        { "key": "agent", "label": "激活代理", "type": "prompts", "dynamic": "agent" },
        { "key": "skills", "label": "项目接管", "type": "skills" },
        { "key": "hot", "label": "高频件", "type": "chars", "sep": true, "items": [
            { "t": "^C", "n": "打断" }, { "t": "^D", "n": "退出" }, { "t": "^L", "n": "清屏" },
            { "t": "^R", "n": "搜历史" }, { "t": "^U", "n": "删到行首" }, { "t": "^W", "n": "删词" },
            { "t": "↑", "n": "历史上" }, { "t": "↓", "n": "历史下" }, { "t": "⇥", "n": "制表符" },
            { "t": "⏎", "n": "换行" } ] },
        { "key": "ctrl", "label": "控制字符", "type": "chars", "items": ctrl },
        { "key": "sym", "label": "符号", "type": "chars", "items": sym },
        { "key": "num", "label": "数字括号", "type": "chars", "items": num },
        { "key": "acts", "label": "动作", "type": "actions", "sep": true, "items": [] },
        { "key": "cfg", "label": "配置", "type": "config", "sep": true }
    ])
}

fn ctrl_note(letter: &str) -> &'static str {
    match letter {
        "A" => "行首",
        "B" => "左移",
        "C" => "打断",
        "D" => "退出",
        "E" => "行尾",
        "F" => "右移",
        "G" => "取消",
        "H" => "退格",
        "I" => "制表",
        "J" => "换行",
        "K" => "删到行尾",
        "L" => "清屏",
        "M" => "回车",
        "N" => "下移",
        "O" => "写行",
        "P" => "上移",
        "Q" => "恢复",
        "R" => "搜历史",
        "S" => "暂停",
        "T" => "换位",
        "U" => "删到行首",
        "V" => "引号入",
        "W" => "删词",
        "X" => "删光标后",
        "Y" => "贴回",
        "Z" => "挂起",
        _ => "",
    }
}

/// 一次性迁移：`quickpanel.json` 还没落盘时，把旧 `prompt_items` 表里的内容按 tab 归拢成 JSON 条目。
///
/// 为什么必须迁：代理页的「提示词与命令」管理视图已整页删除（用户 2026-11-25），
/// 浮窗改读 JSON——不迁的话，用户攒下来的提示词库在界面上就再也看不见了（数据没丢但等于丢了）。
/// 条目一律落成 `{t: 正文}`：旧面板对 `shell` tab 也是**注入文本**而非执行，迁完行为不变。
/// 表空（全新装机）才回内置默认。用户一旦在遥控器页保存过，文件存在，本函数不再参与。
fn migrate_from_prompt_items(st: &State) -> serde_json::Value {
    let rows = st.store.list_prompt_items().unwrap_or_default();
    if rows.is_empty() {
        return default_tabs();
    }
    let mut order: Vec<String> = Vec::new();
    let mut map: std::collections::BTreeMap<String, Vec<qp::QpItem>> = Default::default();
    for r in rows {
        if !map.contains_key(&r.tab) {
            order.push(r.tab.clone());
        }
        map.entry(r.tab.clone()).or_default().push(qp::QpItem {
            t: r.content,
            ..Default::default()
        });
    }
    let mut tabs: Vec<qp::QpTab> = order
        .iter()
        .map(|k| qp::QpTab {
            key: k.clone(),
            label: k.clone(),
            tab_type: "chars".into(),
            dynamic: None,
            sep: false,
            items: map.get(k).cloned().unwrap_or_default(),
        })
        .collect();
    // 旧表里本来就有重复（同一条提示词存了两遍），迁过来先按同一套规则去重。
    qp::dedupe(&mut tabs);
    serde_json::to_value(tabs).unwrap_or_else(|_| default_tabs())
}

/// 读 ~/.llm-switch/quickpanel.json；不存在给内置默认，解析失败回默认并带 error 供面板提示。
/// 热加载口径：面板每次获焦重读本命令（改完文件下次呼出即生效），不引入文件监听。
#[tauri::command]
pub fn quickpanel_config_get(st: TauriState<Arc<State>>) -> serde_json::Value {
    let path = st.config.data_dir().join("quickpanel.json");
    let raw = std::fs::read_to_string(&path).ok();
    let (tabs, err) = match raw {
        Some(s) => match serde_json::from_str::<serde_json::Value>(&s) {
            Ok(v) => (
                v.get("tabs").cloned().unwrap_or_else(default_tabs),
                serde_json::Value::Null,
            ),
            Err(e) => (default_tabs(), serde_json::Value::String(e.to_string())),
        },
        None => (migrate_from_prompt_items(&st), serde_json::Value::Null),
    };
    serde_json::json!({ "tabs": tabs, "path": path.display().to_string(), "error": err })
}

/// 写 ~/.llm-switch/quickpanel.json（遥控器页「快速面板」档编辑后落盘）。
/// 校验与原子替换在 `services::quickpanel::save`：坏配置一律拒写，原文件不动。
/// 返回归一后的整份配置，前端拿它回读（去空白、补默认字段都算数）。
#[tauri::command]
pub fn quickpanel_config_set(
    st: TauriState<Arc<State>>,
    tabs: Vec<QpTab>,
) -> Result<serde_json::Value, String> {
    let path = st.config.data_dir().join("quickpanel.json");
    let saved = qp::save(&path, qp::QpConfig { tabs })?;
    serde_json::to_value(saved.tabs).map_err(|e| e.to_string())
}

/// 跑一条宏（快速面板「动作引用」条目 OK 的那条路，计划 2026-11-25 ②片）。
///
/// 纪律：先确认宏还在（悬空引用**不收窗**，面板留着报错比把用户晦在原地好）
/// → 收窗并把焦点还给唤起前的 App → 交 `ButtonChannel::run_macro`，
/// 与遥控器按键走同一条执行路径（专用后台线程逐步跑、动作步回主线程投递）。
/// 逐步失败由 `run_macro` 自己记日志 + 推 toast（C8 口径），这里只回「已投递」。
#[tauri::command]
pub fn remote_macro_run(
    app: AppHandle,
    channel: TauriState<'_, Arc<ButtonChannel>>,
    id: String,
) -> Result<(), String> {
    let id = id.trim().to_string();
    if id.is_empty() {
        return Err("没说要跑哪条动作".to_string());
    }
    let profile = channel.bound_profile();
    if channel.macro_get(&profile, &id).is_none() {
        return Err("这条动作不存在或已被删除（重新选一条）".to_string());
    }
    let ch: Arc<ButtonChannel> = Arc::clone(&channel);
    tauri::async_runtime::spawn(async move {
        if !promptfloat::dismiss_to_target(&app).await {
            return;
        }
        ch.run_macro(&id, &profile);
    });
    Ok(())
}

/// 跑一个单发动作（快速面板 tab 里的 `{action:...}` 条目，计划 2026-11-25 ⑥片）。
///
/// 与宏同纪律：先收窗把焦点还给唤起前的 App（CGEvent 只打前台，面板不收起就是打自己），
/// 再回主线程发事件并等回执。非 macOS 没有注入实现，明报不支持，不假装成功。
#[tauri::command]
pub async fn remote_action_run(app: AppHandle, action: Action) -> Result<(), String> {
    if action.is_blank() {
        return Err("动作是空的（没内容就没法执行）".to_string());
    }
    if !promptfloat::dismiss_to_target(&app).await {
        return Err("没找到可归还焦点的目标窗口".to_string());
    }
    run_action_on_main(&app, action)
}

/// CGEvent 只能主线程发（同 `promptfloat::inject` 的纪律）：回主线程办并等回执。
#[cfg(target_os = "macos")]
fn run_action_on_main(app: &AppHandle, action: Action) -> Result<(), String> {
    promptfloat::on_main(app, std::time::Duration::from_secs(5), move || {
        crate::services::remote::action_exec::execute(&action)
    })
    .ok_or_else(|| "主线程未回执，动作已丢弃".to_string())?
}

#[cfg(not(target_os = "macos"))]
fn run_action_on_main(_app: &AppHandle, _action: Action) -> Result<(), String> {
    Err("平台不支持（当前仅 macOS）".to_string())
}

/// 执行用户自定义动作（sh -c）。信任边界 = 用户自己的 quickpanel.json（与 shell 配置同级）；
/// confirm 动作的两步确认由面板强制，后端只管执行 + 全量审计到 ~/.llm-switch/actions.log。
#[tauri::command]
pub fn quickpanel_run_action(
    st: TauriState<Arc<State>>,
    label: String,
    cmd: String,
) -> Result<serde_json::Value, String> {
    if cmd.trim().is_empty() {
        return Err("动作命令为空".into());
    }
    let out = std::process::Command::new("sh")
        .args(["-c", &cmd])
        .output()
        .map_err(|e| e.to_string())?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    if !err.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&err);
    }
    let exit = out.status.code().unwrap_or(-1);
    let dir = st.config.data_dir();
    let _ = std::fs::create_dir_all(&dir);
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("actions.log"))
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "{ts}\t{label}\texit={exit}\t{}", cmd.replace('\n', " "));
    }
    let tail = text
        .lines()
        .rev()
        .take(15)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Ok(serde_json::json!({ "exit": exit, "tail": tail }))
}
