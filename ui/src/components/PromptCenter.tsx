// 快速面板中栏：条目列表（提示词/字符/动作/技能/配置同构渲染）。
//
// 从 PromptPanel 拆出（单文件行数预算）：这里只管「怎么把一行画出来」，
// 选中态（hi）、装入右栏等编排仍归 PromptPanel——中栏不碰执行口径。
// **搜索框已去掉**（用户 2026-11-26 拍板）：浮窗是遥控器按键操作的远距离面板，
// 搜索框要打字、要占焦点，与「焦点始终在条目上、OK 随时确认注入」相冲。
// 内容管理归遥控器页，浮窗只负责选与发。
// 远距大字一律取 styles.css 字号阶梯：条目 text-xl、字符 text-2xl、次要说明 text-lg/base。
import { t } from "../lib/i18n";
import { IconCheck } from "./icons";
import type { Row } from "./promptRows";

interface Props {
  rows: Row[];
  /** 当前高亮行下标（方向键与点击共用同一个 hi）。 */
  hi: number;
  /** 点一行：记高亮（焦点不抢走，方向键接着用）。 */
  onPick: (i: number) => void;
  /** 配置读取失败（与「没有条目」是两回事，错误必须长得像错误）。 */
  error: boolean;
  loading: boolean;
  /** 空列表时的一句话（这个 tab 本来就空 / 后端没读到）。 */
  emptyNote: string;
  /** 「最近发送」批量选中的原文（只在 recent tab 传）。 */
  picked?: string[];
}

export default function PromptCenter({
  rows,
  hi,
  onPick,
  error,
  loading,
  emptyNote,
  picked,
}: Props) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-2 pt-2">
        {error ? (
          <div className="py-6 text-center text-lg text-ink-3">{t("prompt.loadFail")}</div>
        ) : rows.length === 0 ? (
          <div className="py-6 text-center text-lg text-ink-3">{loading ? t("prompt.loading") : emptyNote}</div>
        ) : (
          <ul className="space-y-1.5">
            {rows.map((r, i) => (
              <li key={r.id}>
                <button
                  data-row={r.id}
                  className={`flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-xl leading-relaxed text-ink-1 ${
                    i === hi ? "bg-[var(--sel-bg)] ring-1 ring-inset ring-[var(--sel-ring)]" : "hover:bg-slate-700/40"
                  } ${picked?.includes(r.content) ? "ring-1 ring-inset ring-[var(--key-hl-text)]" : ""} ${r.kind === "cfginfo" ? "text-ink-3" : ""}`}
                  onClick={() => onPick(i)}
                >
                  {picked?.includes(r.content) && <IconCheck className="h-4 w-4 shrink-0 text-[var(--key-hl-text)]" aria-hidden />}
                  <RowBody r={r} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

/** 一行的正文：按条目种类换形状（字符大字 + 备注、动作带命令、引用带动作名、其余折行正文）。 */
function RowBody({ r }: { r: Row }) {
  if (r.kind === "char") {
    return (
      <span>
        <span className="font-mono text-2xl">{r.content}</span>
        {r.note && <span className="ml-3 text-lg text-ink-3">{r.note}</span>}
      </span>
    );
  }
  if (r.kind === "action") {
    return (
      <span className="flex min-w-0 items-baseline gap-3">
        <span className="shrink-0">{r.content}</span>
        <span className="truncate font-mono text-base text-ink-3">
          {r.cmd}
          {r.confirm ? ` · ${t("prompt.needConfirm")}` : ""}
        </span>
      </span>
    );
  }
  if (r.kind === "macro" || r.kind === "step") {
    // 只留动作名（用户 2026-11-27：说明性文字不要），字号对齐字符行并加粗，长了折行。
    return <span className="break-words text-2xl font-bold">{r.content}</span>;
  }
  if (r.kind === "cfginfo") {
    return (
      <span>
        <span>{r.content}</span>
        {r.note && <span className="ml-3 text-base text-ink-3">{r.note}</span>}
      </span>
    );
  }
  return <span className="line-clamp-2 whitespace-pre-wrap break-all">{r.content}</span>;
}
