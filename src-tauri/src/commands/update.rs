//! 应用更新 IPC（fix:update-check-and-install）。
//!
//! 三个命令，职责分明：
//! - `update_check`：检测（GitHub 主、Gitee 备）。**自动检查失败要静默**，
//!   所以失败信息放在返回值的 `error` 字段里由前端决定说不说，而不是 Err 糊脸。
//! - `update_download`：走通用后台任务队列（`spawn_task`）下载 + SHA256 校验，
//!   前端复用任务托盘与就地进度条，不另造一套进度通道。
//! - `update_install`：校验通过后拉起系统安装器（A 档最后一步）。

use crate::commands::task::spawn_task;
use crate::services::update;
use tauri::AppHandle;

/// 检测新版本。返回 `Latest`（含 `error` 字段，见服务层口径 2）。
#[tauri::command]
pub async fn update_check() -> Result<serde_json::Value, String> {
    let latest = update::check().await?;
    serde_json::to_value(latest).map_err(|e| e.to_string())
}

/// 下载并校验安装包，返回任务 id（进度走 `task://update`，与装后端 CLI 同一条通道）。
/// 校验不过任务直接失败，**不会**留下可被拉起的路径。
#[tauri::command]
pub fn update_download(
    app: AppHandle,
    url: String,
    sha256: Option<String>,
    name: String,
) -> Result<String, String> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("安装包文件名不合法，拒绝下载".into());
    }
    let label = format!("下载更新包 {name}");
    let id = spawn_task(app, "update-download", &label, move |p| {
        let url = url.clone();
        let name = name.clone();
        async move {
            let path = update::download_and_verify(&url, sha256.as_deref(), &name, |prog, msg| {
                p.set(prog, Some(msg));
            })
            .await?;
            Ok(path.to_string_lossy().to_string())
        }
    });
    Ok(id)
}

/// 拉起系统安装器。macOS 只是打开 dmg，**仍需用户把 App 拖进 Applications**，
/// 返回的文案会写清楚，不许写得像已经装好了。
#[tauri::command]
pub fn update_install(path: String) -> Result<String, String> {
    update::launch_installer(std::path::Path::new(&path))
}
