// 单模型行（单行制，用户 2026-11-19 拍板）：
// ① 模型名（原位双击改名）+ 身份标签（上游 id / 别名）+ 复制 + 工种 + 右对齐读数与开关；
// ② 代理分配灯组。名字一律 break-all 折行而不是 truncate——长 id 被切成
// 「SC117/Ornith-1.5-35B-…」后同族几个模型在眼里一模一样，读不出区别比多占一行贵得多。
// 后端宕机时以前整行 opacity-45（UI-AUDIT-2 A-1）：正文只剩 2.4:1，而宕机时
// 最需要读的就是这行读数；宕机已由父行的 ✕ 状态灯与「最近错误」行说清。
import type { MouseEvent } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { IconButton, StatusDot, Toggle } from "../ui";
import { QueryError } from "../QueryError";
import { IconPencil, IconTrash } from "../icons";
import { CopyButton } from "../CopyButton";
import { InlineEdit } from "../InlineEdit";
import {
  ipc,
  type BackendRow,
  type MatrixCell,
  type ModelMeta,
  type ModelMetric,
} from "../../lib/ipc";
import type { ParamRow } from "../../lib/params";
import { t, ts } from "../../lib/i18n";
import { isMediaKind, WORK_ORDER, type WorkKind } from "../../lib/workkind";
import { fmtTok } from "./fmt";
import { AllocLight } from "./AllocLight";
import { MetaDialog } from "./MetaDialog";
import { cellSlots } from "./alloc";

/** 单主模型代理（矩阵里各占一个模型位；后端页只读灯）。gemini/copilot 是特例，单独处理。 */
const SINGLE_AGENTS = ["codex", "opencode", "pi", "deveco"] as const;

type MetaQuery = UseQueryResult<ModelMeta[], Error>;

export function ModelItem({
  b,
  m,
  work,
  paramRows,
  metrics,
  cells,
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
  // 工种类展示口径：人工声明（chip 干净）vs 名字启发式（chip 标「建议」）；
  // 聊天且未声明不打扰（缺省就是聊天）。
  const declared = flag?.kind?.trim() ?? "";
  const media = isMediaKind(work);
  const showChip = media || declared !== "";
  // 上游真名：没改名时与暴露名同一个（第 2 行据此标「＝上游」，不让人以为漏了什么）。
  const upstream = flag?.upstream_name ?? m;
  const mine = paramRows.filter((r) => r.scope === "model" && r.ref_id === `${b.name}:${m}`);
  const mt = metrics[m];
  const mc = cells[m] ?? {};
  const metaOf = () => metaQ.data?.find((x) => x.model === m);
  // 一个模型现在能占多个槽：分配灯显示全部槽位，别只报第一个。
  const slotsOf = (c?: MatrixCell) => cellSlots(c).join("/");
  const claudeSlot = slotsOf(mc.claude);
  const cbSlot = slotsOf(mc.codebuddy);
  // 未改名（暴露名 === 上游 id）时两行并一行（用户 2026-11-19 截图拍板）：
  // 名字原位可改名，不再两行各显示一遍。改名后恢复两行（上游真名 + 暴露名）。
  const renamed = upstream !== m;
  // 右侧读数（pf/dec/ctx + 声明/参数入口）：两行制与合并行共用一份，ml-auto 靠右。
  // 媒体模型没有 token/上下文语义：pf/dec/ctx 与 ctx 声明、采样参数入口都不给，
  // 一排「—」只会添噪（媒体端点透传代理 11d）。
  const metricsEl = b.direct ? (
    /* 直连后端不经网关，pf/dec/ctx 显示 0 或 — 都是误导（计划决策 5）：
       指标位走空态文案（计划定稿）；「查看直连地址」动作在行头那枚直连徽标上，
       地址是后端级的，不在每个模型行重复挂拷贝。 */
    <span className="ml-auto shrink-0 text-2xs text-ink-4">{t("row.directEmpty")}</span>
  ) : !media ? (
    <span className="ml-auto flex flex-wrap items-center gap-x-2.5 gap-y-1">
      {/* 窄窗先丢 pf/dec 保住 ctx（与顶栏「今日 token」同一降级口径）；
          读数一律 tabular-nums，上下行的数字才能纵向对齐（用户 2026-11-18）。 */}
      <span className="hidden shrink-0 text-sm text-ink-4 lg:inline" title={t("row.pfTip")}>
        pf <b className="font-medium tabular-nums text-ink-3">{mt?.prefill_tps != null ? `~${mt.prefill_tps}` : "—"}</b>
      </span>
      <span className="hidden shrink-0 text-sm text-ink-4 lg:inline" title={t("row.decTip")}>
        dec <b className="font-medium tabular-nums text-ink-3">{mt?.decode_tps ?? "—"}</b>
      </span>
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
        {mine.length > 0 && (
          <span className="absolute -right-1 -top-1.5 rounded bg-cyan-500/70 px-1 text-2xs text-slate-950">
            {mine.length}
          </span>
        )}
      </button>
    </span>
  ) : null;

  return (
    <div className="group/row py-1.5">
      {/* ── 第 1 行：上游模型 id ＋ 右侧开关按钮组 ──
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
        {!media && mt?.vlm === "sup" && (
          <span className="shrink-0 rounded bg-emerald-500/15 px-1.5 py-px text-2xs text-[var(--sem-ok-text)]">VLM</span>
        )}
        {!media && mt?.vlm === "unk" && (
          <span className="shrink-0 rounded bg-slate-800 px-1.5 py-px text-sm text-ink-4" title={t("row.vlmUnkTip")}>
            VLM?
          </span>
        )}
        </div>
        {/* 右列：读数 + 开关 + 删除，钉右不换行（ml-auto 在两列制下已无意义，留着无害）。 */}
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

      {/* ── 第 2 行：代理分配灯组（点灯就地弹槽位浮层分配；未接管也就地分配，写下去即自动接管） ──
          fix:exposed-only-allocation：只有后端里**开了暴露开关**的模型才可访问（用户 2026-11-17）。
          没开开关的模型客户端根本取不到，旧行为却照样给一排能点的灯——一路点到后端才报
          「未暴露，无法分配」，先允许再拒绝是错的顺序，整组入口直接不给。
          媒体工种（11d）：永不进分配格子（工种类互斥在后端焊死），暴露开关语义分叉为
          「进不进该工种端点的可路由清单」，一行说明替代灯组。 */}
      {media ? (
        <p className="mt-1 pl-[18px] text-2xs text-ink-4">
          {ts("wk.mediaNote", { kind: t(`wk.${work}`) })}
        </p>
      ) : exposed ? (
        <div className="mt-1 flex flex-wrap items-center gap-1.5 pl-[18px]">
          <AllocLight
            app="claude"
            lit={!!claudeSlot}
            slot={claudeSlot}
            onClick={(e) => chipClick("claude", m, mc.claude, e)}
          />
          <AllocLight
            app="codebuddy"
            lit={!!cbSlot}
            slot={cbSlot}
            onClick={(e) => chipClick("codebuddy", m, mc.codebuddy, e)}
          />

          {SINGLE_AGENTS.map((app) => {
            const c = mc[app];
            return (
              <AllocLight
                key={app}
                app={app}
                lit={!!c?.in_list}
                // 双槽端（pi 主/小、codex 主/副）显示矩阵里的真实占槽；无槽的显示 main。
                slot={slotsOf(c) || (c?.in_list ? "main" : undefined)}
                onClick={(e) => chipClick(app, m, c, e)}
              />
            );
          })}

          {/* gemini：只有一个模型位（点灯直设为它的模型） */}
          <AllocLight
            app="gemini"
            lit={!!mc.gemini?.is_default}
            onClick={(e) => chipClick("gemini", m, mc.gemini, e)}
          />

          {/* copilot：网关门面可见清单（点灯直切成员） */}
          <AllocLight
            app="copilot"
            lit={!!mc.copilot?.in_list}
            onClick={(e) => chipClick("copilot", m, mc.copilot, e)}
          />

          {/* 「其他」：只读指示灯 */}
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
        <p className="mt-1 pl-[18px] text-2xs text-ink-4">
          {t("row.unexposedNote")}
        </p>
      )}
    </div>
  );
}
