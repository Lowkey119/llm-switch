// 按键映射区的键表与热点坐标（docs/ui_02/mapping.html 原型 KEYS/HOTS 回灌）。
//
// KEYS 只收后端 RemoteButton::ALL 真实支持的 12 键（id 与 buttons.rs::id() 一一对应）；
// 语音键后端未接入，不画（法条：不造假状态）。
// HOTS 按实物图标定的百分比坐标。原图 remote-hi.png（1248×2752）四周有大片透明边、
// 键区以下全是空机身——2026-11-24 按用户蓝线拍板裁到机身 bbox + TV/菜单键行底
// （remote-hi.webp 现为 622×1432），下面数值是裁前标定按偏移/缩放换算到裁后图的口径。
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

/** 图宽高比换算因子：圆直径占宽 w% 时，占高 = w × HOT_F（裁切图 622×1574）。 */
export const HOT_F = 622 / 1574;

/** 统一高亮圆直径（占图宽 %）：实拍全部键帽直径（含方向键位）的均值，
 *  用户 2026-11-20 拍板——不搞一键一圈，一个尺寸通吃。
 *  2026-11-24 随裁切图重算：原 13.4% × 1248px ≈ 167px 物理直径 ÷ 新宽 622px。 */
export const HOT_D = 26.89;

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
 * 2026-11-24 随裁切图重标定：原图裁去四周透明边与蓝线以下空机身（crop 322,205,622×1432），
 * 坐标按 (旧% × 原尺寸 − 裁切偏移) ÷ 新尺寸 换算，键帽物理位置不变。
 */
export const HOTS: Record<string, { l: number; t: number }> = {
  power: { l: 12.92, t: 4.84 },
  up: { l: 36.82, t: 16.68 },
  left: { l: 5.92, t: 28.13 },
  right: { l: 66.73, t: 28.08 },
  ok: { l: 36.51, t: 28.17 },
  down: { l: 36.72, t: 39.45 },
  back: { l: 16.79, t: 50.92 },
  volume_up: { l: 56.06, t: 50.57 },
  home: { l: 16.79, t: 64.92 },
  volume_down: { l: 56.0, t: 64.66 },
  menu: { l: 16.71, t: 78.89 },
  tv: { l: 55.78, t: 78.89 },
  // 语音键：后端未接入、不进 KEYS，但悬停要有气泡说明（用户 2026-11-20 拍板），实测键帽中心。
  mic: { l: 59.55, t: 4.84 },
};
