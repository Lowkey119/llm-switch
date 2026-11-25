// 槽位「绑宏」选择器（C3）：可搜索的宏列表，选中即把宏 id 写进当前槽。
//
// 与 AppPicker 同口径（搜索框 + 结果列表 + 空态说明），但只读不建：
// 宏的增删改一律在卡头「宏库」里做，这里只负责挑一条来绑。
import { useMemo, useState } from "react";
import { IconSearch } from "../../components/icons";
import { errText } from "../../components/QueryError";
import { t } from "../../lib/i18n";
import { fmtMacro, stepCount, useMacros, type Macro } from "./macros";

export function MacroPicker({ onPick }: { onPick: (m: Macro) => void }) {
  const [q, setQ] = useState("");
  const macros = useMacros();
  const kw = q.trim().toLowerCase();
  const hits = useMemo(
    () =>
      kw
        ? (macros.data ?? []).filter((m) => m.name.toLowerCase().includes(kw) || fmtMacro(m).toLowerCase().includes(kw))
        : (macros.data ?? []),
    [macros.data, kw],
  );
  return (
    <div className="flex flex-col gap-2 rounded-[10px] border border-dashed border-slate-600 px-3 py-2.5">
      <label className="relative min-w-0">
        <IconSearch
          className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-ink-4"
          aria-hidden
        />
        <input
          className="w-full rounded-lg border border-slate-700 bg-slate-950 py-1.5 pl-7 pr-2.5 text-sm text-ink-1 placeholder:text-ink-4 focus:border-[var(--key-hl)]"
          placeholder={t("remote.macro.searchPlaceholder")}
          aria-label={t("remote.macro.searchAria")}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => e.stopPropagation()}
        />
      </label>
      <div className="max-h-56 min-h-24 overflow-y-auto rounded-lg border border-slate-800">
        {macros.isLoading ? (
          <p className="px-2.5 py-2 text-sm text-ink-3">{t("remote.macro.loading")}</p>
        ) : macros.isError ? (
          <p className="px-2.5 py-2 text-sm text-[var(--sem-danger-text)]">{errText(macros.error)}</p>
        ) : hits.length === 0 ? (
          <p className="px-2.5 py-2 text-sm text-ink-4">
            {kw ? t("remote.macro.noHit") : t("remote.macro.emptyGuide")}
          </p>
        ) : (
          <ul>
            {hits.map((m) => (
              <li key={m.id}>
                <button
                  type="button"
                  className="flex w-full items-baseline gap-2 border-b border-slate-800 px-2.5 py-1.5 text-left hover:bg-[var(--key-hl)]/10"
                  onClick={() => onPick(m)}
                >
                  <span className="shrink-0 text-sm text-ink-1">{m.name}</span>
                  <span className="tip min-w-0 flex-1 truncate text-2xs text-ink-3" data-tip={fmtMacro(m)}>{fmtMacro(m)}</span>
                  <span className="shrink-0 text-2xs tabular-nums text-ink-4">{stepCount(m.steps.length)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <p className="text-2xs leading-4 text-ink-4">{t("remote.macro.bindHint")}</p>
    </div>
  );
}
