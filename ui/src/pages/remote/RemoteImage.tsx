// 实物图 + 热点层（2026-11-21 从 Mapping 拆出）：主图与面板副本同一套逻辑——
// 灰显高亮、彩斑、悬停/选中/闪烁态全由调用方传入；主图只多连线（端点走 regHot）。
import { useCallback, useRef, useState } from "react";
import remoteImg from "../../assets/remote-hi.webp";
import { t } from "../../lib/i18n";
import { HOT_D, HOT_F, HOTS, KEYS } from "./keys";

export function RemoteImage({
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
  // 热点层定位：量实物图 <img> 的实测渲染框（offsetWidth/Height 是布局 px，缩放免疫）。
  // 之前按「容器与图同框」的假设用容器百分比摆热点，实拍全偏心（用户 2026-11-20）——
  // 热点层直接钉在 img 框上，% 坐标一一映射图，容器怎么怪都不影响。
  const [size, setSize] = useState({ w: 0, h: 0 });
  const ro = useRef<ResizeObserver | null>(null);
  const cbRef = useRef({ onMeasure });
  cbRef.current = { onMeasure };
  const imgRef = useCallback((el: HTMLImageElement | null) => {
    ro.current?.disconnect();
    if (!el) return;
    const measure = () => {
      setSize({ w: el.offsetWidth, h: el.offsetHeight });
      cbRef.current.onMeasure?.();
    };
    measure();
    ro.current = new ResizeObserver(measure);
    ro.current.observe(el);
  }, []);
  return (
    <div className={`relative ${className}`}>
      {/* 灰显高亮（用户 2026-11-19 提议）：选中键时整图去色，只留选中键原色。
          实物图的按键背光本就是彩色荧光，「保原色」比描环更直观，也不新增颜色词汇。
          彩色窗口 = 同一张图按热点圆裁出的圆形补丁（外层圆角 overflow-hidden，
          内层图按热点占 img 框的百分比反向定位回去）。 */}
      <img
        ref={imgRef}
        src={remoteImg}
        alt={t("remote.imgAlt")}
        className={`block max-w-full rounded-xl transition-[filter] duration-300 ${edit && sel ? "grayscale" : ""} ${imgClassName}`}
      />
      {size.w > 0 && (
        <div className="pointer-events-none absolute left-0 top-0" style={{ width: size.w, height: size.h }}>
          {edit &&
            sel &&
            HOTS[sel] &&
            (() => {
              const { l, t: tp } = HOTS[sel];
              const hh = HOT_D * HOT_F;
              return (
                <span
                  aria-hidden
                  className="absolute overflow-hidden rounded-full"
                  style={{ left: `${l}%`, top: `${tp}%`, width: `${HOT_D}%`, height: `${hh}%` }}
                >
                  <img
                    src={remoteImg}
                    alt=""
                    className="absolute block max-w-none"
                    style={{
                      width: `${10000 / HOT_D}%`,
                      height: `${10000 / hh}%`,
                      left: `${(-l * 100) / HOT_D}%`,
                      top: `${(-tp * 100) / hh}%`,
                    }}
                  />
                </span>
              );
            })()}
          {KEYS.map((k) => {
            const h = HOTS[k.id];
            return (
              <button
                key={k.id}
                type="button"
                ref={(el) => regHot?.(k.id, el)}
                aria-label={t(k.nameKey)}
                aria-pressed={sel === k.id}
                onClick={() => onSelect(k.id)}
                onMouseEnter={() => onHover(k.id)}
                onMouseLeave={() => onHover(null)}
                // 统一直径 HOT_D 正圆（用户 2026-11-20 拍板：一个尺寸通吃，含方向键位）。
                style={{ left: `${h.l}%`, top: `${h.t}%`, width: `${HOT_D}%`, aspectRatio: "1" }}
                // 运行态热点不可点也不给 hover 反馈（审查 1.4）：点击被 toggleKey 吞掉，
                // affordance 说谎；mic 标注气泡不受影响。设置态才恢复可点。
                // 高亮圈跟「悬停优先、选中兜底」走（与连线同一目标，用户 2026-11-20）：
                // 悬停卡片时实物上的圈同时亮起来，线才不会看着悬空。
                className={`absolute rounded-full border-2 transition-colors duration-200 ${
                  edit ? "pointer-events-auto" : "pointer-events-none"
                } ${
                  held === k.id
                    ? "border-[var(--key-hl)] bg-[var(--key-hl)]/50 ring-4 ring-[var(--key-hl)]/40"
                    : after === k.id
                      ? "border-[var(--key-hl)]/70 bg-[var(--key-hl)]/25 ring-2 ring-[var(--key-hl)]/20"
                      : live === k.id
                        ? "border-[var(--key-hl)] bg-[var(--key-hl)]/45 ring-4 ring-[var(--key-hl)]/35"
                        : (hover ?? sel) === k.id
                          ? "border-[var(--key-hl)] bg-[var(--key-hl)]/30 ring-2 ring-[var(--key-hl)]/40"
                          : "border-transparent hover:border-[var(--key-hl)] hover:bg-[var(--key-hl)]/20"
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
            className={`tip tip-up pointer-events-auto rounded-full border-2 transition-colors hover:border-[var(--key-hl)] hover:bg-[var(--key-hl)]/20 ${
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
              left: `${HOTS.mic.l}%`,
              top: `${HOTS.mic.t}%`,
              width: `${HOT_D}%`,
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
