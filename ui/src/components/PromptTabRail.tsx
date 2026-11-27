// 快速面板的左侧竖排 tab 轨（v2：tab 由 ~/.llm-switch/quickpanel.json 用户自定义驱动）。
//
// 轨宽 132px + 20px 字号（计划 2026-11-26 §三.6 默认整窗 zoom 2 后从 168 降到 132：
// 物理宽度不变、布局空间减半，再留 168 会把条目列挤没）。sep 标记的 tab 前画虚线分隔。
import { Fragment, useState } from "react";
import { t } from "../lib/i18n";
import { RowMenu, type MenuItem } from "../pages/remote/listOps";

interface TabItem {
  key: string;
  label: string;
  sep?: boolean;
}

interface Props {
  tabs: TabItem[];
  /** 当前 tab（高亮那档）。 */
  tab: string;
  onPick: (key: string) => void;
  /**
   * 按 tab 给右键菜单项（返回空数组 = 该 tab 无菜单）。
   * 计划 §三.7：「最近发送」的清空入口就放这儿，不另造一套菜单组件。
   */
  menuFor?: (key: string) => MenuItem[];
  /** 轨宽（用户拖分隔条调过的值）；默认 132 = 计划 §三.6 的整窗 zoom 2 口径。 */
  width?: number;
}

export default function PromptTabRail(a: Props) {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const [menuKey, setMenuKey] = useState("");
  return (
    <nav
      className="flex shrink-0 flex-col gap-1 overflow-y-auto border-r border-slate-800 px-2 py-2"
      style={{ width: a.width ?? 132 }}
      onPointerDown={(e) => e.stopPropagation()}
      aria-label={t("prompt.title" as never)}
    >
      {a.tabs.map((tb) => (
        <Fragment key={tb.key}>
          {tb.sep && <span className="mx-1 my-1 border-t border-dashed border-slate-700" aria-hidden="true" />}
          <button
            key={tb.key}
            title={tb.label}
            aria-current={a.tab === tb.key ? "page" : undefined}
            onClick={() => a.onPick(tb.key)}
            onContextMenu={(e) => {
              const items = a.menuFor?.(tb.key) ?? [];
              if (!items.length) return;
              e.preventDefault();
              setMenuKey(tb.key);
              setAt({ x: e.clientX, y: e.clientY });
            }}
            className={`w-full truncate rounded-md px-2 py-2 text-left text-xl ${
              a.tab === tb.key
                ? "bg-[var(--sel-bg)] text-ink-1 ring-1 ring-inset ring-[var(--sel-ring)] font-medium"
                : "text-ink-3 hover:text-ink-1"
            }`}
          >
            {tb.label}
          </button>
        </Fragment>
      ))}
      <RowMenu at={at} items={at ? (a.menuFor?.(menuKey) ?? []) : []} onClose={() => setAt(null)} />
    </nav>
  );
}
