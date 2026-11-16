import { cloneElement, useId, type ReactElement, type ReactNode } from "react";

/**
 * 表单字段（UI-AUDIT P1-3）：label 与控件真实关联，错误挂在控件下方。
 *
 * 此前满屏是「placeholder 当标签」——一旦填了值就没人知道这一栏是什么，
 * 读屏也念不出来。这里用 useId 生成 id 注入控件，label 走 htmlFor 真关联，
 * hint / error 用 aria-describedby 挂上，读屏进框就能听到「是什么 + 注意什么 + 哪儿错了」。
 */
export function Field({
  label,
  hint,
  error,
  className,
  children,
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  className?: string;
  /** 单个表单控件（input / select / textarea）。 */
  children: ReactElement<{ id?: string; "aria-describedby"?: string }>;
}) {
  const base = useId();
  const ctrlId = `${base}-c`;
  const describedBy = [hint ? `${base}-h` : null, error ? `${base}-e` : null]
    .filter(Boolean)
    .join(" ");

  const control = cloneElement(children, {
    id: children.props.id ?? ctrlId,
    "aria-describedby": describedBy || undefined,
  });

  return (
    <div className={`min-w-0${className ? ` ${className}` : ""}`}>
      <label
        htmlFor={children.props.id ?? ctrlId}
        className="mb-1 block text-xs font-medium text-ink-2"
      >
        {label}
      </label>
      {control}
      {hint && !error && (
        <p id={`${base}-h`} className="mt-1 text-xs text-ink-4">
          {hint}
        </p>
      )}
      {error && (
        <p id={`${base}-e`} className="mt-1 text-xs text-[var(--sem-danger-text)]">
          {error}
        </p>
      )}
    </div>
  );
}
