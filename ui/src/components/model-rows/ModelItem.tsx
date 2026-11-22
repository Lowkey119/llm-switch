// 单模型行（backend-models 原型回灌 + 两行定稿，用户 2026-11-22）：每个模型**恰好两行**——
// ① 身份行：模型名（原位双击改名）+ 身份标签（上游 id / 别名 / 工种 / VLM——VLM 只挂 MoE）
//    + 常用参数读数 temp·惩罚·ctx·max（继承链生效值，定宽右对齐成列；**双击原位录入**，
//    写模型级覆盖：覆盖请求 > 此处设置 > 代理中的配置 > 后端全局）+ 参数入口 + 开关 + 删除。
// ② 分配行：工种声明下拉打头（不再挤在身份行尾折成第三行），后接「已分配代理 n」
//    悬停明细（× 就地取消）+「分配给代理」下拉（档位端二级槽浮层、清单端一点即分配，
//    复用矩阵同源 chipClick）；多媒体只暴露不分配（一行说明）。
// 名字一律 break-all 折行而不是 truncate——长 id 被切成「SC117/Ornith-1.5-35B-…」后
// 同族几个模型在眼里一模一样，读不出区别比多占一行贵得多。
// 后端宕机时以前整行 opacity-45（UI-AUDIT-2 A-1）：正文只剩 2.4:1，而宕机时
// 最需要读的就是这行读数；宕机已由父行的 ✕ 状态灯与「最近错误」行说清。
import { useState, type MouseEvent } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { IconButton, StatusDot, Toggle } from "../ui";
import { QueryError } from "../QueryError";
import { IconCheck, IconPencil, IconTrash } from "../icons";
import { CopyButton } from "../CopyButton";
import { InlineEdit } from "../InlineEdit";
import { PointPopover } from "../Popover";
import {
  ipc,
  type BackendRow,
  type MatrixCell,
  type MatrixClient,
  type ModelMeta,
  type ModelMetric,
} from "../../lib/ipc";
import type { ParamRow } from "../../lib/params";
import { t, ts } from "../../lib/i18n";
import { isMediaKind, WORK_ORDER, type WorkKind } from "../../lib/workkind";
import { fmtTok } from "./fmt";
import { MetaDialog } from "./MetaDialog";
import { cellSlots, slotLabel } from "./alloc";

type MetaQuery = UseQueryResult<ModelMeta[], Error>;

/** 该端是否占着这个模型：占槽 / 默认 / 在清单（与矩阵口径一致）。 */
const occupiedCell = (c?: MatrixCell) => !!c && (cellSlots(c).length > 0 || c.in_list);

export function ModelItem({
  b,
  m,
  work,
  paramRows,
  metrics,
  cells,
  clients,
  occ,
  showVlm,
  metaQ,
  metaFor,
  setMetaFor,
  confirming,
  setConfirming,
  run,
  saveMeta,
  chipClick,
  onOpenParams,
}: {
  b: BackendRow;
  m: string;
  /** 工种类（父级按 lib/workkind 判定，与路由同口径）。 */
  work: WorkKind;
  paramRows: ParamRow[];
  metrics: Record<string, ModelMetric>;
  cells: Record<string, Record<string, MatrixCell>>;
  /** 矩阵列（app → 端定义）：「分配给代理」下拉的行集。 */
  clients: MatrixClient[];
  /** 占用表（app → slot → 模型）：下拉里标「将替换」。 */
  occ: Record<string, Record<string, string>>;
  /** VLM 徽标只在 MoE 子分组出现（用户 2026-11-22：聊天模型不打 VLM?）。 */
  showVlm: boolean;
  metaQ: MetaQuery;
  metaFor: string | null;
  setMetaFor: (m: string | null) => void;
  confirming: string | null;
  setConfirming: (m: string | null) => void;
  /** 通用操作包装：成功 toast + onChanged（口径在 ModelRows）。 */
  run: (msg: string, fn: () => Promise<unknown>) => Promise<void>;
  saveMeta: (
    model: string,
    ctx: number,
    output: number,
    reasoning: number,
    vlm: number,
    price?: {
      input: number | null;
      output: number | null;
      cache_read: number | null;
      cache_creation: number | null;
    },
  ) => Promise<void>;
  chipClick: (app: string, model: string, cell: MatrixCell | undefined, e: MouseEvent) => void;
  onOpenParams: (model: string) => void;
}) {
  const flag = b.model_flags?.find((f) => f.public_name === m);
  const exposed = flag?.exposed ?? true;
  const declared = flag?.kind?.trim() ?? "";
  const media = isMediaKind(work);
  const showChip = media || declared !== "";
  const upstream = flag?.upstream_name ?? m;
  const mt = metrics[m];
  const mc = cells[m] ?? {};
  const metaOf = () => metaQ.data?.find((x) => x.model === m);
  const renamed = upstream !== m;
  // 「已分配代理 n」悬停明细 / 「分配给代理」下拉（点击点弹层）。
  const [hoverPop, setHoverPop] = useState(false);
  const [assignPop, setAssignPop] = useState<{ x: number; y: number } | null>(null);
  const assigned = clients.filter((c) => occupiedCell(mc[c.app]));

  /** 常用参数读数的生效值：模型 > 全局 > 后端（与 buildChains 同口径的极简版——
   *  只查这三个键，0 视为未配置走下一层；全没配 = null 显「—」走后端默认）。 */
  const effParam = (key: string) => {
    const ref = `${b.name}:${m}`;
    const at = (scope: string, refId: string) =>
      paramRows.find((r) => r.scope === scope && r.ref_id === refId && r.key === key);
    const alive = (r?: ParamRow) => (r && r.value !== 0 && r.value !== "0" ? r : undefined);
    const win = alive(at("model", ref)) ?? alive(at("global", "")) ?? alive(at("backend", b.name));
    return win ? String(win.value) : null;
  };

  /** 读数双击原位录入：写模型级参数覆盖（policy=fill，与参数弹窗同一条 upsert）。
   *  生效链：覆盖请求 > 此处（模型）> 全局 > 后端；删覆盖仍去参数弹窗。 */
  const setParam = (key: string, v: string) => {
    const n = Number(v);
    return run(ts("pw.saved", { k: key }), () =>
      ipc("upsert_param_override", {
        input: {
          scope: "model",
          ref_id: `${b.name}:${m}`,
          key,
          value: v !== "" && Number.isFinite(n) ? n : v,
          policy: "fill",
        },
      }),
    );
  };

  /** 读数单元：定宽标签 + 双击可改的生效值（— = 未配置走后端默认）。 */
  const roCell = (label: string, key: string, width: string) => (
    <span className="hidden shrink-0 items-center text-sm lg:flex">
      <span className={`${width} text-ink-4`}>{label}</span>
      <InlineEdit
        value={effParam(key) ?? ""}
        display={
          <b className={`font-medium tabular-nums ${effParam(key) ? "text-ink-3" : "text-ink-4"}`}>
            {effParam(key) ?? "—"}
          </b>
        }
        editOnDouble
        inputClassName="w-20"
        title={t("row.paramEdit")}
        ariaLabel={ts("row.paramEditAria", { key })}
        onSave={(v) => void setParam(key, v)}
      />
    </span>
  );

  // 右侧读数（temp/惩罚/ctx/max + ctx 声明、参数入口）：定宽 + tabular-nums，
  // 跨行数字纵向成列（用户 2026-11-18 的读数对齐口径不变）；窄窗先藏 temp/惩罚/max 保 ctx。
  // 媒体模型没有 token/上下文/采样语义：整排读数不给（媒体端点透传代理 11d）。
  const metricsEl = b.direct ? (
    /* 直连后端不经网关，读数显示 0 或 — 都是误导（计划决策 5）：指标位走空态文案；
       「查看直连地址」动作在行头那枚直连徽标上，地址是后端级的，不在每个模型行重复挂拷贝。 */
    <span className="ml-auto shrink-0 text-2xs text-ink-4">{t("row.directEmpty")}</span>
  ) : !media ? (
    <span className="ml-auto flex flex-wrap items-center gap-x-2.5 gap-y-1">
      {roCell("temp", "temperature", "w-[3.2rem]")}
      {roCell(t("row.penWord"), "repetition_penalty", "w-[3.2rem]")}
      <span
        className="shrink-0 text-sm text-ink-4"
        title={
          t("row.ctxTip") +
          (metaOf()
            ? ts("row.ctxSrc", {
                s: t(
                  metaOf()!.ctx_source === "manual"
                    ? "row.srcManual"
                    : metaOf()!.ctx_source === "backend"
                      ? "row.srcBackend"
                      : "row.srcUnknown",
                ),
              })
            : "")
        }
      >
        ctx <b className="font-medium tabular-nums text-ink-3">
          {fmtTok(mt?.ctx_used)}/{fmtTok(mt?.ctx_max)}
        </b>
        {metaOf()?.ctx_source === "manual" && (
          <span className="ml-1 rounded bg-cyan-500/15 px-1 text-2xs text-cyan-300">{t("row.badgeManual")}</span>
        )}
        {metaOf()?.price && (
          <span
            className="ml-1 rounded bg-amber-500/15 px-1 text-2xs text-[var(--sem-busy-text)]"
            title={t("row.priceTip")}
          >
            $
          </span>
        )}
      </span>
      {roCell("max", "max_tokens", "w-[2.4rem]")}
      <span className="relative shrink-0">
        <button
          type="button"
          onClick={() => setMetaFor(metaFor === m ? null : m)}
          title={t("row.declareTip")}
          className={`rounded px-1 py-px text-2xs hover:bg-slate-800 ${
            metaFor === m ? "bg-slate-800 text-ink-2" : "text-ink-4 hover:text-ink-2"
          }`}
        >
          <IconPencil width={12} height={12} className="inline-block align-[-2px]" />
          ctx
        </button>
        {metaFor === m &&
          // 读取失败时别打开浮层：浮层拿不到 meta 会显示全 0，一保存就把
          // 用户手工声明的 ctx/单价覆盖成 0——失败必须先摆出来再操作。
          (metaQ.isError ? (
            <span className="absolute right-0 top-6 z-20 whitespace-nowrap rounded-md border border-rose-500/40 bg-slate-900 px-2 py-1">
              <QueryError inline title={t("row.metaReadFail")} error={metaQ.error} onRetry={() => metaQ.refetch()} />
            </span>
          ) : (
            <MetaDialog
              meta={metaOf()}
              onSave={(ctx, output, reasoning, vlm, price) =>
                void saveMeta(m, ctx, output, reasoning, vlm, price)
              }
              onCancel={() => setMetaFor(null)}
            />
          ))}
      </span>
      <button
        type="button"
        onClick={() => onOpenParams(m)}
        title={t("row.overrideParams")}
        className="relative shrink-0 rounded px-1.5 py-px text-sm text-ink-4 hover:bg-slate-800 hover:text-ink-2"
      >
        <IconPencil width={13} height={13} className="inline-block align-[-2px]" />
        {t("row.paramsWord")}
        {paramRows.filter((r) => r.scope === "model" && r.ref_id === `${b.name}:${m}`).length > 0 && (
          <span className="absolute -right-1 -top-1.5 rounded bg-cyan-500/70 px-1 text-2xs text-slate-950">
            {paramRows.filter((r) => r.scope === "model" && r.ref_id === `${b.name}:${m}`).length}
          </span>
        )}
      </button>
    </span>
  ) : null;

  // 工种声明下拉（分配行打头）：人工声明 > 名字启发式（lib/workkind），与路由同口径。
  const kindSelect = (
    <select
      value={declared}
      title={t("wk.setKindTitle")}
      aria-label={`${m} ${t("wk.setKindTitle")}`}
      onChange={(e) =>
        void run(t("wk.kindSet"), () =>
          ipc("set_model_kind", { name: b.name, model: m, kind: e.target.value || null }),
        )
      }
      className="shrink-0 rounded border border-slate-700 bg-slate-900 px-1 py-px text-2xs text-ink-3"
    >
      <option value="">{t("wk.auto")}</option>
      {WORK_ORDER.map((k) => (
        <option key={k} value={k}>
          {t(`wk.${k}`)}
        </option>
      ))}
    </select>
  );

  return (
    <div className="group/row py-1.5">
      {/* ── 第 1 行：身份 + 读数 + 开关 ──
          两列制（用户 2026-11-20 截图：单行 flex-wrap 时名字一长，ml-auto 的读数
          整坨悬到第二行右端）：左列名字+标签自由换行，右列读数/开关/删除钉右不换行。 */}
      <div className="flex items-start gap-x-2.5">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2.5 gap-y-1">
        {/* U10：原先是一颗会呼吸的绿点 + title，而 WKWebView 不渲染 title——
            颜色之外没有第二条路。换成状态灯（◐ 呼吸 / ○ 空闲）+ 读屏文案。 */}
        <StatusDot
          busy={!!mt?.streaming}
          text={mt?.streaming ? t("row.streaming") : t("row.idle")}
        />
        {/* 名字永远原位可编辑（用户 2026-11-19 拍板单行制）：没改名时显示的就是上游 id，
            双击进编辑；后方标签区分身份——未改「上游 id」、已改「别名」（客户端用暴露名请求，
            网关自动改写成上游真名）。不再两行各显示一遍。 */}
        <InlineEdit
          value={m}
          editOnDouble
          className={`min-w-[10rem] break-all font-mono text-xs leading-snug ${
            exposed ? "text-ink-2" : "text-ink-4 line-through"
          }`}
          wrap
          inputClassName="min-w-[10rem] flex-1 font-mono"
          title={
            renamed
              ? ts("row.renameTip", { m, u: upstream })
              : ts("row.upstreamTip", { m })
          }
          ariaLabel={ts("row.exposedNameAria", { m })}
          onSave={(to) => void run(t("row.renamed"), () => ipc("rename_model", { name: b.name, from: m, to }))}
        />
        <span
          className="shrink-0 rounded bg-slate-700/60 px-1.5 py-px text-2xs text-ink-3"
          title={renamed ? ts("row.tagAliasTip", { u: upstream }) : t("row.tagUpstreamTip")}
        >
          {renamed ? t("row.tagAlias") : t("row.tagUpstream")}
        </span>
        <CopyButton
          text={m}
          label={renamed ? ts("row.copyAlias", { m, u: upstream }) : ts("row.copyUpstream", { m })}
          className="opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100"
        />
        {showChip && (
          <span
            className="shrink-0 rounded bg-slate-700/60 px-1.5 py-px text-2xs text-ink-3"
            title={declared ? t("wk.setKindTitle") : t("wk.setKindTitle") + t("wk.fromName")}
          >
            {t(`wk.${work}`)}
            {!declared && media && t("wk.suggested")}
          </span>
        )}
        {/* VLM 徽标只挂 MoE 子分组（用户 2026-11-22：聊天模型不打 VLM?，那是 MoE 的事）。 */}
        {showVlm && mt?.vlm === "sup" && (
          <span className="shrink-0 rounded bg-emerald-500/15 px-1.5 py-px text-2xs text-[var(--sem-ok-text)]">VLM</span>
        )}
        {showVlm && mt?.vlm === "unk" && (
          <span className="shrink-0 rounded bg-slate-800 px-1.5 py-px text-sm text-ink-4" title={t("row.vlmUnkTip")}>
            VLM?
          </span>
        )}
        </div>
        {/* 右列：读数 + 开关 + 删除，钉右不换行。 */}
        <span className="flex shrink-0 items-center gap-x-2.5">
        {metricsEl}
        <Toggle
          checked={exposed}
          onChange={(v) =>
            void run(v ? t("row.exposed") : t("row.hidden"), () =>
              ipc("set_model_exposed", { name: b.name, model: m, exposed: v }),
            )
          }
        />

        {confirming === m ? (
          <span className="flex shrink-0 items-center gap-1.5 text-sm text-ink-3">
            {b.models.length === 1 ? t("row.delLastWarn") : t("row.removeQ")}
            <button
              type="button"
              onClick={() => {
                setConfirming(null);
                void run(
                  b.models.length === 1 ? t("row.delLastDone") : t("row.removedModel"),
                  () => ipc("remove_model", { name: b.name, model: m }),
                );
              }}
              className="rounded border border-rose-500/40 bg-rose-500/15 px-1.5 py-0.5 text-[var(--sem-danger-text)] transition-colors hover:bg-rose-500/25"
            >
              {t("common.confirm")}
            </button>
            <button type="button" onClick={() => setConfirming(null)} className="text-ink-4 hover:text-ink-2">
              {t("dlg.cancel")}
            </button>
          </span>
        ) : (
          <IconButton title={t("row.removeTip")} tone="danger" onClick={() => setConfirming(m)}>
            <IconTrash width={13} height={13} />
          </IconButton>
        )}
        </span>
      </div>

      {/* ── 第 2 行：工种声明 + 分配汇总/下拉（backend-models 原型定稿：每模型恰好两行，
          工种下拉不再挤在身份行尾折成第三行）──
          「已分配代理 n」悬停看明细（代理 · 槽名，× 就地取消分配）；「分配给代理」下拉选端——
          档位端二级槽浮层、清单端一点即设当前，全部复用矩阵同源 chipClick/AllocPopover。
          fix:exposed-only-allocation：只有开了暴露开关的模型才可访问（用户 2026-11-17），
          未暴露整组入口不给。媒体工种永不进分配（工种类互斥在后端焊死），一行说明替代。 */}
      {media ? (
        <div className="mt-1 flex flex-wrap items-center gap-2 pl-[18px]">
          {kindSelect}
          <p className="text-2xs text-ink-4">{ts("wk.mediaNote", { kind: t(`wk.${work}`) })}</p>
        </div>
      ) : exposed ? (
        <div className="mt-1 flex flex-wrap items-center gap-2 pl-[18px]">
          {kindSelect}
          {assigned.length > 0 && (
            <span
              className="relative"
              onMouseEnter={() => setHoverPop(true)}
              onMouseLeave={() => setHoverPop(false)}
            >
              <button type="button" className="rounded border border-slate-700 px-2 py-px text-2xs text-ink-3 hover:border-slate-600 hover:text-ink-2">
                {ts("row.assignN", { n: assigned.length })}
              </button>
              {hoverPop && (
                <div className="absolute left-0 top-6 z-30 w-64 rounded-md border border-slate-700 bg-slate-900 p-1.5 shadow-xl">
                  <div className="break-all px-1.5 pb-1 text-2xs text-ink-4">{ts("row.assignedPop", { m })}</div>
                  {assigned.map((c) => {
                    const cell = mc[c.app];
                    const slots = cellSlots(cell);
                    return (
                      <div key={c.app} className="flex items-baseline gap-2 rounded px-1.5 py-1 text-2xs hover:bg-slate-800">
                        <span className="shrink-0 text-ink-2">{c.label}</span>
                        <span className="truncate text-ink-4">
                          {slots.length
                            ? slots.map((s) => slotLabel(c, s)).join("/")
                            : cell?.is_default
                              ? t("ap.defaultTier")
                              : t("row.inList")}
                        </span>
                        {/* gemini 唯一模型位没有「移出」可言（后端会拒），只给顶替。 */}
                        {c.capacity !== 1 && (
                          <button
                            type="button"
                            className="ml-auto shrink-0 px-1 text-ink-4 hover:text-[var(--sem-danger-text)]"
                            title={ts("row.unassignTip", { app: c.label })}
                            onClick={() =>
                              void run(ts("row.unassigned", { app: c.label }), () =>
                                ipc("matrix_cell", { app: c.app, model: m, op: "remove", slot: null }),
                              )
                            }
                          >
                            ×
                          </button>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}
            </span>
          )}
          <button
            type="button"
            className="rounded border border-dashed border-slate-600 px-2 py-px text-2xs text-ink-3 hover:border-cyan-500/60 hover:text-cyan-300"
            onClick={(e) => setAssignPop({ x: e.clientX, y: e.clientY })}
          >
            {t("row.assignBtn")}
          </button>
          {/* 「其他」：只读指示灯（该端在配置里手工引用了这模型，不经接管清单）。 */}
          <span
            className={`rounded border border-dashed px-2 py-px text-2xs ${
              mt?.other_active
                ? "border-[var(--sem-ok-text)] text-[var(--sem-ok-text)]"
                : "border-slate-700 text-ink-4"
            }`}
            title={t("row.otherTip")}
          >
            {t("row.other")}
          </span>
        </div>
      ) : (
        <div className="mt-1 flex flex-wrap items-center gap-2 pl-[18px]">
          {kindSelect}
          <p className="text-2xs text-ink-4">{t("row.unexposedNote")}</p>
        </div>
      )}

      {/* 「分配给代理」下拉：全部端一行一家（✓ = 该模型已分给它；将替换 = 该端现用什么）。
          点行即走矩阵同源 chipClick：档位端二级槽浮层 / gemini 直设 / 清单端就地设当前。 */}
      {assignPop && (
        <PointPopover onClose={() => setAssignPop(null)} point={assignPop}>
          <div className="max-h-72 min-w-[240px] overflow-auto">
            <div className="break-all px-2.5 py-1 text-2xs text-ink-4">{ts("row.assignPop", { m })}</div>
            {clients.map((c) => {
              const mine = occupiedCell(mc[c.app]);
              const cur = occ[c.app]?.main;
              return (
                <button
                  key={c.app}
                  type="button"
                  className="flex w-full items-baseline gap-2 rounded px-2.5 py-1 text-left text-2xs hover:bg-slate-800"
                  onClick={(e) => {
                    setAssignPop(null);
                    chipClick(c.app, m, mc[c.app], e);
                  }}
                >
                  <span className="inline-flex shrink-0 items-center gap-1 text-ink-2">
                    {mine && <IconCheck className="h-3 w-3 text-[var(--sem-ok-text)]" />}
                    {c.label}
                  </span>
                  {cur && cur !== m && (
                    <span className="truncate text-ink-4">{ts("ap.willReplace", { holder: cur })}</span>
                  )}
                  <span className="ml-auto shrink-0 tabular-nums text-ink-4">
                    {c.capacity != null ? `${c.used}/${c.capacity}` : ""}
                  </span>
                </button>
              );
            })}
          </div>
        </PointPopover>
      )}
    </div>
  );
}
