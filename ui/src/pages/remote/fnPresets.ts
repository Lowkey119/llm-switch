// 系统功能快捷键预设（按部署 OS 分集，用户 2026-11-25：跨平台，预设随系统变）。
//
// 组合键调研自各系统官方文档（2026-11-25）：
//   macOS  — support.apple.com/102650（Mac 键盘快捷键）
//   Windows — support.microsoft.com 键盘快捷方式
//   Linux  — help.gnome.org shell-keyboard-shortcuts（GNOME 为默认口径；KDE 与 Win 更接近，差异见各条）
// 键码沿用本仓 kVK 口径（mods 位图三平台同源：1=Ctrl、2=Shift、4=Alt、8=Meta/⌘/Win/Super）；
// 不可编码的键（如 PrtScn 在 mac kVK 表里没有）不收，宁缺毋滥。
// 遥控器后端目前仅 mac 落地（Remote.tsx 非 mac 显不支持）；Win/Linux 集先按官方口径备好，
// 后端跨平台移植落地即直接生效，前端不再动。
import { os } from "../../lib/key";
import type { FnCategory } from "./FnPicker";
import type { Shortcut } from "./shortcut";

const sc = (mods: number, key: number): Shortcut => ({ mods, key });
// kVK 速记（与 shortcut.ts 同源）
const K = {
  A: 0, S: 1, D: 2, F: 3, H: 4, G: 5, Z: 6, X: 7, C: 8, V: 9, B: 11, Q: 12, W: 13,
  E: 14, R: 15, Y: 16, T: 17, O: 31, U: 32, I: 34, P: 35, L: 37, J: 38, K: 40, N: 45, M: 46,
  Tab: 48, Space: 49, Esc: 53, F1: 122, F2: 120, F4: 118, F9: 101, F10: 109,
  Left: 123, Right: 124, Down: 125, Up: 126, PgUp: 116, PgDn: 121,
};
const CTRL = 1, SHIFT = 2, ALT = 4, META = 8;

/** Windows 集（Win 键 = Meta 位）。 */
const WIN: FnCategory[] = [
  {
    nameKey: "remote.fn.catWindow",
    items: [
      { nameKey: "remote.fn.taskView", sc: sc(META, K.Tab) },
      { nameKey: "remote.fn.desktop", sc: sc(META, K.D) },
      { nameKey: "remote.fn.switchWindow", sc: sc(ALT, K.Tab) },
      { nameKey: "remote.fn.minimizeAll", sc: sc(META, K.M) },
      { nameKey: "remote.fn.closeWindow", sc: sc(ALT, K.F4) },
      { nameKey: "remote.fn.tileLeft", sc: sc(META, K.Left) },
      { nameKey: "remote.fn.tileRight", sc: sc(META, K.Right) },
      { nameKey: "remote.fn.maximize", sc: sc(META, K.Up) },
    ],
  },
  {
    nameKey: "remote.fn.catApp",
    items: [
      { nameKey: "remote.fn.fileExplorer", sc: sc(META, K.E) },
      { nameKey: "remote.fn.runDialog", sc: sc(META, K.R) },
      { nameKey: "remote.fn.settings", sc: sc(META, K.I) },
      { nameKey: "remote.fn.search", sc: sc(META, K.S) },
      { nameKey: "remote.fn.taskManager", sc: sc(CTRL | SHIFT, K.Esc) },
    ],
  },
  {
    nameKey: "remote.fn.catSystem",
    items: [
      { nameKey: "remote.fn.lock", sc: sc(META, K.L) },
      { nameKey: "remote.fn.screenshotSel", sc: sc(META | SHIFT, K.S) },
      { nameKey: "remote.fn.inputSwitch", sc: sc(META, K.Space) },
      { nameKey: "remote.fn.emoji", sc: sc(META, K.C) },
      { nameKey: "remote.fn.clipboardHistory", sc: sc(META, K.V) },
      { nameKey: "remote.fn.voiceTyping", sc: sc(META, K.H) },
      { nameKey: "remote.fn.spaceLeft", sc: sc(META | CTRL, K.Left) },
      { nameKey: "remote.fn.spaceRight", sc: sc(META | CTRL, K.Right) },
    ],
  },
];

/** Linux（GNOME 默认口径；Super 键 = Meta 位）。 */
const LINUX: FnCategory[] = [
  {
    nameKey: "remote.fn.catWindow",
    items: [
      { nameKey: "remote.fn.activities", sc: sc(0, 55) },
      { nameKey: "remote.fn.showApps", sc: sc(META, K.A) },
      { nameKey: "remote.fn.switchWindow", sc: sc(META, K.Tab) },
      { nameKey: "remote.fn.closeWindow", sc: sc(ALT, K.F4) },
      { nameKey: "remote.fn.minimize", sc: sc(ALT, K.F9) },
      { nameKey: "remote.fn.maximize", sc: sc(ALT, K.F10) },
      { nameKey: "remote.fn.tileLeft", sc: sc(META, K.Left) },
      { nameKey: "remote.fn.tileRight", sc: sc(META, K.Right) },
    ],
  },
  {
    nameKey: "remote.fn.catApp",
    items: [{ nameKey: "remote.fn.runDialog", sc: sc(ALT, K.F2) }],
  },
  {
    nameKey: "remote.fn.catSystem",
    items: [
      { nameKey: "remote.fn.lock", sc: sc(META, K.L) },
      { nameKey: "remote.fn.inputSwitch", sc: sc(META, K.Space) },
      { nameKey: "remote.fn.spaceLeft", sc: sc(META, K.PgUp) },
      { nameKey: "remote.fn.spaceRight", sc: sc(META, K.PgDn) },
      { nameKey: "remote.fn.notificationList", sc: sc(META, K.V) },
    ],
  },
];

/** 当前 OS 的系统功能预设（mac 用 FnPicker 原集，其余按官方口径）。 */
export function fnCategoriesFor(which: typeof os, mac: FnCategory[]): FnCategory[] {
  return which === "mac" ? mac : which === "win" ? WIN : LINUX;
}
