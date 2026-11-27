import { useEffect, useRef, useState, type ReactNode } from "react";
import { IconPencil } from "./icons";

/**
 * 原位编辑（UI-AUDIT P0-6 / 决策 D1）。
 *
 * 规范（写进 docs/UI-ALIGNMENT.md §7，四处必须一致）：
 *   · 悬停显形铅笔图标 —— 「这里能改」必须看得见，双击是不可发现的肌肉记忆；
 *   · 单击进入编辑；Enter 保存、Esc 放弃、失焦 = 放弃（不写库）；
 *   · 只有值真变了才提交，误点空白不再触发一次写盘。
 *
 * 失焦为什么不保存：原先「失焦即写库」，点一下别处就多一次后端写入，
 * 端口那种还要跑占用探测 + 弹二次确认，用户根本没打算改。放弃是无害方向。
 */
export function InlineEdit({
  value,
  onSave,
  display,
  validate,
  placeholder,
  title,
  inputClassName,
  className,
  ariaLabel,
  /** 长值换行而不是截断（模型 id 这类：截断后两个近邻 id 根本分不出来）。 */
  wrap,
  /** 双击才进编辑（模型 id 这类：单击另有含义时避免误触）。默认单击。 */
  editOnDouble,
  /** 允许提交空值（模型暴露名清空 = 还原上游 id 这类语义）。默认空 = 放弃。 */
  allowEmpty,
}: {
  value: string;
  onSave: (next: string) => void;
  /** 展示用文本（默认显示 value 本身，密钥这类要显示掩码）。 */
  display?: ReactNode;
  /** 返回错误原因即拒绝提交；调用方负责把原因说出来。 */
  validate?: (next: string) => string | null;
  placeholder?: string;
  title?: string;
  inputClassName?: string;
  className?: string;
  ariaLabel?: string;
  wrap?: boolean;
  editOnDouble?: boolean;
  allowEmpty?: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement>(null);
  // 记住「进入时」的值，失焦时和它比而不是和最新 value 比——后者可能已被轮询刷新。
  const start = useRef(value);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const begin = (e: React.MouseEvent) => {
    e.stopPropagation();
    start.current = value;
    setDraft(value);
    setEditing(true);
  };

  if (!editing) {
    return (
      <span
        role="button"
        tabIndex={0}
        title={title}
        aria-label={ariaLabel ?? title}
        {...(editOnDouble
          ? { onDoubleClick: begin }
          : { onClick: begin })}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            e.stopPropagation();
            start.current = value;
            setDraft(value);
            setEditing(true);
          }
        }}
        className={`group/edit inline-flex min-w-0 cursor-pointer items-center gap-1${
          className ? ` ${className}` : ""
        }`}
      >
        <span className={wrap ? "min-w-0 break-all" : "min-w-0 truncate"}>{display ?? value}</span>
        <IconPencil
          width={11}
          height={11}
          className="shrink-0 text-ink-4 opacity-0 transition-opacity group-hover/edit:opacity-100 group-focus-within/edit:opacity-100"
        />
      </span>
    );
  }

  const finish = (commit: boolean) => {
    setEditing(false);
    if (!commit) return;
    const v = draft.trim();
    // 空值默认放弃；allowEmpty 的调用方自己定义空值语义（如清空暴露名 = 还原上游 id）。
    if ((!v && !allowEmpty) || v === start.current) return; // 没动 = 放弃
    if (validate) {
      const why = validate(v);
      if (why) return; // 调用方已在 validate 里把原因说出来
    }
    onSave(v);
  };

  return (
    <input
      ref={inputRef}
      autoFocus
      value={draft}
      placeholder={placeholder}
      aria-label={ariaLabel ?? title}
      onClick={(e) => e.stopPropagation()}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") finish(true);
        if (e.key === "Escape") finish(false);
      }}
      onBlur={() => finish(false)}
      className={`rounded border border-slate-600 bg-slate-900 px-1.5 py-0.5 text-sm text-ink-1${
        inputClassName ? ` ${inputClassName}` : ""
      }`}
    />
  );
}
