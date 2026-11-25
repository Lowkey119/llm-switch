//! Skills IPC（M6-A）：列表/扫描/重同步/开关/卸载/备份还原/分发方式。
//! 与 REST /admin/skills/* 同一服务层，两个门面共享状态与并发锁。

use crate::gateway::State;
use crate::services::skill::{self, SkillEnv};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State as TauriState;

fn env(st: &State) -> SkillEnv {
    SkillEnv::from_config(&st.config)
}

#[tauri::command]
pub fn get_skills(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let views = skill::list(&env(&st), &st.store)?;
    serde_json::to_value(views).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn scan_skills(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    skill::scan(&env(&st), &st.store)
}

#[tauri::command]
pub fn resync_skills(st: TauriState<'_, Arc<State>>) -> Result<Vec<Value>, String> {
    skill::resync(&env(&st), &st.store)
}

#[tauri::command]
pub fn set_skill_app_enabled(
    st: TauriState<'_, Arc<State>>,
    dir: String,
    app: String,
    enabled: bool,
) -> Result<Value, String> {
    let app = skill::SkillApp::parse(&app)?;
    let state = skill::set_enabled(&env(&st), &st.store, &dir, app, enabled)?;
    serde_json::to_value(state).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn uninstall_skill(st: TauriState<'_, Arc<State>>, dir: String) -> Result<Value, String> {
    let backup = skill::uninstall(&env(&st), &st.store, &dir)?;
    Ok(json!({ "backup": backup }))
}

#[tauri::command]
pub fn list_skill_backups(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let backups = skill::list_backups(&env(&st))?;
    serde_json::to_value(backups).map_err(|e| e.to_string())
}

/// BUG-6：备份页的「备份」按钮——对当前技能库做一次快照。
#[tauri::command]
pub fn backup_skills_now(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    let done = skill::backup_now(&env(&st), &st.store)?;
    Ok(json!({ "backed_up": done }))
}

#[tauri::command]
pub fn delete_skill_backup(st: TauriState<'_, Arc<State>>, id: String) -> Result<bool, String> {
    skill::delete_backup(&env(&st), &id)?;
    Ok(true)
}

#[tauri::command]
pub fn restore_skill_backup(st: TauriState<'_, Arc<State>>, id: String) -> Result<Value, String> {
    let view = skill::restore_backup(&env(&st), &st.store, &id)?;
    serde_json::to_value(view).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_skill_settings(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    Ok(json!({
        "sync_method": skill::get_sync_method(&st.store),
        "download_proxy": skill::get_download_proxy(&st.store),
    }))
}

#[tauri::command]
pub fn set_skill_sync_method(
    st: TauriState<'_, Arc<State>>,
    method: String,
) -> Result<String, String> {
    skill::set_sync_method(&st.store, &method)
}

// ---------- M6-B：安装 / 导入 / 未纳管 / 仓库 ----------

#[tauri::command]
pub async fn install_skill(
    st: TauriState<'_, Arc<State>>,
    owner: String,
    name: String,
    branch: Option<String>,
    path: Option<String>,
) -> Result<Value, String> {
    let view = skill::install_from_repo(
        &env(&st),
        &st.store,
        &owner,
        &name,
        branch.as_deref().unwrap_or("main"),
        path.as_deref(),
    )
    .await?;
    serde_json::to_value(view).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_skills_zip(st: TauriState<'_, Arc<State>>, path: String) -> Result<Value, String> {
    skill::import_zip(&env(&st), &st.store, std::path::Path::new(&path))
}

#[tauri::command]
pub fn scan_unmanaged_skills(st: TauriState<'_, Arc<State>>) -> Result<Vec<Value>, String> {
    skill::scan_unmanaged(&env(&st), &st.store)
}

#[tauri::command]
pub async fn claim_skill(
    st: TauriState<'_, Arc<State>>,
    dir: String,
    apps: Option<Vec<String>>,
) -> Result<Value, String> {
    let view = skill::claim(&env(&st), &st.store, &dir, &apps.unwrap_or_default())?;
    serde_json::to_value(view).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_skill_repos(st: TauriState<'_, Arc<State>>) -> Result<Vec<Value>, String> {
    skill::list_repos(&st.store)
}

#[tauri::command]
pub fn add_skill_repo(
    st: TauriState<'_, Arc<State>>,
    owner: String,
    name: String,
    branch: Option<String>,
) -> Result<bool, String> {
    skill::add_repo(
        &st.store,
        &owner,
        &name,
        branch.as_deref().unwrap_or("main"),
    )?;
    Ok(true)
}

#[tauri::command]
pub fn delete_skill_repo(st: TauriState<'_, Arc<State>>, id: i64) -> Result<bool, String> {
    skill::delete_repo(&st.store, id)
}

#[tauri::command]
pub fn set_skill_repo_enabled(
    st: TauriState<'_, Arc<State>>,
    id: i64,
    enabled: bool,
) -> Result<bool, String> {
    skill::set_repo_enabled(&st.store, id, enabled)
}

// ---------- M6-C：发现 / 搜索 / 更新 ----------

#[tauri::command]
pub async fn discover_skills(
    st: TauriState<'_, Arc<State>>,
    force: Option<bool>,
) -> Result<Value, String> {
    skill::discover(&st.store, force.unwrap_or(false)).await
}

#[tauri::command]
pub async fn search_skills(
    q: String,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Value, String> {
    skill::search_skills_sh(&q, limit.unwrap_or(20), offset.unwrap_or(0)).await
}

#[tauri::command]
pub async fn check_skill_updates(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    skill::check_updates(&env(&st), &st.store).await
}

#[tauri::command]
pub async fn update_skill(
    st: TauriState<'_, Arc<State>>,
    dir: Option<String>,
) -> Result<Value, String> {
    match dir.as_deref() {
        Some(d) => {
            let view = skill::update_skill(&env(&st), &st.store, d).await?;
            serde_json::to_value(view).map_err(|e| e.to_string())
        }
        None => skill::update_all(&env(&st), &st.store).await,
    }
}

#[tauri::command]
pub fn read_skill_doc(st: TauriState<'_, Arc<State>>, dir: String) -> Result<String, String> {
    skill::read_doc(&env(&st), &dir)
}
