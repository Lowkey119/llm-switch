/**
 * 图表共享层（UI-AUDIT P1-5 / 计划 §5 / ui_01 回灌 U8）。
 *
 * 此前 sparkline / 柱图 / 环图三处各写一遍 scale、配色和空态，同一个 app 里
 * 「最大值」的取法都不一样，浅色主题下还得各修各的颜色。这里把四件事收口：
 * 配色（--chart-* token）、坐标上限取整、空态文案、可读性下限（不悬停也能读出量级）。
 *
 * U8 补的是「读法」（原型 README 第 7 条：图表只有形状没有读法）。三条约束定了做法：
 *   1. WKWebView 不渲染 SVG `<title>` 悬浮提示（顶栏十处 tooltip 实测全灭，出处见
 *      styles.css 的 `.tip`），所以悬停读数必须是真实 DOM：柱图用图下读数行，
 *      迷你曲线用 `.tip`；`<title>` 一律撤掉，留着是假读数。
 *   2. 读数不能只给鼠标：柱子可 Tab、可回车/空格，聚焦即读数（描边即焦点线索）。
 *   3. 不配色也得读得出：环图靠图例（名称 + 值 + 百分比），柱图非交互时给 sr-only
 *      数据表，交互时每根柱自带完整 aria-label（role="img" 会把可聚焦子节点挡在
 *      读屏外，所以交互态改用 role="group"）。
 *
 * 文件按图表类型拆开（shared / Sparkline / Bars / Donut），这里只做出口，
 * 页面一律 `from "../components/charts"`，不必知道里面分了几块。
 */

export * from "./shared";
export { Sparkline } from "./Sparkline";
export { Bars } from "./Bars";
export type { BarDatum } from "./Bars";
export { Donut } from "./Donut";
