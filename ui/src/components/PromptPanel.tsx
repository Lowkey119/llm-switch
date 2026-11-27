// 快速面板 v2（quickpanel 原型回灌：docs/ui_03/quickpanel.html）。
//
// tab 轨完全由 ~/.llm-switch/quickpanel.json 驱动（quickpanel_config_get，每次获焦重读 = 热加载）；
// 前端不写死任何分类，内置默认只存在于 Rust 侧兜底。tab 的 type 决定 OK 语义：
//   prompts=装入待注入 / chars=追加待注入 / actions=装入待执行 / skills=只读文档 / config=配置预览。
// 两段式执行（原型定稿）：中栏 OK 只把内容**装入**右栏（可再编辑），右栏 OK（或 ⌥Enter 发送键）
// 才真执行；执行完自动收窗，再呼出停在原状态（浮窗常驻隐藏不销毁，React state 天然还在）。
// 八键口径：Tab=中栏↔右栏换焦点；Esc 逐级后退 退编辑→清待执行→收窗；←→ 换 tab；↑↓ 区内移动；
// OK 短按装入/执行、长按编辑/待删。confirm 动作两步 OK 确认，全部执行落 ~/.llm-switch/actions.log。
// 远距大字（一律取 styles.css 字号阶梯，禁 text-[Npx]）：tab 轨 168px/text-xl、列表 text-xl、字符 text-2xl、正文最小 text-lg。
//
// 注入目标在面板弹出**之前**就拍成快照（谁拿着输入焦点就注给谁）；窗口生死/
// 位置/尺寸/贴边圆钮全在 Rust 侧（gui::promptfloat），这里只画内容。
// 缩放倍率 useZoom 已按窗持久化、窗口位置 Rust 侧已落库——缩放/位置零新机制。
import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { listen } from "@tauri-apps/api/event";
import { useQuery } from "@tanstack/react-query";
import remoteImg from "../assets/remote-hi.webp";
import { copyText } from "../lib/clipboard";
import { ipc, type ClientCol } from "../lib/ipc";
import { setLang, t, useI18n } from "../lib/i18n";
import { useZoom, zoomFactor } from "../lib/zoom";
import { usePromptSpecials, useRecent, useVoiceEdit } from "./promptSpecials";
import { useAppendToTab } from "./promptAppend";
import { useTimedNote } from "../lib/timedNote";
import { usePromptKeys } from "./promptKeys";
import PromptCenter from "./PromptCenter";
import { type Stage } from "./PromptRightPane";
import PromptTabRail from "./PromptTabRail";
import PromptVoiceEditor from "./PromptVoiceEditor";
import PromptFooter from "./PromptFooter";
import PromptTitleBar from "./PromptTitleBar";
import { buildRows, QPConfig, TAB_LABEL_KEYS, type Row, type SkillRow } from "./promptRows";
import type { QpTab as QPTab } from "../pages/remote/quickpanel";
import { MACRO_QUERY } from "../pages/remote/macros";

export default function PromptPanel() {
  useI18n();
  // 缩放手感与浮窗 / 速率线 / 资源小窗完全一致：Cmd+滚轮 / Cmd+±/ 复位 Cmd+0。
  const zoomRef = useRef<number | null>(null);
  useZoom("prompt", (z) => {
    const prev = zoomRef.current;
    zoomRef.current = z;
    if (prev === null || Math.abs(z - prev) < 0.001) return;
    void (async () => {
      try {
        const win = getCurrentWindow();
        const size = await win.innerSize();
        const scale = await win.scaleFactor();
        const ratio = z / prev;
        await win.setSize(
          new LogicalSize(
            Math.max(320, Math.round((size.width / scale) * ratio)),
            Math.max(200, Math.round((size.height / scale) * ratio)),
          ),
        );
      } catch {
        /* 拿不到尺寸就算了，字号照样跟着倍率走 */
      }
    })();
  });

  // 浮窗入口不走主窗 App，语言得自己取一次（缺省中文，与主窗同源偏好）。
  useEffect(() => {
    void ipc<string>("ui_lang_get")
      .then((l) => setLang(l === "en" ? "en" : "zh"))
      .catch(() => {});
  }, []);

  // 形态：面板 or 贴边圆钮（Rust 裁决，前端只跟画）。
  const [orb, setOrb] = useState(false);
  useEffect(() => {
    void ipc<{ orb: boolean }>("prompt_shape")
      .then((s) => setOrb(Boolean(s.orb)))
      .catch(() => {});
    const un = listen<{ orb: boolean }>("prompt://shape", (e) => setOrb(Boolean(e.payload.orb)));
    return () => void un.then((f) => f());
  }, []);

  const clients = useQuery({ queryKey: ["client-list"], queryFn: () => ipc<ClientCol[]>("client_list") });
  // tab 配置：文件不存在给 Rust 侧内置默认；解析失败回默认并带 error（右栏配置态展示）。
  const configQ = useQuery({
    queryKey: ["quickpanel-config"],
    queryFn: () => ipc<QPConfig>("quickpanel_config_get"),
  });

  // 注入目标与辅助功能授权都在 Rust 侧实时问，前端不猜。
  const targetQ = useQuery({
    queryKey: ["prompt-target"],
    queryFn: () =>
      ipc<{ name: string; agent: string | null; usable: boolean } | null>("prompt_target"),
  });
  const a11yQ = useQuery({
    queryKey: ["prompt-a11y"],
    queryFn: () => ipc<boolean>("prompt_a11y_status"),
  });
  const target = targetQ.data ?? null;
  const a11yOk = a11yQ.data !== false;
  useEffect(() => {
    const un = listen("prompt://target", () => void targetQ.refetch());
    return () => void un.then((f) => f());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 面板态特殊宏（音量 ± 缩放档 / Home 循环切目标）——状态在面板这边，见 promptSpecials。
  usePromptSpecials({ targetName: target?.name, refetchTarget: () => void targetQ.refetch() });

  // 语音编辑态（计划 §语音编辑态）：状态与事件都在 promptSpecials 那份 hook 里。
  const voice = useVoiceEdit();

  // 面板被唤起（获得焦点）就刷一次：条目、目标、授权，以及 quickpanel.json（热加载口径）。
  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused) return;
        void targetQ.refetch();
        void a11yQ.refetch();
        void configQ.refetch();
      })
      .then((f) => {
        if (cancelled) f();
        else un = f;
      });
    return () => {
      cancelled = true;
      un?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const [tab, setTab] = useState("");
  const [hi, setHi] = useState(0);
  const [stage, setStage] = useState<Stage | null>(null);
  const [armAct, setArmAct] = useState<string | null>(null);
  // 左轨宽度（用户拖中轨之间的分隔条调过的值）。localStorage 与 ConfigPanel 同款口径：
  // 中栏是 flex-1，轨宽定了两边宽度就都定了，不必再存一份会随窗口大小失效的中栏宽。
  const RAIL_W_KEY = "prompt.tab-rail-w";
  const [railW, setRailW] = useState(() => {
    const raw = Number(localStorage.getItem(RAIL_W_KEY));
    return raw >= 96 && raw <= 360 ? raw : 132;
  });
  // 无注入目标按 OK 的闪烁提醒：2s 自清。
  const [flash, setFlash] = useState(false);
  // 底栏提示：文案与自清定时器走 useTimedNote（与顶栏胶囊同一份实现，不再各写一份 setTimeout）。
  const { note, show: flashNote } = useTimedNote(2000);

  // JSON 驱动的 tab 轨：dynamic:"agent" 的 tab 换成当前注入目标的代理，没目标就整档隐藏。
  const cfgTabs = useMemo<QPTab[]>(() => configQ.data?.tabs ?? [], [configQ.data]);
  // 「最近发送」专用 tab（计划 §三.7）：不新增 UI，合成一个 tab 挂到轨尾，条目从历史读。
  const recent = useRecent();
  const RECENT_KEY = "__recent";
  const cfgTabsAll = useMemo<QPTab[]>(
    () => [...cfgTabs, { key: RECENT_KEY, label: t("prompt.recentTab"), type: "recent", sep: true }],
    [cfgTabs],
  );
  const agentTab = (clients.data ?? []).find((c) => c.app === target?.agent);
  const tabs = useMemo<QPTab[]>(
    () =>
      cfgTabsAll.flatMap((tb) =>
        tb.dynamic === "agent" ? (agentTab ? [{ ...tb, key: agentTab.app, label: agentTab.label }] : []) : [tb],
      ),
    [cfgTabsAll, agentTab],
  );
  const labelOf = (tb: QPTab) => {
    const k = TAB_LABEL_KEYS[tb.key];
    return k ? t(k as never) : tb.label;
  };
  // tab 失效（改了 JSON / 代理切了）回落第一个 prompts tab。
  useEffect(() => {
    if (!tabs.length) return;
    if (!tabs.some((tb) => tb.key === tab)) {
      setTab(tabs.find((tb) => tb.type === "prompts")?.key ?? tabs[0].key);
    }
  }, [tabs, tab]);
  const td = tabs.find((tb) => tb.key === tab);

  // 按当前 tab 的类型摆行（搜索态已随搜索框一起去掉）。
  const skillsQ = useQuery({
    queryKey: ["prompt-skills"],
    queryFn: () => ipc<SkillRow[]>("get_skills"),
    enabled: td?.type === "skills",
  });
  // 宏引用条目要显示动作名（不是 id）：宏库按当前档案读，与遥控器页同一份。
  // 无条件查（不挂 enabled 条件）：控制字符 tab 的条目是 {macro:id}，若首帧 configQ 还没
  // 回来，条件算 false 查询就不发，macroName 查不到→退回 id，浮窗显示一长串 hash
  //（用户 2026-11-26 实报）。常驻缓存，与编辑界面 useMacros 同口径。
  const macrosQ = useQuery({
    queryKey: MACRO_QUERY,
    queryFn: () => ipc<{ id: string; name: string }[]>("remote_macros_get"),
    staleTime: Infinity,
  });
  const macroName = (id: string) => (macrosQ.data ?? []).find((m) => m.id === id)?.name ?? id;

  const rows = useMemo<Row[]>(
    () =>
      buildRows({
        td,
        cfgTabs,
        skills: skillsQ.data ?? [],
        labelOf,
        macroName,
        recent: recent.items,
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [skillsQ.data, td, cfgTabs, macrosQ.data, recent.items],
  );
  useEffect(() => setHi(0), [tab]);
  useEffect(() => {
    if (hi > rows.length - 1) setHi(Math.max(0, rows.length - 1));
  }, [rows.length, hi]);
  const cur = rows[hi];
  // 选中行跟着键盘跑出视野就像「按了没反应」：选中即滚进视口。
  useEffect(() => {
    if (!cur) return;
    document.querySelector(`[data-row="${CSS.escape(String(cur.id))}"]`)?.scrollIntoView({ block: "nearest" });
  }, [cur]);

  // ---- 浮窗只读（用户 2026-11-25 拍板）----
  // 右栏那块多行文本只是注入前的临时编辑，不落库；要存成条目走「追加到 tab」，
  // 逻辑（含同 tab 去重与写盘）在 promptAppend 那份 hook 里。
  const append = useAppendToTab({
    cfgTabs,
    curTabKey: td?.key ?? "",
    stagedText: stage?.kind === "inject" ? stage.text : null,
    note: flashNote,
    onAppended: () => setStage(null),
  });

  // ---- 执行：OK / ⌥Enter 都走这里；右栏已删，stage 只用于语音输入等临时落点 ----
  const execStage = (override?: Stage) => {
    const st = override ?? stage;
    if (!st) return;
    // 无注入目标：OK 发送一律拦截 + 组合框红闪（用户 2026-11-26 门控口径）；
    // ←→↑↓ 浏览不受影响。目标确定后才允许发送。
    if (!target) {
      setFlash(true);
      window.setTimeout(() => setFlash(false), 2000);
      return;
    }
    // 目标不收注入（探测无输入区）：宏/动作/命令禁止；文本放行走下面的「只写剪贴板」降级。
    const usable = target.usable;
    // 「最近发送」历史（计划 §三.7）：文本/命令记正文、宏与单发动作记显示名（重发时同一口径）。
    // 放在门控之后：被拦下来的不算发送，不往历史里灌垃圾。
    const history = st.kind === "run" ? st.label : st.kind === "copied" ? "" : st.text;
    if (history.trim()) recent.add(history);
    if (!usable && st.kind !== "inject") {
      setFlash(true);
      window.setTimeout(() => setFlash(false), 2000);
      flashNote(t("prompt.targetNoInput"), 3000);
      return;
    }
    if (st.kind === "run") {
      // 引用动作 / 单发动作：收窗与焦点归还由命令侧统一办（`dismiss_to_target`），
      // 这里只管投递与把结果说给人看；命令报错（如引用已删）面板不收，用户看得见。
      // 统一口径（计划 §三.4）：同时写一份剪贴板备份（宏发的是按键，剪贴板存显示文本）。
      void copyText(st.label).catch(() => {});
      setStage(null);
      const call =
        st.what === "macro"
          ? ipc("remote_macro_run", { id: st.id })
          : ipc("remote_action_run", { action: st.action });
      void call
        .then(() => flashNote(`${t("prompt.actionDone")} ${st.label}`, 3000))
        .catch((e) => flashNote(String(e), 4000));
      return;
    }
    if (st.kind === "action") {
      // confirm 动作两步确认：第一下只武装，2.5s 不回手撤销。
      if (st.confirm && armAct !== st.id) {
        setArmAct(st.id);
        window.setTimeout(() => setArmAct((v) => (v === st.id ? null : v)), 2500);
        return;
      }
      setArmAct(null);
      setStage(null);
      void ipc<{ exit: number; tail: string }>("quickpanel_run_action", { label: st.label, cmd: st.text })
        .then((r) => flashNote(`${t("prompt.actionDone")} exit=${r.exit}`, 4000))
        .catch((e) => flashNote(String(e), 4000))
        .finally(() => void ipc("prompt_set_visible", { visible: false }).catch(() => {}));
      return;
    }
    // 统一发送口径（用户 2026-11-26「两者统一」）：写剪贴板 + 直接注入目标。
    // 目标不收注入时降级：只写剪贴板 + 提示手动粘贴，1.5s 后收窗（计划 §三.3/三.4）。
    void copyText(st.text)
      .then((ok) => {
        if (!ok) {
          flashNote(t("prompt.copyFail"));
          return;
        }
        setStage({ kind: "copied", text: st.text });
        if (!usable) {
          flashNote(t("prompt.copiedHint"), 3000);
          window.setTimeout(() => void ipc("prompt_set_visible", { visible: false }).catch(() => {}), 1500);
          return;
        }
        // 统一口径：写剪贴板 + 直接注入目标（门控已保证目标可用）。
        flashNote(t("prompt.copiedAndInjected"), 3000);
        void ipc("prompt_inject").catch(() => void ipc("prompt_set_visible", { visible: false }));
      });
  };

  // 右栏已删：没有「装入预览」这一步，OK 直接执行当前行（confirm 动作仍靠 armAct 连按两次）。
  const execRow = (r: Row) => {
    if (r.kind === "char") execStage({ kind: "inject", text: r.content });
    else if (r.kind === "action")
      execStage({ kind: "action", text: r.cmd, label: r.content, confirm: r.confirm, id: r.id });
    else if (r.kind === "macro" || r.kind === "step")
      execStage(
        r.kind === "macro"
          ? { kind: "run", what: "macro", label: r.content, id: r.macro }
          : { kind: "run", what: "action", label: r.content, action: r.action },
      );
  };

  // 语音编辑态的 OK（计划 §语音编辑态）：写剪贴板 + 直接注入目标——与「选条目文本只写剪贴板」
  // 是有意区分的两种语义（语音已经整段说完，该一次落定）。发完退编辑态并清空：
  // 内容已经送到目标那边，留着下次会说出一段重复话。Esc 才「保留内容下次追加」。
  const sendVoice = () => {
    const text = voice.text.trim();
    if (!text) return;
    execStage({ kind: "inject", text });
    voice.consume();
  };



  // 注入失败兜底：目标没拿到焦点或被系统挡住时，重开面板并提示手动粘贴。
  useEffect(() => {
    const un = listen<{ reason?: string; target?: string | null }>("prompt://inject-failed", (e) => {
      const r = e.payload?.reason;
      const who = e.payload?.target;
      const msg =
        r === "no-permission"
          ? t("prompt.injectFailA11y")
          : r === "no-target"
            ? t("prompt.injectFailNoTarget")
            : r === "focus-lost" && who
              ? `${t("prompt.injectFailFocusTo")} ${who}`
              : t("prompt.injectFail");
      flashNote(msg, 6000);
      void ipc("prompt_set_visible", { visible: true }).catch(() => {});
      void a11yQ.refetch();
    });
    return () => void un.then((f) => f());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 未授权时面板上直接给入口：点一下才让系统弹授权框（注入路径上绝不弹）。
  const askA11y = () => {
    void ipc<boolean>("prompt_a11y_ask")
      .then((ok) => flashNote(ok ? t("prompt.a11yGranted") : t("prompt.a11yAskFail"), 5000))
      .catch(() => {})
      .finally(() => void a11yQ.refetch());
  };

  usePromptKeys({
    rowCount: rows.length,
    hi,
    tab,
    tabs,
    editorOpen: false,
    stageOpen: !!stage,
    voiceEdit: voice.editing,
    // OK 短按：右栏已删，没有装入预览这一步——直接执行当前行。
    okShort: () => {
      if (stage) {
        execStage();
        return;
      }
      const r = rows[hi];
      if (r) execRow(r);
    },
    // OK 长按（右栏 + 待注入正文）= 一键把这段临时正文追加进选定的 tab。
    // 浮窗没有「编辑条目」了（内容全在遥控器页管），这里只留这一个写回的入口。
    okLong: () => {
      if (stage?.kind === "inject") append.run();
    },
    flush: () => {},
    clearStage: () => {
      setStage(null);
      setArmAct(null);
    },
    // ⌥Enter 发送键（遥控器专用键的落点）：无视焦点区直接执行当前行。
    send: () => {
      const r = rows[hi];
      if (r) execRow(r);
    },
    setHi,
    setTab,
  });

  // 整窗拖拽：起手 >4px 才交给原生拖（SysFloat 同款约束）。
  const armed = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  const press = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    armed.current = { x: e.clientX, y: e.clientY, moved: false };
  };
  const drift = (e: React.PointerEvent) => {
    const a = armed.current;
    if (!a || a.moved) return;
    if (Math.hypot(e.clientX - a.x, a.y - e.clientY) < 4) return;
    a.moved = true;
    void getCurrentWindow()
      .startDragging()
      .catch(() => {});
  };
  const release = () => {
    armed.current = null;
  };
  // 轨宽分隔条：拖动只走 window 级监听（ConfigPanel 同款），别让整窗拖拽抢走事件。
  const startRailDrag = (e: React.PointerEvent) => {
    e.stopPropagation();
    e.preventDefault();
    let latest = railW;
    const move = (ev: PointerEvent) => {
      // clientX/innerWidth 是视口坐标，轨宽是 zoom 后布局坐标（面板默认 2 倍）：
      // 不同除倍率，分隔条会按倍率偏到光标右侧（用户 2026-11-27 实测）。
      const z = zoomFactor() || 1;
      latest = Math.max(96, Math.min(360, window.innerWidth / z - 320, ev.clientX / z));
      setRailW(latest);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      localStorage.setItem(RAIL_W_KEY, String(latest));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  // 圆钮态：54px 圆钮（.float-orb 与浮窗/速率线/资源小窗同款）。没拖动就松手 = 点击展开。
  if (orb) {
    return (
      <div
        className="float-orb select-none"
        role="button"
        aria-label={t("prompt.orbTip")}
        title={t("prompt.orbTip")}
        onPointerDown={press}
        onPointerMove={drift}
        onPointerUp={() => {
          const a = armed.current;
          armed.current = null;
          if (a && !a.moved) void ipc("prompt_expand").catch(() => {});
        }}
        onPointerCancel={release}
      >
        <img src={remoteImg} alt="" aria-hidden="true" draggable={false} className="pointer-events-none h-10 w-auto" />
      </div>
    );
  }

  return (
    <div
      className="prompt-card flex h-full w-full flex-col select-none"
      onPointerDown={press}
      onPointerMove={drift}
      onPointerUp={release}
      onPointerCancel={release}
    >
      {/* 标题行：遥控器状态 + 电量 + 语音波形 + 注入目标（复用遥控器页共享件）。 */}
      <PromptTitleBar target={target} flash={flash} />

      {/* 语音编辑态回显框（计划 §语音编辑态）：只在录入时挂，不编辑就不占位。 */}
      {voice.editing && (
        <PromptVoiceEditor text={voice.text} onChange={voice.setText} onSend={sendVoice} onExit={voice.exit} />
      )}

      <div className="flex min-h-0 flex-1">
        <PromptTabRail
          tabs={tabs.map((tb) => ({ key: tb.key, label: labelOf(tb), sep: tb.sep }))}
          tab={tab}
          onPick={(k) => setTab(k)}
          width={railW}
          menuFor={(k) =>
            k === RECENT_KEY
              ? [
                  {
                    label: t("prompt.recentClear"),
                    danger: true,
                    onClick: () => recent.clear(),
                  },
                ]
              : []
          }
        />
        {/* 轨宽分隔条：中央竖向、可拖；中栏 flex-1 吃掉剩余宽度，只持久化轨宽。 */}
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={t("prompt.railResize")}
          title={t("prompt.railResize")}
          className="w-1.5 shrink-0 cursor-col-resize bg-slate-800/60 transition-colors hover:bg-[var(--sel-ring)]"
          onPointerDown={startRailDrag}
        />
        <PromptCenter
          rows={rows}
          hi={hi}
          onPick={(i) => setHi(i)}
          error={configQ.isError}
          loading={configQ.isLoading}
          emptyNote={t("prompt.empty")}
        />
      </div>

      <PromptFooter
        note={note}
        a11yOk={a11yOk}
        onAskA11y={askA11y}
        target={target}
        flash={flash}
        onTargetPicked={() => void targetQ.refetch()}
      />

      {/* 右下角缩放手把：原生 resize 拖（Rust 侧静默落库尺寸）。 */}
      <div
        className="absolute right-0 bottom-0 h-4 w-4 cursor-nwse-resize"
        title={t("prompt.resizeTip")}
        onPointerDown={(e) => {
          e.stopPropagation();
          void getCurrentWindow()
            .startResizeDragging("SouthEast")
            .catch(() => {});
        }}
      >
        <svg viewBox="0 0 16 16" className="h-full w-full text-ink-4" aria-hidden="true">
          <path d="M14 6 6 14M14 10l-4 4" stroke="currentColor" strokeWidth="1.2" fill="none" />
        </svg>
      </div>
    </div>
  );
}
