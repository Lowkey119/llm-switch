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
  ["disabled:opacity 压文字（仅 Toggle 填充可留）", () => count(/disabled:opacity-\d+/g) - count(/disabled:opacity-\d+ data-\[state/g), 1],
  ["toast 实现点（flash/setToast/setMsg）", () => count(/const flash|setToast\(|setMsg\(/g), 1],
  ["轮询裸毫秒数（应引用 POLL.*）", () => count(/refetchInterval:\s*\d/g), 0],
  ["轮询未走 every()（暂停会漏）", () => count(/refetchInterval: (?!\(\) =>)/g), 0],
  ["Dialog.Content 上的 vh 高度", dialogVh, 0],
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
