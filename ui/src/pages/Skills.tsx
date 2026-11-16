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
import { IconClose } from "../components/icons";
import type { SkillView, Tab, UpdateItem } from "./skills/types";
import { theadCls } from "../components/Table";

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

  const refresh = () => {
    qc.invalidateQueries({ queryKey: ["skills"] });
    qc.invalidateQueries({ queryKey: ["skill-backups"] });
    qc.invalidateQueries({ queryKey: ["skill-unmanaged"] });
    qc.invalidateQueries({ queryKey: ["skill-repos"] });
  };
  // 所有 mutation 统一走 fail 落 msg 横幅——不加 onError 失败静默无痕，
  // 用户看到的是「点了没反应」（界面停在旧状态，与成功无法区分）。
  const fail = (what: string) => (e: unknown) => toast.err(`${what}失败：${e}`);
  // 原先把后端原始 JSON 直接甩给用户看，这里换成一句人话摘要。
  const scan = useMutation({
    mutationFn: () =>
      ipc<{ added: unknown[]; updated: unknown[]; removed: unknown[] }>("scan_skills"),
    onSuccess: (r) => {
      refresh();
      toast.ok(`扫描完成 · 新增 ${r.added.length} · 更新 ${r.updated.length} · 移除 ${r.removed.length}`);
    },
    onError: fail("扫描"),
  });
  const uninstall = useMutation({
    mutationFn: (dir: string) => ipc<{ backup: string | null }>("uninstall_skill", { dir }),
    onSuccess: (r) => { refresh(); toast.ok(r.backup ? `已卸载，备份：${r.backup}` : "已卸载"); },
    onError: fail("卸载"),
  });
  const checkUpd = useMutation({
    mutationFn: () => ipc<{ updates: UpdateItem[] }>("check_skill_updates"),
    onSuccess: (r) => { setUpdates(r.updates); toast.info(`可更新 ${r.updates.length} 项`); qc.invalidateQueries({ queryKey: ["skills"] }); },
    onError: fail("检查更新"),
  });
  const updateOne = useMutation({ mutationFn: (dir: string) => ipc("update_skill", { dir }), onSuccess: (r) => { refresh(); setUpdates((u) => u.filter((x) => x.dir !== (r as { skill: SkillView }).skill.dir)); }, onError: fail("更新") });
  const updateAll = useMutation({ mutationFn: () => ipc("update_skill", {}), onSuccess: () => { refresh(); setUpdates([]); }, onError: fail("全部更新") });
  const setSync = useMutation({
    mutationFn: (m: string) => ipc("set_skill_sync_method", { method: m }),
    onSuccess: refresh,
    onError: fail("切换分发方式"),
  });
  const showDoc = useMutation({
    mutationFn: (dir: string) => ipc<string>("read_skill_doc", { dir }),
    onSuccess: (content, dir) => setPreview({ dir, content }),
    onError: fail("读取文档"),
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

  return (
    <div className="space-y-4">
      {/* 工具条 */}
      <Card className="flex flex-wrap items-center gap-2">
        <Tabs
          variant="segmented"
          ariaLabel="技能视图"
          value={tab}
          onChange={(id) => setTab(id as Tab)}
          items={(["installed", "matrix", "unmanaged", "discover", "backups", "repos"] as Tab[]).map((t) => ({
            id: t,
            label: { installed: "已安装", matrix: "分配全景", unmanaged: "未纳管", discover: "发现", backups: "备份", repos: "仓库" }[t],
          }))}
        />
        <div className="ml-auto flex items-center gap-2 text-xs text-ink-3">
          <span>分发</span>
          {/* 读失败时不能默认显示「自动」——那是空值兜底，会被当成真值 */}
          {settings.isError && (
            <span className="text-[var(--sem-danger-text)]">
              设置读取失败
              <button
                onClick={() => settings.refetch()}
                className="ml-1 underline underline-offset-2 hover:text-ink-1"
              >
                重试
              </button>
            </span>
          )}
          <select
            value={settings.data?.sync_method ?? "auto"}
            disabled={settings.isError}
            onChange={(e) => setSync.mutate(e.target.value)}
            className="rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-xs"
          >
            <option value="auto">自动（软链优先）</option>
            <option value="symlink">仅软链</option>
            <option value="copy">仅复制</option>
          </select>
          <Button onClick={() => scan.mutate()}>扫描 SSOT</Button>
        </div>
      </Card>


      {tab === "installed" && (
        <Card>
          <div className="mb-3 flex flex-wrap items-center gap-2">
            <SearchInput
              value={q}
              onChange={setQ}
              placeholder="搜索技能名 / 描述 / 仓库"
              inputClassName="w-52"
              count={rows.length}
              total={skills.data?.length ?? 0}
            />
            {(
              [
                ["all", "全部"],
                ["updatable", "可更新"],
                ["local", "本地/未纳管源"],
              ] as const
            ).map(([k, label]) => (
              <button
                key={k}
                onClick={() => setFilter(k)}
                className={`rounded-full border px-2.5 py-0.5 text-2xs ${filter === k ? "border-cyan-500/50 bg-cyan-500/10 text-cyan-300" : "border-slate-700 text-ink-4"}`}
              >
                {label}
              </button>
            ))}
            <div className="ml-auto flex items-center gap-2">
              <Button onClick={() => checkUpd.mutate()}>检查更新</Button>
              <Button variant="primary" disabled={!updates.length} onClick={() => updateAll.mutate()}>
                全部更新{updates.length ? `(${updates.length})` : ""}
              </Button>
            </div>
          </div>

          {/* 读取失败 ≠ 暂无技能：失败给红字 + 重试入口 */}
          {skills.isError ? (
            <QueryError title="读取技能列表失败" error={skills.error} onRetry={() => skills.refetch()} />
          ) : !rows.length ? (
            <p className="text-xs text-ink-4">暂无技能。去「发现」安装，或把目录放进 SSOT 后点「扫描」。</p>
          ) : (
            <table className="w-full text-xs">
              <thead className={theadCls}>
                <tr className="border-b border-slate-800 text-left text-ink-4">
                  <th className="py-1.5 font-medium whitespace-nowrap">技能</th>
                  <th className="font-medium whitespace-nowrap">来源</th>
                  <th className="text-right font-medium whitespace-nowrap">操作</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((s) => (
                  <tr key={s.dir} className="border-b border-slate-800/60 align-top">
                    <td className="py-2 pr-2">
                      <div className="text-ink-2">
                        {s.name}
                        {updSet.has(s.dir) && <span className="ml-2 rounded bg-amber-500/20 px-1 text-2xs text-[var(--sem-busy-text)]">可更新</span>}
                        {!s.managed && <span className="ml-2 rounded bg-slate-600/40 px-1 text-2xs text-ink-3">未纳管</span>}
                      </div>
                      <div className="text-xs text-ink-4">
                        {s.dir}
                        {s.description ? ` · ${s.description}` : ""}
                      </div>
                    </td>
                    <td className="py-2 pr-2 text-ink-3">
                      {s.repo ? `${s.repo}${s.branch ? `@${s.branch}` : ""}` : "本地"}
                    </td>
                    <td className="py-2 text-right whitespace-nowrap">
                      <Button onClick={() => showDoc.mutate(s.dir)}>预览</Button>{" "}
                      {updSet.has(s.dir) && (
                        <Button variant="primary" onClick={() => updateOne.mutate(s.dir)}>更新</Button>
                      )}{" "}
                      <Button variant="danger" onClick={() => setConfirmUninstall({ dir: s.dir, name: s.name })}>卸载</Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      )}

      {tab === "matrix" && <SkillMatrix />}

      {/* 下面四个页签各自成文件（计划 §6.1）：查询与 mutation 跟着页签挂载卸载，
          页面层只留 refresh —— 谁改了数据，就刷新 skills / backups / unmanaged / repos 四把钥匙。 */}
      {tab === "unmanaged" && <UnmanagedSection refresh={refresh} apps={apps} />}

      {tab === "discover" && <DiscoverSection refresh={refresh} />}

      {tab === "backups" && <BackupsSection refresh={refresh} />}

      {tab === "repos" && <ReposSection refresh={refresh} downloadProxy={settings.data?.download_proxy} />}

      {/* 卸载闸门（UI-AUDIT-3 P3-1）：破坏性动作统一走 ConfirmDialog + Risk，
          说清卸载对象、自动备份与恢复路径，样式与双主题一致、按钮文案可控。 */}
      <ConfirmDialog
        open={!!confirmUninstall}
        title={confirmUninstall ? `卸载技能 ${confirmUninstall.name}？` : ""}
        confirmLabel="确认卸载"
        danger
        onClose={() => setConfirmUninstall(null)}
        onConfirm={() => confirmUninstall && uninstall.mutate(confirmUninstall.dir)}
      >
        将从技能库卸载 <b>{confirmUninstall?.name}</b>，并删除各家代理目录里的对应副本。
        <Risk>卸载前会自动备份</Risk>，误删可到「备份」页签还原。
      </ConfirmDialog>

      {/* SKILL.md 预览走 Radix Dialog（UI 审查 P0-3）：焦点陷阱 / Esc 关闭 /
          aria-modal 都是手搜遮罩补不齐的，与 ConfirmDialog、BackendDialog 同族。 */}
      <Dialog.Root open={!!preview} onOpenChange={(o) => !o && setPreview(null)}>
        {preview && (
          <Dialog.Portal>
            <Dialog.Overlay className="fixed inset-0 bg-black/60" />
            <Dialog.Content className="fixed left-1/2 top-1/2 flex max-h-[var(--dialog-max-h)] w-[720px] max-w-[92vw] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-slate-700 bg-slate-900 p-4 shadow-2xl">
              <div className="mb-2 flex items-center justify-between gap-3">
                <Dialog.Title className="truncate font-mono text-xs text-cyan-300" title={`${preview.dir} / SKILL.md`}>
                  {preview.dir} / SKILL.md
                </Dialog.Title>
                <button
                  aria-label="关闭预览"
                  onClick={() => setPreview(null)}
                  className="grid h-7 w-7 shrink-0 place-items-center rounded-md text-ink-4 transition-colors hover:bg-slate-800 hover:text-ink-1"
                >
                  <IconClose width={14} height={14} />
                </button>
              </div>
              <Dialog.Description className="min-h-0 flex-1 overflow-auto">
                <pre className="whitespace-pre-wrap break-all text-xs leading-relaxed text-ink-2">{preview.content}</pre>
              </Dialog.Description>
            </Dialog.Content>
          </Dialog.Portal>
        )}
      </Dialog.Root>
    </div>
  );
}
