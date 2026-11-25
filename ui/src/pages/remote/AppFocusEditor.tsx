// App 聚焦策略编辑器（C6）：挂在「打开应用」表单下方，选完 App 就能顺手配聚焦。
//
// 全程改动即存（没有「保存」钮）：策略只有三个互斥值、延迟是一个数，
// 每次改动就是一条 IPC，弹 toast 回执；存失败当场说清，不许静默丢掉。
import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useToast } from "../../components/Toast";
import { errText } from "../../components/QueryError";
import { IconBan, IconClock, IconClose, IconFocusInput, IconKeyboard } from "../../components/icons";
import { ipc } from "../../lib/ipc";
import { t } from "../../lib/i18n";
import {
  DEFAULT_DELAY_MS,
  MAX_DELAY_MS,
  STRATEGY_TABS,
  strategyOf,
  type AppFocusProfile,
  type FocusStrategy,
} from "./appfocus";
import { eventToShortcut, fmtShortcut, type Shortcut } from "./shortcut";

const TAB_ICON = { none: IconBan, ax: IconFocusInput, shortcut: IconKeyboard };

/** 快捷键捕获框：点一下进入待录，按下的组合键即策略；Esc 取消。 */
function ShortcutCapture({ value, onPick }: { value: Shortcut | null; onPick: (s: Shortcut | null) => void }) {
  const [armed, setArmed] = useState(false);
  const ref = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (armed) ref.current?.focus();
  }, [armed]);
  return (
    <div className="flex min-w-0 items-center gap-1.5">
      <input
        ref={ref}
        readOnly
        aria-label={t("remote.focus.capture")}
        value={armed ? t("remote.focus.capturing") : value ? fmtShortcut(value) : ""}
        placeholder={t("remote.focus.capture")}
        className={`min-w-0 flex-1 rounded-lg border bg-slate-950 px-2.5 py-1.5 text-sm text-ink-1 placeholder:text-ink-4 ${
          armed ? "border-[var(--key-hl)]" : "border-slate-700 focus:border-[var(--key-hl)]"
        }`}
        onClick={() => setArmed(true)}
        onKeyDown={(e) => {
          if (!armed) return;
          e.preventDefault();
          e.stopPropagation();
          if (e.key === "Escape") {
            setArmed(false);
            return;
          }
          const sc = eventToShortcut(e.nativeEvent);
          if (sc) {
            onPick(sc);
            setArmed(false);
          }
        }}
      />
      {value && !armed && (
        <button
          type="button"
          className="shrink-0 rounded p-1 text-ink-4 transition-colors hover:text-ink-1"
          aria-label={t("remote.focus.clear")}
          onClick={() => onPick(null)}
        >
          <IconClose className="h-3.5 w-3.5" aria-hidden />
        </button>
      )}
    </div>
  );
}

/**
 * 某个 App 的聚焦策略。`appName` 只用于读数（没给就退回 bundle id）。
 * 策略为「不聚焦」= 后端删条目，打开就是纯打开。
 */
export function AppFocusEditor({ bundleId, appName }: { bundleId: string; appName?: string }) {
  const qc = useQueryClient();
  const toast = useToast();
  const loaded = useQuery({
    queryKey: ["remote-app-focus", bundleId],
    // Tauri v2 把 Rust 侧 snake_case 参数映射成 camelCase 入参：传 bundle_id 会报
    // missing required key bundleId（用户 2026-11-24 实报）。
    queryFn: () => ipc<AppFocusProfile | null>("remote_app_focus_get", { bundleId }),
  });
  // 乐观回显：改动先落在 draft，IPC 落地后清掉（失败保留原值，不假装存上了）。
  const [draft, setDraft] = useState<AppFocusProfile | null>(null);
  const current = draft ?? loaded.data ?? null;
  const strategy = strategyOf(current);
  const delay = current?.delay_ms ?? DEFAULT_DELAY_MS;
  // 延迟框是自由输入，改动先留在本地，失焦/回车才提交（否则每敲一个数字存一次）。
  // 切到「快捷键」但还没录过组合：先本地挂着等用户按键，别凭空存一条没组合的策略。
  const [pendingShortcut, setPendingShortcut] = useState(false);
  const activeType: FocusStrategy["type"] = pendingShortcut ? "shortcut" : strategy.type;
  const [delayRaw, setDelayRaw] = useState(String(delay));
  useEffect(() => setDelayRaw(String(delay)), [bundleId, delay]);

  const save = async (next: AppFocusProfile) => {
    setDraft(next);
    try {
      await ipc("remote_app_focus_set", { def: next });
      void qc.invalidateQueries({ queryKey: ["remote-app-focus", bundleId] });
      toast.ok(t("remote.focus.saved"));
    } catch (e) {
      toast.err(`${t("remote.focus.fail")}：${errText(e)}`);
    } finally {
      setDraft(null);
    }
  };

  const patch = (strategy: FocusStrategy, delayMs = delay) =>
    void save({ bundle_id: bundleId, strategy, delay_ms: delayMs });

  const pickStrategy = (id: FocusStrategy["type"]) => {
    setPendingShortcut(false);
    if (id === "ax") return patch({ type: "ax" });
    if (id === "none") return patch({ type: "none" });
    // 快捷键策略：沿用上次录过的组合；没录过就先挂着捕获框，等用户按键再落库。
    if (strategy.type === "shortcut") return patch({ type: "shortcut", shortcut: strategy.shortcut });
    setPendingShortcut(true);
  };

  const commitDelay = () => {
    const n = Math.min(MAX_DELAY_MS, Math.max(0, Math.round(Number(delayRaw) || 0)));
    setDelayRaw(String(n));
    if (n !== delay) patch(strategy, n);
  };

  if (loaded.isLoading) return null;
  if (loaded.isError) {
    // 读不到策略就不许闷着配：说清楚读不到，用户才知道这里没在生效。
    return (
      <p className="mt-2.5 rounded-[10px] border border-dashed border-slate-600 px-3 py-2 text-sm text-[var(--sem-danger-text)]">
        {errText(loaded.error)}
      </p>
    );
  }
  return (
    <div className="mt-2.5 flex flex-col gap-2.5 rounded-[10px] border border-slate-700 px-3 py-2.5">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <span className="text-sm font-semibold text-ink-1">{t("remote.focus.title")}</span>
        <span className="min-w-0 truncate text-sm text-ink-3">{appName || bundleId}</span>
      </div>
      <div
        role="group"
        aria-label={t("remote.focus.title")}
        className="flex flex-wrap items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-950 p-1"
      >
        {STRATEGY_TABS.map((tab) => {
          const Icon = TAB_ICON[tab.id];
          const on = activeType === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              aria-pressed={on}
              onMouseDown={(e) => e.preventDefault()}
              data-tip={t(tab.tipKey)}
              onClick={() => pickStrategy(tab.id)}
              className={`tip flex items-center gap-1.5 rounded-md px-2.5 py-1 text-sm transition-colors ${
                on ? "bg-[var(--key-hl)]/15 text-[var(--key-hl)]" : "text-ink-3 hover:bg-slate-800 hover:text-ink-1"
              }`}
            >
              <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden />
              {t(tab.labelKey)}
            </button>
          );
        })}
      </div>
      {activeType === "shortcut" && (
        <ShortcutCapture
          value={strategy.type === "shortcut" ? strategy.shortcut : null}
          onPick={(sc) => {
            setPendingShortcut(false);
            patch(sc ? { type: "shortcut", shortcut: sc } : { type: "none" });
          }}
        />
      )}
      <label className="flex items-center gap-2 text-sm text-ink-2">
        <IconClock className="h-3.5 w-3.5 shrink-0 text-ink-3" aria-hidden />
        <span className="shrink-0">{t("remote.focus.delay")}</span>
        <input
          type="number"
          min={0}
          max={MAX_DELAY_MS}
          step={50}
          className="w-20 rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-sm tabular-nums text-ink-1 focus:border-[var(--key-hl)]"
          aria-label={t("remote.focus.delay")}
          value={delayRaw}
          onChange={(e) => setDelayRaw(e.target.value)}
          onBlur={commitDelay}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commitDelay();
            }
          }}
        />
        <span className="text-sm text-ink-4">{t("remote.focus.delayUnit")}</span>
      </label>
    </div>
  );
}
