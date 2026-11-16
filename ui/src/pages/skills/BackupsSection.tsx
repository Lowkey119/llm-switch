// 「备份」页签：卸载技能时留下的底，可还原；删除是不可逆动作，走确认闸门。
// 计划 §6.1 自 Skills.tsx 拆出，行为不变。
import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card } from "../../components/ui";
import { ConfirmDialog, Risk } from "../../components/ConfirmDialog";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import type { BackupEntry } from "./types";
import { theadCls } from "../../components/Table";

export function BackupsSection({ refresh }: { refresh: () => void }) {
  const toast = useToast();
  const [delId, setDelId] = useState<string | null>(null);
  const backups = useQuery({
    queryKey: ["skill-backups"],
    queryFn: () => ipc<BackupEntry[]>("list_skill_backups"),
  });
  const restore = useMutation({
    mutationFn: (id: string) => ipc("restore_skill_backup", { id }),
    onSuccess: () => { refresh(); toast.ok("已还原"); },
    onError: (e) => toast.err(`还原失败：${e}`),
  });
  const delBackup = useMutation({
    mutationFn: (id: string) => ipc("delete_skill_backup", { id }),
    onSuccess: refresh,
    onError: (e) => toast.err(`删除备份失败：${e}`),
  });
  // BUG-6：主动发起一次备份（原来只有卸载时自动留底，没有手动入口）。
  const backupNow = useMutation({
    mutationFn: () => ipc<{ backed_up: number }>("backup_skills_now"),
    onSuccess: (r) => {
      refresh();
      toast.ok(`已备份 ${r.backed_up} 项技能`);
    },
    onError: (e) => toast.err(`备份失败：${e}`),
  });

  return (
    <Card>
      <div className="mb-2 flex items-center justify-between gap-2">
        <h2 className="text-sm font-semibold">卸载备份（保留最近 20 份）</h2>
        <Button variant="primary" onClick={() => backupNow.mutate()} disabled={backupNow.isPending}>
          {backupNow.isPending ? "备份中…" : "备份"}
        </Button>
      </div>
      {backups.isError ? (
        <QueryError inline title="读取备份列表失败" error={backups.error} onRetry={() => backups.refetch()} />
      ) : !backups.data?.length ? (
        <p className="text-xs text-ink-4">暂无备份。点右上角「备份」立即快照，卸载技能时也会自动留底于此。</p>
      ) : (
        <table className="w-full text-xs">
          <thead className={theadCls}>
            <tr className="border-b border-slate-800 text-left text-ink-4">
              <th className="py-1.5 font-medium">技能</th>
              <th className="font-medium">时间</th>
              <th className="text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {backups.data.map((b) => (
              <tr key={b.backup_id} className="border-b border-slate-800/60">
                <td className="py-1.5">
                  <span className="text-ink-2">{b.name}</span>
                  {/* dir 与 name 同源时不再重复显示（用户 2026-11-16 截图：整列双写） */}
                  {b.dir && b.dir !== b.name && <span className="ml-2 text-ink-4">{b.dir}</span>}
                </td>
                <td className="py-1.5 text-ink-3">{new Date(b.created_at).toLocaleString()}</td>
                <td className="py-1.5 text-right whitespace-nowrap">
                  <Button variant="primary" onClick={() => restore.mutate(b.backup_id)}>还原</Button>{" "}
                  <Button variant="danger" onClick={() => setDelId(b.backup_id)}>删除</Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {delId && (
        <ConfirmDialog
          open
          title="删除该技能备份？"
          onClose={() => setDelId(null)}
          onConfirm={() => delBackup.mutate(delId)}
          confirmLabel="确认删除"
          danger
        >
          <Risk>备份删除后不可恢复</Risk>，这个技能就没有回滚凭据了；当前安装的技能本身不受影响。
        </ConfirmDialog>
      )}
    </Card>
  );
}
