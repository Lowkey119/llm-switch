// 提示词面板的全局键盘口径（自 PromptPanel 拆出）。
//
// 口径：Esc 永远收窗（收窗前先调 flush 把右栏编辑中的改动落盘，防抖窗口可能还
// 没到点）；焦点在搜索框时方向键与回车**照样**驱动面板（上下移选择、回车注入），
// 左右键只在光标到头/尾或搜索框为空时换 tab；焦点在多行编辑器时方向键归文字
// 光标；空手里直接打字跳进搜索框。
import { useEffect, type Dispatch, type RefObject, type SetStateAction } from "react";
import { ipc } from "../lib/ipc";

export interface PromptKeyArgs {
  rowCount: number;
  hi: number;
  tab: string;
  tabs: { key: string }[];
  /** 搜索框里的文本（trim 后）：非空时左右键归光标。 */
  q: string;
  editorOpen: boolean;
  searchRef: RefObject<HTMLInputElement | null>;
  /** 注入当前高亮条目。 */
  inject: () => void;
  /** Esc 收窗前的收尾（右栏编辑的立即落盘）；不传即直接收窗。 */
  flush?: () => void;
  setHi: Dispatch<SetStateAction<number>>;
  setTab: Dispatch<SetStateAction<string>>;
}

export function usePromptKeys(a: PromptKeyArgs): void {
  // 不写依赖数组：每次渲染重挂，闭包里永远是最新的 rows/tab（面板条目会随取用重排）。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        a.flush?.();
        void ipc("prompt_set_visible", { visible: false }).catch(() => {});
        return;
      }
      const el = e.target as HTMLElement | null;
      const inField = el?.tagName === "INPUT" || el?.tagName === "TEXTAREA";
      const stepBy = (d: number) =>
        a.setHi((h) => (a.rowCount ? (h + d + a.rowCount) % a.rowCount : 0));
      const stepTab = (d: number) => {
        const i = a.tabs.findIndex((x) => x.key === a.tab);
        if (i < 0) return;
        a.setTab(a.tabs[(i + d + a.tabs.length) % a.tabs.length].key);
      };
      if (inField) {
        // 搜索框里也要随时吃方向键与回车（用户 2026-11-18）：上下移选择、回车注入是
        // 面板的主操作，不能因为焦点在输入框就被文字光标吃掉。多行编辑器不在内。
        if (el !== a.searchRef.current || e.isComposing) return;
        const input = el as HTMLInputElement;
        const caret = input.selectionStart ?? 0;
        switch (e.key) {
          case "Enter":
            e.preventDefault();
            a.inject();
            return;
          case "ArrowUp":
            e.preventDefault();
            stepBy(-1);
            return;
          case "ArrowDown":
            e.preventDefault();
            stepBy(1);
            return;
          case "ArrowLeft":
            if (!a.q && caret === 0) {
              e.preventDefault();
              stepTab(-1);
            }
            return;
          case "ArrowRight":
            if (!a.q || caret === input.value.length) {
              e.preventDefault();
              stepTab(1);
            }
            return;
        }
        return;
      }
      if (a.editorOpen || e.isComposing) return;
      switch (e.key) {
        case "ArrowLeft":
          if (a.q) return;
          stepTab(-1);
          break;
        case "ArrowRight":
          if (a.q) return;
          stepTab(1);
          break;
        case "ArrowUp":
          stepBy(-1);
          break;
        case "ArrowDown":
          stepBy(1);
          break;
        case "Enter":
          a.inject();
          break;
        default:
          if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
            a.searchRef.current?.focus(); // 本键不吞：聚焦后字符照常落进搜索框
          }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
}
