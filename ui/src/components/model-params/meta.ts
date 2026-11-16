// 参数弹窗的目录口径与纯函数（计划 §6.1 自 ModelParamsDialog.tsx 拆出）。
// 这里只放「不依赖组件状态」的东西：组序/标签、只读组、范围钳制与展示规则。

export interface CatalogEntry {
  key: string;
  group: string;
  default_policy: string;
}

export interface MetaRow {
  model: string;
  cap_ctx: number | null;
  cap_output: number | null;
  /** 视觉能力有效值（探测/人工三态结论）：种类判定用。 */
  vlm?: boolean | null;
}

export interface Props {
  backend: string;
  model: string;
  onClose: () => void;
}

/** 组序与中文标签；runtime 是引擎运行时（目录注释：只读展示，代理不下发）。 */
export const GROUP_ORDER = [
  "sampling",
  "penalties",
  "length",
  "structured",
  "vlm",
  "speculative",
  "reasoning",
  "runtime",
] as const;
export const GROUP_LABEL: Record<string, string> = {
  sampling: "采样",
  penalties: "惩罚",
  length: "长度",
  structured: "结构化输出",
  vlm: "多模态 VLM",
  speculative: "投机解码 / MTP",
  reasoning: "Reasoning",
  runtime: "引擎运行时（只读）",
};
export const READONLY_GROUP = "runtime";
/** 受模型输出上限约束的键；与 length 组其余键一样，0 = 继承。 */
export const OUTPUT_CAPPED = new Set(["max_tokens", "max_completion_tokens"]);

export const field =
  "rounded-md border border-slate-700 bg-slate-900 px-2 py-1 text-xs text-ink-2";

/** 未下发时的说明：网关不设这个参数，也读不到引擎运行时值（引擎不提供统一查询口）。 */
export const NO_OVERRIDE_HINT =
  "代理未设置该参数：请求没显式带上时，由推理后端按自身配置决定";


/** 模型允许范围（编辑钳制依据）：元数据的有效上限（人工声明已钳到后端允许范围）。 */
export interface Limits {
  capOutput: number | null;
  capCtx: number | null;
  lengthKeys: Set<string>;
}

/** 可编辑 = 非只读组 且 在引擎能力集内（能力集未约束时全放行）。 */
export function editable(en: CatalogEntry, capKeys: Set<string> | null): boolean {
  return en.group !== READONLY_GROUP && (!capKeys || capKeys.has(en.key));
}

export function dimReason(en: CatalogEntry): string {
  return en.group === READONLY_GROUP
    ? "引擎运行时参数：代理不下发，只读展示"
    : "该引擎不支持此参数";
}

/** 展示值：长度键的 0 标注「继承」，免得用户以为 0 会下发。 */
export function dispVal(key: string, v: unknown, lengthKeys: Set<string>): string {
  return `${JSON.stringify(v)}${lengthKeys.has(key) && v === 0 ? "（继承）" : ""}`;
}

/** 编辑框下的范围提示（模型允许范围）。 */
export function capHint(
  key: string,
  lim: { capOutput: number | null; capCtx: number | null; lengthKeys: Set<string> },
): string | null {
  if (OUTPUT_CAPPED.has(key) && lim.capOutput != null)
    return `允许 0（继承）～ ${lim.capOutput.toLocaleString()}（模型输出上限）`;
  if (key === "ctx" && lim.capCtx != null)
    return `允许 0（继承）～ ${lim.capCtx.toLocaleString()}（模型上下文上限）`;
  if (lim.lengthKeys.has(key)) return "0 = 继承（不下发）";
  return null;
}

/** 范围校验：只拦数值；长度键超出模型允许范围直接拒绝。 */
export function checkRange(
  key: string,
  v: unknown,
  lim: { capOutput: number | null; capCtx: number | null },
): string | null {
  if (typeof v !== "number") return null; // 非数值键不拦（stop/response_format 等）
  if (v < 0) return "数值不能为负";
  if (OUTPUT_CAPPED.has(key) && lim.capOutput != null && v > lim.capOutput)
    return `超过模型输出上限 ${lim.capOutput.toLocaleString()}`;
  if (key === "ctx" && lim.capCtx != null && v > lim.capCtx)
    return `超过模型上下文上限 ${lim.capCtx.toLocaleString()}`;
  return null;
}
