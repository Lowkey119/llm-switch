// 单个代理的接管备份（原设置页的全局列表按当前代理过滤）。
// 备份是逐字节回滚的唯一凭据，删除不可逆 —— 闸门矩阵要求先确认。
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../../components/ui";
import { ConfirmDialog, Risk } from "../../components/ConfirmDialog";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import { t, tf } from "../../lib/i18n";
import { useTakeoverAct } from "./useTakeoverAct";
import { when, type BackupEntry } from "./types";

export function BackupSection({ app }: { app: string }) {
  const [open, setOpen] = useState(false);
  const [delId, setDelId] = useState<string | null>(null);
  const act = useTakeoverAct();
  const backups = useQuery({
    queryKey: ["takeover-backups"],
    queryFn: () => ipc<{ backups: BackupEntry[] }>("takeover_backups"),
    enabled: open,
  });
  const rows = backups.data?.backups.filter((b) => b.app === app) ?? [];
  const busy = act.isPending;

  return (
    <div className="mt-3 border-t border-slate-800 pt-2">
      <button
        className="text-sm text-ink-3 underline-offset-2 hover:underline"
        onClick={() => setOpen(!open)}
      >
        {open ? t("ag.collapseBackups") : t("ag.viewBackups")}
      </button>
      {open && (
        <ul className="mt-2 space-y-1">
          {backups.isLoading && <li className="text-sm text-ink-4">{t("common.loading")}</li>}
          {/* 读取失败 ≠ 没有备份：原先失败时会落到「还没有备份」，误导用户 */}
          {backups.isError && (
            <li>
              <QueryError inline title={t("ag.backupReadFail")} error={backups.error} onRetry={() => backups.refetch()} />
            </li>
          )}
          {rows.length === 0 && !backups.isLoading && !backups.isError && (
            <li className="text-sm text-ink-4">{t("ag.noBackups")}</li>
          )}
          {rows.map((b) => (
            <li
              key={b.backup_id}
              className="flex flex-wrap items-center gap-2 rounded-md border border-slate-800 px-2 py-1 text-sm"
            >
              <span className="text-ink-4">{when(b.created_at)}</span>
              <span className="break-all font-mono text-ink-4">{b.files.join("、")}</span>
              <span className="ml-auto flex gap-2">
                <Button
                  variant="ghost"
                  disabled={busy}
                  onClick={() => act.mutate({ cmd: "takeover_restore_backup", args: { id: b.backup_id } })}
                >
                  {t("set.restore")}
                </Button>
                <Button variant="ghost" disabled={busy} onClick={() => setDelId(b.backup_id)}>
                  {t("common.delete")}
                </Button>
              </span>
            </li>
          ))}
        </ul>
      )}
      {delId && (
        <ConfirmDialog
          open
          title={t("ag.delBackupTitle")}
          irreversible
          onClose={() => setDelId(null)}
          onConfirm={() => act.mutate({ cmd: "takeover_delete_backup", args: { id: delId } })}
          confirmLabel={t("ag.delBackupConfirm")}
          danger
        >
          {tf("ag.delBackupBody", { risk: <Risk>{t("ag.delBackupRisk")}</Risk> })}
        </ConfirmDialog>
      )}
    </div>
  );
}
