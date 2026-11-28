import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { viewportToLocal, zoomFactor } from "../lib/zoom";
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
 *
 * 智能定向（用户 2026-11-23）：默认贴触发点向右下展开；量出面板实际尺寸后，
 * 下方放不下就向上翻、右边放不下就向左翻（贴到触发点内侧），两头都放不下就
 * 钳进视口留边。矩阵底部行的浮层不再被窗口下缘切掉。
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

  // 先按触发点渲染，useLayoutEffect 量完真实尺寸再定稿（paint 前完成，不闪）。
  const [at, setAt] = useState(() => viewportToLocal(point.x, point.y));

  // 定向计算单独抽出：挂载要算，**内容异步变高也要重算**。
  // 底栏目标组合框实报（计划 2026-11-26 片③）：App 列表是异步查回来的，
  // 挂载时弹层还是空的（量出来很矮→判定“下方放得下”），数据一到长高就直接被窗口下缘裁掉。
  const place = () => {
    const el = ref.current;
    if (!el) return;
    // 统一换算到本地布局空间再比较：getBoundingClientRect 给视觉像素，
    // window.innerWidth 也是视觉像素，而 fixed 的 left/top 走 zoom 树（本地像素），
    // 倍率不为 1 时混用单位会判错方向——全部先除以倍率对齐。
    const z = zoomFactor() || 1;
    const r = el.getBoundingClientRect();
    const w = r.width / z;
    const h = r.height / z;
    const vw = window.innerWidth / z;
    const vh = window.innerHeight / z;
    const pad = 8;
    let x = point.x / z;
    let y = point.y / z;
    if (x + w + pad > vw) x = Math.max(pad, x - w);
    if (y + h + pad > vh) y = Math.max(pad, y - h);
    x = Math.min(Math.max(pad, x), Math.max(pad, vw - w - pad));
    y = Math.min(Math.max(pad, y), Math.max(pad, vh - h - pad));
    setAt({ x, y });
  };

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useLayoutEffect(() => place(), [point.x, point.y]);

  // 内容尺寸一变（异步列表到达 / 图片加载）重新定向；只改 left/top 不改尺寸，不会自激。
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => place());
    ro.observe(el);
    return () => ro.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [point.x, point.y]);

  return (
    <div ref={ref} className="fixed z-50" style={{ left: at.x, top: at.y }}>
      <div className={popoverPanelCls}>{children}</div>
    </div>
  );
}
