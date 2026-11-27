// halogen 页的小展示件（计划 §6.1 自 HalogenPanel.tsx 拆出，行为不变）。
import { useEffect } from "react";

export function Stat({
  label,
  value,
  tone = "slate",
}: {
  label: string;
  value: string;
  tone?: "slate" | "cyan";
}) {
  return (
    <div>
      <div className="text-2xs uppercase tracking-wide text-ink-4">{label}</div>
      <div className={`font-mono ${tone === "cyan" ? "text-cyan-300" : "text-ink-2"}`}>{value}</div>
    </div>
  );
}

export function LogLine({ line }: { line: string }) {
  const cls = /⚠|错误|失败|异常|error/i.test(line)
    ? "text-[var(--sem-busy-text)]"
    : /✅|完成/.test(line)
      ? "text-[var(--sem-ok-text)]"
      : "text-ink-2";
  return <div className={cls}>{line}</div>;
}

// 跟随滚动：新行到达且用户停在底部时滚到底。
export function AutoScroll({
  pre,
  lines,
  enabled,
}: {
  pre: React.RefObject<HTMLPreElement | null>;
  lines: number;
  enabled: boolean;
}) {
  useEffect(() => {
    const el = pre.current;
    if (el && enabled && el.dataset.atBottom !== "0") el.scrollTop = el.scrollHeight;
  }, [lines, enabled, pre]);
  return null;
}
