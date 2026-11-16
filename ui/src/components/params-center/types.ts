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

export const GROUP_LABELS: Record<string, string> = {
  sampling: "采样",
  penalties: "惩罚",
  length: "长度",
  structured: "结构化",
  vlm: "多模态",
  speculative: "投机解码",
  reasoning: "reasoning",
  runtime: "引擎运行时（只读，去控制台改）",
};
