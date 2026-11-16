import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  IconAgents,
  IconBackend,
  IconLogs,
  IconMonitor,
  IconSettings,
  IconSkills,
  IconUsage,
  IconConsole,
  IconMoon,
  IconSun,
  IconFloat,
  IconFloatHide,
  IconSysFloat,
} from "./components/icons";
import { getTheme, subscribeTheme, toggleTheme, type Theme } from "./lib/theme";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { useToast } from "./components/Toast";
import { Freshness } from "./components/Freshness";
import { TaskTray } from "./components/TaskTray";
import { GuidedTour } from "./components/GuidedTour";
import { CommandPalette } from "./components/CommandPalette";
import { VersionChip } from "./components/VersionBadge";
import { StatusPill } from "./components/StatusPill";
import { ipc, type ShellStatus } from "./lib/ipc";
import { PAGES, type PageKey } from "./lib/pages";
import { navigate, useNav } from "./lib/nav";
import { BackendsPage } from "./pages/Backends";
import { AgentsPage } from "./pages/Agents";
import { LogsPage } from "./pages/Logs";
import { MonitorPage } from "./pages/Monitor";
import { SettingsPage } from "./pages/Settings";
import { UsagePage } from "./pages/Usage";

import { ConsolePage } from "./pages/Console";

import { SkillsPage } from "./pages/Skills";
import { PlaceholderPage } from "./pages/Placeholder";
import { useZoom } from "./lib/zoom";
import { every, POLL } from "./lib/poll";

// 边条入口收敛（2026-10 UI 重设计决策 A）：models/cache 不再占边条位，
// 页面与路由保留；claude 角色表已删，槽位分配就在代理详情的「模型分配」里做。
// 分组线（UI 审查 P2-7）：8 个纯图标一视同仁识别成本高，按「运行观测 /
// 接入管理 / 配置」三组用发丝线分隔，不改变 Cmd+1..8 的序号语义。
const NAV: { key: PageKey; icon: typeof IconBackend; groupEnd?: boolean }[] = [
  { key: "monitor", icon: IconMonitor },
  { key: "backends", icon: IconBackend },
  { key: "logs", icon: IconLogs },
  { key: "usage", icon: IconUsage, groupEnd: true },
  { key: "console", icon: IconConsole },
  { key: "agents", icon: IconAgents },
  { key: "skills", icon: IconSkills, groupEnd: true },
  { key: "settings", icon: IconSettings },
];

export default function App() {
  // 主窗也支持 Cmd+滚轮 / Cmd+加减号 缩放（与浮窗共用同一个倍率，存 localStorage）。
  useZoom("main");
  // 导航状态（计划 4.2）：页码与跨页参数都在 lib/nav.ts 那份类型化 store 里。
  // 此前这里是三段 window.addEventListener("llmswitch:...") 的字符串总线 ——
  // 事件名拼错或 detail 形状变了都只在运行时静默失效，现在由类型系统兜住。
  const nav = useNav();
  const page = nav.page;
  const setPage = (p: PageKey) => navigate(p);
  // 导航快捷键 Cmd/Ctrl+1..8（决策 D4 的轻量版）：换页不该先把手动回鼠标。
  // 只吃裸数字键，带 alt/shift 的组合一律放行，别抢系统和其它应用的快捷键。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
      const n = Number(e.key);
      if (!Number.isInteger(n) || n < 1 || n > NAV.length) return;
      e.preventDefault();
      navigate(NAV[n - 1].key);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const status = useQuery({
    queryKey: ["status"],
    queryFn: () => ipc<ShellStatus>("get_status"),
    refetchInterval: () => every(POLL.status),
  });

  // 主题（深绿 / 暖白）：初值与持久化都在 lib/theme.ts，这里只订阅渲染。
  const [theme, setTheme] = useState<Theme>(() => getTheme());
  useEffect(() => subscribeTheme(setTheme), []);

  // 多行浮窗开关（顶栏按钮）：可见性 5s 轮询（浮窗自己关掉也能对上状态）。
  // 只管那一个窗口 —— 各代理的速率线是独立窗口，开关在代理页各管各的。
  const qc = useQueryClient();
  const floatVisible = useQuery({
    queryKey: ["float-visible"],
    queryFn: () => ipc<boolean>("float_is_visible"),
    refetchInterval: () => every(POLL.status),
  });
  const toast = useToast();
  // UI-17 语言切换（用户 2026-11-16）：顶栏「中/EN」按钮，偏好落 `ui.lang`，
  // 写入即重建托盘菜单；主窗文案的逐页翻译随后续切片接入，这里先把开关接通。
  const uiLang = useQuery({
    queryKey: ["ui-lang"],
    queryFn: () => ipc<string>("ui_lang_get"),
  });
  const effLang =
    uiLang.data && uiLang.data !== "auto"
      ? uiLang.data
      : (typeof navigator !== "undefined" && navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en");
  const toggleLang = async () => {
    const next = effLang === "zh" ? "en" : "zh";
    try {
      await ipc("ui_lang_set", { lang: next });
      uiLang.refetch().catch(() => {});
    } catch (e) {
      toast.err(`语言切换失败：${e}`);
    }
  };
  // 资源小窗快捷开关（用户 2026-11-16，附截图）：与设置页同一对 IPC、同一份偏好。
  const hasTauri =
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in (window as object);
  const sysFloat = useQuery({
    queryKey: ["sysfloat-visible"],
    queryFn: () => ipc<boolean>("sysfloat_get_visible"),
    enabled: hasTauri,
    refetchInterval: () => every(POLL.status),
  });
  const toggleSysFloat = async () => {
    const prev = sysFloat.data;
    try {
      await ipc("sysfloat_set_visible", { visible: !(prev ?? false) });
      sysFloat.refetch().catch(() => {});
    } catch (e) {
      if (typeof prev === "boolean") qc.setQueryData(["sysfloat-visible"], prev);
      toast.err(`资源小窗开关失败：${e}`);
    }
  };
  const toggleFloat = async () => {
    const prev = floatVisible.data;
    try {
      const v = await ipc<boolean>("float_toggle");
      // 乐观更新：图标立即随状态切换（同主题键的即时反馈），refetch 只做校准。
      if (typeof v === "boolean") qc.setQueryData(["float-visible"], v);
      floatVisible.refetch().catch(() => {});
      return v;
    } catch (e) {
      if (typeof prev === "boolean") qc.setQueryData(["float-visible"], prev);
      toast.err(`浮窗开关失败：${e}`);
      return null;
    }
  };

  return (
    <div className="flex h-full">
      {/* 左侧窄图标 Sidebar */}
      <aside className="flex w-14 flex-col items-center gap-1 border-r border-slate-800 bg-slate-900/50 py-3">
        {NAV.map(({ key, icon: Icon, groupEnd }, i) => (
          <div key={key} className="contents">
            <button
              title={`${PAGES[key].title}（Cmd+${i + 1}）`}
              aria-label={PAGES[key].title}
              aria-current={page === key ? "page" : undefined}
              onClick={() => setPage(key)}
              className={`group/nav relative grid h-10 w-10 place-items-center rounded-lg transition-colors ${
                page === key
                  ? "bg-slate-800 text-cyan-300"
                  : "text-ink-4 hover:bg-slate-800/60 hover:text-ink-2"
              }`}
            >
              {/* UI-10：原生 title 气泡在 WKWebView 里不渲染，悬停提示改用 CSS 飞标：
                  按钮右侧弹出「页面名（快捷键）」小条，pointer-events-none 不挡点击。 */}
              <span
                aria-hidden
                className="pointer-events-none absolute left-full top-1/2 z-50 ml-2 -translate-y-1/2 whitespace-nowrap rounded border border-slate-700 bg-slate-900 px-2 py-1 text-2xs text-ink-2 opacity-0 shadow-lg transition-opacity group-hover/nav:opacity-100"
              >
                {PAGES[key].title}
                <span className="ml-1 text-ink-4">Cmd+{i + 1}</span>
              </span>
              {/* 选中态补一条形状线索（UI-AUDIT-2 B-3）：纯图标边条以前只靠底色变化，
                  浅底上那一档底色差几乎看不出来；贴边短杠与底色两路冗余，不靠颜色说话。 */}
              {page === key && (
                <span
                  aria-hidden
                  className="absolute -left-2 top-1/2 h-5 w-[3px] -translate-y-1/2 rounded-r-full bg-cyan-300"
                />
              )}
              <Icon />
            </button>
            {groupEnd && <div aria-hidden className="my-1.5 h-px w-6 bg-slate-800" />}
          </div>
        ))}
      </aside>

      <main className="flex min-w-0 flex-1 flex-col">
        {/* 顶部标题栏 + 代理状态胶囊 */}
        <header className="flex h-14 items-center gap-3 border-b border-slate-800 px-5">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <h1 className="text-sm font-semibold leading-tight">{PAGES[page].title}</h1>
              {/* 版本号常驻顶栏（UI-AUDIT-2 F-1）：每一页同一个位置，报障与核对装机不用再翻文件。 */}
              <VersionChip />
              <button
                type="button"
                title={effLang === "zh" ? "切换到英文（托盘菜单即时生效）" : "Switch to Chinese"}
                aria-label="切换语言"
                onClick={toggleLang}
                className="shrink-0 rounded border border-slate-700 bg-slate-800/60 px-1.5 py-px font-mono text-2xs leading-snug text-ink-3 transition-colors hover:border-cyan-500/50 hover:text-cyan-300"
              >
                {effLang === "zh" ? "中/EN" : "EN/中"}
              </button>
            </div>
          </div>
          <div className="ml-auto flex items-center gap-2">
            <button
              title={sysFloat.data ? "隐藏资源悬浮小窗" : "显示资源悬浮小窗"}
              aria-label="开关资源悬浮小窗"
              aria-pressed={sysFloat.data ?? false}
              disabled={!hasTauri}
              onClick={toggleSysFloat}
              className={`grid h-8 w-8 place-items-center rounded-lg border border-slate-700 bg-slate-800/60 transition-colors hover:border-cyan-500/50 hover:text-cyan-300 disabled:opacity-40 ${
                sysFloat.data ? "text-cyan-300" : "text-ink-3"
              }`}
            >
              <IconSysFloat width={16} height={16} />
            </button>
            <button
              title={floatVisible.data ? "隐藏多行浮窗" : "显示多行浮窗"}
              aria-label="开关多行浮窗"
              aria-pressed={floatVisible.data ?? false}
              onClick={toggleFloat}
              className={`grid h-8 w-8 place-items-center rounded-lg border border-slate-700 bg-slate-800/60 transition-colors hover:border-cyan-500/50 hover:text-cyan-300 ${
                floatVisible.data ? "text-cyan-300" : "text-ink-3"
              }`}
            >
              {/* 图标随状态切换（同主题键）：可见 → ✕（点击隐藏）；隐藏 → 画中画（点击显示）。 */}
              {floatVisible.data ? (
                <IconFloatHide width={16} height={16} />
              ) : (
                <IconFloat width={16} height={16} />
              )}
            </button>
            <button
              title={theme === "dark" ? "切换到暖白主题" : "切换到深绿主题"}
              aria-label="切换主题"
              onClick={() => toggleTheme()}
              className="grid h-8 w-8 place-items-center rounded-lg border border-slate-700 bg-slate-800/60 text-ink-3 transition-colors hover:border-cyan-500/50 hover:text-cyan-300"
            >
              {theme === "dark" ? <IconSun width={16} height={16} /> : <IconMoon width={16} height={16} />}
            </button>
            <Freshness />
            <TaskTray />
            <StatusPill status={status.data} error={status.isError} />
          </div>
        </header>

        <section className="min-h-0 flex-1 overflow-auto p-5">
          {/* key={page}：切页自动复位，一页炸了不拖死整窗 */}
          <ErrorBoundary key={page} zone={PAGES[page].title}>
          {page === "backends" ? (
            <BackendsPage paramsBackend={nav.params.paramsBackend} />
          ) : page === "logs" ? (
            <LogsPage />
          ) : page === "monitor" ? (
            <MonitorPage />
          ) : page === "settings" ? (
            <SettingsPage />
          ) : page === "usage" ? (
            <UsagePage />
          ) : page === "console" ? (
            <ConsolePage pendingBackend={nav.params.backend} />
          ) : page === "agents" ? (
            <AgentsPage openMatrixAt={nav.params.matrix} />
          ) : page === "skills" ? (
            <SkillsPage />
          ) : (
            <PlaceholderPage pageKey={page} />
          )}
          </ErrorBoundary>
        </section>
      </main>
      <CommandPalette />
      <GuidedTour />
    </div>
  );
}

