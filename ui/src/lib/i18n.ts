// UI-17 前端 i18n（用户 2026-11-16「本就是 i18n 支持，却少了切换的功能」）。
// 范围拍板「外壳先切」：顶栏、左栏、页名、通用按钮/tooltip 先进词典，
// 页面正文随后续切片接入。语言偏好与托盘共用同一份 `ui.lang`（ui_lang_get/set），
// App 拉到偏好后 setLang 进来，订阅方即时重渲染。
//
// 词典形状：key → [中文, English]。缺 key 时原样返回 key——漏译一眼可见，
// 不会静默掉词。
import { useSyncExternalStore } from "react";

export type Lang = "zh" | "en";

const DICT: Record<string, [string, string]> = {
  // 页名（左栏、顶栏、命令面板、占位页共用）
  "page.backends": ["后端", "Backends"],
  "page.monitor": ["实时监控", "Monitor"],
  "page.logs": ["日志", "Logs"],
  "page.usage": ["用量", "Usage"],
  "page.console": ["后端控制台", "Consoles"],
  "page.agents": ["代理", "Agents"],
  "page.skills": ["Skills 管理", "Skills"],
  "page.settings": ["设置", "Settings"],
  // 顶栏按钮 tooltip / aria
  "top.sysFloatOn": ["隐藏资源悬浮小窗", "Hide resource widget"],
  "top.sysFloatOff": ["显示资源悬浮小窗", "Show resource widget"],
  "top.floatOn": ["隐藏多行浮窗", "Hide float panel"],
  "top.floatOff": ["显示多行浮窗", "Show float panel"],
  "top.themeToWarm": ["切换到暖白主题", "Switch to warm-light theme"],
  "top.themeToDark": ["切换到深绿主题", "Switch to dark theme"],
  "top.langTip": ["切换到英文", "Switch to Chinese"],
  "top.langAria": ["切换语言", "Switch language"],
  // 版本徽章
  "chip.update": ["有更新", "Update"],
  "chip.versionTip": ["当前版本", "Current version"],
  // 命令面板
  "palette.theme": ["切换主题", "Toggle theme"],
  "palette.themeHint": ["深绿 / 暖白", "Dark / Warm"],
  "palette.tour": ["重看引导", "Replay tour"],
  "palette.tourHint": ["重放首次引导", "Replay first-run tour"],
  // 占位页
  "placeholder.body": [
    "该页面的功能将在对应里程碑接入，当前为骨架占位。",
    "This page lands in a later milestone; it is a skeleton placeholder.",
  ],
  // 代理页 Tab 与常用控件
  "agents.tab.agents": ["代理管理", "Agents"],
  "agents.tab.models": ["模型分配全景", "Model Allocation"],
  "agents.tab.install": ["安装引导", "Install Guide"],
  "common.showAllAgents": ["显示全部代理", "Show all agents"],
  "common.rescan": ["重新扫描", "Rescan"],
  "common.scanning": ["扫描中…", "Scanning…"],
};

let lang: Lang = "zh";
const subs = new Set<() => void>();

export function getLang(): Lang {
  return lang;
}

/** App 拉到 `ui.lang` 偏好后写入；变化即广播订阅方重渲染。 */
export function setLang(l: Lang) {
  if (l === lang) return;
  lang = l;
  subs.forEach((f) => f());
}

export function t(key: string): string {
  const e = DICT[key];
  if (!e) return key;
  return lang === "zh" ? e[0] : e[1];
}

function subscribe(cb: () => void) {
  subs.add(cb);
  return () => {
    subs.delete(cb);
  };
}

/** 组件侧入口：语言变化即重渲染（与主题 lib/theme.ts 同型）。 */
export function useI18n() {
  const cur = useSyncExternalStore(subscribe, getLang);
  return { t, lang: cur };
}
