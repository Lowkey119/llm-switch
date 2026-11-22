// 就地分配浮层（从 ModelRows 原样搬出）：贴点击点弹出该端槽位
// （与矩阵页同源组件与算法）。fixed 定位不受行 overflow 裁剪；
// 坐标换算收在 PointPopover 里（zoom 口径）。
import type { Dispatch, SetStateAction } from "react";
import { ipc } from "../../lib/ipc";
import type { MatrixCell, MatrixClient } from "../../lib/ipc";
import { PointPopover } from "../Popover";
import { TierPopover } from "../TierPopover";
import { cellSlots, piLabels } from "./alloc";
import { t, ts } from "../../lib/i18n";

export interface AllocPop {
  app: string;
  model: string;
  x: number;
  y: number;
  /** slots = 选槽（claude/codebuddy/pi/codex）；catalog = 清单制端的两个动作。 */
  kind: "slots" | "catalog";
}

export function AllocPopover({
  pop,
  setPop,
  clients,
  cells,
  occ,
  run,
}: {
  pop: AllocPop | null;
  setPop: Dispatch<SetStateAction<AllocPop | null>>;
  clients: MatrixClient[];
  cells: Record<string, Record<string, MatrixCell>>;
  /** 占用表（app → slot → 模型）：浮层里标「将替换」，与矩阵页同源算法。 */
  occ: Record<string, Record<string, string>>;
  /** 通用操作包装：成功 toast + onChanged（口径在 ModelRows）。 */
  run: (msg: string, fn: () => Promise<unknown>) => Promise<void>;
}) {
  // 关闭态直接不渲染（pop = null）。
  if (!pop) return null;
  const client = clients.find((c) => c.app === pop.app);
  const cell = cells[pop.model]?.[pop.app];
  // 未接管不再挡浮层：分配即接管（用户 2026-11-17）。
  if (!client || !cell) return null;
  const appOcc = occ[client.app] ?? {};
  const slots = client.slots;
  return (
    <PointPopover onClose={() => setPop(null)} point={{ x: pop.x, y: pop.y }}>
      {pop.kind === "catalog" ? (
        <div className="flex min-w-[14rem] flex-col gap-0.5 p-1 text-sm">
          <div className="px-1.5 py-0.5 text-ink-4">
            {ts("ap.inList", { label: client.label, model: pop.model })}
          </div>
          <button
            className="rounded px-1.5 py-1 text-left text-ink-2 hover:bg-slate-800"
            onClick={() => {
              const { app, model } = pop;
              setPop(null);
              void run(ts("ap.setAsModel", { app }), () =>
                ipc("matrix_cell", { app, model, op: "default", slot: null }),
              );
            }}
          >
            {t("ap.setCur")}
          </button>
          <button
            className="rounded px-1.5 py-1 text-left text-[var(--sem-danger-text)] hover:bg-slate-800"
            onClick={() => {
              const { app, model } = pop;
              setPop(null);
              void run(ts("ap.unassigned", { app }), () =>
                ipc("matrix_cell", { app, model, op: "remove", slot: null }),
              );
            }}
          >
            {t("ap.removeList")}
          </button>
        </div>
      ) : (
        <TierPopover
          slots={slots}
          labels={piLabels(client, slots, appOcc)}
          current={cellSlots(cell)}
          occ={appOcc}
          self={pop.model}
          onPick={(slot) => {
            const { app, model } = pop;
            setPop(null);
            void run(ts("ap.assigned", { app, slot }), () =>
              ipc("matrix_cell", { app, model, op: "add", slot }),
            );
          }}
          onRemove={() => {
            const { app, model } = pop;
            setPop(null);
            void run(ts("ap.unassigned", { app }), () =>
              ipc("matrix_cell", { app, model, op: "remove", slot: null }),
            );
          }}
        />
      )}
    </PointPopover>
  );
}
