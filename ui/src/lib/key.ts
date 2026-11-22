// 快捷键的平台口径（ui_01 回灌 U2 顺手收口）。
//
// 为什么收成一处：键帽要按平台显示 ⌘ 还是 Ctrl，而 `pages/settings/PromptCard.tsx`
// 里已经有一份一模一样的平台判断。再抄一遍就是平行实现（AGENTS 铁律 3），
// 平台判断的唯一真相放在这里。

/** 当前是否 macOS。navigator.platform 已废弃但仍是 webview 里最稳的信号，留 UA 兜底。 */
export const isMac = /mac/i.test(navigator.platform || navigator.userAgent);

/**
 * 组合键的键帽写法：macOS 用 `⌘1`（系统惯例不加连接符），其余平台用 `Ctrl+1`。
 *
 * 只用于「展示给人看」，别拿它去拼 Tauri accelerator——注册全局快捷键要的是
 * `CommandOrControl+1` 那种规范串，两边混用会静默注册不上。
 */
export function combo(key: string): string {
  return isMac ? `⌘${key}` : `Ctrl+${key}`;
}
