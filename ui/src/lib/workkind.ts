// 模型工种类（前端镜像）：chat / image_gen / tts / asr / video_gen，与后端
// `gateway::model_kind` 同一口径——判定优先级 **人工声明 > 名字启发式（暴露名 ⊕
// 上游真名都试）> 缺省 chat**（未知默认聊天，宁可放行）。名字启发式必须与 Rust
// `suggested_kind_by_name` 保持一致，否则 UI 显示的工种和路由实际选路会打架。
// 分组展示在 ModelRows；行内人工改声明走 IPC `set_model_kind`。

import type { ModelFlag } from "./ipc";

export type WorkKind = "chat" | "image_gen" | "tts" | "asr" | "video_gen";

/** 分组展示顺序（聊天在前，媒体工种在后）。 */
export const WORK_ORDER: WorkKind[] = ["chat", "image_gen", "tts", "asr", "video_gen"];

export const isWorkKind = (s: string): s is WorkKind =>
  (WORK_ORDER as string[]).includes(s);

/** 生图族确认词（与 Rust suggested_kind_by_name 同步维护）。 */
const IMAGE_PATTERNS = ["flux", "sdxl", "z-image", "qwen-image", "stable-diffusion", "wan-"];

/** 名字启发式（大小写不敏感，取路径尾段）；没特征返回 null（= 缺省聊天）。 */
export function suggestWorkKind(name: string): WorkKind | null {
  const tail = (name.trim().toLowerCase().split("/").pop() ?? "").trim();
  if (IMAGE_PATTERNS.some((p) => tail.includes(p))) return "image_gen";
  if (tail.includes("tts") || tail.includes("speech")) return "tts";
  if (tail.includes("asr") || tail.includes("whisper")) return "asr";
  return null;
}

/** 行级工种类：人工声明 > 名字启发式（暴露名 ⊕ 上游真名）> chat。 */
export function resolveWorkKind(flag?: ModelFlag, publicName?: string): WorkKind {
  const declared = flag?.kind?.trim();
  if (declared && isWorkKind(declared)) return declared;
  const pub = flag?.public_name ?? publicName ?? "";
  const byPub = suggestWorkKind(pub);
  if (byPub) return byPub;
  if (flag && flag.upstream_name !== pub) {
    const byUp = suggestWorkKind(flag.upstream_name);
    if (byUp) return byUp;
  }
  return "chat";
}

/** 是否媒体工种（非聊天：永不进聊天择路与分配格子）。 */
export const isMediaKind = (k: WorkKind) => k !== "chat";
