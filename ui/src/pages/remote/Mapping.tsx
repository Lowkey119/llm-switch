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
import { IconPencil, IconRemote } from "../../components/icons";
import { t } from "../../lib/i18n";
import { zoomFactor } from "../../lib/zoom";
import { ipc, ipcEvent } from "../../lib/ipc";
import { HOT_F, KEYS, SLOTS, type KeyDef, type SlotKey } from "./keys";
import { fmtShortcut, type Shortcut } from "./shortcut";
import { asShortcut, shortcutOf, type Action } from "./action";
import { DEFAULT_KEY, KeyCard, KeyColumn, type KeyConfig } from "./KeyCard";
import { FnEditPanel, FN_CATEGORIES } from "./FnPicker";
import { MacroBoardPane, useMacroBoard } from "./MacroLibrary";
import { RecDock } from "./RecDock";
import { countMacroBindings, type Macro } from "./macros";
import { VoiceDock } from "./VoiceDock";
import { MapCenter } from "./MapCenter";
import { useKeyFlash } from "./useKeyFlash";
import { wirePath, type Wire } from "./wires";
import { SchemeSelect } from "./Schemes";
import { KeyTimingPanel } from "./KeyTimingPanel";

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
  deviceTip,
  profileKeys,
  hasImage,
  voiceAvailable,
}: {
  statusBar?: ReactNode;
  /** 卡头设备切换组合框（Remote.tsx 构建）：选候选=连接并切换整页档案。 */
  deviceSelect?: ReactNode;
  /** 绑定档案的键 id 表（remote_status.profile_keys）：键位区只渲染档案真实有的键。 */
  profileKeys: string[];
  /** 该档案有实物图标定资产（keys.ts::hasRemoteImage）；无图走键名网格中位。 */
  hasImage: boolean;
  /** 档案有语音通道（remote_status.profile_voice）：没有就不画语音坞（旧「按档案能力摆卡」口径）。 */
  voiceAvailable?: boolean;
  /** 设备型号文案（用户 2026-11-24 拍板）：卡头不再挂设备组合框，型号悬停在遥控器图标上。 */
  deviceTip?: string;
}) {
  const qc = useQueryClient();
  const toast = useToast();
  // 档案键表（队列 22）：空键表档案由 Remote.tsx 的「未验证」状态卡拦下，到这里的必有键。
  // 原生透传键（方向四键 / 确定键 / 返回键）不进网格：它们不参与映射，也就没有可编辑的格。
  const keys = useMemo(
    () => KEYS.filter((k) => profileKeys.includes(k.id)),
    [profileKeys],
  );
  const [sel, setSel] = useState<string | null>(null);
  // 宏库（动作库）：列表窗格与编辑窗格共用这一份状态。
  // 2026-11-24 用户拍板：整块移入居中弹层，映射画布独占屏幕——卡头常驻入口 +
  // 槽位录制浮层「动作库」按需入口，编辑完回槽位挑绑。
  const macroB = useMacroBoard();
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
  // 中位列宽 = 画布行高 × 裁切图宽高比（draw 里量，HOT_F 即 622/1432）：
  // 图 absolute 脱流不撑行，列宽得由行高反推，否则 auto 列塌成 0。
  const [imgW, setImgW] = useState(0);
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

  // 功能芯片不再摆在键列下方（用户 2026-11-23 拍板）：改为悬停「定义它的那一格」时浮出名字。
  // 芯片排既占掉键列下方的空白，读数又与槽位重复；信息回到所属的格上，画布只剩键与实物图。
  // 只有快捷键动作能对得上芯片（C2）：其余动作类型（宏 / 文本 / 打开 App…）不参与。
  const fnTipOf = useMemo(() => {
    const bySc = new Map<string, string>();
    for (const c of FN_CATEGORIES)
      for (const p of c.items) bySc.set(scId(p.sc), `${t(p.nameKey)} ${fmtShortcut(p.sc)}`);
    return (a: Action | null | undefined) => {
      const sc = a ? shortcutOf(a) : undefined;
      return sc ? bySc.get(scId(sc)) : undefined;
    };
  }, []);

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
  // 返回撞上的「键名·槽名」。setSlotAction 与功能面板的虚拟键盘共用这一份判据，红闪口径一致。
  const conflictAt = (sc: Shortcut, except: string): string | null => {
    const cur = cachedConfig();
    for (const [kid, kc] of Object.entries(cur.keys)) {
      for (const s of SLOTS) {
        const v = shortcutOf(kc[s.id]);
        if (v && scId(v) === scId(sc) && `${kid}:${s.id}` !== except)
          return `${nameOf(kid)}·${t(s.labelKey)}`;
      }
    }
    return null;
  };

  // 三槽写入（C2 起槽里是动作）：全局唯一只对快捷键判定，
  // 打开 App / 命令 / 文本重复是合法配置（计划 §三）。
  const setSlotAction = (id: string, slot: SlotKey, action: Action | null) => {
    const sc = shortcutOf(action);
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
      k[slot] = action;
      c.keys[id] = k;
      return c;
    });
  };

  // 录制拦截收口（用户 2026-11-20 拍板）：槽位聚焦期间后端 tap 把全部键盘输入
  // （含遥控按键）拦下来发这里——只有聚焦槽吃到组合，别的一律不响应。
  // 聚焦槽由 SlotRow 的 focus/blur 上报记账（captureRef）：不赌 document.activeElement，
  // WKWebView 里事件到达时焦点可能已挪走，赌它就会静默丢键（回显不出现的病根）。
  // 旧后端没这条命令/事件：槽内 keydown 兜底路径照常能录，互不冲突。
  const captureRef = useRef<string | null>(null);
  // 录制槽读数（captureRef 是 ref 不触发重渲染，另存 state 给 UI）；派生值含绑定键槽数。
  const [recSlot, setRecSlot] = useState<string | null>(null);
  const macroBind = useMemo(() => {
    const [kid, slot] = recSlot?.split(":") ?? [];
    const sl = slot ? SLOTS.find((s) => s.id === slot) : undefined;
    return { target: kid ? `${nameOf(kid)}${sl ? `·${t(sl.labelKey)}` : ""}` : null, count: macroB.shown?.id ? countMacroBindings(config.keys, macroB.shown.id) : 0 };
  }, [recSlot, config, macroB.shown]);
  /** 「绑定当前」：优先录制槽，退到功能面板上下文 / 选中键的单击槽。 */
  const bindMacroToSlot = (m: Macro) => {
    const rec = recSlot?.split(":");
    const ctx = rec
      ? { key: rec[0], slot: rec[1] as SlotKey }
      : fnEdit ?? (sel ? { key: sel, slot: "click" as SlotKey } : null);
    if (!ctx) {
      toast.err(t("remote.map.pickKeyFirst"));
      return;
    }
    setSlotAction(ctx.key, ctx.slot, { type: "macro", id: m.id });
  };
  const setSlotRef = useRef(setSlotAction);
  setSlotRef.current = setSlotAction;
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
      setSlotRef.current(id, s as SlotKey, asShortcut(sc));
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
      setRecSlot(null);
      slotRefs.current.get(captureRef.current ?? "")?.blur();
      void ipc("remote_recording_set", { active: false }).catch(() => {});
    }
    setEdit(!edit);
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
    // getBoundingClientRect 返回缩放后（×zoom）尺寸；viewBox 与 path 坐标同乘 zoom 抵消，
    // 但 imgW 是文档空间宽度，必须换回文档高，否则 zoom≠1 时中位列被拉宽 z 倍（用户：缩放失真）。
    const z = zoomFactor() || 1;
    svg.setAttribute("viewBox", `0 0 ${cr.width} ${cr.height}`);
    // 中位列宽随画布高（= 行高）走：宽 = 文档高 × 宽高比（HOT_F = 622/1432）。
    // 同值不 setState，防 RO→setState→RO 回环。
    const w = Math.round(cr.height / z * HOT_F);
    setImgW((v) => (v === w ? v : w));
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
      onSlot={setSlotAction}
      fnTipOf={fnTipOf}
      macros={macroB.list}
      onHover={setHover}
      onFocusSlot={(id) => {
        captureRef.current = id;
        setRecSlot(id);
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
    <Card className="flex min-h-0 flex-1 flex-col">
      {/* 卡头吸顶（用户 2026-11-20 拍板：滚动时这行不动）：负 margin 撑到卡内沿，
          背景 = 卡片合成色（slate-900 60% 叠 slate-950，color-mix 双主题都严丝合缝），
          内容从条下滚过被盖住；pb 盖住 CardHead 自带的 mb-3 缝，不留透底细条。 */}
      <div className="sticky top-0 z-20 -mx-3 -mt-3 rounded-t-lg px-3 pt-1.5 pb-0.5 [background:color-mix(in_srgb,var(--color-slate-900)_60%,var(--color-slate-950))]">
        <CardHead
          icon={IconRemote}
          iconTip={deviceTip}
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
          {/* 方案下拉归功能组（用户 2026-11-21 拍板重排）：它动的是映射，跟按键映射放一起。 */}
          <SchemeSelect />
          {/* 键时序（C4）：双击窗/长按窗/按住连发，一个档案一份。 */}
          <KeyTimingPanel />
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
        {/* 虚拟键盘面板（用户 2026-11-20 终稿）：常驻卡头正下方，编辑态随时可点开。 */}
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
              onCommit={(s, a) => setSlotAction(fnEdit.key, s, a)}
              onSlotChange={(s) => setFnEdit({ ...fnEdit, slot: s })}
              onClear={(s) => setSlotAction(fnEdit.key, s, null)}
              onKeyChange={(id) => {
                setFnEdit({ ...fnEdit, key: id });
                setSel(id);
              }}
              mirror={{ hover, onHover: setHover, live, held, after }}
              onClose={() => setFnOpen(false)}
              onCollapse={() => setFnOpen(false)}
            />
          ) : null)}
        {/* 视口弹性布局（用户 2026-11-24 终稿）：整页撑满可视区、禁横向滚动；
            画布行高 = 键列自然高（min-content），多出来的高度整份让给下方动作库带。 */}
        <div
          ref={canvasRef}
          className="relative mt-2 grid shrink-0 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] gap-3"
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
          <div className="flex min-w-0 flex-col gap-3 min-h-0">
            <KeyColumn side="l">{keys.filter((k) => k.col === "l").map(card)}</KeyColumn>
          </div>
          <div className="relative z-10 min-h-0" style={{ width: imgW }}>
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
              // 中位居中（用户 2026-11-24 拍板「左右间距相等」）：列宽 = 行高 × 宽高比
              // （imgW，draw 里量），图 absolute inset-0 严丝合缝填满列——列居中于两
              // 等宽 1fr 键列之间 ⇒ 到左右表格间距相等；图与热点层同锚点，连线不漂。
              className="h-full"
              imgClassName=""
            />
          </div>
          <div className="flex min-w-0 flex-col gap-3 min-h-0">
            <KeyColumn side="r">{keys.filter((k) => k.col === "r").map(card)}</KeyColumn>
          </div>
        </div>
        {/* 水平线下方带（用户 2026-11-24 定稿）：左大框 = 动作面板，右框 = 语音坞，
            与画布同一三列网格（中列 = imgW）严丝合缝对齐；min-w-0 防编辑器顶破列宽。 */}
        <div
          className="mt-3 grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)] gap-3 [&>*]:min-w-0 [&>*]:min-h-0"
          // 中列宽 = imgW（draw 里量），与画布三列同宽：左框 col-span-2 跨左键列+中列，右框对齐右键列。
          style={{ gridTemplateColumns: `minmax(0,1fr) ${imgW}px minmax(0,1fr)` }}
        >
          {/* 录制态（用户 2026-11-25 拍板）：底栏整块换成全功能虚拟键盘坞，点键拼组合填入录制槽。 */}
          {edit && recSlot ? (() => {
            const [kid, slot] = recSlot.split(":");
            return (
              <div className="col-span-3 min-h-0">
                <RecDock slotLabel={macroBind.target ?? recSlot} macros={macroB.list}
                  onCommit={(a) => kid && slot && setSlotAction(kid, slot as SlotKey, a)} />
              </div>
            );
          })() : (
            <>
              <MacroBoardPane b={macroB} className={voiceAvailable ? "col-span-2" : "col-span-3"}
                bindTarget={macroBind.target} boundCount={macroBind.count}
                onBind={edit ? bindMacroToSlot : undefined} />
              {voiceAvailable ? <VoiceDock /> : <div aria-hidden="true" />}
            </>
          )}
        </div>
        </>
      )}
    </Card>
  );
}
