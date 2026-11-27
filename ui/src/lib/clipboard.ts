// 复制文本（UI-AUDIT-2 E-2）：模型 id、路径这类长串截断之后必须有一键拿全文的出口。
//
// WKWebView 里 navigator.clipboard 不一定可用（非安全上下文或被策略挡掉），
// 所以留一条 execCommand 兜底；两条都失败才说失败——静默「复制成功」比不复制更坏。
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    /* 落到兜底 */
  }
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.top = "-1000px";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(ta);
    return ok;
  } catch {
    return false;
  }
}
