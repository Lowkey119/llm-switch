// 窗口缩放（Cmd+滚轮 / Cmd+加减号 / Cmd+0 复位），每个窗口独立缩放。
//
// 为什么用 CSS `zoom` 而不是 Tauri 的 setZoom：后者在这个版本的 webview API 里
// 并不总是可用，而 `zoom` 在 WebKit 上是稳定的，且能让布局真正按倍率重排
// （浮窗要靠量回来的尺寸收缩窗口，`transform: scale` 那种不改布局的做法量不准）。
//
// 键盘/滚轮事件只会派发给当前获得焦点的 webview，所以「谁有焦点缩放谁」天然成立；
// 倍率按窗口分开存 localStorage（各自记住自己的倍率），两窗之间不再互相同步。

import { useEffect, useRef, useState } from "react";

const MIN = 0.4;
const MAX = 3.0;
// 倍率存储键前缀（按窗口分开存）。只有本文件认这个串，别处不许再拼一遗。
const KEY_PREFIX = "llm-switch.zoom.";

// 倍率的唯一真相：CSS 自定义属性 --app-zoom（生效处见 styles.css 的 html 规则）。
// 用自定义属性而不是直接写 style.zoom，是为了让「当前倍率」有一个 CSS 侧也能读到的名字，
// 反缩放（浮窗圆钮）和将来的动画都能只认它。
let current = 1;

// 倍率订阅者（如控制台 iframe 的反缩放：面板必须实时跟随主窗缩放重排）。
// 与 useZoom 分开：useZoom 会安装 wheel/keydown 监听，多处调用会把一次按键翻倍。
const listeners = new Set<(z: number) => void>();

/** 订阅倍率变化（不安装任何输入监听）。返回退订函数。 */
export function subscribeZoom(cb: (z: number) => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** 当前倍率；没设 = 1。 */
export function zoomFactor(): number {
  return current;
}

/**
 * 把视口坐标换算回 zoom 树里的本地布局空间。
 *
 * fixed 浮层的 containing block 是视口，可它自己写在被 zoom 的 documentElement 里，
 * 于是 left/top 会被倍率再乘一遍 —— 倍率不为 1 时浮层成倍飘偏。全项目只有这一处换算，
 * 弹层组件不许自己再除一遍（计划 §6.4）。
 */
export function viewportToLocal(x: number, y: number): { x: number; y: number } {
  const z = current || 1;
  return { x: x / z, y: y / z };
}

function clamp(v: number): number {
  return Math.min(MAX, Math.max(MIN, v));
}

function load(key: string): number {
  try {
    const raw = localStorage.getItem(key);
    const v = raw ? parseFloat(raw) : 1;
    return Number.isFinite(v) ? clamp(v) : 1;
  } catch {
    return 1;
  }
}

function apply(key: string, z: number) {
  current = z;
  document.documentElement.style.setProperty("--app-zoom", String(z));
  listeners.forEach((f) => f(z));
  try {
    localStorage.setItem(key, String(z));
  } catch {
    /* 隐私模式下写不进去，缩放照样生效 */
  }
}

/**
 * 首次打开某窗口时继承另一个窗口的倍率，只在目标从没存过倍率时生效一次。
 *
 * 提示词面板用它跟主窗拿同一个字号：主窗 Cmd+滚轮放大过，面板却永远 1.0，
 * 两边看着就是两套字体（用户实报「面板字体太小，要同应用字体」）。继承一次后
 * 面板照旧自己记自己的倍率，不破坏「各窗口独立、互不同步」的口径。
 */
export function seedZoomFrom(scope: string, fromScope: string): void {
  try {
    const to = `${KEY_PREFIX}${scope}`;
    if (localStorage.getItem(to) !== null) return;
    const src = localStorage.getItem(`${KEY_PREFIX}${fromScope}`);
    if (src) localStorage.setItem(to, src);
  } catch {
    /* 隐私模式下读不到就算了，面板按默认倍率走 */
  }
}

/**
 * 装上缩放。`scope` 区分窗口（"main" / "float"，速率线是 `rate-<代理>`），
 * 各自独立记忆倍率、互不同步。`onZoom` 在倍率变化并应用到 DOM 之后回调
 * （浮窗与速率线都靠它把窗口尺寸收回来）。
 */
export function useZoom(scope: string, onZoom?: (z: number) => void): number {
  const key = `${KEY_PREFIX}${scope}`;
  const [zoom, setZoom] = useState<number>(() => load(key));
  const cb = useRef(onZoom);
  cb.current = onZoom;

  useEffect(() => {
    apply(key, zoom);
    const id = requestAnimationFrame(() => cb.current?.(zoom));
    return () => cancelAnimationFrame(id);
    // key 一并入列（lint 要求）：scope 在组件生命周期内不变，key 因此是稳定
    // 字符串，依赖比较永远相等，effect 不会因此多跑。
  }, [key, zoom]);

  useEffect(() => {
    const bump = (f: number) => setZoom((prev) => clamp(prev * f));
    const onWheel = (e: WheelEvent) => {
      if (!e.metaKey) return;
      e.preventDefault();
      bump(e.deltaY > 0 ? 0.9 : 1.1);
    };
    const onKey = (e: KeyboardEvent) => {
      // ⌥⌘± 是 macOS 辅助功能「缩放视图」的系统快捷键（功能键区映射用的就是它），
      // 本窗不许抢——否则 app 自己成倍放大、系统缩放被吃掉（用户 2026-11-21）。
      if (!e.metaKey || e.altKey) return;
      if (e.key === "+" || e.key === "=") {
        e.preventDefault();
        bump(1.1);
      } else if (e.key === "-" || e.key === "_") {
        e.preventDefault();
        bump(0.9);
      } else if (e.key === "0") {
        e.preventDefault();
        setZoom(1);
      }
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("wheel", onWheel);
      window.removeEventListener("keydown", onKey);
    };
  }, [key]);

  return zoom;
}
