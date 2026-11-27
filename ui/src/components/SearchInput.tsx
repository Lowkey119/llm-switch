import { useEffect, useRef, useState } from "react";
import { IconClose, IconSearch } from "./icons";
import { t } from "../lib/i18n";

/**
 * 统一搜索框（UI-AUDIT P1-9）：放大镜 + 一键清空。
 *
 * 命中计数不在这里：U6 起「命中 n / 总数」由 `components/FilterBar.tsx` 一处说，
 * 搜索框只管输入，免得同一个数在一条过滤行里出现两种长相。
 *
 * `debounce` 只给真正打后端的过滤用（日志搜索每敲一个字就重查一次既费又闪），
 * 本地过滤保持即时——给纯前端过滤加延迟只会让人觉得输入卡。
 */
export function SearchInput({
  value,
  onChange,
  placeholder,
  ariaLabel,
  debounce = 0,
  inputClassName,
  mono,
}: {
  value: string;
  onChange: (v: string) => void;
  /** 必须写清搜的是什么范围，别只写「搜索…」。 */
  placeholder: string;
  ariaLabel?: string;
  /** >0 时延迟提交毫秒数（用于会触发请求的过滤）。 */
  debounce?: number;
  inputClassName?: string;
  mono?: boolean;
}) {
  const [draft, setDraft] = useState(value);
  const timer = useRef<number | null>(null);

  // 外部清空（切 tab、点重置）要能倒灌进来，否则框里留着上一次的词。
  useEffect(() => setDraft(value), [value]);
  useEffect(
    () => () => {
      if (timer.current) window.clearTimeout(timer.current);
    },
    [],
  );

  const input = (v: string) => {
    setDraft(v);
    if (timer.current) window.clearTimeout(timer.current);
    if (debounce > 0) {
      timer.current = window.setTimeout(() => onChange(v), debounce);
    } else {
      onChange(v);
    }
  };

  const clear = () => {
    if (timer.current) window.clearTimeout(timer.current);
    setDraft("");
    onChange("");
  };

  return (
    <span className="inline-flex items-center gap-1.5">
      <span className="relative inline-flex items-center">
        <IconSearch
          width={12}
          height={12}
          className="pointer-events-none absolute left-2 text-ink-4"
        />
        <input
          value={draft}
          placeholder={placeholder}
          aria-label={ariaLabel ?? placeholder}
          onChange={(e) => input(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") clear();
          }}
          className={`rounded-md border border-slate-700 bg-slate-800/60 py-1 pl-7 pr-6 text-sm text-ink-2 placeholder:text-ink-4 focus-visible:border-cyan-500/60${
            mono ? " font-mono" : ""
          }${inputClassName ? ` ${inputClassName}` : ""}`}
        />
        {draft && (
          <button
            onClick={clear}
            aria-label={t("common.clearSearch")}
            className="absolute right-1 rounded p-0.5 text-ink-4 transition-colors hover:text-ink-2"
          >
            <IconClose width={11} height={11} />
          </button>
        )}
      </span>
    </span>
  );
}
