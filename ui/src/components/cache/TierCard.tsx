// 分层预算卡（从 CacheCard 原样搬出）：settings 覆盖 config.yaml，下次网关重启生效。
// 热层/冷层预算表单 + save_cache_tier mutation 全在本组件里（谁用谁持有）。
import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { QueryError } from "../QueryError";
import { useToast } from "../Toast";
import { Button, Card, Toggle } from "../ui";
import { ipc } from "../../lib/ipc";

/** 分层预算（get_cache_tier）：生效值 + yaml 出厂值 + 在跑实例实况。 */
export interface CacheTier {
  effective: { memory_mb: number; disk_gb: number; persist: boolean };
  yaml: { memory_mb: number; disk_gb: number; persist: boolean };
  live: { memory_budget_bytes: number; disk_budget_bytes: number };
  restart_pending: boolean;
}

export function TierCard({ onSaved }: { onSaved?: () => void }) {
  const toast = useToast();
  // 分层预算：settings 覆盖 yaml，改动下次网关重启才换到在跑实例。
  const tier = useQuery({
    queryKey: ["cache-tier"],
    queryFn: () => ipc<CacheTier>("get_cache_tier"),
  });
  const [tierForm, setTierForm] = useState<{ memory_mb: string; disk_gb: string } | null>(null);
  const [tierPersist, setTierPersist] = useState(true);
  const tierInit = useRef(false);
  useEffect(() => {
    if (tierInit.current || !tier.data) return;
    tierInit.current = true;
    const e = tier.data.effective;
    setTierForm({ memory_mb: String(e.memory_mb), disk_gb: String(e.disk_gb) });
    setTierPersist(e.persist);
  }, [tier.data]);
  const saveTier = useMutation({
    mutationFn: () => {
      if (!tierForm) return Promise.reject("预算表单未就绪");
      const checks: Record<string, string> = {};
      if (tierForm.memory_mb !== "") {
        const n = Number(tierForm.memory_mb);
        if (!Number.isInteger(n) || n < 1) return Promise.reject("热层预算须是 ≥1 的整数 MB（留空 = 跟随配置文件）");
        checks.memory_mb = String(n);
      } else checks.memory_mb = "";
      if (tierForm.disk_gb !== "") {
        const n = Number(tierForm.disk_gb);
        if (!Number.isInteger(n) || n < 0) return Promise.reject("冷层上限须是 ≥0 的整数 GB（0 = 关闭，留空 = 跟随配置文件）");
        checks.disk_gb = String(n);
      } else checks.disk_gb = "";
      return ipc("save_cache_tier", {
        values: { ...checks, persist: tierPersist ? "true" : "false" },
      });
    },
    onSuccess: () => {
      toast.ok("分层预算已保存（下次网关重启生效）");
      tier.refetch();
      onSaved?.();
    },
    onError: (e) => toast.err(`分层预算保存失败：${e}`),
  });

  return (
    <Card className="space-y-2 py-3">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
        <span className="font-medium text-ink-2">分层预算</span>
        {tier.data?.restart_pending && (
          <span className="text-[var(--sem-busy-text)]">已保存，等网关重启生效</span>
        )}
      </div>
      {tier.isError ? (
        <QueryError title="读取分层预算失败" error={tier.error} onRetry={() => tier.refetch()} />
      ) : (
        tierForm && (
          <div className="flex flex-wrap items-center gap-2 text-xs">
            <label className="flex items-center gap-1.5 text-ink-3">
              热层 MB
              <input
                type="number"
                min={1}
                value={tierForm.memory_mb}
                onChange={(e) => setTierForm({ ...tierForm, memory_mb: e.target.value })}
                className="w-24 rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-xs"
              />
            </label>
            <label className="flex items-center gap-1.5 text-ink-3">
              冷层 GB
              <input
                type="number"
                min={0}
                value={tierForm.disk_gb}
                title="0 = 关闭磁盘冷层"
                onChange={(e) => setTierForm({ ...tierForm, disk_gb: e.target.value })}
                className="w-24 rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-xs"
              />
            </label>
            <label className="flex items-center gap-1.5 text-ink-3">
              <Toggle checked={tierPersist} onChange={setTierPersist} />
              跨重启持久化
            </label>
            <Button className="ml-auto" onClick={() => saveTier.mutate()} disabled={saveTier.isPending}>
              存预算
            </Button>
          </div>
        )
      )}
    </Card>
  );
}
