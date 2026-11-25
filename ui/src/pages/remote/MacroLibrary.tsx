// 宏库（C3，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）：可视化增删保存动作序列。
//
// 一条宏 = 名称 + 遇错即停开关 + 步骤（动作或等待）。存储与执行都在 Rust 侧
// （`remote/macros.<profile>.json` + `macros::run_sequence`），这里只管编辑：
// 键槽里只存宏 id，所以改一次所有绑点同时生效；删宏由后端跨「当前映射 + 全部方案」清引用。
// 步骤录入复用槽位那套（虚拟键盘 + ActionForm），学一次两处都会。
// 状态在 macroBoard.ts（行数预算拆出），这里只剩渲染。
import { useMemo, useRef, useState } from "react";
import { Button, Toggle } from "../../components/ui";
import { errText } from "../../components/QueryError";
import { IconChevron, IconClock, IconClose, IconWorkflow } from "../../components/icons";
import { t } from "../../lib/i18n";
import { ACTION_ICON } from "./ActionForms";
import { MacroEditBar } from "./MacroEditBar";
import { fmtMacro, fmtStep, useMacroLabels, type Step } from "./macros";
import { StepEditor, isPresetShortcut } from "./StepEditor";
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
 * 动作面板（用户 2026-11-25 改版）：头部行（绑定当前 / 名称 / 遇错即停 / 新建 / 删除）+
 * 主体（动作列表｜步骤清单，清单底部贴添加步骤入口条 MacroEditBar）。
 */
export function MacroBoardPane({
  b,
  onBind,
  bindTarget,
  boundCount,
  className,
  bare,
}: {
  b: MacroBoardState;
  onBind?: (m: Macro) => void;
  /** 录制中的槽位读数（「键名·槽名」）：绑定当前钮悬停提示它，非编辑态为空。 */
  bindTarget?: string | null;
  /** 当前映射里绑定了这条动作的键槽数：>0 时删除弹层加显式警示（用户 2026-11-24 要求）。 */
  boundCount?: number;
  className?: string;
  /** 嵌进三页签坞时去自带虚线框（坞已给外框，别套两层）。 */
  bare?: boolean;
}) {
  const m = b.shown;
  // 双击/回车原位编辑（用户 2026-11-25）：正在编辑的步骤下标；切换动作列表项即复位。
  // 下标存在 macroBoard（不是本地 state）：底栏「运行命令」芯片加完一步要直接把它开成编辑态。
  const editIdx = b.editStep;
  const setEditIdx = b.setEditStep;
  // 定位行（点击或 ↑↓ 移动，用户 2026-11-25）：高亮当前行，回车即原位编辑。
  const [cur, setCur] = useState<number | null>(null);
  const rowRefs = useRef<(HTMLLIElement | null)[]>([]);
  const macroName = useMacroLabels();
  // 名称框双态（用户 2026-11-25 拍板）：平时是动作列表的模糊检索框（按名称 + 步骤摘要过滤）；
  // 双击左列某条动作、或点「新建动作」，才切成那条动作的名称编辑框。
  const [query, setQuery] = useState("");
  const [nameEdit, setNameEdit] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  // 未保存草稿（id 为空）= 新建动作，名称框直接归它编辑。
  const isNew = !!b.draft && b.draft.id === "";
  const nameMode = isNew || nameEdit;
  // 检索只在「非改名」态生效：改名时列表不许围著新名字跳。
  const q = nameMode ? "" : query.trim().toLowerCase();
  const shownList = useMemo(
    () => (q ? b.list.filter((mm) => `${mm.name} ${fmtMacro(mm)}`.toLowerCase().includes(q)) : b.list),
    [b.list, q],
  );
  /** 双击左列一条：选中它并把名称框切成改名态（选中文本，直接打新的）。 */
  const pickForRename = (mm: Macro) => {
    b.pick(mm);
    setEditIdx(null);
    setCur(null);
    setNameEdit(true);
    requestAnimationFrame(() => nameRef.current?.select());
  };
  const move = (i: number, d: number) => {
    if (!m) return;
    const j = i + d;
    if (j < 0 || j >= m.steps.length) return;
    const steps = [...m.steps];
    [steps[i], steps[j]] = [steps[j], steps[i]];
    b.setDraft({ ...m, steps });
    b.setDirty(true);
    setCur(j);
  };
  /**
   * 开编辑（双击或回车）：普通快捷键组合不进弹层——直接弹底栏那把浮层键盘原位改写
   * （用户 2026-11-25：一个面板里不许出现第二把键盘）；其余类型走 StepEditor。
   */
  const openEdit = (i: number) => {
    const st = m?.steps[i];
    const sc = st?.type === "action" && st.action.type === "shortcut" ? st.action.shortcut : null;
    if (sc && !isPresetShortcut(sc)) {
      b.setKbdStep(i);
      b.setKbd(true);
      return;
    }
    setEditIdx(i);
  };
  /** 步骤清单键盘导航：↑↓ 定位（带滚动），回车开编辑；编辑中不抢键。 */
  const onListKey = (e: React.KeyboardEvent) => {
    if (!m || editIdx !== null) return;
    const n = m.steps.length;
    const at = cur === null ? 0 : cur;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      e.stopPropagation();
      const j = e.key === "ArrowDown" ? Math.min(at + 1, n - 1) : Math.max(at - 1, 0);
      setCur(j);
      rowRefs.current[j]?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Enter" && n > 0) {
      e.preventDefault();
      e.stopPropagation();
      openEdit(at);
    }
  };
  return (
    <div className={`${bare ? "flex min-h-0 flex-1 flex-col gap-2.5 relative" : `${paneCls} relative`} ${className ?? ""}`}>
      {/* 头部行（用户 2026-11-24 定稿，11-25 补等高）：行高钉死 h-9，格内控件一律 h-full
          ——各块同高且垂直中对，与右侧语音坞标题行齐平。
          左格（宽 = 动作列表列）= ①「绑定当前」——编辑态把选中动作绑进正在录制的槽位，
          非编辑态回落静态「我的动作」标签；右格 = ③ 选中动作的名称输入（左边缘与步骤框对齐）+ 遇错即停。 */}
      <div className="flex h-9 shrink-0 gap-2.5">
        <div className="flex h-full w-[22rem] shrink-0 items-center gap-1.5">
          <div className="flex h-full min-w-0 flex-1">
            {onBind ? (
              <button
                type="button"
                disabled={!m || m.id === ""}
                onClick={() => m && onBind(m)}
                data-tip={bindTarget ?? t("remote.macro.bindNoSlot")}
                className="tip flex h-full w-full items-center justify-center gap-1.5 rounded border border-[var(--key-hl-text)]/40 bg-[var(--sel-bg)] px-1.5 py-0.5 text-sm text-[var(--key-hl-text)] transition-colors hover:bg-[var(--key-hl)]/15 disabled:cursor-not-allowed disabled:border-slate-700 disabled:bg-slate-800/60 disabled:text-ink-4"
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
          {/* 动作级的增/删跟列表同格（用户 2026-11-25）：它们操作的就是左边这条动作；
              右格只留名称（编辑）+ 遇错即停 + 步数。 */}
          <Button onClick={() => (b.create(), setEditIdx(null), setCur(null), setNameEdit(true))} className="h-full shrink-0">
            {t("remote.macro.new")}
          </Button>
          <Button
            variant="danger"
            disabled={!m || m.id === "" || b.busy}
            onClick={() => b.setConfirmDel(true)}
            className="h-full shrink-0"
          >
            {t("remote.macro.delete")}
          </Button>
        </div>
        <div className="flex h-full min-w-0 flex-1 items-center gap-2">
          {/* 双态名称框：检索态虚线边、改名态实边，一眼分得清现在在干什么。 */}
          <input
            ref={nameRef}
            className={`h-full min-w-0 flex-1 rounded-lg border bg-slate-950 px-2.5 text-sm text-ink-1 placeholder:text-ink-4 focus:border-[var(--key-hl-text)] ${
              nameMode ? "border-[var(--key-hl-text)]/60" : "border-dashed border-slate-600"
            }`}
            placeholder={t(nameMode ? "remote.macro.namePh" : "remote.macro.searchPh")}
            aria-label={t(nameMode ? "remote.macro.namePh" : "remote.macro.searchPh")}
            value={nameMode ? (m?.name ?? "") : query}
            disabled={nameMode && !m}
            onChange={(e) => {
              if (nameMode) {
                if (m) {
                  b.setDraft({ ...m, name: e.target.value });
                  b.setDirty(true);
                }
              } else setQuery(e.target.value);
            }}
            onKeyDown={(e) => {
              if (e.key !== "Escape") return;
              e.stopPropagation();
              if (nameMode) setNameEdit(false);
              else setQuery("");
            }}
          />
          <label className="tip flex shrink-0 items-center gap-2" data-tip={t("remote.macro.stopTip")}>
            <Toggle
              checked={m?.stop_on_error ?? true}
              disabled={!m}
              onChange={(v) => m && (b.setDraft({ ...m, stop_on_error: v }), b.setDirty(true))}
            />
            <span className="text-sm text-ink-2">{t("remote.macro.stopOnError")}</span>
          </label>
        </div>
      </div>
      {/* 主体两列：动作列表（窄列）｜步骤列表（宽列），一起吃掉大框的弹性高度 */}
      <div className="flex min-h-0 flex-1 gap-2.5">
        <div className="w-[22rem] shrink-0 overflow-y-auto rounded-lg border border-slate-800">
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
                      onClick={() => (b.pick(mm), setEditIdx(null), setCur(null), setNameEdit(false))}
                      onDoubleClick={() => pickForRename(mm)}
                      // 选中 = 有色框（用户 2026-11-25）：不再用淡底高亮条——淡底跟悬停光条长得一样，
                      // 眼睛分不出「选中的」和「鼠标正好在上面的」。ring 不吃布局，不推挤行高。
                      className={`flex w-full flex-col gap-0.5 border-b border-slate-800 px-2.5 py-1.5 text-left transition-colors ${
                        on
                          ? "relative z-10 rounded-md ring-1 ring-inset ring-[var(--key-hl-text)] text-[var(--key-hl-text)]"
                          : "hover:bg-slate-800"
                      }`}
                    >
                      <span className="truncate text-sm text-ink-1" title={mm.name || t("remote.macro.namePh")}>
                        {mm.name || t("remote.macro.namePh")}
                      </span>
                      <span className="truncate text-2xs text-ink-4" title={fmtMacro(mm)}>{fmtMacro(mm)}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
        {/* 步骤清单：宽列，吃满主体高度。整块可聚焦（listbox）：↑↓ 定位、回车原位编辑；
            清单本身只占弹性高，底部贴「添加步骤」入口条（MacroEditBar）。 */}
        <div className="flex min-w-0 flex-1 flex-col rounded-lg border border-slate-800">
          <div
            tabIndex={0}
            role="listbox"
            aria-label={t("remote.macro.stepsAria")}
            onKeyDown={onListKey}
            className="min-h-0 flex-1 overflow-y-auto"
          >
          {!m ? (
            <p className="px-3 py-3 text-sm text-ink-4">{t("remote.macro.pickOrCreate")}</p>
          ) : m.steps.length === 0 ? (
            <p className="px-3 py-3 text-sm text-ink-4">{t("remote.macro.noSteps")}</p>
          ) : (
            <ol>
              {m.steps.map((st, i) => (
                <li
                  key={i}
                  ref={(el) => {
                    rowRefs.current[i] = el;
                  }}
                  role="option"
                  aria-selected={cur === i}
                  className={`border-b border-slate-800 px-2.5 py-1.5 transition-colors last:border-b-0 hover:bg-slate-800/60 ${
                    // 定位行用有色框，悬停用淡底（与动作列表、键位行同一口径，用户 2026-11-25）。
                    cur === i ? "relative z-10 ring-1 ring-inset ring-[var(--key-hl-text)]" : ""
                  }`}
                  // 双击原位编辑（用户 2026-11-25）：按步骤类型分流（快捷键走底栏键盘，其余 StepEditor）。
                  onClick={() => setCur(i)}
                  onDoubleClick={() => openEdit(i)}
                >
                  {/* 控件右对齐（用户 2026-11-25 定稿）：↑↓/× 统一贴在行尾，位置固定
                      好点；步骤文本长到撞上行尾才省略号截断。 */}
                  <div className="flex w-full min-w-0 items-center gap-2">
                    <span className="w-5 shrink-0 text-2xs tabular-nums text-ink-4">{i + 1}</span>
                    <StepIcon step={st} />
                    {editIdx === i ? (
                      <StepEditor
                        step={st}
                        onCommit={(s) => b.replaceStep(i, s)}
                        onClose={() => b.closeEdit(i)}
                        onRemove={() => b.removeStep(i)}
                      />
                    ) : (
                      <span className="min-w-0 flex-1 truncate text-sm text-ink-1" title={fmtStep(st, macroName)}>{fmtStep(st, macroName)}</span>
                    )}
                    <button
                      type="button"
                      aria-label={t("remote.macro.stepUp")}
                      disabled={i === 0}
                      onClick={() => move(i, -1)}
                      className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-ink-1 disabled:cursor-not-allowed disabled:text-ink-4"
                    >
                      <IconChevron className="h-3.5 w-3.5 -rotate-90" />
                    </button>
                    <button
                      type="button"
                      aria-label={t("remote.macro.stepDown")}
                      disabled={i === m.steps.length - 1}
                      onClick={() => move(i, 1)}
                      className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-ink-1 disabled:cursor-not-allowed disabled:text-ink-4"
                    >
                      <IconChevron className="h-3.5 w-3.5 rotate-90" />
                    </button>
                    <button
                      type="button"
                      aria-label={t("remote.macro.stepRemove")}
                      onClick={() => (b.removeStep(i), b.setEditStep(null))}
                      className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-[var(--sem-danger-text)]"
                    >
                      <IconClose className="h-3 w-3" />
                    </button>
                  </div>
                </li>
              ))}
            </ol>
          )}
          </div>
          {/* 添加步骤入口条（芯片行 + 内联录入区 + 向上弹的键盘/菜单）：贴在清单底部。 */}
          <MacroEditBar b={b} onBind={onBind} boundCount={boundCount} />
        </div>
      </div>
    </div>
  );
}
