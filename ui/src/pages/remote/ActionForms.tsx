// 槽位动作录入（C2）：动作类型分段控件 + 非快捷键动作的表单。
//
// FnPicker 面板左栏用它换掉虚拟键盘：选「打开应用/文字/聚焦输入框/运行命令」就出对应表单，
// 提交口与虚拟键盘同一个（父层 setSlotAction），所以目标槽、红闪、乐观写回全部共用一份。
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { errText } from "../../components/QueryError";
import { Button } from "../../components/ui";
import { ipc } from "../../lib/ipc";
import {
  IconAppWindow,
  IconCheck,
  IconFocusInput,
  IconKeyboard,
  IconPencil,
  IconTerminal,
  IconTextType,
  IconWorkflow,
} from "../../components/icons";
import { t } from "../../lib/i18n";
import { ACTION_TYPES, type Action, type ActionType } from "./action";
import { AppPicker, type AppEntry } from "./AppPicker";
import { AppFocusEditor } from "./AppFocusEditor";
import { MacroPicker } from "./MacroPicker";

/** `remote_agents_cli` 的一行（与 Rust `AgentCliEntry` 对齐）。 */
export interface AgentCli {
  app: string;
  label: string;
  bin: string;
  bin_path: string | null;
  version: string | null;
  broken: boolean;
  probed: boolean;
}

/** 动作类型 → 图标（分段控件与宏的步骤行共用一份，别处不许再画第二套）。 */
export const ACTION_ICON: Record<ActionType, typeof IconKeyboard> = {
  shortcut: IconKeyboard,
  open_app: IconAppWindow,
  text: IconTextType,
  focus_input: IconFocusInput,
  run_command: IconTerminal,
  agent_cli: IconTerminal,
  macro: IconWorkflow,
};

const TAB_ICON = ACTION_ICON;

/** 动作类型分段控件（法条：模式切换用分段控件，不用一堆按钮）。 */
export function ActionTypeTabs({ value, onChange }: { value: ActionType; onChange: (t: ActionType) => void }) {
  return (
    <div
      role="group"
      aria-label={t("remote.act.typesAria")}
      className="mb-4 flex flex-wrap items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-950 p-1"
    >
      {ACTION_TYPES.map((a) => {
        const Icon = TAB_ICON[a.id];
        const on = a.id === value;
        return (
          <button
            key={a.id}
            type="button"
            aria-pressed={on}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => onChange(a.id)}
            className={`flex items-center gap-1.5 rounded-md px-2.5 py-1 text-sm transition-colors ${
              on
                ? "bg-[var(--key-hl)]/15 text-[var(--key-hl-text)]"
                : "text-ink-3 hover:bg-slate-800 hover:text-ink-1"
            }`}
          >
            <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden />
            {t(a.labelKey)}
          </button>
        );
      })}
    </div>
  );
}

/** 一行录入：标签 + 输入 + 提交钮（空白不许提交，避免存一条按了没反应的动作）。 */
function OneLiner({
  labelKey,
  tipKey,
  placeholder,
  mono,
  multiline,
  commitKey,
  onCommit,
}: {
  labelKey: string;
  tipKey: string;
  placeholder: string;
  mono?: boolean;
  multiline?: boolean;
  commitKey: string;
  onCommit: (raw: string) => void;
}) {
  const [raw, setRaw] = useState("");
  const blank = raw.trim() === "";
  const submit = () => {
    if (blank) return;
    onCommit(raw.trim());
    setRaw("");
  };
  const field = `min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-sm text-ink-1 placeholder:text-ink-4 focus:border-[var(--key-hl-text)] ${
    mono ? "font-mono text-xs" : ""
  }`;
  return (
    // 撑满所在窗格：注入文字是「大段内容」，框高必须跟着窗格走，
    // 固定一小块（旧 h-16）等于逼用户在小框里数着字数写提示词。
    <div className="flex h-full min-h-0 flex-col gap-2 rounded-[10px] border border-dashed border-slate-600 px-3 py-2.5">
      <span className="shrink-0 text-sm font-semibold text-ink-1">{t(labelKey)}</span>
      {multiline ? (
        <textarea
          className={`${field} min-h-[120px] flex-1 resize-none leading-5`}
          placeholder={placeholder}
          aria-label={t(labelKey)}
          value={raw}
          onChange={(e) => setRaw(e.target.value)}
          onKeyDown={(e) => {
            // 回车即提交（与槽位直录同手感）；Shift+回车换行。
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              e.stopPropagation();
              submit();
            }
          }}
        />
      ) : (
        <input
          className={field}
          placeholder={placeholder}
          aria-label={t(labelKey)}
          value={raw}
          onChange={(e) => setRaw(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              e.stopPropagation();
              submit();
            }
          }}
        />
      )}
      <button
        type="button"
        className="tip flex items-center justify-center gap-1.5 self-start rounded-lg border border-slate-600 px-2.5 py-1 text-sm text-ink-1 transition-colors hover:border-[var(--key-hl-text)] hover:text-[var(--key-hl-text)] disabled:cursor-not-allowed disabled:text-ink-4"
        disabled={blank}
        data-tip={t(tipKey)}
        onClick={submit}
      >
        <IconCheck className="h-3.5 w-3.5 shrink-0" aria-hidden />
        {t(commitKey)}
      </button>
    </div>
  );
}

/**
 * 「打开应用」表单：选择器 + 该 App 的聚焦策略（C6）。
 * 选中即写槽（与虚拟键盘同一个提交口），随后收成一行读数，下方接着配「打开后聚焦」——
 * 多数输入类 App 起来后光标不在输入框里，这两步本来就该一起做。
 */
function OpenAppForm({ onCommit }: { onCommit: (a: Action) => void }) {
  const [picked, setPicked] = useState<AppEntry | null>(null);
  // 启动参数（用户 2026-11-23 要求「浏览对话框自己找的应用也能输入参数」）：
  // 选完不再立即提交，参数填完点提交才落槽——空参数就是原来的行为。
  const [args, setArgs] = useState("");
  const pick = (e: AppEntry) => {
    setPicked(e);
    setArgs("");
  };
  if (!picked) return <AppPicker onPick={pick} />;
  return (
    <>
      <div className="flex min-w-0 items-center gap-2 rounded-[10px] border border-dashed border-slate-600 px-3 py-2">
        <IconAppWindow className="h-4 w-4 shrink-0 text-ink-3" aria-hidden />
        <span className="min-w-0 flex-1 truncate text-sm text-ink-1" title={picked.name}>{picked.name}</span>
        <span className="max-w-[38%] shrink-0 truncate font-mono text-xs text-ink-4" title={picked.bundle_id}>
          {picked.bundle_id}
        </span>
        <button
          type="button"
          className="flex shrink-0 items-center gap-1 rounded-lg border border-slate-600 px-2 py-1 text-sm text-ink-2 transition-colors hover:border-[var(--key-hl-text)] hover:text-[var(--key-hl-text)]"
          onClick={() => setPicked(null)}
        >
          <IconPencil className="h-3 w-3 shrink-0" aria-hidden />
          {t("remote.app.change")}
        </button>
      </div>
      <ArgRow
        labelKey="remote.act.args"
        tipKey="remote.act.argsTip"
        value={args}
        onChange={setArgs}
        onCommit={() =>
          onCommit({ type: "open_app", bundle_id: picked.bundle_id, args: args.trim() })
        }
      />
      <AppFocusEditor bundleId={picked.bundle_id} appName={picked.name} />
    </>
  );
}

/** 一行「标签 + 输入 + 提交」的参数输入（启动参数 / CLI 参数 / 工作目录共用）。 */
function ArgRow({
  labelKey,
  tipKey,
  value,
  onChange,
  onCommit,
  mono,
}: {
  labelKey: string;
  tipKey?: string;
  value: string;
  onChange: (v: string) => void;
  onCommit: () => void;
  mono?: boolean;
}) {
  return (
    <label className={`tip flex min-w-0 items-center gap-2 rounded-[10px] border border-dashed border-slate-600 px-3 py-2 ${tipKey ? "" : "pointer-events-auto"}`}>
      <span className="shrink-0 text-sm text-ink-3">{t(labelKey)}</span>
      <input
        className={`min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-sm text-ink-1 placeholder:text-ink-4 focus:border-[var(--key-hl-text)] ${mono ? "font-mono text-xs" : ""}`}
        placeholder={t("remote.act.argsPh")}
        aria-label={t(labelKey)}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            e.stopPropagation();
            onCommit();
          }
        }}
      />
      <Button variant="primary" onClick={onCommit}>{t("remote.act.commit")}</Button>
    </label>
  );
}

/** 代理 CLI 动作：选一个装好的代理 CLI + 参数 + 工作目录（用户 2026-11-23 要求）。 */
function AgentCliForm({ onCommit }: { onCommit: (a: Action) => void }) {
  const q = useQuery({
    queryKey: ["agents-cli"],
    queryFn: () => ipc<AgentCli[]>("remote_agents_cli"),
    staleTime: 60_000,
  });
  const [app, setApp] = useState("");
  const [args, setArgs] = useState("");
  const [cwd, setCwd] = useState("");
  const rows = (q.data ?? []).filter((a) => !!a.bin_path);
  const ready = rows.length > 0;
  return (
    <div className="flex min-w-0 flex-col gap-2 rounded-[10px] border border-dashed border-slate-600 px-3 py-2.5">
      <span className="text-sm font-semibold text-ink-1">{t("remote.act.agentCli")}</span>
      {q.isError ? (
        <p className="text-sm text-[var(--sem-danger-text)]">{errText(q.error)}</p>
      ) : !ready ? (
        <p className="text-sm text-ink-4">{t(q.data && q.data.some((a) => a.probed) ? "remote.act.agentNone" : "remote.act.agentLoading")}</p>
      ) : (
        <>
          <select
            className="w-full rounded-lg border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-sm text-ink-1 focus:border-[var(--key-hl-text)]"
            aria-label={t("remote.act.agentCli")}
            value={app}
            onChange={(e) => setApp(e.target.value)}
          >
            <option value="">{t("remote.act.agentPick")}</option>
            {rows.map((a) => (
              <option key={a.app} value={a.app}>
                {a.label}
                {a.version ? ` · ${a.version}` : ""}
                {a.broken ? ` · ${t("remote.act.agentBroken")}` : ""}
              </option>
            ))}
          </select>
          <ArgRow
            labelKey="remote.act.args"
            value={args}
            onChange={setArgs}
            onCommit={() => app && onCommit({ type: "agent_cli", app, args: args.trim(), cwd: cwd.trim() })}
          />
          <ArgRow
            labelKey="remote.act.cwd"
            value={cwd}
            onChange={setCwd}
            onCommit={() => app && onCommit({ type: "agent_cli", app, args: args.trim(), cwd: cwd.trim() })}
            mono
          />
        </>
      )}
    </div>
  );
}

/**
 * 非快捷键动作的表单（`shortcut` 类型由 FnPicker 的虚拟键盘负责，不走这里）。
 * 聚焦输入框没有参数，所以它就是一个直接提交的钮。
 */
export function ActionForm({
  type,
  onCommit,
}: {
  type: Exclude<ActionType, "shortcut">;
  onCommit: (a: Action) => void;
}) {
  if (type === "focus_input") {
    return (
      <button
        type="button"
        className="tip flex w-full items-center justify-center gap-2 rounded-[10px] border border-dashed border-slate-600 px-3 py-3 text-sm text-ink-1 transition-colors hover:border-[var(--key-hl-text)] hover:text-[var(--key-hl-text)]"
        data-tip={t("remote.act.focusTip")}
        onClick={() => onCommit({ type: "focus_input" })}
      >
        <IconFocusInput className="h-4 w-4 shrink-0" aria-hidden />
        {t("remote.act.focusCommit")}
      </button>
    );
  }
  if (type === "agent_cli") return <AgentCliForm onCommit={onCommit} />;
  if (type === "open_app") {
    // 应用选择器（C7）+ 打开后聚焦（C6）：选中即写入当前槽。
    return <OpenAppForm onCommit={onCommit} />;
  }
  if (type === "macro") {
    // 宏选择器（C3）：列表来自宏库，选中即把宏 id 写进当前槽（步骤不副本）。
    return <MacroPicker onPick={(m) => onCommit({ type: "macro", id: m.id })} />;
  }
  if (type === "text") {
    return (
      <OneLiner
        labelKey="remote.act.text"
        tipKey="remote.act.textTip"
        placeholder={t("remote.act.textPlaceholder")}
        multiline
        commitKey="remote.act.commit"
        onCommit={(content) => onCommit({ type: "text", content })}
      />
    );
  }
  return (
    <OneLiner
      labelKey="remote.act.runCommand"
      tipKey="remote.act.cmdTip"
      placeholder="open -a Calculator"
      mono
      commitKey="remote.act.commit"
      onCommit={(command) => onCommit({ type: "run_command", command })}
    />
  );
}
