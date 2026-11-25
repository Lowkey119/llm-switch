// 步骤行双击/回车原位编辑（用户 2026-11-25 拍板）：按步骤类型分流——
// 快捷键（非系统功能预设）→ 交回底栏那把浮层键盘原位改写（不再弹第二把键盘）；
// 打开应用 → 直开系统选择框；命令/文本/等待 → 行内输入（等待带 ms 单位；
// 命令清空即删掉这一步，不留空命令）；代理 CLI → 组合框挑代理，挑完把光标交给参数框；
// 系统功能（组合命中预设）→ 预设弹出菜单定位当前项；宏 → 宏选择弹层；聚焦输入框无参数，不可编辑。
import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, fieldCls } from "../../components/ui";
import { t } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";
import { errText } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { MacroPicker } from "./MacroPicker";
import { FN_CATEGORIES } from "./FnPicker";
import type { AgentCli } from "./ActionForms";
import { actionStep, type Step } from "./macros";
import type { Action } from "./action";
import { fmtShortcut, scEquals, type Shortcut } from "./shortcut";
import type { AppEntry } from "./AppPicker";

/** 组合是否命中系统功能预设（预设走菜单改，不走键盘）。 */
export const isPresetShortcut = (sc: Shortcut) =>
  FN_CATEGORIES.some((c) => c.items.some((it) => scEquals(it.sc, sc)));

/** 弹层通用壳：背板点外即收，Esc 也收（捕获阶段，先于页面级 Esc）。 */
function Popover({ onClose, children }: { onClose: () => void; children: React.ReactNode }) {
  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", h, true);
    return () => window.removeEventListener("keydown", h, true);
  }, [onClose]);
  return (
    <>
      <div className="fixed inset-0 z-40" onClick={onClose} />
      <div className="absolute bottom-full left-0 z-50 mb-1">{children}</div>
    </>
  );
}

export function StepEditor({
  step,
  onCommit,
  onClose,
  onRemove,
}: {
  step: Step;
  onCommit: (s: Step) => void;
  onClose: () => void;
  /** 删掉这一步（用户 2026-11-25：命令清空即删步，不留空命令步骤）。 */
  onRemove?: () => void;
}) {
  const toast = useToast();
  const inputRef = useRef<HTMLInputElement | HTMLTextAreaElement | null>(null);
  const action = step.type === "action" ? step.action : null;
  // 行内文本类初值
  const init =
    action?.type === "run_command" ? action.command
    : action?.type === "text" ? action.content
    : action?.type === "agent_cli" ? (action.args ?? "")
    : step.type === "delay" ? String(step.ms)
    : "";
  const [val, setVal] = useState(init);
  useEffect(() => {
    inputRef.current?.focus();
    if (inputRef.current instanceof HTMLInputElement) inputRef.current.select();
  }, []);

  const done = (s: Step) => {
    onCommit(s);
    onClose();
  };

  // —— 无编辑价值的类型提前收口 ——
  if (action?.type === "focus_input") {
    return (
      <Popover onClose={onClose}>
        <div className="rounded-lg border border-slate-600 bg-slate-900 px-3 py-2 text-sm text-ink-3 shadow-2xl">
          {t("remote.macro.stepNoEdit")}
        </div>
      </Popover>
    );
  }

  // —— 打开应用：双击即重开系统选择框（用户拍板），选完即替换 ——
  if (action?.type === "open_app") {
    void ipc<AppEntry | null>("remote_app_pick")
      .then((e) => {
        if (e) done(actionStep({ type: "open_app", bundle_id: e.bundle_id }));
        else onClose();
      })
      .catch((e) => {
        toast.err(errText(e));
        onClose();
      });
    return null;
  }

  // —— 快捷键：命中系统功能预设 → 原位组合框挑一条（用户 2026-11-25：跟代理 CLI 同一手感）——
  // 非预设的普通组合不进本组件（调用方直接弹底栏浮层键盘改写，不再弹第二把键盘）。
  if (action?.type === "shortcut") {
    const cur = action.shortcut;
    if (!isPresetShortcut(cur)) return null;
    return <SysPickRow cur={cur} done={done} onClose={onClose} />;
  }

  // —— 宏：宏选择弹层 ——
  if (action?.type === "macro") {
    return (
      <Popover onClose={onClose}>
        <div className="max-h-64 w-80 overflow-y-auto rounded-lg border border-slate-600 bg-slate-900 p-1.5 shadow-2xl">
          <MacroPicker onPick={(m) => done(actionStep({ type: "macro", id: m.id }))} />
        </div>
      </Popover>
    );
  }

  // —— 代理 CLI：组合框挑代理，挑完把光标交给参数框（用户 2026-11-25：选完代理空一格直接打参数）——
  if (action?.type === "agent_cli") {
    return <AgentCliRow action={action} done={done} onClose={onClose} />;
  }

  // —— 行内输入类：命令 / 文本 / 等待毫秒 ——
  const commitInline = () => {
    if (action?.type === "run_command") {
      // 命令清空 = 删掉这一步（用户 2026-11-25）：空命令执行时是哑步骤，没意义。
      if (!val.trim()) (onRemove ?? onClose)();
      else done(actionStep({ type: "run_command", command: val.trim() }));
      return;
    }
    if (action?.type === "text") done(actionStep({ type: "text", content: val }));
    else if (step.type === "delay") done({ type: "delay", ms: Math.max(0, Number(val) || 0) });
    else onClose();
  };
  const label = step.type === "delay" ? `${t("remote.macro.delayMs")} ` : "";
  return (
    <span
      className="flex min-w-0 flex-1 items-center gap-1.5"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
        if (e.key === "Enter" && (action?.type === "run_command" || step.type === "delay")) {
          e.stopPropagation();
          commitInline();
        }
      }}
    >
      <span className="shrink-0 text-sm text-ink-4">{label}</span>
      {action?.type === "text" ? (
        <textarea
          ref={(el) => {
            inputRef.current = el;
          }}
          rows={3}
          className="min-w-0 flex-1 rounded-lg border border-[var(--key-hl)] bg-slate-950 px-2 py-1 text-sm text-ink-1"
          value={val}
          onChange={(e) => setVal(e.target.value)}
        />
      ) : (
        <input
          ref={(el) => {
            inputRef.current = el;
          }}
          type={step.type === "delay" ? "number" : "text"}
          className="min-w-0 flex-1 rounded-lg border border-[var(--key-hl)] bg-slate-950 px-2 py-1 text-sm text-ink-1"
          value={val}
          onChange={(e) => setVal(e.target.value)}
        />
      )}
      {/* 等待带单位（用户 2026-11-25）：光秃秃一个数字看不出是毫秒还是秒。 */}
      {step.type === "delay" && <span className="shrink-0 text-sm text-ink-4">ms</span>}
      <Button size="sm" onClick={commitInline}>
        {t("remote.macro.stepEditOk")}
      </Button>
      <Button size="sm" onClick={onClose}>
        {t("common.cancel")}
      </Button>
    </span>
  );
}

/**
 * 代理 CLI 原位编辑（用户 2026-11-25 拍板）：组合框挑代理，挑完把光标交给参数框，
 * 空一格直接打参数；回车即成。代理清单与底栏同源（["agents-cli"] 缓存）。
 */
function AgentCliRow({
  action,
  done,
  onClose,
}: {
  action: Extract<Action, { type: "agent_cli" }>;
  done: (s: Step) => void;
  onClose: () => void;
}) {
  const agents = useQuery({
    queryKey: ["agents-cli"],
    queryFn: () => ipc<AgentCli[]>("remote_agents_cli"),
  });
  const [app, setApp] = useState(action.app);
  const [args, setArgs] = useState(action.args ?? "");
  const argsRef = useRef<HTMLInputElement>(null);
  // 参数框先拿焦点：芯片刚落的步骤代理已选好，用户要做的只有打参数。
  useEffect(() => argsRef.current?.focus(), []);
  const commit = () => done(actionStep({ type: "agent_cli", app, args: args.trim() }));
  const list = agents.data ?? [];
  return (
    <span
      className="flex min-w-0 flex-1 items-center gap-1.5"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
        if (e.key === "Enter") {
          e.stopPropagation();
          commit();
        }
      }}
    >
      <select
        value={app}
        aria-label={t("remote.act.agentCli")}
        onChange={(e) => {
          setApp(e.target.value);
          argsRef.current?.focus();
        }}
        className={`${fieldCls} step-select shrink-0`}
      >
        {/* 清单还没回来或本步的代理不在清单里，也得有当前值可选，否则 select 会跳回首项。 */}
        {!list.some((a) => a.app === app) && <option value={app}>{app}</option>}
        {list.map((a) => (
          <option key={a.app} value={a.app}>
            {a.label}
          </option>
        ))}
      </select>
      <input
        ref={argsRef}
        className="min-w-0 flex-1 rounded-lg border border-[var(--key-hl)] bg-slate-950 px-2 py-1 text-sm text-ink-1 placeholder:text-ink-4"
        aria-label={t("remote.act.args")}
        placeholder={t("remote.act.argsPh")}
        value={args}
        onChange={(e) => setArgs(e.target.value)}
      />
      <Button size="sm" onClick={commit}>
        {t("remote.macro.stepEditOk")}
      </Button>
      <Button size="sm" onClick={onClose}>
        {t("common.cancel")}
      </Button>
    </span>
  );
}

/**
 * 系统功能原位编辑（用户 2026-11-25：跟代理 CLI 同一形状，只是没有参数框）：
 * 组合框按分类列全部预设、打开即定位当前项，挑中即成（不再要确定/取消）；
 * Esc / 点外面取消，占位步骤会被撤掉。
 */
function SysPickRow({
  cur,
  done,
  onClose,
}: {
  cur: Shortcut;
  done: (s: Step) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLSelectElement>(null);
  useEffect(() => ref.current?.focus(), []);
  // 当前组合在预设表里的位置（`分类序号.项序号`）——组合框打开就停在它上面。
  const at = (() => {
    for (const [ci, c] of FN_CATEGORIES.entries())
      for (const [ii, it] of c.items.entries()) if (scEquals(it.sc, cur)) return `${ci}.${ii}`;
    return "";
  })();
  return (
    <span
      className="flex min-w-0 flex-1 items-center gap-1.5"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <select
        ref={ref}
        aria-label={t("remote.macro.sysPick")}
        className={`${fieldCls} step-select min-w-0 flex-1`}
        defaultValue={at}
        onChange={(e) => {
          const [ci, ii] = e.target.value.split(".").map(Number);
          const it = FN_CATEGORIES[ci]?.items[ii];
          if (it) done(actionStep({ type: "shortcut", shortcut: it.sc }));
        }}
      >
        {!at && <option value="">{t("remote.macro.sysPick")}</option>}
        {FN_CATEGORIES.map((c, ci) => (
          <optgroup key={c.nameKey} label={t(c.nameKey)}>
            {c.items.map((it, ii) => (
              <option key={it.nameKey} value={`${ci}.${ii}`}>
                {t(it.nameKey)} {fmtShortcut(it.sc)}
              </option>
            ))}
          </optgroup>
        ))}
      </select>
    </span>
  );
}
