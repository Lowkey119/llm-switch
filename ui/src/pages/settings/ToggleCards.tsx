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

export function ToggleCards() {
  const qc = useQueryClient();
  const toast = useToast();
  const status = useQuery({ queryKey: ["status"], queryFn: () => ipc<ShellStatus>("get_status") });
  const pause = useMutation({
    mutationFn: (paused: boolean) => ipc("set_paused", { paused }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["status"] }),
    onError: (e) => toast.err(`切换失败：${e}`),
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
    onError: (e) => toast.err(`浮窗切换失败：${e}`),
  });

  // 流量自动浮窗（切片 11）：有 API 流量自动开浮动监视器、空闲即收，即时生效。
  const autoFloat = useQuery({
    queryKey: ["auto-float"],
    queryFn: () => ipc<boolean>("auto_float_get"),
  });
  const setAutoFloat = useMutation({
    mutationFn: (on: boolean) => ipc("auto_float_set", { on }),
    onSuccess: () => {
      toast.ok("流量自动浮窗已更新（即时生效）");
      autoFloat.refetch();
    },
    onError: (e) => toast.err(`流量自动浮窗设置失败：${e}`),
  });

  // 速率线自动弹出（#46）：代理一有请求就点亮它家的速率线；设置页与托盘共用同一偏好。
  const rlAuto = useQuery({
    queryKey: ["rateline-auto"],
    queryFn: () => ipc<boolean>("rateline_auto_get"),
  });
  const setRlAuto = useMutation({
    mutationFn: (on: boolean) => ipc("rateline_auto_set", { on }),
    onSuccess: () => {
      toast.ok("速率线自动弹出已更新（即时生效）");
      rlAuto.refetch();
    },
    onError: (e) => toast.err(`速率线自动弹出设置失败：${e}`),
  });

  // 资源悬浮小窗（切片 13）：显隐 + 指标勾选，偏好落 settings（gui/sysfloat.rs）。
  const sysFloat = useQuery({
    queryKey: ["sysfloat-visible"],
    queryFn: () => ipc<boolean>("sysfloat_get_visible"),
  });
  const setSysFloat = useMutation({
    mutationFn: (on: boolean) => ipc("sysfloat_set_visible", { visible: on }),
    onSuccess: () => sysFloat.refetch(),
    onError: (e) => toast.err(`资源小窗开关失败：${e}`),
  });
  const sysMetrics = useQuery({
    queryKey: ["sysfloat-metrics"],
    queryFn: () => ipc<string[]>("sysfloat_metrics_get"),
  });
  const setSysMetrics = useMutation({
    mutationFn: (list: string[]) => ipc("sysfloat_metrics_set", { list }),
    onSuccess: () => sysMetrics.refetch(),
    onError: (e) => toast.err(`指标勾选保存失败：${e}`),
  });

  return (
    <>
      <SettingRow
        title="接单开关"
        status={
          <>
            <span className={`whitespace-nowrap text-xs ${status.isError ? "text-[var(--sem-danger-text)]" : "text-ink-3"}`}>
              {status.isError
                ? `状态读取失败：${String(status.error?.message ?? status.error)}`
                : status.data?.paused
                  ? "已暂停接单"
                  : "正在接单"}
            </span>
            {status.isError && (
              <QueryError inline title="状态读取失败" error={status.error} onRetry={() => status.refetch()} />
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
        title="多行浮动面板"
        status={
          <>
            <span className={`whitespace-nowrap text-xs ${floating.isError ? "text-[var(--sem-danger-text)]" : "text-ink-3"}`}>
              {hasTauri
                ? floating.isError
                  ? `状态读取失败：${String(floating.error?.message ?? floating.error)}`
                  : floating.data
                    ? "显示中"
                    : "已收起"
                : "仅桌面版可用"}
            </span>
            {hasTauri && floating.isError && (
              <QueryError inline title="浮窗状态读取失败" error={floating.error} onRetry={() => floating.refetch()} />
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
        {/* UI-12（用户 2026-11-16）：两个自动弹出开关并排两列，一行一个太占纵向。 */}
        <div className="grid grid-cols-2 gap-4 border-t border-slate-800/60 pt-3">
          {/* 流量自动浮窗（切片 11，借鉴 unsloth「API 监控」）：有流量自动开、空闲即收 */}
          <div className="flex min-w-0 items-center gap-4">
            <div className="min-w-0 flex-1">
              <h3 className="text-sm font-semibold">收到 API 流量自动弹出</h3>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {autoFloat.isError && (
                <QueryError inline title="开关读取失败" error={autoFloat.error} onRetry={() => autoFloat.refetch()} />
              )}
              <Toggle
                checked={Boolean(autoFloat.data)}
                disabled={autoFloat.isPending || setAutoFloat.isPending}
                onChange={(v) => setAutoFloat.mutate(v)}
              />
            </div>
          </div>
          {/* 速率线自动弹出（#46）：各代理速率线的自动点亮开关，与托盘勾标同源 */}
          <div className="flex min-w-0 items-center gap-4">
            <div className="min-w-0 flex-1">
              <h3 className="text-sm font-semibold">代理流量自动点亮速率线</h3>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              {rlAuto.isError && (
                <QueryError inline title="开关读取失败" error={rlAuto.error} onRetry={() => rlAuto.refetch()} />
              )}
              <Toggle
                checked={Boolean(rlAuto.data)}
                disabled={rlAuto.isPending || setRlAuto.isPending}
                onChange={(v) => setRlAuto.mutate(v)}
              />
            </div>
          </div>
        </div>
      </SettingRow>

      {/* 资源悬浮小窗（切片 13，借鉴 unsloth 悬浮监控窗设计）：置顶小窗盯两台机器 */}
      <SettingRow
        title="资源悬浮小窗"
        status={
          sysFloat.isError ? (
            <QueryError inline title="小窗状态读取失败" error={sysFloat.error} onRetry={() => sysFloat.refetch()} />
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
        <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-ink-3">
          <span className="text-ink-4">显示指标</span>
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
              {m.label}
            </label>
          ))}
        </div>
      </SettingRow>
    </>
  );
}
