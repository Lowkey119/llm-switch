// 一条到点自己消失的局部提示（浮窗底栏、原位校验行这类「就地冒一下」的场合）。
//
// 为什么要有：全局 toast（`components/Toast`）管跨页的大声回报，但浮窗与胶囊那种
// 贴在控件旁边、2~3s 自清的小提示，各组件此前各写一份 `useState + setTimeout`，
// 定时器漏清、互相覆盖全凭手写（ui-budget「toast 实现点」预算 1 就是拦这个）。
// 这里收成一份：换文案自动掐掉上一条的定时器，卸载即清，调用方只管 show。
import { useEffect, useRef, useState } from "react";

export function useTimedNote(defaultMs = 3000) {
  const [note, setNote] = useState("");
  const timer = useRef(0);
  const clear = () => {
    window.clearTimeout(timer.current);
    setNote("");
  };
  const show = (msg: string, ms = defaultMs) => {
    window.clearTimeout(timer.current);
    setNote(msg);
    timer.current = window.setTimeout(clear, ms);
  };
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return { note, show, clear };
}
