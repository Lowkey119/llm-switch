// 底栏的注入目标组合框（计划 2026-11-26 §三.1）：底部状态栏的提示文本换成常驻目标选择。
// 点它弹运行中 App 列表（带代理的置顶）→ 选一个设为目标（鼠标通道）。
// 红闪 = 无目标或目标不收注入（flash 由面板门控点亮）。
// 快捷键通道（用户 2026-11-27 拍板）：**⌘⇧T** 弹出/收起目标列表——组合本身是定义，
// 绑到哪个遥控器键由用户自定义动作完成；列表打开期间方向键挪高亮 + OK(Enter) 确认。
import { useEffect, useRef, useState } from "react";
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
  const btnRef = useRef<HTMLButtonElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [pop, setPop] = useState<{ x: number; y: number } | null>(null);
  // 高亮行（方向键选）；打开时落到当前目标，没有目标落第一行。
  const [hi, setHi] = useState(0);
  // PointPopover 外点用 mousedown 关闭：再点触发钮会「关了又开」，记关闭时刻挡回弹。
  const closedAt = useRef(0);
  const appsQ = useQuery({
    queryKey: ["prompt-apps"],
    queryFn: () => ipc<AppRow[]>("prompt_apps"),
    enabled: !!pop,
    staleTime: 2000,
  });
  const apps = appsQ.data ?? [];

  const openAt = () => {
    const r = btnRef.current?.getBoundingClientRect();
    if (!r) return;
    // 锚在按钮上沿：底栏贴窗底，PointPopover 量到下方放不下会自动向上翻（"向上弹出"）。
    setHi(Math.max(0, apps.findIndex((a) => a.name === target?.name)));
    setPop({ x: r.left, y: r.top - 4 });
  };
  const close = () => {
    closedAt.current = Date.now();
    setPop(null);
  };

  // ⌘⇧T：弹出/收起目标列表（组合定义见文件头；遥控器把它绑到哪颗键由用户自定义）。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || !e.shiftKey || e.key.toLowerCase() !== "t") return;
      e.preventDefault();
      if (listRef.current) {
        close();
        return;
      }
      openAt();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target?.name, apps]);

  // 高亮行滚进可视区（列表 max-h 内滚动）。
  useEffect(() => {
    listRef.current?.querySelector(`[data-idx="${hi}"]`)?.scrollIntoView({ block: "nearest" });
  }, [hi, apps.length]);

  const pick = (pid: number) => {
    setPop(null);
    void ipc("prompt_target_set", { pid })
      .then(onPicked)
      .catch(() => {});
  };

  // 列表弹开时本容器接键盘：方向键挪高亮、Enter(OK) 确认。preventDefault 拦住
  // 焦点按钮的原生 Enter 激活，保证确认的总是高亮那一行。
  const onPopKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (!apps.length) return;
      setHi((i) => (i + (e.key === "ArrowDown" ? 1 : apps.length - 1)) % apps.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const a = apps[hi];
      if (a) pick(a.pid);
    }
  };

  return (
    <>
      <button
        ref={btnRef}
        // flex-1：组合框吃满底栏剩余宽度（用户 2026-11-27：尽可能宽）。
        className={`min-w-0 flex-1 truncate rounded-md px-2 py-0.5 text-left text-lg ring-1 ring-inset ring-slate-700 hover:bg-slate-700/60 ${
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
          if (Date.now() - closedAt.current < 300) return;
          if (pop) {
            close();
            return;
          }
          const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
          setHi(Math.max(0, apps.findIndex((a) => a.name === target?.name)));
          setPop({ x: r.left, y: r.top - 4 });
        }}
      >
        {target ? `${t("prompt.targetTo")} ${target.name}` : t("prompt.targetPickHint")}
        <IconChevron className="ml-1 inline h-3 w-3 rotate-90" aria-hidden="true" />
      </button>
      {pop && (
        <PointPopover point={pop} onClose={close}>
          <div
            ref={listRef}
            className={`${popoverPanelCls} max-h-64 w-[min(480px,86vw)] overflow-y-auto`}
            role="listbox"
            aria-label={t("prompt.appsTitle")}
            onKeyDown={onPopKey}
          >
            {apps.map((a, i) => (
              <button
                key={a.pid}
                data-idx={i}
                className={`${popoverItemCls} text-lg ${i === hi ? "bg-slate-700 text-ink-1" : ""}`}
                role="option"
                aria-selected={a.name === target?.name}
                onClick={() => pick(a.pid)}
              >
                <span className="truncate">{a.name}</span>
                {a.agent && <span className="ml-2 shrink-0 text-2xs text-ink-4">{a.agent}</span>}
              </button>
            ))}
            {!appsQ.isLoading && apps.length === 0 && (
              <div className="px-2.5 py-1 text-sm text-ink-4">{t("prompt.appsEmpty")}</div>
            )}
          </div>
        </PointPopover>
      )}
    </>
  );
}
