//! 前台 App 监听（C5，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）。
//!
//! 走 `NSWorkspace.didActivateApplicationNotification`（objc2 绑定 + `RcBlock`，与 `speech.rs`
//! 同法）：系统切前台时同步回调，不轮询。文件顶 `allow(unsafe_code)` 与 applist.rs / axfocus.rs 同法。
//!
//! 观察者注册一次、活到进程结束（`mem::forget` 故意不释放）：观察者对象一丢，通知就静默停掉，
//! 与其在通道里再挂一个句柄，不如让它常驻。
#![allow(unsafe_code)]

use block2::RcBlock;
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::NSNotification;
use serde::Serialize;
use std::ptr::NonNull;

/// 前台 App 读数（页面显示与方案匹配都靠它）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FrontApp {
    pub bundle_id: String,
    pub name: String,
}

fn read(app: Option<&NSRunningApplication>) -> Option<FrontApp> {
    let app = app?;
    // 没有 bundle id 的进程（少数命令行壳）不参与方案匹配，直接当「无前台 App」。
    let bundle_id = app.bundleIdentifier()?.to_string();
    let name = app
        .localizedName()
        .map(|n| n.to_string())
        .unwrap_or_else(|| bundle_id.clone());
    Some(FrontApp { bundle_id, name })
}

/// 当前前台 App（未激活任何 App / 拿不到 bundle id 回 None）。
pub fn frontmost() -> Option<FrontApp> {
    read(
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .as_deref(),
    )
}

/// 注册前台 App 监听：`on_change(None)` = 前台没了可读（罕见），同样上报，调用方按「回默认」处理。
///
/// 回调在发通知的线程（主线程）同步执行，务必只做轻活：算一次匹配 + 推一条事件。
pub fn watch<F>(on_change: F)
where
    F: Fn(Option<FrontApp>) + Send + Sync + 'static,
{
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let block = RcBlock::new(move |_note: NonNull<NSNotification>| {
        // 通知只说「前台变了」，具体是谁再读一次 frontmost：比解 userInfo 少一层类型体操。
        on_change(frontmost());
    });
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidActivateApplicationNotification),
            None,
            // queue = None：同步在发通知的线程回调，不额外排队（Tauri 主线程本来就在跑 runloop）。
            None,
            &block,
        )
    };
    std::mem::forget(token);
}
