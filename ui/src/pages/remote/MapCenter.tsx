// 映射区中位分流（队列 22 页面档案化）：有实物图标定资产的档案走 RemoteImage
// 热点层；有键表但无图的档案**不得套用别的型号的实物图**
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
  const { className = "" } = img;
  if (hasImage)
    // 图按行高定尺 + 画布正中（用户 2026-11-24 拍板）：中位列 w-0 不占宽，两侧 1fr
    // 等宽 ⇒ 列心 = 画布心 = 两键列中线。图 absolute left-1/2 -translate-x-1/2
    // 居中钉在列上，h-full w-auto 保原比例（拉伸填满列曾把机身拉宽、热点错位）。
    // 热点层同锚点同变换（RemoteImage），零偏移。资产按档案取，均已物理裁切。
    return <RemoteImage {...img} className={`h-full ${className}`} imgClassName="absolute left-1/2 top-0 h-full w-auto -translate-x-1/2" />;
  const { edit, sel, hover, live, held, after, onSelect, onHover } = img;
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
                  ? "border-[var(--key-hl-text)] bg-[var(--key-hl)]/15 text-[var(--key-hl-text)]"
                  : `border-slate-700 text-ink-3${edit ? " hover:border-[var(--key-hl-text)] hover:text-ink-1" : ""}`
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
