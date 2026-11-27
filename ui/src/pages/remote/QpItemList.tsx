// 快速面板「条目列」（计划 2026-11-25 片②，从 QuickPanelBoard 拆出——那个文件超了行数预算）。
//
// 口径与 tab 列完全一致（用户 2026-11-25）：序号 + 点选 / ⌘点击加选 / Shift 连选 + 整批拖动 +
// 右键五项 + 删除走二次确认。这里只管**行怎么长、手势怎么认**，选中集与落盘都留在父组件，
// 免得把面板的状态形状绑死在这个视图上。
import { RowMenu, moveBy, moveEnd, moveMenu, nextPick, rowMod, targetsOf } from "./listOps";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import {
  IconChevron,
  IconClose,
  IconTerminal,
  IconTextType,
  IconWorkflow,
} from "../../components/icons";
import { t } from "../../lib/i18n";
import { ACTION_ICON } from "./ActionForms";
import { itemKind, type QpItem } from "./quickpanel";
import { ItemEditor } from "./QpItemEditor";

/** 行图标：跟着条目种类走，与动作面板的步骤行同一口径。 */
function ItemIcon({ item }: { item: QpItem }) {
  const Icon =
    itemKind(item) === "macro" ? IconWorkflow
    : itemKind(item) === "action" ? ACTION_ICON[item.action!.type]
    : itemKind(item) === "cmd" ? IconTerminal
    : IconTextType;
  return <Icon className="h-3.5 w-3.5 shrink-0 text-ink-3" aria-hidden />;
}

export type QpItemDrag = { from: number[]; over: number; after: boolean };

export function QpItemList({
  items,
  editable,
  cur,
  editIdx,
  pick,
  anchor,
  drag,
  title,
  menuAt,
  onPick,
  onReorder,
  onDrag,
  onMenu,
  onMove,
  onSetItem,
  onDrop,
  onOpenEdit,
  onCloseEdit,
  onAskDelete,
  onConfirmDelete,
  confirmOpen,
  onConfirmToggle,
  path,
}: {
  items: QpItem[];
  editable: boolean;
  /** 右键菜单位置（null = 没开）。 */
  menuAt: { x: number; y: number } | null;
  /** 当前行（刚切 tab 时可能还没选，批量操作按第 1 行算）。 */
  cur: number | null;
  /** 正在编辑的行（null = 没在编辑）。 */
  editIdx: number | null;
  /** ⌘/Shift 加选的下标集（空 = 没加选，操作落在当前行）。 */
  pick: number[];
  anchor: number;
  drag: QpItemDrag | null;
  title: (it: QpItem) => string;
  onPick: (next: number[], anchor: number, i: number) => void;
  onDrag: (d: QpItemDrag | null) => void;
  /** 右键菜单开在哪（null = 收）。 */
  onMenu: (at: { x: number; y: number } | null) => void;
  /** 右键菜单的挪位结果：子件算好新顺序，父组件写回。 */
  onReorder: (next: QpItem[]) => void;
  onMove: (i: number, d: number) => void;
  onSetItem: (i: number, it: QpItem) => void;
  /** 整批拖动落位：父组件算好新顺序写回。 */
  onDrop: (from: number[], target: number, after: boolean) => void;
  onOpenEdit: (i: number) => void;
  onCloseEdit: (i: number) => void;
  /** 行上垃圾桶：先问一声（与右键删除同一个确认框，不另开一条路）。 */
  onAskDelete: (i: number) => void;
  onConfirmDelete: () => void;
  confirmOpen: boolean;
  onConfirmToggle: (open: boolean) => void;
  /** quickpanel.json 的路径，确认框里写清删的是哪份文件。 */
  path: string;
}) {
  const curItem = cur ?? 0;
  const targets = targetsOf(pick, curItem);
  return (
    <>
      <ol>
        {items.map((it, i) => {
          const sel = targetsOf(pick, curItem);
          const on = sel.includes(i);
          const dropHere = drag && drag.over === i && !drag.from.includes(i);
          return (
            <li
              // key 带上正文：列表现在能整批拖动重排，光用下标会让 React 就地复用错行
              // （法条：可重排列表不许拿数组下标当 key）；条目本身没有 id 字段，
              // 下标 + 正文已够区分，拖动后重挂一行比复用错行便宜。
              key={`${i}-${title(it)}`}
              draggable={editable}
              onDragStart={(e) => {
                const from = sel.includes(i) ? sel : [i];
                if (!sel.includes(i)) {
                  onPick([], i, i);
                }
                onDrag({ from, over: i, after: false });
                e.dataTransfer.effectAllowed = "move";
                // WebKit 硬要求：dragstart 不 setData 就根本不启动拖拽（与 tab 列同一坑）。
                e.dataTransfer.setData("text/plain", from.join(","));
              }}
              onDragOver={(e) => {
                e.preventDefault();
                const r = e.currentTarget.getBoundingClientRect();
                onDrag(drag ? { ...drag, over: i, after: e.clientY > r.top + r.height / 2 } : drag);
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (drag) {
                  onDrop(drag.from, i, drag.after);
                  onPick([], i, i);
                }
                onDrag(null);
              }}
              onDragEnd={() => onDrag(null)}
              onContextMenu={(e) => {
                if (!editable) return;
                e.preventDefault();
                if (!sel.includes(i)) onPick([], i, i);
                onMenu({ x: e.clientX, y: e.clientY });
              }}
              className={`flex items-center gap-2 border-b border-slate-800 px-2.5 py-1 ${
                on ? "relative z-10 rounded-md ring-1 ring-inset ring-[var(--key-hl-text)]" : ""
              } ${dropHere ? (drag?.after ? "border-b-2 border-[var(--key-hl-text)]" : "border-t-2 border-[var(--key-hl-text)]") : ""}`}
              onClick={(e) => {
                const n = nextPick(pick, anchor, i, rowMod(e));
                onPick(n.picked, n.anchor, i);
              }}
              onDoubleClick={() => onOpenEdit(i)}
            >
              <span className="w-5 shrink-0 text-2xs tabular-nums text-ink-4">{i + 1}</span>
              <ItemIcon item={it} />
              {editIdx === i ? (
                <ItemEditor
                  item={it}
                  onCommit={(next) => onSetItem(i, next)}
                  onClose={() => onCloseEdit(i)}
                />
              ) : (
                <span className="min-w-0 flex-1 truncate text-sm text-ink-1" title={title(it)}>
                  {title(it)}
                </span>
              )}
              <button
                type="button"
                aria-label={t("remote.macro.stepUp")}
                disabled={i === 0 || !editable}
                onClick={() => onMove(i, -1)}
                className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-ink-1 disabled:cursor-not-allowed disabled:text-ink-4"
              >
                <IconChevron className="h-3.5 w-3.5 -rotate-90" />
              </button>
              <button
                type="button"
                aria-label={t("remote.macro.stepDown")}
                disabled={i === items.length - 1 || !editable}
                onClick={() => onMove(i, 1)}
                className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-ink-1 disabled:cursor-not-allowed disabled:text-ink-4"
              >
                <IconChevron className="h-3.5 w-3.5 rotate-90" />
              </button>
              <button
                type="button"
                aria-label={t("remote.qp.delItems")}
                disabled={!editable}
                onClick={() => onAskDelete(i)}
                className="grid h-6 w-6 shrink-0 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-[var(--sem-danger-text)] disabled:cursor-not-allowed disabled:text-ink-4"
              >
                <IconClose className="h-3 w-3" />
              </button>
            </li>
          );
        })}
      </ol>

      {/* 右键菜单：与 tab 列同一套（挪位四条 + 走二次确认的删除）。 */}
      <RowMenu
        at={menuAt}
        onClose={() => onMenu(null)}
        items={[
          ...moveMenu({
            top: () => onReorder(moveEnd(items, targets, "top")),
            bottom: () => onReorder(moveEnd(items, targets, "bottom")),
            up: () => onReorder(moveBy(items, targets, -1)),
            down: () => onReorder(moveBy(items, targets, 1)),
            canUp: targets.some((x) => x > 0),
            canDown: targets.some((x) => x < items.length - 1),
          }),
          { label: t("remote.qp.delItems"), danger: true, onClick: () => onConfirmToggle(true) },
        ]}
      />

      {/* 删条目走确认（计划口径：删 tab 内容影响面大，先问一声；删步骤不弹框）。 */}
      <ConfirmDialog
        open={confirmOpen}
        title={t("remote.qp.delItems")}
        onClose={() => onConfirmToggle(false)}
        onConfirm={() => (onConfirmDelete(), onConfirmToggle(false))}
        danger
        irreversible
        confirmLabel={t("remote.qp.delItems")}
      >
        {t("remote.qp.delItemsDesc")
          .replace("{n}", String(targets.length))
          .replace("{path}", path)}
      </ConfirmDialog>
    </>
  );
}
