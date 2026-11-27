// 主题：两套配色（深绿 dark / 暖白 light）+ 一档「跟随系统」（auto），
// 色值全在 styles.css 的 CSS 变量里，这里只管「偏好存哪、解析成哪套、怎么切」。
// index.html / float.html 各有一段同逻辑的内联脚本负责首帧上色——
// 它们把既不是 dark 也不是 light 的存值（含 auto）一律落到系统判定，天然认得 auto；
// 改这里的判定口径时记得两处一起改。
//
// 三态是用户 2026-11-17 点名的：以前只有两态来回切，首次进入虽然跟系统，但存的是
// **解析结果**，之后系统换配色应用不跟着换。现在 auto 存的就是 "auto" 本身，
// 并且监听 prefers-color-scheme 变化实时重绘。

/** 用户偏好（存盘的就是它）。 */
export type ThemePref = "dark" | "light" | "auto";
/** 实际画出来的那套（auto 解析后的结果）。 */
export type Theme = "dark" | "light";

const KEY = "llm-switch.theme";
/** 三态循环顺序：深绿 → 暖白 → 跟随系统 → 深绿。 */
const CYCLE: ThemePref[] = ["dark", "light", "auto"];

function isPref(v: unknown): v is ThemePref {
  return v === "dark" || v === "light" || v === "auto";
}

function systemTheme(): Theme {
  // 系统没说浅色就当深色（本项目默认气质是深色）。
  return typeof matchMedia !== "undefined" && matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

function resolve(pref: ThemePref): Theme {
  return pref === "auto" ? systemTheme() : pref;
}

function preferred(): ThemePref {
  // ?theme=light|dark 是一次性覆盖（调试与截图用），不写回存储。
  const url =
    typeof location !== "undefined"
      ? new URLSearchParams(location.search).get("theme")
      : null;
  if (url === "dark" || url === "light") return url;
  if (typeof localStorage === "undefined") return "auto";
  const saved = localStorage.getItem(KEY);
  if (isPref(saved)) return saved;
  // 没存过就是跟随系统（与旧版首帧行为一致，只是现在存的是 auto 本身）。
  return "auto";
}

function paint(theme: Theme) {
  const el = document.documentElement;
  el.dataset.theme = theme;
  el.style.colorScheme = theme;
}

let pref: ThemePref = preferred();
let current: Theme = resolve(pref);
paint(current);

const listeners = new Set<(t: Theme, p: ThemePref) => void>();

/** 落一次偏好：persist=false 用于系统切换/跨窗同步这类不该回写存盘的场合。 */
function apply(next: ThemePref, persist: boolean) {
  pref = next;
  if (persist) {
    try {
      localStorage.setItem(KEY, next);
    } catch {
      /* 无痕模式下写不进去，本次会话内仍然有效 */
    }
  }
  current = resolve(pref);
  paint(current);
  // 偏好变了就广播，即使解析出的配色没变（auto ↔ dark 在深色系统上配色相同，
  // 但按钮图标与 tooltip 必须跟着档位走）。
  listeners.forEach((f) => f(current, pref));
}

if (typeof window !== "undefined") {
  // 跨窗口同步：主窗 setPref 写 localStorage 只重绘主窗自己的 document，浮窗（速率线 /
  // 资源小窗 / 浮窗）是独立 webview，靠 `storage` 事件（只在其它窗口触发）收到变化后重绘。
  // 没这段，主窗切主题浮窗会停在旧配色（用户报的「浮窗不跟主题」根因）。
  window.addEventListener("storage", (e) => {
    // e.key === null = 别的窗口清了 localStorage；否则只认主题键的变更。
    if (e.key !== null && e.key !== KEY) return;
    const next = isPref(e.newValue) ? e.newValue : preferred();
    if (next === pref) return;
    apply(next, false);
  });
  // 跟随系统：档位是 auto 时，系统换配色立刻跟着重绘（各窗口各自监听，无需同步）。
  if (typeof matchMedia !== "undefined") {
    const mq = matchMedia("(prefers-color-scheme: light)");
    const onSystem = () => {
      if (pref === "auto") apply("auto", false);
    };
    if (mq.addEventListener) mq.addEventListener("change", onSystem);
    else if (mq.addListener) mq.addListener(onSystem);
  }
}

/** 当前**画出来**的那套（auto 已解析）。 */
export function getTheme(): Theme {
  return current;
}

/** 当前**档位**（含 auto）。 */
export function getPref(): ThemePref {
  return pref;
}

export function setPref(next: ThemePref) {
  if (next === pref) return;
  apply(next, true);
}

/** 顶栏三态按钮：深绿 → 暖白 → 跟随系统 → 深绿。 */
export function cyclePref(): ThemePref {
  const i = CYCLE.indexOf(pref);
  apply(CYCLE[(i + 1) % CYCLE.length], true);
  return pref;
}

/**
 * 返回取消订阅函数，直接给 useEffect 用。
 * 回调收 (画出来的配色, 档位)：按钮要按档位画图标，页面要按配色判断。
 */
export function subscribeTheme(fn: (t: Theme, p: ThemePref) => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}
