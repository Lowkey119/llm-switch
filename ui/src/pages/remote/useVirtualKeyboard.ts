// 虚拟键盘状态机（共享）：动作面板与快速面板共用同一套，避免两处各写一份产生行为差异。
// 键盘只管「按键状态 + 组合逻辑」，录入结果通过 onCommit 交给调用方决定落到哪里
// （动作面板落成宏的一步、快速面板落成面板条目）——这正是 VirtualKeyboard 组件的受控契约。
// 写条目统一显式落盘（用户 2026-11-27 拍板）：点键只摆组合，按落盘入口（大手图标）才写条目
// 并收键盘；没落盘前已摆的键随时可改。修饰键点击循环：未选 → 轻点（点亮，供组合）
// → 按住（琥珀，裸修饰键「按住」动作）→ 未选；进入按住态清掉已摆主键（按住动作是裸修饰键）。
import { useEffect, useRef, useState } from "react";
import type { Action } from "./action";

/** mods 位 → 裸修饰键 kVK（左侧，与 Rust actions::MOD_KEYCODES 同源）；bit16 = fn（键盘 UI 内部位）。 */
const MOD_BIT_TO_KVK: Record<number, number> = { 1: 59, 2: 56, 4: 58, 8: 55, 16: 63 };

export function useVirtualKeyboard(onCommit: (action: Action, at: number | null) => void) {
  // at=null=追加新条目/步骤；有值=原位改写那一条。与动作面板旧 kbd+kbdStep 同语义，合并成一个。
  const [kbd, setKbd] = useState<{ at: number | null } | null>(null);
  const [mods, setMods] = useState(0);
  const [main, setMain] = useState<number | null>(null);
  // 处于「按住」态的修饰键位（0=无）：落盘时优先落成裸修饰键按住动作。
  const [holdMod, setHoldMod] = useState(0);
  // 回调挂 ref：commit 闭包每次渲染重建，用最新回调避免读到旧 state。
  const commitRef = useRef(onCommit);
  commitRef.current = onCommit;

  const open = (at: number | null = null) => setKbd({ at });
  const close = () => {
    setKbd(null);
    setMods(0);
    setMain(null);
    setHoldMod(0);
  };
  // bit16 = fn 修饰键（键盘 UI 内部位），落 Shortcut 时转成独立的 fn:true 字段。
  const FN_BIT = 16;
  /** 修饰键点击循环：未选 → 轻点 → 按住 → 未选（无计时判定，每点确定推进一档）。 */
  const modClick = (bit: number) => {
    if (holdMod === bit) {
      setHoldMod(0);
      return;
    }
    if (mods & bit) {
      setMods(mods & ~bit);
      setHoldMod(bit);
      setMain(null);
      return;
    }
    setMods(mods | bit);
  };
  /** 点主键 = 摆上选择（不自动写条目）；按住态下改摆组合，按住态让位。 */
  const stageMain = (kvk: number) => {
    setHoldMod(0);
    setMain(kvk);
  };
  /** 落盘写条目：按住态优先（裸修饰键动作），否则写摆好的组合；写完清状态、收键盘。 */
  const commit = () => {
    if (holdMod) {
      commitRef.current(
        { type: "shortcut", shortcut: { mods: 0, key: MOD_BIT_TO_KVK[holdMod], hold: true } },
        kbd?.at ?? null,
      );
    } else if (main !== null) {
      commitRef.current(
        { type: "shortcut", shortcut: { mods: mods & ~FN_BIT, key: main, ...((mods & FN_BIT) ? { fn: true } : {}) } },
        kbd?.at ?? null,
      );
    } else {
      return;
    }
    close();
  };

  // Esc 收口：捕获阶段先于页面级 Esc，别把收窗也一起触发（两处面板原各自实现的同一逻辑）。
  useEffect(() => {
    if (!kbd) return;
    const h = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      close();
    };
    window.addEventListener("keydown", h, true);
    return () => window.removeEventListener("keydown", h, true);
  }, [kbd]);

  return { kbd, mods, main, holdMod, open, close, modClick, stageMain, commit };
}
