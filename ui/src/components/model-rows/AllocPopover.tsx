// 就地分配浮层（从 ModelRows 原样搬出）：贴点击点弹出该端槽位
// （与矩阵页同源组件与算法）。fixed 定位不受行 overflow 裁剪；
// 坐标换算收在 PointPopover 里（zoom 口径）。
import type { Dispatch, SetStateAction } from "react";
import { ipc } from "../../lib/ipc";
import type { MatrixCell, MatrixClient } from "../../lib/ipc";
import { PointPopover } from "../Popover";
import { TierPopover } from "../TierPopover";
import { cellSlots, piLabels } from "./alloc";

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
  if (!client || !cell || !client.taken_over) return null;
  const appOcc = occ[client.app] ?? {};
  const slots = client.slots;
  return (
    <PointPopover onClose={() => setPop(null)} point={{ x: pop.x, y: pop.y }}>
      {pop.kind === "catalog" ? (
        <div className="flex min-w-[14rem] flex-col gap-0.5 p-1 text-xs">
          <div className="px-1.5 py-0.5 text-ink-4">
            {client.label} 的清单里有 {pop.model}
          </div>
          <button
            className="rounded px-1.5 py-1 text-left text-ink-2 hover:bg-slate-800"
            onClick={() => {
              const { app, model } = pop;
              setPop(null);
              void run(`已设为 ${app} 的模型，重启该客户端后生效`, () =>
                ipc("matrix_cell", { app, model, op: "default", slot: null }),
              );
            }}
          >
            设为当前模型
          </button>
          <button
            className="rounded px-1.5 py-1 text-left text-[var(--sem-danger-text)] hover:bg-slate-800"
            onClick={() => {
              const { app, model } = pop;
              setPop(null);
              void run(`已从 ${app} 取消分配`, () =>
                ipc("matrix_cell", { app, model, op: "remove", slot: null }),
              );
            }}
          >
            移出清单
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
            void run(`已分配给 ${app}（${slot}），重启该客户端后生效`, () =>
              ipc("matrix_cell", { app, model, op: "add", slot }),
            );
          }}
          onRemove={() => {
            const { app, model } = pop;
            setPop(null);
            void run(`已从 ${app} 取消分配`, () =>
              ipc("matrix_cell", { app, model, op: "remove", slot: null }),
            );
          }}
        />
      )}
    </PointPopover>
  );
}
