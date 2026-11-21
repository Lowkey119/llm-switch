import { useRef, type ReactNode } from "react";

/**
 * 全站唯一 Tab（UI-AUDIT P1-1）。
 *
 * 此前三套 Tab 各有各的边距、字号、选中色，同一个 app 里「选中」长三种样子。
 * 这里一次定死两种形态，并补上此前完全缺失的键盘语义：
 * role=tablist + aria-selected + ←/→（纵向 ↑/↓）切换 + Home/End，
 * 未选中项不进 Tab 序列（roving tabindex），Tab 键直接跳到面板内容。
 */

export interface TabItem {
  id: string;
  label: ReactNode;
  title?: string;
}

export function Tabs({
  items,
  value,
  onChange,
  variant = "underline",
  size = "md",
  ariaLabel,
  className,
}: {
  items: TabItem[];
  value: string;
  onChange: (id: string) => void;
  variant?: "underline" | "segmented";
  /** 疏密档（U1b 后只管间距，不再管字号）：md = 页面级分区，sm = 卡片/面板内的次级条。
      *  两档字号都是 `text-sm`——Tab 是控件，法条 §7.2 把控件定在 14，没有 12 的份。 */
  size?: "sm" | "md";
  ariaLabel?: string;
  className?: string;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);

  const move = (from: number, delta: number) => {
    const to = (from + delta + items.length) % items.length;
    onChange(items[to].id);
    // 焦点跟着选中走：roving tabindex 的前提是选中项必须持有焦点。
    refs.current[to]?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent, i: number) => {
    const k = e.key;
    if (k === "ArrowRight" || k === "ArrowDown") {
      e.preventDefault();
      move(i, 1);
    } else if (k === "ArrowLeft" || k === "ArrowUp") {
      e.preventDefault();
      move(i, -1);
    } else if (k === "Home") {
      e.preventDefault();
      refs.current[0]?.focus();
      onChange(items[0].id);
    } else if (k === "End") {
      e.preventDefault();
      const last = items.length - 1;
      refs.current[last]?.focus();
      onChange(items[last].id);
    }
  };

  if (variant === "segmented") {
    return (
      <div
        role="tablist"
        aria-label={ariaLabel}
        className={`flex overflow-hidden rounded-md border border-slate-700${className ? ` ${className}` : ""}`}
      >
        {items.map((it, i) => {
          const on = it.id === value;
          return (
            <button
              key={it.id}
              ref={(el) => {
                refs.current[i] = el;
              }}
              role="tab"
              aria-selected={on}
              tabIndex={on ? 0 : -1}
              title={it.title}
              onClick={() => onChange(it.id)}
              onKeyDown={(e) => onKeyDown(e, i)}
              className={`px-3 py-1 text-sm transition-colors ${
                on ? "bg-slate-700 text-ink-1" : "text-ink-3 hover:bg-slate-800"
              }`}
            >
              {it.label}
            </button>
          );
        })}
      </div>
    );
  }

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      className={`flex items-end ${size === "sm" ? "gap-4" : "gap-5"}${className ? ` ${className}` : ""}`}
    >
      {items.map((it, i) => {
        const on = it.id === value;
        return (
          <button
            key={it.id}
            ref={(el) => {
              refs.current[i] = el;
            }}
            role="tab"
            aria-selected={on}
            tabIndex={on ? 0 : -1}
            title={it.title}
            onClick={() => onChange(it.id)}
            onKeyDown={(e) => onKeyDown(e, i)}
            className={`border-b-2 transition-colors ${
              size === "sm" ? "-mb-px pb-1.5 text-sm" : "pb-2 text-sm"
            } ${
              on
                ? "border-cyan-400 font-medium text-ink-1"
                : "border-transparent text-ink-4 hover:text-ink-2"
            }`}
          >
            {it.label}
          </button>
        );
      })}
    </div>
  );
}
