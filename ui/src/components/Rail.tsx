// 边条（ui_01 原型回灌 U2）：纯图标轨道（用户 2026-11-20 拍板：文字列整体删除，
// 页名与 ⌘1–9 全走悬停 tooltip，让出最多屏幕空间；折叠切换随之没有存在意义）。
//
// 为什么从 App.tsx 拆出来：主窗外壳把边条、顶栏、路由三件事挤在一个文件里，
// App.tsx 已经 425 行（ui-budget 也点名建议再拆）。分组渲染与顶栏各有各的变化
// 理由，分开之后才各改各的。**顶栏一行没动**——计划出局项写明顶栏保持原代码。
//
// 颜色刻意不引入原型的 --accent-soft / --bg-press / --bg-hover 那一套：本仓已有自己的
// 选中态词汇（--sel-bg / --sel-ring）与强调色（cyan 系，SkillMatrix.linked 同一写法），
// 再造一套属平行实现（AGENTS 铁律 3）。原型截图里每个入口后面那圈灰底是它自己漏了
// button 背景重置（color-scheme: dark 下的 UA 按钮底色），不是设计，没带过来。
import type { ReactNode } from "react";
import { t } from "../lib/i18n";
import { combo } from "../lib/key";
import type { PageKey } from "../lib/pages";
import {
  IconAgents,
  IconBackend,
  IconMonitor,
  IconRemote,
  IconSettings,
} from "./icons";

type RailIcon = typeof IconBackend;

/** 边条入口（融合原型回灌：9 页并 5 页，观测/后端/代理各吞并同源页）。顺序即 ⌘1–5 的顺序；
 *  遥控器挪到设置上一位（用户 2026-11-25 拍板，取代 11-21 的置顶口径）。 */
export const NAV_GROUPS: { labelKey: string; items: { key: PageKey; icon: RailIcon }[] }[] = [
  {
    labelKey: "nav.group.main",
    items: [
      { key: "observe", icon: IconMonitor },
      { key: "backends", icon: IconBackend },
      { key: "agents", icon: IconAgents },
      { key: "remote", icon: IconRemote },
      { key: "settings", icon: IconSettings },
    ],
  },
];

/** 拍平后的顺序 = ⌘1–9 的序号依据（App 的快捷键也读它）。 */
export const NAV_ORDER: PageKey[] = NAV_GROUPS.flatMap((g) => g.items.map((i) => i.key));

const pageName = (k: PageKey) => t(`page.${k}`);

/**
 * 悬停飞标：原生 title 气泡在 WKWebView 里不渲染（UI-10），只能自己画。
 * 挂在带 `group/nav` 的按钮里。
 */
function Flyout({ children }: { children: ReactNode }) {
  return (
    <span
      aria-hidden
      className="pointer-events-none absolute left-full top-1/2 z-50 ml-2 -translate-y-1/2 whitespace-nowrap rounded border border-[color-mix(in_srgb,CanvasText_25%,transparent)] bg-[Canvas] px-2 py-1 text-2xs text-[CanvasText] opacity-0 shadow-lg transition-opacity group-hover/nav:opacity-100"
    >
      {children}
    </span>
  );
}

/**
 * 动态品牌标（用户 2026-11-20 拍板重画，不再用位图 logo）：
 * 粗虚线圆慢转（深绿 --brand-ring）+ 彩色「llm」呼吸（蓝→绿→黄渐变，与海报字标同族）。
 * 挂在顶栏页名左侧（边条 56px 轨道里 28px 太小，llm 看不清）。
 */
export function BrandMark() {
  return (
    <svg viewBox="0 0 48 48" className="h-9 w-9 shrink-0" role="img" aria-label="llm-switch">
      <defs>
        <linearGradient id="brand-llm-g" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#00cfff" />
          <stop offset="0.5" stopColor="#00ff7f" />
          <stop offset="1" stopColor="#ffd000" />
        </linearGradient>
      </defs>
      <circle
        className="brand-ring"
        cx="24"
        cy="24"
        r="20"
        fill="none"
        stroke="var(--brand-ring)"
        strokeWidth="5"
        strokeDasharray="9 7"
        strokeLinecap="round"
      />
      <text
        className="brand-llm"
        x="24"
        y="25"
        textAnchor="middle"
        dominantBaseline="central"
        fontSize="18"
        fontWeight="900"
        fill="url(#brand-llm-g)"
      >
        llm
      </text>
    </svg>
  );
}

export function Rail({
  page,
  onNavigate,
  lang,
  onToggleLang,
}: {
  page: PageKey;
  onNavigate: (p: PageKey) => void;
  lang: string;
  onToggleLang: () => void;
}) {
  return (
    // overflow-hidden 会裁掉 Flyout（absolute left-full 探出轨道右侧，用户 2026-11-25 截图拍板移除）：
    // 轨道定宽 56px，内容不会横向溢出，这个 hidden 本来就没有防护对象。
    <aside
      aria-label={t("rail.aria")}
      className="rail flex shrink-0 flex-col gap-1 border-r border-slate-800 bg-slate-900/50 px-2 py-3"
    >
      {NAV_GROUPS.map((group, gi) => (
        <div
          key={group.labelKey}
          className={`flex flex-col gap-0.5${gi > 0 ? " mt-2 border-t border-slate-800 pt-2" : ""}`}
        >
          {/* 组名删掉（用户 2026-11-19）：九项页面名本身就自解释，组标题是重复信息；
              分组靠分隔线保留节奏。 */}
          {group.items.map(({ key, icon: Icon }) => (
            <RailItem
              key={key}
              active={page === key}
              icon={Icon}
              label={pageName(key)}
              shortcut={combo(String(NAV_ORDER.indexOf(key) + 1))}
              onClick={() => onNavigate(key)}
            />
          ))}
        </div>
      ))}

      {/* 底部功能区：语言切换。版本入口已按用户 2026-11-20 拍板搬回顶栏（TopVersionChip）。 */}
      <div className="mt-auto flex border-t border-slate-800 pt-2">
        {/* 中英文切换（用户 2026-11-19 拍板从顶栏搬来）：显示**目标**语言——
            中文态写 EN，英文态写「中文」，按钮说的是点下去会切到哪边。 */}
        <button
          type="button"
          onClick={onToggleLang}
          aria-label={t("top.langAria")}
          className="grid shrink-0 place-items-center rounded-lg px-2.5 p-2 text-2xs font-medium text-ink-3 transition-colors hover:bg-slate-800/60 hover:text-ink-1"
        >
          {t(lang === "zh" ? "app.langBtnEn" : "app.langBtnZh")}
        </button>
      </div>
    </aside>
  );
}

function RailItem({
  active,
  icon: Icon,
  label,
  shortcut,
  onClick,
}: {
  active: boolean;
  icon: RailIcon;
  label: string;
  shortcut: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      aria-label={`${label} ${shortcut}`}
      className={`group/nav relative flex w-full items-center justify-center rounded-lg p-2 text-sm font-medium transition-colors ${
        active
          ? "bg-[var(--sel-bg)] text-cyan-300"
          : "text-ink-3 hover:bg-slate-800/60 hover:text-ink-1"
      }`}
    >
      <Icon className="shrink-0" />
      {/* 页名 + 快捷键都在悬停 tooltip 里（文字列已删，用户 2026-11-20 拍板）。 */}
      <Flyout>
        {label}
        <span className="ml-1 text-ink-4">{shortcut}</span>
      </Flyout>
      {/* 选中态补一条形状线索（UI-AUDIT-2 B-3）：以前只靠底色变化，浅底上那一档
          底色差几乎看不出来；贴边短杠与底色两路冗余，不靠颜色说话。 */}
      {active && (
        <span
          aria-hidden
          className="absolute -left-2 top-1/2 h-5 w-[3px] -translate-y-1/2 rounded-r-full bg-cyan-300"
        />
      )}
    </button>
  );
}
