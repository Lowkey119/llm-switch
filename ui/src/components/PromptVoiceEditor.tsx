// 语音编辑态的回显编辑框（计划 2026-11-26「语音编辑态」）。
//
// 只在语音录入时挂载（不编辑就完全不占位，门控提示改挂条目列顶部状态行）。
// 焦点纪律：进入即 focus，方向键/退格都交浏览器默认（textarea 自己管光标），
// 全局键盘监听在编辑态只留 OK / Esc 两键（见 promptKeys 的 voiceEdit 分支）。
// ⌃K / ⌃U 在这儿自己实现等效编辑——网页 textarea 不认终端那套删到行首/行尾。
import { useEffect, useRef } from "react";
import { t } from "../lib/i18n";

interface Props {
  text: string;
  onChange: (v: string) => void;
  /** OK：写剪贴板 + 注入 + 退出编辑态（面板层实现）。 */
  onSend: () => void;
  /** Esc：退出编辑态但**保留内容**（下次语音接着追加）。 */
  onExit: () => void;
}

export default function PromptVoiceEditor(a: Props) {
  const ref = useRef<HTMLTextAreaElement>(null);
  // 挂载即抢焦点：遥控器方向键要立刻能移光标，晚一拍就是「按了没反应」。
  useEffect(() => {
    ref.current?.focus();
  }, []);
  return (
    <div className="flex shrink-0 flex-col gap-1 border-t border-slate-800 px-3 py-2" onPointerDown={(e) => e.stopPropagation()}>
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-lg font-semibold text-ink-2">{t("prompt.voiceEditing")}</span>
        <span className="shrink-0 text-base text-ink-4">{t("prompt.voiceEditHint")}</span>
      </div>
      <textarea
        ref={ref}
        value={a.text}
        onChange={(e) => a.onChange(e.target.value)}
        aria-label={t("prompt.voiceEditing")}
        className="min-h-20 w-full resize-none rounded-md border border-slate-700 bg-slate-800/60 px-2 py-1.5 text-xl leading-relaxed text-ink-1 focus:ring-2 focus:ring-inset focus:ring-[var(--sel-ring)]"
        onKeyDown={(e) => {
          // ⌃K 删到行尾、⌃U 删到行首：终端手感在网页 textarea 里的等效实现。
          if (e.key === "k" && e.ctrlKey) {
            e.preventDefault();
            const el = e.currentTarget;
            a.onChange(el.value.slice(0, el.selectionStart));
            return;
          }
          if (e.key === "u" && e.ctrlKey) {
            e.preventDefault();
            const el = e.currentTarget;
            a.onChange(el.value.slice(el.selectionEnd));
            return;
          }
          if (e.key === "Enter" && !e.shiftKey) {
            // OK 在遥控器上就是 Enter：编辑态里它是发送键，不许换行。
            e.preventDefault();
            a.onSend();
            return;
          }
          if (e.key === "Escape") {
            e.preventDefault();
            a.onExit();
          }
        }}
      />
    </div>
  );
}
