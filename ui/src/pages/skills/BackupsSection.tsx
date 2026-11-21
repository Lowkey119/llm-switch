// 「备份」页签：卸载技能时留下的底，可还原；删除是不可逆动作，走确认闸门。
// 计划 §6.1 自 Skills.tsx 拆出，行为不变。
import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card } from "../../components/ui";
import { ConfirmDialog, Risk } from "../../components/ConfirmDialog";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";
import type { BackupEntry } from "./types";
import { theadCls } from "../../components/Table";

export function BackupsSection({ refresh, query = "" }: { refresh: () => void; query?: string }) {
  const toast = useToast();
  const [delId, setDelId] = useState<string | null>(null);
  const backups = useQuery({
    queryKey: ["skill-backups"],
    queryFn: () => ipc<BackupEntry[]>("list_skill_backups"),
  });
  const restore = useMutation({
    mutationFn: (id: string) => ipc("restore_skill_backup", { id }),
    onSuccess: () => { refresh(); toast.ok(t("sk.restored")); },
    onError: (e) => toast.err(ts("sk.restoreFail", { e: String(e) })),
  });
  const delBackup = useMutation({
    mutationFn: (id: string) => ipc("delete_skill_backup", { id }),
    onSuccess: refresh,
    onError: (e) => toast.err(ts("sk.delBackupFail", { e: String(e) })),
  });
  // BUG-6：主动发起一次备份（原来只有卸载时自动留底，没有手动入口）。
  const backupNow = useMutation({
    mutationFn: () => ipc<{ backed_up: number }>("backup_skills_now"),
    onSuccess: (r) => {
      refresh();
      toast.ok(ts("sk.backedUpN", { n: r.backed_up }));
    },
    onError: (e) => toast.err(ts("sk.backupFail", { e: String(e) })),
  });

  // 共用搜索框在本页签只按技能名 / 目录名过滤备份清单。
  const low = query.trim().toLowerCase();
  const rows = (backups.data ?? []).filter((b) => !low || `${b.name} ${b.dir}`.toLowerCase().includes(low));

  return (
    <Card>
      <div className="mb-2 flex items-center justify-between gap-2">
        <h2 className="text-base font-semibold">{t("sk.backupTitle")}</h2>
        <Button variant="primary" onClick={() => backupNow.mutate()} disabled={backupNow.isPending}>
          {backupNow.isPending ? t("sk.backingUp") : t("hl.backupLabel")}
        </Button>
      </div>
      {backups.isError ? (
        <QueryError inline title={t("sk.backupReadFail")} error={backups.error} onRetry={() => backups.refetch()} />
      ) : !rows.length ? (
        <p className="text-sm text-ink-4">
          {low && backups.data?.length ? ts("sk.backupNoMatch", { q: query.trim() }) : t("sk.backupEmpty")}
        </p>
      ) : (
        <table className="w-full text-xs">
          <thead className={theadCls}>
            <tr className="border-b border-slate-800 text-left text-ink-4">
              <th className="py-1.5 font-medium">{t("sk.colSkill")}</th>
              <th className="font-medium">{t("logs.th.ts")}</th>
              <th className="text-right font-medium">{t("sk.colAction")}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((b) => (
              <tr key={b.backup_id} className="border-b border-slate-800/60">
                <td className="py-1.5">
                  <span className="text-ink-2">{b.name}</span>
                  {/* dir 与 name 同源时不再重复显示（用户 2026-11-16 截图：整列双写） */}
                  {b.dir && b.dir !== b.name && <span className="ml-2 text-ink-4">{b.dir}</span>}
                </td>
                <td className="py-1.5 text-ink-3">{new Date(b.created_at).toLocaleString()}</td>
                <td className="py-1.5 text-right whitespace-nowrap">
                  <Button variant="primary" onClick={() => restore.mutate(b.backup_id)}>{t("set.restore")}</Button>{" "}
                  <Button variant="danger" onClick={() => setDelId(b.backup_id)}>{t("common.delete")}</Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {delId && (
        <ConfirmDialog
          open
          title={t("sk.delBackupTitle")}
          irreversible
          onClose={() => setDelId(null)}
          onConfirm={() => delBackup.mutate(delId)}
          confirmLabel={t("ag.delBackupConfirm")}
          danger
        >
          {t("sk.delBackupBody")}<Risk>{t("sk.delBackupRisk")}</Risk>{t("sk.delBackupTail")}
        </ConfirmDialog>
      )}
    </Card>
  );
}
