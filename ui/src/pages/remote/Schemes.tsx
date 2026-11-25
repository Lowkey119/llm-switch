// 预存方案下拉（用户 2026-11-20 拍板）：多套映射间切换 + 把当前映射存为新方案；
// 11-21 补齐改名 / 导出 / 导入。
//
// 切换是破坏性动作（整体覆盖当前映射），必须弹警告框二次确认才落 remote_scheme_apply；
// 「＋ 另存为」弹名称输入框，同名覆盖。方案存 Rust 侧
// remote/schemes.<profile>.json（actions.rs），档案由后端按绑定设备解析，前端不感知。
// 导出走 blob 下载（Logs 导出同口径）；导入用 file input 读 JSON 文本。
import { useEffect, useRef, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { errText } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { IconAppWindow } from "../../components/icons";
import { ipc, ipcEvent } from "../../lib/ipc";
import { t } from "../../lib/i18n";

/** 下拉哨兵值，不会与真实方案名相撞（方案名不许为空）。 */
const SAVE = "__save__";
const RENAME = "__rename__";
const DELETE = "__delete__";
const EXPORT = "__export__";
const IMPORT = "__import__";
const BIND = "__bind__";
const UNBIND = "__unbind__";

/** 生效来源（C5）：默认映射，或某个 App 绑定的方案。 */
type Source =
  | { kind: "default" }
  | { kind: "app"; app: string | null; bundleId: string | null; scheme: string };

interface FrontRead {
  app: { bundle_id: string; name: string } | null;
  source: Source;
}

export function SchemeSelect() {
  const qc = useQueryClient();
  const toast = useToast();
  const schemes = useQuery({
    queryKey: ["remote-schemes"],
    queryFn: () => ipc<string[]>("remote_schemes_get"),
  });
  // 动作在弹层里完成，选完即复位；唯一留显的是「最近切换到的方案」（用户 2026-11-21 拍板）：
  // 收起时组合框显示方案名，展开时勾也停在它上面。改名/删除后名字不在列表就自动回落占位。
  const [applied, setApplied] = useState("");
  const shown = (schemes.data ?? []).includes(applied) ? applied : "";
  const [confirmName, setConfirmName] = useState<string | null>(null);
  const [saveOpen, setSaveOpen] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  // 改名 / 删除 / 导出弹层：目标方案名 + 新名；导入直接开文件选择器，不占弹层。
  const [action, setAction] = useState<"rename" | "export" | "delete" | null>(null);
  const [target, setTarget] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);
  const fileRef = useRef<HTMLInputElement | null>(null);
  // 前台 App 换方案（C5）：初值读一次，之后跟着 `remote://front-app` 事件走。
  const front = useQuery({
    queryKey: ["remote-front-app"],
    queryFn: () => ipc<FrontRead>("remote_front_app_get"),
    staleTime: Infinity,
  });
  const bindings = useQuery({
    queryKey: ["remote-app-bindings"],
    queryFn: () => ipc<Record<string, string>>("remote_app_bindings_get"),
    staleTime: Infinity,
  });
  const [live, setLive] = useState<Source | null>(null);
  const [bindOpen, setBindOpen] = useState(false);
  const [bindScheme, setBindScheme] = useState("");
  const [confirmUnbind, setConfirmUnbind] = useState(false);
  useEffect(() => {
    let dead = false;
    let un: (() => void) | undefined;
    // 换前台 = 生效映射可能整套换掉，页面必须跟着回读，否则显示的还是上一个 App 的键位。
    void ipcEvent<Source>("remote://front-app", (s) => {
      if (dead) return;
      setLive(s);
      void qc.invalidateQueries({ queryKey: ["remote-buttons"] });
    }).then((f) => (dead ? f() : (un = f)));
    return () => {
      dead = true;
      un?.();
    };
    // qc 由 react-query 提供、引用稳定，进依赖只为满足 lint，不会重挂监听。
  }, [qc]);
  const frontApp = front.data?.app ?? null;
  const source: Source | null = live ?? front.data?.source ?? null;
  const boundToFront = !!frontApp && !!(bindings.data ?? {})[frontApp.bundle_id];
  /** 前台 App 换绑后三处要一起回读：绑定表、生效来源、键位映射（换方案＝映射变了）。 */
  const refreshFront = () => {
    setLive(null);
    void qc.invalidateQueries({ queryKey: ["remote-app-bindings"] });
    void qc.invalidateQueries({ queryKey: ["remote-front-app"] });
    void qc.invalidateQueries({ queryKey: ["remote-buttons"] });
  };

  const bind = async () => {
    if (!frontApp || !bindScheme || busy) return;
    setBusy(true);
    try {
      await ipc("remote_app_binding_set", { bundle_id: frontApp.bundle_id, scheme: bindScheme });
      toast.ok(t("remote.bind.bound"));
      setBindOpen(false);
      refreshFront();
    } catch (e) {
      toast.err(`${t("remote.bind.fail")}：${errText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const unbind = async () => {
    if (!frontApp || busy) return;
    setBusy(true);
    try {
      await ipc("remote_app_binding_set", { bundle_id: frontApp.bundle_id, scheme: null });
      toast.ok(t("remote.bind.unbound"));
      refreshFront();
    } catch (e) {
      toast.err(`${t("remote.bind.fail")}：${errText(e)}`);
    } finally {
      setBusy(false);
      setConfirmUnbind(false);
    }
  };

  /** 输入名是否命中现有方案（存新方案 / 改名同名都走这条判据）。 */
  const exists = (n: string) => (schemes.data ?? []).includes(n.trim());

  const pick = (v: string) => {
    if (!v) return;
    if (v === SAVE) {
      setName("");
      setSaveOpen(true);
    } else if (v === RENAME || v === EXPORT || v === DELETE) {
      setTarget((schemes.data ?? [])[0] ?? "");
      setName("");
      setAction(v === RENAME ? "rename" : v === EXPORT ? "export" : "delete");
    } else if (v === IMPORT) {
      fileRef.current?.click();
    } else if (v === BIND) {
      setBindScheme(applied || (schemes.data ?? [])[0] || "");
      setBindOpen(true);
    } else if (v === UNBIND) {
      setConfirmUnbind(true);
    } else {
      setConfirmName(v);
    }
  };

  const apply = async () => {
    if (!confirmName) return;
    setBusy(true);
    try {
      await ipc("remote_scheme_apply", { name: confirmName });
      setApplied(confirmName);
      toast.ok(t("remote.scheme.applied"));
      void qc.invalidateQueries({ queryKey: ["remote-buttons"] });
    } catch (e) {
      toast.err(`${t("remote.scheme.applyFail")}：${errText(e)}`);
    } finally {
      setBusy(false);
      setConfirmName(null);
    }
  };

  const save = async () => {
    const n = name.trim();
    if (!n || busy) return;
    setBusy(true);
    try {
      await ipc("remote_scheme_save", { name: n });
      toast.ok(t("remote.scheme.saved"));
      void qc.invalidateQueries({ queryKey: ["remote-schemes"] });
    } catch (e) {
      toast.err(`${t("remote.scheme.saveFail")}：${errText(e)}`);
    } finally {
      setBusy(false);
      setSaveOpen(false);
    }
  };

  const rename = async () => {
    const n = name.trim();
    if (!n || !target || busy) return;
    setBusy(true);
    try {
      await ipc("remote_scheme_rename", { old: target, new: n });
      toast.ok(t("remote.scheme.renamed"));
      void qc.invalidateQueries({ queryKey: ["remote-schemes"] });
      setAction(null);
    } catch (e) {
      toast.err(`${t("remote.scheme.renameFail")}：${errText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const removeScheme = async () => {
    if (!target || busy) return;
    setBusy(true);
    try {
      await ipc("remote_scheme_delete", { name: target });
      toast.ok(t("remote.scheme.deleted"));
      // 删掉的正是最近应用的方案时回落占位，不然组合框显示一个没了的名字。
      if (applied === target) setApplied("");
      void qc.invalidateQueries({ queryKey: ["remote-schemes"] });
      // 后端会一并清掉指向该方案的 App 绑定，绑定表与生效来源要跟着回读。
      void qc.invalidateQueries({ queryKey: ["remote-app-bindings"] });
      void qc.invalidateQueries({ queryKey: ["remote-front-app"] });
    } catch (e) {
      toast.err(`${t("remote.scheme.deleteFail")}：${errText(e)}`);
    } finally {
      setBusy(false);
      setConfirmDelete(false);
      setAction(null);
    }
  };

  const exportScheme = async () => {
    if (!target || busy) return;
    setBusy(true);
    try {
      const text = await ipc<string>("remote_scheme_export", { name: target });
      const url = URL.createObjectURL(new Blob([text], { type: "application/json" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = `llm-switch-scheme-${target}.json`;
      a.click();
      URL.revokeObjectURL(url);
      toast.ok(t("remote.scheme.exported"));
      setAction(null);
    } catch (e) {
      toast.err(`${t("remote.scheme.exportFail")}：${errText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  // 导入：方案名 = 文件名去 .json 后缀；同名覆盖（与存新方案同口径）。
  const importFile = async (f: File | undefined) => {
    if (!f) return;
    setBusy(true);
    try {
      await ipc("remote_scheme_import", {
        name: f.name.replace(/\.json$/i, ""),
        content: await f.text(),
      });
      toast.ok(t("remote.scheme.imported"));
      void qc.invalidateQueries({ queryKey: ["remote-schemes"] });
    } catch (e) {
      toast.err(`${t("remote.scheme.importFail")}：${errText(e)}`);
    } finally {
      setBusy(false);
      if (fileRef.current) fileRef.current.value = "";
    }
  };

  return (
    <>
      <select
        aria-label={t("remote.map.schemes")}
        value={shown}
        onChange={(e) => pick(e.target.value)}
        className="max-w-40 truncate rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-sm text-ink-3"
      >
        <option value="">{t("remote.map.schemes")}</option>
        {(schemes.data ?? []).map((s) => (
          <option key={s} value={s}>
            {s}
          </option>
        ))}
        <option value={SAVE}>{t("remote.scheme.saveNew")}</option>
        {(schemes.data ?? []).length > 0 && (
          <>
            <option value={RENAME}>{t("remote.scheme.rename")}</option>
            <option value={DELETE}>{t("remote.scheme.delete")}</option>
            <option value={EXPORT}>{t("remote.scheme.export")}</option>
          </>
        )}
        <option value={IMPORT}>{t("remote.scheme.import")}</option>
        {/* App 绑定（C5）：把某套方案挂到当前前台 App 上，切过去自动换、切走自动回。 */}
        {frontApp && <option value={BIND}>{t("remote.bind.toFront")}</option>}
        {frontApp && boundToFront && <option value={UNBIND}>{t("remote.bind.unbind")}</option>}
      </select>
      {/* 生效来源徽标（C5）：按 App 热加载了哪个方案一眼可见——不然改了键位不知道写进哪儿。 */}
      {source?.kind === "app" && (
        <span
          className="tip -my-0.5 flex min-w-0 items-center gap-1 rounded border border-[var(--key-hl)]/40 px-1.5 py-0.5 text-2xs text-[var(--key-hl)]"
          data-tip={t("remote.bind.viaAppTip").replace("{n}", source.app ?? source.bundleId ?? "")}
        >
          <IconAppWindow className="h-3 w-3 shrink-0" aria-hidden />
          <span className="tip max-w-[8rem] truncate" data-tip={source.app ?? source.bundleId}>{source.app ?? source.bundleId}</span>
          <span className="shrink-0 text-ink-3">→</span>
          <span className="tip max-w-[6rem] truncate" data-tip={source.scheme}>{source.scheme}</span>
        </span>
      )}
      {/* 导入的隐藏文件选择器：选完读文本走 remote_scheme_import。 */}
      <input
        ref={fileRef}
        type="file"
        accept=".json,application/json"
        className="hidden"
        onChange={(e) => void importFile(e.target.files?.[0])}
      />

      {/* 切换二次确认（必做）：整体覆盖当前映射是破坏性动作；收敛到通用 ConfirmDialog（审查 2.9）。 */}
      <ConfirmDialog
        open={confirmName !== null}
        title={t("remote.scheme.applyTitle")}
        onClose={() => setConfirmName(null)}
        onConfirm={() => void apply()}
        confirmLabel={t("remote.scheme.confirm")}
        danger
      >
        {t("remote.scheme.applyDesc").replace("{n}", confirmName ?? "")}
      </ConfirmDialog>

      {/* 存为新方案：名称输入，同名覆盖。 */}
      {saveOpen && (
        <Dialog.Root open onOpenChange={(o) => !o && !busy && setSaveOpen(false)}>
          <Dialog.Portal>
            <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
            <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] w-[400px] max-w-[92vw] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
              <Dialog.Title className="text-sm font-semibold">
                {t("remote.scheme.saveTitle")}
              </Dialog.Title>
              <input
                autoFocus
                value={name}
                onChange={(e) => setName(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && void save()}
                placeholder={t("remote.scheme.namePh")}
                className="mt-3 w-full rounded border border-slate-700 bg-slate-800/60 px-2 py-1.5 text-sm focus:border-[var(--key-hl)]"
              />
              {/* 同名覆盖必须说破（审查 1.3）：手滑覆盖一套调了很久的映射不可撤销。 */}
              {exists(name) && (
                <p className="mt-2 text-2xs text-[var(--sem-busy-text)]">{t("remote.scheme.overwriteWarn")}</p>
              )}
              <div className="mt-4 flex justify-end gap-2">
                <Button onClick={() => setSaveOpen(false)}>{t("dlg.cancel")}</Button>
                <Button variant="primary" disabled={!name.trim() || busy} onClick={() => void save()}>
                  {t("dlg.save")}
                </Button>
              </div>
            </Dialog.Content>
          </Dialog.Portal>
        </Dialog.Root>
      )}

      {/* 绑前台 App（C5）：选一套方案挂到它身上；切走该 App 自动回默认映射。 */}
      {bindOpen && frontApp && (
        <Dialog.Root open onOpenChange={(o) => !o && !busy && setBindOpen(false)}>
          <Dialog.Portal>
            <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
            <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] w-[400px] max-w-[92vw] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
              <Dialog.Title className="text-sm font-semibold">{t("remote.bind.title")}</Dialog.Title>
              <p className="mt-2 text-sm leading-relaxed text-ink-3">
                {t("remote.bind.desc").replace("{n}", frontApp.name)}
              </p>
              <p className="mt-2 truncate font-mono text-2xs text-ink-4">{frontApp.bundle_id}</p>
              <select
                aria-label={t("remote.bind.scheme")}
                className="mt-3 block w-full rounded-lg border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-sm text-ink-1"
                value={bindScheme}
                onChange={(e) => setBindScheme(e.target.value)}
              >
                {(schemes.data ?? []).map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </select>
              {(schemes.data ?? []).length === 0 && (
                <p className="mt-2 text-2xs text-[var(--sem-danger-text)]">{t("remote.bind.noScheme")}</p>
              )}
              <div className="mt-4 flex justify-end gap-2">
                <Button onClick={() => setBindOpen(false)}>{t("dlg.cancel")}</Button>
                <Button
                  variant="primary"
                  disabled={!bindScheme || busy}
                  onClick={() => void bind()}
                >
                  {t("remote.bind.confirm")}
                </Button>
              </div>
            </Dialog.Content>
          </Dialog.Portal>
        </Dialog.Root>
      )}

      {/* 解绑：切回该 App 不再自动换方案。 */}
      <ConfirmDialog
        open={confirmUnbind}
        title={t("remote.bind.unbindTitle")}
        onClose={() => setConfirmUnbind(false)}
        onConfirm={() => void unbind()}
        confirmLabel={t("remote.bind.unbind")}
      >
        {t("remote.bind.unbindDesc").replace("{n}", frontApp?.name ?? "")}
      </ConfirmDialog>

      {/* 改名 / 删除 / 导出：先选目标方案；改名再填新名（新名占用后端报错）；
          删除走 ConfirmDialog 二次确认（不可逆，且会清掉指向它的 App 绑定）。 */}
      {action !== null && (
        <Dialog.Root open onOpenChange={(o) => !o && !busy && setAction(null)}>
          <Dialog.Portal>
            <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
            <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] w-[400px] max-w-[92vw] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
              <Dialog.Title className="text-sm font-semibold">
                {t(
                  action === "rename"
                    ? "remote.scheme.renameTitle"
                    : action === "delete"
                      ? "remote.scheme.deleteTitle"
                      : "remote.scheme.exportTitle"
                )}
              </Dialog.Title>
              <select
                aria-label={t("remote.scheme.pickScheme")}
                value={target}
                onChange={(e) => setTarget(e.target.value)}
                className="mt-3 w-full rounded border border-slate-700 bg-slate-800/60 px-2 py-1.5 text-sm focus:border-[var(--key-hl)]"
              >
                {(schemes.data ?? []).map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </select>
              {action === "rename" && (
                <input
                  autoFocus
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  onKeyDown={(e) => e.key === "Enter" && void rename()}
                  placeholder={t("remote.scheme.newNamePh")}
                  className="mt-2 w-full rounded border border-slate-700 bg-slate-800/60 px-2 py-1.5 text-sm focus:border-[var(--key-hl)]"
                />
              )}
              <div className="mt-4 flex justify-end gap-2">
                <Button onClick={() => setAction(null)}>{t("dlg.cancel")}</Button>
                {action === "rename" ? (
                  <Button variant="primary" disabled={!name.trim() || !target || busy} onClick={() => void rename()}>
                    {t("dlg.save")}
                  </Button>
                ) : action === "delete" ? (
                  <Button variant="danger" disabled={!target || busy} onClick={() => setConfirmDelete(true)}>
                    {t("remote.scheme.delete")}
                  </Button>
                ) : (
                  <Button variant="primary" disabled={!target || busy} onClick={() => void exportScheme()}>
                    {t("remote.scheme.export")}
                  </Button>
                )}
              </div>
            </Dialog.Content>
          </Dialog.Portal>
        </Dialog.Root>
      )}

      {/* 删除的最后一道闸：目标已选定，这里只问「确定删吗」（不可逆 + 解绑 App 绑定）。 */}
      <ConfirmDialog
        open={confirmDelete}
        title={`${t("remote.scheme.deleteTitle")}「${target}」`}
        onClose={() => setConfirmDelete(false)}
        onConfirm={() => void removeScheme()}
        confirmLabel={t("remote.scheme.delete")}
        danger
        irreversible
      >
        {t("remote.scheme.deleteDesc").replace("{n}", target)}
      </ConfirmDialog>
    </>
  );
}
