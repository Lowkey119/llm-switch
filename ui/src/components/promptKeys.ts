// 提示词面板的全局键盘口径（自 PromptPanel 拆出）。
//
// 遥控器键位口径（quickpanel 原型回灌：语音 · ↑↓←→ · OK · Esc）：
//   ↑↓ 选条目、←→ 换 tab；
//   OK 短按：没装入 → 把当前行装入右栏；已装入 → 再按一次 OK 就是确认，真注入目标应用；
//   OK 长按（已装入待注入正文）= 一键追加进选定的 tab（唯一写回入口）。
//   Esc 逐级后退：清待执行 → 收窗。
//   ⌥Enter = 发送键（遥控器可把专用键映射到它）：任何时候直接执行右栏已装入内容。
// **焦点恒在条目上**（用户 2026-11-26 拍板）：搜索框已拆、右栏改只读，面板里没有任何可聚焦
// 的输入框，所以不再分「焦点区」，Tab 也不再接管（还给浏览器默认行为）。
// 长按判定：keydown 起 600ms 计时，到点算长按；keyup 前没到点算短按。
// 遥控器 OK 注入成 Enter 键，keydown/keyup 都会到达，repeat（按住连发）忽略。
// ←→ 换 tab 同样只认第一下（repeat 不当一步走）：tab 轨环形取模，两档时连发就是「切了又弹回来」
//（用户 2026-11-26 实报）；↑↓ 保留连发，列表按住连着走才是正常手感。
import { useEffect, useRef, type Dispatch, type SetStateAction } from "react";
import { ipc } from "../lib/ipc";

export interface PromptKeyArgs {
  rowCount: number;
  hi: number;
  tab: string;
  tabs: { key: string }[];
  editorOpen: boolean;
  /** 右栏 stage 态（有装入内容）：OK 归面板（确认注入），Esc 先清它。 */
  stageOpen: boolean;
  /**
   * 语音编辑态（计划 §语音编辑态）：焦点在回显 textarea 上，方向键/退格/⌃K/⌃U
   * 全部交 textarea 自己（移光标、删字符），OK / Esc 也由它自己的 onKeyDown 接手，
   * 所以全局监听在这期间一律不接管。
   */
  voiceEdit: boolean;
  /** OK 短按（没装入=装入；已装入=确认注入）。 */
  okShort: () => void;
  /** OK 长按（已装入待注入正文）= 追加进选定 tab。 */
  okLong: () => void;
  /** Esc 收窗前的收尾（当前无编辑态可落盘，留作扩展位）。 */
  flush?: () => void;
  /** Esc 第二级：清空右栏待执行内容。 */
  clearStage: () => void;
  /** ⌥Enter 发送键：执行右栏已装入内容（面板层判断有无 stage）。 */
  send: () => void;
  setHi: Dispatch<SetStateAction<number>>;
  setTab: Dispatch<SetStateAction<string>>;
}

/** OK 长按阈值（毫秒）：超过算长按，与仓内其他长按口径一致。 */
const LONG_MS = 600;

/**
 * 换 tab 双发守卫窗口（毫秒）：同一个方向键在这么短时间里到的第二下当重复丢掉。
 * 为什么 `e.repeat` 不够：遥控器（HID）那一发不是系统自动重复，是**两次独立 keydown**
 * （用户 2026-11-26 实报：真遥控器按 ← 切过去又弹回来，而真键盘/合成键同一步不会）。
 * 两档 tab 轨是环形取模，双发正好弹回原位，看着像「自己乱跳」。
 * 120ms 在人手有意连点之下（>150ms），只吃得住设备双发。
 */
const TAB_DOUBLE_MS = 120;

export function usePromptKeys(a: PromptKeyArgs): void {
  const hold = useRef<{ timer: number; long: boolean } | null>(null);
  // 上一次换 tab 的时刻（双发守卫用，见 TAB_DOUBLE_MS）。
  const lastTabAt = useRef(0);
  // 最新参数挂 ref，监听器只在挂载时装一次。原先是无依赖数组的 effect（每次渲染
  // remove + add 一遍），面板渲染频繁（过滤、取用重排），频繁拆装就是
  // 「按了没反应 / 一下跳两格」的前端那一半（用户 2026-11-25）。
  const box = useRef(a);
  box.current = a;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const a = box.current;
      // 语音编辑态：全局监听整条让位给 textarea（否则方向键会被当成换条目，光标动不了）。
      if (a.voiceEdit) return;
      if (e.key === "Escape") {
        if (a.stageOpen) {
          a.clearStage();
          return;
        }
        a.flush?.();
        void ipc("prompt_set_visible", { visible: false }).catch(() => {});
        return;
      }
      // ⌥Enter 发送键：无视焦点区，直接执行右栏已装入内容（遥控器专用键的落点）。
      if (e.key === "Enter" && e.altKey) {
        if (e.type === "keydown" && !e.repeat) {
          e.preventDefault();
          a.send();
        }
        return;
      }
      // 到顶/到底夹住不动（用户 2026-11-25）：环形取模在长列表里就是「一下跳回开头」，
      // 看着像乱跳好几格。tab 切换仍保留环形（那里环形是便利不是歧义）。
      const stepBy = (d: number) =>
        a.setHi((h) => Math.min(Math.max(h + d, 0), Math.max(a.rowCount - 1, 0)));
      const stepTab = (d: number) => {
        const now = Date.now();
        const gap = now - lastTabAt.current;
        // 连发/双发都刷新窗口但不走一步：lastAt 必须跟随**每一发** keydown，
        // 不能只在放行时更新——否则连发（间隔 100ms）每隔一发距上次放行就 >120ms，
        // 放行一次，3 个 tab 环形取模下就是「在各 tab 间乱跳」（用户 2026-11-26 实报）。
        // 松手静默 >120ms 后再按才放行；人手有意连按 >150ms 不受影响。
        lastTabAt.current = now;
        if (gap < TAB_DOUBLE_MS) return;
        const i = a.tabs.findIndex((x) => x.key === a.tab);
        if (i < 0) return;
        a.setTab(a.tabs[(i + d + a.tabs.length) % a.tabs.length].key);
      };
      // OK（Enter）短按/长按：keydown 起计时，keyup 前没到点 = 短按。
      // 面板里没有可聚焦的输入框了，OK 一律归面板（装入 / 确认注入）。
      if (e.key === "Enter") {
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
      if (a.editorOpen || e.isComposing) return;
      switch (e.key) {
        case "ArrowLeft":
          // 换 tab 是「一步到位」的离散动作，连发（repeat）必须只认第一下：tab 轨是环形取模，
          // 两档时连发两下就是「切过去又弹回来」（用户 2026-11-26 实报：从 2 号按 ← 立刻跳回 2 号）。
          // ↑↓ 不挡——列表按住连着走是正常手感（同一份 repeat 事件在列表里就是走到底）。
          e.preventDefault();
          if (!e.repeat) stepTab(-1);
          break;
        case "ArrowRight":
          e.preventDefault();
          if (!e.repeat) stepTab(1);
          break;
        case "ArrowUp":
          // 浏览态四个方向键一律吞掉默认行为（用户 2026-11-25）：焦点落在列表行/按钮上时，
          // 不吞就会同时驱动页面滚动，出现「按了但选中的不是它」。
          e.preventDefault();
          stepBy(-1);
          break;
        case "ArrowDown":
          e.preventDefault();
          stepBy(1);
          break;
        default:
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
    };
  }, []);
}
