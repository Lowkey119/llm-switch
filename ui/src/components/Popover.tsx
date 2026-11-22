import { useEffect, useRef, type ReactNode } from "react";
import { viewportToLocal } from "../lib/zoom";
import { useEscape, useFocusTrap } from "./a11y";

/**
 * 浮层单一真相（UI-AUDIT P0-3）。
 *
 * 此前三个手写浮层各自 `useEffect` 挂 mousedown 判外点，没有 Esc、没有焦点管理，
 * 键盘根本进不去。这里把四件事一次做对：外点关闭、Esc 关闭、焦点进浮层、
 * 关闭后焦点归还给打开它的元素。
 *
 * 为什么没用 Radix Popover：应用把 CSS `zoom` 装在 documentElement 上
 * （取舍见 lib/zoom.ts），Radix 的碰撞定位读的是视觉像素、写回的却是 zoom 树里的
 * CSS 像素，倍率不为 1 时浮层会成倍飘偏，而它没有留出改写坐标的口子。
 * 所以坐标换算收在 lib/zoom.ts 的 viewportToLocal() 一处，组件里不出现除法。
 */

/** 内容外观：三个浮层此前各写一遍的边框/底色/圆角/阴影。 */
export const popoverPanelCls =
  "rounded-lg border border-slate-700 bg-slate-900 p-1 shadow-xl";

/** 浮层里的菜单项。 */
export const popoverItemCls =
  "flex w-full items-center justify-between gap-3 rounded px-2.5 py-1 text-left text-sm text-ink-2 hover:bg-slate-800";

/**
 * 贴在「触发点」上的浮层：调用方已经知道视口坐标（点表格单元格弹出来的那种）。
 * open 由调用方持有，组件只在 open 为真时挂载。
 */
export function PointPopover({
  point,
  onClose,
  children,
}: {
  /** 视口坐标（未除倍率），即 MouseEvent 的 clientX / clientY。 */
  point: { x: number; y: number };
  onClose: () => void;
  children: ReactNode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  // 焦点进浮层 / Tab 兜在里头 / Esc / 关闭归还给打开它的人——与命令面板、首次引导
  // 共用一份（a11y.ts）：浮层是临时模态，焦点不该从背后溜到表格上。
  useFocusTrap(ref);
  useEscape(onClose);

  // 外点关闭：挂到下一个宏任务，躲开触发点击自身（否则刚打开就被自己判成「点外面」）。
  useEffect(() => {
    const onOutside = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const t = setTimeout(() => document.addEventListener("mousedown", onOutside), 0);
    return () => {
      clearTimeout(t);
      document.removeEventListener("mousedown", onOutside);
    };
  }, [onClose]);

  // 先钳回视口内（视口坐标），再换算到本地布局空间，顺序反了钳不住。
  const at = viewportToLocal(Math.min(point.x, window.innerWidth - 200), point.y);
  return (
    <div ref={ref} className="fixed z-50" style={{ left: at.x, top: at.y }}>
      <div className={popoverPanelCls}>{children}</div>
    </div>
  );
}
