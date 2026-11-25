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
import { useState, type CSSProperties, type MouseEvent } from "react";
import type { UseQueryResult } from "@tanstack/react-query";
import { IconButton, StatusDot, Toggle } from "../ui";
import { QueryError } from "../QueryError";
import { IconPencil, IconTrash } from "../icons";
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
import { isMediaKind, type WorkKind } from "../../lib/workkind";
import { fmtTok } from "./fmt";
import { MetaDialog } from "./MetaDialog";
import { AgentChips } from "./AgentChips";

type MetaQuery = UseQueryResult<ModelMeta[], Error>;

export function ModelItem({
  b,
  m,
  work,
  paramRows,
  metrics,
  cells,
  clients,
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
  /** 矩阵列（app → 端定义）：代理灯牌带的行集。 */
  clients: MatrixClient[];
  /** VLM 徽标只在 MoE 子分组出现（用户 2026-11-22：文本模型不打 VLM?）。 */
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
  const media = isMediaKind(work);
  const upstream = flag?.upstream_name ?? m;
  const mt = metrics[m];
  const mc = cells[m] ?? {};
  const metaOf = () => metaQ.data?.find((x) => x.model === m);
  const renamed = upstream !== m;
  // 显示名切换（用户 11-23 拍板）：改过名的行点「别名」标签，主显示位在上游 id ↔
  // 暴露名间循环；没改过名的没得切。编辑/复制/改名永远按暴露名（m）走，不受显示影响。
  const [showUpstream, setShowUpstream] = useState(false);
  const displayName = renamed && showUpstream ? upstream : m;
  // 提示文案（媒体不分配 / 未暴露）不再占行：挂到工种下拉的悬停提示上（用户 2026-11-23）。
  const [notePop, setNotePop] = useState<{ x: number; y: number } | null>(null);
  const note = media
    ? ts("wk.mediaNote", { kind: t(`wk.${work}`) })
    : !exposed
      ? t("row.unexposedNote")
      : null;

  // 第 2 列读数（用户 2026-11-23 终拍）：只留 ctx 一项——
  // 「手动输入的上下文 / 原生上下文支持长度」（manual_ctx / cap_ctx），
  // temp·pen·max 取消显示（要改走「参数」入口，参数弹窗本来就是全量口径）。
  // 媒体模型没有 token/上下文语义：整排读数不给（媒体端点透传代理 11d）。
  const readouts = b.direct ? (
    /* 直连后端不经网关，读数显示 0 或 — 都是误导（计划决策 5）：指标位走空态文案；
       「查看直连地址」动作在行头那枚直连徽标上，地址是后端级的，不在每个模型行重复挂拷贝。 */
    <span className="text-2xs text-ink-4">{t("row.directEmpty")}</span>
  ) : !media ? (
    <span className="flex items-center">
      {/* ctx 单元定宽（w-28）：读数列整体定宽（--model-readouts-w），跨行不漂移。
          单行显示（用户 11-23 终拍：删 ctx 标签、双行改单行）——
          左值 = 人工声明的上下文（0/未设 = 「—」走自动），右值 = 后端原生支持上限，
          含义走悬停 title。 */}
      <span
        className="flex w-28 shrink-0 items-center gap-1 text-ink-4"
        title={
          t("row.ctxManualTip") +
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
        <b className="text-xs font-medium tabular-nums text-ink-3">
          {metaOf()?.manual_ctx ? fmtTok(metaOf()!.manual_ctx) : "—"}
          /{fmtTok(metaOf()?.cap_ctx ?? null)}
        </b>
        {metaOf()?.price && (
          <span
            className="shrink-0 rounded bg-amber-500/15 px-1 text-2xs text-[var(--sem-busy-text)]"
            title={t("row.priceTip")}
          >
            $
          </span>
        )}
      </span>
    </span>
  ) : null;

  // 第 3 列动作区：ctx 声明入口、参数入口、暴露开关、删除。
  const actions = (
    <span className="flex shrink-0 items-center gap-x-2.5">
      {!b.direct && !media && (
        <>
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
        </>
      )}
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
  );

  // 工种徽标 = 纯展示（用户 2026-11-23 终拍：完全自动识别，取消人工切换覆盖）：
  // 工种一律由名字启发式判定（lib/workkind，与后端 request_kind 同口径），
  // 徽标只把判定结果说出来，不可点。媒体/未暴露的说明文案不占行：
  // 悬停徽标即弹提示（WKWebView 不渲染 title，必须真浮层）。
  const kindBadge = (
    <span
      className="relative shrink-0"
      onMouseEnter={(e) => note && setNotePop({ x: e.clientX, y: e.clientY })}
      onMouseLeave={() => setNotePop(null)}
    >
      <span
        title={t("wk.autoOnlyTitle")}
        className="wk-badge w-14 shrink-0 rounded px-1.5 py-px text-center text-2xs"
      >
        {t(`wk.${work}`)}
      </span>
      {notePop && note && (
        <PointPopover onClose={() => setNotePop(null)} point={notePop}>
          <p className="max-w-[22rem] px-2 py-1 text-2xs text-ink-3">{note}</p>
        </PointPopover>
      )}
    </span>
  );

  // VLM 徽标只挂 MoE 子分组（用户 2026-11-22：聊天模型不打 VLM?，那是 MoE 的事）。
  // 槽位恒占宽（w-10）：没有 VLM 的行留空位，跨行徽标才完整对齐（用户 11-23）。
  const vlmBadge =
    showVlm && mt?.vlm === "sup" ? (
      <span className="w-10 shrink-0 rounded bg-emerald-500/15 px-1 py-px text-center text-2xs text-[var(--sem-ok-text)]">
        VLM
      </span>
    ) : showVlm && mt?.vlm === "unk" ? (
      <span className="w-10 shrink-0 rounded bg-slate-800 px-1 py-px text-center text-2xs text-ink-4" title={t("row.vlmUnkTip")}>
        VLM?
      </span>
    ) : (
      <span className="w-10 shrink-0" aria-hidden />
    );

  return (
    <div
      className="wk-row group/row py-1.5"
      style={{ "--wk": `var(--wk-${work})` } as CSSProperties}
    >
      {/* ── 第 1 行：三列网格（用户 2026-11-23 拍板）──
          ① 身份（灯 + 名字 + 标签 + 工种下拉 + 徽标）② 参数读数 ③ 动作（ctx/参数/开关/删除）。
          三列左右对齐成网格、等分自适应窗宽（styles.css .model-row）。
          名字过长截断 +…（title 挂全称，双击进编辑时输入框里也是全称）。 */}
      <div className="model-row">
        {/* 第 1 列（用户 2026-11-23 终拍）：状态灯 + 模型名左对齐（左缘成一条竖线，
            名字过长截断 +…），其余标识（别名/上游标签、复制、工种徽标、VLM）整组推到
            列尾右对齐；每个标识定宽，跨行逐列完整对齐。 */}
        <div className="flex min-w-0 items-center gap-x-2.5">
          {/* U10：原先是一颗会呼吸的绿点 + title，而 WKWebView 不渲染 title——
              颜色之外没有第二条路。换成状态灯（◐ 呼吸 / ○ 空闲）+ 读屏文案。 */}
          <StatusDot
            busy={!!mt?.streaming}
            text={mt?.streaming ? t("row.streaming") : t("row.idle")}
          />
          {/* 名字永远原位可编辑（用户 2026-11-19 拍板单行制）：没改名时显示的就是上游 id，
              双击进编辑；后方标签区分身份——未改「上游 id」、已改「别名」（客户端用暴露名请求，
              网关自动改写成上游真名）。占满剩余宽度，过长截断 +…（全称走 title 与编辑框）。
              清空提交 = 还原上游 id（用户 11-23 拍板）：取消暴露名，回到上游真名。 */}
          <InlineEdit
            value={m}
            editOnDouble
            display={displayName}
            allowEmpty
            className={`min-w-0 flex-1 font-mono text-xs leading-snug ${
              exposed ? "text-ink-2" : "text-ink-4 line-through"
            }`}
            inputClassName="min-w-0 flex-1 font-mono"
            title={
              renamed ? ts("row.renameTip", { m, u: upstream }) : ts("row.upstreamTip", { m })
            }
            ariaLabel={ts("row.exposedNameAria", { m })}
            onSave={(to) =>
              // 空 = 还原上游 id；与上游同名也走这条路（后端幂等）。
              void run(t("row.renamed"), () =>
                ipc("rename_model", { name: b.name, from: m, to: to === "" ? upstream : to }),
              )
            }
          />
          <span className="ml-auto flex shrink-0 items-center gap-x-2.5">
            {/* 别名/上游标签 = 显示名切换（用户 11-23 拍板）：改过名的行点它在
                「暴露名 ↔ 上游 id」间循环；没改过名的没得切，不响应。 */}
            <button
              type="button"
              disabled={!renamed}
              onClick={() => renamed && setShowUpstream((v) => !v)}
              className={`w-12 shrink-0 truncate rounded px-1 py-px text-center text-2xs ${
                renamed
                  ? "cursor-pointer bg-slate-700/60 text-ink-3 hover:bg-slate-700"
                  : "bg-slate-700/60 text-ink-3"
              }`}
              title={
                renamed
                  ? showUpstream
                    ? t("row.showAlias")
                    : t("row.showUpstream")
                  : t("row.tagUpstreamTip")
              }
            >
              {renamed ? (showUpstream ? t("row.tagUpstream") : t("row.tagAlias")) : t("row.tagUpstream")}
            </button>
            <CopyButton
              text={m}
              label={renamed ? ts("row.copyAlias", { m, u: upstream }) : ts("row.copyUpstream", { m })}
              className="opacity-0 group-hover/row:opacity-100 group-focus-within/row:opacity-100"
            />
            {/* VLM 槽位在工种徽标前（用户 11-23 终拍：工种徽标必须紧贴读数列）：
                槽位恒占宽，VLM 行与非 VLM 行都各归各位。 */}
            {vlmBadge}
            {/* 工种徽标恒显且点击循环切换（用户 2026-11-23「色条+淡晕+徽标」+
                同日拍板删下拉）：组内最后一个元素，右缘贴读数列，跨行成一条竖线。 */}
            {kindBadge}
          </span>
        </div>
        {/* 第 2 列：参数读数（定宽、贴向动作列，中段不留空洞）。第 3 列：动作区。 */}
        <span className="justify-self-end">{readouts}</span>
        {actions}
      </div>

      {/* ── 第 2 行：代理灯牌带（用户 2026-11-23）──
          整行留给全部代理（按代理页顺序），绿显 = 已占（灯上标槽数），灰显 = 未占仍可点。
          悬停看占到的槽位名，点击就地分配（矩阵同源 chipClick），放不下收进「…/更多」跳矩阵。
          fix:exposed-only-allocation：只有开了暴露开关的模型才可访问（用户 2026-11-17），
          未暴露不给灯牌带；媒体工种永不进代理分配（工种类互斥在后端焊死）。
          两者的说明文案不再占行，改挂工种下拉悬停（notePop）。 */}
      {!media && exposed && (
        <div className="mt-1 flex min-w-0 items-center gap-2 pl-[18px]">
          <AgentChips model={m} cells={mc} clients={clients} chipClick={chipClick} />
        </div>
      )}
    </div>
  );
}
