// 提示词面板的全局键盘口径（自 PromptPanel 拆出）。
//
// 遥控器五键口径（用户 2026-11-24 重定，只用 ↑↓←→OK 完成增删改查注入）：
//   浏览态（焦点在搜索框或空手）：↑↓ 选条目、←→ 换 tab、打字跳搜索框；
//     OK 短按 = 注入（无注入目标由面板层闪烁提醒）；OK 长按 = 编辑选中条目。
//   编辑态（焦点在编辑框）：方向键归文字光标；OK 短按 = 保存退出；OK 长按 = 待删确认。
//   Esc：编辑态退编辑回搜索（先落盘），否则收窗。
// 长按判定：keydown 起 600ms 计时，到点算长按；keyup 前没到点算短按。
// 遥控器 OK 注入成 Enter 键，keydown/keyup 都会到达，repeat（按住连发）忽略。
import { useEffect, useRef, type Dispatch, type RefObject, type SetStateAction } from "react";
import { ipc } from "../lib/ipc";

export interface PromptKeyArgs {
  rowCount: number;
  hi: number;
  tab: string;
  tabs: { key: string }[];
  /** 搜索框里的文本（trim 后）：非空时左右键仍换 tab（用户 2026-11-24 拍板）。 */
  q: string;
  editorOpen: boolean;
  searchRef: RefObject<HTMLTextAreaElement | null>;
  /** OK 短按（浏览态=注入；编辑态=保存）。 */
  okShort: () => void;
  /** OK 长按（浏览态=编辑选中条目；编辑态=待删确认）。 */
  okLong: () => void;
  /** Esc 收窗前的收尾（右栏编辑的立即落盘）；不传即直接收窗。 */
  flush?: () => void;
  /** 编辑态的 Esc：退编辑回搜索（先落盘）；不传则 Esc 一律收窗。 */
  cancelEdit?: () => void;
  setHi: Dispatch<SetStateAction<number>>;
  setTab: Dispatch<SetStateAction<string>>;
}

/** OK 长按阈值（毫秒）：超过算长按，与仓内其他长按口径一致。 */
const LONG_MS = 600;

export function usePromptKeys(a: PromptKeyArgs): void {
  const hold = useRef<{ timer: number; long: boolean } | null>(null);
  // 不写依赖数组：每次渲染重挂，闭包里永远是最新的 rows/tab（面板条目会随取用重排）。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const inSearch = el === a.searchRef.current;
      const inField = el?.tagName === "INPUT" || el?.tagName === "TEXTAREA";
      if (e.key === "Escape") {
        if (a.editorOpen && a.cancelEdit) {
          a.cancelEdit();
          return;
        }
        a.flush?.();
        void ipc("prompt_set_visible", { visible: false }).catch(() => {});
        return;
      }
      const stepBy = (d: number) =>
        a.setHi((h) => (a.rowCount ? (h + d + a.rowCount) % a.rowCount : 0));
      const stepTab = (d: number) => {
        const i = a.tabs.findIndex((x) => x.key === a.tab);
        if (i < 0) return;
        a.setTab(a.tabs[(i + d + a.tabs.length) % a.tabs.length].key);
      };
      // OK（Enter）短按/长按：keydown 起计时，keyup 前没到点 = 短按。
      // 编辑框里的方向键归光标，但 OK 仍归面板（保存/待删）。
      if (e.key === "Enter" && (!inField || inSearch || a.editorOpen)) {
        if (e.repeat) return;
        if (e.type === "keydown") {
          e.preventDefault();
          hold.current = {
            timer: window.setTimeout(() => {
              if (hold.current) {
                hold.current.long = true;
                a.okLong();
              }
            }, LONG_MS),
            long: false,
          };
          return;
        }
        const h = hold.current;
        hold.current = null;
        if (!h) return;
        window.clearTimeout(h.timer);
        if (!h.long) {
          e.preventDefault();
          a.okShort();
        }
        return;
      }
      // keyup 只用来结算 OK 长短按；方向键等一律只认 keydown，
      // 否则一次按键上下各触发一次 = 跳两格（用户报的历史 bug，真凶就是 keyup 双触发）。
      if (e.type !== "keydown") return;
      if (inField) {
        // 搜索框里吃方向键（用户 2026-11-18/24）：↑↓ 移选择、←→ 换 tab 是主操作，
        // 不能被文字光标吃掉。编辑框不在内——编辑中方向键归光标。
        if (!inSearch || e.isComposing) return;
        switch (e.key) {
          case "ArrowUp":
            e.preventDefault();
            stepBy(-1);
            return;
          case "ArrowDown":
            e.preventDefault();
            stepBy(1);
            return;
          case "ArrowLeft":
            e.preventDefault();
            stepTab(-1);
            return;
          case "ArrowRight":
            e.preventDefault();
            stepTab(1);
            return;
        }
        return;
      }
      if (a.editorOpen || e.isComposing) return;
      switch (e.key) {
        case "ArrowLeft":
          stepTab(-1);
          break;
        case "ArrowRight":
          stepTab(1);
          break;
        case "ArrowUp":
          stepBy(-1);
          break;
        case "ArrowDown":
          stepBy(1);
          break;
        default:
          if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
            searchFocus(a); // 本键不吞：聚焦后字符照常落进搜索框
          }
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
    };
  });
}

/** 打字跳搜索框：聚焦并把光标挪到末尾（textarea 重挂后 selection 会丢，显式补一次）。 */
function searchFocus(a: PromptKeyArgs) {
  const ta = a.searchRef.current;
  if (!ta) return;
  ta.focus();
  ta.setSelectionRange(ta.value.length, ta.value.length);
}
