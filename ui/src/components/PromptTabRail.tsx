// 提示词速取面板的左侧竖排 tab 轨（自 PromptPanel 拆出，压单文件行数）。
//
// 竖排是用户明确要求：横排在代理多起来后换行三排，把列表顶到看不见。
// 轨宽 96px（用户 2026-11-19 截图：68px 会把「OpenCode / CodeBuddy」截成
// 「Open… / Code…」），文字左对齐，截不下的靠 title 读全名。
import { t } from "../lib/i18n";

/** 「命令」tab 显示短名（完整名放 title），其余去掉「Claude Code / Codex CLI」的尾缀。 */
function shortLabel(key: string, label: string): string {
  return key === "shell" ? t("prompt.tabShellShort") : label.replace(/\s+(Code|CLI)$/i, "");
}

interface Props {
  tabs: { key: string; label: string }[];
  /** 当前 tab；搜索态（q 非空）不高亮任何一档，因为看的是跨 tab 结果。 */
  tab: string;
  searching: boolean;
  onPick: (key: string) => void;
}

export default function PromptTabRail(a: Props) {
  return (
    <nav
      className="flex w-[96px] shrink-0 flex-col gap-1 overflow-y-auto border-r border-slate-800 px-1.5 py-2"
      onPointerDown={(e) => e.stopPropagation()}
    >
      {a.tabs.map((tb) => (
        <button
          key={tb.key}
          title={tb.label}
          aria-current={!a.searching && a.tab === tb.key ? "page" : undefined}
          onClick={() => a.onPick(tb.key)}
          className={`w-full truncate rounded-md px-1.5 py-1.5 text-left text-base ${
            !a.searching && a.tab === tb.key
              ? "bg-[var(--sel-bg)] text-ink-1 ring-1 ring-inset ring-[var(--sel-ring)] font-medium"
              : "text-ink-3 hover:text-ink-1"
          }`}
        >
          {shortLabel(tb.key, tb.label)}
        </button>
      ))}
    </nav>
  );
}
