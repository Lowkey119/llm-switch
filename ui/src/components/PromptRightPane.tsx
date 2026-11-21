// 提示词速取面板的右栏（自 PromptPanel 拆出，压单文件行数）。
//
// 口径（用户 2026-11-19 三条拍板）：常驻显示当前选中条目的内容——不论在不在编辑态
// 都实时跟随选中项；双击进编辑，改动防抖自动保存、失焦立即保存，没有手动「保存 / 取消」。
// 技能条目只读显示文档（用户明确要求技能内容不许在面板里改），走 `doc`；
// 没有选中条目也没在新建时摆「—」（法条：无数据一律 —，不占假文案）。
import type { RefObject } from "react";

interface Props {
  /** 抬头：新建 / 技能文档 / 编辑中 / 常态提示。 */
  title: string;
  /** 技能文档正文（只读档）；非技能条目传 null。 */
  doc: string | null;
  /** 既没选中条目也不在新建。 */
  empty: boolean;
  /** 可编辑态（新建或双击进入编辑）。 */
  editable: boolean;
  value: string;
  taRef: RefObject<HTMLTextAreaElement>;
  onBeginEdit: () => void;
  onChange: (v: string) => void;
  onBlur: () => void;
}

export default function PromptRightPane(a: Props) {
  return (
    <div
      className="flex w-[260px] shrink-0 flex-col gap-1 border-l border-slate-800 px-2.5 pb-2"
      onPointerDown={(e) => e.stopPropagation()}
    >
      <span className="text-sm text-ink-3">{a.title}</span>
      {a.doc !== null ? (
        <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-all rounded-md bg-slate-800/60 px-2 py-1.5 text-2xs leading-relaxed text-ink-2">
          {a.doc}
        </pre>
      ) : a.empty ? (
        <div className="flex min-h-0 flex-1 items-center justify-center text-sm text-ink-4">—</div>
      ) : (
        <textarea
          ref={a.taRef}
          readOnly={!a.editable}
          value={a.value}
          onDoubleClick={a.onBeginEdit}
          onChange={(e) => a.onChange(e.target.value)}
          onBlur={a.onBlur}
          className={`min-h-0 flex-1 select-text resize-none rounded-md px-2 py-1.5 text-sm leading-relaxed ${
            a.editable
              ? "bg-slate-800/60 text-ink-1 ring-1 ring-inset ring-[var(--sel-ring)]"
              : "bg-slate-800/40 text-ink-2"
          }`}
        />
      )}
    </div>
  );
}
