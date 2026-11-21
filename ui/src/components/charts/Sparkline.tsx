import { fmtNum } from "../Num";
import { t, ts } from "../../lib/i18n";
import { AXIS, ChartEmpty, niceMax } from "./shared";

/**
 * 极简 SVG 面积折线（零依赖，M4 图表选型：手写 SVG）。
 *
 * 22px 高的行内曲线塞不下刻度，读法走两头：当前值本来就打印在曲线左边（文字），
 * 曲线末点用圆点钉住「现在在哪」，峰值/当前/采样点数走 `.tip` 悬停读数 + aria。
 * 曲线被横向压扁（preserveAspectRatio=none），所以末点用「零长度线 + 圆头 +
 * non-scaling-stroke」画——直接画 circle 会被压成椭圆。
 */
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
  if (points.length < 2) return <ChartEmpty hint={t("chart.fewPoints")} />;
  const w = 600;
  const peak = Math.max(...points);
  const max = niceMax(peak);
  const latest = points[points.length - 1];
  const step = w / (points.length - 1);
  const y = (v: number) => height - 2 - (v / max) * (height - 4);
  const path = points
    .map((v, i) => `${i === 0 ? "M" : "L"}${(i * step).toFixed(1)},${y(v).toFixed(1)}`)
    .join(" ");
  const area = `${path} L${w},${height} L0,${height} Z`;
  const say = ts("chart.sparkRead", {
    peak: fmtNum(peak),
    now: fmtNum(latest),
    n: points.length,
    unit: unit ? ` ${unit}` : "",
  });
  return (
    <span className="tip tip-edge block w-full" data-tip={say}>
      <svg
        viewBox={`0 0 ${w} ${height}`}
        className="w-full"
        style={{ height }}
        preserveAspectRatio="none"
        role="img"
        aria-label={say}
      >
        {/* 零线：没有它，贴着底边画的折线看不出「离零有多远」 */}
        <line x1={0} y1={y(0)} x2={w} y2={y(0)} stroke={AXIS} strokeWidth={1} />
        <path d={area} fill="color-mix(in srgb, var(--chart-1) 14%, transparent)" />
        <path
          d={path}
          fill="none"
          stroke="var(--chart-1)"
          strokeWidth={1.5}
          vectorEffect="non-scaling-stroke"
        />
        <line
          x1={w - 8}
          y1={y(latest)}
          x2={w - 8}
          y2={y(latest)}
          stroke="var(--chart-1)"
          strokeWidth={4}
          strokeLinecap="round"
          vectorEffect="non-scaling-stroke"
        />
      </svg>
    </span>
  );
}
