// 「发现」页签：仓库索引 + skills.sh 搜索，两边都能一键安装。
// 计划 §6.1 自 Skills.tsx 拆出，行为不变；结果列表是纯本页状态，跟着组件走。
import { useEffect, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card } from "../../components/ui";
import { ipc } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";
import type { DiscoveredItem } from "./types";
import { theadCls } from "../../components/Table";

export function DiscoverSection({ refresh, query = "" }: { refresh: () => void; query?: string }) {
  const toast = useToast();
  const [index, setIndex] = useState<DiscoveredItem[]>([]);
  const [results, setResults] = useState<DiscoveredItem[]>([]);
  const kw = query.trim();

  const doDiscover = useMutation({
    mutationFn: (force: boolean) => ipc<{ skills: DiscoveredItem[]; cached: boolean }>("discover_skills", { force }),
    onSuccess: (r) => setIndex(r.skills),
    onError: (e) => toast.err(ts("sk.indexRefreshFail", { e: String(e) })),
  });
  const doSearch = useMutation({
    mutationFn: (query: string) => ipc<{ skills: DiscoveredItem[] }>("search_skills", { q: query, limit: 20 }),
    onSuccess: (r) => setResults(r.skills),
    onError: (e) => toast.err(ts("sk.searchFail", { e: String(e) })),
  });
  const install = useMutation({
    mutationFn: (v: { owner: string; name: string; branch?: string; path?: string }) => ipc("install_skill", v),
    onSuccess: () => { refresh(); toast.ok(t("sk.installDone")); },
    onError: (e) => toast.err(ts("sk.installFail", { e: String(e) })),
  });

  // 共用搜索框在本页签的语义（用户 2026-11-19）：拿它去进 skills.sh 远端搜索，
  // 输入停顿 500ms 自动查（不再单开一颗框）；仓库索引那一表同时按它本地过滤。
  useEffect(() => {
    if (!kw) return;
    const timer = setTimeout(() => doSearch.mutate(kw), 500);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kw]);

  const low = kw.toLowerCase();
  const indexRows = index.filter((d) => !low || `${d.name} ${d.description ?? ""}`.toLowerCase().includes(low));

  return (
    <div className="space-y-4">
      <Card>
        <div className="mb-2 flex items-center gap-2">
          <h2 className="text-base font-semibold">{t("sk.repoIndex")}</h2>
          <Button onClick={() => doDiscover.mutate(true)}>{t("sk.refreshIndex")}</Button>
          <span className="text-sm text-ink-4">{t("sk.indexNote")}</span>
        </div>
        {!index.length ? (
          <p className="text-sm text-ink-4">{t("sk.indexEmpty")}</p>
        ) : !indexRows.length ? (
          <p className="text-sm text-ink-4">{ts("sk.indexNoMatch", { q: kw })}</p>
        ) : (
          <table className="w-full text-xs">
            <thead className={theadCls}>
              <tr className="border-b border-slate-800 text-left text-ink-4">
                <th className="py-1.5 font-medium">{t("sk.colSkill")}</th>
                <th className="font-medium">{t("sk.colRepo")}</th>
                <th className="font-medium">{t("sk.colPath")}</th>
                <th className="text-right font-medium">{t("sk.colAction")}</th>
              </tr>
            </thead>
            <tbody>
              {indexRows.map((d) => (
                <tr key={d.key} className="border-b border-slate-800/60">
                  <td className="py-1.5">
                    <span className="text-ink-2">{d.name}</span>
                    {d.description && <div className="text-sm text-ink-4">{d.description}</div>}
                  </td>
                  <td className="py-1.5 text-ink-3">{d.repo_owner}/{d.repo_name}@{d.repo_branch}</td>
                  <td className="py-1.5 font-mono text-ink-4">{d.directory}</td>
                  <td className="py-1.5 text-right">
                    <Button
                      variant="primary"
                      onClick={() =>
                        install.mutate({ owner: d.repo_owner, name: d.repo_name, branch: d.repo_branch, path: d.directory })
                      }
                    >
                      {t("sk.install")}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
      <Card>
        <div className="mb-2 flex items-center gap-2">
          <h2 className="text-base font-semibold">{t("sk.shSearch")}</h2>
          {/* 关键词就是页签行那颗共用搜索框（用户 2026-11-19）：这里只报当前在查什么。 */}
          <span className="text-sm text-ink-4">
            {kw ? ts("sk.kwQuote", { kw }) : t("sk.kwHint")}
            {doSearch.isPending && t("sk.searching")}
          </span>
        </div>
        {!results.length ? (
          <p className="text-sm text-ink-4">{t("sk.shEmpty")}</p>
        ) : (
          <table className="w-full text-xs">
            <tbody>
              {results.map((d) => (
                <tr key={d.key} className="border-b border-slate-800/60">
                  <td className="py-1.5 text-ink-2">{d.name}</td>
                  <td className="py-1.5 text-ink-3">{d.repo_owner}/{d.repo_name}</td>
                  <td className="py-1.5 text-ink-4">{d.installs?.toLocaleString() ?? ""} installs</td>
                  <td className="py-1.5 text-right">
                    <Button variant="primary" onClick={() => install.mutate({ owner: d.repo_owner, name: d.repo_name, branch: d.repo_branch, path: d.directory })}>
                      {t("sk.install")}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>
    </div>
  );
}
