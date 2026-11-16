// 接管页的「切模型」（M12-E）。
//
// 两个口径要分清：`current` 是从客户端配置文件现场读出来的**真值**，
// `desired` 是我们上次写下去的期望。不一致就说明有人在客户端那边动过，
// 界面按 current 显示，别拿库里的想法糊弄用户。
//
// 规矩：没接管不让切（我们对没记账的配置没把握）。
//
// 槽位口径：后端把「这个端能填几个模型位」统一放在 `slots` 里——档位制（claude/codebuddy）
// 给的是档位名，pi 给 main/small，codex 给 sub（主槽就是默认档）。档位写进 `tiers`，
// pi/codex 的专属槽写进 `slots`，两类的空串都表示清空该槽。

import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { QueryError } from "./QueryError";
import { Button } from "./ui";
import { ipc } from "../lib/ipc";

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

const SLOT_LABELS: Record<string, string> = {
  haiku: "快速档 haiku",
  sonnet: "主力档 sonnet",
  opus: "重载档 opus",
  fable: "fable",
  subagent: "子任务 subagent",
  lite: "轻量档 lite",
  reasoning: "推理档 reasoning",
  main: "主模型",
  small: "小模型",
  sub: "子代理 sub",
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
  const [open, setOpen] = useState(false);
  const [pick, setPick] = useState<string>(current.default ?? "");
  const [tiers, setTiers] = useState<Record<string, string>>(current.tiers ?? {});
  // 只装「用户动过手」的专属槽：没动过就不发给后端，后端也就不会把用户配好的槽位当改动覆盖。
  const [extra, setExtra] = useState<Record<string, string>>({});
  const toast = useToast();

  const view = useQuery<ModelView>({
    queryKey: ["takeover-model", app],
    queryFn: () => ipc<ModelView>("takeover_model_get", { app }),
    enabled: open,
  });

  const save = useMutation({
    mutationFn: () =>
      ipc<{ changed: boolean; backup_id: string | null }>("takeover_model_set", {
        app,
        default: pick || null,
        tiers,
        slots: extra,
      }),
    onSuccess: (r) => {
      toast.ok(r.changed ? `已写入（备份 ${r.backup_id ?? "无"}）` : "本来就是这个名字，没改动");
      // 详情与矩阵都读配置真值，一并失效（原 key "takeover-status" 不存在，属失效空转）。
      qc.invalidateQueries({ queryKey: ["takeover"] });
      qc.invalidateQueries({ queryKey: ["matrix"] });
      qc.invalidateQueries({ queryKey: ["takeover-model", app] });
      qc.invalidateQueries({ queryKey: ["takeover-backups"] });
    },
    onError: (e) => toast.err(`写入失败：${e}`),
  });

  const options = view.data?.options ?? [];
  const groups = view.data?.option_groups ?? [];
  const tierKeys = view.data?.tiers ?? [];
  const slotKeys = view.data?.slots ?? [];
  const tierSet = new Set(tierKeys);
  // 默认档行：pi 的主槽就是 main（走 slots 通道），再摆一行「默认」只会重复且恒为空。
  const showDefault = !slotKeys.includes("main");
  const extraKeys = slotKeys.filter((k) => !tierSet.has(k) && EXTRA_SLOTS.includes(k));
  const slotValue = (k: string) => extra[k] ?? slotTruth(app, k, current);

  const dirty =
    (showDefault && pick !== (current.default ?? "")) ||
    tierKeys.some((t) => (current.tiers?.[t] ?? "") !== (tiers[t] ?? "")) ||
    extraKeys.some((k) => slotTruth(app, k, current) !== slotValue(k));

  // 一行一槽：label 定宽 + 下拉定宽，所有行的左边缘必须对齐（这是这个面板被嫌丑的地方）。
  const rowCls = "grid grid-cols-[6rem_minmax(0,18rem)] items-center gap-2 text-xs";
  const selectCls =
    "w-full min-w-0 rounded-md border border-slate-700 bg-slate-900 px-2 py-1 font-mono text-xs";
  // 空值这一项的真实语义是「该槽不填」，不是「保持原样」——已经填了模型的槽
  // 选它就是把槽清空，标签必须说实话，否则一次误点就悄悄清掉一个槽位。
  const optionsNode = (filled: boolean) => (
    <>
      <option value="">{filled ? "（清空该槽）" : "（不指定）"}</option>
      {groups.length > 0 ? (
        // 按后端分组：一眼看出模型是谁提供的；同名共享模型在 `a + b` 组合组里。
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
    <div className="mt-1">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <span className="text-ink-4">当前模型</span>
        <span className="font-mono text-ink-2">{current.default ?? "（未设）"}</span>
        {current.default &&
          view.data &&
          !options.includes(current.default) &&
          managed && (
            <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[var(--sem-busy-text)]">
              未在暴露清单里
            </span>
          )}
        {Object.keys(current.tiers ?? {}).length > 0 && (
          <span className="text-ink-4">
            档位 {Object.entries(current.tiers).map(([k, v]) => `${k}:${v}`).join(" ")}
          </span>
        )}
        {extraKeys.length > 0 && (
          <span className="text-ink-4">
            {extraKeys
              .map((k) => `${SLOT_LABELS[k] ?? k}:${slotTruth(app, k, current) || "—"}`)
              .join(" ")}
          </span>
        )}
        <span className="ml-auto">
          <Button
            variant="ghost"
            disabled={!managed}
            title={managed ? undefined : "先接管才能切模型"}
            onClick={() => setOpen(!open)}
          >
            {open ? "收起" : "切换模型"}
          </Button>
        </span>
      </div>

      {open && (
        <div className="mt-2 rounded-md border border-slate-800 bg-slate-950/60 p-2">
          {view.isLoading && <div className="text-xs text-ink-4">读取清单…</div>}
          {view.isError && (
            <QueryError inline title="读取失败" error={view.error} onRetry={() => view.refetch()} />
          )}
          {view.data && (
            <div className="space-y-1.5">
              {showDefault && (
                <label className={rowCls}>
                  <span className="text-ink-4">默认</span>
                  <select value={pick} onChange={(e) => setPick(e.target.value)} className={selectCls}>
                    {optionsNode(!!pick)}
                  </select>
                </label>
              )}
              {slotKeys.map((key) =>
                tierSet.has(key) ? (
                  <label key={key} className={rowCls}>
                    <span className="truncate text-ink-4" title={SLOT_LABELS[key] ?? key}>{SLOT_LABELS[key] ?? key}</span>
                    <select
                      value={tiers[key] ?? ""}
                      onChange={(e) => setTiers({ ...tiers, [key]: e.target.value })}
                      className={selectCls}
                    >
                      {optionsNode(!!(tiers[key] ?? ""))}
                    </select>
                  </label>
                ) : extraKeys.includes(key) ? (
                  <label key={key} className={rowCls}>
                    <span className="truncate text-ink-4" title={SLOT_LABELS[key] ?? key}>{SLOT_LABELS[key] ?? key}</span>
                    <select
                      value={slotValue(key)}
                      onChange={(e) => setExtra({ ...extra, [key]: e.target.value })}
                      className={selectCls}
                    >
                      {optionsNode(!!slotValue(key))}
                    </select>
                  </label>
                ) : null,
              )}
              <div className="flex flex-wrap items-center gap-2 pt-1 text-xs">
                <Button
                  onClick={() => save.mutate()}
                  disabled={!dirty || save.isPending || !view.data.taken_over}
                >
                  {save.isPending ? "写入中…" : "应用"}
                </Button>
                {!view.data.taken_over && (
                  <span className="text-[var(--sem-busy-text)]">该端尚未接管，不能改模型</span>
                )}
                {dirty && view.data.taken_over && <span className="text-ink-4">有改动</span>}
              </div>
            </div>
          )}
          {(note || view.data?.note) && (
            <p className="mt-2 text-xs text-ink-4">{note ?? view.data?.note}</p>
          )}
        </div>
      )}
    </div>
  );
}
