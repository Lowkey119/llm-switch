/**
 * 命令面板（借鉴 unsloth studio features/find-in-page 的即时过滤 + 跳转）：⌘K / Ctrl+K 唤起，
 * 输入即时过滤页面与动作，回车跳转/执行。
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { PAGES, type PageKey } from "../lib/pages";
import { navigate } from "../lib/nav";
import { replayTour } from "./GuidedTour";
import { toggleTheme } from "../lib/theme";

interface Command {
  id: string;
  label: string;
  hint?: string;
  keywords: string;
  run: () => void;
}

const PAGE_ORDER: PageKey[] = [
  "monitor",
  "backends",
  "logs",
  "usage",
  "console",
  "agents",
  "skills",
  "settings",
];

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const commands = useMemo<Command[]>(() => {
    const pageCmds: Command[] = PAGE_ORDER.map((key) => ({
      id: `page:${key}`,
      label: PAGES[key].title,
      keywords: `${key} ${PAGES[key].title}`,
      run: () => navigate(key),
    }));
    const actions: Command[] = [
      {
        id: "action:theme",
        label: "切换主题",
        hint: "深绿 / 暖白",
        keywords: "theme 主题 深色 浅色 dark light",
        run: () => toggleTheme(),
      },
      {
        id: "action:tour",
        label: "重看引导",
        hint: "重放首次引导",
        keywords: "tour 引导 教程 guide",
        run: () => replayTour(),
      },
    ];
    return [...pageCmds, ...actions];
  }, []);

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
      } else if (e.key === "Escape") {
        setOpen(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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
        className="h-fit w-full max-w-lg overflow-hidden rounded-xl border border-slate-700 bg-slate-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <input
          ref={inputRef}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={onInputKey}
          placeholder="搜索页面或动作…"
          className="w-full border-b border-slate-800 bg-transparent px-4 py-3 text-sm text-ink-1 placeholder:text-ink-4"
        />
        <ul className="max-h-80 overflow-auto p-1.5">
          {filtered.length === 0 && (
            <li className="px-3 py-6 text-center text-sm text-ink-4">无匹配项</li>
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
