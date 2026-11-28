// 「打开应用」选择器（用户 2026-11-24 改版）：不再页内扫已安装列表，
// 点按钮直接弹系统 .app 选择框（NSOpenPanel，默认定位 /Applications），
// 选完回条目由父层写入槽位（与虚拟键盘同一个提交口）。
import { useEffect, useRef, useState } from "react";
import { errText } from "../../components/QueryError";
import { IconAppWindow } from "../../components/icons";
import { t } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";

export interface AppEntry {
  name: string;
  bundle_id: string;
  path: string;
}

/**
 * 选择器本体：一个按钮 + 错误行。系统面板取消 = 静默回到按钮，不算错误。
 * onPick 拿到条目即由父层写入槽位。
 * `auto`（默认开）：挂载即弹面板——点「打开应用」芯片的本意就是直接选应用；
 * 取消后留按钮再开。StrictMode 双挂载用 ref 挡住，面板不弹两次。
 */
export function AppPicker({ onPick, auto = true }: { onPick: (entry: AppEntry) => void; auto?: boolean }) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const browse = async () => {
    setBusy(true);
    setErr(null);
    try {
      const entry = await ipc<AppEntry | null>("remote_app_pick");
      if (entry) onPick(entry);
    } catch (e) {
      setErr(errText(e));
    } finally {
      setBusy(false);
    }
  };
  const started = useRef(false);
  useEffect(() => {
    if (!auto || started.current) return;
    started.current = true;
    void browse();
    // browse 只随挂载跑一次，依赖恒定为 lint 口径。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div className="flex flex-col gap-2 rounded-[10px] border border-dashed border-slate-600 px-3 py-2.5">
      <button
        type="button"
        disabled={busy}
        onClick={() => void browse()}
        className="flex items-center justify-center gap-1.5 rounded-lg border border-slate-600 px-2.5 py-2 text-sm text-ink-1 transition-colors hover:border-[var(--key-hl-text)] hover:text-[var(--key-hl-text)] disabled:cursor-wait disabled:text-ink-4"
      >
        <IconAppWindow className="h-4 w-4 shrink-0" aria-hidden />
        {t("remote.app.browse")}
      </button>
      {err && <p className="text-sm text-[var(--sem-danger-text)]">{err}</p>}
    </div>
  );
}
