import type { ReactNode } from "react";

/**
 * 数字展示（UI-AUDIT P1-4）：等宽 + tabular-nums + 千分位。
 *
 * 轮询刷新的表里数字宽度会变，列宽跟着抖，眼睛追不住哪一格跳了。
 * tabular-nums 让每个数字占同一宽度，千分位让五位数和六位数一眼分得开。
 */
export function Num({
  children,
  digits,
  prefix,
  suffix,
  className,
}: {
  children: number | string | null | undefined;
  digits?: number;
  prefix?: string;
  suffix?: string;
  className?: string;
}) {
  let text: string;
  if (children === null || children === undefined || children === "") {
    text = "—";
  } else if (typeof children === "number") {
    text = children.toLocaleString("en-US", {
      minimumFractionDigits: digits ?? 0,
      maximumFractionDigits: digits ?? 0,
    });
  } else {
    text = children;
  }
  return (
    <span className={`font-mono tabular-nums${className ? ` ${className}` : ""}`}>
      {prefix}
      {text}
      {suffix}
    </span>
  );
}

/** 只格式化不渲染，用于拼进已有文案。 */
export function fmtNum(n: number, digits?: number): string {
  return n.toLocaleString("en-US", {
    minimumFractionDigits: digits ?? 0,
    maximumFractionDigits: digits ?? 0,
  });
}

/** 大数压缩（1.2k / 3.4M），用于卡片这类放不下完整位数的地方。 */
export function compactNum(n: number): ReactNode {
  const abs = Math.abs(n);
  if (abs >= 1e9) return `${(n / 1e9).toFixed(1)}B`;
  if (abs >= 1e6) return `${(n / 1e6).toFixed(1)}M`;
  if (abs >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
  return String(n);
}
