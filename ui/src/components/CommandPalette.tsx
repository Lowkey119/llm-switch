/**
 * 命令面板（借鉴 unsloth studio features/find-in-page 的即时过滤 + 跳转）：⌘K / Ctrl+K 唤起，
 * 输入即时过滤页面与动作，回车跳转/执行。
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { useEscape, useFocusTrap } from "./a11y";
import { PAGES, type PageKey } from "../lib/pages";
import { navigate } from "../lib/nav";
import { replayTour } from "./GuidedTour";
import { cyclePref } from "../lib/theme";
import { t, useI18n } from "../lib/i18n";

interface Command {
  id: string;
  label: string;
  hint?: string;
  keywords: string;
  run: () => void;
}

const PAGE_ORDER: PageKey[] = [
  "observe",
  "backends",
  "agents",
  "remote",
  "settings",
];

export function CommandPalette() {
  const { lang } = useI18n();
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  const commands = useMemo<Command[]>(() => {
    const pageCmds: Command[] = PAGE_ORDER.map((key) => ({
      id: `page:${key}`,
      label: t(PAGES[key].titleKey),
      keywords: `${key} ${t(PAGES[key].titleKey)}`,
      run: () => navigate(key),
    }));
    const actions: Command[] = [
      {
        id: "action:theme",
        label: t("palette.theme"),
        hint: t("palette.themeHint"),
        keywords: "theme 主题 深色 浅色 跟随系统 dark light auto",
        run: () => cyclePref(),
      },
      {
        id: "action:tour",
        label: t("palette.tour"),
        hint: t("palette.tourHint"),
        keywords: "tour 引导 教程 guide",
        run: () => replayTour(),
      },
    ];
    return [...pageCmds, ...actions];
    // lang：切语言后标签要跟着重取词
  }, [lang]);

  const filtered = useMemo(() => {
    const s = q.trim().toLowerCase();
    if (!s) return commands;
    return commands.filter((c) => c.keywords.toLowerCase().includes(s));
  }, [q, commands]);

  // ⌘K / Ctrl+K 开合。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Esc 关闭 + 焦点兜在面板里（U10）：面板是模态，原先 Tab 会溜到背后页面上。
  useEscape(() => setOpen(false), open);
  useFocusTrap(panelRef, open);

  // 打开时聚焦、清空查询、复位选中。
  useEffect(() => {
    if (open) {
      setQ("");
      setSel(0);
      inputRef.current?.focus();
    }
  }, [open]);

  // 查询变化后选中复位到首项。
  useEffect(() => setSel(0), [q]);

  if (!open) return null;

  const run = (c: Command) => {
    c.run();
    setOpen(false);
  };

  const onInputKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSel((s) => Math.min(filtered.length - 1, s + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSel((s) => Math.max(0, s - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const c = filtered[sel];
      if (c) run(c);
    }
  };

  return (
    <div
      className="fixed inset-0 z-[90] flex justify-center bg-black/50 p-4 pt-[15vh]"
      onClick={() => setOpen(false)}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={t("palette.title")}
        className="h-fit w-full max-w-lg overflow-hidden rounded-xl border border-slate-700 bg-slate-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <input
          ref={inputRef}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={onInputKey}
          aria-label={t("palette.search")}
          placeholder={`${t("palette.search")}…`}
          className="w-full border-b border-slate-800 bg-transparent px-4 py-3 text-sm text-ink-1 placeholder:text-ink-4"
        />
        <ul className="max-h-80 overflow-auto p-1.5">
          {filtered.length === 0 && (
            <li className="px-3 py-6 text-center text-sm text-ink-4">{t("palette.noMatch")}</li>
          )}
          {filtered.map((c, k) => (
            <li key={c.id}>
              <button
                onMouseEnter={() => setSel(k)}
                onClick={() => run(c)}
                className={`flex w-full flex-col rounded-md px-3 py-2 text-left ${
                  k === sel ? "bg-cyan-500/15 text-cyan-200" : "text-ink-2 hover:bg-slate-800/60"
                }`}
              >
                <span className="text-sm">{c.label}</span>
                <span className="text-2xs text-ink-4">{c.hint}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
