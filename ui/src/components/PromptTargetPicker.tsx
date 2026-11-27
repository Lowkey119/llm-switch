// 底栏的注入目标组合框（计划 2026-11-26 §三.1）：底部状态栏的提示文本换成常驻目标选择。
// 点它弹运行中 App 列表（带代理的置顶）→ 选一个设为目标（鼠标通道）。
// 红闪 = 无目标或目标不收注入（flash 由面板门控点亮）。
import { useState } from "react";
import { PointPopover, popoverItemCls, popoverPanelCls } from "./Popover";
import { IconChevron } from "./icons";
import { t } from "../lib/i18n";
import { ipc } from "../lib/ipc";
import { useQuery } from "@tanstack/react-query";

interface AppRow {
  pid: number;
  name: string;
  agent: string | null;
}

interface Props {
  target: { name: string; usable: boolean } | null;
  flash: boolean;
  onPicked: () => void;
}

export default function PromptTargetPicker({ target, flash, onPicked }: Props) {
  const [pop, setPop] = useState<{ x: number; y: number } | null>(null);
  const appsQ = useQuery({
    queryKey: ["prompt-apps"],
    queryFn: () => ipc<AppRow[]>("prompt_apps"),
    enabled: !!pop,
    staleTime: 2000,
  });

  const pick = (pid: number) => {
    setPop(null);
    void ipc("prompt_target_set", { pid })
      .then(onPicked)
      .catch(() => {});
  };

  return (
    <>
      <button
        className={`shrink-0 rounded-md px-2 py-0.5 text-lg ring-1 ring-inset ring-slate-700 hover:bg-slate-700/60 ${
          !target
            ? "text-[var(--sem-danger-text)]"
            : target.usable
              ? "text-ink-2"
              : "text-[var(--sem-busy-text)]"
        } ${flash ? "animate-pulse bg-[var(--sem-danger)]/20 font-semibold ring-[var(--sem-danger)]" : ""}`}
        title={t("prompt.targetTip")}
        aria-haspopup="listbox"
        aria-expanded={!!pop}
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
          setPop({ x: r.left, y: r.bottom + 4 });
        }}
      >
        {target ? `${t("prompt.targetTo")} ${target.name}` : t("prompt.targetPickHint")}
        <IconChevron className="ml-1 inline h-3 w-3 rotate-90" aria-hidden="true" />
      </button>
      {pop && (
        <PointPopover point={pop} onClose={() => setPop(null)}>
          <div className={`${popoverPanelCls} max-h-64 overflow-y-auto`} role="listbox" aria-label={t("prompt.appsTitle")}>
            {(appsQ.data ?? []).map((a) => (
              <button key={a.pid} className={popoverItemCls} role="option" aria-selected={a.name === target?.name} onClick={() => pick(a.pid)}>
                <span className="truncate">{a.name}</span>
                {a.agent && <span className="ml-2 shrink-0 text-2xs text-ink-4">{a.agent}</span>}
              </button>
            ))}
            {!appsQ.isLoading && (appsQ.data ?? []).length === 0 && (
              <div className="px-2.5 py-1 text-sm text-ink-4">{t("prompt.appsEmpty")}</div>
            )}
          </div>
        </PointPopover>
      )}
    </>
  );
}
