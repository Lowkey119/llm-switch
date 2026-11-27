// 技能分配矩阵（Skills 页「分配全景」tab；原代理 tab 片二，
// docs/plans/2026-10-09-代理管理tab.md，2026-10-17 按队列③搬回本页）：
// 行 = 技能，列 = 各代理，格 = 该端的部署状态。
// 数据同源：get_skills + client_list（D24 —— 两张矩阵同一份列源），格子操作
// set_skill_app_enabled 与技能库共用 ["skills"] 缓存，两边改动即时互见。
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "./Toast";
import { QueryError, errText } from "./QueryError";
import { ipc, type ClientCol } from "../lib/ipc";
import { t, ts } from "../lib/i18n";

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
/** 状态含义提示（悬停可见）；存 i18n key，渲染时 t() 取词。 */
export const ACTUAL_TITLE: Record<string, string> = {
  linked: "sk.actualLinked",
  copied: "sk.actualCopied",
  missing: "sk.actualMissing",
  foreign: "sk.actualForeign",
  orphan: "sk.actualOrphan",
  absent: "sk.actualAbsent",
};

export function SkillMatrix({ query = "" }: { query?: string }) {
  const qc = useQueryClient();
  const toast = useToast();
  // 搜索词改用页签行那颗共用搜索框（用户 2026-11-19）：本页签只按它过滤自己。
  const q = query;

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
      toast.ok(v.enabled ? t("sk.enabledOn") : t("sk.enabledOff"));
    },
    onError: (e) => toast.err(ts("mm.fail", { e: String(e) })),
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
          <h3 className="text-base font-semibold text-ink-1">{t("sk.matrixTitle")}</h3>
        </div>
      </div>
      <div className="overflow-x-auto">
        {skills.isLoading || clients.isLoading ? (
          <div className="px-3 py-3 text-xs text-ink-4">{t("common.loading")}</div>
        ) : skills.isError || clients.isError ? (
          /* 两个查询任一失败都要摆出来：clients 挂了列会全空，
             表看起来像「没有代理」，其实是读取失败。 */
          <div className="px-3 py-3">
            <QueryError
              inline
              title={t("ag.readFailBadge")}
              error={errText(skills.error) || errText(clients.error)}
              onRetry={() => {
                if (skills.isError) skills.refetch();
                if (clients.isError) clients.refetch();
              }}
            />
          </div>
        ) : rows.length === 0 ? (
          <div className="px-3 py-3 text-xs text-ink-4">
            {t("sk.matrixEmpty")}
          </div>
        ) : (
          <table className="w-full text-left text-xs">
            <thead>
              <tr className="border-b border-slate-800 text-left text-ink-4">
                <th className="py-1.5 pl-3 font-medium">{t("sk.colSkill")}</th>
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
                        {t("skills.unmanagedBadge")}
                      </span>
                    )}
                  </td>
                  {apps.map((a) => {
                    const st = s.apps.find((x) => x.app === a);
                    const actual = st?.actual ?? "absent";
                    return (
                      <td key={a} className="px-1 py-1 text-center">
                        <button
                          title={`${t(ACTUAL_TITLE[actual])}${st?.enabled ? t("sk.wantEnabled") : ""}`}
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
        {t("sk.legend")}
      </div>
    </div>
  );
}
