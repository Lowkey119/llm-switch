// 快速面板档（计划 2026-11-25 ③④⑤b）：把动作面板那套骨架搬来编辑 `quickpanel.json`——
// 左列（原动作列表）= tab 列表，右列（原步骤清单）= 该 tab 的内容条目，
// 底部芯片行 = 往当前 tab 加一条内容。
//
// 芯片口径（用户 2026-11-26 拍板）：只留「我的动作 / 编辑内容」两颗——快捷键/打开应用/
// 代理 CLI/运行命令四颗撤销，这些执行形态通过「我的动作」引用现成动作即可达成。
// 「我的动作」落的是**引用**（`{macro:id}`，改动作同步生效），不是副本。
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { RowMenu, dropAt, moveBy, moveEnd, moveMenu, nextPick, rowMod, targetsOf } from "./listOps";
import { Button, Toggle, barSelectCls } from "../../components/ui";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { errText } from "../../components/QueryError";
import { IconHandPointUp, IconKeyboard, IconTextType, IconWorkflow } from "../../components/icons";
import { t } from "../../lib/i18n";
import { MacroPicker } from "./MacroPicker";
import { paneCls } from "./MacroLibrary";
import { ModeSelect, type BoardMode } from "./remoteBits";
import { isPresetShortcut } from "./StepEditor";
import { VirtualKeyboard } from "./VirtualKeyboard";
import { fmtStep, useMacroLabels } from "./macros";
import { fmtShortcut } from "./shortcut";
import { itemKind, useQuickPanelBoard, type QpItem } from "./quickpanel";
import { qpNote } from "./QpItemEditor";
import { QpItemList } from "./QpItemList";
import { useVirtualKeyboard } from "./useVirtualKeyboard";
import { RemoteImage } from "./RemoteImage";
import { useKeyFlash } from "./useKeyFlash";
import { hasRemoteImage } from "./keys";

/** 芯片行（用户 2026-11-26 拍板）：三颗——① 编辑内容 ② 我的动作 ③ 快捷键。
 *  快捷键芯片与动作编辑面板同一把虚拟键盘，但录入结果落成**字符**（`⌃C`）进条目正文，
 *  不是生成动作条目（计划 2026-11-26 §三.5）；键盘收起后行内编辑接着开，能在后面继续打字。 */
const QP_CHIPS = [
  { id: "text", labelKey: "remote.qp.addContent", Icon: IconTextType },
  { id: "macro", labelKey: "remote.qp.addMacro", Icon: IconWorkflow },
  { id: "shortcut", labelKey: "remote.act.shortcut", Icon: IconKeyboard },
] as const;

export function QuickPanelBoard({
  mode,
  onMode,
  className,
  bare,
  profileId,
}: {
  mode: BoardMode;
  onMode: (m: BoardMode) => void;
  className?: string;
  /** 嵌进底栏带时去自带虚线框（坞已给外框，别套两层）。 */
  bare?: boolean;
  /** 当前遥控器档案：右侧实物图与热点跟它切。 */
  profileId?: string | null;
}) {
  const b = useQuickPanelBoard();
  const macroName = useMacroLabels();
  // 只读图：跟随实体键事件亮热点，不点击、不改映射。
  const readOnlyRef = useRef(false);
  const { live, held, after } = useKeyFlash({ editRef: readOnlyRef, selectKey: () => {} });
  const [editIdx, setEditIdx] = useState<number | null>(null);
  const [cur, setCur] = useState<number | null>(null);
  // 虚拟键盘：与动作面板共用同一套状态机（useVirtualKeyboard），只提供「录入结果落到哪」的回调。
  // at 有值 = 改写已有的动作条目（照旧落 action）；at=null = 快捷键芯片新增，
  // 按计划 §三.5 落成**字符**（`⌃C`）进正文，不生成动作条目；按住动作是按键状态不是文本，落动作条目。
  const kbdState = useVirtualKeyboard((action, at) => {
    if (at !== null) {
      b.setItem(at, { action });
      return;
    }
    if (action.type === "shortcut" && action.shortcut.hold) {
      b.addItem({ action });
      return;
    }
    addChip({ t: action.type === "shortcut" ? fmtShortcut(action.shortcut) : "" });
  });
  const { kbd, mods } = kbdState;
  const [pop, setPop] = useState<{ at: number | null } | null>(null);
  const [confirmDel, setConfirmDel] = useState(false);
  // 多选：picked = ctrl/⌘ 加选的下标集（空 = 没加选，操作落在当前 tab 上）；
  // anchor = shift 连选的锚点。右键菜单与拖拽都吃这套选中集。
  const [picked, setPicked] = useState<number[]>([]);
  const [anchor, setAnchor] = useState(0);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [drag, setDrag] = useState<{ from: number[]; over: number; after: boolean } | null>(null);
  // 条目列的另一套多选/拖动/右键状态（片②）：与 tab 列同一套做法、同一套手感，但各存各的，
  // 切 tab 就清掉条目加选（否则下标指的是另一个 tab 的行）。
  const [ipick, setIpick] = useState<number[]>([]);
  const [ianchor, setIanchor] = useState(0);
  const [idrag, setIdrag] = useState<{ from: number[]; over: number; after: boolean } | null>(null);
  const [imenu, setImenu] = useState<{ x: number; y: number } | null>(null);
  const [iconfirm, setIconfirm] = useState(false);
  useEffect(() => setIpick([]), [b.sel]);
  // 当前 tab 被别处挪走（键盘导航、增删、自动落盘回落）时，加选集必须跟着作废：
  // 否则高亮停在旧行、右栏却是另一个 tab（用户实报「切到 1 号自己跳到 2 号」）。
  useEffect(() => {
    if (picked.length && !picked.includes(b.sel)) {
      setPicked([]);
      setAnchor(b.sel);
    }
  }, [b.sel, picked]);

  const tab = b.shown;
  // 条目列的「当前行」兜底：刚切 tab 时 cur 可能是 null，批量操作按第 1 行算。
  const curItem = cur ?? 0;
  const editable = b.editable;
  const readOnlyNote = tab ? t("remote.qp.readonly").replace("{type}", tab.type) : "";
  // 待删清单（多选删除的二次确认要说清删掉哪些、共几条内容——危险操作不能只写「确定吗」）。
  const delList = targetsOf(picked, b.sel)
    .map((i) => b.list[i])
    .filter(Boolean);
  const delNames =
    delList.length > 3
      ? `${delList
          .slice(0, 3)
          .map((x) => x.label)
          .join("、")}${t("remote.qp.delMore").replace("{n}", String(delList.length))}`
      : delList.map((x) => x.label).join("、");
  const delItems = delList.reduce((n, x) => n + (x.items?.length ?? 0), 0);

  /** 条目读数（无数据一律 —，不摆占位文案）。 */
  const title = (it: QpItem) => {
    switch (itemKind(it)) {
      case "macro":
        return `${t("remote.qp.macroRef")} ${macroName(it.macro ?? "")}`;
      case "action":
        return fmtStep({ type: "action", action: it.action! }, macroName);
      case "cmd":
        return it.t?.trim() || it.cmd || "—";
      case "text":
        // 备注字段已取消（计划 2026-11-26「备注字段与去重」）：JSON 的 `n` 保留兼容老配置但不读不写。
        return it.t || "—";
      default:
        return "—";
    }
  };

  /** 追加一条并把光标交给它（芯片的统一出口）。
   *  非预设快捷键条目不开行内编辑：它从键盘录完即完整，行内也没得可编辑
   *  （开了只会让 ItemEditor 空渲染，历史上这里还诱发了渲染期回调死循环）。 */
  const addChip = (it: QpItem) => {
    const at = b.addItem(it);
    if (at !== null) {
      const bareShortcut = it.action?.type === "shortcut" && !isPresetShortcut(it.action.shortcut);
      if (!bareShortcut) setEditIdx(at);
      setCur(at);
    }
  };

  const pickChip = (id: (typeof QP_CHIPS)[number]["id"]) => {
    if (id === "text") return addChip({ t: "" });
    if (id === "macro") return setPop({ at: null });
    if (id === "shortcut") return kbdState.open(); // 与动作面板同一把键盘、同一套添加机制
  };

  /** 双击/回车原位编辑：按种类分流（普通快捷键交回底栏那把键盘，不弹第二把）。 */
  const openEdit = (i: number) => {
    const it = b.items[i];
    if (!it || !editable) return;
    const a = it.action;
    if (itemKind(it) === "macro") return setPop({ at: i });
    if (a?.type === "shortcut" && !isPresetShortcut(a.shortcut)) return kbdState.open(i);
    setEditIdx(i);
  };

  /** 收起编辑：刚加的那条没填出内容就撤掉，库里不许留半成品。 */
  const closeEdit = (i: number) => {
    setEditIdx(null);
    if (b.items[i] && itemKind(b.items[i]) === null) b.removeItem(i);
  };

  // pop 浮层的 Esc 收口（虚拟键盘的 Esc 已由 useVirtualKeyboard 统一处理）。
  useEffect(() => {
    if (!pop) return;
    const h = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setPop(null);
    };
    window.addEventListener("keydown", h, true);
    return () => window.removeEventListener("keydown", h, true);
  }, [pop]);

  return (
    <div className={`${bare ? "flex min-h-0 flex-1 flex-col gap-2.5 relative" : `${paneCls} relative`} ${className ?? ""}`}>
      {/* 头部行：与动作面板同款 h-9 等高栅格，左格宽 = tab 列表列，右格与内容列左边缘对齐。 */}
      <div className="flex h-9 shrink-0 gap-2.5">
        <div className="flex h-full w-[22rem] shrink-0 items-center gap-1.5">
          <div className="flex h-full min-w-0 flex-1">
            <ModeSelect mode={mode} onChange={onMode} />
          </div>
          <Button onClick={b.addTab} className="h-full shrink-0">
            {t("remote.qp.newTab")}
          </Button>
          <Button variant="danger" disabled={!tab || b.busy} onClick={() => setConfirmDel(true)} className="h-full shrink-0">
            {t("remote.qp.delTab")}
          </Button>
        </div>
        <div className="flex h-full min-w-0 flex-1 items-center gap-2">
          <input
            className="h-full min-w-0 flex-1 rounded-lg border border-slate-600 bg-slate-950 px-2.5 text-sm text-ink-1 placeholder:text-ink-4 focus:border-[var(--key-hl-text)]"
            placeholder={t("remote.qp.tabName")}
            aria-label={t("remote.qp.tabName")}
            value={tab?.label ?? ""}
            disabled={!tab}
            onChange={(e) => tab && b.patchTab(b.sel, { label: e.target.value })}
          />
          <select
            aria-label={t("remote.qp.type")}
            className={`h-full shrink-0 cursor-pointer ${barSelectCls}`}
            value={tab?.type ?? "chars"}
            disabled={!tab}
            onChange={(e) => tab && b.patchTab(b.sel, { type: e.target.value })}
          >
            {["chars", "actions", "prompts", "skills", "config"].map((ty) => (
              <option key={ty} value={ty}>
                {t(`remote.qp.type.${ty}` as never)}
              </option>
            ))}
          </select>
          <label className="tip flex shrink-0 items-center gap-2" data-tip={t("remote.qp.sepTip")}>
            <Toggle checked={tab?.sep ?? false} disabled={!tab} onChange={(v) => tab && b.patchTab(b.sel, { sep: v })} />
            <span className="text-sm text-ink-2">{t("remote.qp.sep")}</span>
          </label>
        </div>
      </div>

      {/* 主体两列：tab 列表（窄列）｜内容清单（宽列 + 底部芯片行），与动作面板同构。 */}
      <div className="flex min-h-0 flex-1 gap-2.5">
        <div className="w-[22rem] shrink-0 overflow-y-auto rounded-lg border border-slate-800">
          {b.cfg.isError ? (
            <p className="px-2.5 py-2 text-sm text-[var(--sem-danger-text)]">{errText(b.cfg.error)}</p>
          ) : b.cfg.isLoading ? (
            <p className="px-2.5 py-2 text-sm text-ink-4">—</p>
          ) : b.list.length === 0 ? (
            <p className="px-2.5 py-2 text-sm text-ink-4">{t("remote.qp.emptyTab")}</p>
          ) : (
            <ul
              aria-label={t("remote.qp.tabsAria")}
              onKeyDown={(e) => {
                // ↑↓ 在 tab 列里换 tab（遥控器与不用鼠标的人也得能走）；
                // 菜单键/上下文菜单键在焦点行上弹菜单，与鼠标右键同一套。
                const n = b.list.length;
                if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                  e.preventDefault();
                  const j = e.key === "ArrowDown" ? Math.min(b.sel + 1, n - 1) : Math.max(b.sel - 1, 0);
                  setPicked([]);
                  setAnchor(j);
                  b.setSel(j);
                } else if (e.key === "ContextMenu") {
                  e.preventDefault();
                  const r = (e.target as HTMLElement).getBoundingClientRect();
                  setMenu({ x: r.left, y: r.bottom });
                }
              }}
            >
              {b.list.map((tb, i) => {
                const sel = targetsOf(picked, b.sel);
                const on = sel.includes(i);
                const dropHere = drag && drag.over === i && !drag.from.includes(i);
                return (
                  <li
                    key={`${tb.key}-${i}`}
                    draggable
                    onDragStart={(e) => {
                      const from = sel.includes(i) ? sel : [i];
                      if (!sel.includes(i)) {
                        setPicked([]);
                        setAnchor(i);
                        b.setSel(i);
                      }
                      setDrag({ from, over: i, after: false });
                      e.dataTransfer.effectAllowed = "move";
                      // WebKit 的硬要求：dragstart 里不 setData 就**根本不启动拖拽**
                      // （没有载荷它直接放弃，表现就是「拖不动」）。内容没人读，占位用。
                      e.dataTransfer.setData("text/plain", from.join(","));
                    }}
                    onDragOver={(e) => {
                      e.preventDefault();
                      const r = e.currentTarget.getBoundingClientRect();
                      setDrag((d) => (d ? { ...d, over: i, after: e.clientY > r.top + r.height / 2 } : d));
                    }}
                    onDrop={(e) => {
                      e.preventDefault();
                      if (drag) {
                        b.setList(dropAt(b.list, drag.from, i, drag.after));
                        setPicked([]);
                      }
                      setDrag(null);
                    }}
                    onDragEnd={() => setDrag(null)}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      if (!sel.includes(i)) {
                        setPicked([]);
                        setAnchor(i);
                        b.setSel(i);
                      }
                      setMenu({ x: e.clientX, y: e.clientY });
                    }}
                    className={`flex items-center border-b border-slate-800 ${
                      on ? "rounded-md ring-1 ring-inset ring-[var(--key-hl-text)]" : ""
                    } ${dropHere ? (drag?.after ? "border-b-2 border-[var(--key-hl-text)]" : "border-t-2 border-[var(--key-hl-text)]") : ""}`}
                  >
                    <button
                      type="button"
                      aria-pressed={b.sel === i}
                      onClick={(e) => {
                        const n = nextPick(picked, anchor, i, rowMod(e));
                        setPicked(n.picked);
                        setAnchor(n.anchor);
                        b.setSel(i);
                        setEditIdx(null);
                        setCur(null);
                      }}
                      className={`flex min-w-0 flex-1 flex-col gap-0.5 px-2.5 py-1.5 text-left transition-colors ${
                        b.sel === i ? "text-[var(--key-hl-text)]" : "hover:bg-slate-800"
                      }`}
                    >
                      <span className="flex w-full min-w-0 items-baseline gap-1.5">
                        <span className="shrink-0 text-2xs tabular-nums text-ink-4">{i + 1}.</span>
                        <span className="truncate text-sm text-ink-1" title={tb.label || t("remote.qp.tabName")}>
                          {tb.label || t("remote.qp.tabName")}
                        </span>
                        <span className="ml-auto shrink-0 text-2xs tabular-nums text-ink-4">
                          {tb.items?.length ?? 0}
                        </span>
                      </span>
                      <span className="truncate text-2xs text-ink-4" title={tb.key}>
                        {t(`remote.qp.type.${tb.type}` as never)}
                        {qpNote(tb.type)}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <div className="flex min-w-0 flex-1 flex-col rounded-lg border border-slate-800">
          <div tabIndex={0} role="listbox" aria-label={t("remote.qp.itemsAria")} className="min-h-0 flex-1 overflow-y-auto">
            {!tab ? (
              <p className="px-2.5 py-2 text-sm text-ink-4">—</p>
            ) : b.items.length === 0 ? (
              <p className="px-2.5 py-2 text-sm text-ink-4">{editable ? t("remote.qp.emptyItems") : readOnlyNote}</p>
            ) : (
              <QpItemList
                items={b.items}
                editable={editable}
                cur={cur}
                editIdx={editIdx}
                pick={ipick}
                anchor={ianchor}
                drag={idrag}
                menuAt={imenu}
                title={title}
                onPick={(next, a, i) => (setIpick(next), setIanchor(a), setCur(i))}
                onDrag={setIdrag}
                onMenu={setImenu}
                onReorder={b.setItems}
                onMove={b.moveItem}
                onSetItem={b.setItem}
                onDrop={(from, target, after) => b.setItems(dropAt(b.items, from, target, after))}
                onOpenEdit={openEdit}
                onCloseEdit={closeEdit}
                onAskDelete={(i) => (setIpick([]), setCur(i), setIconfirm(true))}
                confirmOpen={iconfirm}
                onConfirmToggle={setIconfirm}
                onConfirmDelete={() => (b.delItems(targetsOf(ipick, curItem)), setIpick([]))}
                path={b.cfg.data?.path ?? "quickpanel.json"}
              />
            )}
          </div>
          {/* 芯片行（只读 tab 收起，改不动的东西不给入口）：贴在清单底部，弹层一律向上开。 */}
          {editable && (
            <div className="relative flex w-full shrink-0 flex-wrap items-center gap-1.5 border-t border-slate-800 px-2 py-1.5">
              <span
                className="tip tip-up shrink-0 pr-0.5 text-ink-4"
                data-tip={t("remote.qp.addItemHint")}
                role="img"
                aria-label={t("remote.qp.addItemHint")}
              >
                <IconHandPointUp className="h-4 w-4" />
              </span>
              {QP_CHIPS.map((c) => (
                <button
                  key={c.id}
                  type="button"
                  onClick={() => pickChip(c.id)}
                  className="flex shrink-0 items-center gap-1.5 rounded-md border border-slate-700 bg-slate-950 px-2 py-1 text-sm text-ink-2 transition-colors hover:border-slate-500 hover:text-ink-1"
                >
                  <c.Icon className="h-3.5 w-3.5 shrink-0" aria-hidden />
                  {t(c.labelKey as never)}
                </button>
              ))}
              <span className="ml-auto flex shrink-0 items-center gap-1.5">
                <span className="shrink-0 text-sm font-bold tabular-nums text-ink-1">
                  {b.dirty ? t("remote.macro.unsaved") : t("remote.qp.count").replace("{n}", String(b.items.length))}
                </span>
              </span>
              {pop && (
                <>
                  <div className="fixed inset-0 z-40" onClick={() => setPop(null)} />
                  <div className="absolute bottom-full left-0 z-50 mb-1">
                    <div className="max-h-64 w-80 overflow-y-auto rounded-lg border border-slate-600 bg-slate-900 p-1.5 shadow-2xl">
                      <MacroPicker
                        onPick={(m) => {
                          // 芯片开的 = 追加一条引用；双击宏行开的 = 换掉那一条（不新增）。
                          if (pop.at !== null) b.setItem(pop.at, { macro: m.id });
                          else addChip({ macro: m.id });
                          setPop(null);
                        }}
                      />
                    </div>
                  </div>
                </>
              )}
            </div>
          )}
        </div>

        {/* 右侧只读实物图：按高度等比缩放，按键时热点圈联动，不点击不改映射。 */}
        {hasRemoteImage(profileId) && (
          <div className="flex w-[9rem] shrink-0 items-start justify-center overflow-hidden rounded-lg border border-slate-800">
            <RemoteImage
              profileId={profileId}
              edit={false}
              sel={null}
              hover={null}
              live={live}
              held={held}
              after={after}
              onSelect={() => {}}
              onHover={() => {}}
              className="h-full"
              imgClassName="h-full w-auto"
            />
          </div>
        )}
      </div>

      {/* 虚拟键盘：与动作面板同一把（向上弹、水平居中，104 键窄窗口横滚不缩键）。 */}
      {kbd && createPortal(
        <>
          <div className="fixed inset-0 z-40" onClick={kbdState.close} />
          <div className="fixed bottom-3 left-1/2 z-50 max-h-[70%] w-[1180px] max-w-[calc(100%-1.5rem)] -translate-x-1/2 overflow-auto rounded-lg border border-slate-600 bg-slate-900 p-3 shadow-2xl">
            {kbd.at !== null && (
              <p className="mb-2 text-sm text-[var(--key-hl-text)]">
                {t("remote.macro.kbdRewrite").replace("{n}", String(kbd.at + 1))}
              </p>
            )}
            <div className="min-w-[1120px]">
              <VirtualKeyboard mods={mods} main={kbdState.main} holdMod={kbdState.holdMod} onMod={kbdState.modClick} onMain={kbdState.stageMain} onCommit={kbdState.commit} />
            </div>
          </div>
        </>,
        document.body,
      )}

      {/* 右键菜单：挪位四条 + 删除（删除走同一个二次确认，不另开一条路）。 */}
      <RowMenu
        at={menu}
        onClose={() => setMenu(null)}
        items={[
          ...moveMenu({
            top: () => b.setList(moveEnd(b.list, targetsOf(picked, b.sel), "top")),
            bottom: () => b.setList(moveEnd(b.list, targetsOf(picked, b.sel), "bottom")),
            up: () => b.setList(moveBy(b.list, targetsOf(picked, b.sel), -1)),
            down: () => b.setList(moveBy(b.list, targetsOf(picked, b.sel), 1)),
            canUp: targetsOf(picked, b.sel).some((x) => x > 0),
            canDown: targetsOf(picked, b.sel).some((x) => x < b.list.length - 1),
          }),
          { label: t("remote.qp.delTab"), danger: true, onClick: () => setConfirmDel(true) },
        ]}
      />

      <ConfirmDialog
        open={confirmDel}
        title={`${t("remote.qp.delTab")}「${delNames}」`}
        onClose={() => setConfirmDel(false)}
        onConfirm={() => (b.delTabs(targetsOf(picked, b.sel)), setPicked([]), setConfirmDel(false))}
        danger
        irreversible
        confirmLabel={t("remote.qp.delTab")}
      >
        {t("remote.qp.delTabDesc")
          .replace("{n}", String(delItems))
          .replace("{path}", b.cfg.data?.path ?? "quickpanel.json")}
      </ConfirmDialog>
    </div>
  );
}

/** 只读类型在 tab 行上标一句原因（状态不只靠颜色，法条）。 */
