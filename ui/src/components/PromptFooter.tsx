// 快速面板的底栏。
//
// 「增 / 删」已随浮窗只读化撤掉（用户 2026-11-25 拍板：内容全在遥控器页 → 快速面板档编辑，
// 浮窗只在注入前临时改文本，要留下走右栏「追加到 tab」）。
// 现在整条只剩：临时提示（装入待注入 / 动作结果 / 注入兜底 / 授权结果）+ 未授权时的红字与「授权」入口。
// 容器可拖窗，按钮挡拖。
import { t } from "../lib/i18n";

interface Props {
  /** 临时提示（盖在键盘提示上）。 */
  note: string;
  a11yOk: boolean;
  onAskA11y: () => void;
}

export default function PromptFooter(a: Props) {
  return (
    // 容器**不**挡拖：拖窗起手挂在外层 prompt-card 上，这里一 stopPropagation 就整条底栏拖不动
    // （用户 2026-11-26 实报「底栏拖不动面板」）。只有里面的「授权」钮要挡拖，别一按就拖窗。
    <div className="flex items-center justify-end border-t border-slate-800 px-3 py-1.5">
      <span
        className={`truncate text-lg ${a.a11yOk ? "text-ink-4" : "text-[var(--sem-danger-text)]"}`}
        title={a.a11yOk ? undefined : t("prompt.cfgA11y")}
      >
        {a.note || (a.a11yOk ? t("prompt.kbdHint") : t("prompt.a11yMissing"))}
      </span>
      {!a.a11yOk && (
        <button
          className="shrink-0 rounded-md px-2 py-1 text-lg text-ink-1 ring-1 ring-inset ring-[var(--sel-ring)] hover:bg-slate-700/60"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={a.onAskA11y}
        >
          {t("prompt.a11yAsk")}
        </button>
      )}
    </div>
  );
}
