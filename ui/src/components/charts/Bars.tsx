import { useEffect, useRef, useState } from "react";
import { fmtNum } from "../Num";
import { ts } from "../../lib/i18n";
import { AXIS, ChartEmpty, GRID, TICK, chartColor, niceMax, tickIdx, tickLabel, xAnchor } from "./shared";

export interface BarDatum {
  key: string;
  value: number;
  /** 读数行的补充说明，如 `入 1,204 · 12 请求`，跟值一起读出来。 */
  sub?: string;
}

/**
 * 柱状图（零依赖）：y 轴四等分刻度 + 网格、基线、x 轴抽稀刻度，不悬停也能读出量级。
 * UI-14（用户 2026-11-16）：柱身按六色循环上色；给了 onSelect 时柱子可点，
 * 再点同一根取消选中，选中柱保持全亮、其余压暗，方便对着读数。
 * U8：读数行收在这里——悬停 / 聚焦 / 选中三种来源共用一行，页面不必再自己写一句。
 */
export function Bars({
  rows,
  height = 132,
  unit,
  hint,
  onSelect,
  selected,
}: {
  rows: BarDatum[];
  height?: number;
  unit?: string;
  /** 没有读数时（未悬停未选中）读数行的提示语。 */
  hint?: string;
  onSelect?: (key: string | null) => void;
  selected?: string | null;
}) {
  const [hover, setHover] = useState<string | null>(null);
  // viewBox 宽取实测宽（ResizeObserver）：以前定死 480，卡片比这宽时 SVG 等比缩放
  // 居中留白，图不占满卡片、柱子挤在一起（用户实报）；实测宽下像素 1:1，文字不糊。
  const box = useRef<HTMLDivElement>(null);
  const [measured, setMeasured] = useState(480);
  const hasRows = rows.length > 0;
  // 依赖 hasRows：空数据早退时 box 根本没渲染，挂载那次挂不上观察器，
  // 数据到达后宽度就永远停在默认 480（用量页柱图只占左半的根因）。
  useEffect(() => {
    const n = box.current;
    if (!hasRows || !n) return;
    const ro = new ResizeObserver(() => setMeasured(Math.max(240, n.clientWidth)));
    ro.observe(n);
    return () => ro.disconnect();
  }, [hasRows]);
  if (rows.length === 0) return <ChartEmpty />;
  const interactive = !!onSelect;
  const W = measured;
  const padL = 34;
  const padR = 4;
  const padT = 10;
  const padB = 14;
  const plotH = Math.max(24, height - padT - padB);
  const iw = (W - padL - padR) / rows.length;
  const bw = Math.min(28, iw * 0.64);
  const vals = rows.map((r) => r.value);
  const peak = Math.max(...vals);
  const max = niceMax(Math.max(peak, 1));
  const read = (r: BarDatum) =>
    `${r.key} · ${fmtNum(r.value)}${unit ? ` ${unit}` : ""}${r.sub ? ` · ${r.sub}` : ""}`;
  const cur = rows.find((r) => r.key === (hover ?? selected));
  const spot = !!(hover || selected);
  const ticks = tickIdx(rows.length);
  const pick = (key: string) => onSelect?.(selected === key ? null : key);
  return (
    <div ref={box}>
      <svg
        viewBox={`0 0 ${W} ${height}`}
        className="w-full"
        style={{ height }}
        role={interactive ? "group" : "img"}
        aria-label={ts("chart.barAria", {
          n: rows.length,
          peak: fmtNum(peak),
          unit: unit ? ` ${unit}` : "",
        })}
      >
        <g aria-hidden="true">
          {[0, 1, 2, 3, 4].map((g) => {
            const gy = padT + (plotH * g) / 4;
            return (
              <g key={g}>
                <line
                  x1={padL}
                  y1={gy}
                  x2={W - padR}
                  y2={gy}
                  stroke={g === 4 ? AXIS : GRID}
                  strokeWidth={1}
                  strokeDasharray={g === 4 ? undefined : "2 4"}
                />
                <text x={padL - 6} y={gy + 3} textAnchor="end" fill={TICK} style={{ fontSize: 10 }}>
                  {tickLabel(max - (max / 4) * g)}
                </text>
              </g>
            );
          })}
          {ticks.map((i) => (
            <text
              key={i}
              x={padL + i * iw + iw / 2}
              y={height - 2}
              textAnchor={xAnchor(i, rows.length)}
              fill={TICK}
              style={{ fontSize: 10 }}
            >
              {rows[i].key}
            </text>
          ))}
        </g>
        {rows.map((r, i) => {
          const bh = Math.max(2, (r.value / max) * plotH);
          const on = selected === r.key;
          const hot = hover === r.key;
          return (
            <rect
              key={r.key}
              x={padL + i * iw + (iw - bw) / 2}
              y={padT + plotH - bh}
              width={bw}
              height={bh}
              rx={2}
              fill={chartColor(i)}
              opacity={spot ? (on || hot ? 1 : 0.35) : 0.85}
              stroke={hot ? "var(--focus)" : on ? "var(--sel-ring)" : undefined}
              strokeWidth={hot || on ? 1.5 : undefined}
              className={interactive ? "cursor-pointer" : undefined}
              tabIndex={interactive ? 0 : undefined}
              role={interactive ? "button" : undefined}
              aria-pressed={interactive ? on : undefined}
              aria-label={interactive ? read(r) : undefined}
              onMouseEnter={() => setHover(r.key)}
              onMouseLeave={() => setHover(null)}
              onFocus={() => setHover(r.key)}
              onBlur={() => setHover(null)}
              onClick={interactive ? () => pick(r.key) : undefined}
              onKeyDown={
                interactive
                  ? (e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        pick(r.key);
                      }
                    }
                  : undefined
              }
            />
          );
        })}
      </svg>
      <div className="mt-1 min-h-[1.25rem] text-xs text-ink-3">{cur ? read(cur) : (hint ?? "")}</div>
      {/* 非交互时柱子不可聚焦，读屏改吃这张隐藏表；交互时每根柱自带完整 label，不再重复一遍。 */}
      {!interactive && (
        <ul className="sr-only">
          {rows.map((r) => (
            <li key={r.key}>{read(r)}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
