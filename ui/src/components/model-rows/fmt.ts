// 模型行共用的格式化（计划 §6.1 自 ModelRows.tsx 拆出）。

export function fmtTok(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}
