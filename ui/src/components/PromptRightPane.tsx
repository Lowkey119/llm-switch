// 提示词速取面板的右栏（自 PromptPanel 拆出，压单文件行数）。
//
// 一框两用（用户 2026-11-24 拍板，遥控器快速面板口径）：右栏的多行框按 mode 切换——
//   search = 搜索框：打字过滤条目（跨 tab），↑↓ 移选择、回车注入、Esc 收窗（键盘口径在 promptKeys）；
//   edit   = 编辑框：显示选中条目内容，双击/回车进入，改动防抖自动保存、失焦立即保存；
//   doc    = 技能文档只读档（用户明确要求技能内容不许在面板里改）；
//   none   = 没选中条目也不在新建，摆「—」（法条：无数据一律 —，不占假文案）。
// 抬头实时说明当前模式与按键动作——遥控器用户没有悬停提示可依赖，口径必须写在脸上。
import type { RefObject } from "react";
import { t } from "../lib/i18n";

export type RightMode = "search" | "edit" | "doc" | "none";

interface Props {
  mode: RightMode;
  /** 抬头（新建 / 编辑中 / 常态提示）。 */
  title: string;
  /** 技能文档正文（只读档）。 */
  doc: string;
  /** 搜索框现值 + 回调（mode=search 时生效）。 */
  searchValue: string;
  onSearch: (v: string) => void;
  searchRef: RefObject<HTMLTextAreaElement>;
  /** 编辑框：值 + 回调（mode=edit/doc 时生效）。 */
  value: string;
  taRef: RefObject<HTMLTextAreaElement>;
  /** 编辑框可打字（新建或编辑中）。 */
  editable: boolean;
  onBeginEdit: () => void;
  onChange: (v: string) => void;
  onBlur: () => void;
}

export default function PromptRightPane(a: Props) {
  const hint =
    a.mode === "search"
      ? t("prompt.rightSearchHint")
      : a.mode === "edit"
        ? t("prompt.rightEditHint")
        : null;
  return (
    <div
      className="flex w-[340px] shrink-0 flex-col gap-1.5 border-l border-slate-800 px-2.5 pb-2"
      onPointerDown={(e) => e.stopPropagation()}
    >
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-lg text-ink-2">{a.title}</span>
        {hint && <span className="shrink-0 text-base text-ink-4">{hint}</span>}
      </div>
      {a.mode === "search" ? (
        <textarea
          ref={a.searchRef}
          autoFocus
          value={a.searchValue}
          placeholder={t("prompt.search")}
          onChange={(e) => a.onSearch(e.target.value)}
          className="min-h-0 flex-1 select-text resize-none rounded-md bg-slate-800/60 px-2.5 py-2 text-xl leading-relaxed text-ink-1 placeholder:text-ink-3 ring-1 ring-inset ring-[var(--sel-ring)]"
        />
      ) : a.mode === "doc" ? (
        <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-all rounded-md bg-slate-800/60 px-2.5 py-2 text-base leading-relaxed text-ink-2">
          {a.doc}
        </pre>
      ) : a.mode === "none" ? (
        <div className="flex min-h-0 flex-1 items-center justify-center text-lg text-ink-4">—</div>
      ) : (
        <textarea
          ref={a.taRef}
          readOnly={!a.editable}
          value={a.value}
          onDoubleClick={a.onBeginEdit}
          onChange={(e) => a.onChange(e.target.value)}
          onBlur={a.onBlur}
          className={`min-h-0 flex-1 select-text resize-none rounded-md px-2.5 py-2 text-xl leading-relaxed ${
            a.editable
              ? "bg-slate-800/60 text-ink-1 ring-1 ring-inset ring-[var(--sel-ring)]"
              : "bg-slate-800/40 text-ink-2"
          }`}
        />
      )}
    </div>
  );
}
