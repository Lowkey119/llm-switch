// 模型矩阵（M12-G）：行 = 对外暴露的模型，列 = 各客户端，格子 = 这个端用不用它。
//
// 为什么要有这张表：以前想「让 codex 多用两个模型」得去接管页一家家点开。矩阵把
// 分配关系摊平成一页点几下的事，而格子上的勾**永远来自各家配置文件**，不是数据库里
// 的期望——手工改过的东西一眼就能看出来。
//
// 两层的规矩（后端同样校验一遍）：
// · 该端实际可见 = 全局暴露 ∩ 该端清单；没接管过 = 全暴露（零配置）；
//   接管后的清单只写**分配给它的模型**，所以格子亮 = 真的分配过，不再有
//   「清单缺省=全暴露」造成的整列恒亮；
// · claude / codebuddy 没有清单概念，只有档位槽（claude: haiku/sonnet/opus/fable/
//   subagent + 默认；codebuddy: lite/reasoning + 默认），格子语义是**占槽**——点格子
//   弹档位浮层（与后端页模型行同一套 TierPopover），同档互斥、被占直接顶替；
// · pi 是两槽制（主模型 main / 小模型 small，互斥）：格子亮 = 占了某个槽，点格子
//   同样弹槽位浮层；槽位显示名 = 实配模型的有意义前缀（modelPrefix），默认档自理不摆 ★；
// · gemini 容量 1：点格子 = 直接切为唯一模型。
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { QueryError } from "./QueryError";
import { ipc } from "../lib/ipc";
import type { MatrixCell, MatrixClient, MatrixView } from "../lib/ipc";
import { TierPopover } from "./TierPopover";
import { PointPopover } from "./Popover";
import { buildOccupancy, cellSlots, piLabels, slotLabel } from "./model-rows/alloc";
import { every, POLL } from "../lib/poll";

/** 这个项目没有 clsx，就地拼 class。 */
const cx = (...parts: Array<string | false | null>) => parts.filter(Boolean).join(" ");


/** 开着槽位浮层的格子（fixed 定位：表格有 overflow 容器，absolute 会被裁）。 */
interface PopState {
  model: string;
  app: string;
  x: number;
  y: number;
}

/** 列序（UI-11，用户 2026-11-16）：Claude Code、Codex、Codebuddy、OpenCode、
 *  Pi、DevEco、Copilot、Gemini。后端 ClientApp::ALL 的顺序不动（别处共用），
 *  只在这里按用户口径重排；没登记的新代理排最后、保持原有相对顺序。 */
const COL_ORDER = ["claude", "codex", "codebuddy", "opencode", "pi", "deveco", "copilot", "gemini"];

function sortCols(cs: MatrixClient[]): MatrixClient[] {
  const rank = (app: string) => {
    const i = COL_ORDER.indexOf(app);
    return i === -1 ? COL_ORDER.length : i;
  };
  return [...cs].sort((a, b) => rank(a.app) - rank(b.app));
}

/** 列头提示里的槽位清单文本。 */
function slotListText(client: MatrixClient): string {
  return client.app === "pi" ? "主模型 + 小模型" : `main + ${client.tiers.join("/")}`;
}

function Cell({
  client,
  cell,
  busy,
  onOpenPop,
  onOp,
}: {
  client: MatrixClient;
  cell: MatrixCell;
  busy: boolean;
  onOpenPop: (app: string, x: number, y: number) => void;
  onOp: (app: string, op: "add" | "remove" | "default") => void;
}) {
  const toast = useToast();
  const tiered = client.slots.length > 0;
  // pi / codex：槽位只约束档位槽，格子还兼清单的追加/取消（BUG-4，用户 2026-11-16）。
  const listable = client.app === "pi" || client.app === "codex";
  const gemini = client.capacity === 1 && client.app === "gemini";
  const usable = client.taken_over && !busy;
  // 一个模型现在能同时占多个槽，所以格子看的是 slots 全集。
  const held = cell.slots ?? (cell.slot ? [cell.slot] : []);
  const occupying = held.length > 0 || cell.is_default;
  const title = !client.taken_over
    ? `${client.label} 尚未接管：先在接管页接管才能分配模型`
    : listable
      ? occupying
        ? `占着 ${held.map((s) => slotLabel(client, s)).join(" / ")}：点格子取消分配，点「槽」调整槽位（同槽被占即顶替）`
        : cell.in_list
          ? `已在 ${client.label} 的模型清单：点格子移出，点「槽」占档位槽`
          : `点格子把模型加进 ${client.label} 的模型清单（槽位制只约束档位槽，清单可多模型）`
      : tiered
      ? occupying
        ? `占着 ${held.map((s) => slotLabel(client, s)).join(" / ")}，点格子调整槽位（同一槽被占即顶替，不同槽可以是同一个模型）`
        : `${client.label} 是槽位制（${slotListText(client)}）：点格子弹槽位选择`
      : gemini
        ? cell.is_default
          ? "gemini 唯一模型位已占用；点别的模型即切换"
          : "Gemini CLI 只有一个模型位：点格子直接设为它的模型"
        : cell.is_default
          ? `${client.label} 的默认，点格子移出`
          : cell.in_list
            ? client.default_allowed
              ? `点格子移出；点 ★ 设为 ${client.label} 的默认`
              : `点格子移出 ${client.label} 的可见清单`
            : `点格子让 ${client.label} 也能用它`;
  return (
    <td className="px-0.5 py-1 align-middle" title={title}>
      {!client.taken_over ? (
        <span className="text-xs text-ink-4">—</span>
      ) : (
        /* 格子以前是裸 div + onClick：鼠标能点、键盘走不到（UI-AUDIT-2 G-2，
           P0-3 的漏网）。不能直接换成 <button>：里面那颗★也是按钮，
           按钮套按钮是非法 HTML——用与 Row/InlineEdit 同款的 role=button 口径。 */
        <div
          role="button"
          tabIndex={usable ? 0 : -1}
          aria-disabled={!usable}
          onKeyDown={(ev) => {
            if (ev.key !== "Enter" && ev.key !== " ") return;
            ev.preventDefault();
            if (!usable) return;
            if (listable) onOp(client.app, cell.in_list ? "remove" : "add");
            else if (tiered) onOpenPop(client.app, ev.currentTarget.getBoundingClientRect().right, ev.currentTarget.getBoundingClientRect().bottom);
            else if (gemini) {
              if (cell.is_default) toast.info("gemini 只有一个模型位，点别的模型切换");
              else onOp(client.app, "default");
            } else onOp(client.app, cell.in_list ? "remove" : "add");
          }}
          className={cx(
            'inline-flex w-full flex-wrap items-center justify-center gap-1 rounded border px-1 py-1 text-2xs leading-none transition-colors',
            cell.in_list
              ? 'border-cyan-500/50 bg-cyan-500/10 text-ink-1'
              : 'border-slate-800 text-ink-4',
            cell.is_default && 'border-amber-500/70 bg-amber-500/15',
            usable ? 'cursor-pointer hover:bg-slate-800' : 'dim-idle',
          )}
          onClick={(ev) => {
            if (!usable) return;
            // pi / codex（BUG-4）：格子本体 = 清单追加/取消；档位槽改动走「槽」浮层。
            if (listable) {
              onOp(client.app, cell.in_list ? "remove" : "add");
              return;
            }
            // 档位制端：点哪格都弹档位浮层（与后端页模型行同一套交互），
            // 不再盲发 remove——那对「没占槽但恒亮」的格子是空操作，点了没反应。
            if (tiered) {
              onOpenPop(client.app, ev.clientX, ev.clientY);
              return;
            }
            if (gemini) {
              if (cell.is_default) toast.info("gemini 只有一个模型位，点别的模型切换");
              else onOp(client.app, "default");
              return;
            }
            onOp(client.app, cell.in_list ? "remove" : "add");
          }}
        >
          {listable && (
            <button
              type="button"
              className="leading-none text-ink-4 hover:text-ink-1"
              title={`调整 ${client.label} 的档位槽（点格子本身是清单追加/取消）`}
              onClick={(ev) => {
                ev.stopPropagation();
                if (usable) onOpenPop(client.app, ev.clientX, ev.clientY);
              }}
            >
              槽
            </button>
          )}
          {client.default_allowed && (cell.in_list || tiered || gemini) && (
            <button
              type="button"
              className={cx(
                'leading-none',
                cell.is_default
                  ? 'text-[var(--sem-busy-text)]'
                  : 'text-ink-4 hover:text-ink-1',
              )}
              title={cell.is_default ? `${client.label} 的默认模型` : `设为 ${client.label} 的默认`}
              onClick={(ev) => {
                ev.stopPropagation();
                if (usable && !cell.is_default) onOp(client.app, 'default');
              }}
            >
              ★
            </button>
          )}
          {cell.in_list ? <span>✓</span> : <span>·</span>}
          {held.length > 0 && (
            <span className="text-ink-4">
              {held.map((s) => (client.app === "pi" ? (s === "small" ? "小" : "主") : s)).join("/")}
            </span>
          )}
        </div>
      )}
    </td>
  );
}

export default function ModelMatrix({ installedApps }: { installedApps?: Set<string> | null }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [pop, setPop] = useState<PopState | null>(null);
  const { data, isLoading, error, refetch } = useQuery<MatrixView>({
    queryKey: ['matrix'],
    queryFn: () => ipc<MatrixView>('matrix_get'),
    refetchInterval: () => every(POLL.normal),
  });

  const mutate = useMutation({
    mutationFn: (vars: { app: string; model: string; op: string; slot?: string }) =>
      ipc('matrix_cell', { ...vars, slot: vars.slot ?? null }),
    onSuccess: (_r, vars) => {
      setPop(null);
      qc.invalidateQueries({ queryKey: ['matrix'] });
      qc.invalidateQueries({ queryKey: ['models'] });
      qc.invalidateQueries({ queryKey: ['takeover'] });
      toast.ok(`已写入（${vars.app}）：重启该客户端后生效`);
    },
    onError: (e: Error) => {
      setPop(null);
      toast.err(`改动失败：${e.message}`);
    },
  });

  if (isLoading) return null;
  if (error) {
    return (
      <QueryError title="模型矩阵读取失败" error={error} onRetry={() => refetch()} />
    );
  }
  const view = data!;
  if (!view.rows.length) return null;
  const busy = mutate.isPending;
  // 只显示已安装代理的列（installedApps 为 null = 全部）：未装代理不参与分配。
  // 门面（copilot）不再例外——装了就和别家一样按装没装决定列在不在，「显示全部代理」把它带回来。
  // UI-11：装完再按用户口径重排列序。
  const cols = sortCols(
    installedApps == null
      ? view.clients
      : view.clients.filter((c) => installedApps.has(c.app)),
  );

  // 档位占用表（app → slot → 模型）：浮层里标「将替换」，与后端页同源同算法（alloc.ts）。
  const occByApp = buildOccupancy(view);

  return (
    <div className="rounded-lg border border-slate-800 bg-slate-900">
      {/* UI-11 再补刀（用户 2026-11-16，附截图 1 号区）：卡片头整行删除——
          标题、?、右侧提示都不要了，表格直接顶到卡片沿。 */}
      <div className="overflow-x-auto">
        {/* UI-11 再补刀 2（用户 2026-11-16，附截图）：代理列**等宽且最窄**——
            table-fixed 下代理列统一 w-[64px]，模型列吃剩余宽度；列头不再 nowrap，
            格子内容 flex-wrap：槽位多就换行，但**不得**把列撑宽。 */}
        <table className="w-full table-fixed text-left text-2xs">
          <thead>
            <tr className="text-ink-3">
              <th className="py-2 pl-3 pr-2 text-left font-normal">模型</th>
              {cols.map((c) => {
                const full = c.capacity != null && c.used >= c.capacity;
                return (
                  <th
                    key={c.app}
                    className="w-[64px] px-0.5 py-2 text-center font-normal"
                    title={[
                      c.label,
                      c.taken_over ? '已接管' : '未接管',
                      c.capacity
                        ? `最多 ${c.capacity} 个模型位，已用 ${c.used}${full ? "（已满：点空格子弹档位浮层顶替）" : ""}`
                        : `数量不限（清单制：已用 ${c.used}）`,
                      c.note ?? '',
                    ]
                      .filter(Boolean)
                      .join(' · ')}
                  >
                    {c.label}
                    {!c.taken_over && <span className="ml-1 text-ink-4">×</span>}
                    {c.capacity != null && (
                      <span
                        className={cx(
                          'ml-1 font-mono text-2xs',
                          full ? 'text-[var(--sem-busy-text)]' : 'text-ink-4',
                        )}
                      >
                        {c.used}/{c.capacity}
                      </span>
                    )}
                  </th>
                );
              })}
            </tr>
          </thead>
          <tbody>
            {view.rows.map((row) => (
              <tr key={row.model} className="border-t border-slate-800/60">
                {/* 队列④ 后端标签加入后居中会让两边缘参差、名字贴边线（用户实看反馈）：
                    改回左对齐 + pl-3 留边距，与技能矩阵首列同口径 */}
                <td className="py-1 pl-3 pr-2 align-middle text-left text-ink-1">
                  <div className="flex items-center gap-1.5">
                    {/* 长模型名限宽、最多两行（title 看全名）：不约束会把行高撑得参差不齐 */}
                    <span className="line-clamp-2 min-w-0" title={row.model}>
                      {row.model}
                    </span>
                    {/* 队列④：灰色后缀小标签——该模型来自哪些启用后端（名字本体不动） */}
                    {row.backends.length > 0 && (
                      <span
                        title={`来自后端：${row.backends.join("、")}`}
                        className="shrink-0 rounded border border-slate-700/70 bg-slate-800/60 px-1 py-px text-2xs font-normal text-ink-4"
                      >
                        {row.backends.join(", ")}
                      </span>
                    )}
                  </div>
                </td>
                {cols.map((c) => (
                  <Cell
                    key={c.app}
                    client={c}
                    cell={row.cells[c.app]}
                    busy={busy}
                    onOpenPop={(app, x, y) =>
                      setPop(
                        pop?.model === row.model && pop?.app === app ? null : { model: row.model, app, x, y },
                      )
                    }
                    onOp={(app, op) => mutate.mutate({ app, model: row.model, op })}
                  />
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {/* 档位浮层：fixed 定位（表格 overflow 裁不到），贴着点击点弹出。
          ⚠️ 主窗口缩放挂在 documentElement 的 CSS zoom 上：WebKit 的 clientX/Y
          是**视口空间**（FloatPanel 同族实测：量测值已含 zoom），而 zoom 树里的
          fixed left/top 布局时会**再乘一次**倍率——不除回去弹层就偏到
          「点击点 × zoom」（用户实看：1.33 倍缩放时向右下漂约 1/3 屏）。 */}
      {pop &&
        (() => {
          const client = view.clients.find((c) => c.app === pop.app);
          const cell = view.rows.find((r) => r.model === pop.model)?.cells[pop.app];
          if (!client || !cell || !client.taken_over) return null;
          const current = cellSlots(cell);
          const slots = client.slots;
          const occ = occByApp[client.app] ?? {};
          // pi 槽位显示名 = 实配模型的有意义前缀（占着谁显示谁），空槽退回角色词。
          const labels = piLabels(client, slots, occ);
          return (
            <PointPopover onClose={() => setPop(null)} point={{ x: pop.x, y: pop.y }}>
              <TierPopover
                slots={slots}
                labels={labels}
                current={current}
                occ={occ}
                self={pop.model}
                onPick={(slot) => {
                  setPop(null);
                  mutate.mutate({ app: client.app, model: pop.model, op: "add", slot });
                }}
                onRemove={() => {
                  setPop(null);
                  mutate.mutate({ app: client.app, model: pop.model, op: "remove" });
                }}
              />
            </PointPopover>
          );
        })()}
    </div>
  );
}
