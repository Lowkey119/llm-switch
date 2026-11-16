// 网关运维卡（UI-AUDIT-2 G-4 拆片）：日志级别、日志保留、预设配置一键还原。
// 从 pages/Settings.tsx 整块搬出，行为不变；搬出来的另一个理由是设置页已经长过
// 门禁线（>550 行），这三张卡与路由/熔断表单没有共享状态，本就该各自独立。
import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card, inputCls, selectCls } from "../../components/ui";
import { ConfirmDialog, Risk } from "../../components/ConfirmDialog";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";

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
      toast.ok("日志级别已更新（即时生效）");
      logLevel.refetch();
    },
    onError: (e) => toast.err(`日志级别设置失败：${e}`),
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
      toast.ok("日志保留策略已保存（下一轮维护起生效）");
      setRetDays(null);
      setRetMb(null);
      retention.refetch();
    },
    onError: (e) => toast.err(`保留策略保存失败：${e}`),
  });

  // 预设配置归档：一键还原把归档解回 ~/.llm-switch（覆盖前自动快照现行文件），
  // 一键备份把当前配置打成归档。归档有两个来源：自己备份的那份（存在即优先）与
  // 随包内嵌那份——原先只有「还原」，等于只能还原到出厂预设，用户调好的配置没处存。
  const archiveInfo = useQuery({
    queryKey: ["config-archive-info"],
    queryFn: () =>
      ipc<{
        bytes: number;
        entries: number;
        top: string[];
        source: "user" | "embedded" | "broken";
        updated_ms: number;
        path: string;
        history: number;
      }>("config_archive_info"),
  });
  const [confirmRestore, setConfirmRestore] = useState(false);
  const restoreArchive = useMutation({
    mutationFn: () =>
      ipc<{
        restored: number;
        backed_up: number;
        unchanged: number;
        skipped: { path: string; reason: string }[];
        backup_dir: string;
        source: string;
      }>("config_archive_restore"),
    onSuccess: (r) => {
      const skip = r.skipped.length ? `，跳过 ${r.skipped.length} 项` : "";
      const from = r.source === "user" ? "自己的备份" : "内置预设";
      toast.ok(
        `已用${from}还原 ${r.restored} 个文件（备份 ${r.backed_up}、未变 ${r.unchanged}）${skip}`,
      );
    },
    onError: (e) => toast.err(`还原失败：${e}`),
  });
  const backupArchive = useMutation({
    mutationFn: () =>
      ipc<{
        files: number;
        bytes: number;
        target: string;
        skipped: { path: string; reason: string }[];
        history: number;
      }>("config_archive_backup"),
    onSuccess: (r) => {
      const skip = r.skipped.length ? `，跳过 ${r.skipped.length} 项` : "";
      toast.ok(
        `已备份 ${r.files} 个文件（${(r.bytes / 1024 / 1024).toFixed(1)}MB）${skip}；` +
          `还原时优先用它，历史留 ${r.history} 份`,
      );
      archiveInfo.refetch();
    },
    onError: (e) => toast.err(`备份失败：${e}`),
  });
  const info = archiveInfo.data;
  const archiveDesc = info
    ? `${info.source === "user" ? "自己的备份" : info.source === "broken" ? "备份损坏（回落内置）" : "内置预设"} · ${(info.bytes / 1024 / 1024).toFixed(1)}MB · ${info.entries} 项` +
      (info.source === "user" && info.updated_ms
        ? ` · 更新于 ${new Date(info.updated_ms).toLocaleString()}`
        : "") +
      (info.source === "user" && info.history ? ` · 历史 ${info.history} 份` : "")
    : "归档读取失败";

  return (
    <>
      {/* 网关自身日志级别：级别热改控制落库详细度，日志页可查看。 */}
      <Card className="space-y-2 py-3">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
          <span className="font-medium text-ink-2">网关日志级别</span>
          <select
            className={selectCls}
            aria-label="网关日志级别"
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

      {/* 网关日志保留：天数 + 体积双淘汰，界面调参不再编辑配置文件。 */}
      <Card className="space-y-2 py-3">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
          <span className="font-medium text-ink-2">网关日志保留</span>
          <label className="flex items-center gap-1.5 text-ink-3">
            天数
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
            体积上限 MB
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
            保存
          </Button>
        </div>
        {retention.isError && (
          <QueryError inline title="读取保留策略失败" error={retention.error} onRetry={() => retention.refetch()} />
        )}
      </Card>

      {/* 预设配置一键还原：覆盖用户目录下的配置，属于不可逆动作，
          闸门走全站统一的 ConfirmDialog（UI-AUDIT-2 G-4）——原先这里是原生
          confirm()：样式与主题无关、文案里塞满 \n，还绕过了「破坏性动作统一闸门」。 */}
      <Card className="space-y-2 py-3">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
          <span className="font-medium text-ink-2">预设配置归档</span>
          <Button
            disabled={backupArchive.isPending}
            onClick={() => backupArchive.mutate()}
            title="把当前 ~/.llm-switch 里的配置打成归档；还原时优先用它"
          >
            {backupArchive.isPending ? "备份中…" : "备份当前配置"}
          </Button>
          <Button
            variant="danger"
            disabled={restoreArchive.isPending}
            onClick={() => setConfirmRestore(true)}
          >
            {restoreArchive.isPending ? "还原中…" : "还原"}
          </Button>
          <span className="text-ink-4">覆盖前自动快照到 config-restore-backups/</span>
        </div>
        {backupArchive.data && (
          <p className="text-xs text-ink-3">
            本次备份 {backupArchive.data.files} 个文件 ·{" "}
            {(backupArchive.data.bytes / 1024 / 1024).toFixed(1)}MB ·{" "}
            {backupArchive.data.target}
            {backupArchive.data.skipped.length
              ? `；跳过 ${backupArchive.data.skipped.length} 项`
              : ""}
          </p>
        )}
        {backupArchive.isError && (
          <p className="text-xs text-[var(--sem-danger-text)]">
            备份失败：{String(backupArchive.error)}
          </p>
        )}
        {restoreArchive.data && (
          <p className="text-xs text-ink-3">
            本次：还原 {restoreArchive.data.restored}、备份 {restoreArchive.data.backed_up}、未变{" "}
            {restoreArchive.data.unchanged}
            {restoreArchive.data.backup_dir && `；快照目录 ${restoreArchive.data.backup_dir}`}
          </p>
        )}
        {restoreArchive.isError && (
          <p className="text-xs text-[var(--sem-danger-text)]">还原失败：{String(restoreArchive.error)}</p>
        )}
        {confirmRestore && (
          <ConfirmDialog
            open
            title={
              info?.source === "user"
                ? "用自己的备份覆盖当前配置？"
                : "用随包内嵌的预设覆盖当前配置？"
            }
            confirmLabel="确认还原"
            danger
            onClose={() => setConfirmRestore(false)}
            onConfirm={() => restoreArchive.mutate()}
          >
            将覆盖 <b>~/.llm-switch</b> 下的 config.yaml、agent-configs、scripts、skills、
            config_snapshots。被覆盖的现行文件会先自动快照，误还原可找回；
            <Risk>正在运行的网关会读到新配置</Risk>。
            {info?.source === "user" ? (
              <>
                <br />
                这次用的是<b>自己的备份</b>（{archiveDesc}）。
              </>
            ) : (
              <>
                <br />
                这次用的是<b>随包内嵌的预设</b>——先「备份当前配置」再还原，调好的东西才找得回来。
              </>
            )}
          </ConfirmDialog>
        )}
      </Card>
    </>
  );
}
