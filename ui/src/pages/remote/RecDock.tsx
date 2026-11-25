// 录制态底栏键盘坞（用户 2026-11-25 拍板）：点槽位进入录制后，底栏（动作编辑 +
// 语音录入）整块被全功能虚拟键盘覆盖——直接点键拼组合填入正在录制的槽，
// 顶部三个页签切「虚拟键盘 / 我的动作 / 系统功能」，默认虚拟键盘。
// 槽位保持录制态（键帽 mousedown 不吃焦点），填完一槽点下一槽接着填，Esc/再点槽退出。
import { useState } from "react";
import { t } from "../../lib/i18n";
import { FN_CATEGORIES } from "./FnPicker";
import { asShortcut, type Action } from "./action";
import type { Macro } from "./macros";
import { fmtMacro } from "./macros";
import { VirtualKeyboard } from "./VirtualKeyboard";
import { fmtShortcut } from "./shortcut";

type Tab = "kbd" | "mine" | "sys";

export function RecDock({
  slotLabel,
  macros,
  onCommit,
}: {
  /** 正在录制的「键名·槽名」（顶行读数）。 */
  slotLabel: string;
  macros: Macro[];
  onCommit: (a: Action) => void;
}) {
  const [tab, setTab] = useState<Tab>("kbd");
  const [mods, setMods] = useState(0);
  const [main, setMain] = useState<number | null>(null);
  const [fn, setFn] = useState(false);
  // 点主键 = 组合即成：填入录制槽并清主键（修饰键按住不放，方便连绑同修饰的多个槽）。
  const pressMain = (kvk: number) => {
    if (kvk === 63) {
      setFn((v) => !v);
      return;
    }
    onCommit(asShortcut({ mods, key: kvk, ...(fn ? { fn: true } : {}) }));
    setMain(null);
  };
  const tabs: [Tab, string][] = [
    ["kbd", t("remote.map.recKbd")],
    ["mine", t("remote.macro.mineTitle")],
    ["sys", t("remote.macro.sysTitle")],
  ];
  return (
    <div className="flex min-h-0 flex-col gap-2 rounded-lg border border-slate-700 bg-slate-900 p-3">
      <div className="flex flex-wrap items-center gap-1.5">
        <div role="tablist" aria-label={t("remote.map.recKbd")} className="flex gap-1.5">
          {tabs.map(([id, label]) => (
            <button
              key={id}
              type="button"
              role="tab"
              id={`recdock-tab-${id}`}
              aria-controls="recdock-panel"
              aria-selected={tab === id}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => setTab(id)}
              className={`rounded-md px-2.5 py-1 text-sm transition-colors ${
                tab === id
                  ? "bg-[var(--key-hl)]/15 text-[var(--key-hl)]"
                  : "text-ink-3 hover:bg-slate-800 hover:text-ink-1"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
        <span className="ml-auto text-sm text-ink-3">
          {t("remote.map.recSlotNow")} <b className="text-ink-1">{slotLabel}</b>
          {main !== null && (
            <span className="ml-2 text-[var(--key-hl)]">{fmtShortcut({ mods, key: main, ...(fn ? { fn: true } : {}) })}</span>
          )}
        </span>
      </div>
      {tab === "kbd" ? (
        /* 104 键固定宽簇最小需求 ~760px：窄窗横滚兜底，小键盘不许被裁（UI-AUDIT-5 P1-5）。 */
        <div id="recdock-panel" role="tabpanel" aria-labelledby={`recdock-tab-${tab}`} className="min-h-0 flex-1 overflow-x-auto">
          <div className="w-[760px]">
            <VirtualKeyboard mods={mods} main={main} onMod={(b) => setMods((v) => v ^ b)} onMain={pressMain} />
          </div>
        </div>
      ) : tab === "mine" ? (
        <div id="recdock-panel" role="tabpanel" aria-labelledby={`recdock-tab-${tab}`} className="min-h-0 flex-1 overflow-y-auto rounded-lg border border-slate-800">
          {macros.length === 0 ? (
            <p className="px-2.5 py-2 text-sm text-ink-4">{t("remote.macro.empty")}</p>
          ) : (
            macros.map((m) => (
              <button
                key={m.id}
                type="button"
                onMouseDown={(e) => e.preventDefault()}
                onClick={() => onCommit({ type: "macro", id: m.id })}
                className="flex w-full items-baseline gap-2 border-b border-slate-800 px-2.5 py-1.5 text-left last:border-b-0 hover:bg-[var(--key-hl)]/10"
              >
                <span className="shrink-0 text-sm text-ink-1">{m.name}</span>
                <span className="tip min-w-0 flex-1 truncate text-2xs text-ink-3" data-tip={fmtMacro(m)}>{fmtMacro(m)}</span>
              </button>
            ))
          )}
        </div>
      ) : (
        <div id="recdock-panel" role="tabpanel" aria-labelledby={`recdock-tab-${tab}`} className="min-h-0 flex-1 overflow-y-auto px-1 py-1">
          {FN_CATEGORIES.map((c) => (
            <div key={c.nameKey} className="mb-2 last:mb-0">
              <p className="mb-1 text-2xs text-ink-4">{t(c.nameKey)}</p>
              <div className="flex flex-wrap gap-1.5">
                {c.items.map((it) => (
                  <button
                    key={it.nameKey}
                    type="button"
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => onCommit(asShortcut(it.sc))}
                    className="flex items-baseline gap-1.5 rounded-md border border-slate-700 px-2 py-1 text-sm text-ink-1 transition-colors hover:border-[var(--key-hl)] hover:text-[var(--key-hl)]"
                  >
                    {t(it.nameKey)}
                    {/* 组合键尾缀与 FnChips 同形（UI-AUDIT-5 P2-4）：绑前就知道绑的是什么键。 */}
                    <span className="text-2xs tabular-nums text-ink-4">{fmtShortcut(it.sc)}</span>
                  </button>
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
