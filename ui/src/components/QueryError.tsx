import { IconClose } from "./icons";
import { t } from "../lib/i18n";

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
  details,
  onRetry,
  retryLabel = t("common.retry"),
  inline = false,
  onDismiss,
}: {
  title: string;
  error?: unknown;
  /**
   * 多个数据源同时失败时逐条传（用量页：日聚合 / 统计 / 成本 / 媒体计量）。
   * 拼成一长串用「；」分隔会把一行拉到 130+ 字符，mono 下根本读不完哪条对应哪个源。
   */
  details?: string[];
  onRetry?: () => void;
  retryLabel?: string;
  inline?: boolean;
  onDismiss?: () => void;
}) {
  const detail = errText(error);
  // 明细统一归一成行数组：块状逐行排，inline 只有一行就连起来。
  // 两个变体都只读 lines，否则给 inline 传 details 会静默吞掉。
  const lines = (details?.length ? details : detail ? [detail] : []).filter(Boolean);

  if (inline) {
    // 用 span 而非 div：这些提示常嵌在 <span> / <td> 里，div 会撑破父级行内布局。
    return (
      <span className="flex min-w-0 flex-wrap items-center gap-2 text-sm text-[var(--sem-danger-text)]">
        <span className="min-w-0 break-words">
          <span className="font-medium">{title}</span>
          {lines.length > 0 && (
            <span className="break-all">
              {t("common.colon")}
              {lines.join(t("common.semi"))}
            </span>
          )}
        </span>
        {onRetry && (
          <button
            onClick={onRetry}
            className="shrink-0 underline underline-offset-2 hover:text-ink-1"
          >
            {retryLabel}
          </button>
        )}
        {onDismiss && (
          <button
            onClick={onDismiss}
            aria-label={t("common.dismiss")}
            className="shrink-0 rounded p-0.5 text-ink-4 transition-colors hover:text-ink-2"
          >
            <IconClose width={11} height={11} />
          </button>
        )}
      </span>
    );
  }

  // role=alert：读取失败是「程序出事了」，得让读屏当场念出来（U10）。
  // inline 变体故意不加——它常一处页面出现好几条（每行/每卡一条），
  // 首屏一起念会把人淹掉，且它就贴在出问题的东西旁边，指认已经够了。
  return (
    <div role="alert" className="rounded-xl border border-rose-500/40 bg-rose-500/10 p-4">
      <h2 className="text-base font-semibold text-[var(--sem-danger-text)]">{title}</h2>
      {lines.length > 0 && (
        <ul className="mt-2 flex flex-col gap-1">
          {lines.map((l, i) => (
            <li key={i} className="break-all font-mono text-xs text-[var(--sem-danger-text)]">
              {l}
            </li>
          ))}
        </ul>
      )}
      {onRetry && (
        <button
          onClick={onRetry}
          className="mt-3 rounded-lg border border-slate-700 bg-slate-800/60 px-3 py-1.5 text-sm text-ink-2 transition-colors hover:border-cyan-500/50"
        >
          {retryLabel}
        </button>
      )}
    </div>
  );
}
