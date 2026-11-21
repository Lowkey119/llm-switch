// 语音卡的展示件：实时电平波形 + 上次会话指标行（从 VoicePanel 拆出，行数预算）。
import { t } from "../../lib/i18n";
import { AXIS } from "../../components/charts/shared";

/**
 * 实时电平波形：包络绕中线上下镜像，画成经典波形图样子。
 * 显示增益 4×：设备麦原始电平低（说话约 0.1），不放大看着像直线
 * （用户 2026-11-21：曲线幅度调高——只调画法，不动增益 dB）；
 * 右侧百分比数字仍是真实电平，不撒谎。刻度固定 0…1 满幅，不像速率图
 * 那样按峰值自动缩放——自动缩放会让整段静音也铺满高度，看着像在收声。
 */
export function LevelMeter({ levels }: { levels: number[] }) {
  if (levels.length < 2) {
    return <div className="text-sm text-ink-4">{"—"}</div>;
  }
  const w = 600;
  const h = 44;
  const step = w / (levels.length - 1);
  const GAIN = 4;
  const mid = h / 2;
  const y = (v: number) => mid - Math.min(1, Math.max(0, v * GAIN)) * (mid - 1);
  const top = levels
    .map((v, i) => `${i === 0 ? "M" : "L"}${(i * step).toFixed(1)},${y(v).toFixed(1)}`)
    .join(" ");
  const n = levels.length - 1;
  const bottom = levels
    .map((v, i) => `L${((n - i) * step).toFixed(1)},${(h - y(v)).toFixed(1)}`)
    .join(" ");
  return (
    <svg
      viewBox={`0 0 ${w} ${h}`}
      className="w-full"
      style={{ height: h }}
      preserveAspectRatio="none"
      role="img"
      aria-label={t("remote.voice.levelAria")}
    >
      <line x1={0} y1={mid} x2={w} y2={mid} stroke={AXIS} strokeWidth={1} />
      <path d={`${top} ${bottom} Z`} fill="color-mix(in srgb, var(--chart-1) 22%, transparent)" />
      <path d={top} fill="none" stroke="var(--chart-1)" strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
      <path d={bottom} fill="none" stroke="var(--chart-1)" strokeWidth={1} opacity={0.7} vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

export function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline gap-1.5">
      <dt className="text-ink-4">{label}</dt>
      <dd className="tabular-nums text-ink-1">{value}</dd>
    </div>
  );
}
