import { IconClose } from "./icons";

/**
 * 读取/操作失败的统一呈现（UI-AUDIT P0-4）。
 *
 * 「读取失败 ≠ 空态」是这套 UI 一直守着的底线，但红卡样式此前在六处各写一遍、
 * 灰底横幅又写了三遍，改一次要改九处。这里收成两个变体：
 *   默认   —— 页面级整块失败（配 ErrorBoundary 同款外观）
 *   inline —— 工具条里的一行提示，带重试或关闭
 */

/** 把 react-query 的 error / catch 到的未知值转成人能读的一行。 */
export function errText(e: unknown): string {
  if (e == null) return "";
  if (e instanceof Error) return e.message || String(e);
  const s = typeof e === "string" ? e : String(e);
  // Tauri 的 invoke 报错常带 `Error: ` 前缀，留着只会占掉半行宽度。
  return s.replace(/^Error:\s*/, "").trim();
}

export function QueryError({
  title,
  error,
  onRetry,
  retryLabel = "重试",
  inline = false,
  onDismiss,
}: {
  title: string;
  error?: unknown;
  onRetry?: () => void;
  retryLabel?: string;
  inline?: boolean;
  onDismiss?: () => void;
}) {
  const detail = errText(error);

  if (inline) {
    // 用 span 而非 div：这些提示常嵌在 <span> / <td> 里，div 会撑破父级行内布局。
    return (
      <span className="flex min-w-0 flex-wrap items-center gap-2 text-xs text-[var(--sem-danger-text)]">
        <span className="min-w-0 break-words">
          <span className="font-medium">{title}</span>
          {detail && <span className="break-all">：{detail}</span>}
        </span>
        {onRetry && (
          <button
            onClick={onRetry}
            className="shrink-0 underline underline-offset-2 hover:text-ink-1"
          >
            重试
          </button>
        )}
        {onDismiss && (
          <button
            onClick={onDismiss}
            aria-label="关闭提示"
            className="shrink-0 rounded p-0.5 text-ink-4 transition-colors hover:text-ink-2"
          >
            <IconClose width={11} height={11} />
          </button>
        )}
      </span>
    );
  }

  return (
    <div className="rounded-xl border border-rose-500/40 bg-rose-500/10 p-4">
      <h2 className="text-sm font-semibold text-[var(--sem-danger-text)]">{title}</h2>
      {detail && <p className="mt-2 break-all font-mono text-xs text-[var(--sem-danger-text)]">{detail}</p>}
      {onRetry && (
        <button
          onClick={onRetry}
          className="mt-3 rounded-lg border border-slate-700 bg-slate-800/60 px-3 py-1.5 text-xs text-ink-2 transition-colors hover:border-cyan-500/50"
        >
          {retryLabel}
        </button>
      )}
    </div>
  );
}
