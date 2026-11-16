// 模型 id 的可读短标签：与后端 takeover::model_prefix 同一套规则（TS 镜像）——
// 剥掉尾部量化/格式后缀（-mlx-8bit、-oq6e-mtp…），仍过长再截断到 16 字符。
// 用途：pi 双槽（主/小模型）的槽位显示名 = 用户实际配置的有意义模型前缀。

const TAIL_TAGS = [
  "8bit", "mtp", "oq6e", "oq4", "oq6", "mlx", "a3b", "apex",
  "fp16", "next", "oq", "bf16", "int4", "gguf",
] as const;

const MAX_LEN = 16;

/** 浮窗短名：先剥组织前缀（`a16621388703/Qwen3.8-35B-A3B-GGUF` →
 * `Qwen3.8-35B-A3B-GGUF`），再逐层剥尾缀白名单；仍过长交给 CSS truncate
 * （标准截断 + 省略号），这里不重复截。 */
export function shortModel(id: string): string {
  let cur = id.trim();
  const slash = cur.lastIndexOf("/");
  if (slash >= 0) cur = cur.slice(slash + 1);
  for (;;) {
    const pos = cur.lastIndexOf("-");
    if (pos < 0) break;
    const tail = cur.slice(pos + 1).toLowerCase();
    if (!(TAIL_TAGS as readonly string[]).includes(tail)) break;
    cur = cur.slice(0, pos);
  }
  return cur || id.trim();
}

export function modelPrefix(id: string): string {
  let cur = id.trim();
  // 循环剥尾段：匹配白名单即整段去掉（多段后缀逐层剥）。
  for (;;) {
    const pos = cur.lastIndexOf("-");
    if (pos < 0) break;
    const tail = cur.slice(pos + 1).toLowerCase();
    if (!(TAIL_TAGS as readonly string[]).includes(tail)) break;
    cur = cur.slice(0, pos);
  }
  const chars = [...cur];
  return chars.length > MAX_LEN ? `${chars.slice(0, MAX_LEN).join("")}…` : cur;
}
