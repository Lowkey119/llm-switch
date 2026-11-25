// 对外链接的唯一出处与打开方式（铁律：URL 集中定义，不散落字面量）。
//
// 打开口径（用户 2026-11-19 拍板）：**全部外链统一走应用内浏览器**——Rust 侧
// `open_internal` 开一个内置 webview 窗口加载页面（同一 URL 复用同一窗口），
// 不再跳出去到系统浏览器；`open_url`（系统浏览器）只作内置窗口开不起来时的兜底，
// 浏览器里跑 UI（无 Tauri）再退到 `window.open`。三级兜底，一条都不许静默失败。
import { ipc } from "./ipc";

/** 讲解视频（B 站 · 中英字幕）：README 首段与升级对话框标题栏共用。 */
export const DEMO_VIDEO = "https://www.bilibili.com/video/BV1GVsA6bExE";

/** 打开 http/https 外链：应用内窗口 → 系统浏览器 → window.open。 */
export function openExternal(url: string): void {
  void ipc("open_internal", { url }).catch(() => {
    void ipc("open_url", { url }).catch(() => {
      window.open(url, "_blank", "noopener,noreferrer");
    });
  });
}
