// 网关运维卡（UI-AUDIT-2 G-4 拆片）：日志级别、日志保留、预设配置一键还原。
// 从 pages/Settings.tsx 整块搬出，行为不变；搬出来的另一个理由是设置页已经长过
// 门禁线（>550 行），这三张卡与路由/熔断表单没有共享状态，本就该各自独立。
import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card, inputCls, selectCls } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";

export function GatewayOps() {
  const toast = useToast();

  // 网关日志级别（从日志页搬来）：读当前生效值，选择即热更保存。
  const logLevel = useQuery({
    queryKey: ["log-level"],
    queryFn: () => ipc<string>("get_log_level"),
  });
  const setLevel = useMutation({
    mutationFn: (lv: string) => ipc("set_log_level", { level: lv }),
    onSuccess: () => {
      toast.ok(t("set.levelUpdated"));
      logLevel.refetch();
    },
    onError: (e) => toast.err(ts("set.levelFail", { e: String(e) })),
  });

  // 网关日志保留：settings 覆盖，保存即下一轮维护 tick 生效。
  // 草稿值 null = 未编辑（显示生效值）；保存后清空草稿回到生效值显示。
  const retention = useQuery({
    queryKey: ["logs-retention"],
    queryFn: () => ipc<{ retention_days: number; max_mb: number }>("logs_retention"),
  });
  const [retDays, setRetDays] = useState<number | null>(null);
  const [retMb, setRetMb] = useState<number | null>(null);
  const setRetention = useMutation({
    mutationFn: () =>
      ipc("set_logs_retention", {
        retention_days: retDays ?? retention.data?.retention_days ?? 7,
        max_mb: retMb ?? retention.data?.max_mb ?? 50,
      }),
    onSuccess: () => {
      toast.ok(t("set.retentionSaved"));
      setRetDays(null);
      setRetMb(null);
      retention.refetch();
    },
    onError: (e) => toast.err(ts("set.retentionFail", { e: String(e) })),
  });

  // 预设配置归档 / 一键还原已并入 pages/settings/PresetPanel.tsx（fix:merge-preset-cards）：
  // 两个都叫「还原」却还原回不同东西的两张卡，合并成一张、来源写在按钮上。

  return (
    <>
      {/* 网关自身日志级别：级别热改控制落库详细度，日志页可查看。
          两列排版（用户 2026-11-21）：标题占左半，下拉占右半，不再拉通栏。 */}
      <Card className="space-y-2 py-3">
        <div className="grid grid-cols-2 items-center gap-3 text-sm">
          <span className="font-medium text-ink-2">{t("set.logLevel")}</span>
          <select
            className={selectCls}
            aria-label={t("set.logLevel")}
            value={logLevel.data ?? ""}
            onChange={(e) => setLevel.mutate(e.target.value)}
            disabled={setLevel.isPending}
          >
            {["error", "warn", "info", "debug", "trace"].map((lv) => (
              <option key={lv} value={lv}>
                {lv}
              </option>
            ))}
          </select>
        </div>
      </Card>

      {/* 网关日志保留：天数 + 体积双淘汰，界面调参不再编辑配置文件。同款两列排版。 */}
      <Card className="space-y-2 py-3">
        <div className="grid grid-cols-2 items-center gap-3 text-sm">
          <span className="font-medium text-ink-2">{t("set.retentionTitle")}</span>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <label className="flex items-center gap-1.5 text-ink-3">
              {t("set.days")}
              <input
                type="number"
                min={1}
                max={3650}
                className={`${inputCls} w-20`}
                value={retDays ?? retention.data?.retention_days ?? 7}
                onChange={(e) => setRetDays(Number(e.target.value))}
              />
            </label>
            <label className="flex items-center gap-1.5 text-ink-3">
              {t("set.sizeMb")}
              <input
                type="number"
                min={1}
                max={1024}
                className={`${inputCls} w-20`}
                value={retMb ?? retention.data?.max_mb ?? 50}
                onChange={(e) => setRetMb(Number(e.target.value))}
              />
            </label>
            <Button disabled={setRetention.isPending} onClick={() => setRetention.mutate()}>
              {t("dlg.save")}
            </Button>
          </div>
        </div>
        {retention.isError && (
          <QueryError inline title={t("set.retentionReadFail")} error={retention.error} onRetry={() => retention.refetch()} />
        )}
      </Card>

    </>
  );
}
