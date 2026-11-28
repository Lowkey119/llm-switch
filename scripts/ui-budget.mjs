#!/usr/bin/env node
// UI 债务度量：每轮改造前后各跑一次，数字只许降不许升。
// 预算 = 本轮收口时的实测值，作用是「已归零的不许长回来、暂欠的别再加」。
// 用法：node scripts/ui-budget.mjs
import { readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SRC = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "ui", "src");

function walk(dir, re, out = []) {
  for (const e of readdirSync(dir)) {
    const p = path.join(dir, e);
    if (statSync(p).isDirectory()) walk(p, re, out);
    else if (re.test(e)) out.push(p);
  }
  return out;
}

// 类名类指标只看 ts/tsx：styles.css 里 --chart-* 的 hex 定义是合法出处，不是债。
const code = walk(SRC, /\.(tsx|ts)$/);
const all = code.map((f) => readFileSync(f, "utf8")).join("\n");
const count = (re) => (all.match(re) ?? []).length;

/**
 * Dialog.Content 上的 vh 高度（计划 §6.1 门禁）。
 *
 * zoom 挂在 documentElement 上，vh 会被倍率乘高：zoom 到 1.33 时 86vh 的弹窗
 * 实际比窗口还高，标题和关闭钮直接被顶出去。所以弹窗高度一律用百分比。
 * 这里按行扫：命中 Dialog.Content 之后往下看若干行，出现 `数字vh` 就算违规。
 */
function dialogVh() {
  let hits = 0;
  for (const f of code) {
    const lines = readFileSync(f, "utf8").split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (!lines[i].includes("Dialog.Content")) continue;
      for (const win of lines.slice(i, i + 12)) {
        if (/\d+vh/.test(win)) {
          hits++;
          console.log(`    ${path.relative(SRC, f)}:${i + 1}  Dialog.Content 用了 vh 高度`);
          break;
        }
      }
    }
  }
  return hits;
}

/**
 * U11：unicode 字形当功能图标（计划出局项之外的页面）。
 *
 * 功能图标的唯一出处是 `components/icons.tsx` 那份线性族（同一 strokeWidth 1.7、圆头、fill none）。
 * 拿 ▾▸●○★✓✕ 这类字符顶替图标有两个硬伤：读屏念不出（或念成「黑圆点」），而 WKWebView 不渲染
 * `title`，原先挂在旁边的解释本来就是空的。计数前先剥掉注释——注释里提到这些字形不算债。
 * 白名单：`icons.tsx`（族本身）、`ui.tsx`（StatusDot 的状态形状是刻意的双路冗余，旁边必有文字/sr-only）、
 * 浮窗那四件（计划出局项 2「浮窗与速率线全部不改」）。
 */
function glyphIcons() {
  const skip = [
    /components\/icons\.tsx$/,
    /components\/ui\.tsx$/,
    /components\/(PromptPanel|FloatPanel|SysFloat|FloatRateLine)\.tsx$/,
  ];
  let n = 0;
  for (const f of code) {
    const rel = path.relative(SRC, f).split(path.sep).join("/");
    if (skip.some((re) => re.test(rel))) continue;
    const src = readFileSync(f, "utf8").replace(/\/\*[\s\S]*?\*\//g, "").replace(/\/\/[^\n]*/g, "");
    n += (src.match(/[▾▸●○★✓✕]/g) ?? []).length;
  }
  return n;
}

/**
 * U1b：正文按角色分流后剩下的 `text-xs`（12 号）。
 *
 * 法条 §7.2：`text-xs` 只留给「说明 / 表格次要列」，正文、控件、按钮、输入框一律 `text-sm`(14)。
 * U1b 把在范围的站点逐行分完流，于是「按角色该升 14 却还写着 12」的站点归零，这一行钉死在 0。
 * 剩下的那些 `text-xs` 全是**按角色该留的**，不算站点：
 *   表格根（`<table` / `tableCls`）  等宽数据（模型 id、路径、URL、JSON、日志）  微标注（`uppercase`）
 *   图表与矩阵读数（`charts/` `model-matrix/` `SkillMatrix`）  等宽数值（`tabular-nums`）
 * 注释里提到 `text-xs` 不算站点。出局文件（浮窗四件 / PromptPanel / UpdateDialog / TaskTray /
 * GuidedTour / CommandPalette / Halogen 那几件）按计划出局项 2、3 保持原样，不计。
 * PromptRightPane / PromptTabRail / PromptFooter 是 PromptPanel 为压行数拆出去的右栏、
 * tab 轨与底栏，字号口径跟着面板走，一并出局。
 */
function bodyTier() {
  const skip = [
    /components\/(PromptPanel|PromptRightPane|PromptTabRail|PromptFooter|FloatPanel|SysFloat|FloatRateLine|UpdateDialog|TaskTray|GuidedTour|CommandPalette)\.tsx$/,
    /pages\/Halogen(Panel|Params)\.tsx$/,
    /pages\/halogen\//,
  ];
  const keepLine = /(?:<table|tableCls|font-mono|uppercase|tabular-nums)/;
  const keepFile = /components\/(charts\/|model-matrix\/|SkillMatrix\.tsx)/;
  let n = 0;
  for (const f of code) {
    const rel = path.relative(SRC, f).split(path.sep).join("/");
    if (skip.some((re) => re.test(rel))) continue;
    const lines = readFileSync(f, "utf8").split("\n");
    for (let i = 0; i < lines.length; i++) {
      const t = lines[i].trim();
      if (!t.includes("text-xs")) continue;
      if (t.startsWith("*") || t.startsWith("//") || t.startsWith("/*")) continue;
      if (keepLine.test(t) || keepFile.test(rel)) continue;
      n++;
      console.log(`    ${rel}:${i + 1}  正文档站点仍写着 text-xs（应升 text-sm）`);
    }
  }
  return n;
}

const ROWS = [
  ["outline-none（吃掉焦点环）", () => count(/outline-none/g), 0],
  ["text-[9px] 及以下", () => count(/text-\[[1-9]px\]/g), 0],
  // UI 审查 2026-11：文字色唯一合法来源是 text-ink-1..4（随主题翻转、对比度已校）；
  // text-slate-950 只许用在亮底反白徽章上，故只禁 1xx–8xx。
  ["text-slate-* 作文字（用 text-ink-1..4）", () => count(/text-slate-[1-8]\d\d/g), 0],
  // 字号刻度收定：正文 text-xs(12px) 起步，徽章用 text-2xs；不再写任意值。
  ["任意字号 text-[Npx]（用 text-2xs/xs/sm/md）", () => count(/text-\[\d+px\]/g), 0],
  ["硬编码图表 hex", () => count(/#22d3ee|#a78bfa|#34d399|#fbbf24|#f472b6|#94a3b8/g), 0],
  ["裸状态色 text-rose-*/red/emerald/amber", () => count(/text-(?:rose|red|emerald|amber)-\d/g), 0],
  // P3-4：disabled:opacity-40/50 会把正文压到 ~2.9:1（对比度必崩）。合法残留只有
  // Toggle 的填充压暗（压灯/底不压文字），文字降级一律 text-ink-4 + cursor-not-allowed。
  ["disabled:opacity 压文字（仅 Toggle 填充可留）", () => count(/disabled:opacity-\d+/g) - count(/disabled:opacity-\d+ data-\[state/g), 0],
  ["toast 实现点（flash/setToast/setMsg）", () => count(/const flash|setToast\(|setMsg\(/g), 1],
  ["轮询裸毫秒数（应引用 POLL.*）", () => count(/refetchInterval:\s*\d/g), 0],
  ["轮询未走 every()（暂停会漏）", () => count(/refetchInterval: (?!\(\) =>)/g), 0],
  ["Dialog.Content 上的 vh 高度", dialogVh, 0],
  // U9 连续重排：栅格列数由容器宽算出来（styles.css 的 .tiles / .kpi-grid / .form-grid /
  // .param-grid / .cards-2 / .split-*），不再按视口断点整排跳列。
  // 预算 5 = 本轮收口后剩下的，全部是**故意留的**，别当债清掉：
  //   pages/HalogenPanel.tsx ×2、pages/HalogenParams.tsx ×2 —— 计划出局项 3「原型没画的保持现状」；
  //   pages/agents/InstallPanel.tsx ×1 —— 用户 2026-11-17 拍板「安装引导一行两列」，宽窗不许自己变三列。
  ["栅格硬断点（用 styles.css 的连续重排栅格）", () => count(/(?:sm|md|lg|xl|2xl):grid-cols-/g), 5],
  ["unicode 字形当功能图标（用 icons.tsx 线性族）", glyphIcons, 0],
  ["正文/控件站点残留 text-xs（U1b 分流后按角色留档）", bodyTier, 0],
  [">550 行的 UI 文件", () => bigFiles(550).length, 0],
];

/** 超过 `max` 行的 UI 文件，按行数从多到少。 */
function bigFiles(max) {
  return walk(SRC, /\.tsx$/)
    .map((f) => [f, readFileSync(f, "utf8").split("\n").length])
    .filter((entry) => entry[1] > max)
    .sort((a, b) => b[1] - a[1]);
}

let bad = 0;
console.log("UI 债务度量（ui/src）\n");
for (const [name, fn, budget] of ROWS) {
  const now = fn();
  const over = now > budget;
  if (over) bad++;
  console.log(`  ${over ? "OVER" : "ok  "}  ${name.padEnd(34)} ${String(now).padStart(4)} / 预算 ${budget}`);
}
// >400 行只告警不失败：拆文件是持续收敛的事，硬失败会让每个小改动都得先还债。
const warn = bigFiles(400);
if (warn.length) {
  console.log(`\n提示：${warn.length} 个文件超过 400 行（计划 §6.1 目标），建议顺手再拆：`);
  for (const [f, n] of warn) console.log(`    ${String(n).padStart(4)} 行  ${path.relative(SRC, f)}`);
}

console.log(bad === 0 ? "\n全部在预算内" : `\n${bad} 项超预算`);
process.exit(bad === 0 ? 0 : 1);
