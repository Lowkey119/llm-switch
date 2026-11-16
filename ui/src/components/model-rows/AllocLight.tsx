// 分配灯（计划 §6.1 自 ModelRows.tsx 拆出）：点灯就地弹该端的槽位浮层完成分配；
// 未接管的端由调用方退回跳代理页矩阵（先接管才能写）。
import { gotoMatrix } from "../../lib/nav";
import type { MouseEvent } from "react";

/** 分配灯：给了 onClick 就地分配，没给退回跳矩阵（唯一兜底写入口）。 */
export function AllocLight({
  app,
  slot,
  lit,
  onClick,
}: {
  app: string;
  slot?: string;
  lit: boolean;
  onClick?: (e: MouseEvent) => void;
}) {
  return (
    <button
      onClick={(e) => (onClick ? onClick(e) : gotoMatrix())}
      title={
        lit
          ? `已分配给 ${app}${slot ? `（${slot}）` : ""}；点击就地调整`
          : `未分配给 ${app}；点击就地分配`
      }
      className={`rounded border px-2 py-px text-center text-2xs leading-snug transition-colors ${
        lit
           ? "border-[var(--sem-ok-text)] text-[var(--sem-ok-text)]"
          // 未分配以前是 border-transparent：一颗能点的灯长得跟空白一样，
          // 「这里可以点」全靠悬停才看得出来（UI-AUDIT-2 E-3）。虚线框说「空槽」。
          : "border-dashed border-slate-700 text-ink-4 hover:border-slate-600 hover:text-ink-2"
      }`}
    >
      {app}
      {lit && slot && <span className="block">{slot}</span>}
    </button>
  );
}
