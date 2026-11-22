// 模型参数覆盖弹窗（重设计）：参数目录全集平铺 + 双击行内覆盖。
// - 平铺：目录（后端下发）按组全量展示；引擎能力集内的键可双击编辑，不支持/只读的灰显；
// - 范围：长度键编辑限制在模型允许范围内（模型输出/上下文上限，来自元数据声明与探测）；
// - 继承链：每行内联展示 生效值与来源（模型覆盖 > 全局默认 > 后端默认 > 不下发），不再单开预览区。
import { useMemo, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useQuery } from "@tanstack/react-query";
import { Badge, Button } from "./ui";
import { IconClose } from "./icons";
import { CopyButton } from "./CopyButton";
import { SearchInput } from "./SearchInput";
import { QueryError, errText } from "./QueryError";
import { ipc } from "../lib/ipc";
import { t, tf, ts } from "../lib/i18n";
import {
  buildChains,
  POLICIES,
  TEMPLATES,
  WINNER_KEYS,
  type ChainLink,
  type ParamRow,
} from "../lib/params";
import { detectKind, KIND_LABEL, NON_INFERENCE_KINDS } from "../lib/modelkind";
import { useParamWrite } from "./model-params/useParamWrite";
import {
  GROUP_LABEL,
  GROUP_ORDER,
  noOverrideHint,
  READONLY_GROUP,
  capHint,
  dimReason,
  field,
  dispVal,
  editable,
  type CatalogEntry,
  type MetaRow,
} from "./model-params/meta";
interface Props {
  backend: string;
  model: string;
  onClose: () => void;
}

export function ModelParamsDialog({ backend, model, onClose }: Props) {
  const [filter, setFilter] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [policy, setPolicy] = useState("fill");
  /** 非推理种类（向量化/重排/语音）默认只出说明面板；误判时可展开全量。 */
  const [showAll, setShowAll] = useState(false);

  const params = useQuery({
    queryKey: ["params"],
    // IPC 返回裸数组（REST /admin/params 才包 {overrides}——别再按包裹形状取）。
    queryFn: () => ipc<ParamRow[]>("list_param_overrides"),
  });
  const catalog = useQuery({
    queryKey: ["param-catalog"],
    queryFn: () => ipc<CatalogEntry[]>("get_param_catalog"),
  });
  const caps = useQuery({
    queryKey: ["backend-caps", backend],
    queryFn: () =>
      ipc<{ engine: string | null; keys: string[] }>("get_backend_capabilities", { backend }),
  });
  const metaQ = useQuery({
    queryKey: ["model-meta"],
    queryFn: () => ipc<MetaRow[]>("model_meta_get"),
    refetchOnWindowFocus: false,
  });

  // 稳定引用：否则下面 useMemo 的依赖每次渲染都变（eslint react-hooks 会报）。
  const rows = useMemo(() => params.data ?? [], [params.data]);
  const modelRef = `${backend}:${model}`;
  const mine = useMemo(
    () =>
      new Map(
        rows
          .filter((r) => r.scope === "model" && r.ref_id === modelRef)
          .map((r) => [r.key, r] as const),
      ),
    [rows, modelRef],
  );
  // 长度量纲键集合：这些键配 0 = 继承（与后端 catalog 同口径）。
  const lengthKeys = useMemo(
    () =>
      new Set(
        (catalog.data ?? [])
          .filter((c) => c.group === "length")
          .map((c) => c.key),
      ),
    [catalog.data],
  );
  const chainMap = useMemo(
    () =>
      new Map<string, ChainLink>(
        buildChains(rows, backend, model, lengthKeys).map((c) => [c.key, c]),
      ),
    [rows, backend, model, lengthKeys],
  );

  // 模型种类：向量化/重排/语音类不吃采样参数——不再平铺，改成说明面板
  // （名称启发式可能误判，留「查看全部参数」逃生口）。

  // 模型允许范围（双击编辑的钳制依据）：元数据的有效上限（人工声明已钳到后端允许范围）。
  const meta = metaQ.data?.find((x) => x.model === model);
  // 纯规则（可编辑判定 / 范围提示与校验 / 展示值）在 model-params/meta.ts，这里只喂模型上限。
  const lim = { capOutput: meta?.cap_output ?? null, capCtx: meta?.cap_ctx ?? null, lengthKeys };
  const { busy, rangeErr, setRangeErr, save, drop, applyTemplate } = useParamWrite({
    modelRef,
    mine,
    lim,
    onSaved: () => cancelEdit(),
  });
  const kind = detectKind(model, meta?.vlm);
  const nonInference = !showAll && NON_INFERENCE_KINDS.includes(kind);

  // 引擎能力集：与后端门控同语义——空集/未取到 = 未约束（全放行）。
  const capKeys = useMemo(
    () => (caps.data?.keys?.length ? new Set(caps.data.keys) : null),
    [caps.data],
  );

  const entries = catalog.data ?? [];
  const q = filter.trim().toLowerCase();
  const groups = GROUP_ORDER.map((g) => ({
    g,
    label: t(GROUP_LABEL[g]),
    items: entries.filter((e) => e.group === g && (!q || e.key.includes(q))),
  })).filter((x) => x.items.length > 0);


  const startEdit = (en: CatalogEntry) => {
    if (!editable(en, capKeys)) return;
    const m = mine.get(en.key);
    const c = chainMap.get(en.key);
    setEditing(en.key);
    setDraft(
      m
        ? JSON.stringify(m.value)
        : c && c.winner !== "inherit"
          ? JSON.stringify(c.winner_value)
          : "",
    );
    setPolicy(m?.policy ?? en.default_policy);
    setRangeErr(null);
  };

  // 删覆盖是「改了配置」级动作：就地两步确认（闸门矩阵的轻量档），不弹整窗。
  // 删覆盖是「改了配置」级动作：就地两步确认（闸门矩阵的轻量档），不弹整窗。
  const [dropArmed, setDropArmed] = useState(false);

  const cancelEdit = () => {
    setEditing(null);
    setRangeErr(null);
    setDropArmed(false);
  };


  return (
    <Dialog.Root open onOpenChange={(o) => !o && onClose()}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
        {/* 高度用百分比不用 vh：app 的缩放挂在 documentElement 的 CSS zoom 上，
            vh 会被倍率乘高（zoom>1.17 时对话框比窗口高，标题/关闭钮/完成钮
            全被顶出屏幕）；百分比按 zoom 后的视口折算，永远居中可见。 */}
        <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] flex max-h-[var(--dialog-max-h)] w-[820px] max-w-[95%] -translate-x-1/2 -translate-y-1/2 flex-col rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
          <div className="flex shrink-0 items-start justify-between gap-3">
            <div className="min-w-0">
              <Dialog.Title className="flex min-w-0 items-center gap-2 text-base font-semibold">
                <span className="min-w-0 truncate" title={ts("pd.title", { b: backend, m: model })}>{ts("pd.title", { b: backend, m: model })}</span>
                {/* 弹窗标题里是完整模型 id：顺手给一个复制出口（E-2），报障时不用手择。 */}
                <CopyButton text={model} label={ts("pd.copyId", { m: model })} className="mt-0.5" />
              </Dialog.Title>
              <p className="mt-1 text-sm text-ink-4">
                {tf("pd.desc", { b: <b>{t("pd.descBold")}</b> })}
                {caps.data?.engine ? ts("pd.engine", { e: caps.data.engine }) : ""}
              </p>
            </div>
            <button
              onClick={onClose}
              aria-label={t("pd.closeAria")}
              title={t("pd.closeTip")}
              className="inline-flex shrink-0 items-center gap-1 rounded-md border border-slate-700 px-3 py-1.5 text-sm text-ink-3 hover:bg-slate-800 hover:text-ink-1"
            >
              <IconClose width={12} height={12} />
              {t("pd.close")}
            </button>
          </div>

          {/* 工具行：过滤 + 模板 */}
          <div className="mt-3 flex shrink-0 flex-wrap items-center gap-2">
            <SearchInput value={filter} onChange={setFilter} placeholder={t("pd.filterPh")} inputClassName="w-44" />
            <span className="text-ink-4">|</span>
            <span className="text-sm text-ink-4">{t("pd.templates")}</span>
            {TEMPLATES.map((tpl) => (
              <Button key={tpl.name} variant="ghost" disabled={busy} onClick={() => applyTemplate(tpl)}>
                {tpl.name}
              </Button>
            ))}
            <span className="ml-auto text-sm text-ink-4">
              {ts("pd.overrideN", { n: mine.size })}
            </span>
          </div>

          {/* 平铺参数表（非推理种类换成说明面板） */}
          {nonInference ? (
            <div className="mt-3 min-h-0 flex-1 overflow-y-auto pr-1">
              <div className="rounded-lg border border-slate-800 px-6 py-8 text-center">
                <p className="text-sm text-ink-2">
                  {ts("pd.nonInference", { kind: t(KIND_LABEL[kind]) })}
                </p>
                <button
                  onClick={() => setShowAll(true)}
                  className="mt-4 text-sm text-ink-4 underline-offset-2 hover:text-ink-2 hover:underline"
                >
                  {t("pd.showAll")}
                </button>
              </div>
            </div>
          ) : (
          <div className="mt-3 min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
            {/* 读取失败 ≠ 没有参数：原先失败会落到「没有匹配」，看着像参数表本来就是空的 */}
            {(params.isError || catalog.isError) && (
              <div className="rounded-md border border-slate-800 px-3 py-4 text-center">
                <QueryError
                  inline
                  title={t("pd.readFail")}
                  error={errText(params.error) || errText(catalog.error)}
                  onRetry={() => {
                    if (params.isError) params.refetch();
                    if (catalog.isError) catalog.refetch();
                  }}
                />
              </div>
            )}
            {groups.length === 0 && !params.isError && !catalog.isError && (
              <p className="rounded-md border border-slate-800 px-3 py-4 text-center text-sm text-ink-4">
                {ts("pd.noMatch", { q: filter })}
              </p>
            )}
            {groups.map(({ g, label, items }) => (
              <section key={g}>
                <h3
                  className={`mb-1 text-xs font-medium uppercase tracking-wide ${
                    g === READONLY_GROUP ? "text-ink-4" : "text-ink-3"
                  }`}
                >
                  {label}
                  {g === "vlm" && meta?.vlm === true && (
                    <span className="ml-1.5 rounded bg-emerald-500/15 px-1 text-2xs normal-case text-[var(--sem-ok-text)]">
                      {t("pd.vlmYes")}
                    </span>
                  )}
                </h3>
                {/* 一行两列由容器宽 ÷ --flow-param 算出来（U9）：这一行是 176px 键名列
                    + 值 + 徽标，340px 是它不折行的实测下限；对话框 max-w-[95%] 收窄时它自己退成一列。 */}
                <div className="param-grid gap-1.5">
                  {items.map((en) => {
                    const c = chainMap.get(en.key);
                    const m = mine.get(en.key);
                    const ok = editable(en, capKeys);
                    const hint = capHint(en.key, lim);
                    return (
                      <div
                        key={en.key}
                        onDoubleClick={() => startEdit(en)}
                        title={ok ? ts("pd.dblTip", { hint: hint ? ts("pd.dblHint", { hint }) : "" }) : dimReason(en)}
                        className={`flex items-center gap-2 rounded-md border border-slate-800 px-2.5 py-1.5 text-sm ${
                          ok ? "cursor-pointer hover:bg-slate-800/50" : "cursor-not-allowed"
                        } ${editing === en.key ? "col-span-full bg-slate-800/60" : ""}`}
                      >
                        <span className="w-44 shrink-0 truncate font-mono text-ink-2" title={en.key}>
                          {en.key}
                          {m && (
                            <span className="ml-1 rounded bg-cyan-500/15 px-1 text-2xs text-cyan-300">{t("pd.overrideBadge")}</span>
                          )}
                        </span>

                        {editing === en.key ? (
                          <span
                            className="flex flex-1 flex-wrap items-center gap-1.5"
                            onClick={(e) => e.stopPropagation()}
                          >
                            <input
                              autoFocus
                              value={draft}
                              onChange={(e) => {
                                setDraft(e.target.value);
                                setRangeErr(null);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === "Enter") save(en.key, draft, policy);
                                if (e.key === "Escape") cancelEdit();
                              }}
                              placeholder={t("pd.valuePh")}
                              className={`${field} w-44 font-mono`}
                            />
                            <select value={policy} onChange={(e) => setPolicy(e.target.value)} className={field}>
                              {POLICIES.map((p) => (
                                <option key={p.v} value={p.v}>
                                  {t(p.labelKey)}
                                </option>
                              ))}
                            </select>
                            <Button variant="primary" disabled={busy} onClick={() => save(en.key, draft, policy)}>
                              {t("dlg.save")}
                            </Button>
                            <button onClick={cancelEdit} className="text-ink-4 hover:text-ink-2">
                              {t("dlg.cancel")}
                            </button>
                            {m && (
                              <button
                                onClick={() => (dropArmed ? drop(m.id) : setDropArmed(true))}
                                className={
                                  dropArmed
                                    ? "text-[var(--sem-danger-text)] underline underline-offset-2"
                                    : "text-ink-4 hover:text-[var(--sem-danger-text)]"
                                }
                                title={dropArmed ? t("pd.dropArmTip") : t("pd.dropTip")}
                              >
                                {dropArmed ? t("pd.dropConfirm") : t("pd.drop")}
                              </button>
                            )}
                            {hint && <span className="text-sm text-ink-4">{hint}</span>}
                            {rangeErr && <span className="w-full text-sm text-[var(--sem-danger-text)]">{rangeErr}</span>}
                          </span>
                        ) : (
                          <>
                            <span
                              className="min-w-0 flex-1 truncate font-mono text-ink-2"
                              /* 覆盖态与未覆盖态都要 title：截断后全文无处读回（P3-5），
                                 原先只在「未下发」分支给了 hint，覆盖态的值截断即丢。 */
                              title={
                                c && c.winner !== "inherit"
                                  ? dispVal(en.key, c.winner_value, lengthKeys)
                                  : noOverrideHint()
                              }
                            >
                              {c && c.winner !== "inherit"
                                ? dispVal(en.key, c.winner_value, lengthKeys)
                                : t("pd.notSent")}
                            </span>
                            {c && <Badge>{t(WINNER_KEYS[c.winner])}</Badge>}
                            {c?.winner_policy && c.winner_policy !== "fill" && <Badge>{c.winner_policy}</Badge>}
                            {!ok && (
                              <span className="shrink-0 text-sm text-ink-4">
                                {en.group === READONLY_GROUP ? t("pd.readonly") : t("pd.unsupported")}
                              </span>
                            )}
                          </>
                        )}
                      </div>
                    );
                  })}
                </div>
              </section>
            ))}
          </div>
          )}

          <div className="mt-4 flex shrink-0 items-center justify-between gap-3">
            <span className="text-sm text-ink-4">
              {t("pd.footer")}
            </span>
            <Button onClick={onClose}>{t("pd.done")}</Button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
