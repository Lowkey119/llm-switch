/**
 * 共享表格外壳（UI 审查 P1-4）。
 *
 * 此前 13 个文件各写各的 <table>：行密度三种并存，且只有 Logs 有 sticky 表头，
 * 长列表一滚表头就丢。这里定一次口径——
 *
 *   - tableCls：左对齐 + 正文刻度 + 等宽数字（tabular-nums，数字列不再左右抖动）；
 *   - theadCls：sticky 表头 + 不透明底（半透明卡片上滚动内容不会透上来）+ 下边线；
 *   - 密度：px-4 py-2（矩阵类表格自定密度，不套这套）。
 *
 * 注意：sticky 只在「表格随页面滚动」或「外层自己 overflow-auto」时生效。
 * 外层若套 overflow-x-auto，纵向 sticky 会被这个滚动容器吃掉——窄表（≥900px
 * 窗口放得下）不要套 TableShell，直接让表头贴页面滚动区。
 */

export const tableCls = "w-full text-left text-xs tabular-nums";

/** sticky 表头：底色必须不透明（bg-slate-900 与卡片底同值），否则滚动内容透底。
 *  UI-15（用户 2026-11-16，附截图）：sticky 要挂在 th 上而不是 thead 上——
 *  WebKit 不支持 thead 上的 position:sticky，以前表头会被滚动的行穿透。
 *  下边线也画在 th 上：tr 的边框不跟着 sticky 的 th 悬停，会留在原位被滚走。 */
export const theadCls =
  "text-ink-4 [&_tr]:border-b [&_tr]:border-slate-800 [&_th]:sticky [&_th]:top-0 [&_th]:z-10 [&_th]:bg-slate-900 [&_th]:border-b [&_th]:border-slate-800";

/** 横向滚动兜底：只给列数多、可能超 900px 窗口的宽表用（会牺牲页面级 sticky）。 */
export function TableShell({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={`overflow-x-auto${className ? ` ${className}` : ""}`}>{children}</div>;
}
