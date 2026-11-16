#!/usr/bin/env node
// 对比度门禁：UI 走查清单里对比度部分的自动化版。
// 读 styles.css 的 token，按 WCAG 2.1 相对亮度算比值；不达标 exit 1。
// 用法：node scripts/check-contrast.mjs [--json]
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const CSS = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "ui",
  "src",
  "styles.css",
);

/* ── 颜色数学 ─────────────────────────────────────────────────────── */
function parseColor(raw) {
  const s = raw.trim();
  let m = /^#([0-9a-f]{3,8})$/i.exec(s);
  if (m) {
    let h = m[1];
    if (h.length === 3) h = h.split("").map((c) => c + c).join("");
    if (h.length === 4) h = h.split("").map((c) => c + c).join("").slice(0, 6) + h[3] + h[3];
    const n = parseInt(h.slice(0, 6), 16);
    return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255, a: h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1 };
  }
  m = /^rgba?\(([^)]+)\)$/i.exec(s);
  if (m) {
    const p = m[1].split(/[,/\s]+/).filter(Boolean).map(Number);
    return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 };
  }
  throw new Error(`无法解析颜色: ${raw}`);
}

function lum({ r, g, b }) {
  const f = (v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

/** 把带 alpha 的前景叠到底色上。 */
function over(fg, bg) {
  if (fg.a >= 1) return fg;
  const mix = (a, b) => a * fg.a + b * (1 - fg.a);
  return { r: mix(fg.r, bg.r), g: mix(fg.g, bg.g), b: mix(fg.b, bg.b), a: 1 };
}

function ratio(fg, bg) {
  const [x, y] = [lum(fg), lum(bg)].sort((a, b) => b - a);
  return (x + 0.05) / (y + 0.05);
}

/* ── token 解析（支持 var() 链与 :root 兜底） ─────────────────────── */
function loadTokens(css) {
  const themes = { dark: {}, light: {} };
  const root = {};
  for (const [, sel, body] of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    const target = /data-theme="dark"/.test(sel)
      ? themes.dark
      : /data-theme="light"/.test(sel)
        ? themes.light
        : /^:root\s*$/.test(sel.trim())
          ? root
          : null;
    if (!target) continue;
    for (const [, k, v] of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
      target[k] = v.trim().replace(/\/\*.*?\*\//g, "").trim();
    }
  }
  // 允许传 "--name" 或 "var(--name)" 两种写法。
  const bare = (v) => {
    const m = /^var\((--[\w-]+)\)$/.exec(v.trim());
    return m ? m[1] : v.trim();
  };
  const resolve = (name, theme, seen = new Set()) => {
    name = bare(name);
    if (seen.has(name)) throw new Error(`token 循环引用: ${name}`);
    seen.add(name);
    const raw = themes[theme][name] ?? root[name];
    if (raw == null) throw new Error(`未定义 token: ${name}（${theme}）`);
    return /^var\(/.test(raw) ? resolve(raw, theme, seen) : raw;
  };
  return { resolve, themes };
}

/* ── 判定表（正文 ≥4.5，大字号/图形 ≥3） ────────────────────────── */
const TEXT = 4.5;
const GRAPHIC = 3;
const CHECKS = [
  // [说明, 前景, 背景, 门槛]
  ["正文 text-2", "var(--text-2)", "var(--color-slate-900)", TEXT],
  ["标题 text-1", "var(--text-1)", "var(--color-slate-900)", TEXT],
  ["次要 text-3", "var(--text-3)", "var(--color-slate-900)", TEXT],
  ["弱注释 text-4", "var(--text-4)", "var(--color-slate-900)", TEXT],
  ["次要文字 / 页面底", "var(--text-4)", "var(--color-slate-950)", TEXT],
  ["语义·正常(字)", "var(--sem-ok-text)", "var(--color-slate-900)", TEXT],
  ["语义·危险(字)", "var(--sem-danger-text)", "var(--color-slate-900)", TEXT],
  ["语义·进行中(字)", "var(--sem-busy-text)", "var(--color-slate-900)", TEXT],
  ["空闲数据流点", "var(--dot-idle)", "var(--color-slate-900)", GRAPHIC],
  ["语义·正常(灯)", "var(--sem-ok)", "var(--color-slate-900)", GRAPHIC],
  ["语义·进行中(灯)", "var(--sem-busy)", "var(--color-slate-900)", GRAPHIC],
  ["语义·危险(灯)", "var(--sem-danger)", "var(--color-slate-900)", GRAPHIC],
  ["焦点环", "var(--focus)", "var(--color-slate-900)", GRAPHIC],
  ["图表 1", "var(--chart-1)", "var(--color-slate-900)", GRAPHIC],
  ["图表 2", "var(--chart-2)", "var(--color-slate-900)", GRAPHIC],
  ["图表 3", "var(--chart-3)", "var(--color-slate-900)", GRAPHIC],
  ["图表 4", "var(--chart-4)", "var(--color-slate-900)", GRAPHIC],
  ["图表 5", "var(--chart-5)", "var(--color-slate-900)", GRAPHIC],
  ["图表 6", "var(--chart-6)", "var(--color-slate-900)", GRAPHIC],
];

// 注释里也写着 token 名（--sem-ok 等），先剥掉再解析，否则选择器会被注释污染。
const css = readFileSync(CSS, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const { resolve } = loadTokens(css);

/**
 * 压暗档位（UI-AUDIT-2 A-1）：`--dim-idle` 是整行压暗的唯一合法值。
 * 它是个 alpha 而不是颜色，所以不能进 CHECKS；这里单独把正文/标题按它叠到底色上，
 * 要求仍 ≥4.5:1——谁把 --dim-idle 调小，这里立刻红。
 */
function dimRows() {
  const out = [];
  for (const theme of ["dark", "light"]) {
    const alpha = Number(resolve("--dim-idle", theme));
    const bg = parseColor(resolve("--color-slate-900", theme));
    for (const [label, tok] of [
      ["压暗后正文 text-2", "--text-2"],
      ["压暗后标题 text-1", "--text-1"],
    ]) {
      const c = parseColor(resolve(tok, theme));
      const f = { ...c, a: alpha };
      const r = ratio(over(f, bg), bg);
      out.push({
        theme,
        label,
        fg: `${resolve(tok, theme)} @ ${alpha}`,
        bg: resolve("--color-slate-900", theme),
        ratio: +r.toFixed(2),
        min: TEXT,
        pass: r >= TEXT,
      });
    }
  }
  return out;
}
const rows = [];
for (const theme of ["dark", "light"]) {
  for (const [label, fg, bg, min] of CHECKS) {
    const f = over(parseColor(resolve(fg, theme)), parseColor(resolve(bg, theme)));
    const b = parseColor(resolve(bg, theme));
    const r = ratio(f, b);
    rows.push({ theme, label, fg: resolve(fg, theme), bg: resolve(bg, theme), ratio: +r.toFixed(2), min, pass: r >= min });
  }
}
rows.push(...dimRows());

if (process.argv.includes("--json")) {
  console.log(JSON.stringify(rows, null, 2));
} else {
  for (const theme of ["dark", "light"]) {
    console.log(`\n${theme === "dark" ? "深色" : "浅色"}主题`);
    for (const r of rows.filter((x) => x.theme === theme)) {
      console.log(`  ${r.pass ? "PASS" : "FAIL"}  ${r.label.padEnd(18)} ${String(r.ratio).padStart(6)}:1  (需 ${r.min})  ${r.fg} on ${r.bg}`);
    }
  }
}
const bad = rows.filter((r) => !r.pass);
console.log(`\n${bad.length === 0 ? "全部达标" : `${bad.length} 组不达标`}`);
process.exit(bad.length === 0 ? 0 : 1);
