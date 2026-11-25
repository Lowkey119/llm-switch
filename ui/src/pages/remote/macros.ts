// 宏（C3，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）：命名的动作序列。
//
// 形状与 Rust `services::remote::macros::{Macro, Step}` 对齐（serde tagged union）。
// 槽里只存宏 id（`Action::Macro`），步骤住宏库 `remote/macros.<profile>.json`：
// 改一次所有绑点同时生效，删宏由后端跨全部方案清引用。
import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { t } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";
import { ACTION_TYPES, fmtAction, type Action, type ActionType } from "./action";

export interface Macro {
  id: string;
  name: string;
  /** true = 某步失败即中断；false = 记错继续跑完。 */
  stop_on_error: boolean;
  steps: Step[];
}

/** 序列步骤：一个动作，或一段等待（步骤里不许再套宏，后端 normalize 也这么判）。 */
export type Step = { type: "action"; action: Action } | { type: "delay"; ms: number };

export const MACRO_QUERY = ["remote-macros"];

/** 单步延迟上限（毫秒），与后端 `MAX_DELAY_MS` 同源。 */
export const MAX_DELAY_MS = 5000;

/** 步骤编辑器可选的步骤种类：动作类型去掉「宏」（不套娃）+ 等待 + 系统功能（内置组合清单）。 */
export type StepKind = Exclude<ActionType, "macro"> | "delay" | "sys";
export const STEP_KINDS: { id: StepKind; labelKey: string }[] = [
  // filter 不收窄联合类型，显式换一遍 id（新增动作类型时这里跟着走）。
  // 注入文字 / 聚焦输入框不进芯片行（用户 2026-11-24 拍板）：动作序列里用不上，
  // 槽位直录（FnPicker 那套）仍保留这两个类型。
  ...ACTION_TYPES.filter((a) => a.id !== "macro" && a.id !== "text" && a.id !== "focus_input").map(
    (a) => ({ id: a.id as StepKind, labelKey: a.labelKey }),
  ),
  { id: "sys", labelKey: "remote.macro.sysTitle" },
  { id: "delay", labelKey: "remote.macro.delay" },
];

export const emptyMacro = (): Macro => ({ id: "", name: "", stop_on_error: true, steps: [] });

/** 当前映射里绑定了这条动作的键槽数（删除前的二次确认警示用，用户 2026-11-24）。 */
export function countMacroBindings(
  keys: Record<string, { click: Action | null; double: Action | null; long: Action | null }>,
  id: string,
): number {
  return Object.values(keys).reduce(
    (n, kc) =>
      n +
      (["click", "double", "long"] as const).filter((s) => {
        const a = kc[s];
        return a?.type === "macro" && a.id === id;
      }).length,
    0,
  );
}

/** 宏列表（按档案分仓，后端按绑定设备解析档案）。列表只在宏库/选择器里读，故常驻缓存。 */
export const useMacros = () =>
  useQuery({
    queryKey: MACRO_QUERY,
    queryFn: () => ipc<Macro[]>("remote_macros_get"),
    staleTime: Infinity,
  });

/** id → 名称的读数函数（槽位显示宏动作要用）。查不到退回 id：宁可露 id 也不空着。 */
export function useMacroLabels(): (id: string) => string {
  const q = useMacros();
  const list = q.data;
  return useMemo(() => {
    const map = new Map((list ?? []).map((m) => [m.id, m.name]));
    return (id: string) => map.get(id) ?? id;
  }, [list]);
}

export const delayStep = (ms: number): Step => ({ type: "delay", ms: Math.min(MAX_DELAY_MS, Math.max(0, Math.round(ms))) });
export const actionStep = (action: Action): Step => ({ type: "action", action });

export function fmtStep(s: Step, macroName?: (id: string) => string): string {
  return s.type === "delay" ? `${t("remote.macro.delay")} ${s.ms}ms` : fmtAction(s.action, macroName);
}

/** 一行读数（列表用）：`⌘⇧4 → 等待 500ms → ↩`；没步骤用 `—`，不留占位文案。 */
export function fmtMacro(m: Macro, macroName?: (id: string) => string): string {
  if (!m.steps.length) return "—";
  return m.steps.map((s) => fmtStep(s, macroName)).join(" → ");
}

export const stepCount = (n: number) => t("remote.macro.count").replace("{n}", String(n));
