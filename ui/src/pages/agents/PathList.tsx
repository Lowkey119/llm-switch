// 配置文件路径清单（片三）：每行显示落点，可就地改到别的位置，越界要过二次确认。
//
// 编辑态（正在改哪一行、草稿、越界待确认）收在组件内部：这些状态只有这里用，
// 提到页面层只会让 AgentsPage 多背三个 state。
import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { ConfirmDialog, Risk } from "../../components/ConfirmDialog";
import { StatusDot } from "../../components/ui";
import { ipc } from "../../lib/ipc";
import { t, tf, ts } from "../../lib/i18n";
import type { FileEval } from "./types";

interface Props {
  app: string;
  files: FileEval[];
  /** 家目录：显示折叠成 ~/，同时是「越界」的判定基准。 */
  home: string;
}

export function PathList({ app, files, home }: Props) {
  const qc = useQueryClient();
  const toast = useToast();
  const [editIdx, setEditIdx] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [risk, setRisk] = useState<{ app: string; default: string; path: string } | null>(null);

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["takeover"] });
    qc.invalidateQueries({ queryKey: ["takeover-files"] });
    qc.invalidateQueries({ queryKey: ["matrix"] });
  };
  // 路径自定义：default 是内置路径（身份键），后端做白名单校验。
  const setPath = useMutation({
    mutationFn: (v: { app: string; default: string; path: string }) =>
      ipc("takeover_path_set", v),
    onSuccess: () => {
      toast.ok(t("ag.pathUpdated"));
      setEditIdx(null);
      invalidate();
    },
    onError: (e) => toast.err(ts("ag.pathSaveFail", { e: String(e) })),
  });
  const resetPath = useMutation({
    mutationFn: (v: { app: string; default: string }) => ipc("takeover_path_reset", v),
    onSuccess: () => {
      toast.ok(t("ag.pathResetOk"));
      setEditIdx(null);
      invalidate();
    },
    onError: (e) => toast.err(ts("ag.pathResetFail", { e: String(e) })),
  });

  /** 路径显示折叠：家目录下换成 ~/，一眼看出是否越界。 */
  const fold = (p: string) => (home && p.startsWith(home) ? `~${p.slice(home.length)}` : p);
  /** 保存路径：先做越界检查，越界弹风险确认，否则直接落。 */
  const commit = (def: string, raw: string) => {
    const value = raw.trim();
    const abs =
      value === "~" ? home : value.startsWith("~/") ? home + value.slice(1) : value;
    if (home && !abs.startsWith(home)) {
      setRisk({ app, default: def, path: value });
      return;
    }
    setPath.mutate({ app, default: def, path: value });
  };

  return (
    <>
      <ul className="mt-2 space-y-0.5">
        {files.map((f, i) => (
          <li
            key={f.default_path || f.path}
            className="flex flex-wrap items-center gap-2 text-sm text-ink-4"
          >
            {/* 已纳管/未纳管走状态灯（形状 + sr-only），与配置文件那份 ●/○ 同一口径。 */}
            {f.managed ? <StatusDot state="healthy" text={t("ag.fileManaged")} /> : <StatusDot text={t("ag.fileUnmanaged")} />}
            {editIdx === i ? (
              <span className="flex min-w-0 flex-1 items-center gap-1.5">
                <input
                  autoFocus
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") commit(f.default_path, draft);
                    if (e.key === "Escape") setEditIdx(null);
                  }}
                  title={t("ag.newPathTip")}
                  className="min-w-0 flex-1 rounded border border-slate-700 bg-slate-900 px-1.5 py-0.5 font-mono text-xs text-ink-2"
                />
                <button
                  className="shrink-0 text-cyan-300 hover:underline disabled:text-ink-4"
                  disabled={setPath.isPending}
                  onClick={() => commit(f.default_path, draft)}
                >
                  {t("dlg.save")}
                </button>
                <button className="shrink-0 text-ink-4 hover:text-ink-2" onClick={() => setEditIdx(null)}>
                  {t("dlg.cancel")}
                </button>
              </span>
            ) : (
              <>
                <span className="break-all font-mono" title={f.path}>
                  {fold(f.path)}
                </span>
                {f.overridden && (
                  <span
                    className="shrink-0 rounded border border-amber-500/40 bg-amber-500/10 px-1 text-2xs text-[var(--sem-busy-text)]"
                    title={ts("ag.builtinDefault", { p: fold(f.default_path) })}
                  >
                    {t("ag.customized")}
                  </span>
                )}
                {!f.exists && <span className="text-ink-4">{t("ag.notYetExists")}</span>}
                <span className="text-ink-4">{f.note}</span>
                <span className="ml-auto flex shrink-0 gap-2">
                  <button
                    className="text-ink-4 hover:text-ink-2"
                    title={t("ag.repathTip")}
                    onClick={() => {
                      setEditIdx(i);
                      setDraft(f.path);
                    }}
                  >
                    {t("ag.repath")}
                  </button>
                  {f.overridden && (
                    <button
                      className="text-[var(--sem-busy-text)] hover:text-[var(--sem-busy-text)] disabled:text-ink-4"
                      disabled={resetPath.isPending}
                      title={ts("ag.restorePathTip", { p: fold(f.default_path) })}
                      onClick={() => resetPath.mutate({ app, default: f.default_path })}
                    >
                      {t("ag.restoreDefault")}
                    </button>
                  )}
                </span>
              </>
            )}
          </li>
        ))}
      </ul>

      {/* 越界风险确认（片三）：自定义路径在家目录之外时的二次闸门。 */}
      <ConfirmDialog
        open={!!risk}
        title={t("ag.outsideTitle")}
        danger
        confirmLabel={t("ag.useAnyway")}
        onClose={() => setRisk(null)}
        onConfirm={() => {
          if (risk) setPath.mutate(risk);
        }}
      >
        {tf("ag.outsideBody", {
          b: <b>{risk?.path}</b>,
          home: home || "~",
          risk: <Risk>{t("ag.outsideRisk")}</Risk>,
        })}
      </ConfirmDialog>
    </>
  );
}
