import { useState } from "react";
import { IconChevron, IconSort } from "./icons";

/**
 * 共享表格外壳（UI 审查 P1-4）。
 *
 * 此前 13 个文件各写各的 <table>：行密度三种并存，且只有 Logs 有 sticky 表头，
 * 长列表一滚表头就丢。这里定一次口径——
 *
 *   - tableCls：左对齐 + 正文刻度 + 等宽数字（tabular-nums，数字列不再左右抖动）；
 *   - theadCls：sticky 表头 + 不透明底（半透明卡片上滚动内容不会透上来）+ 下边线；
 *   - 密度：px-4 py-2（矩阵类表格自定密度，不套这套）。
 *
 * 注意：sticky 只在「表格随页面滚动」或「外层自己 overflow-auto」时生效。
 * 外层若套 overflow-x-auto，纵向 sticky 会被这个滚动容器吃掉——窄表（≥900px
 * 窗口放得下）不要套 TableShell，直接让表头贴页面滚动区。
 */

export const tableCls = "w-full text-left text-xs tabular-nums";
/** sticky 表头：底色必须不透明（bg-slate-900 与卡片底同值），否则滚动内容透底。
 *  UI-15（用户 2026-11-16，附截图）：sticky 要挂在 th 上而不是 thead 上——
 *  WebKit 不支持 thead 上的 position:sticky，以前表头会被滚动的行穿透。
 *  下边线也画在 th 上：tr 的边框不跟着 sticky 的 th 悬停，会留在原位被滚走。 */
export const theadCls =
  "text-ink-4 [&_tr]:border-b [&_tr]:border-slate-800 [&_th]:sticky [&_th]:top-0 [&_th]:z-10 [&_th]:bg-slate-900 [&_th]:border-b [&_th]:border-slate-800";

/** 横向滚动兜底：只给列数多、可能超 900px 窗口的宽表用（会牺牲页面级 sticky）。 */
export function TableShell({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={`overflow-x-auto${className ? ` ${className}` : ""}`}>{children}</div>;
}

/* ── 可排序表头（ui_01 回灌 U4）────────────────────────────────────────
   以前表格只能看不能排：想看「哪个模型最贵」只能自己用眼睛扫一列数字。
   排序状态机、表头单元格、比较器都只在这里实现一次，消费页只声明列与取值。 */

/** 排序状态：null = 未排序（保持数据原序）。 */
export type SortState<K extends string> = { key: K; dir: "asc" | "desc" } | null;

/**
 * 表头排序状态机：点一次升序、再点降序、第三下回到原序。
 * 做三态而不是两态，是因为「取消排序」在按时间倒序的日志这类列表里是真需求。
 */
export function useSort<K extends string>(initial: SortState<K> = null) {
  const [sort, setSort] = useState<SortState<K>>(initial);
  const toggle = (key: K) =>
    setSort((prev) =>
      prev?.key !== key
        ? { key, dir: "asc" }
        : prev.dir === "asc"
          ? { key, dir: "desc" }
          : null,
    );
  return { sort, toggle };
}

/**
 * 可排序表头单元格。`aria-sort` 是给读屏的真相，箭头只是给眼睛的线索（aria-hidden）。
 * 按钮挂在 th 里面而不是把 th 本身做成按钮：th 承担列标题语义，按钮承担动作。
 */
export function SortTh<K extends string>({
  label,
  k,
  sort,
  onSort,
  className,
  align = "left",
}: {
  label: string;
  k: K;
  sort: SortState<K>;
  onSort: (k: K) => void;
  className?: string;
  align?: "left" | "right";
}) {
  const dir = sort?.key === k ? sort.dir : null;
  return (
    <th
      aria-sort={dir === null ? "none" : dir === "asc" ? "ascending" : "descending"}
      className={`${align === "right" ? "text-right" : "text-left"}${className ? ` ${className}` : ""}`}
    >
      <button
        type="button"
        onClick={() => onSort(k)}
        className={`inline-flex items-center gap-1 font-medium transition-colors ${
          dir ? "text-cyan-300" : "text-ink-4 hover:text-ink-1"
        }`}
      >
        {label}
        {dir === null ? (
          <IconSort aria-hidden width={12} height={12} className="opacity-60" />
        ) : (
          <IconChevron
            aria-hidden
            width={12}
            height={12}
            className={dir === "asc" ? "-rotate-90" : "rotate-90"}
          />
        )}
      </button>
    </th>
  );
}

/**
 * 按列排序。空值不参与比较、一律沉底：缺数据的行不许冒充 0 或空串挤到前面。
 * 返回新数组（[...rows]）——react-query 缓存里那份数组就地排会污染其它消费方。
 */
export function sortRows<T>(
  rows: T[],
  sort: SortState<string>,
  value: (row: T, key: string) => string | number | null | undefined,
): T[] {
  if (!sort) return rows;
  const dir = sort.dir === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    const x = value(a, sort.key);
    const y = value(b, sort.key);
    if (x == null && y == null) return 0;
    if (x == null) return 1;
    if (y == null) return -1;
    if (typeof x === "number" && typeof y === "number") return (x - y) * dir;
    return String(x).localeCompare(String(y), undefined, { numeric: true }) * dir;
  });
}
