// 参数中心共用的类型与分组标签（计划 §6.1 自 ParamsCenter.tsx 拆出）。
export interface ParamRow {
  id: number;
  scope: string;
  ref_id: string;
  key: string;
  value: unknown;
  policy: string;
}

export type Scope = "global" | "backend" | "model";

export interface CatalogEntry {
  key: string;
  group: string;
  default_policy: string;
}

/** 目录分组展示 key（渲染时 t()；未知分组原样显示组名）。 */
export const GROUP_LABELS: Record<string, string> = {
  sampling: "pc.grpSampling",
  penalties: "pc.grpPenalties",
  length: "pc.grpLength",
  structured: "pc.grpStructured",
  vlm: "pc.grpVlm",
  speculative: "pc.grpSpeculative",
  reasoning: "pc.grpReasoning",
  runtime: "pc.grpRuntime",
};
