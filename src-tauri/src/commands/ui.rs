//! 界面语言偏好（UI-17，用户 2026-11-16）：顶栏「中/EN」按钮的存取口。
//!
//! 偏好落 settings 表 `ui.lang`（"zh" / "en" / 缺省=跟随系统）。写入即生效：
//! 更新 `gui::i18n` 的运行时覆盖并重建托盘菜单；主窗口文案由前端自己按
//! 同一偏好渲染。启动时 `gui` setup 读回这份偏好，重启后保持选择。

use crate::gateway::State;
use crate::gui::i18n::{self, Lang};
use std::sync::Arc;
use tauri::{AppHandle, State as TauriState};

const KEY: &str = "ui.lang";

/// 读当前语言偏好：`"zh" | "en" | "auto"`（auto = 跟随系统探测）。
#[tauri::command]
pub fn ui_lang_get(st: TauriState<'_, Arc<State>>) -> Result<String, String> {
    Ok(st
        .store
        .get_setting(KEY)
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "auto".into()))
}

/// 写语言偏好并即时生效（托盘菜单重建）。非法值报错，不落库。
#[tauri::command]
pub fn ui_lang_set(
    app: AppHandle,
    st: TauriState<'_, Arc<State>>,
    lang: String,
) -> Result<String, String> {
    let parsed = match lang.as_str() {
        "zh" => Some(Lang::Zh),
        "en" => Some(Lang::En),
        "auto" => None,
        other => return Err(format!("不认识的语言: {other}")),
    };
    if parsed.is_none() {
        st.store
            .set_setting(KEY, "auto")
            .map_err(|e| e.to_string())?;
    } else {
        st.store
            .set_setting(KEY, lang.as_str())
            .map_err(|e| e.to_string())?;
    }
    i18n::set_override(parsed);
    crate::gui::refresh_tray_now(&app, &st);
    // 返回生效后的实际语言，前端按钮文案直接跟着走。
    Ok(match i18n::lang() {
        Lang::Zh => "zh",
        Lang::En => "en",
    }
    .to_string())
}
