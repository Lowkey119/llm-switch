// 快速面板中栏的行模型与建行（从 PromptPanel 拆出，行数预算 §6.1）。
//
// 浮窗**只读**（用户 2026-11-25 拍板：内容编辑全在遥控器页），行只有六种，`kind` 决定 OK 语义：
// - `char` 文本内容 `{t,n}` → 追加进待注入（连着拼字符）；
// - `action` shell 命令 `{cmd}` → 装入待执行（`quickpanel_run_action`）；
// - `macro` **引用**已有动作 `{macro:id}` → 交 `remote_macro_run`（改动作同步生效）；
// - `step` 单发动作 `{action:{...}}` → 交 `remote_action_run`；
// - `skill` 项目技能（只读预览）；`cfginfo` 配置 tab 的只读读数；`add` 新增入口行。
import { t } from "../lib/i18n";
import { fmtAction, type Action } from "../pages/remote/action";
import { itemKind, type QpItem, type QpTab } from "../pages/remote/quickpanel";

export type SkillRow = { dir: string; name: string; description?: string | null };

/** quickpanel.json 的形状（tab/item 类型与遥控器页编辑器共用一份，见 `pages/remote/quickpanel.ts`）。 */
export interface QPConfig {
  tabs: QpTab[];
  path: string;
  error: string | null;
}

/** 内置 tab 的 i18n 标签（用户自定义 tab 用 JSON 里的 label 原文）。 */
export const TAB_LABEL_KEYS: Record<string, string> = {
  common: "prompt.tabCommon",
  shell: "prompt.tabShell",
  skills: "prompt.tabSkills",
  hot: "prompt.tabHot",
  ctrl: "prompt.tabCtrl",
  sym: "prompt.tabSym",
  num: "prompt.tabNum",
  acts: "prompt.tabActs",
  cfg: "prompt.tabCfg",
};

export type Row =
  | { kind: "skill"; id: string; content: string; dir: string }
  | { kind: "char"; id: string; content: string; note: string }
  | { kind: "action"; id: string; content: string; cmd: string; confirm: boolean }
  | { kind: "macro"; id: string; content: string; macro: string }
  | { kind: "step"; id: string; content: string; action: Action }
  | { kind: "cfginfo"; id: string; content: string; note: string };

/**
 * JSON 条目 → 行。种类判定与后端 `QpItem::kind` 同优先级（macro > action > cmd > 文本），
 * 所以**任何类型的 tab 都能混放**四种条目（用户要的就是「往 tab 里加一条已有动作」）。
 */
export function qpRows(td: QpTab, macroName: (id: string) => string): Row[] {
  return (td.items ?? []).map((it: QpItem, i) => {
    const id = `${td.key}-${i}`;
    switch (itemKind(it)) {
      case "macro":
        return { kind: "macro", id, macro: it.macro ?? "", content: macroName(it.macro ?? "") };
      case "action":
        return { kind: "step", id, action: it.action as Action, content: fmtAction(it.action as Action, macroName) };
      case "cmd":
        return {
          kind: "action",
          id,
          content: (it.t ?? "").trim() || (it.cmd ?? ""),
          cmd: it.cmd ?? "",
          confirm: it.confirm === true,
        };
      default:
        return { kind: "char", id, content: it.t ?? "", note: it.n ?? "" };
    }
  });
}

export function buildRows(o: {
  td?: QpTab;
  cfgTabs: QpTab[];
  skills: SkillRow[];
  labelOf: (tb: QpTab) => string;
  macroName: (id: string) => string;
}): Row[] {
  const { td, cfgTabs, skills, labelOf, macroName } = o;
  // 搜索态已随浮窗搜索框一起去掉（用户 2026-11-26）：浮窗只按 tab 列条目，
  // 要跨类找东西去遥控器页（那里有搜索框也不抢遥控器焦点）。
  if (!td) return [];
  switch (td.type) {
    // prompts 与 chars/actions 同一读法：条目都在 JSON 里（原 `prompt_items` 那条线已退役）。
    case "chars":
    case "actions":
    case "prompts":
      return qpRows(td, macroName);
    case "config":
      return cfgTabs.map((tb, i) => ({
        kind: "cfginfo",
        id: "g" + i,
        content: `${labelOf(tb)} · ${tb.type}`,
        note: tb.items ? `${tb.items.length} ${t("prompt.cfgItems")}` : "",
      }));
    case "skills":
      return skills.map((s) => ({ kind: "skill", id: "sk-" + s.name, content: s.name, dir: s.dir }));
    default:
      return [];
  }
}
