// 预设配置一键还原：读存档目录清单，全量或按代理还原。
// 原挂代理页「模型分配全景」顶部，用户 2026-11-16「把这个迁移到设置模块中」迁入设置页
// 网关运维节；与 GatewayOps 的「预设配置归档」（整目录归档）是两回事。
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card } from "../../components/ui";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";

/** 预设配置一键还原：读存档目录清单，全量或按代理还原（目录由后端下发，不写死）。 */
interface PresetEntry {
  app: string;
  target: string;
  source: string;
  source_exists: boolean;
  target_exists: boolean;
  differs: boolean;
}

export function PresetPanel() {
  const qc = useQueryClient();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const [confirm, setConfirm] = useState<{ app: string | null } | null>(null);
  const preset = useQuery({
    queryKey: ["preset"],
    queryFn: () => ipc<{ dir: string; entries: PresetEntry[] }>("preset_status"),
    enabled: open,
  });
  const restore = useMutation({
    mutationFn: (app: string | null) => ipc("preset_restore", { app }),
    onSuccess: (res: unknown) => {
      const r = res as { restored: unknown[]; snapshot: string };
      toast.ok(
        `已还原 ${r.restored.length} 个配置文件${r.snapshot ? `（原件快照：${r.snapshot}）` : ""}`,
      );
      setConfirm(null);
      qc.invalidateQueries({ queryKey: ["preset"] });
      qc.invalidateQueries({ queryKey: ["takeover"] });
      qc.invalidateQueries({ queryKey: ["matrix"] });
    },
    onError: (e) => toast.err(`还原失败：${e}`),
  });
  const entries = preset.data?.entries ?? [];
  const changed = entries.filter((e) => e.differs && e.source_exists).length;
  return (
    <Card className="mb-3">
      <div className="flex flex-wrap items-center gap-2">
        <button
          onClick={() => setOpen(!open)}
          className="px-2 py-1 text-xs text-ink-2 hover:text-ink-1"
        >
          {open ? "▾" : "▸"} 预设配置一键还原
        </button>
        <span className="text-xs text-ink-4">
          从 {preset.data?.dir ?? "~/.llm-switch/agent-configs"} 存档整套还原各代理配置
        </span>
        {open && !preset.isLoading && preset.isError && (
          <QueryError inline title="预设读取失败" error={preset.error} />
        )}
        {open && !preset.isLoading && !preset.isError && (
          <Button
            disabled={changed === 0 || restore.isPending}
            onClick={() => setConfirm({ app: null })}
          >
            {restore.isPending ? "还原中…" : `全部还原${changed > 0 ? `（${changed} 项有差异）` : "（无差异）"}`}
          </Button>
        )}
      </div>
      {open && (
        <ul className="mt-2 space-y-1">
          {entries.map((e) => (
            <li key={`${e.app}:${e.target}`} className="flex items-center gap-2 text-xs">
              <span
                className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                  !e.source_exists
                    ? "bg-[var(--dot-idle)]"
                    : e.differs
                      ? "bg-amber-400"
                      : "bg-emerald-400"
                }`}
              />
              <span className="w-20 shrink-0 text-ink-2">{e.app}</span>
              <span className="truncate text-ink-4" title={e.target}>{e.target}</span>
              <span className="shrink-0 text-ink-4">
                {!e.source_exists ? "缺存档" : e.differs ? "有差异" : "一致"}
              </span>
              {e.differs && e.source_exists && (
                <button
                  onClick={() => setConfirm({ app: e.app })}
                  className="ml-auto shrink-0 rounded border border-slate-700 px-1.5 py-0.5 text-ink-3 hover:text-cyan-300"
                >
                  还原此项
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      <ConfirmDialog
        open={!!confirm}
        title="还原预设配置"
        danger
        confirmLabel="还原"
        onClose={() => setConfirm(null)}
        onConfirm={() => restore.mutate(confirm?.app ?? null)}
      >
        将用 <b>{preset.data?.dir ?? "~/myllm/agent-configs"}</b> 里的存档
        {confirm?.app ? `覆盖 ${confirm.app} 的配置文件` : "覆盖全部代理的配置文件"}
        。现行文件会先快照到存档目录的 .restore-backups/ 下。确认执行？
      </ConfirmDialog>
    </Card>
  );
}
