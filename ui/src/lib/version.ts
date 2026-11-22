// 版本号（UI-AUDIT-2 F-1）：真值只有一处——向 Tauri 要 `tauri.conf.json` 里的 version，
// 那才是打进包名、装机后唯一可信的那一个。`ui/package.json` 与 `Cargo.toml` 里的版本
// 是发布时手工同步的副本，构建期抄一份只会多一个会漂移的真相源，所以不抄。
//
// 读不到 = 不在 Tauri 里跑（纯浏览器 `npm run dev`），保持回落值即可，不弹错刷屏。
// 全窗共用这一份：多个组件各自 await getVersion() 会各发一次 IPC，且首帧各自闪一下 dev。
import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";

/** 拿不到版本时的占位（只在浏览器 dev 里见得到）。 */
export const VERSION_FALLBACK = "dev";

let current = VERSION_FALLBACK;
let started = false;
const subs = new Set<() => void>();

function startOnce() {
  if (started) return;
  started = true;
  getVersion()
    .then((v) => {
      if (!v || v === current) return;
      current = v;
      for (const fn of subs) fn();
    })
    .catch(() => {
      /* 非 Tauri 环境：保持回落值 */
    });
}

/** 带 v 前缀的展示形态；回落值不加前缀，免得读成「v dev」。 */
export function formatVersion(v: string): string {
  if (!/^\d/.test(v)) return v;
  return v.startsWith("v") ? v : `v${v}`;
}

/** 订阅全局版本号（首帧可能是回落值，拿到真值后一起刷新）。 */
export function useAppVersion(): string {
  const [v, setV] = useState(current);
  useEffect(() => {
    const fn = () => setV(current);
    subs.add(fn);
    startOnce();
    fn();
    return () => {
      subs.delete(fn);
    };
  }, []);
  return v;
}
