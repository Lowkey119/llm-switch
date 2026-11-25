//! 后台任务队列 IPC（借鉴 unsloth studio 后台任务 + 完成通知）：前端轮询进度、显未读红点，
//! 变更经 `task://update` 事件推给前端。
//!
//! 定位是「队列外壳」：`spawn_task` 给任意长操作用（登记 Running → 跑 work 可回报进度 →
//! 收尾 emit），`list_tasks` / `task_unread` / `mark_tasks_seen` 供前端画进度条与红点。
//! 现有 Skills/接管/后端更新等逐个接入留后续片。

use crate::services::task_queue::{self, TaskStatus};
use tauri::{AppHandle, Emitter};

/// 进度回报句柄：work 闭包用它推进度，顺带 emit 给前端。
#[derive(Clone)]
pub struct Progress {
    app: AppHandle,
    id: String,
}

impl Progress {
    /// 推进度（0.0..=1.0）并推 `task://update`。
    pub fn set(&self, progress: f64, message: Option<&str>) {
        task_queue::global().update(&self.id, progress, message);
        emit_task(&self.app, &self.id);
    }
}

fn emit_task(app: &AppHandle, id: &str) {
    if let Some(t) = task_queue::global().list().into_iter().find(|t| t.id == id) {
        let _ = app.emit("task://update", &t);
    }
}

/// 在后台跑一个带进度的长操作：登记 Running → 跑 `work`（可经 [`Progress`] 回报进度）→
/// 收尾并 emit。返回任务 id。work 返回 `Ok(结果文案)` / `Err(错误文案)`。
pub fn spawn_task<F, Fut>(app: AppHandle, kind: &str, label: &str, work: F) -> String
where
    F: FnOnce(Progress) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
{
    let id = task_queue::global().start(kind, label);
    let app_task = app.clone();
    let id_task = id.clone();
    tokio::spawn(async move {
        let progress = Progress {
            app: app_task.clone(),
            id: id_task.clone(),
        };
        match work(progress).await {
            Ok(msg) => {
                task_queue::global().finish(&id_task, TaskStatus::Done, Some(&msg));
            }
            Err(e) => {
                task_queue::global().finish(&id_task, TaskStatus::Failed, Some(&e));
            }
        }
        emit_task(&app_task, &id_task);
    });
    id
}

#[tauri::command]
pub fn list_tasks() -> Vec<task_queue::Task> {
    task_queue::global().list()
}

#[tauri::command]
pub fn task_unread() -> usize {
    task_queue::global().unread()
}

#[tauri::command]
pub fn mark_tasks_seen() {
    task_queue::global().mark_seen();
}
