// halogen 页共用的取值/格式化小工具与确认动作类型（计划 §6.1 自 HalogenPanel.tsx 拆出）。
export const num = (v: unknown): number | null => (typeof v === "number" ? v : null);
export const str = (v: unknown): string => (typeof v === "string" ? v : "");
export const fmt = (n: number | null, d = 0): string =>
  n == null ? "—" : n.toLocaleString("zh-CN", { maximumFractionDigits: d });
export const fmtBytes = (n: number | null): string => {
  if (n == null) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${u[i]}`;
};

/** 需要二次确认的动作；对象形态带目标版本 tag。 */
export type Confirm =
  | null
  | "stop"
  | "restart"
  | "backup"
  | "restore"
  | { switchTag: string }
  /** 删除本地镜像（腾磁盘）：目标 tag。当前版本与运行中镜像由后端拒绝。 */
  | { delTag: string }
  /** 互斥启动确认：另一引擎（值 = 引擎名）还在跑，启动本引擎要先停它。 */
  | { startExclusive: string };

/** 版本号三段比较：缺段当 0。 */
export function cmpVer(a: string, b: string): number {
  const pa = a.split(".").map((x) => parseInt(x, 10) || 0);
  const pb = b.split(".").map((x) => parseInt(x, 10) || 0);
  for (let i = 0; i < 3; i++) {
    if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) - (pb[i] ?? 0);
  }
  return 0;
}
