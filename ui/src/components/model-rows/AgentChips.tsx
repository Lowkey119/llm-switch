// 代理灯牌带（用户 2026-11-23）：后端页每个模型行的第二行整行留给全部代理，
// 按代理页顺序排列（claude / codex / codebuddy / pi / opencode / deveco / copilot / gemini）。
//
// 灯牌语义：`代理名/槽数`——该模型在该代理占住的槽位数（清单制在清单即记 1）；
// 绿显 = 至少占一个位（仍可点击调整），灰显 = 一个没占（仍可点击分配）。
// 悬停 → 智能浮层列出占到的槽位名；点击 → 复用矩阵同源 chipClick（档位端槽位浮层 /
// 清单端就地动作）；灯牌带放不下时最后一个显示「…/更多」，点击跳代理页分配矩阵。
import { useLayoutEffect, useRef, useState, type MouseEvent } from "react";
import type { MatrixCell, MatrixClient } from "../../lib/ipc";
import { PointPopover } from "../Popover";
import { cellSlots, slotLabel } from "./alloc";
import { t, ts } from "../../lib/i18n";
import { cx } from "../../lib/cx";
import { gotoMatrix } from "../../lib/nav";

/** 「…/更多」占位宽度（估算：放不下时给它预留的位）。 */
const MORE_W = 76;

/** 该模型在这端占了几格：占槽数；清单制在清单记 1；gemini 默认位记 1。 */
function heldCount(cell: MatrixCell | undefined): number {
  if (!cell) return 0;
  const n = cellSlots(cell).length;
  if (n > 0) return n;
  return cell.in_list || cell.is_default ? 1 : 0;
}

export function AgentChips({
  model,
  cells,
  clients,
  chipClick,
}: {
  model: string;
  /** app → 该模型在这端的格子。 */
  cells: Record<string, MatrixCell>;
  /** 全部代理列（已按代理页顺序排好）。 */
  clients: MatrixClient[];
  /** 矩阵同源点击（ModelRows.chipClick）。 */
  chipClick: (app: string, model: string, cell: MatrixCell | undefined, e: MouseEvent) => void;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  // null = 正在测量（全量渲染）；数字 = 可见灯牌数，溢出时末尾挂「…/更多」。
  const [visible, setVisible] = useState<number | null>(null);
  const [hover, setHover] = useState<{ app: string; x: number; y: number } | null>(null);

  useLayoutEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const measure = () => {
      const chips = Array.from(el.querySelectorAll<HTMLElement>("[data-chip]"));
      if (!chips.length) return;
      const W = el.clientWidth;
      const fits = (k: number) => {
        const last = chips[k - 1];
        return last.offsetLeft + last.offsetWidth <= W;
      };
      if (fits(chips.length)) {
        setVisible(chips.length);
        return;
      }
      let k = chips.length - 1;
      while (k > 0 && chips[k - 1].offsetLeft + chips[k - 1].offsetWidth + MORE_W > W) k--;
      setVisible(Math.max(k, 1));
    };
    setVisible(null);
    measure();
    const ro = new ResizeObserver(() => setVisible(null));
    ro.observe(el);
    return () => ro.disconnect();
  }, [clients.length, model]);

  // 测量帧（visible=null）全量渲染，useLayoutEffect 里量完再收——paint 前完成，不闪。
  const shown = visible == null ? clients : clients.slice(0, visible);
  const overflow = visible != null && visible < clients.length;

  const hoverClient = hover ? clients.find((c) => c.app === hover.app) : undefined;
  const hoverCell = hover ? cells[hover.app] : undefined;
  const hoverSlots = hoverCell ? cellSlots(hoverCell) : [];

  return (
    <div
      ref={wrapRef}
      className="flex min-w-0 flex-nowrap items-center gap-1.5 overflow-hidden"
      onMouseLeave={() => setHover(null)}
    >
      {shown.map((c) => {
        const cell = cells[c.app];
        const n = heldCount(cell);
        return (
          <button
            key={c.app}
            data-chip
            type="button"
            title={n > 0 ? ts("ap.litTip", { app: c.label, slot: "" }) : ts("ap.dimTip", { app: c.label })}
            className={cx(
              "shrink-0 rounded border px-1.5 py-px text-2xs tabular-nums transition-colors",
              n > 0
                ? "border-[var(--sem-ok-text)]/50 bg-[color-mix(in_srgb,var(--sem-ok-text)_10%,transparent)] text-[var(--sem-ok-text)] hover:bg-[color-mix(in_srgb,var(--sem-ok-text)_18%,transparent)]"
                : "border-slate-700 text-ink-4 hover:border-slate-500 hover:text-ink-2",
            )}
            onClick={(e) => chipClick(c.app, model, cell, e)}
            onMouseEnter={(e) => setHover({ app: c.app, x: e.clientX, y: e.clientY })}
          >
            {c.label}
            {n > 0 ? `/${n}` : ""}
          </button>
        );
      })}
      {overflow && (
        <button
          type="button"
          className="shrink-0 rounded border border-dashed border-slate-600 px-1.5 py-px text-2xs text-ink-3 hover:border-cyan-500/60 hover:text-cyan-300"
          title={t("row.moreMatrix")}
          onClick={gotoMatrix}
        >
          {t("row.moreMatrix")}
        </button>
      )}
      {hover && hoverClient && (
        <PointPopover onClose={() => setHover(null)} point={{ x: hover.x, y: hover.y }}>
          <div className="min-w-[160px] text-2xs">
            <div className="px-2 py-1 text-ink-4">{hoverClient.label}</div>
            {hoverSlots.length > 0 ? (
              hoverSlots.map((s) => (
                <div key={s} className="px-2 py-0.5 text-ink-2">
                  {slotLabel(hoverClient, s)}
                </div>
              ))
            ) : (
              <div className="px-2 py-0.5 text-ink-4">{t("row.noSlotHeld")}</div>
            )}
          </div>
        </PointPopover>
      )}
    </div>
  );
}
