// 控制台顶部：后端标签网格（从 ConsolePage 原样搬出，纯展示 + 回调）。
import { StatusDot } from "../../components/ui";
import { t } from "../../lib/i18n";
import type { BackendRow } from "../../lib/ipc";

/** 左列点亮口径（见 useConsoleFlow）：每个后端此刻在飞的请求数。 */
export type FlowMap = Record<string, number>;

/**
 * 后端标签网格——过滤分段已按用户 2026-11-20 拍板删除：每格自带健康状态灯，
 * 有效无效一目了然，分段是多余的重复表达。
 */
export function BackendTags({
  rows,
  selected,
  flow,
  onSelect,
}: {
  rows: BackendRow[];
  selected: number | null;
  flow: FlowMap;
  onSelect: (id: number) => void;
}) {
  const sel = rows.find((x) => x.id === selected) ?? rows[0] ?? null;
  // 横排标签：宽度随名字自适应（用户 2026-11-20 拍板），超长封顶截断、放不下自动换行。
  return (
    <div className="flex max-h-48 shrink-0 flex-wrap gap-2 overflow-auto">
      {rows.map((b) => {
        // 有数据流（此刻在飞请求）= 点亮，否则压暗。
        const live = (flow[b.name] ?? 0) > 0;
        return (
          <button
            key={b.id}
            onClick={() => onSelect(b.id)}
            className={`max-w-64 rounded-lg border-l-[3px] px-3 py-2 text-left text-sm transition-colors ${
              sel?.id === b.id
                ? "border-l-cyan-500/50 bg-cyan-500/10 text-ink-1"
                : live
                  ? "border-l-[var(--sw-on)] bg-[var(--sw-on)]/10 text-ink-1 shadow-[0_0_14px_-4px_var(--sw-on)]"
                  : "border-slate-800 bg-slate-900/40 text-ink-3 hover:border-slate-700"
            }`}
          >
            <span className="flex min-w-0 items-center gap-2">
              <StatusDot state={b.health_state} />
              <span className="truncate" title={`${b.name} · ${b.base_url}`}>{b.name}</span>
            </span>
          </button>
        );
      })}
      {!rows.length && <p className="px-1 text-sm text-ink-4">{t("console.tagNone")}</p>}
    </div>
  );
}
