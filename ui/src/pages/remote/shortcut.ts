// 快捷键录制/显示：KeyboardEvent.code ↔ CGEvent 虚拟键码（kVK）双向表。
//
// 数值逐条取自 core-graphics 0.25.0 的 KeyCode 常量（后端 enigo 注入用的就是它，
// 两端同源才不会「显示 A 注入 B」）。mods 位图约定（与 Rust Shortcut 一致）：
// bit0 ctrl · bit1 shift · bit2 alt · bit3 meta。裸单键 = mods 0，无黑名单。
// C1 扩展：fn = 组合带 Fn 位（只来自 flagsChanged(Fn) 边沿）；side = 修饰键左右侧。
// 两字段可缺省（旧配置、keydown 兜底路径），缺省 = fn false / side null。
import { t } from "../../lib/i18n";

export type ModSide = "left" | "right";

export interface Shortcut {
  mods: number;
  key: number;
  fn?: boolean;
  side?: ModSide | null;
}

/** KeyboardEvent.code → CGEvent 虚拟键码。修饰键不在表内（不能单独成快捷键）。 */
export const CODE_TO_KVK: Record<string, number> = {
  KeyA: 0, KeyS: 1, KeyD: 2, KeyF: 3, KeyH: 4, KeyG: 5, KeyZ: 6, KeyX: 7,
  KeyC: 8, KeyV: 9, KeyB: 11, KeyQ: 12, KeyW: 13, KeyE: 14, KeyR: 15, KeyY: 16,
  KeyT: 17, KeyO: 31, KeyU: 32, KeyI: 34, KeyP: 35, KeyL: 37, KeyJ: 38, KeyK: 40,
  KeyN: 45, KeyM: 46,
  Digit1: 18, Digit2: 19, Digit3: 20, Digit4: 21, Digit6: 22, Digit5: 23,
  Digit9: 25, Digit7: 26, Digit8: 28, Digit0: 29,
  Minus: 27, Equal: 24, BracketLeft: 33, BracketRight: 30, Backslash: 42,
  Semicolon: 41, Quote: 39, Comma: 43, Period: 47, Slash: 44, Backquote: 50,
  Tab: 48, Enter: 36, Space: 49, Backspace: 51, Escape: 53, Delete: 117,
  ArrowLeft: 123, ArrowRight: 124, ArrowDown: 125, ArrowUp: 126,
  Home: 115, End: 119, PageUp: 116, PageDown: 121,
  F1: 122, F2: 120, F3: 99, F4: 118, F5: 96, F6: 97, F7: 98, F8: 100,
  F9: 101, F10: 109, F11: 103, F12: 111, F13: 105, F14: 107, F15: 113,
  F16: 106, F17: 64, F18: 79, F19: 80, F20: 90,
  Numpad0: 82, Numpad1: 83, Numpad2: 84, Numpad3: 85, Numpad4: 86, Numpad5: 87,
  Numpad6: 88, Numpad7: 89, Numpad8: 91, Numpad9: 92,
  NumpadEnter: 76, NumpadAdd: 69, NumpadSubtract: 78, NumpadMultiply: 67,
  NumpadDivide: 75, NumpadDecimal: 65,
};

/** kVK → 显示标签（Apple 展示惯例：字母数字符号原字符，功能键给名字）。 */
const KVK_LABEL: Record<number, string> = {
  0: "A", 1: "S", 2: "D", 3: "F", 4: "H", 5: "G", 6: "Z", 7: "X", 8: "C", 9: "V",
  11: "B", 12: "Q", 13: "W", 14: "E", 15: "R", 16: "Y", 17: "T", 18: "1", 19: "2",
  20: "3", 21: "4", 22: "6", 23: "5", 24: "=", 25: "9", 26: "7", 27: "-", 28: "8",
  29: "0", 30: "]", 31: "O", 32: "U", 33: "[", 34: "I", 35: "P", 37: "L", 38: "J",
  39: "'", 40: "K", 41: ";", 42: "\\", 43: ",", 44: "/", 45: "N", 46: "M", 47: ".",
  50: "`", 36: "Enter", 48: "Tab", 49: "Space", 51: "Delete", 53: "Esc", 117: "FwdDel",
  64: "F17", 79: "F18", 80: "F19", 90: "F20", 96: "F5", 97: "F6", 98: "F7",
  99: "F3", 100: "F8", 101: "F9", 103: "F11", 105: "F13", 106: "F16", 107: "F14",
  109: "F10", 111: "F12", 113: "F15", 118: "F4", 120: "F2", 122: "F1",
  65: "Num.", 67: "Num*", 69: "Num+", 71: "NumClear", 75: "Num/", 76: "NumEnter",
  78: "Num-", 81: "Num=", 82: "Num0", 83: "Num1", 84: "Num2", 85: "Num3",
  86: "Num4", 87: "Num5", 88: "Num6", 89: "Num7", 91: "Num8", 92: "Num9",
  72: "Vol+", 73: "Vol-", 74: "Mute",
  114: "Help", 115: "Home", 116: "PgUp", 119: "End", 121: "PgDn",
  123: "←", 124: "→", 125: "↓", 126: "↑",
};

/** 可独立成键的修饰键 kVK → (mods 位, 侧)；与 Rust actions::MOD_KEYCODES 同源。63=Fn 无位。 */
const MOD_KVK: Record<number, [number, ModSide | null]> = {
  54: [8, "right"], 55: [8, "left"], 56: [2, "left"], 58: [4, "left"], 59: [1, "left"],
  60: [2, "right"], 61: [4, "right"], 62: [1, "right"], 63: [0, null],
};

/** 从按键事件提取快捷键；修饰键 / 表外键返回 null（不拦截、不入库）。
 *  单修饰键绑定只走后端 tap 的边沿甄别（CaptureTracker）：keydown 兜底路径分不出左右侧、
 *  也拿不到 Fn 真边沿（KeyDown 自带伪位），故修饰键在这里照旧不算组合。 */
export function eventToShortcut(e: KeyboardEvent): Shortcut | null {
  const key = CODE_TO_KVK[e.code];
  if (key === undefined) return null;
  let mods = 0;
  if (e.ctrlKey) mods |= 1;
  if (e.shiftKey) mods |= 2;
  if (e.altKey) mods |= 4;
  if (e.metaKey) mods |= 8;
  return { mods, key };
}

/** 修饰位 → 字形（Apple 展示顺序 ⌃⌥⇧⌘）。 */
const MOD_GLYPH: Record<number, string> = { 1: "⌃", 2: "⇧", 4: "⌥", 8: "⌘" };

/** 展示格式化：fn + ⌃⌥⇧⌘ + 键标签；C1：单修饰键给「左⌘」式名字，组合带侧给「右⌘C」前缀。 */
export function fmtShortcut(s: Shortcut): string {
  // 单修饰键绑定：直接给键名（左/右侧 + 字形，Fn 给 "Fn"）。
  if (s.mods === 0 && MOD_KVK[s.key]) {
    const [, side] = MOD_KVK[s.key];
    if (s.key === 63) return "Fn";
    const sideTag = side ? t(side === "left" ? "remote.sc.left" : "remote.sc.right") : "";
    return sideTag + MOD_GLYPH[MOD_KVK[s.key][0]];
  }
  let out = s.fn ? "fn " : "";
  if (s.side) out += t(s.side === "left" ? "remote.sc.left" : "remote.sc.right");
  if (s.mods & 1) out += "⌃";
  if (s.mods & 4) out += "⌥";
  if (s.mods & 2) out += "⇧";
  if (s.mods & 8) out += "⌘";
  return out + (KVK_LABEL[s.key] ?? `?${s.key}`);
}
