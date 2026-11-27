import type { ReactNode } from "react";

/**
 * 可点展开行（UI-AUDIT P0-3）：键盘可达、展开态可被读屏读出。
 *
 * 裸 `<tr onClick>` 的问题不是样式，是它根本不在 Tab 序列里，读屏也听不出
 * 「这一行能按、按了会展开」。这里把 role / tabIndex / Enter+Space / aria-expanded
 * 四件事一次做对，表格行和 div 行共用同一套行为。
 */
function rowKeyDown(onClick: () => void, e: React.KeyboardEvent) {
  if (e.key === "Enter" || e.key === " ") {
    // 空格默认滚动页面，展开行必须吃掉它。
    e.preventDefault();
    onClick();
  }
}

/** 表格里的可展开行。 */
export function TableRow({
  expanded,
  onClick,
  className,
  children,
}: {
  expanded?: boolean;
  onClick: () => void;
  className?: string;
  children: ReactNode;
}) {
  return (
    <tr
      role="button"
      tabIndex={0}
      aria-expanded={expanded}
      onClick={onClick}
      onKeyDown={(e) => rowKeyDown(onClick, e)}
      className={`cursor-pointer${className ? ` ${className}` : ""}`}
    >
      {children}
    </tr>
  );
}

/** 非表格的可展开行（后端列表这类 flex 行）。 */
export function DivRow({
  expanded,
  onClick,
  className,
  children,
}: {
  expanded?: boolean;
  onClick: () => void;
  className?: string;
  children: ReactNode;
}) {
  return (
    <div
      role="button"
      tabIndex={0}
      aria-expanded={expanded}
      onClick={onClick}
      onKeyDown={(e) => rowKeyDown(onClick, e)}
      className={`cursor-pointer${className ? ` ${className}` : ""}`}
    >
      {children}
    </div>
  );
}
