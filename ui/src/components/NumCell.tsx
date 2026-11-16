// 带钳制的数值输入（fix:ui-trust-and-form-exit）：设置页路由/熔断表单专用形态，
// 独立成文件也守住「单文件 ≤550 行」门禁（计划 §6.1）。
import { useEffect, useRef, useState } from "react";
import { FieldLabel, inputCls } from "./ui";

/**
 * 原实现清空输入框瞬间 Number("")=0 写进表单并标脏，越界值原样落盘。
 * 这里字段自持原始字符串：合法才回写表单；空/非法/越界就地标红并禁保存；
 * 「放弃修改」等外部复位（value 变了且不是本字段推的）自动收回显示。
 */
export function NumCell({
  label,
  hint,
  value,
  min,
  max,
  step,
  disabled,
  onCommit,
  onValidity,
}: {
  label: string;
  hint?: string;
  value: number;
  min: number;
  max?: number;
  step?: string;
  disabled?: boolean;
  onCommit: (n: number) => void;
  onValidity: (bad: boolean) => void;
}) {
  const [raw, setRaw] = useState<string | null>(null);
  const lastPush = useRef(value);
  useEffect(() => {
    if (value !== lastPush.current) {
      lastPush.current = value;
      setRaw(null);
    }
  }, [value]);
  const shown = raw ?? String(value);
  const n = Number(shown);
  const bad =
    shown.trim() === "" || !Number.isFinite(n) || n < min || (max !== undefined && n > max);
  useEffect(() => onValidity(bad), [bad]); // eslint-disable-line react-hooks/exhaustive-deps
  const msg =
    shown.trim() === "" ? "必填" : `取值范围：${min} ~ ${max ?? "∞"}`;
  return (
    <FieldLabel label={label} hint={hint}>
      <input
        type="number"
        min={min}
        max={max}
        step={step}
        disabled={disabled}
        value={shown}
        aria-invalid={bad || undefined}
        onChange={(e) => {
          const v = e.target.value;
          setRaw(v);
          const nn = Number(v);
          if (v.trim() !== "" && Number.isFinite(nn) && nn >= min && (max === undefined || nn <= max)) {
            lastPush.current = nn;
            onCommit(nn);
          }
        }}
        className={`${inputCls}${bad ? " border-rose-500/70" : ""}`}
      />
      {bad && (
        <span role="alert" className="text-[var(--sem-danger-text)]">
          {msg}
        </span>
      )}
    </FieldLabel>
  );
}
