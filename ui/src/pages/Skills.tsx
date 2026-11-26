// Skills 页（M6-D）：技能库列表 + 分配全景（队列③，自代理页搬回）+ 未纳管 / 发现 / 备份 / 仓库。
import { useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../components/Toast";
import { SkillMatrix } from "../components/SkillMatrix";
import { Tabs } from "../components/Tabs";
import { QueryError } from "../components/QueryError";
import { ConfirmDialog, Risk } from "../components/ConfirmDialog";
import { SearchInput } from "../components/SearchInput";
import { ipc, type ClientCol } from "../lib/ipc";
import { UnmanagedSection } from "./skills/UnmanagedSection";
import { DiscoverSection } from "./skills/DiscoverSection";
import { BackupsSection } from "./skills/BackupsSection";
import { ReposSection } from "./skills/ReposSection";
import { Button, Card } from "../components/ui";

import { t, tf, ts } from "../lib/i18n";
import { IconClose, IconSearch, IconSkills } from "../components/icons";
import { EmptyState } from "../components/States";
import { ClearFiltersButton, FilterBar } from "../components/FilterBar";
import type { FilterTag } from "../components/FilterBar";
import type { SkillView, Tab, UpdateItem } from "./skills/types";
import { theadCls, useSort, sortRows, SortTh } from "../components/Table";
import type { SortState } from "../components/Table";

// 技能表的可排序列（ui_01 回灌 U4）：「操作」列没有可排的值，不开。
type SkillKey = "name" | "repo";

/** 页签标签存 i18n key，渲染时 t() 取词。 */
const TAB_LABELS: Record<Tab, string> = {
  installed: "sk.tabInstalled",
  matrix: "sk.tabMatrix",
  unmanaged: "sk.tabUnmanaged",
  discover: "sk.tabDiscover",
  backups: "sk.tabBackups",
  repos: "sk.tabRepos",
};

// 客户端列清单不再写死：两张矩阵同源（D24），列与标签都来自后端 `client_list`。


export function SkillsPage() {
  const qc = useQueryClient();
  const [tab, setTab] = useState<Tab>("installed");
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState<"all" | "updatable" | "local">("all");
  const [preview, setPreview] = useState<{ dir: string; content: string } | null>(null);
  // 卸载是破坏性动作，闸门走全站统一的 ConfirmDialog（UI-AUDIT-3 P3-1）——
  // 原先这里是原生 confirm()：样式与双主题无关、按钮文案不可控，绕过统一闸门。
  const [confirmUninstall, setConfirmUninstall] = useState<{ dir: string; name: string } | null>(null);
  const [updates, setUpdates] = useState<UpdateItem[]>([]);
  const toast = useToast();

  const skills = useQuery({ queryKey: ["skills"], queryFn: () => ipc<SkillView[]>("get_skills") });
  const settings = useQuery({ queryKey: ["skill-settings"], queryFn: () => ipc<{ sync_method: string; download_proxy: string }>("get_skill_settings") });
  const clients = useQuery({ queryKey: ["client-list"], queryFn: () => ipc<ClientCol[]>("client_list") });

  // 认领纳管 = 一键发到所有启用代理（用户 2026-11-19 拍板）：要单家取消去「分配全景」逐格关。
  // 不再弹勾选框：两处入口（已安装页的「未纳管」徽标与「未纳管」页签）同一个动作，点了就该有结果。
  const claim = useMutation({
    mutationFn: (v: { dir: string; apps: string[] }) => ipc("claim_skill", { dir: v.dir, apps: v.apps }),
    onSuccess: () => {
      refresh();
      toast.ok(t("skills.claimed"));
    },
    onError: (e) => toast.err(`${t("skills.claimFail")}：${e}`),
  });

  const refresh = () => {
    qc.invalidateQueries({ queryKey: ["skills"] });
    qc.invalidateQueries({ queryKey: ["skill-backups"] });
    qc.invalidateQueries({ queryKey: ["skill-unmanaged"] });
    qc.invalidateQueries({ queryKey: ["skill-repos"] });
  };
  // 所有 mutation 统一走 fail 落 msg 横幅——不加 onError 失败静默无痕，
  // 用户看到的是「点了没反应」（界面停在旧状态，与成功无法区分）。
  const fail = (what: string) => (e: unknown) => toast.err(ts("sk.opFail", { what, e: String(e) }));
  // 原先把后端原始 JSON 直接甩给用户看，这里换成一句人话摘要。
  const scan = useMutation({
    mutationFn: () =>
      ipc<{ added: unknown[]; updated: unknown[]; removed: unknown[] }>("scan_skills"),
    onSuccess: (r) => {
      refresh();
      toast.ok(ts("sk.scanDone", { a: r.added.length, u: r.updated.length, r: r.removed.length }));
    },
    onError: fail(t("sk.actScan")),
  });
  const uninstall = useMutation({
    mutationFn: (dir: string) => ipc<{ backup: string | null }>("uninstall_skill", { dir }),
    onSuccess: (r) => { refresh(); toast.ok(r.backup ? ts("sk.uninstalledBk", { b: r.backup }) : t("sk.uninstalled")); },
    onError: fail(t("sk.actUninstall")),
  });
  const checkUpd = useMutation({
    mutationFn: () => ipc<{ updates: UpdateItem[] }>("check_skill_updates"),
    onSuccess: (r) => { setUpdates(r.updates); toast.info(ts("sk.updAvail", { n: r.updates.length })); qc.invalidateQueries({ queryKey: ["skills"] }); },
    onError: fail(t("skills.checkUpd")),
  });
  const updateOne = useMutation({ mutationFn: (dir: string) => ipc("update_skill", { dir }), onSuccess: (r) => { refresh(); setUpdates((u) => u.filter((x) => x.dir !== (r as { skill: SkillView }).skill.dir)); }, onError: fail(t("sk.actUpdate")) });
  const updateAll = useMutation({ mutationFn: () => ipc("update_skill", {}), onSuccess: () => { refresh(); setUpdates([]); }, onError: fail(t("skills.updateAll")) });
  const setSync = useMutation({
    mutationFn: (m: string) => ipc("set_skill_sync_method", { method: m }),
    onSuccess: refresh,
    onError: fail(t("sk.actSetSync")),
  });
  const showDoc = useMutation({
    mutationFn: (dir: string) => ipc<string>("read_skill_doc", { dir }),
    onSuccess: (content, dir) => setPreview({ dir, content }),
    onError: fail(t("sk.actReadDoc")),
  });

  const updSet = new Set(updates.map((u) => u.dir));
  // 列与标签的唯一来源：后端下发的客户端清单（与两张矩阵同一份数据，D24）；
  // 技能分配全景 2026-10-17 自代理页搬回本页「分配全景」tab（队列③，回滚 10-09 分工），
  // 分发启停由它管理；本列表认领时还用得到 apps 过滤。
  // 停用的代理不参与认领（代理 tab 片三）。
  const cols = (clients.data ?? []).filter((c) => c.enabled);
  const apps = cols.map((c) => c.app);
  const rows = (skills.data ?? []).filter((s) => {
    if (filter === "updatable" && !updSet.has(s.dir)) return false;
    if (filter === "local" && s.managed && s.repo) return false;
    const hay = `${s.dir} ${s.name} ${s.description ?? ""} ${s.repo ?? ""}`.toLowerCase();
    return !q.trim() || hay.includes(q.trim().toLowerCase());
  });

  // 是否处于筛选态（搜索词或分段筛选任一生效）——空态要据此说不同的话。
  const filtered = !!q.trim() || filter !== "all";
  // 已选条件 chip（U6）：搜索词与范围各一颗，各自能单清。
  const scopeLabel =
    filter === "updatable" ? t("skills.segUpdatable") : filter === "local" ? t("skills.segLocal") : "";
  const activeTags = [
    q.trim() ? { key: "q", label: t("filter.tagSearch"), value: q.trim(), clear: () => setQ("") } : null,
    filter !== "all" ? { key: "scope", label: t("filter.tagFilter"), value: scopeLabel, clear: () => setFilter("all") } : null,
  ].filter(Boolean) as FilterTag[];
  const clearFilters = () => {
    setQ("");
    setFilter("all");
  };

  // 排序（ui_01 回灌 U4）：在过滤后的行上排，过滤条件与排序互不干扰。
  const skillSort = useSort<SkillKey>();
  const view = sortRows(rows, skillSort.sort as SortState<string>, (s, k) =>
    k === "repo" ? s.repo : s.name,
  );

  return (
    <div className="space-y-4 pt-[var(--content-pad-top)]">
      {/* 工具条：不带计数——条数归过滤条的「命中 n / 总数」说，同一页不两处报同一个数（U6）。
          扫描 / 检查更新与 Tab、过滤同属一组操作，留在工具条上，拆开会两头找。 */}
      <Card className="flex flex-wrap items-center gap-2">
        <Tabs
          variant="segmented"
          ariaLabel={t("sk.viewAria")}
          value={tab}
          onChange={(id) => setTab(id as Tab)}
          items={(["installed", "matrix", "unmanaged", "discover", "backups", "repos"] as Tab[]).map((tb) => ({
            id: tb,
            label: t(TAB_LABELS[tb]),
          }))}
        />
        {/* 共用搜索框（用户 2026-11-19）：六个页签共用一颗，但**各页签只搜自己**——
            已安装 / 分配全景 / 未纳管 / 备份 / 仓库 是本地过滤，「发现」拿它去进远端搜索。 */}
        <SearchInput value={q} onChange={setQ} placeholder={t("skills.searchTabPh")} inputClassName="w-52" />
        <div className="ml-auto flex items-center gap-2 text-sm text-ink-3">
          <span>{t("sk.distribute")}</span>
          {/* 读失败时不能默认显示「自动」——那是空值兜底，会被当成真值 */}
          {settings.isError && (
            <span className="text-[var(--sem-danger-text)]">
              {t("sk.settingsReadFail")}
              <button
                onClick={() => settings.refetch()}
                className="ml-1 underline underline-offset-2 hover:text-ink-1"
              >
                {t("common.retry")}
              </button>
            </span>
          )}
          <select
            value={settings.data?.sync_method ?? "auto"}
            disabled={settings.isError}
            onChange={(e) => setSync.mutate(e.target.value)}
            className="rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-sm"
          >
            <option value="auto">{t("sk.syncAuto")}</option>
            <option value="symlink">{t("sk.syncSymlink")}</option>
            <option value="copy">{t("sk.syncCopy")}</option>
          </select>
          <Button onClick={() => scan.mutate()}>{t("sk.scanSsot")}</Button>
        </div>
      </Card>


      {tab === "installed" && (
        <Card>
          <FilterBar
            className="mb-3"
            count={rows.length}
            total={skills.data?.length ?? 0}
            active={activeTags}
            onClearAll={filtered ? clearFilters : undefined}
            trailing={
              <>
                <Button onClick={() => checkUpd.mutate()}>{t("skills.checkUpd")}</Button>
                <Button variant="primary" disabled={!updates.length} onClick={() => updateAll.mutate()}>
                  {t("skills.updateAll")}
                  {updates.length ? `(${updates.length})` : ""}
                </Button>
              </>
            }
          >
            <Tabs
              variant="segmented"
              size="sm"
              ariaLabel={t("filter.bySkill")}
              value={filter}
              onChange={(id) => setFilter(id as "all" | "updatable" | "local")}
              items={[
                { id: "all", label: t("filter.all") },
                { id: "updatable", label: t("skills.segUpdatable") },
                { id: "local", label: t("skills.segLocal") },
              ]}
            />
          </FilterBar>

          {/* 读取失败 ≠ 暂无技能：失败给红字 + 重试入口。卡内用 inline，
              块状红框套在卡里就是「卡中卡」（impeccable nested-cards）。 */}
          {skills.isError ? (
            <QueryError inline title={t("skills.listError")} error={skills.error} onRetry={() => skills.refetch()} />
          ) : !rows.length ? (
            filtered ? (
              <EmptyState
                icon={IconSearch}
                title={t("skills.emptyFiltered")}
                action={<ClearFiltersButton onClick={clearFilters} />}
              />
            ) : (
              <EmptyState icon={IconSkills} title={t("skills.emptyTitle")} desc={t("skills.emptyDesc")} />
            )
          ) : (
            <table className="w-full text-xs">
              <thead className={theadCls}>
                <tr className="border-b border-slate-800 text-left text-ink-4">
                  <SortTh label={t("sk.colSkill")} k="name" sort={skillSort.sort} onSort={skillSort.toggle} className="py-1.5 whitespace-nowrap" />
                  <SortTh label={t("sk.colSource")} k="repo" sort={skillSort.sort} onSort={skillSort.toggle} className="whitespace-nowrap" />
                  <th className="text-right font-medium whitespace-nowrap">{t("sk.colAction")}</th>
                </tr>
              </thead>
              <tbody>
                {view.map((s) => (
                  <tr key={s.dir} className="border-b border-slate-800/60 align-top">
                    <td className="py-2 pr-2">
                      <div className="text-ink-2">
                        {s.name}
                        {updSet.has(s.dir) && <span className="ml-2 rounded bg-amber-500/20 px-1 text-2xs text-[var(--sem-busy-text)]">{t("skills.segUpdatable")}</span>}
                        {!s.managed && (
                          <button
                            className="ml-2 rounded bg-slate-600/40 px-1 text-2xs text-ink-3 hover:bg-slate-600/70 hover:text-ink-1"
                            title={t("skills.claimTip")}
                            onClick={() => claim.mutate({ dir: s.dir, apps })}
                          >
                            {t("skills.unmanagedBadge")}
                          </button>
                        )}
                      </div>
                      <div className="text-sm text-ink-4">
                        {s.dir}
                        {s.description ? ` · ${s.description}` : ""}
                      </div>
                    </td>
                    <td className="py-2 pr-2 text-ink-3">
                      {s.repo ? `${s.repo}${s.branch ? `@${s.branch}` : ""}` : t("usage.scopeLocal")}
                    </td>
                    <td className="py-2 text-right whitespace-nowrap">
                      <Button onClick={() => showDoc.mutate(s.dir)}>{t("sk.preview")}</Button>{" "}
                      {updSet.has(s.dir) && (
                        <Button variant="primary" onClick={() => updateOne.mutate(s.dir)}>{t("sk.actUpdate")}</Button>
                      )}{" "}
                      <Button variant="danger" onClick={() => setConfirmUninstall({ dir: s.dir, name: s.name })}>{t("sk.actUninstall")}</Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      )}

      {tab === "matrix" && <SkillMatrix query={q} />}

      {/* 下面四个页签各自成文件（计划 §6.1）：查询与 mutation 跟着页签挂载卸载，
          页面层只留 refresh —— 谁改了数据，就刷新 skills / backups / unmanaged / repos 四把钥匙。
          共用搜索框的 q 透传下去，各页签自己解释。 */}
      {tab === "unmanaged" && <UnmanagedSection refresh={refresh} apps={apps} query={q} />}

      {tab === "discover" && <DiscoverSection refresh={refresh} query={q} />}

      {tab === "backups" && <BackupsSection refresh={refresh} query={q} />}

      {tab === "repos" && <ReposSection refresh={refresh} downloadProxy={settings.data?.download_proxy} query={q} />}

      {/* 卸载闸门（UI-AUDIT-3 P3-1）：破坏性动作统一走 ConfirmDialog + Risk，
          说清卸载对象、自动备份与恢复路径，样式与双主题一致、按钮文案可控。 */}
      <ConfirmDialog
        open={!!confirmUninstall}
        title={confirmUninstall ? ts("sk.uninstallTitle", { name: confirmUninstall.name }) : ""}
        confirmLabel={t("sk.uninstallConfirm")}
        danger
        onClose={() => setConfirmUninstall(null)}
        onConfirm={() => confirmUninstall && uninstall.mutate(confirmUninstall.dir)}
      >
        {tf("sk.uninstallBody", { b: <b>{confirmUninstall?.name}</b> })}
        <Risk>{t("sk.uninstallRisk")}</Risk>
        {t("sk.uninstallRestore")}
      </ConfirmDialog>

      {/* SKILL.md 预览走 Radix Dialog（UI 审查 P0-3）：焦点陷阱 / Esc 关闭 /
          aria-modal 都是手搜遮罩补不齐的，与 ConfirmDialog、BackendDialog 同族。 */}
      <Dialog.Root open={!!preview} onOpenChange={(o) => !o && setPreview(null)}>
        {preview && (
          <Dialog.Portal>
            <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
            <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] flex max-h-[var(--dialog-max-h)] w-[720px] max-w-[92%] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-slate-700 bg-slate-900 p-4 shadow-2xl">
              <div className="mb-2 flex items-center justify-between gap-3">
                <Dialog.Title className="truncate font-mono text-xs text-cyan-300" title={`${preview.dir} / SKILL.md`}>
                  {preview.dir} / SKILL.md
                </Dialog.Title>
                <button
                  aria-label={t("sk.closePreview")}
                  onClick={() => setPreview(null)}
                  className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-ink-4 transition-colors hover:bg-slate-800 hover:text-ink-1"
                >
                  <IconClose width={14} height={14} />
                </button>
              </div>
              <Dialog.Description className="min-h-0 flex-1 overflow-auto">
                <pre className="whitespace-pre-wrap break-all text-sm leading-relaxed text-ink-2">{preview.content}</pre>
              </Dialog.Description>
            </Dialog.Content>
          </Dialog.Portal>
        )}
      </Dialog.Root>
    </div>
  );
}
