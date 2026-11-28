// 后端取用三态控件（计划 2026-11-18-backend-direct-mode）：停用 / 网关 / 直连。
// 复用 Tabs 的 segmented 档（roving tabindex 键盘导航现成），不新造分段控件。
// 切到「直连」必须先过确认框（后果 + 可逆说明），确认后才发请求；
// 环境变量缺失另出一块提示（what → why → how + 一键复制 export），按计划「文案」节口径。
import { useState } from "react";
import { Tabs } from "./Tabs";
import { ConfirmDialog } from "./ConfirmDialog";
import { CopyButton } from "./CopyButton";
import { useToast } from "./Toast";
import { copyText } from "../lib/clipboard";
import { ipc, type BackendRow } from "../lib/ipc";
import { t, ts } from "../lib/i18n";

type Mode = "off" | "gateway" | "direct";

interface DirectResult {
  rewritten: number;
  rewrite_failed: { app?: string }[];
  key_var: string;
  key_var_set: boolean;
}

export function DirectModeControl({
  b,
  managedCount,
  onDisable,
  onEnable,
  onChanged,
}: {
  b: BackendRow;
  /** 已接管（managed）的代理数：确认框与 toast 的 N。 */
  managedCount: number;
  /** 停用走父页 mutation：halogen/gufo 同端口互斥的「顺手关了谁」提示在那边。 */
  onDisable: () => void;
  /** 启用同走父页 mutation（互斥提示 + 刷新都在那边）：停用态切回网关/直连先补启用。 */
  onEnable: () => void;
  onChanged: () => void;
}) {
  const toast = useToast();
  const [confirming, setConfirming] = useState(false);
  const [envWarn, setEnvWarn] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const mode: Mode = !b.enabled ? "off" : b.direct ? "direct" : "gateway";

  const apply = async (next: Mode) => {
    setBusy(true);
    try {
      if (next === "off") {
        onDisable();
        return;
      }
      // 停用态切回网关/直连 = 要重新用这家：set_backend_direct 不碰 enabled（计划
      // 2026-11-18 决策 2/4），必须补一步启用，否则 toast 报成功而模式仍停在停用。
      if (mode === "off") onEnable();
      const back = next === "gateway";
      const r = await ipc<DirectResult>("set_backend_direct", { name: b.name, direct: !back });
      const failed = (r.rewrite_failed ?? [])
        .map((x) => x.app)
        .filter(Boolean)
        .join("、");
      if (failed) {
        toast.err(
          ts(back ? "bk.toast.directOffPartial" : "bk.toast.directPartial", {
            n: r.rewrite_failed.length,
            list: failed,
          }),
        );
      } else {
        toast.ok(ts(back ? "bk.toast.directOff" : "bk.toast.directOn", { n: r.rewritten }));
      }
      // 变量名由后端回传（与写入端同源），这里只回显。
      if (!back && !r.key_var_set) setEnvWarn(r.key_var);
      onChanged();
    } catch (e) {
      toast.err(ts("bk.toast.toggleFail", { e: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  const onChange = (next: string) => {
    if (busy || next === mode) return;
    if (next === "direct") {
      setConfirming(true);
      return;
    }
    void apply(next as Mode);
  };

  return (
    <>
      <span
        title={t("row.mode.tooltip")}
        aria-disabled={busy || undefined}
        className={busy ? "pointer-events-none" : ""}
      >
        <Tabs
          variant="segmented"
          size="sm"
          ariaLabel={t("row.mode.tooltip")}
          value={mode}
          onChange={onChange}
          items={[
            { id: "off", label: t("row.mode.off") },
            { id: "gateway", label: t("row.mode.gateway") },
            { id: "direct", label: t("row.mode.direct") },
          ]}
        />
      </span>
      {b.direct && (
        <span
          title={`${t("row.directBadgeTip")} ${b.base_url}`}
          className="flex max-w-[260px] items-center gap-1 rounded border border-slate-700 px-1.5 py-0.5 text-2xs text-ink-3"
        >
          <span className="truncate">
            {t("row.mode.direct")} {b.base_url}
          </span>
          {/* 「查看直连地址」动作：地址被 truncate 时一键拷走全文（UI 法条：省略号必配拷贝）。 */}
          <CopyButton text={b.base_url} label={`${t("row.directViewAddr")}：${b.base_url}`} />
        </span>
      )}
      <ConfirmDialog
        open={confirming}
        title={ts("dlg.directTitle", { name: b.name })}
        confirmLabel={t("dlg.directConfirm")}
        onClose={() => setConfirming(false)}
        onConfirm={() => void apply("direct")}
      >
        <p>{t("dlg.directB1")}</p>
        <ul className="mt-1 list-disc pl-4">
          <li>{ts("dlg.directB2", { name: b.name })}</li>
          <li>{ts("dlg.directB3", { n: managedCount, url: b.base_url })}</li>
          <li>{ts("dlg.directB4", { var: b.key_var })}</li>
        </ul>
        <p className="mt-2">{t("dlg.directB5")}</p>
      </ConfirmDialog>
      <ConfirmDialog
        open={envWarn !== null}
        title={ts("row.envMissingTitle", { var: envWarn ?? "" })}
        confirmLabel={t("row.envCopy")}
        onClose={() => setEnvWarn(null)}
        onConfirm={() => {
          // 复制走全站唯一入路（与 CopyButton 同源）；成败必须说出来，静默假装成功比不复制更坏。
          void copyText(`export ${envWarn}=sk-…`).then((ok) =>
            ok ? toast.ok(t("row.envCopied")) : toast.err(t("row.envCopyFail")),
          );
          setEnvWarn(null);
        }}
      >
        <p>{ts("row.envMissingBody", { var: envWarn ?? "" })}</p>
        <p className="mt-2">
          <code className="break-all font-mono text-2xs text-ink-2">export {envWarn}=sk-…</code>
        </p>
      </ConfirmDialog>
    </>
  );
}
