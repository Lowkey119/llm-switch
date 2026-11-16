// 「仓库」页签：登记/启停技能仓库，「发现」页签的索引来源。
// 计划 §6.1 自 Skills.tsx 拆出，行为不变。
import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card, Toggle } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import type { RepoRow } from "./types";
import { theadCls } from "../../components/Table";

export function ReposSection({ refresh, downloadProxy }: { refresh: () => void; downloadProxy?: string }) {
  const toast = useToast();
  const [newRepo, setNewRepo] = useState({ owner: "", name: "", branch: "main" });
  const repos = useQuery({
    queryKey: ["skill-repos"],
    queryFn: () => ipc<RepoRow[]>("list_skill_repos"),
  });
  const addRepo = useMutation({
    mutationFn: () => ipc("add_skill_repo", { owner: newRepo.owner.trim(), name: newRepo.name.trim(), branch: newRepo.branch.trim() || "main" }),
    onSuccess: () => { setNewRepo({ owner: "", name: "", branch: "main" }); refresh(); },
    onError: (e) => toast.err(`添加仓库失败：${e}`),
  });
  const delRepo = useMutation({
    mutationFn: (id: number) => ipc("delete_skill_repo", { id }),
    onSuccess: refresh,
    onError: (e) => toast.err(`删除仓库失败：${e}`),
  });
  const repoToggle = useMutation({
    mutationFn: (v: { id: number; enabled: boolean }) => ipc("set_skill_repo_enabled", v),
    onSuccess: refresh,
    onError: (e) => toast.err(`启停仓库失败：${e}`),
  });

  return (
    <Card>
      <div className="mb-3 flex items-center gap-2">
        <h2 className="mr-auto text-sm font-semibold">技能仓库清单</h2>
        <input value={newRepo.owner} onChange={(e) => setNewRepo({ ...newRepo, owner: e.target.value })} placeholder="owner" className="w-28 rounded border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs" />
        <input value={newRepo.name} onChange={(e) => setNewRepo({ ...newRepo, name: e.target.value })} placeholder="repo" className="w-36 rounded border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs" />
        <input value={newRepo.branch} onChange={(e) => setNewRepo({ ...newRepo, branch: e.target.value })} placeholder="branch" className="w-24 rounded border border-slate-700 bg-slate-800/60 px-2 py-1 text-xs" />
        <Button variant="primary" disabled={!newRepo.owner.trim() || !newRepo.name.trim()} onClick={() => addRepo.mutate()}>
          添加
        </Button>
      </div>
      {repos.isError ? (
        <QueryError inline title="读取仓库清单失败" error={repos.error} onRetry={() => repos.refetch()} />
      ) : !repos.data?.length ? (
        <p className="text-xs text-ink-4">尚未登记仓库。添加后「发现」页签可索引进来的技能。</p>
      ) : (
        <table className="w-full text-xs">
          <thead className={theadCls}>
            <tr className="border-b border-slate-800 text-left text-ink-4">
              <th className="py-1.5 font-medium">仓库</th>
              <th className="font-medium">分支</th>
              <th className="font-medium">启用</th>
              <th className="text-right font-medium">操作</th>
            </tr>
          </thead>
          <tbody>
            {repos.data.map((r) => (
              <tr key={r.id} className="border-b border-slate-800/60">
                <td className="py-1.5 font-mono text-ink-2">{r.owner}/{r.name}</td>
                <td className="py-1.5 text-ink-3">{r.branch}</td>
                <td><Toggle checked={r.enabled} onChange={(v) => repoToggle.mutate({ id: r.id, enabled: v })} /></td>
                <td className="py-1.5 text-right">
                  <Button variant="danger" onClick={() => delRepo.mutate(r.id)}>删除</Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="mt-3 border-t border-slate-800 pt-2 text-xs text-ink-4">
        下载加速前缀：<code className="text-ink-4">{downloadProxy || "（直连 github.com）"}</code>
        ，可通过 <code className="text-ink-4">PUT /admin/skills/settings</code> 的 download_proxy 调整（如 https://gh-proxy.com/）。
      </div>
    </Card>
  );
}
