// 「未纳管」页签：扫出来的技能目录认领纳管，或从 ZIP 整包导入。
// 查询与 mutation 都挂在这里（计划 §6.1 自 Skills.tsx 拆出，行为不变）：
// 只有这个页签用得到，留在页面层只会让 SkillsPage 白背一堆 state。
import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card, fieldCls } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import type { UnmanagedItem } from "./types";
import { theadCls } from "../../components/Table";

export function UnmanagedSection({ refresh, apps }: { refresh: () => void; apps: string[] }) {
  const toast = useToast();
  const [zipPath, setZipPath] = useState("");
  const unmanaged = useQuery({
    queryKey: ["skill-unmanaged"],
    queryFn: () => ipc<UnmanagedItem[]>("scan_unmanaged_skills"),
  });
  const claim = useMutation({
    mutationFn: (v: { dir: string; apps: string[] }) => ipc("claim_skill", { dir: v.dir, apps: v.apps }),
    onSuccess: () => { refresh(); toast.ok("已认领纳管"); },
    onError: (e) => toast.err(`认领失败：${e}`),
  });
  const importZip = useMutation({
    mutationFn: (p: string) => ipc<{ installed: unknown[]; skipped: { dir: string; reason: string }[] }>("import_skills_zip", { path: p }),
    onSuccess: (r) => { refresh(); setZipPath(""); toast.ok(`导入 ${r.installed.length} 项${r.skipped.length ? `，跳过 ${r.skipped.length} 项` : ""}`); },
    onError: (e) => toast.err(`导入失败：${e}`),
  });

  return (
    <Card>
      <div className="mb-2 flex items-center justify-between">
        <h2 className="text-sm font-semibold">未纳管技能</h2>
        <Button onClick={() => unmanaged.refetch()}>重新扫描</Button>
      </div>
      {unmanaged.isError ? (
        <QueryError inline title="扫描未纳管技能失败" error={unmanaged.error} onRetry={() => unmanaged.refetch()} />
      ) : !unmanaged.data?.length ? (
        <p className="text-xs text-ink-4">五端目录与 SSOT 中没有未纳管的技能目录。</p>
      ) : (
        <table className="w-full text-xs">
          <thead className={theadCls}>
            <tr className="border-b border-slate-800 text-left text-ink-4">
              <th className="py-1.5 font-medium">技能</th>
              <th className="font-medium">发现于</th>
              <th className="text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {unmanaged.data.map((u) => (
              <tr key={u.dir} className="border-b border-slate-800/60">
                <td className="py-1.5">
                  <span className="text-ink-2">{u.name}</span>
                  <span className="ml-2 text-ink-4">{u.dir}</span>
                </td>
                <td className="py-1.5 text-ink-3">{u.found_in.join(", ")}</td>
                <td className="py-1.5 text-right">
                  <Button variant="primary" onClick={() => claim.mutate({ dir: u.dir, apps: u.found_in.filter((f) => apps.includes(f)) })}>
                    认领纳管
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="mt-4 border-t border-slate-800 pt-3">
        <div className="mb-2 text-xs text-ink-4">ZIP 导入（本地绝对路径，可含多个技能目录）：</div>
        <div className="flex gap-2">
          <input
            value={zipPath}
            onChange={(e) => setZipPath(e.target.value)}
            placeholder="/path/to/skills-pack.zip"
            className={fieldCls + " w-96 max-w-full"}
          />
          <Button variant="primary" disabled={!zipPath.trim()} onClick={() => importZip.mutate(zipPath.trim())}>
            导入 ZIP
          </Button>
        </div>
      </div>
    </Card>
  );
}
