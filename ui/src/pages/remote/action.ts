// 槽位动作（C2，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）：
// 三槽每槽一个动作，「快捷键」只是其中一种。形状与 Rust `actions::Action` 对齐
// （serde tagged union，`type` 判别，snake_case）。
//
// 旧配置（槽值是裸 Shortcut）由后端读入时迁移成 `{type:"shortcut"}`，
// 前端拿到的永远是新形状，不必在这里再兜旧形状。
//
// C3 起多一种「宏」：槽里只存宏 id（步骤住宏库 `macros.ts`），改一次所有绑点同时生效。
import { t } from "../../lib/i18n";
import { fmtShortcut, type Shortcut } from "./shortcut";

export type Action =
  | { type: "shortcut"; shortcut: Shortcut }
  | { type: "open_app"; bundle_id: string; args?: string }
  // 代理 CLI（用户 2026-11-23 要求）：在终端里起一个装好的代理 CLI，可带参数与工作目录。
  | { type: "agent_cli"; app: string; args?: string; cwd?: string }
  | { type: "text"; content: string }
  | { type: "focus_input" }
  | { type: "run_command"; command: string }
  | { type: "macro"; id: string };

export type ActionType = Action["type"];

/** 动作类型清单（分段控件的顺序 = 日常常用度）；图标单一族 icons.tsx。 */
export const ACTION_TYPES: { id: ActionType; labelKey: string }[] = [
  { id: "shortcut", labelKey: "remote.act.shortcut" },
  { id: "open_app", labelKey: "remote.act.openApp" },
  { id: "agent_cli", labelKey: "remote.act.agentCli" },
  { id: "text", labelKey: "remote.act.text" },
  { id: "focus_input", labelKey: "remote.act.focusInput" },
  { id: "run_command", labelKey: "remote.act.runCommand" },
  { id: "macro", labelKey: "remote.act.macro" },
];

export const asShortcut = (sc: Shortcut): Action => ({ type: "shortcut", shortcut: sc });

/** 取动作里的快捷键（非快捷键动作回 null）——冲突判定只对快捷键。 */
export const shortcutOf = (a: Action | null | undefined): Shortcut | null =>
  a && a.type === "shortcut" ? a.shortcut : null;

/** bundle id 尾段（com.apple.Terminal → Terminal）：槽框窄，全 id 塞不下。 */
const appTail = (bundleId: string) => bundleId.trim().split(".").filter(Boolean).pop() ?? bundleId;

/**
 * 槽位展示：短、可扫读；长值（bundle id / 文本 / 命令）一律截断，父层 truncate 兜底。
 * 宏只有 id，名字得去宏库查——`macroName` 由调用方给（`macros.ts` 的 `useMacroLabels`），
 * 查不到就退回 id，宁可露 id 也不空着。
 */
export function fmtAction(a: Action, macroName?: (id: string) => string): string {
  switch (a.type) {
    case "shortcut":
      return fmtShortcut(a.shortcut);
    case "open_app": {
      const a1 = a.args?.trim();
      return `${t("remote.act.openApp")} ${appTail(a.bundle_id)}${a1 ? ` ${a1}` : ""}`;
    }
    case "agent_cli": {
      const a1 = a.args?.trim() ?? "";
      const s1 = `${a.app}${a1 ? ` ${a1}` : ""}`;
      return `$ ${s1.length > 16 ? `${s1.slice(0, 16)}…` : s1}`;
    }
    case "text": {
      const s = a.content.trim().replace(/\s+/g, " ");
      return `${t("remote.act.text")} ${s.length > 8 ? `${s.slice(0, 8)}…` : s}`;
    }
    case "focus_input":
      return t("remote.act.focusInput");
    case "run_command": {
      const s = a.command.trim();
      return `$ ${s.length > 14 ? `${s.slice(0, 14)}…` : s}`;
    }
    case "macro": {
      // 只显示宏名（用户 2026-11-24：绑「动作1」就显示「动作1」，别挂「动作」前缀重复）。
      const n = macroName ? macroName(a.id) : a.id;
      return n;
    }
  }
}
