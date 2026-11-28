// 实物图 + 热点层（2026-11-21 从 Mapping 拆出）：主图与面板副本同一套逻辑——
// 灰显高亮、彩斑、悬停/选中/闪烁态全由调用方传入；主图只多连线（端点走 regHot）。
import { useCallback, useRef, useState } from "react";
import { t } from "../../lib/i18n";
import { KEYS, remoteAsset } from "./keys";

export function RemoteImage({
  profileId,
  edit,
  sel,
  hover,
  live,
  held,
  after,
  onSelect,
  onHover,
  regHot,
  onMeasure,
  className = "",
  imgClassName = "",
}: {
  /** 档案决定实物图与热点坐标（无图档案不会走到这里）。 */
  profileId?: string | null;
  edit: boolean;
  /** 高亮灰显的目标键（主图 = 画布选中；副本 = 正在映射的键）。 */
  sel: string | null;
  hover: string | null;
  live: string | null;
  held: string | null;
  after: string | null;
  onSelect: (id: string) => void;
  onHover: (id: string | null) => void;
  /** 热点元素登记（主图连线的端点；副本不传）。 */
  regHot?: (id: string, el: HTMLButtonElement | null) => void;
  /** 图框实测完成回调（主图据此在热点挂载后重画连线）。 */
  onMeasure?: () => void;
  className?: string;
  imgClassName?: string;
}) {
  const { src: imgSrc, hots, hotF, hotD } = remoteAsset(profileId)!;

  // 热点层定位：与 img 像素级重合，零布局测量。主图 absolute 居中钉位（left-1/2
  // -translate-x-1/2 + h-full + w-auto）：层用同一套定位 + aspect-ratio=图自然宽高比，
  // 盒与 img 恒重合；副本走常规流（根 shrink-to-fit 收到图宽），层 inset-0 即图框。
  // （此前量 img 偏移钉层，布局中途量到过期尺寸就整体漂移，用户 2026-11-24 抓。）
  const [ar, setAr] = useState(0);
  const ro = useRef<ResizeObserver | null>(null);
  // 上一次量到的图框尺寸（RO 去抖用，见 measure）。
  const sizeRef = useRef<{ w: number; h: number } | null>(null);
  const cbRef = useRef({ onMeasure });
  cbRef.current = { onMeasure };
  const imgRef = useCallback((el: HTMLImageElement | null) => {
    ro.current?.disconnect();
    if (!el) return;
    const measure = () => {
      if (!el.naturalWidth) return;
      // 只在图框真的变了才往下报（含 ar 与 onMeasure 两路）。
      // 无条件上报 = 父层 setState 恒变 = 挂 imgTick 的 effect 每次渲染重挂 RO，
      // 虚拟键盘这类浮层一改动布局就自激（用户 2026-11-26 实报：开键盘整窗卡死）。
      const next = el.getBoundingClientRect();
      const last = sizeRef.current;
      if (last && Math.abs(last.w - next.width) < 0.5 && Math.abs(last.h - next.height) < 0.5) return;
      sizeRef.current = { w: next.width, h: next.height };
      setAr(el.naturalWidth / el.naturalHeight);
      cbRef.current.onMeasure?.();
    };
    if (el.complete) measure();
    else el.addEventListener("load", measure, { once: true });
    ro.current = new ResizeObserver(measure);
    ro.current.observe(el);
  }, []);
  const abs = imgClassName.includes("absolute");
  return (
    <div className={`relative ${className}`}>
      {/* 灰显高亮（用户 2026-11-19 提议）：选中键时整图去色，只留选中键原色。
          实物图的按键背光本就是彩色荧光，「保原色」比描环更直观，也不新增颜色词汇。
          彩色窗口 = 同一张图按热点圆裁出的圆形补丁（外层圆角 overflow-hidden，
          内层图按热点占 img 框的百分比反向定位回去）。 */}
      <img
        ref={imgRef}
        src={imgSrc}
        alt={t("remote.imgAlt")}
        className={`block max-w-full rounded-xl transition-[filter] duration-300 ${edit && sel ? "grayscale" : ""} ${imgClassName}`}
      />
      {ar > 0 && (
        // 主图态：层与 img 同定位类 + 同 aspect-ratio ⇒ 盒恒重合；副本态：根收缩到图宽，inset-0。
        <div
          className={
            abs
              ? "pointer-events-none absolute left-1/2 top-0 h-full -translate-x-1/2"
              : "pointer-events-none absolute inset-0"
          }
          style={abs ? { aspectRatio: String(ar) } : undefined}
        >
          {edit &&
            sel &&
            hots[sel] &&
            (() => {
              const { l, t: tp } = hots[sel];
              const hh = hotD * hotF;
              return (
                <span
                  aria-hidden
                  className="absolute overflow-hidden rounded-full"
                  style={{ left: `${l}%`, top: `${tp}%`, width: `${hotD}%`, height: `${hh}%` }}
                >
                  <img
                    src={imgSrc}
                    alt=""
                    className="absolute block max-w-none"
                    style={{
                      width: `${10000 / hotD}%`,
                      height: `${10000 / hh}%`,
                      left: `${(-l * 100) / hotD}%`,
                      top: `${(-tp * 100) / hh}%`,
                    }}
                  />
                </span>
              );
            })()}
          {KEYS.map((k) => {
            const h = hots[k.id];
            // 档案键表里存在但实物图没有的键（如 1 代无 TV 键）不画假热点。
            if (!h) return null;
            return (
              <button
                key={k.id}
                type="button"
                ref={(el) => regHot?.(k.id, el)}
                aria-label={t(k.nameKey)}
                aria-pressed={sel === k.id}
                onClick={() => {
                  // 运行态不可点（审查 1.4：点击被 toggleKey 吞掉，affordance 说谎），
                  // 但悬停照常报热点（用户 2026-11-25：悬停实物也要出圈与连线）。
                  if (edit) onSelect(k.id);
                }}
                onMouseEnter={() => onHover(k.id)}
                onMouseLeave={() => onHover(null)}
                // 统一直径 hotD 正圆（用户 2026-11-20 拍板：一个尺寸通吃，含方向键位）。
                style={{ left: `${h.l}%`, top: `${h.t}%`, width: `${hotD}%`, aspectRatio: "1" }}
                // 高亮圈跟「悬停优先、选中兜底」走（与连线同一目标，用户 2026-11-20）：
                // 悬停卡片时实物上的圈同时亮起来，线才不会看着悬空；反向悬停实物同样
                // 报 hover（用户 2026-11-25），所以热点常开 pointer-events，
                // 不再用 CSS :hover 画可点假象（圈由 hover 状态统一画）。
                className={`absolute pointer-events-auto rounded-full border-2 transition-colors duration-200 ${
                  held === k.id
                    ? "border-[var(--key-hl)] bg-[var(--key-hl)]/50 ring-4 ring-[var(--key-hl)]/40"
                    : after === k.id
                      ? "border-[var(--key-hl)]/70 bg-[var(--key-hl)]/25 ring-2 ring-[var(--key-hl)]/20"
                      : live === k.id
                        ? "border-[var(--key-hl)] bg-[var(--key-hl)]/45 ring-4 ring-[var(--key-hl)]/35"
                        : (hover ?? sel) === k.id
                          ? "border-[var(--key-hl)] bg-[var(--key-hl)]/30 ring-2 ring-[var(--key-hl)]/40"
                          : "border-transparent"
                }`}
              />
            );
          })}
          {/* 语音键（用户 2026-11-20 拍板）：无映射卡，但悬停实物要有气泡说明用途——
              标注不可点、不造假映射。后端已接入（usage 0x3E，2026-11-21 真机）后，
              实体按下的高亮也归它：hover 圈是纯 CSS，按下圈必须读 held/live/after，
              与 KEYS 热点同一优先级（用户 2026-11-21 抓：按语音键实物图不亮）。
              position 必须走内联：.tip 是无层样式（position:relative），压过 @layer 里的
              Tailwind absolute，之前圈根本没钉在图上、气泡自然看不见（用户 2026-11-20 抓）。 */}
          <span
            className={`tip tip-below pointer-events-auto rounded-full border-2 transition-colors hover:border-[var(--key-hl)] hover:bg-[var(--key-hl)]/20 ${
              held === "mic"
                ? "border-[var(--key-hl)] bg-[var(--key-hl)]/50 ring-4 ring-[var(--key-hl)]/40"
                : after === "mic"
                  ? "border-[var(--key-hl)]/70 bg-[var(--key-hl)]/25 ring-2 ring-[var(--key-hl)]/20"
                  : live === "mic"
                    ? "border-[var(--key-hl)] bg-[var(--key-hl)]/45 ring-4 ring-[var(--key-hl)]/35"
                    : "border-transparent"
            }`}
            data-tip={t("remote.key.micHint")}
            style={{
              position: "absolute",
              left: `${hots.mic.l}%`,
              top: `${hots.mic.t}%`,
              width: `${hotD}%`,
              aspectRatio: "1",
            }}
          >
            <span className="sr-only">{t("remote.key.micHint")}</span>
          </span>
        </div>
      )}
    </div>
  );
}
