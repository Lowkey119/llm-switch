// 版本号徽章（UI-AUDIT-2 F-1）：以前整个 UI 一处都读不到自己跑的是哪一版——
// 报障、比对装机、确认升级是否生效，全都只能回去翻 tauri.conf.json。
// 挂在顶栏页名旁边：每一页都在同一个位置，永远看得见，且不吃走位。
// UI-13（用户 2026-11-16）：取消点击版本号跳设置页；版本号后方挂「有更新」标签——
// 更新检测代码后续接入，届时该标签只在检测到新版本时显示，现在是占位常显。
import { formatVersion, useAppVersion } from "../lib/version";

export function VersionChip({ className }: { className?: string }) {
  const v = formatVersion(useAppVersion());
  return (
    <span className={`flex items-center gap-1${className ? ` ${className}` : ""}`}>
      <span
        className="shrink-0 rounded border border-slate-700 bg-slate-800/60 px-1.5 py-px font-mono text-2xs leading-snug text-ink-3"
        title={`llm-switch ${v}`}
      >
        {v}
      </span>
      <span
        className="shrink-0 rounded border border-amber-500/60 bg-amber-500/15 px-1.5 py-px text-2xs leading-snug text-[var(--sem-busy-text)]"
        title="检测到新版本，点击版本号旁的更新按钮升级（更新功能开发中）"
      >
        有更新
      </span>
    </span>
  );
}
