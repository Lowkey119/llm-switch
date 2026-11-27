// 公用虚拟键盘（2026-11-20 从 FnPicker 拆出）：受控组件——mods/main 状态归调用方，
// 键盘只画键帽、上报点击。两个调用方：FnPicker（拼组合填槽位）与独立打字面板
// （遥控器方向键导航 + OK 注入到代理编辑框）。键帽描边口径两边共用。
// 标准 104 全配列（用户 2026-11-25 按实物图定稿）：F 区分组、导航簇（Insert/Home/PgUp·
// Del/End/PgDn）对齐数字行、倒 T 方向簇在其下、数字小键盘右侧通高。
// 布局 = 96 子列网格（1 键宽 = 4 子列，宽键按真实比例占 6/7/8/9…子列），
// 每键 gridColumn/gridRow 绝对落位，三区块（主键区/导航簇/小键盘）天然逐行对齐。
import { t } from "../../lib/i18n";
import { os, type Os } from "../../lib/key";
import { IconHandPointUp } from "../../components/icons";
import { CODE_TO_KVK, type Shortcut } from "./shortcut";

/** ⇧ 双字符键（美区布局，原型 ui_03）：小字副字符在上、主字符在下。 */
const SHIFT_LABEL: Record<number, string> = {
  50: "~", 18: "!", 19: "@", 20: "#", 21: "$", 23: "%", 22: "^", 26: "&", 28: "*", 25: "(", 29: ")", 27: "_", 24: "+",
  33: "{", 30: "}", 42: "|", 41: ":", 39: '"', 43: "<", 47: ">", 44: "?",
};

/** 虚拟键盘键位：mod = 修饰键（mods 位，点击切换按住）；fn = Fn 键（独立位，非 mods）；否则主键（kVK）。 */
export type KKey = { label: string; key: number; mod?: number; fn?: boolean };

/** 落位键 = 键位 + 网格坐标（子列 c、行 r、宽 span、高 rowspan）。 */
type Placed = KKey & { c: number; r: number; s?: number; rs?: number };

const P = (label: string, key: number, c: number, r: number, s = 4, rs?: number, extra?: Partial<KKey>): Placed => ({
  label, key, c, r, s, rs, ...extra,
});

/** 数字行 13 键（` 1..0 - =）：c 从 0 起每键 4 子列。 */
const NUM_ROW: Placed[] = [
  ["`", 50], ["1", 18], ["2", 19], ["3", 20], ["4", 21], ["5", 23], ["6", 22], ["7", 26],
  ["8", 28], ["9", 25], ["0", 29], ["-", 27], ["=", 24],
].map(([label, key], i) => P(label as string, key as number, i * 4, 2));

/** 字母 Q 行 13 键（Tab 后）：Tab 宽 6，Q 起 c6。 */
const Q_ROW: Placed[] = [
  ["Q", 12], ["W", 13], ["E", 14], ["R", 15], ["T", 17], ["Y", 16], ["U", 32], ["I", 34],
  ["O", 31], ["P", 35], ["[", 33], ["]", 30],
].map(([label, key], i) => P(label as string, key as number, 6 + i * 4, 3));

/** 字母 A 行 11 键（Caps 后）：Caps 宽 7，A 起 c8。 */
const A_ROW: Placed[] = [
  ["A", 0], ["S", 1], ["D", 2], ["F", 3], ["G", 5], ["H", 4], ["J", 38], ["K", 40],
  ["L", 37], [";", 41], ["'", 39],
].map(([label, key], i) => P(label as string, key as number, 8 + i * 4, 4));

/** 字母 Z 行 10 键（左 Shift 后）：Shift 宽 9，Z 起 c12。 */
const Z_ROW: Placed[] = [
  ["Z", 6], ["X", 7], ["C", 8], ["V", 9], ["B", 11], ["N", 45], ["M", 46], [",", 43],
  [".", 47], ["/", 44],
].map(([label, key], i) => P(label as string, key as number, 12 + i * 4, 5));

/**
 * 底部修饰键行按部署 OS 适配（用户 2026-11-25：跨平台，键位随系统变）。
 * mods 位图三平台同源（⌃/Ctrl=1、⌥/Alt=4、⌘/Win·Super=8），只是键帽写法与物理键不同：
 * mac 有 fn（kVK 63，独立 fn 位）；Win/Linux 的 Win/Super 都落 Meta 位。
 */
function bottomRow(which: Os): Placed[] {
  if (which === "mac") {
    return [
      P("⌃", 59, 0, 6, 4, undefined, { mod: 1 }),
      P("⌥", 58, 4, 6, 4, undefined, { mod: 4 }),
      P("⌘", 55, 8, 6, 4, undefined, { mod: 8 }),
      // fn 是修饰键（fn?:boolean 独立位），不是主键——bit16 仅在键盘 UI 内部代表 fn，
      // 落 Shortcut 时转成 fn:true（见 useVirtualKeyboard::commit）。
      P("fn", 63, 12, 6, 4, undefined, { mod: 16 }),
      P("Space", 49, 16, 6, 32),
      P("⌘", 55, 48, 6, 4, undefined, { mod: 8 }),
      P("⌥", 58, 52, 6, 4, undefined, { mod: 4 }),
      P("fn", 63, 56, 6, 4, undefined, { mod: 16 }),
    ];
  }
  const meta = which === "win" ? "Win" : "Super";
  return [
    P("Ctrl", 59, 0, 6, 4, undefined, { mod: 1 }),
    P(meta, 55, 4, 6, 4, undefined, { mod: 8 }),
    P("Alt", 58, 8, 6, 4, undefined, { mod: 4 }),
    P("Space", 49, 12, 6, 32),
    P("Alt", 58, 44, 6, 4, undefined, { mod: 4 }),
    P(meta, 55, 48, 6, 4, undefined, { mod: 8 }),
    P("Menu", 110, 52, 6, 4),
    P("Ctrl", 59, 56, 6, 4, undefined, { mod: 1 }),
  ];
}

/**
 * 标准 104 全配列（用户 2026-11-25 按实物图定稿）：
 * 行 1 F 区（Esc | F1-4 | F5-8 | F9-12 ‖ PrtSc·ScrLk·Pause），
 * 行 2-6 主键区 ‖ 导航簇（行 2-3）+ 倒 T（行 5-6）‖ 小键盘（行 2-6 通高，+ 与 ⏎ 跨两行）。
 * 列坐标单位 = 子列（1 键宽 = 4）；主键区 0-59，导航簇 64-75，小键盘 80-95。
 */
const KEYS: Placed[] = [
  // F 区（用户 2026-11-25：组间 gap 半键，F12 右缘对齐退格键右缘 c60）
  P("Esc", 53, 0, 1),
  P("F1", 122, 8, 1), P("F2", 120, 12, 1), P("F3", 99, 16, 1), P("F4", 118, 20, 1),
  P("F5", 96, 26, 1), P("F6", 97, 30, 1), P("F7", 98, 34, 1), P("F8", 100, 38, 1),
  P("F9", 101, 44, 1), P("F10", 109, 48, 1), P("F11", 103, 52, 1), P("F12", 111, 56, 1),
  P("PrtSc", 105, 64, 1), P("ScrLk", 107, 68, 1), P("Pause", 113, 72, 1),
  // 主键区
  ...NUM_ROW, P("⌫", 51, 52, 2, 8),
  P("Tab", 48, 0, 3, 6), ...Q_ROW, P("\\", 42, 54, 3, 6),
  P("⇪", 57, 0, 4, 7), ...A_ROW, P("↵", 36, 52, 4, 8),
  P("⇧", 56, 0, 5, 9, undefined, { mod: 2 }), ...Z_ROW, P("⇧", 56, 52, 5, 8, undefined, { mod: 2 }),
  // 导航簇（对齐数字行/字母行）+ 倒 T 方向簇
  P("Ins", 114, 64, 2), P("Home", 115, 68, 2), P("PgUp", 116, 72, 2),
  P("Del", 117, 64, 3), P("End", 119, 68, 3), P("PgDn", 121, 72, 3),
  P("↑", 126, 68, 5),
  P("←", 123, 64, 6), P("↓", 125, 68, 6), P("→", 124, 72, 6),
  // 数字小键盘（右侧通高）
  P("Num", 71, 80, 2), P("/", 75, 84, 2), P("*", 67, 88, 2), P("-", 78, 92, 2),
  P("7", 89, 80, 3), P("8", 91, 84, 3), P("9", 92, 88, 3), P("+", 69, 92, 3, 4, 2),
  P("4", 86, 80, 4), P("5", 87, 84, 4), P("6", 88, 88, 4),
  P("1", 83, 80, 5), P("2", 84, 84, 5), P("3", 85, 88, 5), P("⏎", 76, 92, 5, 4, 2),
  P("0", 82, 80, 6, 8), P(".", 65, 88, 6),
];

/** 物理修饰键 code → mods 位（左右同位，与 shortcut.ts 位图同源）。 */
export const PHYS_MODS: Record<string, number> = {
  ControlLeft: 1, ControlRight: 1, ShiftLeft: 2, ShiftRight: 2, AltLeft: 4, AltRight: 4, MetaLeft: 8, MetaRight: 8,
};

/**
 * 键帽网格（受控）：mods 位图点亮修饰键，main 点亮主键；点击上报后由调用方决定语义
 * （拼组合 / 注入打字）。选中态青描边加粗——两套主题都读得出「按下了哪颗」。
 * 96 子列网格绝对落位（见 KEYS），容器 min-w 保证键帽不挤形，窄窗由外层横滚兜底。
 * holdMod：处于「按住」态的修饰键位（琥珀描边+填充，与轻点的青色一眼可辨）；
 * onCommit：给则画落盘入口（大手图标，用户 2026-11-27 拍板：写条目一律显式落盘）。
 */
export function VirtualKeyboard({
  mods,
  main,
  onMod,
  onMain,
  holdMod = 0,
  onCommit,
  className = "",
}: {
  mods: number;
  main: number | null;
  onMod: (bit: number) => void;
  onMain: (kvk: number) => void;
  holdMod?: number;
  onCommit?: () => void;
  className?: string;
}) {
  const keys = [...KEYS, ...bottomRow(os)];
  const canCommit = main !== null || holdMod !== 0;
  return (
    <div className={className}>
      <div
        className="grid grid-cols-[repeat(96,minmax(0,1fr))]"
        style={{ gridTemplateRows: "repeat(6, 46px)" }}
        aria-label={t("remote.kbd.aria")}
      >
        {keys.map((k, i) => {
          const hold = !!k.mod && holdMod === k.mod;
          const on = !hold && (k.mod ? (mods & k.mod) !== 0 : main === k.key);
          return (
            <button
              key={`${k.label}-${i}`}
              type="button"
              onMouseDown={(e) => e.preventDefault()}
              aria-pressed={on || hold}
              onClick={() => (k.mod ? onMod(k.mod) : onMain(k.key))}
              style={{ gridColumn: `${k.c + 1} / span ${k.s ?? 4}`, gridRow: `${k.r} / span ${k.rs ?? 1}` }}
              className={`m-[3px] flex min-w-0 flex-col items-center justify-center rounded border px-1 text-sm leading-none tabular-nums transition-colors ${
                hold
                  ? "border-[var(--sem-busy)] bg-[var(--sem-busy)]/25 font-bold text-ink-1"
                  : on
                    ? "border-[var(--key-hl-text)] bg-[var(--key-hl)]/20 font-bold text-ink-1"
                    : "border-slate-600 bg-slate-800 text-ink-2 hover:border-slate-400 hover:text-ink-1"
              }`}
            >
              {SHIFT_LABEL[k.key] ? (
                <>
                  <span className={`text-2xs leading-[13px] ${on || hold ? "text-ink-2" : "text-ink-4"}`}>
                    {SHIFT_LABEL[k.key]}
                  </span>
                  <span className="leading-[19px]">{k.label}</span>
                </>
              ) : (
                k.label
              )}
            </button>
          );
        })}
      </div>
      {/* 落盘入口（用户 2026-11-27 拍板）：大手图标 = 把摆好的组合写进条目并收键盘。
          没摆键时禁用；按住态下提示再点修饰键可取消。 */}
      {onCommit && (
        <div className="mt-2 flex items-center justify-end gap-2">
          {holdMod !== 0 && <span className="text-sm text-[var(--sem-busy-text)]">{t("remote.kbd.holdOn")}</span>}
          <button
            type="button"
            disabled={!canCommit}
            onClick={onCommit}
            aria-label={t("remote.kbd.commit")}
            data-tip={t("remote.kbd.commit")}
            className={`tip tip-up flex h-12 w-12 items-center justify-center rounded-lg border-2 transition-colors ${
              canCommit
                ? "border-[var(--sem-ok)] bg-[var(--sem-ok)]/15 text-[var(--sem-ok)] hover:bg-[var(--sem-ok)]/30"
                : "cursor-not-allowed border-slate-700 text-ink-4"
            }`}
          >
            <IconHandPointUp className="h-7 w-7" aria-hidden />
          </button>
        </div>
      )}
    </div>
  );
}

/** 物理键盘事件 → (mods 位 | 主键 kVK)：修饰键与主键两条口，与 VirtualKeyboard 上报同形。 */
export function physicalKey(e: KeyboardEvent): { mod?: number; kvk?: number } {
  const bit = PHYS_MODS[e.code];
  if (bit) return { mod: bit };
  const kvk = CODE_TO_KVK[e.code];
  return kvk !== undefined ? { kvk } : {};
}

/** 组合形状（供调用方拼 Shortcut）：main 未定 = 组合未成。 */
export function compose(mods: number, main: number | null): Shortcut | null {
  return main !== null ? { mods, key: main } : null;
}
