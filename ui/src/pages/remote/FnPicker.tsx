// 系统功能预设与芯片组（用户 2026-11-25 拍板：整页功能编辑面板删除——槽位点录后
// 直接去下方三页签坞的虚拟键盘页绑定，这里只留预设目录与芯片组组件）。
// macOS 集在此；其余 OS 见 fnPresets.ts，按部署系统自动切换。
import { os } from "../../lib/key";
import { t } from "../../lib/i18n";
import { fmtShortcut, type Shortcut } from "./shortcut";
import { fnCategoriesFor } from "./fnPresets";

/** 功能分类：分类名 + 芯片（kVK 组合）。 */
export interface FnCategory {
  nameKey: string;
  items: { nameKey: string; sc: Shortcut }[];
}

/**
 * 常用功能按键（用户 2026-11-21 拍板：分类网格摆在键盘下方，点一下 = 直接填入当前槽）。
 * 键码为 kVK（与后端注入同一口径）；媒体键 72/73/74 与前端 KVK_LABEL、
 * 后端 inject_shortcut 媒体特判同源。
 */
/** macOS 系统功能预设（其余 OS 见 fnPresets.ts，按部署系统自动切换）。 */
const FN_CATEGORIES_MAC: FnCategory[] = [
  {
    nameKey: "remote.fn.catWindow",
    items: [
      { nameKey: "remote.fn.mission", sc: { mods: 1, key: 126 } },
      { nameKey: "remote.fn.windows", sc: { mods: 1, key: 125 } },
      { nameKey: "remote.fn.spaceLeft", sc: { mods: 1, key: 123 } },
      { nameKey: "remote.fn.spaceRight", sc: { mods: 1, key: 124 } },
      { nameKey: "remote.fn.desktop", sc: { mods: 0, key: 103 } },
      { nameKey: "remote.fn.minimize", sc: { mods: 8, key: 46 } },
      { nameKey: "remote.fn.minimizeAll", sc: { mods: 12, key: 46 } },
      { nameKey: "remote.fn.closeWindow", sc: { mods: 8, key: 13 } },
      { nameKey: "remote.fn.fullscreen", sc: { mods: 9, key: 3 } },
    ],
  },
  {
    nameKey: "remote.fn.catMedia",
    items: [
      { nameKey: "remote.fn.volUp", sc: { mods: 0, key: 72 } },
      { nameKey: "remote.fn.volDown", sc: { mods: 0, key: 73 } },
      { nameKey: "remote.fn.mute", sc: { mods: 0, key: 74 } },
    ],
  },
  {
    nameKey: "remote.fn.catApp",
    items: [
      { nameKey: "remote.fn.hideApp", sc: { mods: 8, key: 4 } },
      { nameKey: "remote.fn.hideOthers", sc: { mods: 12, key: 4 } },
      { nameKey: "remote.fn.quitApp", sc: { mods: 8, key: 12 } },
      { nameKey: "remote.fn.forceQuit", sc: { mods: 12, key: 53 } },
    ],
  },
  {
    nameKey: "remote.fn.catSystem",
    items: [
      { nameKey: "remote.fn.spotlight", sc: { mods: 8, key: 49 } },
      { nameKey: "remote.fn.emoji", sc: { mods: 9, key: 49 } },
      { nameKey: "remote.fn.inputSwitch", sc: { mods: 1, key: 49 } },
      { nameKey: "remote.fn.lock", sc: { mods: 9, key: 12 } },
      { nameKey: "remote.fn.screenshotSel", sc: { mods: 10, key: 23 } },
      { nameKey: "remote.fn.screenshotFull", sc: { mods: 10, key: 20 } },
      { nameKey: "remote.fn.emptyTrash", sc: { mods: 10, key: 51 } },
      { nameKey: "remote.fn.showHidden", sc: { mods: 10, key: 47 } },
    ],
  },
  {
    nameKey: "remote.fn.catZoom",
    items: [
      // macOS 辅助功能「缩放视图」原生快捷键（系统设置里启用「使用键盘快捷键缩放」后生效）：
      // ⌘⌥= 放大、⌘⌥- 缩小、⌘⌥8 开关。走普通键盘事件注入，系统自己响应。
      { nameKey: "remote.fn.zoomIn", sc: { mods: 12, key: 24 } },
      { nameKey: "remote.fn.zoomOut", sc: { mods: 12, key: 27 } },
      { nameKey: "remote.fn.zoomToggle", sc: { mods: 12, key: 28 } },
    ],
  },
];

/** 系统功能预设按部署 OS 取集（用户 2026-11-25：跨平台，预设随系统变）。 */
export const FN_CATEGORIES: FnCategory[] = fnCategoriesFor(os, FN_CATEGORIES_MAC);

/** 功能芯片组（虚线分组框）：坞虚拟键盘页摆键盘下方；onPick = 填入当前目标槽。
 *  mapped = 已绑到某遥控键槽的芯片 nameKey 集合（绿框绿字标出）；disabled = 只读展示。 */
export function FnChips({
  cats,
  onPick,
  className = "",
  mapped,
  disabled = false,
}: {
  cats: FnCategory[];
  onPick: (sc: Shortcut) => void;
  className?: string;
  mapped?: Set<string>;
  disabled?: boolean;
}) {
  return (
    <div className={`flex flex-wrap gap-2.5 ${className}`}>
      {cats.map((c) => (
        <div
          key={c.nameKey}
          aria-label={t(c.nameKey)}
          className="flex flex-wrap gap-1.5 rounded-[10px] border border-dashed border-slate-600 px-3 py-2"
        >
          {c.items.map((p) => (
            <button
              key={p.nameKey}
              type="button"
              disabled={disabled}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onPick(p.sc)}
              aria-label={`${t(c.nameKey)}：${t(p.nameKey)} ${fmtShortcut(p.sc)}${mapped?.has(p.nameKey) ? "（已映射）" : ""}`}
              className={`flex items-baseline gap-1.5 rounded-md border px-2.5 py-1 text-sm transition-colors ${
                mapped?.has(p.nameKey)
                  ? "border-[var(--sem-ok-text)] text-[var(--sem-ok-text)]"
                  : "border-slate-700 text-ink-1"
              } ${disabled ? "cursor-default" : "hover:border-[var(--key-hl-text)] hover:bg-[var(--key-hl)]/10"}`}
            >
              {t(p.nameKey)}
              <span className="text-2xs tabular-nums text-ink-4">{fmtShortcut(p.sc)}</span>
            </button>
          ))}
        </div>
      ))}
    </div>
  );
}
