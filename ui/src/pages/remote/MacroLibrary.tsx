// 宏库（C3，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）：可视化增删保存动作序列。
//
// 一条宏 = 名称 + 遇错即停开关 + 步骤（动作或等待）。存储与执行都在 Rust 侧
// （`remote/macros.<profile>.json` + `macros::run_sequence`），这里只管编辑：
// 键槽里只存宏 id，所以改一次所有绑点同时生效；删宏由后端跨「当前映射 + 全部方案」清引用。
// 步骤录入复用槽位那套（虚拟键盘 + ActionForm），学一次两处都会。
// 状态在 macroBoard.ts（行数预算拆出），这里只剩渲染。
import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button, Toggle } from "../../components/ui";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { errText } from "../../components/QueryError";
import { IconChevron, IconClock, IconClose, IconWand, IconWorkflow } from "../../components/icons";
import { t } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";
import { ACTION_ICON, type AgentCli } from "./ActionForms";
import { FN_CATEGORIES } from "./FnPicker";
import { VirtualKeyboard } from "./VirtualKeyboard";
import type { AppEntry } from "./AppPicker";
import {
  MAX_DELAY_MS,
  STEP_KINDS,
  actionStep,
  delayStep,
  fmtMacro,
  fmtStep,
  stepCount,
  type Step,
  type StepKind,
} from "./macros";
export { useMacroBoard, type MacroBoardState } from "./macroBoard";
import type { MacroBoardState } from "./macroBoard";
import type { Macro } from "./macros";

/** 步骤行的图标：动作步骤跟着动作类型，等待步骤用表盘。 */
function StepIcon({ step }: { step: Step }) {
  if (step.type === "delay") return <IconClock className="h-3.5 w-3.5 shrink-0 text-ink-3" aria-hidden />;
  const Icon = ACTION_ICON[step.action.type];
  return <Icon className="h-3.5 w-3.5 shrink-0 text-ink-3" aria-hidden />;
}

export const paneCls = "flex min-h-0 flex-col gap-2.5 rounded-[10px] border border-dashed border-slate-600 px-3 py-2.5";

/**
 * 动作面板大框（用户 2026-11-24 定稿水平布局）：横贯左键列 + 中列。
 * 头部 = 库切换（我的动作 / 系统功能）+ 名称 + 遇错即停 + 新建；
 * 主体 = 动作列表（窄列）｜步骤列表（宽列）；底部 = 步骤类型芯片 + 录入区 + 保存行。
 * 虚拟键盘**不常驻**——选「快捷键」步骤时点「键盘」弹浮层，拼完一个组合自动收起。
 */
export function MacroBoardPane({
  b,
  onBind,
  bindTarget,
  boundCount = 0,
  className,
}: {
  b: MacroBoardState;
  onBind?: (m: Macro) => void;
  /** 录制中的槽位读数（「键名·槽名」）：绑定当前钮悬停提示它，非编辑态为空。 */
  bindTarget?: string | null;
  /** 当前映射里绑定了这条动作的键槽数：>0 时删除弹层加显式警示（用户 2026-11-24 要求）。 */
  boundCount?: number;
  className?: string;
}) {
  const m = b.shown;
  const canSave = b.dirty && !b.busy && !!m && m.name.trim() !== "" && m.steps.length > 0;
  // 芯片行的即时动作态（用户 2026-11-24 定稿：点芯片即办，不再展开通用表单）
  const [appBusy, setAppBusy] = useState(false);
  const [appErr, setAppErr] = useState<string | null>(null);
  const [cmd, setCmd] = useState("");
  const [cliApp, setCliApp] = useState<string | null>(null);
  const [cliArgs, setCliArgs] = useState("");
  // ① 名称框兼作混合检索（用户 2026-11-24）：新建动作（未保存草稿）时输入的名字
  // 实时按「名称 + 步骤摘要」过滤 ② 列表——边打边找已有动作，点一条即载入编辑；
  // 没找到就接着这个名字存新动作。已保存动作改名不触发过滤（那是编辑，不是检索）。
  const q = b.draft && b.draft.id === "" ? b.draft.name.trim().toLowerCase() : "";
  const shownList = useMemo(
    () => (q ? b.list.filter((mm) => `${mm.name} ${fmtMacro(mm)}`.toLowerCase().includes(q)) : b.list),
    [b.list, q],
  );
  // 代理 CLI 菜单数据源（与 ActionForms 同源同查询键，缓存共用）。
  const agents = useQuery({
    queryKey: ["agents-cli"],
    queryFn: () => ipc<AgentCli[]>("remote_agents_cli"),
    enabled: b.stepKind === "agent_cli" && !cliApp,
  });
  const pickKind = (k: StepKind) => {
    if (k === "open_app") {
      // 直开系统 .app 选择框，选完即落一步（参数不在这里配——步骤先落，之后可再编辑）。
      setAppBusy(true);
      setAppErr(null);
      void ipc<AppEntry | null>("remote_app_pick")
        .then((e) => e && b.addStep(actionStep({ type: "open_app", bundle_id: e.bundle_id })))
        .catch((e) => setAppErr(errText(e)))
        .finally(() => setAppBusy(false));
      return;
    }
    if (k === "shortcut") {
      // 快捷键 = 直开虚拟键盘（拼完自动收起并落一步，与旧「键盘」钮同一提交口）。
      b.setStepKind("shortcut");
      b.setKbd(true);
      return;
    }
    if (k === "agent_cli") setCliApp(null);
    // 再点当前激活的弹菜单芯片 = 收起菜单（sys / agent_cli 同款收口）。
    if ((k === "sys" || k === "agent_cli") && b.stepKind === k) {
      b.setStepKind("shortcut");
      return;
    }
    b.setStepKind(k);
  };
  const move = (i: number, d: number) => {
    if (!m) return;
    const j = i + d;
    if (j < 0 || j >= m.steps.length) return;
    const steps = [...m.steps];
    [steps[i], steps[j]] = [steps[j], steps[i]];
    b.setDraft({ ...m, steps });
    b.setDirty(true);
  };
  // 虚拟键盘浮层的 Esc 收口（注释承诺「点外面或 Esc 也收」，实现补齐）：
  // 捕获阶段先于页面级 Esc（映射退设置态），浮层开着时 Esc 只收浮层。
  // 用户 2026-11-24 追加：系统功能 / 代理 CLI 的向上弹出菜单同样要 Esc 可退——
  // 外点收口由各自的 fixed 背板负责（见弹出块），这里只管键盘。
  useEffect(() => {
    const popup = b.stepKind === "sys" || (b.stepKind === "agent_cli" && !cliApp);
    if (!b.kbd && !popup) return;
    const h = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      if (b.kbd) b.setKbd(false);
      else b.setStepKind("shortcut");
    };
    window.addEventListener("keydown", h, true);
    return () => window.removeEventListener("keydown", h, true);
  }, [b.kbd, b.stepKind, cliApp]);
  return (
    <div className={`${paneCls} relative ${className ?? ""}`}>
      {/* 头部行（用户 2026-11-24 定稿）：左格（宽 = 动作列表列）= ①「绑定当前」——
          编辑态把选中动作绑进正在录制的槽位，非编辑态回落静态「我的动作」标签；
          右格 = ③ 选中动作的名称输入（左边缘与步骤框对齐）+ 遇错即停 + 新建动作。 */}
      <div className="flex shrink-0 gap-2.5">
        <div className="w-44 shrink-0">
          {onBind ? (
            <button
              type="button"
              disabled={!m || m.id === ""}
              onClick={() => m && onBind(m)}
              data-tip={bindTarget ?? t("remote.macro.bindNoSlot")}
              className="tip flex h-full w-full items-center justify-center gap-1.5 rounded border border-[var(--key-hl)]/40 bg-[var(--sel-bg)] px-1.5 py-0.5 text-sm text-[var(--key-hl)] transition-colors hover:bg-[var(--key-hl)]/15 disabled:cursor-not-allowed disabled:border-slate-700 disabled:bg-slate-800/60 disabled:text-ink-4"
            >
              <IconWorkflow className="h-3.5 w-3.5 shrink-0" aria-hidden />
              {t("remote.macro.bindCurrent")}
            </button>
          ) : (
            <span className="flex h-full items-center justify-center rounded border border-slate-700 bg-slate-800/60 px-1.5 py-0.5 text-sm text-ink-1">
              {t("remote.macro.mineTitle")}
            </span>
          )}
        </div>
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <input
            className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-sm text-ink-1 placeholder:text-ink-4 focus:border-[var(--key-hl)]"
            placeholder={t("remote.macro.namePh")}
            aria-label={t("remote.macro.namePh")}
            value={m?.name ?? ""}
            disabled={!m}
            onChange={(e) => m && (b.setDraft({ ...m, name: e.target.value }), b.setDirty(true))}
          />
          <label className="tip flex shrink-0 items-center gap-2" data-tip={t("remote.macro.stopTip")}>
            <Toggle
              checked={m?.stop_on_error ?? true}
              disabled={!m}
              onChange={(v) => m && (b.setDraft({ ...m, stop_on_error: v }), b.setDirty(true))}
            />
            <span className="text-sm text-ink-2">{t("remote.macro.stopOnError")}</span>
          </label>
          <Button onClick={b.create} className="shrink-0">
            {t("remote.macro.new")}
          </Button>
        </div>
      </div>
      {/* 主体两列：动作列表（窄列）｜步骤列表（宽列），一起吃掉大框的弹性高度 */}
      <div className="flex min-h-0 flex-1 gap-2.5">
        <div className="w-44 shrink-0 overflow-y-auto rounded-lg border border-slate-800">
          {b.macros.isError ? (
            <p className="px-2.5 py-2 text-sm text-[var(--sem-danger-text)]">{errText(b.macros.error)}</p>
          ) : shownList.length === 0 ? (
            <p className="px-2.5 py-2 text-sm text-ink-4">
              {q ? t("remote.macro.searchEmpty") : t("remote.macro.empty")}
            </p>
          ) : (
            <ul>
              {shownList.map((mm) => {
                const on = b.shown?.id === mm.id;
                return (
                  <li key={mm.id}>
                    <button
                      type="button"
                      aria-pressed={on}
                      onClick={() => b.pick(mm)}
                      className={`flex w-full flex-col gap-0.5 border-b border-slate-800 px-2.5 py-1.5 text-left transition-colors ${
                        on ? "bg-[var(--sel-bg)] text-[var(--key-hl)]" : "hover:bg-slate-800"
                      }`}
                    >
                      <span className="truncate text-sm text-ink-1">
                        {mm.name || t("remote.macro.namePh")}
                      </span>
                      <span className="truncate text-2xs text-ink-4">{fmtMacro(mm)}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
        {/* 步骤清单：宽列，吃满主体高度 */}
        <div className="min-w-0 flex-1 overflow-y-auto rounded-lg border border-slate-800">
          {!m ? (
            <p className="px-3 py-3 text-sm text-ink-4">{t("remote.macro.pickOrCreate")}</p>
          ) : m.steps.length === 0 ? (
            <p className="px-3 py-3 text-sm text-ink-4">{t("remote.macro.noSteps")}</p>
          ) : (
            <ol>
              {m.steps.map((st, i) => (
                <li key={i} className="flex items-center gap-2 border-b border-slate-800 px-2.5 py-1.5 last:border-b-0">
                  <span className="w-5 shrink-0 text-2xs tabular-nums text-ink-4">{i + 1}</span>
                  <StepIcon step={st} />
                  <span className="min-w-0 flex-1 truncate text-sm text-ink-1">{fmtStep(st)}</span>
                  <button
                    type="button"
                    aria-label={t("remote.macro.stepUp")}
                    disabled={i === 0}
                    onClick={() => move(i, -1)}
                    className="grid h-6 w-6 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-ink-1 disabled:cursor-not-allowed disabled:text-ink-4"
                  >
                    <IconChevron className="h-3.5 w-3.5 -rotate-90" />
                  </button>
                  <button
                    type="button"
                    aria-label={t("remote.macro.stepDown")}
                    disabled={i === m.steps.length - 1}
                    onClick={() => move(i, 1)}
                    className="grid h-6 w-6 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-ink-1 disabled:cursor-not-allowed disabled:text-ink-4"
                  >
                    <IconChevron className="h-3.5 w-3.5 rotate-90" />
                  </button>
                  <button
                    type="button"
                    aria-label={t("remote.macro.stepRemove")}
                    onClick={() => (b.setDraft({ ...m, steps: m.steps.filter((_, j) => j !== i) }), b.setDirty(true))}
                    className="grid h-6 w-6 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-[var(--sem-danger-text)]"
                  >
                    <IconClose className="h-3 w-3" />
                  </button>
                </li>
              ))}
            </ol>
          )}
        </div>
      </div>
      {/* 步骤类型芯片行（用户 2026-11-24 定稿）：
          快捷键 = 直开虚拟键盘浮层（拼完自动收起并落一步，不再另设「键盘」钮和组合录入行）；
          打开应用 = 直开系统 .app 选择框，选完即落一步（参数/聚焦策略不在这里配）；
          代理 CLI = 向上弹菜单挑代理，选中才出参数行；
          系统功能 = 内置常用组合清单（调度中心/截屏…），点一条落一步；
          注入文字 / 聚焦输入框已撤出芯片行（动作序列用不上，槽位直录仍保留这两个类型）。 */}
      <div role="group" aria-label={t("remote.macro.stepKindsAria")} className="flex shrink-0 flex-wrap items-center gap-1.5">
        {STEP_KINDS.map((k) => {
          const Icon = k.id === "delay" ? IconClock : k.id === "sys" ? IconWand : ACTION_ICON[k.id as Exclude<StepKind, "sys" | "delay">];
          const on = k.id === b.stepKind;
          return (
            <button
              key={k.id}
              type="button"
              aria-pressed={on}
              disabled={k.id === "open_app" && appBusy}
              onClick={() => pickKind(k.id)}
              className={`flex items-center gap-1.5 rounded-md px-2 py-1 text-sm transition-colors disabled:cursor-wait ${
                on ? "bg-[var(--key-hl)]/15 text-[var(--key-hl)]" : "text-ink-3 hover:bg-slate-800 hover:text-ink-1"
              }`}
            >
              <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden />
              {t(k.labelKey)}
            </button>
          );
        })}
        {b.stepKind === "delay" && (
          <span className="flex items-center gap-1.5">
            <input
              type="number"
              min={0}
              max={MAX_DELAY_MS}
              step={50}
              className="w-20 rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-sm tabular-nums text-ink-1 focus:border-[var(--key-hl)]"
              aria-label={t("remote.macro.delayMs")}
              value={b.delayMs}
              onChange={(e) => b.setDelayMs(e.target.value)}
            />
            <Button onClick={() => b.addStep(delayStep(Number(b.delayMs) || 0))}>{t("remote.macro.addStep")}</Button>
          </span>
        )}
      </div>
      {appErr && <p className="shrink-0 text-sm text-[var(--sem-danger-text)]">{appErr}</p>}
      {/* 系统功能：向上弹出菜单（与代理 CLI 同款），按功能分组，点一条即落一步。
          背板兜外点（用户 2026-11-24 要求）：菜单外任意处点击即收；Esc 收口见上方 effect。 */}
      {b.stepKind === "sys" && (
        <div className="relative shrink-0">
          <div className="fixed inset-0 z-40" onClick={() => b.setStepKind("shortcut")} />
          <div className="absolute bottom-full left-0 z-50 mb-1 max-h-64 w-96 overflow-y-auto rounded-lg border border-slate-600 bg-slate-900 p-2 shadow-2xl">
            {FN_CATEGORIES.map((c) => (
              <div key={c.nameKey} className="mb-2 last:mb-0">
                <p className="mb-1 text-2xs text-ink-4">{t(c.nameKey)}</p>
                <div className="flex flex-wrap gap-1.5">
                  {c.items.map((it) => (
                    <button
                      key={it.nameKey}
                      type="button"
                      onClick={() => b.addStep(actionStep({ type: "shortcut", shortcut: it.sc }))}
                      className="rounded-md border border-slate-700 px-2 py-1 text-sm text-ink-1 transition-colors hover:border-[var(--key-hl)] hover:text-[var(--key-hl)]"
                    >
                      {t(it.nameKey)}
                    </button>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
      {/* 运行命令：一行输入 + 添加。 */}
      {b.stepKind === "run_command" && (
        <div className="flex shrink-0 items-center gap-2">
          <input
            className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-sm text-ink-1 placeholder:text-ink-4 focus:border-[var(--key-hl)]"
            placeholder={t("remote.act.cmdTip")}
            aria-label={t("remote.act.runCommand")}
            value={cmd}
            onChange={(e) => setCmd(e.target.value)}
            onKeyDown={(e) => e.stopPropagation()}
          />
          <Button
            disabled={!cmd.trim()}
            onClick={() => {
              b.addStep(actionStep({ type: "run_command", command: cmd.trim() }));
              setCmd("");
            }}
          >
            {t("remote.macro.addStep")}
          </Button>
        </div>
      )}
      {/* 代理 CLI：向上弹菜单挑代理；选中才出参数行（参数属于这一步，不是绑键）。 */}
      {b.stepKind === "agent_cli" && (
        <div className="relative shrink-0">
          {!cliApp ? (
            <>
              <div className="fixed inset-0 z-40" onClick={() => b.setStepKind("shortcut")} />
              <div className="absolute bottom-full left-0 z-50 mb-1 max-h-56 w-72 overflow-y-auto rounded-lg border border-slate-600 bg-slate-900 p-1.5 shadow-2xl">
                {agents.isPending ? (
                  <p className="px-2 py-1.5 text-sm text-ink-3">{t("remote.app.loading")}</p>
                ) : agents.isError ? (
                  <p className="px-2 py-1.5 text-sm text-[var(--sem-danger-text)]">{errText(agents.error)}</p>
                ) : (agents.data ?? []).length === 0 ? (
                  <p className="px-2 py-1.5 text-sm text-ink-4">{t("remote.act.agentCliEmpty")}</p>
                ) : (
                  <ul>
                    {(agents.data ?? []).map((a) => (
                      <li key={a.app}>
                        <button
                          type="button"
                          disabled={a.broken}
                          onClick={() => setCliApp(a.app)}
                          className="flex w-full items-baseline gap-2 rounded px-2 py-1.5 text-left hover:bg-slate-800 disabled:cursor-not-allowed disabled:text-ink-4"
                        >
                          <span className="shrink-0 text-sm text-ink-1">{a.label}</span>
                          <span className="min-w-0 truncate font-mono text-2xs text-ink-4">{a.bin}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            </>
          ) : (
            <div className="flex items-center gap-2">
              <span className="shrink-0 text-sm text-ink-1">{cliApp}</span>
              <input
                className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950 px-2.5 py-1.5 text-sm text-ink-1 placeholder:text-ink-4 focus:border-[var(--key-hl)]"
                placeholder={t("remote.act.argsPh")}
                aria-label={t("remote.act.args")}
                value={cliArgs}
                onChange={(e) => setCliArgs(e.target.value)}
                onKeyDown={(e) => e.stopPropagation()}
              />
              <Button
                onClick={() => {
                  b.addStep(actionStep({ type: "agent_cli", app: cliApp, args: cliArgs.trim() }));
                  setCliApp(null);
                  setCliArgs("");
                }}
              >
                {t("remote.macro.addStep")}
              </Button>
              <button
                type="button"
                onClick={() => setCliApp(null)}
                className="shrink-0 rounded-lg border border-slate-700 px-2 py-1 text-sm text-ink-2 transition-colors hover:border-[var(--key-hl)] hover:text-[var(--key-hl)]"
              >
                {t("remote.app.change")}
              </button>
            </div>
          )}
        </div>
      )}
      <div className="flex shrink-0 items-center gap-2">
        <Button variant="primary" disabled={!canSave} onClick={() => void b.save()}>
          {t("remote.macro.save")}
        </Button>
        {m && m.id !== "" && (
          <Button variant="danger" disabled={b.busy} onClick={() => b.setConfirmDel(true)}>
            {t("remote.macro.delete")}
          </Button>
        )}
        <span className="ml-auto shrink-0 text-2xs text-ink-4">
          {b.dirty ? t("remote.macro.unsaved") : stepCount(m?.steps.length ?? 0)}
        </span>
      </div>
      {/* 虚拟键盘浮层：拼完一个组合即收起；点外面或 Esc 也收。 */}
      {b.kbd && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => b.setKbd(false)} />
          <div className="absolute bottom-12 right-0 z-50 w-[700px] max-w-[96%] rounded-lg border border-slate-600 bg-slate-900 p-3 shadow-2xl">
            <p className="mb-2 text-sm leading-relaxed text-ink-4">{t("remote.macro.kbdHint")}</p>
            <VirtualKeyboard
              mods={b.mods}
              main={b.main}
              onMod={(bit) => b.setMods((v) => v ^ bit)}
              onMain={(kvk) => {
                b.pressMain(kvk);
                b.setKbd(false);
              }}
            />
          </div>
        </>
      )}
      <ConfirmDialog
        open={b.confirmDel}
        title={`${t("remote.macro.delete")}「${b.draft?.name ?? ""}」`}
        onClose={() => b.setConfirmDel(false)}
        onConfirm={() => void b.del()}
        danger
        irreversible
        confirmLabel={t("remote.macro.delete")}
      >
        {t("remote.macro.deleteDesc")}
        {onBind && m && m.id !== "" && boundCount > 0 && (
          <p className="mt-2 text-sm text-[var(--sem-busy-text)]">
            {t("remote.macro.deleteBoundWarn").replace("{n}", String(boundCount))}
          </p>
        )}
      </ConfirmDialog>
    </div>
  );
}
