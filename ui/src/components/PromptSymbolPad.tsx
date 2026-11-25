// 快速面板的虚拟符号键盘（用户 2026-11-24）：遥控器只有五键、语音输入法打不准
// `/`、`!`、`^C` 这类编码 agent 常用字符，所以按需从面板左侧弹一块符号盘，
// 点一下即把字符插进右栏编辑框的光标处（不是打字通道，纯快捷插入）。
//
// 两组：常用符号（路径/命令行高频）+ 控制字符（readline/emacs 风格，^A ^C ^K…，
// 终端里就是 Ctrl+字母，语音念不出来）。布局是网格按钮，方向键也能走（原生 button）。
import { t } from "../lib/i18n";

const SYMBOLS = ["/", "!", "?", ".", ",", ":", ";", "-", "_", "@", "#", "$", "%", "&", "*", "|", "~", "<", ">", "=", "+", "\\", "`"];
const CTRL = ["^A", "^B", "^C", "^D", "^E", "^F", "^K", "^N", "^P", "^R", "^T", "^U", "^W", "^Y"];

export default function PromptSymbolPad({ onInsert }: { onInsert: (text: string) => void }) {
  return (
    <div
      className="flex w-[220px] shrink-0 flex-col gap-2 overflow-y-auto border-r border-slate-800 px-2 py-2"
      onPointerDown={(e) => e.stopPropagation()}
    >
      <p className="text-lg text-ink-4">{t("prompt.padHint")}</p>
      <div>
        <p className="mb-1 text-lg text-ink-3">{t("prompt.padSymbols")}</p>
        <div className="grid grid-cols-6 gap-1">
          {SYMBOLS.map((s) => (
            <button
              key={s}
              type="button"
              onClick={() => onInsert(s)}
              className="grid h-9 place-items-center rounded-md border border-slate-700 bg-slate-800/60 font-mono text-lg text-ink-1 hover:border-[var(--sel-ring)] hover:text-[var(--key-hl)]"
            >
              {s}
            </button>
          ))}
        </div>
      </div>
      <div>
        <p className="mb-1 text-lg text-ink-3">{t("prompt.padCtrl")}</p>
        <div className="grid grid-cols-4 gap-1">
          {CTRL.map((s) => (
            <button
              key={s}
              type="button"
              onClick={() => onInsert(s)}
              className="grid h-9 place-items-center rounded-md border border-slate-700 bg-slate-800/60 font-mono text-lg text-ink-1 hover:border-[var(--sel-ring)] hover:text-[var(--key-hl)]"
            >
              {s}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
