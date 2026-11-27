import { compactNum } from "../Num";
import { t } from "../../lib/i18n";

/** 六色循环，全部走 token：硬编码深色 hex 在暖白纸上只有 2.2:1。 */
export const CHART_COLORS = [
  "var(--chart-1)",
  "var(--chart-2)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--chart-5)",
  "var(--chart-6)",
] as const;

export const chartColor = (i: number) => CHART_COLORS[i % CHART_COLORS.length];

/** 基线：柱子从哪儿量起是信息。双主题各自 ≥3:1（U8 前吃 slate-700，深底只有 1.66:1）。 */
export const AXIS = "var(--chart-axis)";

/** 网格线：装饰，帮眼睛对齐刻度。数值全在刻度文字与读数行上，不靠网格读。 */
export const GRID = "var(--chart-grid)";

/** 刻度文字：跟正文同一套 ink 档，随主题翻转，对比度有门禁管着（fill-slate-* 没人管）。 */
export const TICK = "var(--color-ink-4)";

/**
 * 把最大值抬到一个「好看的」刻度上（1→1、7→10、134→200、2.3M→3M）。
 * 贴着数据顶端的坐标轴读不出量级，取整后峰值那一格才有参照。
 */
export function niceMax(max: number): number {
  if (max <= 1) return 1;
  const exp = Math.floor(Math.log10(max));
  const base = Math.pow(10, exp);
  const unit = max / base;
  const step = unit <= 1 ? 1 : unit <= 2 ? 2 : unit <= 3 ? 3 : unit <= 5 ? 5 : 10;
  return step * base;
}

/** x 轴刻度对齐：区间两端贴边，中间居中（只有一根柱时也居中）。 */
export const xAnchor = (i: number, n: number) =>
  i === 0 && n > 1 ? "start" : i === n - 1 ? "end" : "middle";

/**
 * x 轴刻度取哪几条：均匀抽稀到 ≤cap 条，首尾必给。
 * 中间日期标了也挤，但区间两端必须读得出——不然「近 30 天」看不出是哪 30 天。
 */
export const tickIdx = (count: number, cap = 8) => {
  const n = Math.min(count, cap);
  return Array.from(new Set(Array.from({ length: n }, (_, k) => Math.round((k * (count - 1)) / Math.max(1, n - 1)))));
};

/** 刻度标签：千位起压缩（否则 5 位数把左侧留白撑爆），整千的「200.0k」写成「200k」。 */
export const tickLabel = (v: number) =>
  Math.abs(v) >= 1000
    ? String(compactNum(v)).replace(/\.0(?=[kMB]$)/, "")
    : String(Math.round(v));

/** 空态：三处此前分别是「—」「暂无」「空白」，统一成一句。 */
export function ChartEmpty({ hint }: { hint?: string }) {
  return <div className="py-8 text-center text-xs text-ink-4">{hint ?? t("chart.empty")}</div>;
}

/**
 * 图例的一行：色块 + 名称 + 值（可带百分比）。
 * onHover 给环图做联动；联动时压暗的是色块不是文字（文字压暗对比度必崩）。
 */
export function LegendRow({
  color,
  label,
  value,
  share,
  active,
  dimmed,
  onHover,
}: {
  color: string;
  label: string;
  value: string;
  share?: string;
  active?: boolean;
  dimmed?: boolean;
  onHover?: (label: string | null) => void;
}) {
  return (
    <li
      className={`flex items-center gap-2 rounded px-1${active ? " bg-[var(--sel-bg)]" : ""}`}
      onMouseEnter={onHover ? () => onHover(label) : undefined}
      onMouseLeave={onHover ? () => onHover(null) : undefined}
    >
      <span
        className="h-2 w-2 shrink-0 rounded-full"
        style={{ background: color, opacity: dimmed ? 0.35 : 1 }}
      />
      {/* UI-14：名字不再截断（c.. / codebu.. 读不出是谁）——宁可撑开图例列。 */}
      <span className="whitespace-nowrap text-ink-2" title={label}>
        {label}
      </span>
      <span className="ml-auto shrink-0 pl-2 font-mono text-ink-4">
        {value}
        {share && <span className="ml-1 text-ink-4">{share}</span>}
      </span>
    </li>
  );
}
