// 虚拟键盘状态机（共享）：动作面板与快速面板共用同一套，避免两处各写一份产生行为差异。
// 键盘只管「按键状态 + 组合逻辑」，录入结果通过 onCommit 交给调用方决定落到哪里
// （动作面板落成宏的一步、快速面板落成面板条目）——这正是 VirtualKeyboard 组件的受控契约。
import { useEffect, useRef, useState } from "react";
import type { Action } from "./action";

export function useVirtualKeyboard(onCommit: (action: Action, at: number | null) => void) {
  // at=null=追加新条目/步骤；有值=原位改写那一条。与动作面板旧 kbd+kbdStep 同语义，合并成一个。
  const [kbd, setKbd] = useState<{ at: number | null } | null>(null);
  const [mods, setMods] = useState(0);
  const [main, setMain] = useState<number | null>(null);
  // 回调挂 ref：commit 闭包每次渲染重建，用最新回调避免读到旧 state。
  const commitRef = useRef(onCommit);
  commitRef.current = onCommit;

  const open = (at: number | null = null) => setKbd({ at });
  const close = () => {
    setKbd(null);
    setMods(0);
    setMain(null);
  };
  // bit16 = fn 修饰键（键盘 UI 内部位），落 Shortcut 时转成独立的 fn:true 字段。
  const FN_BIT = 16;
  // 点主键 = 组合完成：把 action 交给调用方落地，然后清状态、收键盘。
  const commit = (kvk: number) => {
    commitRef.current(
      { type: "shortcut", shortcut: { mods: mods & ~FN_BIT, key: kvk, ...((mods & FN_BIT) ? { fn: true } : {}) } },
      kbd?.at ?? null,
    );
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

  return { kbd, mods, main, setMods, setMain, open, close, commit };
}
