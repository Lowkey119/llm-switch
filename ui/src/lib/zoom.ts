// 窗口缩放（Cmd+滚轮 / Cmd+加减号 / Cmd+0 复位），每个窗口独立缩放。
//
// 为什么用 CSS `zoom` 而不是 Tauri 的 setZoom：后者在这个版本的 webview API 里
// 并不总是可用，而 `zoom` 在 WebKit 上是稳定的，且能让布局真正按倍率重排
// （浮窗要靠量回来的尺寸收缩窗口，`transform: scale` 那种不改布局的做法量不准）。
//
// 键盘/滚轮事件只会派发给当前获得焦点的 webview，所以「谁有焦点缩放谁」天然成立；
// 倍率按窗口分开存 localStorage（各自记住自己的倍率），两窗之间不再互相同步。

import { useEffect, useRef, useState } from "react";

// 离散缩放档位（用户 2026-11-24 拍板：全应用统一，只许这几档，杜绝连续缩放的极端倍率）。
// 区间 = 遥控器页实测健康档：0.8 以下键表/动作库带开始局促，1.25 以上编辑器表单
// 开始吃竖向滚动。拖窗框改宽高与档位无关——布局按窗口连续重排（min-w-0 栅格），
// 档位只约束 Cmd 缩放倍率。
const MAIN_STEPS = [0.8, 0.9, 1.0, 1.1, 1.25];
// 浮动窗口（浮窗 / 系统浮窗 / 速率线 / 提示词面板）另开高档：这些窗本体就是小条，
// 1.25 封顶字还是太小看不清（用户 2026-11-24）。窗口随内容 refit，放大只是把窗撑大，
// 不会挤坏内部布局。2.5 封顶用户仍嫌小（2026-11-25 再拍板）放到 4.0；
// 同日三拍：资源监控与状态浮窗还要再放开一倍 ⇒ 上限 8.0（窗口超屏由 Rust 侧贴边/工作区钳制兜底）。
const FLOAT_STEPS = [0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0, 8.0];

/**
 * 提示词速取面板（快速面板）专属三档（计划 2026-11-26 §三.6）：这扇窗是用遥控器在
 * 远距离读的，档位多了反而不好选，拍板只留 1 / 1.5 / 2，**默认 2**（远距离大字是既定口径）。
 * 音量 ± 键就在三档里循环（面板态映射表）。
 */
const PROMPT_STEPS = [1, 1.5, 2];

/** 按窗口取档位表：主窗一套，快速面板一套，其余（float / sysfloat / rate-*）共用浮动窗一套。 */
function stepsFor(scope: string): number[] {
  if (scope === "main") return MAIN_STEPS;
  if (scope === "prompt") return PROMPT_STEPS;
  return FLOAT_STEPS;
}

/** 没存过倍率时的默认档：快速面板 2（计划 §三.6），其余 1。 */
function defaultFor(scope: string): number {
  return scope === "prompt" ? 2 : 1;
}

/** 吸附到最近档位（旧 localStorage 存的是连续倍率，读入即归一）。 */
function snap(steps: number[], v: number): number {
  let best = steps[0];
  for (const s of steps) if (Math.abs(s - v) < Math.abs(best - v)) best = s;
  return best;
}
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

/** 循环到下一档（顶栏缩放钮入口）：到头回卷，立即生效并广播，回新倍率。 */
export function cycleZoom(scope: string): number {
  const steps = stepsFor(scope);
  const key = `${KEY_PREFIX}${scope}`;
  const cur = load(key, steps, defaultFor(scope));
  const next = steps[(steps.indexOf(cur) + 1) % steps.length];
  apply(key, next);
  return next;
}

/**
 * 按方向走一档（面板态音量 ± 键入口，计划 §三.2）：到头回卷（1 → 1.5 → 2 → 1）。
 * 与 `cycleZoom` 同一套档位表与落库口径，只是多收一个方向。
 */
export function stepZoom(scope: string, dir: number): number {
  const steps = stepsFor(scope);
  const key = `${KEY_PREFIX}${scope}`;
  const cur = load(key, steps, defaultFor(scope));
  const i = steps.indexOf(cur);
  const n = steps.length;
  const base = i < 0 ? steps.indexOf(defaultFor(scope)) : i;
  // +1 下一档、-1 上一档；取模处理两头回卷（+n 先抬正，避开 JS 负数取模）。
  const next = steps[((base + (dir >= 0 ? 1 : -1)) % n + n) % n];
  apply(key, next);
  return next;
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

function clamp(steps: number[], v: number): number {
  return snap(steps, v);
}

function load(key: string, steps: number[], fallback: number): number {
  try {
    const raw = localStorage.getItem(key);
    const v = raw ? parseFloat(raw) : fallback;
    return Number.isFinite(v) ? clamp(steps, v) : fallback;
  } catch {
    return fallback;
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
 * 装上缩放。`scope` 区分窗口（"main" / "float"，速率线是 `rate-<代理>`），
 * 各自独立记忆倍率、互不同步。`onZoom` 在倍率变化并应用到 DOM 之后回调
 * （浮窗与速率线都靠它把窗口尺寸收回来）。
 */
export function useZoom(scope: string, onZoom?: (z: number) => void): number {
  const key = `${KEY_PREFIX}${scope}`;
  const steps = stepsFor(scope);
  const [zoom, setZoom] = useState<number>(() => load(key, steps, defaultFor(scope)));
  const cb = useRef(onZoom);
  cb.current = onZoom;

  useEffect(() => {
    apply(key, zoom);
    const id = requestAnimationFrame(() => cb.current?.(zoom));
    return () => cancelAnimationFrame(id);
    // key 一并入列（lint 要求）：scope 在组件生命周期内不变，key 因此是稳定
    // 字符串，依赖比较永远相等，effect 不会因此多跑。
  }, [key, zoom]);

  // 外部入口（顶栏循环钮）走 apply 直写倍率，本窗 state 必须跟上——
  // 不跟的话下一次 Cmd+滚轮从陈旧值起算，会把档位弹回旧轨。
  useEffect(() => subscribeZoom((z) => setZoom((prev) => (prev === z ? prev : z))), []);

  // 跨窗同步：同一个 scope 被多个窗口共用时（浮窗与各家速率线同用 "float"），
  // 任一窗口 Cmd 缩放写 localStorage，其余窗口靠 storage 事件跟随重排。
  // 只认自己这把 key；主窗（"main"）没有第二个共用者，天然不受影响。
  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key !== key || e.newValue == null) return;
      const v = parseFloat(e.newValue);
      if (Number.isFinite(v)) setZoom(clamp(steps, v));
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, [key, steps]);

  useEffect(() => {
    // 缩放走「档位表逐档走」，不再乘 1.1 后就近吸附（用户 2026-11-25 拓）：
    // 乘算 + 就近吸附在跳档大的地方会**卡住**——2.0×1.1=2.2 仍吸附回 2.0、
    // 4.0×1.1=4.4 仍吸附回 4.0，封顶抬多高都摸不到。改成按档位表下标 ±1，
    // 每一档都踩得到，Cmd 滚轮 / Cmd± 与顶栏循环钮同一口径。
    const bump = (f: number) =>
      setZoom((prev) => {
        const i = steps.indexOf(snap(steps, prev));
        const j = f > 1 ? Math.min(i + 1, steps.length - 1) : Math.max(i - 1, 0);
        return steps[j];
      });
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
    // steps 由 scope 派生（scope 生命周期内不变），入列只为 lint 口径。
  }, [key, steps]);

  return zoom;
}
