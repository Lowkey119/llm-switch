// 按键映射区的键表与热点坐标（docs/ui_02/mapping.html 原型 KEYS/HOTS 回灌）。
//
// KEYS 只收后端 RemoteButton::ALL 真实支持的 12 键（id 与 buttons.rs::id() 一一对应）；
// 语音键后端未接入，不画（法条：不造假状态）。
// HOTS 按实物图标定的百分比坐标。原图 remote-hi.png（1248×2752）四周有大片透明边、
// 键区以下全是空机身——webp 已裁掉透明区并砍到 TV/菜单键行以下（297×720），
// 下面数值是裁前标定按偏移/缩放换算到裁后图的口径。
import type { ComponentType, SVGProps } from "react";
import {
  IconKeyBack,
  IconKeyDown,
  IconKeyHome,
  IconKeyLeft,
  IconKeyMenu,
  IconKeyMinus,
  IconKeyOk,
  IconKeyPlus,
  IconKeyPower,
  IconKeyRight,
  IconKeyTv,
  IconKeyUp,
} from "../../components/icons";

export interface KeyDef {
  /** 后端 ButtonConfig 的键（RemoteButton::id()）。 */
  id: string;
  /** i18n 键名。 */
  nameKey: string;
  icon: ComponentType<SVGProps<SVGSVGElement>>;
  /** 所在列：实物图左 / 右。 */
  col: "l" | "r";
}

/** 左列 6 卡 + 右列 6 卡（自上而下与实物图按键位置大致对齐）。 */
export const KEYS: KeyDef[] = [
  { id: "power", nameKey: "remote.key.power", icon: IconKeyPower, col: "l" },
  { id: "up", nameKey: "remote.key.up", icon: IconKeyUp, col: "l" },
  { id: "left", nameKey: "remote.key.left", icon: IconKeyLeft, col: "l" },
  { id: "back", nameKey: "remote.key.back", icon: IconKeyBack, col: "l" },
  { id: "home", nameKey: "remote.key.home", icon: IconKeyHome, col: "l" },
  { id: "menu", nameKey: "remote.key.menu", icon: IconKeyMenu, col: "l" },
  { id: "ok", nameKey: "remote.key.ok", icon: IconKeyOk, col: "r" },
  { id: "right", nameKey: "remote.key.right", icon: IconKeyRight, col: "r" },
  { id: "down", nameKey: "remote.key.down", icon: IconKeyDown, col: "r" },
  { id: "volume_up", nameKey: "remote.key.volume_up", icon: IconKeyPlus, col: "r" },
  { id: "volume_down", nameKey: "remote.key.volume_down", icon: IconKeyMinus, col: "r" },
  { id: "tv", nameKey: "remote.key.tv", icon: IconKeyTv, col: "r" },
];

/** 图宽高比换算因子：圆直径占宽 w% 时，占高 = w × HOT_F（图 1248×2752）。 */
export const HOT_F = 1248 / 2752;

/** 统一高亮圆直径（占图宽 %）：实拍全部键帽直径（含方向键位）的均值，
 *  用户 2026-11-20 拍板——不搞一键一圈，一个尺寸通吃。 */
export const HOT_D = 13.4;

/** 有实物图标定资产的档案（remote-hi.webp + HOTS 只标定了小米 RC001/RC003）。
 *  其余档案有键表也走键名网格（MapCenter），不得套图（队列 22 口径）。 */
export const hasRemoteImage = (profileId?: string | null) => profileId === "xiaomi_rc00x";

/** 三槽键名（与后端 Slot serde 名对齐）。 */
export type SlotKey = "click" | "double" | "long";
export const SLOTS: { id: SlotKey; labelKey: string }[] = [
  { id: "click", labelKey: "remote.map.slotClick" },
  { id: "double", labelKey: "remote.map.slotDouble" },
  { id: "long", labelKey: "remote.map.slotLong" },
];

/**
 * 热点圆心盒左上角（% 图坐标，直径统一 HOT_D）。2026-11-20 用 mi-remote.png 实拍标定：
 * 圆键取黑色键帽 blob bbox 中心，方向键/ok 取背光字形质心（圆盘上无独立键帽轮廓）。
 * 热点层按 <img> 实测渲染框定位（Mapping.tsx），% 坐标直接映射图，缩放免疫。
 */
export const HOTS: Record<string, { l: number; t: number }> = {
  power: { l: 32.24, t: 10.22 },
  up: { l: 44.15, t: 16.99 },
  left: { l: 28.75, t: 23.54 },
  right: { l: 59.06, t: 23.51 },
  ok: { l: 44.0, t: 23.56 },
  down: { l: 44.1, t: 30.01 },
  back: { l: 34.17, t: 36.57 },
  volume_up: { l: 53.74, t: 36.37 },
  home: { l: 34.17, t: 44.58 },
  volume_down: { l: 53.71, t: 44.43 },
  menu: { l: 34.13, t: 52.57 },
  tv: { l: 53.6, t: 52.57 },
  // 语音键：后端未接入、不进 KEYS，但悬停要有气泡说明（用户 2026-11-20 拍板），实测键帽中心。
  mic: { l: 55.48, t: 10.22 },
};
