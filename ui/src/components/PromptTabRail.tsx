// 快速面板的左侧竖排 tab 轨（v2：tab 由 ~/.llm-switch/quickpanel.json 用户自定义驱动）。
//
// 轨宽 168px + 20px 字号（quickpanel 原型定稿：遥控器远距离操作，tab 名必须一眼读全，
// 「Claude Code」这类长名不截断）。sep 标记的 tab 前画虚线分隔（提示词类 / 字符类 / 动作 / 配置分组）。
import { Fragment } from "react";
import { t } from "../lib/i18n";

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
}

export default function PromptTabRail(a: Props) {
  return (
    <nav
      className="flex w-[168px] shrink-0 flex-col gap-1 overflow-y-auto border-r border-slate-800 px-2 py-2"
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
    </nav>
  );
}
