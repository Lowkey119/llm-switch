// 代理详情的「模型分配」（M12-E）。
//
// 两个口径要分清：`current` 是从客户端配置文件现场读出来的**真值**，
// `desired` 是我们上次写下去的期望。不一致就说明有人在客户端那边动过，
// 界面按 current 显示，别拿库里的想法糊弄用户。
//
// 交互（fix:agent-detail-assign-and-takeover 2 号，用户 2026-11-16）：
// 一槽一个组合框、**常驻**、**选择即写入**——没有折叠、没有草稿、没有「应用」按钮。
// 没接管也照样能选：写下去由后端顺手接管（用户 2026-11-17），旧「先接管才能切模型」的
// 规矩随之作废（清空全部槽位则自动取消接管）。
//
// 槽位口径：后端把「这个端能填几个模型位」统一放在 `slots` 里——档位制（claude/codebuddy）
// 给的是档位名，pi 给 main/small，codex 给 sub（主槽就是默认档）。档位写进 `tiers`，
// pi/codex 的专属槽写进 `slots`，两类的空串都表示清空该槽。
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { QueryError } from "./QueryError";
import { ipc } from "../lib/ipc";
import { t, ts } from "../lib/i18n";

interface Sel {
  default: string | null;
  tiers: Record<string, string>;
  pi_main?: string | null;
  pi_small?: string | null;
  codex_sub?: string | null;
}

interface OptionGroup {
  backend: string;
  models: string[];
}

interface ModelView {
  app: string;
  current: Sel;
  desired: Sel;
  options: string[];
  /** 按后端分组（optgroup 用）；缺省时回退平铺 options。 */
  option_groups?: OptionGroup[];
  tiers: string[];
  /** 面板可编辑的槽位键（含档位；无档位端可能为空）。 */
  slots?: string[];
  taken_over: boolean;
  note?: string;
}

/** 一次写入的载荷：整表默认 + 档位，外加本次动过的那个专属槽。 */
interface Write {
  default: string | null;
  tiers: Record<string, string>;
  slots: Record<string, string>;
}

/** 槽位标签存 i18n key，渲染时 t() 取词。 */
const SLOT_LABELS: Record<string, string> = {
  haiku: "cp.slot.haiku",
  sonnet: "cp.slot.sonnet",
  opus: "cp.slot.opus",
  fable: "cp.slot.fable",
  subagent: "cp.slot.subagent",
  lite: "cp.slot.lite",
  reasoning: "cp.slot.reasoning",
  main: "cp.slot.main",
  small: "cp.slot.small",
  sub: "cp.slot.sub",
};

/** 走 `slots` 通道（而非 `tiers`）的专属槽：pi 的主/小、codex 的子代理。 */
const EXTRA_SLOTS = ["main", "small", "sub"];

/** 槽位当前真值（配置文件里读出来的），空串 = 未设。 */
function slotTruth(app: string, key: string, current: Sel): string {
  if (key === "main") return app === "codex" ? (current.default ?? "") : (current.pi_main ?? "");
  if (key === "small") return current.pi_small ?? "";
  if (key === "sub") return current.codex_sub ?? "";
  return "";
}

export function ClientModelPicker({
  app,
  current,
  managed,
  note,
}: {
  app: string;
  current: Sel;
  managed: boolean;
  note?: string;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  // 乐观回显：`current` 来自父层的 takeover 查询，写完要等它重新拉回来才变；
  // 中间那一小会儿先按用户刚选的显示，否则下拉会先弹回旧值再跳新值。
  // 真值一旦离开写入前的旧值（说明刷新回来了）或写入失败，就交还真值。
  const [echo, setEcho] = useState<{ key: string; from: string; value: string } | null>(null);

  const view = useQuery<ModelView>({
    queryKey: ["takeover-model", app],
    queryFn: () => ipc<ModelView>("takeover_model_get", { app }),
  });

  // 选择即写入：只发「本次动过的那一格」，其余槽位原样带上——后端 `tiers` 是整表替换，
  // 漏带等于把别的档位洗空；专属槽后端会按文件真值补没动过的，所以只带动过的那一个。
  const write = useMutation({
    mutationFn: (vars: Write) =>
      ipc<{ changed: boolean; backup_id: string | null }>("takeover_model_set", { app, ...vars }),
    onSuccess: (r) => {
      toast.ok(r.changed ? ts("cp.written", { id: r.backup_id ?? t("cfg.noneBackup") }) : t("cp.noChange"));
      // 详情与矩阵都读配置真值，一并失效（原 key "takeover-status" 不存在，属失效空转）。
      qc.invalidateQueries({ queryKey: ["takeover"] });
      qc.invalidateQueries({ queryKey: ["matrix"] });
      qc.invalidateQueries({ queryKey: ["takeover-model", app] });
      qc.invalidateQueries({ queryKey: ["takeover-backups"] });
    },
    onError: (e) => {
      setEcho(null);
      toast.err(ts("cp.writeFail", { e: String(e) }));
    },
  });

  // 乐观回显的读与写（口径见上面 echo 的注释）。
  const shown = (key: string, truth: string) =>
    echo && echo.key === key && truth === echo.from ? echo.value : truth;
  const pick = (key: string, from: string, value: string, vars: Write) => {
    setEcho({ key, from, value });
    write.mutate(vars);
  };

  const options = view.data?.options ?? [];
  const groups = view.data?.option_groups ?? [];
  const tierKeys = view.data?.tiers ?? [];
  const slotKeys = view.data?.slots ?? [];
  const tierSet = new Set(tierKeys);
  // 默认档行：pi 的主槽就是 main（走 slots 通道），再摆一行「默认」只会重复且恒为空。
  const showDefault = !slotKeys.includes("main");
  /* 「当前模型」取主位真值：有默认档行的端就是 default；pi 这类主槽走 slots 的端
     读 main 槽（用户 2026-11-17 截图：pi 主/小模型都配着，这里却永远「（未设）」——
     因为那行只盯 default，而 pi 的主槽是 pi_main）。下面的「未在暴露清单里」
     徽标跟着同一个真值判断，不许再各看各的。 */
  const mainTruth = showDefault ? (current.default ?? "") : slotTruth(app, "main", current);
  // ui_04：「当前模型」独立行已删（同一真值不在两处各写一遍）；主槽行挂「当前生效」徽标，
  // 主位真值不在暴露清单里时另挂「未在暴露清单里」（口径不变，只是换了位置）。
  const mainBadge = (
    <>
      {mainTruth && (
        <span className="shrink-0 rounded border border-emerald-500/40 px-1 text-2xs text-[var(--sem-ok-text)]">
          {t("cp.curEffective")}
        </span>
      )}
      {mainTruth && view.data && !options.includes(mainTruth) && managed && (
        <span className="shrink-0 rounded bg-amber-500/15 px-1.5 py-0.5 text-2xs text-[var(--sem-busy-text)]">
          {t("cp.notInList")}
        </span>
      )}
    </>
  );
  const extraKeys = slotKeys.filter((k) => !tierSet.has(k) && EXTRA_SLOTS.includes(k));
  // 写单格时没动过的部分照文件真值带上（不许拿库里的期望去盖）。
  const base: Write = { default: current.default ?? null, tiers: current.tiers ?? {}, slots: {} };
  const busy = write.isPending;

  // 一行一槽（ui_04）：label 定宽 120px 右对齐，下拉吃满剩余宽度，所有行左缘对齐。
  const rowCls = "grid grid-cols-[7.5rem_minmax(0,1fr)] items-center gap-2 text-sm";
  const selectCls =
    "w-full min-w-0 rounded-md border border-slate-700 bg-slate-900 px-2 py-1 font-mono text-xs";
  // 组合框标题说实话：选了就写盘，选空就是把这一格清空（旧标签「保持原样」会误导）。
  const hint = t("cp.hint");
  const optionsNode = (filled: boolean) => (
    <>
      <option value="">{filled ? t("cp.clearSlot") : t("cp.unset")}</option>
      {groups.length > 0 ? (
        // 按后端分组 + 显示暴露名（全站口径）：一眼看出模型是谁提供的。
        groups.map((g) => (
          <optgroup key={g.backend} label={g.backend}>
            {g.models.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </optgroup>
        ))
      ) : (
        options.map((m) => (
          <option key={m} value={m}>
            {m}
          </option>
        ))
      )}
    </>
  );

  return (
    <div>
      <div className="rounded-md border border-slate-800 bg-slate-950/60 p-2">
        {view.isLoading && <div className="text-sm text-ink-4">{t("cfg.readingList")}</div>}
        {view.isError && (
          <QueryError inline title={t("ag.readFailBadge")} error={view.error} onRetry={() => view.refetch()} />
        )}
        {view.data && (
          <div className="space-y-1.5">
            {showDefault && (
              <label className={rowCls}>
                <span className="text-ink-4">{t("cp.default")}</span>
                <span className="flex min-w-0 items-center gap-2">
                  <select
                    value={shown("default", current.default ?? "")}
                    disabled={busy}
                    title={hint}
                    onChange={(e) =>
                      pick("default", current.default ?? "", e.target.value, {
                        ...base,
                        default: e.target.value || null,
                      })
                    }
                    className={selectCls}
                  >
                    {optionsNode(!!(current.default ?? ""))}
                  </select>
                  {mainBadge}
                </span>
              </label>
            )}
            {slotKeys.map((key) =>
              tierSet.has(key) ? (
                <label key={key} className={rowCls}>
                  <span className="truncate text-ink-4" title={t(SLOT_LABELS[key] ?? key)}>{t(SLOT_LABELS[key] ?? key)}</span>
                  <select
                    value={shown(key, current.tiers?.[key] ?? "")}
                    disabled={busy}
                    title={hint}
                    onChange={(e) =>
                      pick(key, current.tiers?.[key] ?? "", e.target.value, {
                        ...base,
                        tiers: { ...(current.tiers ?? {}), [key]: e.target.value },
                      })
                    }
                    className={selectCls}
                  >
                    {optionsNode(!!(current.tiers?.[key] ?? ""))}
                  </select>
                </label>
              ) : extraKeys.includes(key) ? (
                <label key={key} className={rowCls}>
                  <span className="truncate text-ink-4" title={t(SLOT_LABELS[key] ?? key)}>{t(SLOT_LABELS[key] ?? key)}</span>
                  <span className="flex min-w-0 items-center gap-2">
                    <select
                      value={shown(key, slotTruth(app, key, current))}
                      disabled={busy}
                      title={hint}
                      onChange={(e) =>
                        pick(key, slotTruth(app, key, current), e.target.value, {
                          ...base,
                          slots: { [key]: e.target.value },
                        })
                      }
                      className={selectCls}
                    >
                      {optionsNode(!!slotTruth(app, key, current))}
                    </select>
                    {key === "main" && !showDefault && mainBadge}
                  </span>
                </label>
              ) : null,
            )}
          </div>
        )}
        {(note || view.data?.note) && (
          <p className="mt-2 text-sm text-ink-4">{note ?? view.data?.note}</p>
        )}
      </div>
    </div>
  );
}
