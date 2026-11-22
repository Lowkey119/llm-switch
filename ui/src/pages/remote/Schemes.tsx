// 预存方案下拉（用户 2026-11-20 拍板）：多套映射间切换 + 把当前映射存为新方案；
// 11-21 补齐改名 / 导出 / 导入。
//
// 切换是破坏性动作（整体覆盖当前映射），必须弹警告框二次确认才落 remote_scheme_apply；
// 「＋ 另存为」弹名称输入框，同名覆盖。方案存 Rust 侧
// remote/schemes.<profile>.json（actions.rs），档案由后端按绑定设备解析，前端不感知。
// 导出走 blob 下载（Logs 导出同口径）；导入用 file input 读 JSON 文本。
import { useRef, useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { errText } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { ipc } from "../../lib/ipc";
import { t } from "../../lib/i18n";

/** 下拉哨兵值，不会与真实方案名相撞（方案名不许为空）。 */
const SAVE = "__save__";
const RENAME = "__rename__";
const EXPORT = "__export__";
const IMPORT = "__import__";

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
  // 改名 / 导出弹层：目标方案名 + 新名；导入直接开文件选择器，不占弹层。
  const [action, setAction] = useState<"rename" | "export" | null>(null);
  const [target, setTarget] = useState("");
  const fileRef = useRef<HTMLInputElement | null>(null);

  /** 输入名是否命中现有方案（存新方案 / 改名同名都走这条判据）。 */
  const exists = (n: string) => (schemes.data ?? []).includes(n.trim());

  const pick = (v: string) => {
    if (!v) return;
    if (v === SAVE) {
      setName("");
      setSaveOpen(true);
    } else if (v === RENAME || v === EXPORT) {
      setTarget((schemes.data ?? [])[0] ?? "");
      setName("");
      setAction(v === RENAME ? "rename" : "export");
    } else if (v === IMPORT) {
      fileRef.current?.click();
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
        className="rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-sm text-ink-3"
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
            <option value={EXPORT}>{t("remote.scheme.export")}</option>
          </>
        )}
        <option value={IMPORT}>{t("remote.scheme.import")}</option>
      </select>
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
                className="mt-3 w-full rounded border border-slate-700 bg-slate-800/60 px-2 py-1.5 text-sm focus:border-cyan-500/60"
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

      {/* 改名 / 导出：先选目标方案；改名再填新名（新名占用后端报错）。 */}
      {action !== null && (
        <Dialog.Root open onOpenChange={(o) => !o && !busy && setAction(null)}>
          <Dialog.Portal>
            <Dialog.Overlay className="fixed inset-0 z-[90] bg-black/60" />
            <Dialog.Content className="fixed left-1/2 top-1/2 z-[90] w-[400px] max-w-[92vw] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-700 bg-slate-900 p-5 shadow-2xl">
              <Dialog.Title className="text-sm font-semibold">
                {t(action === "rename" ? "remote.scheme.renameTitle" : "remote.scheme.exportTitle")}
              </Dialog.Title>
              <select
                aria-label={t("remote.scheme.pickScheme")}
                value={target}
                onChange={(e) => setTarget(e.target.value)}
                className="mt-3 w-full rounded border border-slate-700 bg-slate-800/60 px-2 py-1.5 text-sm focus:border-cyan-500/60"
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
                  className="mt-2 w-full rounded border border-slate-700 bg-slate-800/60 px-2 py-1.5 text-sm focus:border-cyan-500/60"
                />
              )}
              <div className="mt-4 flex justify-end gap-2">
                <Button onClick={() => setAction(null)}>{t("dlg.cancel")}</Button>
                {action === "rename" ? (
                  <Button variant="primary" disabled={!name.trim() || !target || busy} onClick={() => void rename()}>
                    {t("dlg.save")}
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
    </>
  );
}
