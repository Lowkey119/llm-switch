// 主题：两套配色（深绿 dark / 暖白 light），色值全在 styles.css 的 CSS 变量里，
// 这里只管「选哪个、存哪、怎么切」。index.html 里有一段同逻辑的内联脚本负责首帧上色。
export type Theme = "dark" | "light";

const KEY = "llm-switch.theme";

function preferred(): Theme {
  // ?theme=light|dark 是一次性覆盖（调试与截图用），不写回存储。
  const url =
    typeof location !== "undefined"
      ? new URLSearchParams(location.search).get("theme")
      : null;
  if (url === "dark" || url === "light") return url;
  const saved = typeof localStorage !== "undefined" ? localStorage.getItem(KEY) : null;
  if (saved === "dark" || saved === "light") return saved;
  // 首次进入跟随系统；系统没说浅色就当深色（本项目默认气质是深色）。
  return typeof matchMedia !== "undefined" && matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}

function paint(theme: Theme) {
  const el = document.documentElement;
  el.dataset.theme = theme;
  el.style.colorScheme = theme;
}

let current: Theme = preferred();
paint(current);

const listeners = new Set<(t: Theme) => void>();

// 跨窗口同步：主窗 setTheme 写 localStorage 只重绘主窗自己的 document，浮窗（速率线 /
// 资源小窗 / 浮窗）是独立 webview，靠 `storage` 事件（只在其它窗口触发）收到变化后重绘。
// 没这段，主窗切主题浮窗会停在旧配色（用户报的「浮窗不跟主题」根因）。
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    // e.key === null = 别的窗口清了 localStorage；否则只认主题键的变更。
    if (e.key !== null && e.key !== KEY) return;
    const next =
      e.newValue === "dark" || e.newValue === "light" ? e.newValue : preferred();
    if (next === current) return;
    current = next;
    paint(current);
    listeners.forEach((f) => f(current));
  });
}

export function getTheme(): Theme {
  return current;
}

export function setTheme(theme: Theme) {
  if (theme === current) return;
  current = theme;
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    /* 无痕模式下写不进去，本次会话内仍然有效 */
  }
  paint(theme);
  listeners.forEach((f) => f(theme));
}

export function toggleTheme(): Theme {
  setTheme(current === "dark" ? "light" : "dark");
  return current;
}

/** 返回取消订阅函数，直接给 useEffect 用。 */
export function subscribeTheme(fn: (t: Theme) => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}
