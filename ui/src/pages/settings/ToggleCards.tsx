// 设置页三张开关卡（从 SettingsPage 原样搬出）：接单开关 / 多行浮动面板 /
// 资源悬浮小窗。各自 query 与 mutation 跟着卡走（谁用谁持有），本组件即挂载即拉取。
// 开关类卡片统一走 SettingRow：左标题+说明、右状态+开关，下沿补充行走 children
// （自动带分隔线）。三张卡结构同源，改一处全站生效。
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { QueryError } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { SettingRow, Toggle } from "../../components/ui";
import { METRIC_KEYS } from "../../components/SysFloat";
import { ipc, type ShellStatus } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";

export function ToggleCards() {
  const qc = useQueryClient();
  const toast = useToast();
  const status = useQuery({ queryKey: ["status"], queryFn: () => ipc<ShellStatus>("get_status") });
  const pause = useMutation({
    mutationFn: (paused: boolean) => ipc("set_paused", { paused }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["status"] }),
    onError: (e) => toast.err(ts("set.toggleFail", { e: String(e) })),
  });

  // 浮窗只有桌面版存在（浏览器里打开这份 UI 时没有 Tauri 运行时），
  // 所以查询直接禁掉，卡片退化成一句说明，而不是转圈报错。
  const hasTauri =
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in (window as object);
  const floating = useQuery({
    queryKey: ["float-visible"],
    queryFn: () => ipc<boolean>("float_is_visible"),
    enabled: hasTauri,
  });
  const toggleFloat = useMutation({
    mutationFn: (visible: boolean) => ipc("float_set_visible", { visible }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["float-visible"] }),
    onError: (e) => toast.err(ts("set.floatReadFail", { e: String(e) })),
  });

  // 流量自动浮窗（切片 11）：有 API 流量自动开浮动监视器、空闲即收，即时生效。
  const autoFloat = useQuery({
    queryKey: ["auto-float"],
    queryFn: () => ipc<boolean>("auto_float_get"),
  });
  const setAutoFloat = useMutation({
    mutationFn: (on: boolean) => ipc("auto_float_set", { on }),
    onSuccess: () => {
      toast.ok(t("set.autoFloatSaved"));
      autoFloat.refetch();
    },
    onError: (e) => toast.err(ts("set.autoFloatFail", { e: String(e) })),
  });

  // 速率线自动弹出（#46）：代理一有请求就点亮它家的速率线；设置页与托盘共用同一偏好。
  const rlAuto = useQuery({
    queryKey: ["rateline-auto"],
    queryFn: () => ipc<boolean>("rateline_auto_get"),
  });
  const setRlAuto = useMutation({
    mutationFn: (on: boolean) => ipc("rateline_auto_set", { on }),
    onSuccess: () => {
      toast.ok(t("set.rlAutoSaved"));
      rlAuto.refetch();
    },
    onError: (e) => toast.err(ts("set.rlAutoFail", { e: String(e) })),
  });

  // 资源悬浮小窗（切片 13）：显隐 + 指标勾选，偏好落 settings（gui/sysfloat.rs）。
  const sysFloat = useQuery({
    queryKey: ["sysfloat-visible"],
    queryFn: () => ipc<boolean>("sysfloat_get_visible"),
  });
  const setSysFloat = useMutation({
    mutationFn: (on: boolean) => ipc("sysfloat_set_visible", { visible: on }),
    onSuccess: () => sysFloat.refetch(),
    onError: (e) => toast.err(ts("set.sysFloatToggleFail", { e: String(e) })),
  });
  const sysMetrics = useQuery({
    queryKey: ["sysfloat-metrics"],
    queryFn: () => ipc<string[]>("sysfloat_metrics_get"),
  });
  const setSysMetrics = useMutation({
    mutationFn: (list: string[]) => ipc("sysfloat_metrics_set", { list }),
    onSuccess: () => sysMetrics.refetch(),
    onError: (e) => toast.err(ts("set.metricsSaveFail", { e: String(e) })),
  });

  return (
    /* 开关卡两列排版（用户 2026-11-21）：窄开关两两并排，资源悬浮小窗带指标勾选独占一行。 */
    <div className="grid grid-cols-2 gap-3">
      <SettingRow
        title={t("set.acceptTitle")}
        status={
          <>
            <span className={`whitespace-nowrap text-sm ${status.isError ? "text-[var(--sem-danger-text)]" : "text-ink-3"}`}>
              {status.isError
                ? ts("set.statusReadFail", { e: String(status.error?.message ?? status.error) })
                : status.data?.paused
                  ? t("top.paused")
                  : t("set.accepting")}
            </span>
            {status.isError && (
              <QueryError inline title={t("set.statusReadFailT")} error={status.error} onRetry={() => status.refetch()} />
            )}
          </>
        }
        control={
          <Toggle
            checked={!status.data?.paused}
            disabled={pause.isPending || !status.data}
            onChange={(v) => pause.mutate(!v)}
          />
        }
      />

      <SettingRow
        title={t("set.floatTitle")}
        status={
          <>
            <span className={`whitespace-nowrap text-sm ${floating.isError ? "text-[var(--sem-danger-text)]" : "text-ink-3"}`}>
              {hasTauri
                ? floating.isError
                  ? ts("set.statusReadFail", { e: String(floating.error?.message ?? floating.error) })
                  : floating.data
                    ? t("set.showing")
                    : t("set.collapsed")
                : t("set.desktopOnly")}
            </span>
            {hasTauri && floating.isError && (
              <QueryError inline title={t("set.floatReadFail")} error={floating.error} onRetry={() => floating.refetch()} />
            )}
          </>
        }
        control={
          <Toggle
            checked={Boolean(floating.data)}
            disabled={!hasTauri || toggleFloat.isPending}
            onChange={(v) => toggleFloat.mutate(v)}
          />
        }
      >
      </SettingRow>

      {/* 两个自动弹出开关升为独立卡（用户 2026-11-21 两列排版）：从前挤在浮动面板卡的下沿，
          现在一家一格，与其余开关同形。流量自动浮窗（切片 11）/ 速率线自动弹出（#46）。 */}
      <SettingRow
        title={t("set.autoFloatTitle")}
        status={
          autoFloat.isError ? (
            <QueryError inline title={t("set.switchReadFail")} error={autoFloat.error} onRetry={() => autoFloat.refetch()} />
          ) : undefined
        }
        control={
          <Toggle
            checked={Boolean(autoFloat.data)}
            disabled={autoFloat.isPending || setAutoFloat.isPending}
            onChange={(v) => setAutoFloat.mutate(v)}
          />
        }
      />
      <SettingRow
        title={t("set.rateLineAutoTitle")}
        status={
          rlAuto.isError ? (
            <QueryError inline title={t("set.switchReadFail")} error={rlAuto.error} onRetry={() => rlAuto.refetch()} />
          ) : undefined
        }
        control={
          <Toggle
            checked={Boolean(rlAuto.data)}
            disabled={rlAuto.isPending || setRlAuto.isPending}
            onChange={(v) => setRlAuto.mutate(v)}
          />
        }
      />

      {/* 资源悬浮小窗（切片 13，借鉴 unsloth 悬浮监控窗设计）：置顶小窗盯两台机器；
          带指标勾选行，独占整行（col-span-2）。 */}
      <div className="col-span-2">
      <SettingRow
        title={t("set.sysFloatTitle")}
        status={
          sysFloat.isError ? (
            <QueryError inline title={t("set.sysFloatReadFail")} error={sysFloat.error} onRetry={() => sysFloat.refetch()} />
          ) : undefined
        }
        control={
          <Toggle
            checked={Boolean(sysFloat.data)}
            disabled={sysFloat.isPending || setSysFloat.isPending}
            onChange={(v) => setSysFloat.mutate(v)}
          />
        }
      >
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-sm text-ink-3">
          <span className="text-ink-4">{t("set.showMetrics")}</span>
          {METRIC_KEYS.map((m) => (
            <label key={m.key} className="flex cursor-pointer items-center gap-1.5">
              <input
                type="checkbox"
                checked={(sysMetrics.data ?? []).includes(m.key)}
                disabled={sysMetrics.isPending || setSysMetrics.isPending}
                onChange={(e) => {
                  const cur = new Set(sysMetrics.data ?? []);
                  if (e.target.checked) cur.add(m.key);
                  else cur.delete(m.key);
                  setSysMetrics.mutate(METRIC_KEYS.map((k) => k.key).filter((k) => cur.has(k)));
                }}
              />
              {t(m.labelKey)}
            </label>
          ))}
        </div>
      </SettingRow>
      </div>
    </div>
  );
}
