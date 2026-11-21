// Ponytail 节（各客户端共用）：第三方 ponytail 插件的装/卸 + 上游更新检测。
//
// 与速率扩展那节（ExtSection）共用同一套后端机器，只是 `pack: "ponytail"`：
// 装法逐家不同（pi 扩展 / marketplace / npm / extensions install），全部走该端**官方**
// CLI，机制逐家实测见 `src-tauri/src/services/ponytail.rs` 模块头。
//
// 版本分三层，各说各的，不互相冒充：
//   上游最新（GitHub / npm 双源）· 本地检出（llm-switch 数据目录里那份源码）· 该端装没装。
// 「更新检出」只动源码，**不碰任何代理的配置**；各端要不要跟着重装，用户逐端点。
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { useI18n, ts } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";
import { EXT_STATE_LABEL, EXT_STATE_STYLE, type ExtRow, type ExtStatus, type ExtResult } from "./ExtSection";

/** `ponytail_status` / `ponytail_check` / `ponytail_update` 的返回。 */
interface Overview {
  checkout: string;
  checked_out: boolean;
  local_version: string | null;
  upstream: {
    version: string;
    tag: string;
    source: string;
    url: string;
    published: string | null;
    checked_at: number;
  } | null;
  has_update: boolean;
  error: string | null;
  cached_at: number | null;
}

/** epoch 秒 → 年月日（与 UpdateDialog 同一口径：只到日，不装精确到分）。 */
function dayOf(epochSec: number | null | undefined): string | null {
  if (!epochSec) return null;
  return new Date(epochSec * 1000).toISOString().slice(0, 10);
}

export function PonytailSection({ app }: { app: string }) {
  const { t } = useI18n();
  const qc = useQueryClient();
  const toast = useToast();

  const ext = useQuery({
    queryKey: ["ext", app, "ponytail"],
    queryFn: () => ipc<ExtStatus>("ext_status", { app, pack: "ponytail" }),
  });
  const ov = useQuery({
    queryKey: ["ponytail"],
    queryFn: () => ipc<Overview>("ponytail_status"),
  });

  const rows: ExtRow[] = ext.data?.rows ?? [];
  const installed = rows.some((r) => r.state === "installed" || r.state === "outdated");

  /** 装/卸：与速率扩展同一套结果语义（unchanged 不算改动）。 */
  const mutate = useMutation({
    mutationFn: (cmd: "ext_install" | "ext_uninstall") => ipc<ExtResult[]>(cmd, { app, pack: "ponytail" }),
    onSuccess: (res, cmd) => {
      qc.invalidateQueries({ queryKey: ["ext", app, "ponytail"] });
      const changed = res.filter((r) => r.action !== "unchanged");
      if (changed.length === 0) {
        toast.ok(cmd === "ext_uninstall" ? t("pt.noneToRemove") : t("pt.alreadyInstalled"));
        return;
      }
      toast.ok(cmd === "ext_uninstall" ? t("pt.uninstalled") : t("pt.installed"));
    },
    onError: (e: unknown, cmd: "ext_uninstall" | "ext_install") =>
      toast.err(cmd === "ext_uninstall" ? `${t("pt.failUninstall")}${e}` : `${t("pt.failInstall")}${e}`),
  });

  const check = useMutation({
    mutationFn: () => ipc<Overview>("ponytail_check"),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: ["ponytail"] });
      if (r.error) toast.err(`${t("pt.failCheck")}${r.error}`);
      else if (r.has_update) toast.ok(t("pt.updateFound"));
      else toast.ok(t("pt.noUpdate"));
    },
    onError: (e: unknown) => toast.err(`${t("pt.failCheck")}${e}`),
  });

  const upgrade = useMutation({
    mutationFn: () => ipc<Overview>("ponytail_update"),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: ["ponytail"] });
      toast.ok(r.local_version ? `${t("pt.upgraded")}${r.local_version}` : t("pt.upgradedNoVersion"));
    },
    onError: (e: unknown) => toast.err(`${t("pt.failUpgrade")}${e}`),
  });

  const o = ov.data;
  const day = dayOf(o?.upstream?.checked_at ?? o?.cached_at);
  const busy = mutate.isPending || check.isPending || upgrade.isPending;

  return (
    <div className="mt-3 border-t border-slate-800 pt-2">
      <div className="mb-1.5 flex items-center justify-between gap-2">
        <span className="text-sm font-medium text-ink-2">{t("pt.title")}</span>
        {ext.data?.hint && (
          <span className="min-w-0 truncate text-right text-2xs text-ink-4" title={ext.data.hint}>
            {ext.data.hint}
          </span>
        )}
      </div>

      {ov.isError ? (
        <QueryError inline title={t("pt.failStatus")} error={ov.error} onRetry={() => ov.refetch()} />
      ) : (
        <div className="space-y-1">
          {/* 版本三层各说各的：取不到一律 —，不拿上游冒充本地，也不拿本地冒充该端已装。
              首帧未到货时先说「载入中」，不让一排 — 假装成真实读数。 */}
          {ov.isPending && !o ? (
            <p className="text-2xs text-ink-4">{t("pt.loading")}</p>
          ) : (
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-2xs text-ink-3">
            <span title={t("pt.upstreamTip")}>
              {t("pt.upstream")}{" "}
              <span className="text-ink-2">{o?.upstream?.version ?? "—"}</span>
              {o?.upstream ? ` · ${o.upstream.source}` : ""}
            </span>
            <span title={t("pt.localTip")}>
              {t("pt.local")}{" "}
              <span className="text-ink-2">{o?.local_version ?? "—"}</span>
            </span>
            {o?.has_update ? (
              <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1 text-[var(--sem-busy-text)]">
                {t("pt.hasUpdate")}
              </span>
            ) : o?.upstream ? (
              <span className="rounded border border-emerald-500/40 bg-emerald-500/10 px-1 text-[var(--sem-ok-text)]">
                {t("pt.uptodate")}
              </span>
            ) : (
              <span className="text-ink-4">{t("pt.notChecked")}</span>
            )}
            {day && <span className="text-ink-4">{ts("pt.checkedAt", { at: day })}</span>}
            </div>
          )}

          {/* 检测失败只在这一行说清，不糊满屏：自动检查失败后端不会带 error，
              能到这里说明是用户手点的检查，此时必须把原因讲明白。 */}
          {o?.error && <p className="text-2xs text-[var(--sem-danger-text)]">{ts("pt.checkFailed", { why: o.error })}</p>}
          {o && !o.checked_out && <p className="text-2xs text-ink-4">{t("pt.noCheckout")}</p>}
          {o && (
            <p className="min-w-0 truncate text-2xs text-ink-4" title={o.checkout}>
              {t("pt.checkoutDir")} {o.checkout}
            </p>
          )}

          {ext.isError ? (
            <QueryError inline title={t("pt.failStatus")} error={ext.error} onRetry={() => ext.refetch()} />
          ) : (
            <>
              {rows.map((r) => (
                <div key={r.name} className="flex items-center gap-2 rounded px-1.5 py-1 hover:bg-slate-800/40">
                  <span className="min-w-0 truncate text-sm text-ink-2" title={`${r.name} · ${r.desc}`}>
                    {r.name}
                  </span>
                  <span
                    className={`ml-auto shrink-0 rounded border px-1 text-2xs ${EXT_STATE_STYLE[r.state]}`}
                    title={r.path}
                  >
                    {t(EXT_STATE_LABEL[r.state])}
                  </span>
                </div>
              ))}
              <div className="flex flex-wrap items-center gap-1.5 pt-0.5">
                <Button size="xs" variant="primary" disabled={busy} onClick={() => mutate.mutate("ext_install")}>
                  {mutate.isPending && mutate.variables === "ext_install"
                    ? t("pt.busy")
                    : installed
                      ? t("pt.reinstall")
                      : t("pt.install")}
                </Button>
                {installed && (
                  <Button size="xs" variant="ghost" disabled={busy} onClick={() => mutate.mutate("ext_uninstall")}>
                    {mutate.isPending && mutate.variables === "ext_uninstall" ? t("pt.uninstalling") : t("pt.uninstall")}
                  </Button>
                )}
                <Button size="xs" variant="ghost" disabled={busy} onClick={() => check.mutate()}>
                  {check.isPending ? t("pt.checking") : t("pt.check")}
                </Button>
                {o?.has_update && (
                  <Button size="xs" variant="ghost" disabled={busy} onClick={() => upgrade.mutate()}>
                    {upgrade.isPending ? t("pt.upgrading") : t("pt.upgrade")}
                  </Button>
                )}
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
