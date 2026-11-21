// 「未纳管」页签：扫出来的技能目录认领纳管，或从 ZIP 整包导入。
// 查询与 mutation 都挂在这里（计划 §6.1 自 Skills.tsx 拆出，行为不变）：
// 只有这个页签用得到，留在页面层只会让 SkillsPage 白背一堆 state。
import { useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { Button, Card, fieldCls } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";
import type { UnmanagedItem } from "./types";
import { theadCls } from "../../components/Table";

export function UnmanagedSection({
  refresh,
  apps,
  query = "",
}: {
  refresh: () => void;
  apps: string[];
  /** 共用搜索框（页签行那颗）：本页签只按技能名 / 目录名过滤自己。 */
  query?: string;
}) {
  const toast = useToast();
  const [zipPath, setZipPath] = useState("");
  const unmanaged = useQuery({
    queryKey: ["skill-unmanaged"],
    queryFn: () => ipc<UnmanagedItem[]>("scan_unmanaged_skills"),
  });
  const claim = useMutation({
    mutationFn: (v: { dir: string; apps: string[] }) => ipc("claim_skill", { dir: v.dir, apps: v.apps }),
    onSuccess: () => { refresh(); toast.ok(t("skills.claimed")); },
    onError: (e) => toast.err(`${t("skills.claimFail")}：${e}`),
  });
  const importZip = useMutation({
    mutationFn: (p: string) => ipc<{ installed: unknown[]; skipped: { dir: string; reason: string }[] }>("import_skills_zip", { path: p }),
    onSuccess: (r) => { refresh(); setZipPath(""); toast.ok(ts("sk.importDone", { n: r.installed.length, skip: r.skipped.length ? ts("sk.importSkipped", { n: r.skipped.length }) : "" })); },
    onError: (e) => toast.err(ts("sk.importFail", { e: String(e) })),
  });

  const kw = query.trim().toLowerCase();
  const rows = (unmanaged.data ?? []).filter(
    (u) => !kw || `${u.name} ${u.dir}`.toLowerCase().includes(kw),
  );

  return (
    <Card>
      <div className="mb-2 flex items-center justify-between">
        <h2 className="text-base font-semibold">{t("sk.unmanagedTitle")}</h2>
        <Button onClick={() => unmanaged.refetch()}>{t("sk.rescan")}</Button>
      </div>
      {unmanaged.isError ? (
        <QueryError inline title={t("sk.unmanagedScanFail")} error={unmanaged.error} onRetry={() => unmanaged.refetch()} />
      ) : !rows.length ? (
        <p className="text-sm text-ink-4">
          {kw && unmanaged.data?.length ? ts("sk.unmanagedNoMatch", { q: query.trim() }) : t("sk.unmanagedEmpty")}
        </p>
      ) : (
        <table className="w-full text-xs">
          <thead className={theadCls}>
            <tr className="border-b border-slate-800 text-left text-ink-4">
              <th className="py-1.5 font-medium">{t("sk.colSkill")}</th>
              <th className="font-medium">{t("sk.colFoundIn")}</th>
              <th className="text-right font-medium">{t("sk.colAction")}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((u) => (
              <tr key={u.dir} className="border-b border-slate-800/60">
                <td className="py-1.5">
                  <span className="text-ink-2" title={u.dir}>{u.name}</span>
                  {/* 目录名与技能名多数同一个（扫目录扫出来的），重复显示纯属占位（用户 2026-11-19）；
                      不同才补灰字目录名，title 始终能读回完整路径。 */}
                  {u.dir !== u.name && <span className="ml-2 text-ink-4">{u.dir}</span>}
                </td>
                <td className="py-1.5 text-ink-3">{u.found_in.join(", ")}</td>
                <td className="py-1.5 text-right">
                  {/* 一键认领到**所有启用代理**（用户 2026-11-19 拍板）：原先只发到 found_in 那些端，
                      与启用清单对不上时就是空清单——认领了个寂寞，就是“点了没反应”的根源。
                      要单家取消去「分配全景」逐格关。 */}
                  <Button variant="primary" onClick={() => claim.mutate({ dir: u.dir, apps })}>
                    {t("sk.claimBtn")}
                  </Button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="mt-4 border-t border-slate-800 pt-3">
        <div className="mb-2 text-sm text-ink-4">{t("sk.zipImport")}</div>
        <div className="flex gap-2">
          <input
            value={zipPath}
            onChange={(e) => setZipPath(e.target.value)}
            placeholder="/path/to/skills-pack.zip"
            className={fieldCls + " w-96 max-w-full"}
          />
          <Button variant="primary" disabled={!zipPath.trim()} onClick={() => importZip.mutate(zipPath.trim())}>
            {t("sk.importZip")}
          </Button>
        </div>
      </div>
    </Card>
  );
}
