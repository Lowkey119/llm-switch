// 模型种类（按名称启发式 + 已有探测事实）：llm / moe / vlm / embedding / rerank / asr / tts。
// 用途：后端模型清单分组展示 + 参数弹窗按种类适配（向量化/语音类不吃采样参数）。
// 判定口径（优先级从上到下）：
// - embedding / rerank / asr / tts 先按名称认领（这类模型名几乎都自带身份词）；
// - moe 按常见命名（AxB 激活参数量、MoE 字样）；
// - vlm 用探测/人工事实（model_meta.vlm === true）或名称（-vl/llava/internvl 等）；
// - 都不沾 = llm（默认大头，别乱猜）。

export type ModelKind = "llm" | "moe" | "vlm" | "embedding" | "rerank" | "asr" | "tts";

export const KIND_LABEL: Record<ModelKind, string> = {
  llm: "LLM",
  moe: "MoE",
  vlm: "视觉",
  embedding: "向量化",
  rerank: "重排",
  asr: "语音识别",
  tts: "语音合成",
};

/** 分组展示顺序（推理类在前、专用小模型在后）。 */
export const KIND_ORDER: ModelKind[] = ["llm", "moe", "vlm", "embedding", "rerank", "asr", "tts"];

/** 非推理种类：采样/惩罚/长度等参数不会随请求下发，参数弹窗不再平铺。 */
export const NON_INFERENCE_KINDS: readonly ModelKind[] = ["embedding", "rerank", "asr", "tts"];

/** 名称启发式（大小写不敏感）；`vlmFact` = model_meta.vlm 有效值（探测/人工三态结论）。 */
export function detectKind(name: string, vlmFact?: boolean | null): ModelKind {
  const n = name.toLowerCase();
  if (/rerank/.test(n)) return "rerank";
  if (/embed|\bbge\b|gte-|mxbai|nomic|\be5[-.]/.test(n)) return "embedding";
  if (/whisper|\basr\b|speech[-_]?2[-_]?text|audio[-_]?to[-_]?text|paraformer|sense[-_]?voice/.test(n))
    return "asr";
  if (
    /(^|[-_.])tts($|[-_.])|text[-_]?to[-_]?speech|kokoro|\bvits\b|fish[-_]?speech|cosyvoice/.test(n)
  )
    return "tts";
  // MoE：AxB 激活参数命名（35b-a3b / -a14b…）或名字里直接写 MoE。
  if (/moe|-\d+(\.\d+)?b[-_]a\d+(\.\d+)?b\b|[-_]a\d{1,3}b\b/.test(n)) return "moe";
  // 视觉：探测事实优先，名称兜底。
  if (vlmFact === true) return "vlm";
  if (
    /(^|[-_./])(vl|vlm)($|[-_./])|vision|llava|internvl|qwen[0-9.]*[-_]?vl|glm[-_.]?4v|molmo|pixtral|smolvlm|mini[-_]?cpm[-_]?v|aria[-_]?2|florence/.test(n)
  )
    return "vlm";
  return "llm";
}
