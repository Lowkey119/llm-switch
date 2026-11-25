import type { ReactNode } from "react";
import { IconClose } from "./icons";
import { t, tf, ts } from "../lib/i18n";

/**
 * 统一过滤条（ui_01 原型第 6 条回灌，U6）。
 *
 * 此前每页各拼各的：日志是一排裸控件挤在 flex 里、技能是搜索框加三个手写 pill、
 * 后端页压根没有过滤。同一个「我正在筛什么、筛掉多少、怎么清掉」在三处三种说法，
 * 而且除了搜索框自带的清空键，**没有任何地方能一眼看出当前生效的条件有哪些**。
 *
 * 这里只收「筛选状态的呈现」：已选条件 chip（单个清除 / 一键清除）+ 命中计数。
 * 控件本身仍由页面自己给（搜索框 / 分段 / 下拉的维度是页面的事，不是共享的事），
 * 所以本组件是个容器而不是一个大表单——避免为了统一把三页塞进同一套 props 里。
 *
 * 排布口径（1280 实测出来的）：计数与已选条件 chip 都**紧跟过滤控件**，页面动作由
 * `ml-auto` 顶到行尾。此前计数和 chip 一起被塞在右边那一格里，日志页上「0 条 清除过滤」
 * 直接贴住「实时 tail 当前级别…」，筛选结果和页面动作糊成一串没有归属的字。
 *
 * 计数口径：知道总数就说「命中 n / 总数」，不知道（后端分页/条件查询）就说「n 条」，
 * 不拿当前页行数冒充全量。
 */

export interface FilterTag {
  /** 稳定标识，用于 key。 */
  key: string;
  /** 维度名，如「级别」。 */
  label: string;
  /** 当前值，如「error」。布尔型条件省略它，chip 只写维度名（不写「仅错误：是」这种废话）。 */
  value?: string;
  /** 清掉这一个条件。 */
  clear: () => void;
}

/**
 * 「清除过滤」链接：过滤条的 chip 行与各页「筛没了」空态共用同一颗。
 * 同一个动作此前在日志空态、后端空态、chip 行里各写一遍下划线按钮，正是 U6 要消掉的重复。
 */
export function ClearFiltersButton({ onClick, className = "" }: { onClick: () => void; className?: string }) {
  return (
    <button
      onClick={onClick}
      className={`underline underline-offset-2 transition-colors hover:text-ink-2 ${className}`}
    >
      {t("filter.clearAll")}
    </button>
  );
}

export function FilterBar({
  children,
  count,
  total,
  active = [],
  onClearAll,
  trailing,
  className = "",
}: {
  /** 过滤控件（搜索框 / 分段 / 下拉），顺序即视觉主次。 */
  children: ReactNode;
  /** 过滤后的行数；不传就不显示计数（例如条里只放动作）。 */
  count?: number;
  /** 过滤前的总行数。省略表示总数未知，此时不硬凑一个分母。 */
  total?: number;
  /** 当前生效的条件。空数组时整条 chip 行与「清除过滤」都不出现。 */
  active?: FilterTag[];
  /** 一键清除。只在有生效条件时渲染，落在 chip 行末尾——紧挨着它要清掉的那些 chip。 */
  onClearAll?: () => void;
  /**
   * 不是过滤的页面动作（实时 tail、当前日志级别、导出、清理…）。
   * UI-16 定案这些控件与过滤条同占一行（密度），但它们是设置与动作，
   * 不该进已选条件 chip 也不该参与计数，所以单独一个尾部槽位，不混进 children。
   */
  trailing?: ReactNode;
  className?: string;
}) {
  return (
    <div className={`flex flex-wrap items-center gap-2 text-sm ${className}`}>
      {children}
      {count !== undefined && (
        <span className="ml-1 text-ink-4 tabular-nums">
          {total !== undefined ? tf("filter.hit", { n: count, total }) : tf("filter.count", { n: count })}
        </span>
      )}
      {/* chip 跟控件同流、不另起整行：另起整行会把尾部动作顶到上一行，
          「已选条件」反而离它描述的控件更远；同流时窄窗换行也始终贴着自己的控件。 */}
      {active.length > 0 && (
        <span className="flex items-center gap-1">
          {active.map((f) => (
            <span
              key={f.key}
              className="inline-flex items-center gap-1 rounded border border-cyan-500/30 bg-cyan-500/10 px-2 py-0.5 text-2xs text-ink-1"
            >
              {f.value ? (
                <>
                  <span className="text-ink-4">{f.label}：</span>
                  {f.value}
                </>
              ) : (
                f.label
              )}
              <button
                onClick={f.clear}
                aria-label={ts("filter.clearOne", { label: f.label })}
                className="rounded p-0.5 text-ink-4 transition-colors hover:text-ink-1"
              >
                <IconClose width={10} height={10} />
              </button>
            </span>
          ))}
          {onClearAll && <ClearFiltersButton onClick={onClearAll} className="ml-1" />}
        </span>
      )}
      {trailing && <span className="ml-auto flex items-center gap-2">{trailing}</span>}
    </div>
  );
}
