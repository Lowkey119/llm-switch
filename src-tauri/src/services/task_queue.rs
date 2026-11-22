//! 后台任务队列（借鉴 unsloth studio `core/scheduled_tasks`）：把带进度的长操作收拢到一处，
//! 统一提供进度、完成通知与未读计数。
//!
//! 定位是「队列外壳」：Skills 安装/更新/备份还原、agent 接管、后端版本更新/回退等长操作，
//! 各自散落在 commands 里、点了没进度也分不清成功失败。这里先立一个通用注册表——
//! `start` 登记一条 Running、`update` 推进度、`finish` 收尾并累加未读；前端轮询 `list`
//! 画进度条、`unread` 显红点、`mark_seen` 清红点。命令层负责把变更 emit 给前端。
//!
//! 核心是纯内存结构，不依赖 Tauri，便于单测。

use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 任务状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Done,
    Failed,
}

/// 一条后台任务。
#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    /// 归类（如 "skill-install" / "takeover" / "backend-update"），前端可按类分组。
    pub kind: String,
    /// 人类可读标签。
    pub label: String,
    pub status: TaskStatus,
    /// 0.0..=1.0；不确定进度的任务恒 0。
    pub progress: f64,
    /// 进度/结果文案。
    pub message: String,
    pub started_at: u64,
    pub finished_at: Option<u64>,
}

/// 保留的历史任务上限：完成的任务只留最近这么多条，避免无限增长。
const MAX_HISTORY: usize = 50;

#[derive(Default)]
pub struct TaskQueue {
    tasks: RwLock<HashMap<String, Task>>,
    /// 插入顺序（新在末尾）；`list` 反序返回（新在前）。
    order: RwLock<Vec<String>>,
    /// 完成（Done/Failed）后累加、`mark_seen` 清零的未读计数。
    unread: AtomicUsize,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl TaskQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一条 Running 任务，返回其 id。
    pub fn start(&self, kind: &str, label: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let task = Task {
            id: id.clone(),
            kind: kind.to_string(),
            label: label.to_string(),
            status: TaskStatus::Running,
            progress: 0.0,
            message: String::new(),
            started_at: now_ms(),
            finished_at: None,
        };
        self.tasks
            .write()
            .expect("task queue lock poisoned")
            .insert(id.clone(), task);
        let mut order = self.order.write().expect("task order lock poisoned");
        order.push(id.clone());
        // 裁剪历史：只保留最近 MAX_HISTORY 条（按插入顺序，优先淘汰已完成的旧项）。
        if order.len() > MAX_HISTORY {
            let mut tasks = self.tasks.write().expect("task queue lock poisoned");
            let mut i = 0;
            while order.len() > MAX_HISTORY && i < order.len() {
                let done = tasks
                    .get(&order[i])
                    .is_some_and(|t| t.status != TaskStatus::Running);
                if done {
                    let old = order.remove(i);
                    tasks.remove(&old);
                } else {
                    i += 1;
                }
            }
        }
        id
    }

    /// 推进度（不改变状态）。
    pub fn update(&self, id: &str, progress: f64, message: Option<&str>) {
        if let Some(t) = self
            .tasks
            .write()
            .expect("task queue lock poisoned")
            .get_mut(id)
        {
            t.progress = progress.clamp(0.0, 1.0);
            if let Some(m) = message {
                t.message = m.to_string();
            }
        }
    }

    /// 收尾：置终态、记完成时间、累加未读。
    pub fn finish(&self, id: &str, status: TaskStatus, message: Option<&str>) {
        let mut finished = false;
        if let Some(t) = self
            .tasks
            .write()
            .expect("task queue lock poisoned")
            .get_mut(id)
        {
            if t.status == TaskStatus::Running {
                finished = true;
            }
            t.status = status;
            t.progress = if status == TaskStatus::Done {
                1.0
            } else {
                t.progress
            };
            if let Some(m) = message {
                t.message = m.to_string();
            }
            t.finished_at = Some(now_ms());
        }
        if finished {
            self.unread.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// 返回全部任务，新在前。
    pub fn list(&self) -> Vec<Task> {
        let order = self.order.read().expect("task order lock poisoned");
        let tasks = self.tasks.read().expect("task queue lock poisoned");
        order
            .iter()
            .rev()
            .filter_map(|id| tasks.get(id).cloned())
            .collect()
    }

    pub fn unread(&self) -> usize {
        self.unread.load(Ordering::Relaxed)
    }

    pub fn mark_seen(&self) {
        self.unread.store(0, Ordering::Relaxed);
    }
}

/// 全局单例：命令层无需把队列穿过 State 传递。
pub fn global() -> &'static Arc<TaskQueue> {
    static Q: OnceLock<Arc<TaskQueue>> = OnceLock::new();
    Q.get_or_init(|| Arc::new(TaskQueue::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_then_finish_marks_unread() {
        let q = TaskQueue::new();
        let id = q.start("skill-install", "安装 foo");
        assert_eq!(q.unread(), 0);
        let running = q.list();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].status, TaskStatus::Running);
        q.finish(&id, TaskStatus::Done, Some("完成"));
        assert_eq!(q.unread(), 1);
        let after = q.list();
        assert_eq!(after[0].status, TaskStatus::Done);
        assert_eq!(after[0].progress, 1.0);
    }

    #[test]
    fn mark_seen_clears_unread() {
        let q = TaskQueue::new();
        let id = q.start("takeover", "接管 claude");
        q.finish(&id, TaskStatus::Failed, Some("失败"));
        assert_eq!(q.unread(), 1);
        q.mark_seen();
        assert_eq!(q.unread(), 0);
    }

    #[test]
    fn double_finish_counts_once() {
        let q = TaskQueue::new();
        let id = q.start("backend-update", "更新后端");
        q.finish(&id, TaskStatus::Done, None);
        q.finish(&id, TaskStatus::Done, None); // 已终态，不再计未读
        assert_eq!(q.unread(), 1);
    }

    #[test]
    fn update_clamps_progress() {
        let q = TaskQueue::new();
        let id = q.start("k", "l");
        q.update(&id, 1.5, Some("x"));
        assert_eq!(q.list()[0].progress, 1.0);
        q.update(&id, -0.2, None);
        assert_eq!(q.list()[0].progress, 0.0);
    }

    #[test]
    fn list_is_newest_first() {
        let q = TaskQueue::new();
        let a = q.start("k", "first");
        let b = q.start("k", "second");
        let list = q.list();
        assert_eq!(list[0].id, b);
        assert_eq!(list[1].id, a);
    }
}
