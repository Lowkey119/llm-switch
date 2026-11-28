// 列表行交互公共件（tab 列表 / 条目列表 / 动作列表共用一套，别各写一份）。
//
// 口径（用户 2026-11-25）：列表要能多选（点选 / ctrl⌘ 加减选 / shift 连选）、
// 拖动批量排序、右键菜单（前移/后移/移到顶/移到底）、行首带序号。
// 这里只放**纯函数 + 菜单组件**：选中态仍由各面板自己持有，避免把三块面板的
// 状态形状绑到一起（它们的行数据完全不同）。
import { PointPopover } from "../../components/Popover";
import { t } from "../../lib/i18n";

/** 鼠标修饰键（mac 的 ⌘ 与 ctrl 同义，一起算）。 */
export type PickMod = { ctrl: boolean; shift: boolean };

export const rowMod = (e: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): PickMod => ({
  ctrl: e.ctrlKey || e.metaKey,
  shift: e.shiftKey,
});

/**
 * 点一行后的新选中集：普通点 = 只选它；ctrl/⌘ = 加选/减选；shift = 从锚点连选。
 * 锚点只在普通点与加减选时挪（shift 连选不挪锚点，否则连按 shift 会一路缩回去）。
 */
export function nextPick(
  picked: number[],
  anchor: number,
  i: number,
  mod: PickMod,
): { picked: number[]; anchor: number } {
  if (mod.shift) {
    const [lo, hi] = anchor <= i ? [anchor, i] : [i, anchor];
    return { picked: Array.from({ length: hi - lo + 1 }, (_, k) => lo + k), anchor };
  }
  if (mod.ctrl) {
    return {
      picked: picked.includes(i) ? picked.filter((x) => x !== i) : [...picked, i],
      anchor: i,
    };
  }
  return { picked: [], anchor: i };
}

/** 真正要下手的下标：有加选就按加选，否则算当前那一行。 */
export const targetsOf = (picked: number[], sel: number) =>
  picked.length ? [...picked].sort((a, b) => a - b) : [sel];

/** 前移/后移整批：批内相邻的不互换，整批一起挪一格（越界的原地不动）。 */
export function moveBy<T>(list: T[], idxs: number[], d: number): T[] {
  const set = new Set(idxs);
  const arr = [...list];
  for (const i of [...set].sort((a, b) => (d < 0 ? a - b : b - a))) {
    const j = i + d;
    if (j < 0 || j >= arr.length || set.has(j)) continue;
    [arr[i], arr[j]] = [arr[j], arr[i]];
    set.delete(i);
    set.add(j);
  }
  return arr;
}

/** 移到顶/底：批内相对顺序保持。 */
export function moveEnd<T>(list: T[], idxs: number[], end: "top" | "bottom"): T[] {
  const set = new Set(idxs);
  const moved = list.filter((_, i) => set.has(i));
  const rest = list.filter((_, i) => !set.has(i));
  return end === "top" ? [...moved, ...rest] : [...rest, ...moved];
}

/** 拖放到某行的前/后：落点按「原序列」算，内部换算成移除后的下标。 */
export function dropAt<T>(list: T[], idxs: number[], target: number, after: boolean): T[] {
  const set = new Set(idxs);
  if (set.has(target)) return list;
  const pos = target + (after ? 1 : 0);
  const moved = list.filter((_, i) => set.has(i));
  const rest = list.filter((_, i) => !set.has(i));
  const at = list.filter((_, i) => !set.has(i)).filter((_, k) => list.indexOf(rest[k]) < pos).length;
  return [...rest.slice(0, at), ...moved, ...rest.slice(at)];
}

export type MenuItem = { label: string; onClick: () => void; danger?: boolean; disabled?: boolean };

/**
 * 右键菜单：光标处弹出自定义菜单（禁掉系统菜单），点别处/Esc 即收。
 * 定位不自己写：直接套仓内浮层单一真相 `PointPopover`（量完自身宽高再翻向 + clamp 进视口，
 * zoom 坐标换算也收在那一处）。原先本组件自己 `useLayoutEffect` 写 `el.style.left/top`，
 * 而 JSX 里 `style={{ left: at.x, top: at.y }}` 用的是**未除倍率的鼠标坐标**：
 * 倍率 1.25 下菜单一开始就偏离光标，重渲染时 React 还可能把纠正后的坐标盖回去（用户 2026-11-26）。
 */
export function RowMenu({
  at,
  items,
  onClose,
}: {
  at: { x: number; y: number } | null;
  items: MenuItem[];
  onClose: () => void;
}) {
  if (!at) return null;
  return (
    <PointPopover point={at} onClose={onClose}>
      <div
        role="menu"
        className="min-w-40"
        onContextMenu={(e) => e.preventDefault()}
      >
        {items.map((m) => (
          <button
            key={m.label}
            type="button"
            role="menuitem"
            disabled={m.disabled}
            onClick={() => (m.onClick(), onClose())}
            className={`block w-full rounded px-2.5 py-1.5 text-left text-sm disabled:cursor-not-allowed disabled:text-ink-4 ${
              m.danger ? "text-[var(--sem-danger-text)]" : "text-ink-1"
            } hover:bg-slate-700`}
          >
            {m.label}
          </button>
        ))}
        <p className="mt-1 border-t border-slate-700 px-2.5 pt-1 text-2xs text-ink-4">
          {t("remote.list.menuHint")}
        </p>
      </div>
    </PointPopover>
  );
}

/** 右键菜单的四条挪位项（各面板只喂「怎么挪」，措辞这里统一）。 */
export const moveMenu = (o: {
  top: () => void;
  bottom: () => void;
  up: () => void;
  down: () => void;
  canUp: boolean;
  canDown: boolean;
}): MenuItem[] => [
  { label: t("remote.list.moveTop"), onClick: o.top },
  { label: t("remote.list.moveBottom"), onClick: o.bottom },
  { label: t("remote.list.moveUp"), onClick: o.up, disabled: !o.canUp },
  { label: t("remote.list.moveDown"), onClick: o.down, disabled: !o.canDown },
];
