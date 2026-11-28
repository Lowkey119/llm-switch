import { useEffect, useRef, type RefObject } from "react";

/**
 * 自定义浮层的可达性底座（ui_01 回灌 U10）。
 *
 * Radix Dialog 自带焦点陷阱 + `Esc` + 关闭归还，但本仓有三处浮层不是 Radix：
 * 手写浮层 `Popover`（CSS zoom 定位逼出来的，见 `lib/zoom.ts`）、命令面板、首次引导。
 * 原先三处各写一遍或干脆没写：Popover 有陷阱、命令面板只有 `Esc`、引导连 `Esc` 都没有
 * ——键盘用户开进去就出不来。这里收一份三处共用，别再长第四套。
 */

/** 容器内可聚焦元素（DOM 序，跳过禁用项与 `aria-hidden`）。 */
export function focusables(root: HTMLElement): HTMLElement[] {
  return Array.from(
    root.querySelectorAll<HTMLElement>(
      "button, [href], input, select, textarea, [tabindex]:not([tabindex='-1'])",
    ),
  ).filter((el) => !el.hasAttribute("disabled") && el.getAttribute("aria-hidden") !== "true");
}

/**
 * 焦点陷阱：打开时把焦点收进容器（默认第一个可聚焦项），`Tab` 兜在里头转，
 * 关闭/卸载时还给打开它的那个元素。
 *
 * 归还只认真正可聚焦的元素：鼠标点开的 `activeElement` 常是 body，
 * 对 body 调 `focus()` 只会把焦点从别处抢走。
 */
export function useFocusTrap(ref: RefObject<HTMLElement | null>, active = true) {
  useEffect(() => {
    if (!active) return;
    const el = ref.current;
    if (!el) return;
    const prev = document.activeElement as HTMLElement | null;
    const opener = prev && prev !== document.body ? prev : null;
    // 焦点直接落到第一个可聚焦项：容器自己不该带焦点，否则 Tab 要先空走一格。
    (focusables(el)[0] ?? el).focus?.();

    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      const items = focusables(el);
      if (!items.length) return;
      const first = items[0];
      const last = items[items.length - 1];
      // 焦点还在容器自己身上时（没有可聚焦项的兜底），shift+Tab 也往里收。
      if (e.shiftKey && (document.activeElement === first || document.activeElement === el)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    // 捕获阶段：内层组件先看到事件（输入框自己的方向键等），这里只兜边界。
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      opener?.focus?.();
    };
  }, [ref, active]);
}

/**
 * `Esc` 关闭。默认挂捕获阶段并 `stopPropagation`：模态浮层要赢过背后的页面，
 * 也不能一次关掉两层（浮层套在弹窗里时尤其明显）。
 *
 * 非模态的浮层（如控制台全屏）传 `capture: false`，让位给真正打开着的对话框。
 */
export function useEscape(onClose: () => void, active = true, capture = true) {
  const cb = useRef(onClose);
  cb.current = onClose;
  useEffect(() => {
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (capture) e.stopPropagation();
      cb.current();
    };
    document.addEventListener("keydown", onKey, capture);
    return () => document.removeEventListener("keydown", onKey, capture);
  }, [active, capture]);
}
