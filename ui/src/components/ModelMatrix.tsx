// 模型矩阵（ui_04 转置，用户 2026-11-21）：行 = 各客户端，列 = 按后端分组的有效模型；
// 格子里直接列出该端占用的模型（一模型一行），「＋」弹该组未占用的模型做新分配。
//
// 为什么要有这张表：以前想「让 codex 多用两个模型」得去接管页一家家点开。矩阵把
// 分配关系摊平成一页点几下的事，而格子里的模型**永远来自各家配置文件**，不是数据库里
// 的期望——手工改过的东西一眼就能看出来。
//
// 两层的规矩（后端同样校验一遍）：
// · 该端实际可见 = 全局暴露 ∩ 该端清单；没接管过 = 全暴露（零配置）；
//   接管后的清单只写**分配给它的模型**，所以格子里有名字 = 真的分配过；
// · claude / codebuddy 没有清单概念，只有档位槽（claude: haiku/sonnet/opus/fable/
//   subagent + 默认；codebuddy: lite/reasoning + 默认），条目语义是**占槽**——点条目
//   弹档位浮层（与后端页模型行同一套 TierPopover），同档互斥、被占直接顶替；
// · pi 是两槽制（主模型 main / 小模型 small，互斥）：占了槽就显示槽名；
// · gemini 容量 1：唯一模型位已占时点它只提示，点别的模型（走＋浮层）即切换。
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { QueryError } from "./QueryError";
import { ipc } from "../lib/ipc";
import type { MatrixClient, MatrixView } from "../lib/ipc";
import { TierPopover } from "./TierPopover";
import { PointPopover } from "./Popover";
import { buildOccupancy, cellSlots, piLabels, slotLabel } from "./model-rows/alloc";
import { isFull, isTierOnly } from "./model-matrix/Cell";
import { groupKey, otherGroupLabel } from "../lib/modelGroups";
import { every, POLL } from "../lib/poll";
import { cx } from "../lib/cx";
import { t, ts } from "../lib/i18n";

type MatrixRow = MatrixView["rows"][number];

/** 开着槽位浮层的格子（fixed 定位：表格有 overflow 容器，absolute 会被裁）。 */
interface PopState {
  model: string;
  app: string;
  x: number;
  y: number;
}

/** 开着「添加模型」浮层的格子（ui_04：格内只列已占用模型，新分配走＋入口）。
 *  change = 要顶掉的现占模型（单模型槽端点：点已占行弹「变更模型」，用户 2026-11-21）。 */
interface AddPopState {
  app: string;
  group: string;
  x: number;
  y: number;
  change?: string;
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

/** 首列按后端分组（fix:model-lists-group-by-backend）：组名 = 该模型来自的后端拼串
 *  （多后端共享的归 `a + b`，与后端 option_groups 逐字同口径），组序按组名字典序稳定排。
 *  没标注后端的行归「其它」，绝不因为要分组就丢行。 */
function groupRows(rows: MatrixRow[]): Array<{ group: string; rows: MatrixRow[] }> {
  const map = new Map<string, MatrixRow[]>();
  for (const row of rows) {
    const key = row.backends.length ? groupKey(row.backends) : otherGroupLabel();
    const bucket = map.get(key);
    if (bucket) bucket.push(row);
    else map.set(key, [row]);
  }
  return [...map.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([group, rs]) => ({ group, rows: rs }));
}

export default function ModelMatrix({ installedApps }: { installedApps?: Set<string> | null }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [pop, setPop] = useState<PopState | null>(null);
  const [addPop, setAddPop] = useState<AddPopState | null>(null);
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
      toast.ok(ts("mm.written", { app: vars.app }));
    },
    onError: (e: Error) => {
      setPop(null);
      toast.err(ts("mm.fail", { e: e.message }));
    },
  });

  if (isLoading) return null;
  if (error) {
    return (
      <QueryError title={t("mm.readFail")} error={error} onRetry={() => refetch()} />
    );
  }
  const view = data!;
  if (!view.rows.length) return null;
  const busy = mutate.isPending;
  // 只显示已安装代理的行（installedApps 为 null = 全部）：未装代理不参与分配。
  // 门面（copilot）不再例外——装了就和别家一样按装没装决定行在不在，「显示全部代理」把它带回来。
  // UI-11：装完再按用户口径重排行序。
  const cols = sortCols(
    installedApps == null
      ? view.clients
      : view.clients.filter((c) => installedApps.has(c.app)),
  );
  // 列分组（ui_04 转置）：模型按后端分组当列，组序按组名字典序稳定排。
  const groups = groupRows(view.rows);

  // 档位占用表（app → slot → 模型）：浮层里标「将替换」，与后端页同源同算法（alloc.ts）。
  const occByApp = buildOccupancy(view);

  return (
    <div className="rounded-lg border border-slate-800 bg-slate-900">
      {/* UI-11 再补刀（用户 2026-11-16，附截图 1 号区）：卡片头整行删除——
          标题、?、右侧提示都不要了，表格直接顶到卡片沿。 */}
      <div className="overflow-x-auto">
        {/* ui_04 定案（用户 2026-11-21）：行=代理，列=后端分组；每个代理占了哪些模型直接列在格子里
            （一模型一行，长 ID break-all 换行），列宽 table-fixed 在可视区内摊分，**不出现横向滚动条**。
            点已占行：档位槽端弹槽位浮层，清单端直接取消分配；「＋」弹该组可分配（未被占）的模型。 */}
        <table className="w-full table-fixed text-left text-2xs">
          <thead>
            <tr className="text-ink-3">
              <th className="w-[150px] py-2 pl-3 pr-2 text-left font-normal">{t("mm.agent")}</th>
              {groups.map((g) => (
                <th
                  key={g.group}
                  className="border-b border-slate-800 bg-slate-800/40 px-2 py-1 text-center font-normal text-ink-2"
                >
                  {g.group}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {cols.map((c) => {
              const full = isFull(c);
              // 满了 ≠ 锁死（用户 2026-11-17）：档位槽端满了照样能点，点进来选要顶替哪个槽。
              let fullNote = "";
              if (full) {
                fullNote = isTierOnly(c) ? t("mm.fullTier") : t("mm.fullList");
              }
              // 该端对这个模型是否占着：占槽 / 默认 / 在清单。占着返回其槽位集，没占返回 null。
              const heldOf = (row: MatrixRow) => {
                const cell = row.cells[c.app];
                if (!cell) return null;
                const slots = cell.slots ?? (cell.slot ? [cell.slot] : []);
                return slots.length > 0 || cell.is_default || cell.in_list ? slots : null;
              };
              return (
                <tr key={c.app} className="border-t border-slate-800/60 align-top">
                  <td
                    className="py-1 pl-3 pr-2 text-left text-ink-1"
                    title={[
                      c.label,
                      c.taken_over ? t("ag.takenOver") : t("mm.notTaken"),
                      c.capacity
                        ? ts("mm.capacity", { c: c.capacity, u: c.used, note: fullNote })
                        : ts("mm.unlimited", { u: c.used }),
                      c.note ?? "",
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  >
                    <span className="block truncate" title={c.label}>
                      {c.label}
                      {!c.taken_over && <span className="ml-1 text-ink-4">×</span>}
                    </span>
                    <span
                      className={cx(
                        "block font-mono text-2xs",
                        full ? "text-[var(--sem-busy-text)]" : "text-ink-4",
                      )}
                    >
                      {c.capacity != null ? `${c.used}/${c.capacity}` : ts("mm.unlimited", { u: c.used })}
                    </span>
                  </td>
                  {groups.map((g) => (
                    <td key={g.group} className="px-1.5 py-1">
                      {g.rows.map((row) => {
                        const slots = heldOf(row);
                        if (!slots) return null;
                        const cell = row.cells[c.app];
                        return (
                          <button
                            key={row.model}
                            type="button"
                            disabled={busy}
                            className={cx(
                              "mb-0.5 block w-full break-all rounded border px-1.5 py-0.5 text-left font-mono transition-colors",
                              cell.is_default && slots.length === 0
                                ? "border-amber-500/70 bg-amber-500/15 text-ink-1"
                                : "border-cyan-500/50 bg-cyan-500/10 text-ink-1",
                              busy ? "dim-idle" : "hover:bg-slate-800",
                            )}
                            title={t("mm.heldTip")}
                            onClick={(ev) => {
                              setAddPop(null);
                              // 档位槽端且占着槽：弹槽位浮层（顶替/清空都在浮层里）。
                              if (c.slots.length > 0 && slots.length > 0) {
                                setPop(
                                  pop?.model === row.model && pop?.app === c.app
                                    ? null
                                    : { model: row.model, app: c.app, x: ev.clientX, y: ev.clientY },
                                );
                                return;
                              }
                              // 单模型槽端点（无槽或容量 1）：点已占行 = 变更模型，从可用清单里选一个顶掉现占；
                              // 不再只能取消（用户 2026-11-21：别的后端占了也得能在这换）。
                              if (c.slots.length === 0 || c.capacity === 1) {
                                setAddPop({ app: c.app, group: "", change: row.model, x: ev.clientX, y: ev.clientY });
                                return;
                              }
                              // 其余（槽+清单双轨端的清单占用行）：直接取消分配。
                              mutate.mutate({ app: c.app, model: row.model, op: "remove" });
                            }}
                          >
                            {row.model}
                            {slots.length > 0 ? (
                              <span className="ml-1 text-ink-4">{slots.map((s) => slotLabel(c, s)).join("/")}</span>
                            ) : (
                              cell.is_default && <span className="ml-1 text-ink-4">· 默认</span>
                            )}
                          </button>
                        );
                      })}
                      <button
                        type="button"
                        disabled={busy}
                        className={cx(
                          // 通栏虚线占位（用户 2026-11-21 截图拍板）：与模型行同宽充满列，
                          // 不再悬一个小加号在格中；空列一眼看出是可添加，不是缺渲染。
                          "mt-0.5 block w-full rounded border border-dashed border-slate-700 px-1.5 py-0.5 text-center text-ink-4",
                          busy ? "dim-idle" : "hover:border-cyan-500/60 hover:bg-slate-800 hover:text-cyan-300",
                        )}
                        title={ts("mm.addCellTip", { label: c.label })}
                        onClick={(ev) => {
                          // 该组全被占用时浮层是空的——点了要有话说，不静默（原型同口径给 toast）。
                          if (!g.rows.some((row) => !heldOf(row))) {
                            toast.info(ts("mm.addEmpty", { label: c.label }));
                            return;
                          }
                          setAddPop(
                            addPop?.app === c.app && addPop?.group === g.group
                              ? null
                              : { app: c.app, group: g.group, x: ev.clientX, y: ev.clientY },
                          );
                        }}
                      >
                        ＋
                      </button>
                    </td>
                  ))}
                </tr>
              );
            })}
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
          // 未接管也照样弹浮层：选完槽位由后端顺手接管（用户 2026-11-17）。
          if (!client || !cell) return null;
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
      {/* 添加/变更模型浮层（ui_04）：＋ 列该组里该端尚未占用的模型；变更模式列全部可用模型，
          选中即顶掉现占（容量 1 端走 default，清单端 add 新 + remove 旧）。档位槽端选完弹槽位浮层。 */}
      {addPop &&
        (() => {
          const client = view.clients.find((c) => c.app === addPop.app);
          const g = groups.find((x) => x.group === addPop.group);
          if (!client) return null;
          const isChange = !!addPop.change;
          const list = isChange
            ? view.rows
            : (g?.rows ?? []).filter((row) => {
                const cell = row.cells[client.app];
                if (!cell) return true;
                const slots = cell.slots ?? (cell.slot ? [cell.slot] : []);
                return !(slots.length > 0 || cell.is_default || cell.in_list);
              });
          if (!list.length) return null;
          return (
            <PointPopover onClose={() => setAddPop(null)} point={{ x: addPop.x, y: addPop.y }}>
              <div className="max-h-64 min-w-[200px] overflow-auto">
                <div className="px-2.5 py-1 text-2xs text-ink-4">
                  {isChange ? ts("mm.changeHint", { model: addPop.change! }) : t("mm.addHint")}
                </div>
                {list.map((row) => {
                  const cur = isChange && row.model === addPop.change;
                  return (
                    <button
                      key={row.model}
                      type="button"
                      disabled={cur}
                      className={cx(
                        "flex w-full items-center justify-between gap-2 break-all rounded px-2.5 py-1 text-left font-mono",
                        cur ? "text-ink-4" : "text-ink-2 hover:bg-slate-800",
                      )}
                      onClick={(ev) => {
                        setAddPop(null);
                        if (isChange) {
                          const old = addPop.change!;
                          if (client.app === "copilot") {
                            // 门面可见清单：没有默认档概念（default 会被后端拒），先加新再拆旧。
                            mutate.mutate({ app: client.app, model: row.model, op: "add" });
                            mutate.mutate({ app: client.app, model: old, op: "remove" });
                          } else {
                            // 「设为默认」= 让这个端用它：gemini 唯一合法算子（add/remove 都会被拒），
                            // 清单端（opencode/DevEco）也会顺手补进清单（后端 ensure_default_visible 口径）。
                            mutate.mutate({ app: client.app, model: row.model, op: "default" });
                            // 清单端再把旧模型从清单摘掉完成替换；gemini 只有一个位，remove 会报错，不发。
                            if (client.capacity !== 1) {
                              mutate.mutate({ app: client.app, model: old, op: "remove" });
                            }
                          }
                          return;
                        }
                        if (client.slots.length > 0) {
                          setPop({ model: row.model, app: client.app, x: ev.clientX, y: ev.clientY });
                        } else if (client.capacity === 1) {
                          // gemini：＋ 也走「设为默认」（add 会被后端拒：只有一个模型位）。
                          mutate.mutate({ app: client.app, model: row.model, op: "default" });
                        } else {
                          mutate.mutate({ app: client.app, model: row.model, op: "add" });
                        }
                      }}
                    >
                      <span>{row.model}</span>
                      {cur && <span className="shrink-0 font-sans text-2xs">{t("mm.curModel")}</span>}
                    </button>
                  );
                })}
              </div>
            </PointPopover>
          );
        })()}
    </div>
  );
}
