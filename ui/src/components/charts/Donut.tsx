import { useState } from "react";
import { fmtNum } from "../Num";
import { t, ts } from "../../lib/i18n";
import { ChartEmpty, LegendRow, TICK, chartColor } from "./shared";

/**
 * 环图 + 图例（图例带百分比：只有绝对值时读不出「这块占多少」）。
 * U8：环心是读数位——默认合计，悬停哪块就换成那块的量与占比；图例同项高亮、
 * 其余压暗（压的是色块，文字不动）。图例本身就是文字替代，读屏与不配色都能读。
 */
export function Donut({ items, unit }: { items: { label: string; value: number }[]; unit?: string }) {
  const [hover, setHover] = useState<string | null>(null);
  const total = items.reduce((a, i) => a + i.value, 0);
  if (!total) return <ChartEmpty />;
  const R = 40;
  const C = 2 * Math.PI * R;
  const cur = items.find((i) => i.label === hover);
  const pct = (v: number) => `${((v / total) * 100).toFixed(0)}%`;
  let acc = 0;
  return (
    <div className="flex items-center gap-4">
      <svg
        viewBox="0 0 100 100"
        className="h-28 w-28 shrink-0"
        role="img"
        aria-label={ts("chart.donutAria", {
          total: fmtNum(total),
          unit: unit ? ` ${unit}` : "",
          n: items.length,
        })}
      >
        {/* 旋转交给 g，环心文字才不会跟着歪掉 */}
        <g transform="rotate(-90 50 50)">
          {items.map((it, i) => {
            const frac = it.value / total;
            const node = (
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
                opacity={cur && cur.label !== it.label ? 0.3 : 1}
                onMouseEnter={() => setHover(it.label)}
                onMouseLeave={() => setHover(null)}
              />
            );
            acc += frac;
            return node;
          })}
        </g>
        <text
          x={50}
          y={51}
          textAnchor="middle"
          fill="var(--color-ink-1)"
          style={{ fontSize: 15, fontWeight: 600 }}
        >
          {fmtNum(cur ? cur.value : total)}
        </text>
        <text x={50} y={63} textAnchor="middle" fill={TICK} style={{ fontSize: 9 }}>
          {cur ? pct(cur.value) : (unit ?? t("chart.total"))}
        </text>
      </svg>
      <ul className="min-w-0 space-y-1 text-xs">
        {items.map((it, i) => (
          <LegendRow
            key={it.label}
            color={chartColor(i)}
            label={it.label}
            value={fmtNum(it.value)}
            share={pct(it.value)}
            active={cur?.label === it.label}
            dimmed={!!cur && cur.label !== it.label}
            onHover={setHover}
          />
        ))}
      </ul>
    </div>
  );
}
