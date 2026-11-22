// 按键映射区（docs/ui_02/mapping.html 原型回灌；范围 = 真实数据骨架，用户 2026-11-19 拍板）。
//
// 接后端真实能力：三槽快捷键绑定（S-D）走 remote_buttons_get / remote_buttons_set
// （整体替换，落 settings 即时生效）。键卡与槽位交互在 KeyCard.tsx，键码表在 shortcut.ts。
// 未接入的（要扩 Rust IPC 契约，一律不画）：方案条、语音触发——原型里那些是演示态，
// 照搬就是造假状态。
// 实物图热点坐标按 mi-remote.png 实拍标定（统一直径 HOT_D），见 keys.ts。
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Card, CardHead, SkeletonRows } from "../../components/ui";
import { QueryError, errText } from "../../components/QueryError";
import { useToast } from "../../components/Toast";
import { IconKeyboard, IconPencil, IconRemote } from "../../components/icons";
import { t } from "../../lib/i18n";
import { ipc, ipcEvent } from "../../lib/ipc";
import { KEYS, SLOTS, type KeyDef, type SlotKey } from "./keys";
import type { Shortcut } from "./shortcut";
import { DEFAULT_KEY, KeyCard, KeyColumn, type KeyConfig } from "./KeyCard";
import { FnCollapsedBar, FnChips, FnEditPanel, FN_CATEGORIES, type FnCategory } from "./FnPicker";
import { MapCenter } from "./MapCenter";
import { useKeyFlash } from "./useKeyFlash";
import { wirePath, type Wire } from "./wires";
import { SchemeSelect } from "./Schemes";

/** 与后端 ButtonConfig 对齐（三槽形状，S-A）。 */
interface ButtonConfig {
  keys: Record<string, KeyConfig>;
}
/** remote_buttons_get 的快照（hid 通道状态这里用不到，不读）。 */
interface ButtonsSnapshot {
  config: ButtonConfig;
  hid: unknown;
}

const EMPTY: ButtonConfig = { keys: {} };

/** 组合身份键（冲突判定与「已映射」标记共用）：C1 起 fn/side 参与——
 *  左⌘ 与右⌘ 是两个不同组合，fn⌘X 与 ⌘X 也是。 */
const scId = (s: Shortcut) => `${s.mods}:${s.key}:${s.fn ? 1 : 0}:${s.side ?? ""}`;

export function Mapping({
  statusBar,
  deviceSelect,
  profileKeys,
  hasImage,
}: {
  statusBar?: ReactNode;
  /** 卡头设备切换组合框（Remote.tsx 构建）：选候选=连接并切换整页档案。 */
  deviceSelect?: ReactNode;
  /** 绑定档案的键 id 表（remote_status.profile_keys）：键位区只渲染档案真实有的键。 */
  profileKeys: string[];
  /** 该档案有实物图标定资产（keys.ts::hasRemoteImage）；无图走键名网格中位。 */
  hasImage: boolean;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  // 档案键表（队列 22）：空键表档案由 Remote.tsx 的「未验证」状态卡拦下，到这里的必有键。
  const keys = useMemo(() => KEYS.filter((k) => profileKeys.includes(k.id)), [profileKeys]);
  const [sel, setSel] = useState<string | null>(null);
  // 设置态 / 运行态（用户 2026-11-19 拍板）：卡标题就是开关。
  // 设置态 = 点选/真键选中 + 灰显高亮；运行态 = 真键只闪不选：单击闪一下、双击闪两下、长按持续亮。
  const [edit, setEdit] = useState(false);
  const editRef = useRef(false);
  editRef.current = edit;
  // 实体键实时态（live/held/after）由 useKeyFlash 从 remote://key 事件驱动，见 selectKey 后调用。
  const [wires, setWires] = useState<Wire[]>([]);
  // 功能编辑面板（用户 2026-11-20 拍板内嵌化，终稿：常驻卡头下方、可折叠）：
  // fnEdit = 编辑上下文（键 + 槽，关面板不丢，随时点开接着用）；fnOpen = 展开态。
  const [fnEdit, setFnEdit] = useState<{ key: string; slot: SlotKey } | null>(null);
  const [fnOpen, setFnOpen] = useState(false);
  const fnOpenRef = useRef(fnOpen);
  fnOpenRef.current = fnOpen;
  // 实物图与热点层抽到 RemoteImage（自量图框）；这里只要它量完触发一次连线重画。
  const [imgTick, setImgTick] = useState(0);
  // 悬停键卡也画连线（用户 2026-11-20：要连线呀）——选中或悬停任一即亮这一条，仍不画全量灰线。
  const [hover, setHover] = useState<string | null>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const cardRefs = useRef(new Map<string, HTMLDivElement>());
  const hotRefs = useRef(new Map<string, HTMLButtonElement>());

  const buttons = useQuery({
    queryKey: ["remote-buttons"],
    queryFn: () => ipc<ButtonsSnapshot>("remote_buttons_get"),
  });
  // 旧后端（S-A 之前的运行实例）回的是旧形状 {bindings,rapid}，没有 keys——
  // 归一成空态整页照常可用（开发前端配旧二进制的实况；生产前后端同版发布，这是兜底）。
  const rawCfg = buttons.data?.config;
  const config = rawCfg?.keys ? rawCfg : EMPTY;
  // HID 通道错误（抑制器起不来等）必须显出来（inject 模块口径「不许静默」）——
  // 之前只沉在快照里，页面看不见，录制拦截失效都无从归因。
  const hidErr = (buttons.data?.hid as { error?: string | null } | undefined)?.error ?? null;

  // 已绑到某遥控键槽的功能芯片（绿框绿字）：编辑态与运行态都显示——
  // 退出编辑态照样一眼看清哪些功能键在用（用户 2026-11-21）。
  const mappedFns = useMemo(() => {
    const bound = new Set<string>();
    for (const kc of Object.values(config.keys))
      for (const s of SLOTS) {
        const v = kc[s.id];
        if (v) bound.add(scId(v));
      }
    const out = new Set<string>();
    for (const c of FN_CATEGORIES)
      for (const p of c.items) if (bound.has(scId(p.sc))) out.add(p.nameKey);
    return out;
  }, [config]);

  const save = useMutation({
    mutationFn: (next: ButtonConfig) => ipc("remote_buttons_set", { config: next }),
    onSuccess: () => toast.ok(t("remote.map.saved")),
    onError: (e) => toast.err(`${t("remote.map.saveFail")}：${errText(e)}`),
    // 成败都回读后端为准：失败时这一步同时把乐观写入回滚掉。
    onSettled: () => {
      void qc.invalidateQueries({ queryKey: ["remote-buttons"] });
    },
  });

  // 缓存里可能是旧形状（同 config 的归一口径）：读缓存一律走这里，缺 keys 当空态。
  const cachedConfig = (): ButtonConfig => {
    const c = qc.getQueryData<ButtonsSnapshot>(["remote-buttons"])?.config;
    return c?.keys ? c : EMPTY;
  };

  // 改动基于缓存里的最新 config 计算并乐观写回：连点两把键时，
  // 第二次若还基于 refetch 前的旧 config 整体替换，会把第一次的改动吞掉。
  const mutateConfig = (fn: (c: ButtonConfig) => ButtonConfig) => {
    const cur = cachedConfig();
    const next = fn({ keys: { ...cur.keys } });
    qc.setQueryData<ButtonsSnapshot>(["remote-buttons"], (prev) => (prev ? { ...prev, config: next } : prev));
    save.mutate(next);
  };

  const keyOf = (c: ButtonConfig, id: string): KeyConfig => ({ ...(c.keys[id] ?? DEFAULT_KEY) });

  // 三槽绑定（S-D）：前端预检全局唯一（后端兜底整份拒绝）——撞了别的键/槽
  // 就红闪 900ms + toast 说明撞在哪，**不保存**（原绑定天然保留，无需还原动作）。
  const [flash, setFlash] = useState<string | null>(null);
  const redFlashTimer = useRef(0);
  // 卸载时清红闪定时器（审查 P3.3）：回调摸已卸载 state，React 18 不告警但属脏账。
  useEffect(() => () => window.clearTimeout(redFlashTimer.current), []);
  const slotRefs = useRef(new Map<string, HTMLButtonElement>());
  const nameOf = (id: string) => t(KEYS.find((k) => k.id === id)?.nameKey ?? id);

  // 组合全局唯一检查（S-D 口径）：除 except（`${键}:${槽}`）外任何槽占了同一组合即冲突，
  // 返回撞上的「键名·槽名」。setShortcut 与功能弹层的虚拟键盘共用这一份判据，红闪口径一致。
  const conflictAt = (sc: Shortcut, except: string): string | null => {
    const cur = cachedConfig();
    for (const [kid, kc] of Object.entries(cur.keys)) {
      for (const s of SLOTS) {
        const v = kc[s.id];
        if (v && scId(v) === scId(sc) && `${kid}:${s.id}` !== except)
          return `${nameOf(kid)}·${t(s.labelKey)}`;
      }
    }
    return null;
  };

  const setShortcut = (id: string, slot: SlotKey, sc: Shortcut | null) => {
    const at = sc ? conflictAt(sc, `${id}:${slot}`) : null;
    if (at) {
      setFlash(`${id}:${slot}`);
      window.clearTimeout(redFlashTimer.current);
      redFlashTimer.current = window.setTimeout(() => setFlash(null), 900);
      toast.err(t("remote.map.slotConflict").replace("{a}", at));
      return;
    }
    mutateConfig((c) => {
      const k = keyOf(c, id);
      k[slot] = sc;
      c.keys[id] = k;
      return c;
    });
  };

  // 运行态功能键区只留已映射的芯片（用户 2026-11-21）：没绑的芯片和空分类都不显示。
  const filterMapped = (cats: FnCategory[]) =>
    cats
      .map((c) => ({ ...c, items: c.items.filter((p) => mappedFns.has(p.nameKey)) }))
      .filter((c) => c.items.length > 0);

  // 画布功能芯片的填入目标：正在聚焦直录的槽最优先（点槽进录制 → 点芯片一键映射，
  // 不必碰实物键，用户 2026-11-21）；其次面板编辑上下文（fnEdit）；再次选中键的
  // 「单击」槽；都没有才提示先点键（不猜目标，免得误写别的键）。
  const pickFn = (sc: Shortcut) => {
    const cap = captureRef.current;
    const [capKey, capSlot] = cap?.split(":") ?? [];
    const ctx = capKey
      ? { key: capKey, slot: capSlot as SlotKey }
      : fnEdit ?? (sel ? { key: sel, slot: "click" as SlotKey } : null);
    if (!ctx) {
      toast.err(t("remote.map.pickKeyFirst"));
      return;
    }
    setShortcut(ctx.key, ctx.slot, sc);
    // 填入后给录制中的槽关闸（captured 直录同口径）；撞车红闪时也一并退出录制，
    // 想改就再点一次槽。
    if (cap) slotRefs.current.get(cap)?.blur();
  };

  // 录制拦截收口（用户 2026-11-20 拍板）：槽位聚焦期间后端 tap 把全部键盘输入
  // （含遥控按键）拦下来发这里——只有聚焦槽吃到组合，别的一律不响应。
  // 聚焦槽由 SlotRow 的 focus/blur 上报记账（captureRef）：不赌 document.activeElement，
  // WKWebView 里事件到达时焦点可能已挪走，赌它就会静默丢键（回显不出现的病根）。
  // 旧后端没这条命令/事件：槽内 keydown 兜底路径照常能录，互不冲突。
  const captureRef = useRef<string | null>(null);
  const setShortcutRef = useRef(setShortcut);
  setShortcutRef.current = setShortcut;
  useEffect(() => {
    let dead = false;
    let un: (() => void) | undefined;
    // C1 起后端甄别器只在「组合完整」时发事件（主键按下，或单修饰键松开），
    // 且带 fn/side——前端不再自己滤修饰键，收到即提交。
    void ipcEvent<Shortcut>("remote://captured", (sc) => {
      if (dead) return;
      const slot = captureRef.current;
      if (!slot) return;
      const [id, s] = slot.split(":");
      setShortcutRef.current(id, s as SlotKey, sc);
      slotRefs.current.get(slot)?.blur();
    }).then((f) => (dead ? f() : (un = f)));
    // 窗口失焦（切去别的应用）立刻关闸，不许替别的应用吞键盘。
    const off = () => void ipc("remote_recording_set", { active: false }).catch(() => {});
    window.addEventListener("blur", off);
    return () => {
      dead = true;
      un?.();
      window.removeEventListener("blur", off);
      void ipc("remote_recording_set", { active: false }).catch(() => {});
    };
  }, []);

  // 点选联动（原型口径）：选中的键高亮，键卡滚到可视区。
  const selectKey = (id: string) => {
    setSel(id);
    cardRefs.current.get(id)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  };

  // 退出设置态（用户 2026-11-19 拍板）：再点一次已选中的键 = 取消选中，实物图还原原色。
  // 实体键实时联动走 selectKey（真按同键两次要保持高亮，不做 toggle）。
  // 运行态下点卡不选中（只读展示），选中动作只在设置态生效。
  const toggleKey = (id: string) => {
    if (!edit) return;
    if (sel === id) setSel(null);
    else selectKey(id);
  };

  // 标题开关：进/出设置态；退出时清选中，实物图还原原色。
  // 退出设置态同时给正在录制的槽关闸（用户 2026-11-20：点标题就能退出录制）。
  const toggleMode = () => {
    if (edit) {
      setSel(null);
      setFnOpen(false);
      slotRefs.current.get(captureRef.current ?? "")?.blur();
      void ipc("remote_recording_set", { active: false }).catch(() => {});
    }
    setEdit(!edit);
  };

  // 虚拟键盘直达（用户 2026-11-21 拍板）：一键进设置态并展开虚拟键盘面板，
  // 省掉「点标题进设置态 → 点折叠条展开」两步；面板已开着再点即收起。
  const openVirtKbd = () => {
    if (edit && fnOpen) {
      setFnOpen(false);
      return;
    }
    setEdit(true);
    setFnEdit(fnEdit ?? { key: sel ?? keys[0].id, slot: "click" });
    setFnOpen(true);
  };

  // Esc 退出设置态还原原色。功能面板开着时 Esc 归面板（关面板不退设置态）；
  // Radix 弹层（方案对话框）在 document 捕获阶段已对 Esc preventDefault，这里让位：
  // Esc 只关最上层弹层，不连带退出设置态（审查 1.2）。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        if (fnOpenRef.current) return;
        setEdit(false);
        setSel(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 实体键实时联动（用户 2026-11-19 拍板）：事件 → live/held/after，逻辑在 useKeyFlash。
  const { live, held, after } = useKeyFlash({ editRef, captureRef, slotRefs, selectKey });

  // 连线几何与原型 drawWires 一致：卡侧边中点 → 热点侧边中点的三次贝塞尔 + 箭头。
  // 布局一切变化（窗口缩放 / 实物图加载 / 边条折叠）都由 ResizeObserver 兜住重画。
  const draw = useCallback(() => {
    const cv = canvasRef.current;
    const svg = svgRef.current;
    if (!cv || !svg) return;
    const cr = cv.getBoundingClientRect();
    svg.setAttribute("viewBox", `0 0 ${cr.width} ${cr.height}`);
    const out: Wire[] = [];
    for (const k of keys) {
      const cardEl = cardRefs.current.get(k.id);
      const hotEl = hotRefs.current.get(k.id);
      if (!cardEl || !hotEl) continue;
      const kr = cardEl.getBoundingClientRect();
      const hr = hotEl.getBoundingClientRect();
      const kc = cachedConfig().keys[k.id];
      const mapped = !!kc && !!(kc.click || kc.double || kc.long);
      const y1 = kr.top + kr.height / 2 - cr.top;
      const hy = hr.top + hr.height / 2 - cr.top;
      const left = k.col === "l";
      const x1 = left ? kr.right - cr.left : kr.left - cr.left;
      const x2 = left ? hr.left - cr.left - 4 : hr.right - cr.left + 4;
      out.push({ id: k.id, ...wirePath(x1, y1, x2, hy, left), mapped });
    }
    setWires(out);
  }, [keys]);

  useEffect(() => {
    const cv = canvasRef.current;
    if (!cv) return;
    draw();
    const ro = new ResizeObserver(draw);
    ro.observe(cv);
    return () => ro.disconnect();
    // hover/sel/imgTick 也要重画：热点层在图框量出后才挂载（imgTick，比 RO 晚一个 React 周期），
    // 只挂 cv 的 RO 会拿到空/旧端点——连线连到图边的病根（用户 2026-11-20 抓）。
  }, [draw, buttons.data, hover, sel, imgTick]);

  const card = (k: KeyDef) => (
    <KeyCard
      key={k.id}
      k={k}
      kc={config.keys[k.id] ?? DEFAULT_KEY}
      selected={sel === k.id}
      live={live === k.id}
      edit={edit}
      flash={flash}
      onSelect={toggleKey}
      onSlot={setShortcut}
      onHover={setHover}
      onFocusSlot={(id) => {
        captureRef.current = id;
      }}
      onFn={(slot) => {
        setFnEdit({ key: k.id, slot });
        setFnOpen(true);
      }}
      regSlot={(key, el) => {
        if (el) slotRefs.current.set(key, el);
        else slotRefs.current.delete(key);
      }}
      setRef={(el) => {
        if (el) cardRefs.current.set(k.id, el);
        else cardRefs.current.delete(k.id);
      }}
    />
  );

  return (
    <Card>
      {/* 卡头吸顶（用户 2026-11-20 拍板：滚动时这行不动）：负 margin 撑到卡内沿，
          背景 = 卡片合成色（slate-900 60% 叠 slate-950，color-mix 双主题都严丝合缝），
          内容从条下滚过被盖住；pb 盖住 CardHead 自带的 mb-3 缝，不留透底细条。 */}
      <div className="sticky top-0 z-20 -mx-4 -mt-4 rounded-t-lg px-4 pt-4 pb-2 [background:color-mix(in_srgb,var(--color-slate-900)_60%,var(--color-slate-950))]">
        <CardHead
          icon={IconRemote}
          title={
          /* 标题即设置态开关（用户 2026-11-19 拍板）：点亮进设置态，再点退出回运行态。
             编辑中（用户 2026-11-20 拍板）：文本换红色呼吸的「编辑中...」，再点还原退出。
             其后（用户 2026-11-21 拍板）：虚拟键盘直达按钮 + 设备切换组合框。 */
          <span className="flex flex-wrap items-center gap-1.5">
          <button
            type="button"
            onClick={toggleMode}
            aria-pressed={edit}
            data-tip={t(edit ? "remote.map.editOff" : "remote.map.editOn")}
            className={`tip -my-0.5 -ml-1 flex items-center gap-1.5 rounded px-1.5 py-0.5 transition-colors ${
              edit ? "bg-[var(--sel-bg)] text-[var(--key-hl)] ring-1 ring-[var(--key-hl)]/40" : "hover:text-ink-1"
            }`}
          >
            <IconPencil className="h-3 w-3 shrink-0" />
            {edit ? <span className="live-breathe text-[var(--sem-danger-text)]">{t("remote.map.editing")}</span> : t("remote.map.title")}
          </button>
          <button
            type="button"
            onClick={openVirtKbd}
            aria-expanded={edit && fnOpen}
            data-tip={t("remote.map.virtKbdTip")}
            className={`tip -my-0.5 flex items-center gap-1.5 rounded px-1.5 py-0.5 transition-colors ${
              edit && fnOpen ? "bg-[var(--sel-bg)] text-[var(--key-hl)]" : "hover:text-ink-1"
            }`}
          >
            <IconKeyboard className="h-3 w-3 shrink-0" />
            {t("remote.map.virtKbd")}
          </button>
          {/* 方案下拉归功能组（用户 2026-11-21 拍板重排）：它动的是映射，跟按键映射/虚拟键盘放一起。 */}
          <SchemeSelect />
          {deviceSelect}
          </span>
        }
      >
        {/* 卡头右侧 = 设备状态条（用户 2026-11-20 拍板：状态条上移进这里、提示文本删，
            整页压到一屏不滚动；运行/设置两态的说明由标题按钮 data-tip 承担）。 */}
        {statusBar}
      </CardHead>
      </div>
      {hidErr && (
        <div className="mb-2">
          <QueryError inline title={hidErr} onRetry={() => void buttons.refetch()} />
        </div>
      )}
      {buttons.isError ? (
        <div className="mt-3">
          <QueryError
            inline
            title={t("remote.map.loadFail")}
            error={buttons.error}
            onRetry={() => void buttons.refetch()}
          />
        </div>
      ) : !buttons.data ? (
        <div className="mt-3">
          <SkeletonRows rows={4} />
        </div>
      ) : (
        <>
        {/* 虚拟键盘面板（用户 2026-11-20 终稿）：常驻卡头正下方，折叠成一条栏，
            编辑态随时可点开；点槽位「功能」带键/槽上下文直接展开。 */}
        {edit &&
          (fnOpen && fnEdit ? (
            <FnEditPanel
              key={`${fnEdit.key}:${fnEdit.slot}`}
              fnEdit={fnEdit}
              config={config.keys}
              conflictAt={conflictAt}
              hasImage={hasImage}
              keys={keys}
              // 连续映射：填入不关面板——换键换槽接着设，Esc/折叠才收。
              onCommit={(s, sc) => setShortcut(fnEdit.key, s, sc)}
              onSlotChange={(s) => setFnEdit({ ...fnEdit, slot: s })}
              onClear={(s) => setShortcut(fnEdit.key, s, null)}
              onKeyChange={(id) => {
                setFnEdit({ ...fnEdit, key: id });
                setSel(id);
              }}
              // 副本与主图联动（用户 2026-11-21）：悬停/闪烁态同源，两边实物图一起亮。
              mirror={{ hover, onHover: setHover, live, held, after }}
              onClose={() => setFnOpen(false)}
              onCollapse={() => setFnOpen(false)}
            />
          ) : (
            <FnCollapsedBar
              onOpen={() => {
                setFnEdit(fnEdit ?? { key: sel ?? keys[0].id, slot: "click" });
                setFnOpen(true);
              }}
            />
          ))}
        {/* 窄窗收档（审查 2.2）：映射区给最小宽度 + 横向滚动，窗口再窄也不无声挤压键列。 */}
        <div className="overflow-x-auto">
        <div
          ref={canvasRef}
          className="relative mt-4 grid min-w-[920px] grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-6"
        >
          <svg ref={svgRef} aria-hidden="true" className="pointer-events-none absolute inset-0 z-0 h-full w-full">
            {/* 两类线（用户 2026-11-20 拍板，原型回灌时保留）：① 常显虚线——三槽有任一映射的键，
                行↔实物键平时就牵着，一眼看清哪些键绑了、绑在哪；没映射的键不连。
                ② 高亮实线——悬停优先、没悬停显选中，荧光青加箭头，压在虚线之上、互不影响。 */}
            {wires.map((w) =>
              w.id !== (hover ?? sel) && w.mapped ? (
                <path
                  key={w.id}
                  d={w.d}
                  fill="none"
                  className="stroke-[var(--key-hl)]/45"
                  strokeWidth={2.5}
                  strokeDasharray="7 6"
                />
              ) : null,
            )}
            {wires.map((w) =>
              w.id === (hover ?? sel) ? (
                <g key={w.id} className="stroke-[var(--key-hl)]">
                  <path d={w.d} fill="none" strokeWidth={4} />
                  <path d={w.arrow} fill="none" strokeWidth={3.5} />
                </g>
              ) : null,
            )}
          </svg>
          <div className="flex min-w-0 flex-col gap-3 self-start">
            <KeyColumn side="l">{keys.filter((k) => k.col === "l").map(card)}</KeyColumn>
            {/* 功能键区（用户 2026-11-21 拍板）：键列下方空白正好放分类芯片，
                点一下 = 填入当前编辑上下文（面板目标槽优先，否则选中键的单击槽）；
                已映射的芯片绿框标出；退出编辑态只显示已映射的（只读），没绑的不显。 */}
            <FnChips
              cats={edit ? FN_CATEGORIES.slice(0, 2) : filterMapped(FN_CATEGORIES.slice(0, 2))}
              onPick={pickFn}
              mapped={mappedFns}
              disabled={!edit}
            />
          </div>
          <MapCenter
            hasImage={hasImage}
            keys={keys}
            edit={edit}
            sel={sel}
            hover={hover}
            live={live}
            held={held}
            after={after}
            onSelect={toggleKey}
            onHover={setHover}
            regHot={(id, el) => {
              if (el) hotRefs.current.set(id, el);
              else hotRefs.current.delete(id);
            }}
            onMeasure={() => setImgTick((v) => v + 1)}
            className="z-10 self-center"
            // 原型口径：图高 clamp(400px,54vh,500px)、宽按比例——再大就挤没两侧键列的可读宽度。
            imgClassName="h-[clamp(400px,54vh,500px)]"
          />
          <div className="flex min-w-0 flex-col gap-3 self-start">
            <KeyColumn side="r">{keys.filter((k) => k.col === "r").map(card)}</KeyColumn>
            <FnChips
              cats={edit ? FN_CATEGORIES.slice(2) : filterMapped(FN_CATEGORIES.slice(2))}
              onPick={pickFn}
              mapped={mappedFns}
              disabled={!edit}
            />
          </div>
        </div>
        </div>
        </>
      )}
    </Card>
  );
}
