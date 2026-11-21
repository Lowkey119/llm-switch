// 映射区中位分流（队列 22 页面档案化）：有实物图标定资产的档案走 RemoteImage
// 热点层；有键表但无图的档案（如小米 1 代）**不得套用别的型号的实物图**
// （上游「未识别不套图」口径），降级为键名网格——悬停/选中高亮与两侧键列同源，
// 键卡、三槽、连线逻辑全部复用 Mapping，不另起平行实现。
import { t } from "../../lib/i18n";
import { RemoteImage } from "./RemoteImage";
import type { KeyDef } from "./keys";

export function MapCenter({
  hasImage,
  keys,
  ...img
}: {
  /** 该档案是否有实物图标定资产（keys.ts::hasRemoteImage）。 */
  hasImage: boolean;
  /** 档案键表过滤后的键（与两侧键列同一份）。 */
  keys: KeyDef[];
} & Parameters<typeof RemoteImage>[0]) {
  if (hasImage) return <RemoteImage {...img} />;
  const { edit, sel, hover, live, held, after, onSelect, onHover, className = "" } = img;
  return (
    <div
      className={`w-[240px] self-center rounded-[10px] border border-dashed border-slate-600 px-3.5 py-2.5 ${className}`}
    >
      <p className="mb-2 text-center text-2xs text-ink-3">{t("remote.gridTitle")}</p>
      <div className="grid grid-cols-3 gap-1.5">
        {keys.map((k) => {
          const Icon = k.icon;
          // 高亮目标与实物热点层同口径：按住 > 余晖 > 实时 > 悬停优先选中兜底。
          const on =
            edit &&
            (held === k.id ||
              after === k.id ||
              live === k.id ||
              (hover ?? sel) === k.id);
          return (
            <button
              key={k.id}
              type="button"
              disabled={!edit}
              aria-pressed={sel === k.id}
              onClick={() => onSelect(k.id)}
              onMouseEnter={() => onHover(k.id)}
              onMouseLeave={() => onHover(null)}
              // 运行态不可点也不给 hover 反馈（与实物热点 1.4 口径一致）。
              className={`flex flex-col items-center gap-1 rounded-md border px-1 py-1.5 text-2xs transition-colors disabled:cursor-default ${
                on
                  ? "border-[var(--key-hl)] bg-[var(--key-hl)]/15 text-[var(--key-hl)]"
                  : `border-slate-700 text-ink-3${edit ? " hover:border-[var(--key-hl)] hover:text-ink-1" : ""}`
              }`}
            >
              <Icon className="h-4 w-4 shrink-0" />
              <span className="w-full truncate text-center">{t(k.nameKey)}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
