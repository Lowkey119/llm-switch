// 技能分配矩阵（Skills 页「分配全景」tab；原代理 tab 片二，
// docs/plans/2026-10-09-代理管理tab.md，2026-10-17 按队列③搬回本页）：
// 行 = 技能，列 = 各代理，格 = 该端的部署状态。
// 数据同源：get_skills + client_list（D24 —— 两张矩阵同一份列源），格子操作
// set_skill_app_enabled 与技能库共用 ["skills"] 缓存，两边改动即时互见。
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { QueryError, errText } from "./QueryError";
import { ipc, type ClientCol } from "../lib/ipc";
import { SearchInput } from "./SearchInput";

interface AppState {
  app: string;
  enabled: boolean;
  actual: string; // linked | copied | missing | foreign | orphan | absent
}
interface SkillRow {
  dir: string;
  name: string;
  managed: boolean;
  apps: AppState[];
}

/** 实际状态芯片配色（矩阵格）。 */
export const ACTUAL_STYLE: Record<string, string> = {
  linked: "border-cyan-500/40 bg-cyan-500/10 text-cyan-300",
  copied: "border-emerald-500/40 bg-emerald-500/10 text-[var(--sem-ok-text)]",
  missing: "border-amber-500/40 bg-amber-500/10 text-[var(--sem-busy-text)]",
  foreign: "border-rose-500/40 bg-rose-500/10 text-[var(--sem-danger-text)]",
  orphan: "border-violet-500/40 bg-violet-500/10 text-violet-300",
  absent: "border-slate-700 bg-slate-800/40 text-ink-4",
};
/** 状态含义提示（悬停可见）。 */
export const ACTUAL_TITLE: Record<string, string> = {
  linked: "软链生效",
  copied: "复制生效",
  missing: "已启用但目标缺失（断链）",
  foreign: "同名异物（非本工具部署，未触碰）",
  orphan: "SSOT 源缺失的残留副本",
  absent: "未部署",
};

export function SkillMatrix() {
  const qc = useQueryClient();
  const toast = useToast();
  const [q, setQ] = useState("");

  const skills = useQuery({
    queryKey: ["skills"],
    queryFn: () => ipc<SkillRow[]>("get_skills"),
  });
  const clients = useQuery({
    queryKey: ["client-list"],
    queryFn: () => ipc<ClientCol[]>("client_list"),
  });

  const toggle = useMutation({
    mutationFn: (v: { dir: string; app: string; enabled: boolean }) =>
      ipc("set_skill_app_enabled", v),
    onSuccess: (_r, v) => {
      qc.invalidateQueries({ queryKey: ["skills"] });
      toast.ok(v.enabled ? "已启用：重启该客户端后生效" : "已停用");
    },
    onError: (e) => toast.err(`改动失败：${e}`),
  });

  // 列与标签的唯一来源：后端下发的客户端清单（与模型矩阵同一份数据，D24）。
  // 停用的代理不列（代理 tab 片三：从两张矩阵消失，分配保留）。
  const cols = (clients.data ?? []).filter((c) => c.enabled);
  const apps = cols.map((c) => c.app);
  const labelOf = (app: string) => cols.find((c) => c.app === app)?.label ?? app;
  const rows = (skills.data ?? []).filter((s) => {
    const hay = `${s.dir} ${s.name}`.toLowerCase();
    return !q.trim() || hay.includes(q.trim().toLowerCase());
  });

  return (
    <div className="rounded-lg border border-slate-800 bg-slate-900">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-slate-800 px-3 py-2">
        <div>
          <h3 className="text-sm font-semibold text-ink-1">技能分配全景</h3>
        </div>
        <SearchInput value={q} onChange={setQ} placeholder="搜索技能名 / 描述" inputClassName="w-44" />
      </div>
      <div className="overflow-x-auto">
        {skills.isLoading || clients.isLoading ? (
          <div className="px-3 py-3 text-xs text-ink-4">载入中…</div>
        ) : skills.isError || clients.isError ? (
          /* 两个查询任一失败都要摆出来：clients 挂了列会全空，
             表看起来像「没有代理」，其实是读取失败。 */
          <div className="px-3 py-3">
            <QueryError
              inline
              title="读取失败"
              error={errText(skills.error) || errText(clients.error)}
              onRetry={() => {
                if (skills.isError) skills.refetch();
                if (clients.isError) clients.refetch();
              }}
            />
          </div>
        ) : rows.length === 0 ? (
          <div className="px-3 py-3 text-xs text-ink-4">
            暂无技能。到「发现」tab 安装，或把目录放进 SSOT 后点「扫描」。
          </div>
        ) : (
          <table className="w-full text-left text-xs">
            <thead>
              <tr className="border-b border-slate-800 text-left text-ink-4">
                <th className="py-1.5 pl-3 font-medium">技能</th>
                {apps.map((a) => (
                  <th key={a} className="px-1 text-center font-medium">
                    {labelOf(a)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((s) => (
                <tr key={s.dir} className="border-b border-slate-800/60">
                  <td className="py-1.5 pl-3">
                    <span className="text-ink-2">{s.name}</span>
                    {!s.managed && (
                      <span className="ml-2 rounded bg-slate-600/40 px-1 text-2xs text-ink-3">
                        未纳管
                      </span>
                    )}
                  </td>
                  {apps.map((a) => {
                    const st = s.apps.find((x) => x.app === a);
                    const actual = st?.actual ?? "absent";
                    return (
                      <td key={a} className="px-1 py-1 text-center">
                        <button
                          title={`${ACTUAL_TITLE[actual]}${st?.enabled ? " · 期望启用" : ""}`}
                          onClick={() =>
                            toggle.mutate({ dir: s.dir, app: a, enabled: !st?.enabled })
                          }
                          className={`w-full rounded border py-0.5 text-2xs ${ACTUAL_STYLE[actual]}`}
                        >
                          {actual === "absent" ? "—" : actual}
                        </button>
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      <div className="border-t border-slate-800 px-3 py-2 text-xs leading-relaxed text-ink-4">
        linked 软链生效 · copied 复制生效 · missing 已启用但断链 · foreign 同名异物（未触碰） ·
        orphan 源缺失残留 · — 未部署
      </div>
    </div>
  );
}
