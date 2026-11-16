// 「发现」页签：仓库索引 + skills.sh 搜索，两边都能一键安装。
// 计划 §6.1 自 Skills.tsx 拆出，行为不变；结果列表是纯本页状态，跟着组件走。
import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card, fieldCls } from "../../components/ui";
import { ipc } from "../../lib/ipc";
import type { DiscoveredItem } from "./types";
import { theadCls } from "../../components/Table";

export function DiscoverSection({ refresh }: { refresh: () => void }) {
  const toast = useToast();
  const [index, setIndex] = useState<DiscoveredItem[]>([]);
  const [results, setResults] = useState<DiscoveredItem[]>([]);
  const [searchQ, setSearchQ] = useState("");

  const doDiscover = useMutation({
    mutationFn: (force: boolean) => ipc<{ skills: DiscoveredItem[]; cached: boolean }>("discover_skills", { force }),
    onSuccess: (r) => setIndex(r.skills),
    onError: (e) => toast.err(`刷新索引失败：${e}`),
  });
  const doSearch = useMutation({
    mutationFn: (query: string) => ipc<{ skills: DiscoveredItem[] }>("search_skills", { q: query, limit: 20 }),
    onSuccess: (r) => setResults(r.skills),
    onError: (e) => toast.err(`搜索失败：${e}`),
  });
  const install = useMutation({
    mutationFn: (v: { owner: string; name: string; branch?: string; path?: string }) => ipc("install_skill", v),
    onSuccess: () => { refresh(); toast.ok("安装完成"); },
    onError: (e) => toast.err(`安装失败：${e}`),
  });

  return (
    <div className="space-y-4">
      <Card>
        <div className="mb-2 flex items-center gap-2">
          <h2 className="text-sm font-semibold">仓库索引</h2>
          <Button onClick={() => doDiscover.mutate(true)}>刷新索引</Button>
          <span className="text-xs text-ink-4">来自「仓库」页签中启用的仓库，TTL 10 分钟</span>
        </div>
        {!index.length ? (
          <p className="text-xs text-ink-4">点击「刷新索引」从启用仓库拉取技能清单。</p>
        ) : (
          <table className="w-full text-xs">
            <thead className={theadCls}>
              <tr className="border-b border-slate-800 text-left text-ink-4">
                <th className="py-1.5 font-medium">技能</th>
                <th className="font-medium">仓库</th>
                <th className="font-medium">路径</th>
                <th className="text-right font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {index.map((d) => (
                <tr key={d.key} className="border-b border-slate-800/60">
                  <td className="py-1.5">
                    <span className="text-ink-2">{d.name}</span>
                    {d.description && <div className="text-xs text-ink-4">{d.description}</div>}
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
                      安装
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
          <h2 className="text-sm font-semibold">skills.sh 搜索</h2>
          <input
            value={searchQ}
            onChange={(e) => setSearchQ(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && searchQ.trim() && doSearch.mutate(searchQ.trim())}
            placeholder="关键词，回车搜索"
            className={fieldCls + " w-56"}
          />
        </div>
        {!results.length ? (
          <p className="text-xs text-ink-4">搜索公共目录 skills.sh。</p>
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
                      安装
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
