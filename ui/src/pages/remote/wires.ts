// 键卡 → 热点的连线几何（从 Mapping 拆出压行数）：三次贝塞尔主线 + 箭头两笔，
// 口径与原型 drawWires 一致。
/** 一条键行 → 热点的连线。 */
export interface Wire {
  id: string;
  d: string;
  arrow: string;
  /** 该键三槽是否至少有一个映射：有则平时常显虚线，无则不连（用户 2026-11-20 拍板）。 */
  mapped: boolean;
}

/** 卡侧边中点 (x1,y1) → 热点侧边中点 (x2,hy) 的贝塞尔主线 + 箭头路径；left = 卡在左列。 */
export function wirePath(x1: number, y1: number, x2: number, hy: number, left: boolean) {
  const c = left ? 56 : -56;
  return {
    d: `M${x1} ${y1}C${x1 + c} ${y1},${x2 - c} ${hy},${x2} ${hy}`,
    arrow: `M${x1} ${y1}l${left ? 11 : -11} -6m${left ? -11 : 11} 6l${left ? 11 : -11} 6`,
  };
}
