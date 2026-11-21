import { useEffect, useState, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  IconMoon,
  IconSun,
  IconThemeAuto,
  IconFlowAuto,
  IconPromptPanel,
  IconFloat,
  IconFloatHide,
  IconSysFloat,
} from "./components/icons";
import {
  cyclePref,
  getPref,
  getTheme,
  subscribeTheme,
  type Theme,
  type ThemePref,
} from "./lib/theme";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { useToast } from "./components/Toast";
import { TaskTray } from "./components/TaskTray";
import { GuidedTour } from "./components/GuidedTour";
import { CommandPalette } from "./components/CommandPalette";
import { BrandMark, Rail, NAV_ORDER } from "./components/Rail";
import { TopVersionChip } from "./components/VersionBadge";
import { StatusPill } from "./components/StatusPill";
import { RemoteChip } from "./components/RemoteChip";
import { ipc, type ShellStatus } from "./lib/ipc";
import { PAGES, type PageKey } from "./lib/pages";
import { setLang, t, ts, useI18n } from "./lib/i18n";
import { navigate, useNav } from "./lib/nav";
import { BackendsPage } from "./pages/Backends";
import { AgentsPage } from "./pages/Agents";
import { ObservePage } from "./pages/Observe";
import { SettingsPage } from "./pages/Settings";
import { RemotePage } from "./pages/Remote";
import { useZoom } from "./lib/zoom";
import { every, POLL } from "./lib/poll";

// 边条入口收敛（2026-10 UI 重设计决策 A）：models/cache 不再占边条位，
// 页面与路由保留；claude 角色表已删，槽位分配就在代理详情的「模型分配」里做。
// ui_01 回灌 U2：入口清单与分组（运行观测 / 接入管理 / 配置）连同折叠态一起
// 搬到 components/Rail.tsx，这里只留 NAV_ORDER 给 ⌘1–9 当序号依据。

/** 顶栏开关按钮：开启时右上角挂绿点徽标（与遥控器 RemoteChip 同口径，用户 2026-11-21）；关闭无点。 */
function TopToggle({
  on,
  tip,
  disabled,
  onClick,
  children,
}: {
  on: boolean;
  tip: string;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className={`tip tip-edge grid h-8 w-8 place-items-center rounded-lg border border-slate-700 bg-slate-800/60 transition-colors hover:border-cyan-500/50 hover:text-cyan-300 disabled:cursor-not-allowed disabled:text-ink-4 ${
        on ? "text-cyan-300" : "text-ink-3"
      }`}
      data-tip={tip}
      aria-label={tip}
      aria-pressed={on}
      disabled={disabled}
      onClick={onClick}
    >
      <span className="relative">
        {children}
        {on && (
          <span aria-hidden className="absolute -right-1.5 -top-1.5 h-2 w-2 rounded-full bg-[var(--sem-ok)]" />
        )}
      </span>
    </button>
  );
}

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
      if (!Number.isInteger(n) || n < 1 || n > NAV_ORDER.length) return;
      e.preventDefault();
      navigate(NAV_ORDER[n - 1]);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const status = useQuery({
    queryKey: ["status"],
    queryFn: () => ipc<ShellStatus>("get_status"),
    refetchInterval: () => every(POLL.status),
  });

  // 主题三态（深绿 / 暖白 / 跟随系统）：初值与持久化都在 lib/theme.ts，这里只订阅渲染。
  // 按钮图标看**档位**（auto 在深色系统上解析成 dark，图标仍要画 auto），
  // 所以档位与解析结果两个都得留着。
  const [theme, setTheme] = useState<Theme>(() => getTheme());
  const [themePref, setThemePref] = useState<ThemePref>(() => getPref());
  useEffect(() => subscribeTheme((t, p) => {
    setTheme(t);
    setThemePref(p);
  }), []);

  // 多行浮窗开关（顶栏按钮）：可见性 5s 轮询（浮窗自己关掉也能对上状态）。
  // 只管那一个窗口 —— 各代理的速率线是独立窗口，开关在代理页各管各的。
  const qc = useQueryClient();
  const floatVisible = useQuery({
    queryKey: ["float-visible"],
    queryFn: () => ipc<boolean>("float_is_visible"),
    refetchInterval: () => every(POLL.status),
  });
  const toast = useToast();
  // 提示词速取面板快捷开关（顶栏最右，用户 2026-11-18）：可见态跟着轮询走——
  // 用户用快捷键唤起、用 Esc / 圆钮收起，按钮图标也得跟着变，不能只认自己点的那次。
  const promptVisible = useQuery({
    queryKey: ["prompt-visible"],
    queryFn: async () => (await ipc<{ visible?: boolean }>("prompt_shape")).visible ?? false,
    refetchInterval: () => every(POLL.status),
  });
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
  // 词典是真文案的唯一入路：偏好拉到（或切过）就写进 i18n store，
  // useI18n 订阅变化让外壳即时重渲染。以前 setLang 没人调，词典形同虚设，
  // 「中/EN」只翻了按钮自己。
  const { lang } = useI18n();
  useEffect(() => {
    setLang(effLang === "en" ? "en" : "zh");
  }, [effLang]);
  void lang; // 订阅即重渲染，文案都走 t()
  const pageTitle = (k: PageKey) => t(`page.${k}`);
  const toggleLang = async () => {
    const next = effLang === "zh" ? "en" : "zh";
    try {
      await ipc("ui_lang_set", { lang: next });
      uiLang.refetch().catch(() => {});
    } catch (e) {
      toast.err(ts("app.langFail", { e: String(e) }));
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
      toast.err(ts("app.sysFloatFail", { e: String(e) }));
    }
  };

  // 速率线「有流量自动弹出」顶栏快捷开关（用户 2026-11-17，附截图指的那一格）：
  // 后端与设置页早就有（rateline.auto，#46），这里只是把入口提到顶栏，
  // 同一个 queryKey —— 设置页或托盘改了一边，这边轮询就对上。
  const rlAuto = useQuery({
    queryKey: ["rateline-auto"],
    queryFn: () => ipc<boolean>("rateline_auto_get"),
    enabled: hasTauri,
    refetchInterval: () => every(POLL.status),
  });
  const toggleRlAuto = async () => {
    const prev = rlAuto.data;
    const next = !(prev ?? false);
    // 乐观更新（同资源小窗/浮窗口径）：失败再还原，别让用户等 IPC。
    qc.setQueryData(["rateline-auto"], next);
    try {
      await ipc("rateline_auto_set", { on: next });
      rlAuto.refetch().catch(() => {});
    } catch (e) {
      if (typeof prev === "boolean") qc.setQueryData(["rateline-auto"], prev);
      toast.err(ts("app.rlAutoFail", { e: String(e) }));
    }
  };
  const togglePrompt = async () => {
    const next = !(promptVisible.data ?? false);
    try {
      await ipc("prompt_set_visible", { visible: next });
      // 乐观更新 + refetch 校准（面板可能被圆钮态、快捷键等旁路改掉）。
      qc.setQueryData(["prompt-visible"], next);
      promptVisible.refetch().catch(() => {});
    } catch (e) {
      toast.err(ts("app.promptFail", { e: String(e) }));
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
      toast.err(ts("app.floatFail", { e: String(e) }));
      return null;
    }
  };

  return (
    <div className="flex h-full">
      {/* 边条（ui_01 回灌 U2）：图标 + 名称的固定宽轨道（折叠已按用户 2026-11-20 拍板取消）。 */}
      <Rail page={page} onNavigate={setPage} lang={effLang} onToggleLang={() => void toggleLang()} />

      <main className="flex min-w-0 flex-1 flex-col">
        {/* 顶部标题栏 + 代理状态胶囊。
            布局（用户 2026-11-20 截图拍板）：版本号回到顶栏页名右侧（TopVersionChip，
            有更新时与「有更新」呼吸交替），中英文切换留在边条底部；
            状态胶囊改为 flex 居中占一栏（此前是绝对定位叠一层，中等宽度会与
            右侧按钮组叠字——用户截图指出）：空间不够时它自己缩，不吃掉按钮。 */}
        <header className="flex h-14 items-center gap-3 border-b border-slate-800 px-5">
          {/* 动态品牌标（用户 2026-11-20 拍板从边条挪来顶栏）：边条 56px 轨道里 28px
              太小看不清「llm」，顶栏给到 32px 并加粗。 */}
          <BrandMark />
          <h1 className="shrink-0 text-base font-bold leading-tight">{pageTitle(page)}</h1>
          <TopVersionChip />
          {/* 状态胶囊：flex 居中（md 以下不显示，窄窗宁可让位给按钮）。 */}
          <div className="hidden min-w-0 flex-1 items-center justify-center overflow-hidden md:flex">
            <StatusPill status={status.data} error={status.isError} />
          </div>
          <div className="ml-auto flex shrink-0 items-center gap-2">
            {/* 遥控器快捷入口（用户 2026-11-19 截图拍板）：双态图标（连接/未连接），
                点击直达遥控器页；已绑定才出现，各页常驻。 */}
            <RemoteChip onGo={() => setPage("remote")} />
            <TopToggle
              on={!!rlAuto.data}
              tip={t(rlAuto.data ? "top.rateAutoOn" : "top.rateAutoOff")}
              disabled={!hasTauri}
              onClick={toggleRlAuto}
            >
              <IconFlowAuto width={16} height={16} />
            </TopToggle>
            <TopToggle
              on={!!sysFloat.data}
              tip={t(sysFloat.data ? "top.sysFloatOn" : "top.sysFloatOff")}
              disabled={!hasTauri}
              onClick={toggleSysFloat}
            >
              <IconSysFloat width={16} height={16} />
            </TopToggle>
            <TopToggle
              on={!!floatVisible.data}
              tip={t(floatVisible.data ? "top.floatOn" : "top.floatOff")}
              disabled={!hasTauri}
              onClick={toggleFloat}
            >
              {/* 图标随状态切换（同主题键）：可见 → ✕（点击隐藏）；隐藏 → 画中画（点击显示）。 */}
              {floatVisible.data ? (
                <IconFloatHide width={16} height={16} />
              ) : (
                <IconFloat width={16} height={16} />
              )}
            </TopToggle>
            {/* 接单开关与后台任务并排贴在状态胶囊左边（用户 2026-11-17：
                「这个接单开关放到顶栏，随时能点的位置」）——接单状态本来就写在这颗胶囊上，
                开关离它最近才看得懂按下去会变什么。 */}
            {/* 提示词速取快捷开关：用户 2026-11-18 定在顶栏最右（状态胶囊原来的位置）。
                之前只有「显示」，收起只能靠 Esc / 快捷键——这里做成真开关。 */}
            <TopToggle
              on={!!promptVisible.data}
              tip={t(promptVisible.data ? "top.promptHide" : "top.promptShow")}
              disabled={!hasTauri}
              onClick={() => void togglePrompt()}
            >
              {/* 图标随状态切换（与浮窗开关同一口径）：可见 → ✕（点了收起）。 */}
              {promptVisible.data ? (
                <IconFloatHide width={16} height={16} />
              ) : (
                <IconPromptPanel width={16} height={16} />
              )}
            </TopToggle>
            <TaskTray />
            <button
              className={`tip tip-edge grid h-8 w-8 place-items-center rounded-lg border border-slate-700 bg-slate-800/60 transition-colors hover:border-cyan-500/50 hover:text-cyan-300 ${
                themePref === "auto" ? "text-cyan-300" : "text-ink-3"
              }`}
              data-tip={ts(themePref === "auto" ? "top.themeAutoNow" : "top.themeNow", {
                now: t(
                  themePref === "auto"
                    ? theme === "dark"
                      ? "theme.dark"
                      : "theme.light"
                    : themePref === "dark"
                      ? "theme.dark"
                      : "theme.light",
                ),
                next: t(
                  themePref === "auto"
                    ? "theme.dark"
                    : themePref === "dark"
                      ? "theme.light"
                      : "theme.auto",
                ),
              })}
              aria-label={t("palette.theme")}
              onClick={() => cyclePref()}
            >
              {/* 三态各一枚图标（用户 2026-11-17）：深绿=月、暖白=日、跟随系统=半明半暗圆。 */}
              {themePref === "dark" ? (
                <IconMoon width={16} height={16} />
              ) : themePref === "light" ? (
                <IconSun width={16} height={16} />
              ) : (
                <IconThemeAuto width={16} height={16} />
              )}
            </button>
          </div>
        </header>

        {/* 顶部留白挪进各页根节点：sticky 吸附在滚动容器**内容盒**边缘，
            容器自带 padding-top 会留一条缝，内容从固定条上方穿过（日志表头/遥控器卡头，
            用户 2026-11-20 截图）。左右与底部 padding 不影响 sticky，保留。 */}
        <section className="min-h-0 flex-1 overflow-auto px-[var(--content-pad)] pb-[var(--content-pad)]">
          {/* key={page}：切页自动复位，一页炸了不拖死整窗 */}
          <ErrorBoundary key={page} zone={t(PAGES[page].titleKey)}>
          {page === "backends" ? (
            <BackendsPage
              paramsBackend={nav.params.paramsBackend}
              consoleBackend={nav.params.consoleBackend}
              consolePulse={nav.params.consolePulse}
            />
          ) : page === "observe" ? (
            <ObservePage />
          ) : page === "remote" ? (
            <RemotePage />
          ) : page === "agents" ? (
            <AgentsPage openMatrixAt={nav.params.matrix} />
          ) : (
            <SettingsPage />
          )}
          </ErrorBoundary>
        </section>
      </main>
      <CommandPalette />
      <GuidedTour />
    </div>
  );
}

