// 提示词速取面板的底栏（自 PromptPanel 拆出，压单文件行数）。
//
// 左边是 符号盘开关 / 增 / 删（技能条目只读，增删收起；「改」按钮撤了——改内容 = 长按 OK），
// 右边是键盘提示，未授权时换成红字 + 「授权」入口（用户 2026-11-19：面板里直接能发起
// 系统授权，不用去设置页找）。容器可拖窗，按钮各自挡拖。
import { t } from "../lib/i18n";

interface Props {
  /** 项目接管 tab：技能内容只读，增删整组收起。 */
  skillsTab: boolean;
  /** 有选中条目（删除按钮才亮）。 */
  hasTarget: boolean;
  /** 删除两步确认的第一击已落下。 */
  armedDel: boolean;
  /** 临时提示（复制失败 / 注入兜底 / 授权结果），盖在键盘提示上。 */
  note: string;
  a11yOk: boolean;
  /** 符号盘展开态（用户 2026-11-25：左侧按需弹虚拟符号键盘）。 */
  padOpen: boolean;
  onTogglePad: () => void;
  onAdd: () => void;
  onDel: () => void;
  onAskA11y: () => void;
}

export default function PromptFooter(a: Props) {
  return (
    <div className="flex items-center justify-between border-t border-slate-800 px-3 py-1.5">
      <div className="flex gap-1.5">
        <button
          className={`rounded-md px-2 py-1 text-lg ${
            a.padOpen ? "bg-[var(--sel-bg)] text-ink-1 ring-1 ring-inset ring-[var(--sel-ring)]" : "text-ink-2 hover:bg-slate-700/60"
          }`}
          onPointerDown={(e) => e.stopPropagation()}
          onClick={a.onTogglePad}
        >
          {t("prompt.padToggle")}
        </button>
        {a.skillsTab ? null : (
          <>
            <button
              className="rounded-md px-2 py-1 text-lg text-ink-2 hover:bg-slate-700/60"
              onPointerDown={(e) => e.stopPropagation()}
              onClick={a.onAdd}
            >
              {t("prompt.add")}
            </button>
            <button
              className={`rounded-md px-2 py-1 text-lg ${
                a.hasTarget ? "text-[var(--sem-danger-text)] hover:bg-slate-700/60" : "text-ink-4"
              }`}
              title={a.hasTarget ? undefined : t("prompt.noTarget")}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={a.onDel}
            >
              {a.armedDel ? t("prompt.confirmDel") : t("prompt.del")}
            </button>
          </>
        )}
      </div>
      <div className="flex min-w-0 items-center gap-2">
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
    </div>
  );
}
