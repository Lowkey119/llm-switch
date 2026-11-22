// 公用虚拟键盘（2026-11-20 从 FnPicker 拆出）：受控组件——mods/main 状态归调用方，
// 键盘只画键帽、上报点击。两个调用方：FnPicker（拼组合填槽位）与独立打字面板
// （遥控器方向键导航 + OK 注入到代理编辑框）。键帽描边口径两边共用。
import { CODE_TO_KVK, type Shortcut } from "./shortcut";

/** ⇧ 双字符键（美区布局，原型 ui_03）：小字副字符在上、主字符在下。 */
const SHIFT_LABEL: Record<number, string> = {
  50: "~", 18: "!", 19: "@", 20: "#", 21: "$", 23: "%", 22: "^", 26: "&", 28: "*", 25: "(", 29: ")", 27: "_", 24: "+",
  33: "{", 30: "}", 42: "|", 41: ":", 39: '"', 43: "<", 47: ">", 44: "?",
};

/** 虚拟键盘键位：mod = 修饰键（mods 位，点击切换按住）；否则主键（kVK）；w = 宽键 flex 系数。 */
export type KKey = { label: string; key: number; mod?: number; w?: string };

/**
 * 标准 104 键盘位排列（用户 2026-11-20 拍板）：修饰键按 Mac 键盘实际位置摆放
 * （⇧ 在字母行首尾、⌃⌥⌘ 在空格两侧），不再悬在键盘上方一排。kVK 与 shortcut.ts 同源。
 * 键帽 flex-1 撑满容器宽（用户 2026-11-20：键盘最大化），宽键按真实比例给 flex 系数。
 */
export const KBD_ROWS: KKey[][] = [
  [
    { label: "Esc", key: 53 },
    { label: "F1", key: 122 }, { label: "F2", key: 120 }, { label: "F3", key: 99 }, { label: "F4", key: 118 },
    { label: "F5", key: 96 }, { label: "F6", key: 97 }, { label: "F7", key: 98 }, { label: "F8", key: 100 },
    { label: "F9", key: 101 }, { label: "F10", key: 109 }, { label: "F11", key: 103 }, { label: "F12", key: 111 },
  ],
  [
    { label: "`", key: 50 }, { label: "1", key: 18 }, { label: "2", key: 19 }, { label: "3", key: 20 },
    { label: "4", key: 21 }, { label: "5", key: 23 }, { label: "6", key: 22 }, { label: "7", key: 26 },
    { label: "8", key: 28 }, { label: "9", key: 25 }, { label: "0", key: 29 }, { label: "-", key: 27 },
    { label: "=", key: 24 }, { label: "⌫", key: 51, w: "flex-[1.4]" },
  ],
  [
    { label: "Tab", key: 48, w: "flex-[1.4]" },
    { label: "Q", key: 12 }, { label: "W", key: 13 }, { label: "E", key: 14 }, { label: "R", key: 15 },
    { label: "T", key: 17 }, { label: "Y", key: 16 }, { label: "U", key: 32 }, { label: "I", key: 34 },
    { label: "O", key: 31 }, { label: "P", key: 35 }, { label: "[", key: 33 }, { label: "]", key: 30 },
    { label: "\\", key: 42 },
  ],
  [
    { label: "⇪", key: 57, w: "flex-[1.4]" },
    { label: "A", key: 0 }, { label: "S", key: 1 }, { label: "D", key: 2 }, { label: "F", key: 3 },
    { label: "G", key: 5 }, { label: "H", key: 4 }, { label: "J", key: 38 }, { label: "K", key: 40 },
    { label: "L", key: 37 }, { label: ";", key: 41 }, { label: "'", key: 39 }, { label: "↵", key: 36, w: "flex-[1.4]" },
  ],
  [
    { label: "⇧", key: 56, mod: 2, w: "flex-[1.8]" },
    { label: "Z", key: 6 }, { label: "X", key: 7 }, { label: "C", key: 8 }, { label: "V", key: 9 },
    { label: "B", key: 11 }, { label: "N", key: 45 }, { label: "M", key: 46 }, { label: ",", key: 43 },
    { label: ".", key: 47 }, { label: "/", key: 44 }, { label: "⇧", key: 56, mod: 2, w: "flex-[1.8]" },
  ],
  [
    { label: "⌃", key: 59, mod: 1, w: "flex-[1.2]" }, { label: "⌥", key: 58, mod: 4, w: "flex-[1.2]" },
    { label: "⌘", key: 55, mod: 8, w: "flex-[1.2]" }, { label: "Space", key: 49, w: "flex-[4.5]" },
    { label: "⌘", key: 55, mod: 8, w: "flex-[1.2]" }, { label: "⌥", key: 58, mod: 4, w: "flex-[1.2]" },
  ],
];

/** 物理修饰键 code → mods 位（左右同位，与 shortcut.ts 位图同源）。 */
export const PHYS_MODS: Record<string, number> = {
  ControlLeft: 1, ControlRight: 1, ShiftLeft: 2, ShiftRight: 2, AltLeft: 4, AltRight: 4, MetaLeft: 8, MetaRight: 8,
};

/**
 * 键帽网格（受控）：mods 位图点亮修饰键，main 点亮主键；点击上报后由调用方决定语义
 * （拼组合 / 注入打字）。选中态青描边加粗——两套主题都读得出「按下了哪颗」。
 */
export function VirtualKeyboard({
  mods,
  main,
  onMod,
  onMain,
  className = "",
}: {
  mods: number;
  main: number | null;
  onMod: (bit: number) => void;
  onMain: (kvk: number) => void;
  className?: string;
}) {
  const keyBtn = (k: KKey, j: string | number) => {
    const on = k.mod ? (mods & k.mod) !== 0 : main === k.key;
    return (
      <button
        key={j}
        type="button"
        onMouseDown={(e) => e.preventDefault()}
        aria-pressed={on}
        onClick={() => (k.mod ? onMod(k.mod) : onMain(k.key))}
        className={`flex h-[46px] min-w-0 flex-1 flex-col items-center justify-center rounded border px-1 text-sm leading-none tabular-nums transition-colors ${k.w ?? ""} ${
          on
            ? "border-[var(--key-hl)] bg-[var(--key-hl)]/20 font-bold text-ink-1"
            : "border-slate-600 bg-slate-800 text-ink-2 hover:border-slate-400 hover:text-ink-1"
        }`}
      >
        {SHIFT_LABEL[k.key] ? (
          <>
            <span className={`text-2xs leading-[13px] ${on ? "text-ink-2" : "text-ink-4"}`}>
              {SHIFT_LABEL[k.key]}
            </span>
            <span className="leading-[19px]">{k.label}</span>
          </>
        ) : (
          k.label
        )}
      </button>
    );
  };
  const row = (r: KKey[], i: number) => <div key={i} className="flex gap-1.5">{r.map(keyBtn)}</div>;
  return (
    <div className={`flex flex-col gap-1.5 ${className}`} aria-label="Virtual keyboard">
      {KBD_ROWS.slice(0, 4).map(row)}
      {/* 底部段（原型 ui_03）：修饰/空格行在左，倒 T 方向键簇在右——↑ 在 ↓ 正上方。 */}
      <div className="flex gap-1.5">
        <div className="flex min-w-0 flex-1 flex-col gap-1.5">{KBD_ROWS.slice(4).map(row)}</div>
        <div className="grid shrink-0 grid-cols-3 gap-1.5">
          <span />
          {keyBtn({ label: "↑", key: 126, w: "w-[46px]" }, "u")}
          <span />
          {keyBtn({ label: "←", key: 123, w: "w-[46px]" }, "l")}
          {keyBtn({ label: "↓", key: 125, w: "w-[46px]" }, "d")}
          {keyBtn({ label: "→", key: 124, w: "w-[46px]" }, "r")}
        </div>
      </div>
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
