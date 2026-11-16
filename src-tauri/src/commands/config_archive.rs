//! 预设配置归档的 IPC 出口：设置页一键把归档还原回数据目录，也一键把当前配置
//! 备份成归档（用户备份优先于随包内嵌那份）。

use crate::gateway::State;
use crate::services::config_archive;
use serde_json::Value;
use std::sync::Arc;
use tauri::State as TauriState;

/// 归档概览（大小 / 条目数 / 白名单 / 来源），设置页展示用。
#[tauri::command]
pub fn config_archive_info(st: TauriState<'_, Arc<State>>) -> Value {
    config_archive::info(&st.config.data_dir())
}

/// 一键备份：把当前数据目录里的白名单条目打成归档（还原时优先用它）。
#[tauri::command]
pub fn config_archive_backup(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    config_archive::backup(&st.config.data_dir())
}

/// 一键还原：解压归档到数据目录（默认 `~/.llm-switch`；自己的备份优先，没有才用
/// 随包内嵌那份），覆盖前先快照现行文件。
#[tauri::command]
pub fn config_archive_restore(st: TauriState<'_, Arc<State>>) -> Result<Value, String> {
    config_archive::restore(&st.config.data_dir())
}
