// 单模型行（三行制，用户口径，从 ModelRows 原样搬出）：
// ① 上游模型 id + 暴露开关/摘除；② 暴露别名（原位改名）+ 右对齐常用读数与原位覆盖入口；
// ③ 代理分配灯组。名字一律 break-all 折行而不是 truncate——长 id 被切成
// 「SC117/Ornith-1.5-35B-…」后同族几个模型在眼里一模一样，读不出区别比多占一行贵得多。
// 后端宕机时以前整行 opacity-45（UI-AUDIT-2 A-1）：正文只剩 2.4:1，而宕机时
// 最需要读的就是这行读数；宕机已由父行的 ✕ 状态灯与「最近错误」行说清。
import type { MouseEvent } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { IconButton, Toggle } from "../ui";
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

  return (
    <div className="group/row py-1.5">
      {/* ── 第 1 行：上游模型 id ＋ 右侧开关按钮组 ── */}
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
        <span
          className={`inline-block h-2 w-2 shrink-0 rounded-full ${
            mt?.streaming ? "live-breathe bg-[var(--sw-on)]" : "bg-[var(--dot-idle)]"
          }`}
          title={mt?.streaming ? "数据流：有请求在飞" : "空闲"}
        />
        <span
          className={`min-w-[10rem] break-all font-mono text-xs leading-snug ${
            exposed ? "text-ink-2" : "text-ink-4 line-through"
          }`}
          title={`上游模型 id：${upstream}`}
        >
          {upstream}
        </span>
        <CopyButton
          text={upstream}
          label={`复制上游模型 id：${upstream}`}
          className="opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100"
        />
        {mt?.vlm === "sup" && (
          <span className="shrink-0 rounded bg-emerald-500/15 px-1.5 py-px text-2xs text-[var(--sem-ok-text)]">VLM</span>
        )}
        {mt?.vlm === "unk" && (
          <span className="shrink-0 rounded bg-slate-800 px-1.5 py-px text-xs text-ink-4" title="图文能力未知，默认放行">
            VLM?
          </span>
        )}
        <span className="flex-1" />
        <Toggle
          checked={exposed}
          onChange={(v) =>
            void run(v ? "已暴露" : "已隐藏", () =>
              ipc("set_model_exposed", { name: b.name, model: m, exposed: v }),
            )
          }
        />

        {confirming === m ? (
          <span className="flex shrink-0 items-center gap-1.5 text-xs text-ink-3">
            {b.models.length === 1 ? "删掉最后一个模型将连带删除后端，" : "摘除？"}
            <button
              type="button"
              onClick={() => {
                setConfirming(null);
                void run(
                  b.models.length === 1 ? "已删除最后一个模型，后端随之删除" : "已摘掉该模型",
                  () => ipc("remove_model", { name: b.name, model: m }),
                );
              }}
              className="rounded border border-rose-500/40 bg-rose-500/15 px-1.5 py-0.5 text-[var(--sem-danger-text)] transition-colors hover:bg-rose-500/25"
            >
              确认
            </button>
            <button type="button" onClick={() => setConfirming(null)} className="text-ink-4 hover:text-ink-2">
              取消
            </button>
          </span>
        ) : (
          <IconButton title="摘除该模型" tone="danger" onClick={() => setConfirming(m)}>
            <IconTrash width={13} height={13} />
          </IconButton>
        )}      </div>

      {/* ── 第 2 行：暴露别名（原位改名）＋ 右对齐常用读数与原位覆盖入口 ── */}
      <div className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 pl-[18px]">
        {/* 改名走全站统一的 InlineEdit（UI-AUDIT-2 G-1）：原先这里自己写了一套
            「失焦即落库」，点一下别处就多一次后端写入，与 §7.6 决策 D1 相反。 */}
        <InlineEdit
          value={m}
          className={`min-w-[10rem] break-all font-mono text-xs leading-snug ${
            exposed ? "text-ink-2" : "text-ink-4 line-through"
          }`}
          wrap
          inputClassName="min-w-0 flex-1 font-mono"
          title={`暴露名 ${m}（单击改名：Enter 保存 / Esc 或失焦放弃；客户端用暴露名请求，网关自动改写成上游真名）`}
          ariaLabel={`模型 ${m} 的暴露名`}
          onSave={(to) => void run("已改名", () => ipc("rename_model", { name: b.name, from: m, to }))}
        />
        {/* 暴露名也要能一键拷走（用户口径）：客户端配置里填的是暴露名，
            抄它比抄上游 id 更常用，只给上游配拷贝等于少一半。 */}
        <CopyButton
          text={m}
          label={`复制暴露名（客户端用的名字）：${m}`}
          className="opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100"
        />
        {upstream === m && (
          <span className="shrink-0 text-2xs text-ink-4" title="未改名：暴露名与上游 id 同一个">
            ＝上游
          </span>
        )}
        <span className="flex-1" />
        <span className="shrink-0 text-xs text-ink-4" title="prefill 速率（在飞=实时，空闲=7日均值）">
          pf <b className="font-medium text-ink-3">{mt?.prefill_tps != null ? `~${mt.prefill_tps}` : "—"}</b>
        </span>
        <span className="shrink-0 text-xs text-ink-4" title="decode 速率（tok/s）">
          dec <b className="font-medium text-ink-3">{mt?.decode_tps ?? "—"}</b>
        </span>
        <span
          className="shrink-0 text-xs text-ink-4"
          title={`实际上下文（最近一次请求输入）/ 支持上限${
            metaOf()
              ? ` · 来源：${metaOf()!.ctx_source === "manual" ? "人工声明" : metaOf()!.ctx_source === "backend" ? "后端真值" : "未知"}`
              : ""
          }`}
        >
          ctx <b className="font-medium text-ink-3">
            {fmtTok(mt?.ctx_used)}/{fmtTok(mt?.ctx_max)}
          </b>
          {metaOf()?.ctx_source === "manual" && (
            <span className="ml-1 rounded bg-cyan-500/15 px-1 text-2xs text-cyan-300">手</span>
          )}
          {metaOf()?.price && (
            <span
              className="ml-1 rounded bg-amber-500/15 px-1 text-2xs text-[var(--sem-busy-text)]"
              title="已设手工单价（USD/百万 token，仅在线后端计价）"
            >
              $
            </span>
          )}
        </span>
        <span className="relative shrink-0">
          <button
            type="button"
            onClick={() => setMetaFor(metaFor === m ? null : m)}
            title="声明上下文/输出上限（0 = 自动取后端真值）"
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
                <QueryError inline title="元数据读取失败" error={metaQ.error} onRetry={() => metaQ.refetch()} />
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
          title="覆盖参数"
          className="relative shrink-0 rounded px-1.5 py-px text-xs text-ink-4 hover:bg-slate-800 hover:text-ink-2"
        >
          <IconPencil width={13} height={13} className="inline-block align-[-2px]" />
          参数
          {mine.length > 0 && (
            <span className="absolute -right-1 -top-1.5 rounded bg-cyan-500/70 px-1 text-2xs text-slate-950">
              {mine.length}
            </span>
          )}
        </button>
      </div>

      {/* ── 第 3 行：代理分配灯组（点灯就地弹槽位浮层分配；未接管退回跳矩阵） ── */}
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
          title="有非接管矩阵的下游客户端直连统一端口调用过该模型（近 7 天，仅显示不可操作）"
        >
          其他
        </span>
      </div>
    </div>
  );
}
