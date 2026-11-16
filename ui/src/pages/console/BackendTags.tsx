// 控制台顶部：后端标签网格（从 ConsolePage 原样搬出，纯展示 + 回调）。
import { StatusDot } from "../../components/ui";
import type { BackendRow } from "../../lib/ipc";

/** 左列点亮口径（见 useConsoleFlow）：每个后端此刻在飞的请求数。 */
export type FlowMap = Record<string, number>;

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
  return (
    // 横排网格，每行至少 3 个，宽屏自动多列，把横向空间让给控制台
    <div className="grid max-h-48 shrink-0 gap-2 overflow-auto grid-cols-[repeat(auto-fill,minmax(160px,1fr))]">
      {rows.map((b) => {
        // 有数据流（此刻在飞请求）= 点亮，否则压暗。
        const live = (flow[b.name] ?? 0) > 0;
        // 第二行是变量长度文案（engine/协议 + 入口说明），截断后必须 title 读回（P3-5）。
        const line2 =
          (b.engine ?? b.protocol) +
          (b.engine === "halogen" || b.engine === "gufo"
            ? " · SSH 控制面"
            : b.webui_url
              ? ""
              : " · 入口=base_url");
        return (
          <button
            key={b.id}
            onClick={() => onSelect(b.id)}
            className={`w-full rounded-lg border-l-[3px] px-3 py-2 text-left text-sm transition-colors ${
              sel?.id === b.id
                ? "border-l-cyan-500/50 bg-cyan-500/10 text-ink-1"
                : live
                  ? "border-l-[var(--sw-on)] bg-[var(--sw-on)]/10 text-ink-1 shadow-[0_0_14px_-4px_var(--sw-on)]"
                  : "border-slate-800 bg-slate-900/40 text-ink-3 hover:border-slate-700"
            }`}
          >
            <span className="flex items-center gap-2">
              <StatusDot state={b.health_state} />
              <span className="truncate" title={`${b.name} · ${b.base_url}`}>{b.name}</span>
            </span>
            <span className="mt-0.5 block truncate text-xs text-ink-4" title={line2}>
              {line2}
            </span>
          </button>
        );
      })}
      {!rows.length && <p className="px-1 text-xs text-ink-4">尚无后端。</p>}
    </div>
  );
}
