import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Toggle } from "./ui";
import { IconCaret, IconClose } from "./icons";
import { InlineEdit } from "./InlineEdit";
import { QueryError } from "./QueryError";
import { ipc } from "../lib/ipc";
import { t, ts } from "../lib/i18n";

/**
 * claude 语义开关（P0-a）。
 *
 * 键名、中文名、类型都由后端 `defs()` 给——前端不再抄一份表，
 * 免得两边漂。真值永远是 `~/.claude/settings.json`，这里只是把手。
 */

interface Def {
  key: string;
  label: string;
  kind: "bool" | "number";
}

interface SwitchState {
  defs: Def[];
  state: Record<string, boolean | string | null>;
}

export function ClaudeSwitches({ managed }: { managed: boolean }) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const sw = useQuery<SwitchState>({
    queryKey: ["claude-switches"],
    queryFn: () => ipc<SwitchState>("claude_switches_get"),
  });

  const put = useMutation({
    mutationFn: ({ key, value }: { key: string; value: unknown }) =>
      ipc("claude_switch_set", { key, value }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["claude-switches"] }),
  });

  const defs = sw.data?.defs ?? [];
  const state = sw.data?.state ?? {};
  const onCount = defs.filter((d) =>
    d.kind === "bool" ? state[d.key] === true : state[d.key] != null,
  ).length;
  const err = put.error instanceof Error ? put.error.message : null;

  return (
    <div className="mt-2 rounded-lg border border-slate-800 bg-slate-900/40">
      <button
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-sm text-ink-2 hover:text-ink-1"
        onClick={() => setOpen(!open)}
      >
        <IconCaret open={open} className="text-ink-4" />
        <span>{t("cw.title")}</span>
        <span className="text-ink-4">
          {/* 读取失败时别显示「全部默认」——那是空数据的默认文案，会误导 */}
          {sw.isError ? t("ag.readFailBadge") : onCount > 0 ? ts("cw.onCount", { n: onCount }) : t("cw.allDefault")}
        </span>
        {!managed && <span className="text-ink-4">{t("cw.takeoverHint")}</span>}
      </button>
      {open && (
        <div className="grid grid-cols-2 gap-x-4 gap-y-1.5 border-t border-slate-800 px-2.5 py-2">
          {sw.isLoading && <div className="text-sm text-ink-4">{t("pt.loading")}</div>}
          {/* 读取失败 ≠ 没有开关：原先失败时 defs 为空、面板一片空白，看着像「本来就没开关」 */}
          {sw.isError && (
            <div className="col-span-2">
              <QueryError inline title={t("cw.readFail")} error={sw.error} onRetry={() => sw.refetch()} />
            </div>
          )}
          {defs.map((d) => (
            <label key={d.key} className="flex items-center gap-2 text-sm">
              {d.kind === "bool" ? (
                <>
                  <Toggle
                    checked={state[d.key] === true}
                    disabled={!managed || put.isPending}
                    onChange={(v) => put.mutate({ key: d.key, value: v })}
                  />
                  <span className={state[d.key] === true ? "text-ink-2" : "text-ink-3"}>
                    {d.label}
                  </span>
                </>
              ) : (
                <>
                  {/* 原位编辑走全站 InlineEdit（UI-AUDIT-3 P3-2 / 决策 D1）——原先这里
                      onBlur 即写库，点一下别处半截值就进了 claude 配置文件，且用
                      placeholder 当标签（P1-3 反模式）。现：悬停铅笔可发现、Enter 保存、
                      Esc/失焦 = 放弃；清键仍走右侧专用按钮（InlineEdit 空值 = 放弃，
                      恰好挡住「顺手清键」这条更危险的路径）。 */}
                  {managed ? (
                    <InlineEdit
                      value={(state[d.key] as string | null) ?? ""}
                      display={state[d.key] != null ? String(state[d.key]) : "—"}
                      ariaLabel={d.label}
                      title={ts("cw.editTip", { label: d.label })}
                      inputClassName="w-24"
                      className="text-ink-2"
                      onSave={(v) => put.mutate({ key: d.key, value: v })}
                    />
                  ) : (
                    <span className="text-ink-3">
                      {state[d.key] != null ? String(state[d.key]) : "—"}
                    </span>
                  )}
                  <span className="text-ink-3">{d.label}</span>
                  {state[d.key] != null && (
                    <button
                      className="grid h-5 w-5 shrink-0 place-items-center rounded text-ink-4 hover:text-[var(--sem-danger-text)] disabled:cursor-not-allowed"
                      aria-label={t("cw.clearTip")}
                      disabled={!managed || put.isPending}
                      onClick={() => put.mutate({ key: d.key, value: "" })}
                    >
                      <IconClose width={12} height={12} />
                    </button>
                  )}
                </>
              )}
            </label>
          ))}
          {err && <div className="col-span-2 text-sm text-[var(--sem-danger-text)]">{err}</div>}
        </div>
      )}
    </div>
  );
}
