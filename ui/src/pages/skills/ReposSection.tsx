// 「仓库」页签：登记/启停技能仓库，「发现」页签的索引来源。
// 计划 §6.1 自 Skills.tsx 拆出，行为不变。
import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card, Toggle } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import { t, tf, ts } from "../../lib/i18n";
import type { RepoRow } from "./types";
import { theadCls } from "../../components/Table";

export function ReposSection({
  refresh,
  downloadProxy,
  query = "",
}: {
  refresh: () => void;
  downloadProxy?: string;
  /** 共用搜索框：本页签只按 owner / name / branch 过滤仓库清单。 */
  query?: string;
}) {
  const toast = useToast();
  const [newRepo, setNewRepo] = useState({ owner: "", name: "", branch: "main" });
  const repos = useQuery({
    queryKey: ["skill-repos"],
    queryFn: () => ipc<RepoRow[]>("list_skill_repos"),
  });
  const addRepo = useMutation({
    mutationFn: () => ipc("add_skill_repo", { owner: newRepo.owner.trim(), name: newRepo.name.trim(), branch: newRepo.branch.trim() || "main" }),
    onSuccess: () => { setNewRepo({ owner: "", name: "", branch: "main" }); refresh(); },
    onError: (e) => toast.err(ts("sk.addRepoFail", { e: String(e) })),
  });
  const delRepo = useMutation({
    mutationFn: (id: number) => ipc("delete_skill_repo", { id }),
    onSuccess: refresh,
    onError: (e) => toast.err(ts("sk.delRepoFail", { e: String(e) })),
  });
  const repoToggle = useMutation({
    mutationFn: (v: { id: number; enabled: boolean }) => ipc("set_skill_repo_enabled", v),
    onSuccess: refresh,
    onError: (e) => toast.err(ts("sk.toggleRepoFail", { e: String(e) })),
  });

  const low = query.trim().toLowerCase();
  const rows = (repos.data ?? []).filter(
    (r) => !low || `${r.owner}/${r.name}@${r.branch}`.toLowerCase().includes(low),
  );

  return (
    <Card>
      <div className="mb-3 flex items-center gap-2">
        <h2 className="mr-auto text-base font-semibold">{t("sk.repoListTitle")}</h2>
        <input value={newRepo.owner} onChange={(e) => setNewRepo({ ...newRepo, owner: e.target.value })} placeholder="owner" className="w-28 rounded border border-slate-700 bg-slate-800/60 px-2 py-1 text-sm" />
        <input value={newRepo.name} onChange={(e) => setNewRepo({ ...newRepo, name: e.target.value })} placeholder="repo" className="w-36 rounded border border-slate-700 bg-slate-800/60 px-2 py-1 text-sm" />
        <input value={newRepo.branch} onChange={(e) => setNewRepo({ ...newRepo, branch: e.target.value })} placeholder="branch" className="w-24 rounded border border-slate-700 bg-slate-800/60 px-2 py-1 text-sm" />
        <Button variant="primary" disabled={!newRepo.owner.trim() || !newRepo.name.trim()} onClick={() => addRepo.mutate()}>
          {t("sk.add")}
        </Button>
      </div>
      {repos.isError ? (
        <QueryError inline title={t("sk.repoReadFail")} error={repos.error} onRetry={() => repos.refetch()} />
      ) : !rows.length ? (
        <p className="text-sm text-ink-4">
          {low && repos.data?.length ? ts("sk.repoNoMatch", { q: query.trim() }) : t("sk.repoEmpty")}
        </p>
      ) : (
        <table className="w-full text-xs">
          <thead className={theadCls}>
            <tr className="border-b border-slate-800 text-left text-ink-4">
              <th className="py-1.5 font-medium">{t("sk.colRepo")}</th>
              <th className="font-medium">{t("sk.colBranch")}</th>
              <th className="font-medium">{t("filter.enabled")}</th>
              <th className="text-right font-medium">{t("sk.colAction")}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => (
              <tr key={r.id} className="border-b border-slate-800/60">
                <td className="py-1.5 font-mono text-ink-2">{r.owner}/{r.name}</td>
                <td className="py-1.5 text-ink-3">{r.branch}</td>
                <td><Toggle checked={r.enabled} onChange={(v) => repoToggle.mutate({ id: r.id, enabled: v })} /></td>
                <td className="py-1.5 text-right">
                  <Button variant="danger" onClick={() => delRepo.mutate(r.id)}>{t("common.delete")}</Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="mt-3 border-t border-slate-800 pt-2 text-sm text-ink-4">
        {t("sk.proxyPrefix")}<code className="text-ink-4">{downloadProxy || t("sk.directGithub")}</code>
        {tf("sk.proxyNote", { c: <code className="text-ink-4">PUT /admin/skills/settings</code> })}
      </div>
    </Card>
  );
}
