// 快速面板右栏：待执行（stage）/ 技能文档 / 配置 JSON / 预览 四态，**全部只读**。
//
// 两段式执行口径（用户 2026-11-26 定稿）：中栏 OK 把内容**装入**右栏，再按一次 OK 才真注入
// 目标应用；右栏不再是编辑框——它只是「即将发出去的东西」的读数窗。
// 为什么改成只读（用户 2026-11-26 拍板）：浮窗用遥控器操作，编辑框会抢焦点，
// 焦点一跑方向键就废；现在**焦点始终留在条目上**，OK 随时能确认注入。
// 要改内容去遥控器页改（那里才是内容主场）；要留下右栏这段临时正文走「追加到 tab」。
//
// 正文用 <pre> 不用 textarea：只读展示没焦点，天然拿不到键盘（select-text 保留可选中复制）。
// 字号 text-4xl（44px，用户要求比原来大一倍），只给这类远距离读数用。
import { t } from "../lib/i18n";
import type { Action } from "../pages/remote/action";
import type { Row } from "./promptRows";

export type RightMode = "stage" | "doc" | "config" | "preview" | "none";

/**
 * 右栏已装入的待执行内容：注入 / 命令 / 执行（引用动作、单发动作）。
 * 四态一律只读显示；`action` 的 confirm 两步确认仍靠 OK 连按。
 */
export type Stage =
  | { kind: "inject"; text: string }
  | { kind: "action"; text: string; label: string; confirm: boolean; id: string }
  | { kind: "run"; what: "macro"; label: string; id: string }
  | { kind: "run"; what: "action"; label: string; action: Action };

/**
 * 右栏四态到底显示什么（待执行 stage > 技能文档 > 配置 JSON > 预览）。
 * 从 PromptPanel 拆过来：抬头、按键提示、正文与配置预览都是右栏自己的事，面板只喂原料。
 */
export function rightView(o: {
  stage: Stage | null;
  /** confirm 动作已武装（等第二下 OK）：抬头改成确认提示。 */
  armed: boolean;
  /** 中栏当前行（预览态的正文来源）。 */
  cur: Row | undefined;
  /** 当前 tab 的 type（config 档整栏给 JSON）。 */
  tabType?: string;
  /** quickpanel.json 读数（路径 + 解析失败原因）。 */
  cfg: { path: string; error: string | null } | null;
  tabs: unknown;
}): { mode: RightMode; title: string; hint: string; value: string; configText: string } {
  const { stage, cur, tabType, cfg } = o;
  const showDoc = cur?.kind === "skill";
  const mode: RightMode = stage ? "stage" : showDoc ? "doc" : tabType === "config" ? "config" : cur ? "preview" : "none";
  let title = t("prompt.preview");
  let hint = "";
  if (stage) {
    title =
      stage.kind === "action"
        ? t("prompt.stageAction")
        : stage.kind === "run"
          ? t("prompt.stageRun")
          : t("prompt.stageInject");
    hint = o.armed ? t("prompt.armConfirm") : t("prompt.stageHint");
  } else if (showDoc) {
    title = t("prompt.skillDoc");
  } else if (tabType === "config") {
    title = t("prompt.cfgJson");
    hint = t("prompt.cfgHint");
  } else if (cur?.kind === "char") {
    title = cur.note ? `${cur.content} ${cur.note}` : cur.content;
  } else if (cur?.kind === "action") {
    title = cur.content;
  }
  const value = stage ? (stage.kind === "run" ? stage.label : stage.text) : (cur?.content ?? "");
  const configText = `${cfg?.path ?? ""}${cfg?.error ? `\n${t("prompt.cfgErr")}${cfg.error}` : ""}\n\n${JSON.stringify({ tabs: o.tabs }, null, 2)}`;
  return { mode, title, hint, value, configText };
}

interface Props {
  mode: RightMode;
  /** 抬头（待注入 / 待执行 / 技能文档 / 配置…）。 */
  title: string;
  /** 抬头右侧的按键口径提示。 */
  hint: string;
  /** stage/preview 的文本。 */
  value: string;
  /** 可追加到的 tab（空数组 = 这一态不给追加）。 */
  appendTabs: { key: string; label: string }[];
  appendTab: string;
  onAppendTab: (key: string) => void;
  onAppend: () => void;
  /** 技能文档只读文本（mode=doc）。 */
  doc: string;
  /** quickpanel.json 只读文本（mode=config）。 */
  configText: string;
}

export default function PromptRightPane(a: Props) {
  // 待注入正文：44px 大字（用户 2026-11-26：比原来大一倍），远距离一眼读完。
  const body =
    "min-h-0 flex-1 select-text overflow-auto whitespace-pre-wrap break-all rounded-md bg-slate-800/40 px-3 py-2 text-4xl leading-relaxed text-ink-1";
  return (
    <div
      className="flex w-[400px] shrink-0 flex-col gap-1.5 border-l border-slate-800 px-3 pb-2"
      onPointerDown={(e) => e.stopPropagation()}
    >
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-lg text-ink-2 font-semibold">{a.title}</span>
        {a.hint && <span className="shrink-0 text-base text-ink-4">{a.hint}</span>}
      </div>
      {a.mode === "doc" ? (
        <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-all rounded-md bg-slate-800/60 px-3 py-2 text-lg leading-relaxed text-ink-2">
          {a.doc}
        </pre>
      ) : a.mode === "config" ? (
        <pre className="min-h-0 flex-1 overflow-auto whitespace-pre-wrap break-all rounded-md bg-slate-800/60 px-3 py-2 font-mono text-base leading-relaxed text-ink-2">
          {a.configText}
        </pre>
      ) : a.mode === "none" ? (
        <div className="flex min-h-0 flex-1 items-center justify-center text-lg text-ink-4">—</div>
      ) : (
        <pre className={body}>{a.value}</pre>
      )}
      {/* 唯一的写回入口：把右栏这段临时正文落成指定 tab 的一条内容（OK 长按同义）。 */}
      {a.appendTabs.length > 0 && (
        <div className="flex shrink-0 items-center gap-1.5">
          <select
            className="min-w-0 flex-1 rounded-md border border-slate-700 bg-slate-800 px-2 py-1 text-base text-ink-2"
            value={a.appendTab}
            aria-label={t("prompt.appendPick")}
            onPointerDown={(e) => e.stopPropagation()}
            onChange={(e) => a.onAppendTab(e.target.value)}
          >
            {a.appendTabs.map((tb) => (
              <option key={tb.key} value={tb.key}>
                {tb.label}
              </option>
            ))}
          </select>
          <button
            className="shrink-0 rounded-md px-2 py-1 text-base text-ink-1 ring-1 ring-inset ring-[var(--sel-ring)] hover:bg-slate-700/60"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={a.onAppend}
          >
            {t("prompt.append")}
          </button>
        </div>
      )}
    </div>
  );
}
