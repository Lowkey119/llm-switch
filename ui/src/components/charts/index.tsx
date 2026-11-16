import { fmtNum } from "../Num";

/**
 * 图表共享层（UI-AUDIT P1-5 / 计划 §5）。
 *
 * 此前 sparkline / 柱图 / 环图三处各写一遍 scale、配色和空态，同一个 app 里
 * 「最大值」的取法都不一样，浅色主题下还得各修各的颜色。这里把四件事收口：
 * 配色（--chart-* token）、坐标上限取整、空态文案、可读性下限（不悬停也能读出量级）。
 */

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

/** 坐标轴用的中性色（网格、基线），同样跟主题翻转。 */
export const AXIS = "var(--chart-axis)";

/**
 * 把最大值抬到一个「好看的」刻度上（1→1、7→10、134→200、2.3M→3M）。
 * 贴着数据顶端的坐标轴读不出量级，取整后峰值那一格才有参照。
 */
export function niceMax(max: number): number {
  if (max <= 1) return 1;
  const exp = Math.floor(Math.log10(max));
  const base = Math.pow(10, exp);
  const unit = max / base;
  const step = unit <= 1 ? 1 : unit <= 2 ? 2 : unit <= 5 ? 5 : 10;
  return step * base;
}

/** 空态：三处此前分别是「—」「暂无」「空白」，统一成一句。 */
export function ChartEmpty({ hint = "暂无数据" }: { hint?: string }) {
  return <div className="py-8 text-center text-xs text-ink-4">{hint}</div>;
}

/** 图例的一行：色块 + 名称 + 值（可带百分比）。 */
export function LegendRow({
  color,
  label,
  value,
  share,
}: {
  color: string;
  label: string;
  value: string;
  share?: string;
}) {
  return (
    <li className="flex items-center gap-2">
      <span className="h-2 w-2 shrink-0 rounded-full" style={{ background: color }} />
      {/* UI-14：名字不再截断（c.. / codebu.. 读不出是谁）——宁可撑开图例列。 */}
      <span className="whitespace-nowrap text-ink-2" title={label}>{label}</span>
      <span className="ml-auto shrink-0 pl-2 font-mono text-ink-4">
        {value}
        {share && <span className="ml-1 text-ink-4">{share}</span>}
      </span>
    </li>
  );
}

/** 极简 SVG 面积折线（零依赖，M4 图表选型：手写 SVG）。 */
export function Sparkline({
  points,
  height,
  unit,
}: {
  points: number[];
  height: number;
  /** 峰值标注的单位，如 `tok/s`。 */
  unit?: string;
}) {
  if (points.length < 2) return <ChartEmpty hint="采样点不足" />;
  const w = 600;
  const max = niceMax(Math.max(...points));
  const peak = Math.max(...points);
  const step = w / (points.length - 1);
  const y = (v: number) => height - (v / max) * (height - 12) - 10;
  const path = points
    .map((v, i) => `${i === 0 ? "M" : "L"}${(i * step).toFixed(1)},${y(v).toFixed(1)}`)
    .join(" ");
  const area = `${path} L${w},${height} L0,${height} Z`;
  const peakX = Math.min(w - 4, points.indexOf(peak) * step);
  return (
    <svg
      viewBox={`0 0 ${w} ${height}`}
      className="w-full"
      style={{ height }}
      preserveAspectRatio="none"
      role="img"
      aria-label={`峰值 ${fmtNum(peak)}${unit ? ` ${unit}` : ""}，共 ${points.length} 个采样点`}
    >
      {/* 零线：没有它，贴着底边画的折线看不出「离零有多远」 */}
      <line x1={0} y1={y(0)} x2={w} y2={y(0)} stroke={AXIS} strokeWidth={1} />
      <path d={area} fill="color-mix(in srgb, var(--chart-1) 14%, transparent)" />
      <path d={path} fill="none" stroke="var(--chart-1)" strokeWidth={1.5} />
      <text
        x={peakX}
        y={Math.max(9, y(peak) - 3)}
        textAnchor={peakX > w - 60 ? "end" : "start"}
        className="fill-slate-500"
        style={{ fontSize: 9 }}
      >
        {fmtNum(peak)}
        {unit ? ` ${unit}` : ""}
      </text>
    </svg>
  );
}

export interface BarDatum {
  key: string;
  value: number;
  /** 悬停提示的完整一行。 */
  title?: string;
}

/** 柱状图（零依赖）：带基线、峰值标注和首尾刻度，不悬停也能读出量级。
 *  UI-14（用户 2026-11-16）：柱身按六色循环上色；给了 onSelect 时柱子可点，
 *  再点同一根取消选中，选中柱保持全亮、其余压暗，方便对着读数。 */
export function Bars({
  rows,
  height = 90,
  unit,
  onSelect,
  selected,
}: {
  rows: BarDatum[];
  height?: number;
  unit?: string;
  onSelect?: (key: string | null) => void;
  selected?: string | null;
}) {
  if (rows.length === 0) return <ChartEmpty />;
  const w = 600;
  const padBottom = 12;
  const bw = Math.min(28, (w / rows.length) * 0.7);
  const gap = (w - bw * rows.length) / (rows.length + 1);
  const max = niceMax(Math.max(...rows.map((r) => r.value), 1));
  const base = height - padBottom;
  const peak = Math.max(...rows.map((r) => r.value));
  return (
    <svg
      viewBox={`0 0 ${w} ${height}`}
      className="w-full"
      style={{ height }}
      role="img"
      aria-label={`共 ${rows.length} 个区间，峰值 ${fmtNum(peak)}${unit ? ` ${unit}` : ""}`}
    >
      <line x1={0} y1={base} x2={w} y2={base} stroke={AXIS} strokeWidth={1} />
      {rows.map((r, i) => {
        const bh = Math.max(2, (r.value / max) * (base - 14));
        const x = gap + i * (bw + gap);
        return (
          <rect
            key={r.key}
            x={x}
            y={base - bh}
            width={bw}
            height={bh}
            rx={2}
            fill={chartColor(i)}
            opacity={selected ? (selected === r.key ? 1 : 0.35) : r.value === peak ? 1 : 0.85}
            className={onSelect ? "cursor-pointer" : undefined}
            onClick={
              onSelect
                ? () => onSelect(selected === r.key ? null : r.key)
                : undefined
            }
          >
            <title>{r.title ?? `${r.key}: ${fmtNum(r.value)}`}</title>
          </rect>
        );
      })}
      {/* 峰值标在最高的那根柱顶上，x 轴只标首尾——中间日期标了也挤 */}
      <text
        x={gap + rows.findIndex((r) => r.value === peak) * (bw + gap) + bw / 2}
        y={Math.max(9, base - Math.max(2, (peak / max) * (base - 14)) - 3)}
        textAnchor="middle"
        className="fill-slate-500"
        style={{ fontSize: 9 }}
      >
        {fmtNum(peak)}
        {unit ? ` ${unit}` : ""}
      </text>
      <text x={gap} y={height - 2} className="fill-slate-500" style={{ fontSize: 9 }}>
        {rows[0].key}
      </text>
      <text x={w - gap} y={height - 2} textAnchor="end" className="fill-slate-500" style={{ fontSize: 9 }}>
        {rows[rows.length - 1].key}
      </text>
    </svg>
  );
}

/** 环图 + 图例（图例带百分比：只有绝对值时读不出「这块占多少」）。 */
export function Donut({ items }: { items: { label: string; value: number }[] }) {
  const total = items.reduce((a, i) => a + i.value, 0);
  if (!total) return <ChartEmpty />;
  const R = 40;
  const C = 2 * Math.PI * R;
  let acc = 0;
  return (
    <div className="flex items-center gap-4">
      <svg
        viewBox="0 0 100 100"
        className="h-28 w-28 shrink-0 -rotate-90"
        role="img"
        aria-label={`合计 ${fmtNum(total)}，共 ${items.length} 项`}
      >
        {items.map((it, i) => {
          const frac = it.value / total;
          const el = (
            <circle
              key={it.label}
              cx={50}
              cy={50}
              r={R}
              fill="none"
              stroke={chartColor(i)}
              strokeWidth={14}
              strokeDasharray={`${frac * C} ${C}`}
              strokeDashoffset={-acc * C}
            >
              <title>{`${it.label}: ${fmtNum(it.value)}（${(frac * 100).toFixed(1)}%）`}</title>
            </circle>
          );
          acc += frac;
          return el;
        })}
      </svg>
      <ul className="min-w-0 space-y-1 text-xs">
        {items.map((it, i) => (
          <LegendRow
            key={it.label}
            color={chartColor(i)}
            label={it.label}
            value={fmtNum(it.value)}
            share={`${((it.value / total) * 100).toFixed(0)}%`}
          />
        ))}
      </ul>
    </div>
  );
}
