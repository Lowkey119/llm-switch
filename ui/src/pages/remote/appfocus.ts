// App 聚焦策略（C6，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）：
// 「打开应用」只是把窗口拉起来，多数输入类 App 起来后光标不在输入框里，
// 于是每个 App 可以配一条「打开后等多久、用什么方式聚焦」。
// 形状与 Rust `appfocus::AppFocusProfile` 对齐（serde tagged union，`type` 判别）。
import type { Shortcut } from "./shortcut";

export type FocusStrategy =
  | { type: "none" }
  | { type: "ax" }
  | { type: "shortcut"; shortcut: Shortcut };

export interface AppFocusProfile {
  bundle_id: string;
  strategy: FocusStrategy;
  delay_ms: number;
}

/** 延迟上限（与后端 `appfocus::MAX_DELAY_MS` 同源）：超了按上限截。 */
export const MAX_DELAY_MS = 5000;

/** 没配过策略时的默认延迟：够 App 把窗口拉起来，又不至于让用户觉得卡。 */
export const DEFAULT_DELAY_MS = 600;

export const strategyOf = (p: AppFocusProfile | null | undefined): FocusStrategy =>
  p?.strategy ?? { type: "none" };

/** 分段控件清单（图标单一族 icons.tsx）。 */
export const STRATEGY_TABS: {
  id: FocusStrategy["type"];
  labelKey: string;
  tipKey: string;
}[] = [
  { id: "none", labelKey: "remote.focus.none", tipKey: "remote.focus.noneTip" },
  { id: "ax", labelKey: "remote.focus.ax", tipKey: "remote.focus.axTip" },
  {
    id: "shortcut",
    labelKey: "remote.focus.shortcut",
    tipKey: "remote.focus.shortcutTip",
  },
];
