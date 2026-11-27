// 模型矩阵（matrix-slots 回灌，用户 2026-11-22）：行 = 各客户端，列 = 槽位 1..6 + 「…」溢出列。
// 为什么槽位列：列若按后端分组会随后端数爆炸；槽位列恒定 7 列，一个代理一行，
// 「这家在用什么」一眼到底。两类格子语义（后端 matrix.rs 同源口径）：
// · 档位槽端（claude / codebuddy）：每槽一个模型——列 i = 第 i 个槽键（main/haiku/…），
//   点已占格 = 顶替或清空此槽，点空槽虚线格 = 分配；
// · 清单制端（codex / pi / opencode / DevEco / gemini / copilot）：模型清单，可添加任意
//   多个（填满 6+ 槽），实际用哪个由用户在该代理内选择——矩阵只维护清单。
//   点已占格 = 顶替或从清单移出；清单末尾挂虚线「＋」格加入；gemini 容量 1，
//   唯一模型即全部，点选即整体切换（default 算子）。
// 可用模型 = 后端开了暴露开关的模型（matrix 行集同源），浮层按后端分组展示。
// 同模型可占多槽 / 多次入清单——槽位互斥性后端已放开。
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { QueryError } from "./QueryError";
import { ipc } from "../lib/ipc";
import type { MatrixCell, MatrixClient, MatrixView } from "../lib/ipc";
import { PointPopover } from "./Popover";
import { buildOccupancy, cellSlots, slotLabel, sortCols } from "./model-rows/alloc";
import { isFull, isTierOnly } from "./model-matrix/Cell";
import { groupKey, otherGroupLabel } from "../lib/modelGroups";
import { every, POLL } from "../lib/poll";
import { cx } from "../lib/cx";
import { t, ts } from "../lib/i18n";

type MatrixRow = MatrixView["rows"][number];

/** 槽位列数（用户拍板）：6 个槽列 + 恒定「…」溢出列。 */
const MAX_COLS = 6;

/** 浮层状态（单实例：开新的自动关旧的）。
 * slot = 档位槽格（分配/顶替/清空）；item = 清单项（顶替/移出）；
 * add = 清单末尾虚线「＋」；more = 「…」溢出列。 */
type Pop =
  | { kind: "slot"; app: string; slot: string; x: number; y: number }
  | { kind: "item"; app: string; model: string; x: number; y: number }
  | { kind: "add"; app: string; x: number; y: number }
  | { kind: "more"; app: string; x: number; y: number };

/** 列序口径收在 model-rows/alloc.ts（sortCols）：矩阵与后端页灯牌带共用一份，不再各排各的。 */

/** 后端分组（浮层列可用模型用）：组名 = 该模型来自的后端拼串，组序按组名字典序稳定排。 */
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

/** 该端对这个模型是否占着：占槽 / 默认 / 在清单。 */
const occupied = (cell?: MatrixCell) =>
  !!cell && (cellSlots(cell).length > 0 || cell.in_list);

/** 清单制端的「占用清单」：按暴露行序收集该端在用的模型（格子 = 第 i 个）。 */
function occupiedList(view: MatrixView, app: string) {
  return view.rows
    .map((r) => ({ model: r.model, cell: r.cells[app] }))
    .filter((it) => occupied(it.cell));
}

export default function ModelMatrix({ installedApps }: { installedApps?: Set<string> | null }) {
  const qc = useQueryClient();
  const toast = useToast();
  const [pop, setPop] = useState<Pop | null>(null);
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
  const cols = sortCols(
    installedApps == null
      ? view.clients
      : view.clients.filter((c) => installedApps.has(c.app)),
  );
  // 浮层可用模型按后端分组（与后端 option_groups 逐字同口径）。
  const groups = groupRows(view.rows);
  // 档位占用表（app → slot → 模型）：档位行的格内容 + 浮层「当前」都吃这份。
  const occByApp = buildOccupancy(view);

  /** 一串格子写操作依次发出（替换 = 两步：写新 + 拆旧；各自成功各自回执）。 */
  const runOps = (app: string, ops: Array<{ model: string; op: string; slot?: string }>) => {
    setPop(null);
    for (const o of ops) mutate.mutate({ app, model: o.model, op: o.op, slot: o.slot });
  };

  /** 清单制「顶替」算子分流（与后端白名单逐字对齐，2026-11-21「还是不能替换」的教训）：
   * gemini 只认 default；copilot 门面只认 add/remove（无默认档）；pi 不认 default（自理），
   * 占槽就带槽名 add 直接顶替；默认档被占 → default 新（后端顺手补清单）再拆旧；
   * 其余 = add 新 + remove 旧。 */
  const replaceOps = (c: MatrixClient, cell: MatrixCell | undefined, oldModel: string, nextModel: string) => {
    if (c.capacity === 1) return [{ model: nextModel, op: "default" }];
    if (c.facade) return [{ model: nextModel, op: "add" }, { model: oldModel, op: "remove" }];
    const slots = cell ? cellSlots(cell) : [];
    if (c.app === "pi") return [{ model: nextModel, op: "add", slot: slots[0] }, { model: oldModel, op: "remove" }];
    if (cell?.is_default) return [{ model: nextModel, op: "default" }, { model: oldModel, op: "remove" }];
    if (slots.length) return [{ model: nextModel, op: "add", slot: slots[0] }, { model: oldModel, op: "remove" }];
    return [{ model: nextModel, op: "add" }, { model: oldModel, op: "remove" }];
  };

  const heldCls = (amber: boolean) =>
    cx(
      "block w-full rounded border px-1.5 py-1 text-left transition-colors",
      amber ? "border-amber-500/70 bg-amber-500/15" : "border-cyan-500/50 bg-cyan-500/10",
      "text-ink-1",
      busy ? "dim-idle" : "hover:bg-slate-800",
    );
  const emptyCls = cx(
    "block w-full rounded border border-dashed border-slate-700 px-1.5 py-1 text-center text-ink-4",
    busy ? "dim-idle" : "hover:border-cyan-500/60 hover:bg-slate-800 hover:text-cyan-300",
  );

  return (
    <div className="rounded-lg border border-slate-800 bg-slate-900">
      <div className="overflow-x-auto">
        {/* 槽位列定案（matrix-slots 原型，用户 2026-11-22）：table-fixed 把 6 个槽列
            在可视区内摊分，**不出现横向滚动条**；长模型 ID break-all 换行不撑列。 */}
        <table className="w-full table-fixed text-left text-2xs">
          <thead>
            <tr className="text-ink-3">
              <th className="w-[150px] py-2 pl-3 pr-2 text-left font-normal">{t("mm.agent")}</th>
              {Array.from({ length: MAX_COLS }, (_, i) => (
                <th
                  key={i}
                  className="border-b border-slate-800 bg-slate-800/40 px-2 py-1 text-center font-normal text-ink-2"
                >
                  {ts("mm.slotN", { n: i + 1 })}
                </th>
              ))}
              <th className="w-[64px] border-b border-slate-800 bg-slate-800/40 px-2 py-1 text-center font-normal text-ink-2">
                …
              </th>
            </tr>
          </thead>
          <tbody>
            {cols.map((c) => {
              const tiered = isTierOnly(c);
              const occ = occByApp[c.app] ?? {};
              const list = tiered ? [] : occupiedList(view, c.app);
              // gemini 容量 1：唯一模型位已占就不再摆「＋」（add 会被后端拒）。
              const canAdd = c.capacity !== 1 || list.length === 0;
              // 满了 ≠ 锁死（用户 2026-11-17）：档位槽端满了照样能点，点进来选要顶替哪个槽。
              let fullNote = "";
              if (isFull(c)) fullNote = tiered ? t("mm.fullTier") : t("mm.fullList");
              const extra = tiered ? c.slots.length - MAX_COLS : list.length - MAX_COLS;

              const slotCells = () =>
                Array.from({ length: MAX_COLS }, (_, i) => {
                  const slot = c.slots[i];
                  if (!slot) return <td key={i} className="px-1 py-1 text-center text-ink-4">—</td>;
                  const model = occ[slot];
                  return (
                    <td key={i} className="px-1.5 py-1">
                      {model ? (
                        <button
                          type="button"
                          disabled={busy}
                          title={ts("mm.slotHeldTip", { slot: slotLabel(c, slot) })}
                          className={heldCls(slot === "main")}
                          onClick={(ev) =>
                            setPop(
                              pop?.kind === "slot" && pop.app === c.app && pop.slot === slot
                                ? null
                                : { kind: "slot", app: c.app, slot, x: ev.clientX, y: ev.clientY },
                            )
                          }
                        >
                          <span className="block text-2xs text-ink-4">{slotLabel(c, slot)}</span>
                          <span className="block break-all font-mono">{model}</span>
                        </button>
                      ) : (
                        <button
                          type="button"
                          disabled={busy}
                          title={ts("mm.slotEmptyTip", { slot: slotLabel(c, slot) })}
                          className={emptyCls}
                          onClick={(ev) => setPop({ kind: "slot", app: c.app, slot, x: ev.clientX, y: ev.clientY })}
                        >
                          {ts("mm.slotN", { n: i + 1 })} ＋
                        </button>
                      )}
                    </td>
                  );
                });

              const listCells = () =>
                Array.from({ length: MAX_COLS }, (_, i) => {
                  const it = list[i];
                  if (it) {
                    const slots = cellSlots(it.cell);
                    // pi/codex 的专属槽占用显示角色名（主/小、主/副），其余按槽序列号。
                    const label =
                      (c.app === "pi" || c.app === "codex") && slots.length
                        ? slots.map((s) => slotLabel(c, s)).join("/")
                        : ts("mm.slotN", { n: i + 1 });
                    return (
                      <td key={i} className="px-1.5 py-1">
                        <button
                          type="button"
                          disabled={busy}
                          title={t("mm.itemTip")}
                          className={heldCls(it.cell!.is_default)}
                          onClick={(ev) =>
                            setPop(
                              pop?.kind === "item" && pop.app === c.app && pop.model === it.model
                                ? null
                                : { kind: "item", app: c.app, model: it.model, x: ev.clientX, y: ev.clientY },
                            )
                          }
                        >
                          <span className="block text-2xs text-ink-4">{label}</span>
                          <span className="block break-all font-mono">{it.model}</span>
                        </button>
                      </td>
                    );
                  }
                  if (i === list.length && canAdd) {
                    return (
                      <td key={i} className="px-1.5 py-1">
                        <button
                          type="button"
                          disabled={busy}
                          title={ts("mm.addCellTip", { label: c.label })}
                          className={emptyCls}
                          onClick={(ev) => setPop({ kind: "add", app: c.app, x: ev.clientX, y: ev.clientY })}
                        >
                          {ts("mm.slotN", { n: i + 1 })} ＋
                        </button>
                      </td>
                    );
                  }
                  return <td key={i} className="px-1 py-1 text-center text-ink-4">—</td>;
                });

              return (
                <tr key={c.app} className="border-t border-slate-800/60 align-top">
                  <td
                    className="tip py-1 pl-3 pr-2 text-left text-ink-1"
                    data-tip={[
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
                    <span className="block truncate">
                      {c.label}
                      {!c.taken_over && <span className="ml-1 text-2xs text-ink-4">{t("mm.notTaken")}</span>}
                    </span>
                    <span
                      className={cx(
                        "block font-mono text-2xs",
                        isFull(c) ? "text-[var(--sem-busy-text)]" : "text-ink-4",
                      )}
                    >
                      {c.capacity != null ? `${c.used}/${c.capacity}` : ts("mm.unlimited", { u: c.used })}
                    </span>
                  </td>
                  {tiered ? slotCells() : listCells()}
                  <td className="px-1.5 py-1">
                    {extra > 0 || (!tiered && c.capacity == null) ? (
                      <button
                        type="button"
                        disabled={busy}
                        title={extra > 0 ? ts("mm.moreTip", { n: extra }) : ts("mm.addCellTip", { label: c.label })}
                        className={emptyCls}
                        onClick={(ev) => setPop({ kind: "more", app: c.app, x: ev.clientX, y: ev.clientY })}
                      >
                        {extra > 0 ? `+${extra}` : "…"}
                      </button>
                    ) : (
                      <span className="block text-center text-ink-4">—</span>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {/* 「…」溢出浮层：档位端列第 7 槽起，清单端列第 7 个模型起；点条目转入对应编辑浮层。
          清单制不限数量（用户 2026-11-23）：满 6 个后「…」常驻，点开 = 溢出清单 + 继续添加。
          ⚠️ PointPopover 已处理主窗口 CSS zoom 的坐标换算（fixed 弹层在 zoom 树里会二次乘倍率）。 */}
      {pop?.kind === "more" &&
        (() => {
          const client = view.clients.find((x) => x.app === pop.app);
          if (!client) return null;
          const occ = occByApp[client.app] ?? {};
          const items = isTierOnly(client)
            ? client.slots.slice(MAX_COLS).map((s) => ({ key: s, label: slotLabel(client, s), slot: s as string | undefined, model: occ[s] }))
            : occupiedList(view, client.app)
                .slice(MAX_COLS)
                .map((it, j) => ({ key: `${it.model}-${j}`, label: ts("mm.slotN", { n: MAX_COLS + j + 1 }), slot: undefined as string | undefined, model: it.model }));
          const canAddMore = !isTierOnly(client) && client.capacity == null;
          return (
            <PointPopover onClose={() => setPop(null)} point={{ x: pop.x, y: pop.y }}>
              <div className="max-h-64 min-w-[220px] overflow-auto">
                {items.length > 0 && (
                  <div className="px-2.5 py-1 text-2xs text-ink-4">{ts("mm.moreTip", { n: items.length })}</div>
                )}
                {items.map((it) => (
                  <button
                    key={it.key}
                    type="button"
                    className="flex w-full items-baseline justify-between gap-2 rounded px-2.5 py-1 text-left hover:bg-slate-800"
                    onClick={(ev) =>
                      it.slot
                        ? setPop({ kind: "slot", app: client.app, slot: it.slot, x: ev.clientX, y: ev.clientY })
                        : setPop({ kind: "item", app: client.app, model: it.model!, x: ev.clientX, y: ev.clientY })
                    }
                  >
                    <span className="shrink-0 text-ink-3">{it.label}</span>
                    <span className={cx("break-all font-mono", it.model ? "text-ink-2" : "text-ink-4")}>
                      {it.model ?? t("mm.emptySlot")}
                    </span>
                  </button>
                ))}
                {canAddMore && (
                  <button
                    type="button"
                    className="mt-1 w-full rounded border border-dashed border-slate-700 px-2 py-1 text-2xs text-ink-3 hover:border-cyan-500/60 hover:text-cyan-300"
                    onClick={(ev) => setPop({ kind: "add", app: client.app, x: ev.clientX, y: ev.clientY })}
                  >
                    {ts("mm.addCellTip", { label: client.label })}
                  </button>
                )}
              </div>
            </PointPopover>
          );
        })()}
      {/* 模型选择浮层（slot / item / add 共用）：可用模型按后端分组；
          slot = 顶替该槽（带清空）；item = 顶替该清单项（带移出，gemini 唯一位不给移出）；
          add = 追加进清单（实际用哪个在该代理内选择）。 */}
      {pop && pop.kind !== "more" &&
        (() => {
          const client = view.clients.find((x) => x.app === pop.app);
          if (!client) return null;
          let title: string | undefined;
          let current: string | undefined;
          let clear: { label: string; run: () => void } | undefined;
          if (pop.kind === "slot") {
            current = (occByApp[client.app] ?? {})[pop.slot];
            // 顶替语境不再挂「选一个模型顶掉「…」」标题行（用户 2026-11-23）：
            // 每行右侧的「将替换」已经说清了语义，标题只是重复。
            if (current) {
              const held = current;
              clear = { label: t("mm.clearSlot"), run: () => runOps(client.app, [{ model: held, op: "remove", slot: pop.slot }]) };
            }
          } else if (pop.kind === "item") {
            current = pop.model;
            // gemini 唯一模型位没有「移出」可言（后端会拒），只给顶替。
            if (client.capacity !== 1) {
              clear = { label: t("mm.removeListItem"), run: () => runOps(client.app, [{ model: pop.model, op: "remove" }]) };
            }
          } else {
            title = t("mm.joinHint");
          }
          return (
            <PointPopover onClose={() => setPop(null)} point={{ x: pop.x, y: pop.y }}>
              <div className="max-h-72 min-w-[240px] max-w-[340px] overflow-auto">
                {title && <div className="px-2.5 py-1 text-2xs text-ink-4">{title}</div>}
                {/* 清空/移出按钮上移到浮层顶部（用户 2026-11-23）：长列表里不用再滚到底。 */}
                {clear && (
                  <button
                    type="button"
                    onClick={clear.run}
                    className="mb-1 w-full rounded border border-[var(--sem-danger-text)]/50 px-2 py-1 text-2xs text-[var(--sem-danger-text)] hover:bg-red-500/10"
                  >
                    {clear.label}
                  </button>
                )}
                {groups.map((g) => (
                  <div key={g.group}>
                    <div className="px-2.5 pt-1.5 text-2xs text-ink-4">{g.group}</div>
                    {g.rows.map((row) => {
                      const cur = row.model === current;
                      return (
                        <button
                          key={row.model}
                          type="button"
                          disabled={cur || busy}
                          className={cx(
                            "flex w-full items-baseline justify-between gap-2 rounded px-2.5 py-1 text-left",
                            cur ? "text-ink-4" : "text-ink-2 hover:bg-slate-800",
                          )}
                          onClick={() => {
                            if (pop.kind === "slot") {
                              runOps(client.app, [{ model: row.model, op: "add", slot: pop.slot }]);
                            } else if (pop.kind === "item") {
                              const cell = view.rows.find((r) => r.model === pop.model)?.cells[client.app];
                              runOps(client.app, replaceOps(client, cell, pop.model, row.model));
                            } else {
                              // gemini 唯一位：＋ 也走「设为默认」（add 会被后端拒）。
                              runOps(client.app, [{ model: row.model, op: client.capacity === 1 ? "default" : "add" }]);
                            }
                          }}
                        >
                          <span className="break-all font-mono">{row.model}</span>
                          <span className={cx("ml-auto shrink-0 font-sans text-2xs", !cur && "text-ink-4")}>
                            {cur ? t("mm.curModel") : pop.kind === "add" ? "＋" : current ? t("mm.willReplaceShort") : t("mm.assign")}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                ))}
              </div>
            </PointPopover>
          );
        })()}
    </div>
  );
}
