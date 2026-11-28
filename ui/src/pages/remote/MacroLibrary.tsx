// 宏库（C3，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）：可视化增删保存动作序列。
//
// 一条宏 = 名称 + 遇错即停开关 + 步骤（动作或等待）。存储与执行都在 Rust 侧
// （`remote/macros.<profile>.json` + `macros::run_sequence`），这里只管编辑：
// 键槽里只存宏 id，所以改一次所有绑点同时生效；删宏由后端做映射检查——
// 还有键槽引用就拒绝删除（先解映射，或只改名/改内容，用户 2026-11-27 口径）。
// 步骤录入复用槽位那套（虚拟键盘 + ActionForm），学一次两处都会。
// 状态在 macroBoard.ts（行数预算拆出），这里只剩渲染。
import { useRef, useState } from "react";
import { RowMenu, dropAt, moveBy, moveEnd, moveMenu, nextPick, rowMod, targetsOf } from "./listOps";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Button, Toggle } from "../../components/ui";
import { errText } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { IconChevron, IconClock, IconClose } from "../../components/icons";
import { t } from "../../lib/i18n";
import { ACTION_ICON } from "./ActionForms";
import { MacroEditBar } from "./MacroEditBar";
import { ModeSelect, type BoardMode } from "./remoteBits";
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
  boundCount,
  className,
  bare,
  mode,
  onMode,
}: {
  b: MacroBoardState;
  /** 当前映射里绑定了这条动作的键槽数：>0 时删除钮直接拦下并提示（用户 2026-11-27 口径：有映射禁删）。 */
  boundCount?: number;
  className?: string;
  /** 嵌进三页签坞时去自带虚线框（坞已给外框，别套两层）。 */
  bare?: boolean;
  /** 两档编辑的当前档与切换口（组合框在头部左格，两档共用一颗）。 */
  mode: BoardMode;
  onMode: (m: BoardMode) => void;
}) {
  const m = b.shown;
  const toast = useToast();
  // 双击/回车原位编辑（用户 2026-11-25）：正在编辑的步骤下标；切换动作列表项即复位。
  // 下标存在 macroBoard（不是本地 state）：底栏「运行命令」芯片加完一步要直接把它开成编辑态。
  const editIdx = b.editStep;
  const setEditIdx = b.setEditStep;
  // 定位行（点击或 ↑↓ 移动，用户 2026-11-25）：高亮当前行，回车即原位编辑。
  const [cur, setCur] = useState<number | null>(null);
  // 动作列表（左列）的多选 / 拖动 / 右键菜单态（计划 2026-11-25 片③）：
  // 与步骤清单同一套 listOps 口径，但选中集各管各的（行数据完全不同）。
  const [mpick, setMpick] = useState<number[]>([]);
  const [manchor, setManchor] = useState(0);
  const [mdrag, setMdrag] = useState<{ from: number[]; over: number; after: boolean } | null>(null);
  const [mmenu, setMmenu] = useState<{ x: number; y: number } | null>(null);
  const [mconfirm, setMconfirm] = useState(false);
  // 步骤批量操作（计划 2026-11-25 片④）：加选 / 整批拖动 / 右键五项，用的就是 tab 列、
  // 条目列那套 `listOps` 公共件，不另写第二套。口径与另两处不同的一点：
  // **删步骤直接生效、不弹确认框**（计划定的两档：影响面小的不问）。
  const [pick, setPick] = useState<number[]>([]);
  const [anchor, setAnchor] = useState(0);
  const [drag, setDrag] = useState<{ from: number[]; over: number; after: boolean } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const rowRefs = useRef<(HTMLLIElement | null)[]>([]);
  const macroName = useMacroLabels();
  // 名称框（用户 2026-11-26 拍板：快速面板里的搜索框去掉）：这一栏只编辑当前动作的名称，
  // 列表不再按查询过滤——浮窗/遥控器场景里搜索框要占焦点，与「方向键随时可用」相冲。
  const nameRef = useRef<HTMLInputElement>(null);
  /** 双击左列一条：选中它并把名称选中文本，直接打新的就是改名。 */
  const pickForRename = (mm: Macro) => {
    b.pick(mm);
    setEditIdx(null);
    setCur(null);
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
   * 整列步骤换掉（拖动落位 / 右键挪位 / 批量删都走这里）。
   * 顺手关掉原位编辑：`editStep` 与行内弹层都是按下标定位的，重排后不关就会指着另一条。
   */
  const setSteps = (next: Step[]) => {
    if (!m) return;
    b.setDraft({ ...m, steps: next });
    b.setDirty(true);
    b.setEditStep(null);
  };
  /** 批量删步骤：不弹框（与删 tab 内容不同档），一次删干净并清掉加选。 */
  const delSteps = (idxs: number[]) => {
    if (!m) return;
    const set = new Set(idxs);
    setSteps(m.steps.filter((_, k) => !set.has(k)));
    setPick([]);
    setCur(null);
  };
  // 右键菜单在条件渲染之外，没选中动作时就是空列（菜单也不会开）。
  const steps = m?.steps ?? [];
  // 本批要下手的下标：有加选按加选，没加选算当前定位行（与 tab 列、条目列同一口径）。
  const stepTargets = targetsOf(pick, cur ?? 0);
  // 动作列表（片③）：加选集优先，没加选就落在当前选中的那条上；挪完即清加选。
  const mTargets = targetsOf(mpick, b.list.findIndex((x) => x.id === b.shown?.id));
  const moveMacros = (next: Macro[]) => {
    void b.reorder(next);
    setMpick([]);
  };
  /**
   * 开编辑（双击或回车）：普通快捷键组合不进弹层——直接弹底栏那把浮层键盘原位改写
   * （用户 2026-11-25：一个面板里不许出现第二把键盘）；其余类型走 StepEditor。
   */
  const openEdit = (i: number) => {
    const st = m?.steps[i];
    const sc = st?.type === "action" && st.action.type === "shortcut" ? st.action.shortcut : null;
    if (sc && !isPresetShortcut(sc)) {
      b.openKbd(i);
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
          左格（宽 = 动作列表列）= 静态「我的动作」标签；右格 = 选中动作的名称输入（左边缘与步骤框对齐）+ 遇错即停。
          「绑定当前」已删（用户 2026-11-25 拍板）：做键映射时直接在键位里选动作（ActionForms 的
          MacroPicker），底栏再摆一颗是重复入口。 */}
      <div className="flex h-9 shrink-0 gap-2.5">
        <div className="flex h-full w-[22rem] shrink-0 items-center gap-1.5">
          <div className="flex h-full min-w-0 flex-1">
            <ModeSelect mode={mode} onChange={onMode} />
          </div>
          {/* 动作级的增/删跟列表同格（用户 2026-11-25）：它们操作的就是左边这条动作；
              右格只留名称（编辑）+ 遇错即停 + 步数。 */}
          <Button onClick={() => (b.create(), setEditIdx(null), setCur(null))} className="h-full shrink-0">
            {t("remote.macro.new")}
          </Button>
          <Button
            variant="danger"
            disabled={!m || m.id === "" || b.busy}
            onClick={() => {
              // 有映射禁删（用户 2026-11-27）：当前映射数得到的键槽直接拦下，
              // 面板表/方案里数不到的由后端拒绝并回原因。
              if ((boundCount ?? 0) > 0) {
                toast.err(t("remote.macro.deleteBoundWarn").replace("{n}", String(boundCount)));
                return;
              }
              b.setConfirmDel(true);
            }}
            className="h-full shrink-0"
          >
            {t("remote.macro.delete")}
          </Button>
        </div>
        <div className="flex h-full min-w-0 flex-1 items-center gap-2">
          {/* 双态名称框：检索态虚线边、改名态实边，一眼分得清现在在干什么。 */}
          <input
            ref={nameRef}
            className="h-full min-w-0 flex-1 rounded-lg border border-[var(--key-hl-text)]/60 bg-slate-950 px-2.5 text-sm text-ink-1 placeholder:text-ink-4 focus:border-[var(--key-hl-text)]"
            placeholder={t("remote.macro.namePh")}
            aria-label={t("remote.macro.namePh")}
            value={m?.name ?? ""}
            disabled={!m}
            onChange={(e) => {
              if (!m) return;
              b.setDraft({ ...m, name: e.target.value });
              b.setDirty(true);
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
          ) : b.list.length === 0 ? (
            <p className="px-2.5 py-2 text-sm text-ink-4">{t("remote.macro.empty")}</p>
          ) : (
            <ul>
              {b.list.map((mm, i) => {
                // 选中态照 tab 列口径（用户 2026-11-26 实报「批量选择没实现」）：
                // 高亮看**多选集**而非只看当前编辑动作——⌘ 加选的行也要亮，
                // 否则加选成功了却看不出来，跟没实现一样。没加选时 sel=[当前行]，行为不变。
                const sel = targetsOf(mpick, b.list.findIndex((x) => x.id === b.shown?.id));
                const on = sel.includes(i);
                const multi = sel.length > 1;
                const dropHere = mdrag && mdrag.over === i && !mdrag.from.includes(i);
                return (
                  <li
                    key={mm.id}
                    draggable
                    onDragStart={(e) => {
                      const from = sel.includes(i) ? sel : [i];
                      setMdrag({ from, over: i, after: false });
                      e.dataTransfer.effectAllowed = "move";
                      // WebKit 硬要求：dragstart 不 setData 就根本不启动拖拽（内容没人读，占位用）。
                      e.dataTransfer.setData("text/plain", from.join(","));
                    }}
                    onDragOver={(e) => {
                      e.preventDefault();
                      const r = e.currentTarget.getBoundingClientRect();
                      setMdrag((d) => (d ? { ...d, over: i, after: e.clientY > r.top + r.height / 2 } : d));
                    }}
                    onDrop={(e) => {
                      e.preventDefault();
                      if (mdrag) void b.reorder(dropAt(b.list, mdrag.from, i, mdrag.after));
                      setMdrag(null);
                      setMpick([]);
                    }}
                    onDragEnd={() => setMdrag(null)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      b.pick(mm);
                      setManchor(i);
                      setMmenu({ x: e.clientX, y: e.clientY });
                    }}
                    className={`border-b border-slate-800 ${
                      dropHere
                        ? mdrag?.after
                          ? "border-b-2 border-[var(--key-hl-text)]"
                          : "border-t-2 border-[var(--key-hl-text)]"
                        : ""
                    }`}
                  >
                    <button
                      type="button"
                      aria-pressed={on || multi}
                      onClick={(e) => {
                        const n = nextPick(mpick, manchor, i, rowMod(e));
                        setMpick(n.picked);
                        setManchor(n.anchor);
                        b.pick(mm);
                        setEditIdx(null);
                        setCur(null);
                      }}
                      onDoubleClick={() => pickForRename(mm)}
                      // 选中 = 有色框（用户 2026-11-25）：不再用淡底高亮条——淡底跟悬停光条长得一样，
                      // 眼睛分不出「选中的」和「鼠标正好在上面的」。ring 不吃布局，不推挤行高。
                      className={`flex w-full flex-col gap-0.5 border-b border-slate-800 px-2.5 py-1.5 text-left transition-colors ${
                        on
                          ? "relative z-10 rounded-md ring-1 ring-inset ring-[var(--key-hl-text)] text-[var(--key-hl-text)]"
                          : "hover:bg-slate-800"
                      }`}
                    >
                      <span className="flex w-full min-w-0 items-baseline gap-1.5">
                        <span className="shrink-0 text-2xs tabular-nums text-ink-4">{i + 1}.</span>
                        <span className="truncate text-sm text-ink-1" title={mm.name || t("remote.macro.namePh")}>
                          {mm.name || t("remote.macro.namePh")}
                        </span>
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
              {m.steps.map((st, i) => {
                const on = stepTargets.includes(i);
                const dropHere = drag && drag.over === i && !drag.from.includes(i);
                return (
                <li
                  // 可重排列表不许拿裸下标当 key（法条）：步骤本身没 id，下标 + 步骤文本已够区分。
                  key={`${i}-${fmtStep(st, macroName)}`}
                  ref={(el) => {
                    rowRefs.current[i] = el;
                  }}
                  role="option"
                  aria-selected={on}
                  draggable
                  onDragStart={(e) => {
                    const from = on ? stepTargets : [i];
                    if (!on) {
                      setPick([]);
                      setAnchor(i);
                      setCur(i);
                    }
                    setDrag({ from, over: i, after: false });
                    e.dataTransfer.effectAllowed = "move";
                    // WebKit 硬要求：dragstart 不 setData 就根本不启动拖拽（与 tab 列同一坑）。
                    e.dataTransfer.setData("text/plain", from.join(","));
                  }}
                  onDragOver={(e) => {
                    e.preventDefault();
                    const r = e.currentTarget.getBoundingClientRect();
                    setDrag((d) => (d ? { ...d, over: i, after: e.clientY > r.top + r.height / 2 } : d));
                  }}
                  onDrop={(e) => {
                    e.preventDefault();
                    if (drag) setSteps(dropAt(m.steps, drag.from, i, drag.after));
                    setDrag(null);
                    setPick([]);
                  }}
                  onDragEnd={() => setDrag(null)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    if (!on) {
                      setPick([]);
                      setAnchor(i);
                      setCur(i);
                    }
                    setMenu({ x: e.clientX, y: e.clientY });
                  }}
                  className={`border-b border-slate-800 px-2.5 py-1.5 transition-colors last:border-b-0 hover:bg-slate-800/60 ${
                    // 定位行用有色框，悬停用淡底（与动作列表、键位行同一口径，用户 2026-11-25）。
                    on ? "relative z-10 ring-1 ring-inset ring-[var(--key-hl-text)]" : ""
                  } ${dropHere ? (drag?.after ? "border-b-2 border-[var(--key-hl-text)]" : "border-t-2 border-[var(--key-hl-text)]") : ""}`}
                  // 双击原位编辑（用户 2026-11-25）：按步骤类型分流（快捷键走底栏键盘，其余 StepEditor）。
                  onClick={(e) => {
                    const n = nextPick(pick, anchor, i, rowMod(e));
                    setPick(n.picked);
                    setAnchor(n.anchor);
                    setCur(i);
                  }}
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
                      onClick={() => delSteps(pick.includes(i) ? pick : [i])}
                      className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-[var(--sem-danger-text)]"
                    >
                      <IconClose className="h-3 w-3" />
                    </button>
                  </div>
                </li>
                );
              })}
            </ol>
          )}
          </div>
          {/* 添加步骤入口条（芯片行 + 内联录入区 + 向上弹的键盘/菜单）：贴在清单底部。 */}
          <MacroEditBar b={b} />
        </div>
      </div>

      {/* 步骤右键菜单：挪位四条 + 删除（删步骤不弹确认框，计划定的口径）。
          与 tab 列、条目列同一套措辞与行为（`moveMenu` / `RowMenu`）。 */}
      {/* 动作列表右键菜单（片③）：挪位四条 + 删除，删除走二次确认（写清删几条）。
          与步骤列、tab 列同一套 `moveMenu` / `RowMenu`，不另写一份。 */}
      <RowMenu
        at={mmenu}
        onClose={() => (setMmenu(null), setMpick([]))}
        items={[
          ...moveMenu({
            top: () => moveMacros(moveEnd(b.list, mTargets, "top")),
            bottom: () => moveMacros(moveEnd(b.list, mTargets, "bottom")),
            up: () => moveMacros(moveBy(b.list, mTargets, -1)),
            down: () => moveMacros(moveBy(b.list, mTargets, 1)),
            canUp: mTargets.some((x) => x > 0),
            canDown: mTargets.some((x) => x < b.list.length - 1),
          }),
          {
            label: t("remote.macro.delete"),
            danger: true,
            onClick: () => (setMmenu(null), setMconfirm(true)),
          },
        ]}
      />
      <ConfirmDialog
        open={mconfirm}
        title={t("remote.macro.delManyTitle").replace("{n}", String(mTargets.length))}
        onClose={() => setMconfirm(false)}
        onConfirm={() => void b.delMany(mTargets.map((i) => b.list[i]?.id).filter(Boolean))}
        danger
        irreversible
        confirmLabel={t("remote.macro.delete")}
      >
        {t("remote.macro.delManyDesc")}
      </ConfirmDialog>
      <RowMenu
        at={menu}
        onClose={() => setMenu(null)}
        items={[
          ...moveMenu({
            top: () => setSteps(moveEnd(steps, stepTargets, "top")),
            bottom: () => setSteps(moveEnd(steps, stepTargets, "bottom")),
            up: () => setSteps(moveBy(steps, stepTargets, -1)),
            down: () => setSteps(moveBy(steps, stepTargets, 1)),
            canUp: stepTargets.some((x) => x > 0),
            canDown: stepTargets.some((x) => x < steps.length - 1),
          }),
          {
            label: t("remote.macro.stepsRemove"),
            danger: true,
            onClick: () => delSteps(stepTargets),
          },
        ]}
      />
    </div>
  );
}
