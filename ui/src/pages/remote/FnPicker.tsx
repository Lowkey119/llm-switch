// 功能编辑面板（FnPicker）：内嵌在映射画布下方（用户 2026-11-20 拍板：不再弹对话框——
// 点槽位「功能」→ 面板在实物图/键卡下方展开并滚动到位，可折叠可关闭，键卡全程可见）。
// 槽定义（SLOTS）在 keys.ts，键码表在 shortcut.ts。
import { useCallback, useEffect, useRef, useState } from "react";
import { IconChevron, IconClose } from "../../components/icons";
import { os } from "../../lib/key";
import { t } from "../../lib/i18n";
import { KEYS, SLOTS, type KeyDef, type SlotKey } from "./keys";
import { fmtShortcut, type Shortcut } from "./shortcut";
import { asShortcut, fmtAction, shortcutOf, type Action, type ActionType } from "./action";
import { ActionForm, ActionTypeTabs } from "./ActionForms";
import { fnCategoriesFor } from "./fnPresets";
import { useMacroLabels } from "./macros";
import { DEFAULT_KEY, type KeyConfig } from "./KeyCard";
import { MapCenter } from "./MapCenter";
import { physicalKey, VirtualKeyboard } from "./VirtualKeyboard";
import { wirePath, type Wire } from "./wires";

/** 功能分类：分类名 + 芯片（kVK 组合）。 */
export interface FnCategory {
  nameKey: string;
  items: { nameKey: string; sc: Shortcut }[];
}

/**
 * 常用功能按键（用户 2026-11-21 拍板：分类网格摆在键盘下方，点一下 = 直接填入当前槽）。
 * 键码为 kVK（与后端注入同一口径）；媒体键 72/73/74 与前端 KVK_LABEL、
 * 后端 inject_shortcut 媒体特判同源。
 */
/** macOS 系统功能预设（其余 OS 见 fnPresets.ts，按部署系统自动切换）。 */
const FN_CATEGORIES_MAC: FnCategory[] = [
  {
    nameKey: "remote.fn.catWindow",
    items: [
      { nameKey: "remote.fn.mission", sc: { mods: 1, key: 126 } },
      { nameKey: "remote.fn.windows", sc: { mods: 1, key: 125 } },
      { nameKey: "remote.fn.spaceLeft", sc: { mods: 1, key: 123 } },
      { nameKey: "remote.fn.spaceRight", sc: { mods: 1, key: 124 } },
      { nameKey: "remote.fn.desktop", sc: { mods: 0, key: 103 } },
      { nameKey: "remote.fn.minimize", sc: { mods: 8, key: 46 } },
      { nameKey: "remote.fn.minimizeAll", sc: { mods: 12, key: 46 } },
      { nameKey: "remote.fn.closeWindow", sc: { mods: 8, key: 13 } },
      { nameKey: "remote.fn.fullscreen", sc: { mods: 9, key: 3 } },
    ],
  },
  {
    nameKey: "remote.fn.catMedia",
    items: [
      { nameKey: "remote.fn.volUp", sc: { mods: 0, key: 72 } },
      { nameKey: "remote.fn.volDown", sc: { mods: 0, key: 73 } },
      { nameKey: "remote.fn.mute", sc: { mods: 0, key: 74 } },
    ],
  },
  {
    nameKey: "remote.fn.catApp",
    items: [
      { nameKey: "remote.fn.hideApp", sc: { mods: 8, key: 4 } },
      { nameKey: "remote.fn.hideOthers", sc: { mods: 12, key: 4 } },
      { nameKey: "remote.fn.quitApp", sc: { mods: 8, key: 12 } },
      { nameKey: "remote.fn.forceQuit", sc: { mods: 12, key: 53 } },
    ],
  },
  {
    nameKey: "remote.fn.catSystem",
    items: [
      { nameKey: "remote.fn.spotlight", sc: { mods: 8, key: 49 } },
      { nameKey: "remote.fn.emoji", sc: { mods: 9, key: 49 } },
      { nameKey: "remote.fn.inputSwitch", sc: { mods: 1, key: 49 } },
      { nameKey: "remote.fn.lock", sc: { mods: 9, key: 12 } },
      { nameKey: "remote.fn.screenshotSel", sc: { mods: 10, key: 23 } },
      { nameKey: "remote.fn.screenshotFull", sc: { mods: 10, key: 20 } },
      { nameKey: "remote.fn.emptyTrash", sc: { mods: 10, key: 51 } },
      { nameKey: "remote.fn.showHidden", sc: { mods: 10, key: 47 } },
    ],
  },
  {
    nameKey: "remote.fn.catZoom",
    items: [
      // macOS 辅助功能「缩放视图」原生快捷键（系统设置里启用「使用键盘快捷键缩放」后生效）：
      // ⌘⌥= 放大、⌘⌥- 缩小、⌘⌥8 开关。走普通键盘事件注入，系统自己响应。
      { nameKey: "remote.fn.zoomIn", sc: { mods: 12, key: 24 } },
      { nameKey: "remote.fn.zoomOut", sc: { mods: 12, key: 27 } },
      { nameKey: "remote.fn.zoomToggle", sc: { mods: 12, key: 28 } },
    ],
  },
];

/** 系统功能预设按部署 OS 取集（用户 2026-11-25：跨平台，预设随系统变）。 */
export const FN_CATEGORIES: FnCategory[] = fnCategoriesFor(os, FN_CATEGORIES_MAC);

/** 功能芯片组（虚线分组框）：面板键盘下方与画布键列下方共用；onPick = 填入当前目标槽。
 *  mapped = 已绑到某遥控键槽的芯片 nameKey 集合（绿框绿字标出）；disabled = 只读展示。 */
export function FnChips({
  cats,
  onPick,
  className = "",
  mapped,
  disabled = false,
}: {
  cats: FnCategory[];
  onPick: (sc: Shortcut) => void;
  className?: string;
  mapped?: Set<string>;
  disabled?: boolean;
}) {
  return (
    <div className={`flex flex-wrap gap-2.5 ${className}`}>
      {cats.map((c) => (
        <div
          key={c.nameKey}
          aria-label={t(c.nameKey)}
          className="flex flex-wrap gap-1.5 rounded-[10px] border border-dashed border-slate-600 px-3 py-2"
        >
          {c.items.map((p) => (
            <button
              key={p.nameKey}
              type="button"
              disabled={disabled}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => onPick(p.sc)}
              aria-label={`${t(c.nameKey)}：${t(p.nameKey)} ${fmtShortcut(p.sc)}${mapped?.has(p.nameKey) ? "（已映射）" : ""}`}
              className={`flex items-baseline gap-1.5 rounded-md border px-2.5 py-1 text-sm transition-colors ${
                mapped?.has(p.nameKey)
                  ? "border-[var(--sem-ok-text)] text-[var(--sem-ok-text)]"
                  : "border-slate-700 text-ink-1"
              } ${disabled ? "cursor-default" : "hover:border-[var(--key-hl)] hover:bg-[var(--key-hl)]/10"}`}
            >
              {t(p.nameKey)}
              <span className="text-2xs tabular-nums text-ink-4">{fmtShortcut(p.sc)}</span>
            </button>
          ))}
        </div>
      ))}
    </div>
  );
}

/** 实物图镜像态（Mapping 透传）：副本与主图共享悬停/闪烁态，两边实物图一起亮（联动）。 */
export type RemoteMirror = {
  hover: string | null;
  onHover: (id: string | null) => void;
  live: string | null;
  held: string | null;
  after: string | null;
};

/**
 * 功能编辑面板（原型 docs/ui_03 回灌，2026-11-21）：左 = 虚拟键盘（倒 T 方向键簇）+
 * 虚线分组的功能芯片；中栏 = 当前键读数 + 三态编辑框（悬停 ✕ 清除）+ 提示；
 * 右 = 实物图副本（点键换编辑键）。无面板头无 ✕（Esc 关、右上角折叠钮收起）。
 * 点修饰键（可多选）后点主键 = 组合立即填入当前槽；功能芯片点一下直接填入；
 * 撞了别的键槽 → 槽位红闪、不填（父层判据）。
 */
export function FnPicker({
  slot,
  onSlotChange,
  onClear,
  values,
  init,
  conflict,
  onCommit,
  onClose,
  onCollapse,
  keyId,
  onKeyChange,
  keyName,
  mirror,
  hasImage,
  keys,
}: {
  /** 当前目标槽（复刻卡选中项）。 */
  slot: SlotKey;
  onSlotChange: (s: SlotKey) => void;
  /** 清除某槽绑定（原型 ui_03：槽框悬停 ✕）。 */
  onClear: (s: SlotKey) => void;
  /** 三槽现值（复刻卡上显示，切槽一眼看清各槽绑了啥）。C2 起槽值是动作。 */
  values: Record<SlotKey, Action | null>;
  init: Action | null;
  conflict: (sc: Shortcut, slot: SlotKey) => boolean;
  onCommit: (slot: SlotKey, action: Action) => void;
  onClose: () => void;
  /** 收起成一条栏（折叠钮，编辑态随时可再点开）。 */
  onCollapse: () => void;
  /** 正在映射的遥控键 id + 切换口（用户 2026-11-20 拍板：切换卡下拉换键，
   *  不回滚实物界面就能把所有键连续一个个设完）。 */
  keyId: string;
  onKeyChange: (id: string) => void;
  keyName: string;
  mirror: RemoteMirror;
  /** 档案有无实物图（队列 22）：无图档案副本走键名网格。 */
  hasImage: boolean;
  keys: KeyDef[];
}) {
  const slotLabel = t(SLOTS.find((s) => s.id === slot)!.labelKey);
  const macroName = useMacroLabels();
  const CurIcon = KEYS.find((x) => x.id === keyId)?.icon;
  // 起势跟着当前槽的动作走：快捷键动作回填组合，其余类型直接切到对应表单。
  const initRef = useRef(init);
  initRef.current = init;
  const initSc = shortcutOf(init);
  const [mods, setMods] = useState(initSc?.mods ?? 0);
  const [main, setMain] = useState<number | null>(initSc?.key ?? null);
  const [atype, setAtype] = useState<ActionType>(init?.type ?? "shortcut");
  // 换键/换槽重新对齐（同一槽内提交后不重置，免得字打到一半被拽回快捷键面板）。
  useEffect(() => {
    const cur = initRef.current;
    setAtype(cur?.type ?? "shortcut");
    const sc = shortcutOf(cur);
    setMods(sc?.mods ?? 0);
    setMain(sc?.key ?? null);
  }, [keyId, slot]);
  const [bad, setBad] = useState(false);
  const boxRef = useRef<HTMLDivElement | null>(null);
  // 中栏状态框 ↔ 副本实物热点的连线（用户 2026-11-21：有映射就连线，机制与画布一样——
  // 常显虚线 + 悬停优先、当前槽兜底的高亮实线，高亮最后画压顶）。
  const svgRef = useRef<SVGSVGElement | null>(null);
  const slotBoxRefs = useRef(new Map<SlotKey, HTMLButtonElement>());
  const hotRefs = useRef(new Map<string, HTMLButtonElement>());
  const [hoverSlot, setHoverSlot] = useState<SlotKey | null>(null);
  const [pwires, setPwires] = useState<Wire[]>([]);
  const [imgTick, setImgTick] = useState(0);
  const drawP = useCallback(() => {
    const box = boxRef.current;
    const svg = svgRef.current;
    if (!box || !svg) return;
    const br = box.getBoundingClientRect();
    svg.setAttribute("viewBox", `0 0 ${br.width} ${br.height}`);
    const hot = hotRefs.current.get(keyId);
    const out: Wire[] = [];
    if (hot)
      for (const s of SLOTS) {
        const el = slotBoxRefs.current.get(s.id);
        if (!el || !values[s.id]) continue; // 有映射才连（画布同口径）
        const er = el.getBoundingClientRect();
        const hr = hot.getBoundingClientRect();
        out.push({
          id: s.id,
          mapped: true,
          ...wirePath(
            er.right - br.left,
            er.top + er.height / 2 - br.top,
            hr.left - br.left - 4,
            hr.top + hr.height / 2 - br.top,
            true,
          ),
        });
      }
    setPwires(out);
  }, [keyId, values]);
  useEffect(() => {
    drawP();
    const ro = new ResizeObserver(drawP);
    if (boxRef.current) ro.observe(boxRef.current);
    return () => ro.disconnect();
  }, [drawP, imgTick]);
  const tryCommit = (sc: Shortcut): boolean => {
    if (conflict(sc, slot)) {
      setBad(true);
      window.setTimeout(() => setBad(false), 900);
      return false;
    }
    onCommit(slot, asShortcut(sc));
    return true;
  };
  // 点主键/物理主键 = 组合立即填入当前槽（回显行 + 填入钮已删，用户 2026-11-21）；
  // 撞了别的键槽 → 槽位红闪、不填、组合保留接着改。
  const pressMain = (kvk: number) => {
    if (tryCommit({ mods, key: kvk })) {
      setMods(0);
      setMain(null);
    }
  };
  // 挂载即滚进视野（用户 2026-11-20 拍板：进入方式 = 滚动到面板，不再弹层抢焦点）。
  useEffect(() => {
    boxRef.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, []);
  // 真实按键响应（用户 2026-11-20 草图）：物理键盘按下直接拼组合——修饰键按下即点亮、
  // 松手不熄（与点虚拟键同口径，再点才熄）。槽位正被聚焦直录（用户去点了别的槽）时
  // 按键归槽位、面板不抢；Esc 关面板（内嵌后没有 Radix 兜底，自己吃）。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.repeat) return;
      // 焦点在输入控件、槽位，或别的弹层里（宏库用 Radix Dialog，焦点被锁在里面）时不抢按键。
      if (
        (document.activeElement as HTMLElement | null)?.closest(
          "[data-slot], select, input, textarea, [role='dialog']",
        )
      )
        return;
      if (e.key === "Escape") {
        onClose();
        return;
      }
      // 非快捷键动作：物理键盘归上面的输入框，不许再来拼组合。
      if (atype !== "shortcut") return;
      e.preventDefault();
      const { mod, kvk } = physicalKey(e);
      if (mod) setMods((v) => v | mod);
      else if (kvk !== undefined) pressMain(kvk);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, pressMain, atype]);
  const noSteal = { onMouseDown: (e: React.MouseEvent) => e.preventDefault() };
  return (
    <div
      ref={boxRef}
      aria-label={`${keyName} · ${slotLabel}`}
      className="relative mt-4 rounded-lg border border-slate-700 bg-slate-950 p-8"
    >
      {/* 连线层：中栏状态框 → 副本实物热点（有映射的槽），机制与画布一致——
          虚线常显、高亮实线（悬停槽框优先、当前槽兜底）最后画压顶。 */}
      <svg ref={svgRef} aria-hidden="true" className="pointer-events-none absolute inset-0 z-0 h-full w-full">
        {pwires.map((w) =>
          w.id !== (hoverSlot ?? slot) ? (
            <path key={w.id} d={w.d} fill="none" className="stroke-[var(--key-hl)]/45" strokeWidth={2.5} strokeDasharray="7 6" />
          ) : null,
        )}
        {pwires.map((w) =>
          w.id === (hoverSlot ?? slot) ? (
            <g key={w.id} className="stroke-[var(--key-hl)]">
              <path d={w.d} fill="none" strokeWidth={4} />
              <path d={w.arrow} fill="none" strokeWidth={3.5} />
            </g>
          ) : null,
        )}
      </svg>
      {/* 折叠钮（用户 2026-11-20 终稿：面板头与 ✕ 删除，Esc 关面板）：浮在右上角，收起成一条栏。 */}
      <button
        type="button"
        {...noSteal}
        aria-label={t("remote.fn.collapse")}
        onClick={onCollapse}
        className="absolute right-2 top-2 z-10 grid h-6 w-6 place-items-center rounded text-ink-4 transition-colors hover:bg-slate-800 hover:text-ink-1"
      >
        <IconChevron className="h-3.5 w-3.5 rotate-90" />
      </button>
      {/* 原型 ui_03 骨架：三栏 gap 24px = 面板内衬节奏；items-center 中/右栏纵向居中。 */}
      <div className="flex min-w-0 items-center gap-6">
          {/* 左：动作类型分段控件 + 该类型的录入区。快捷键 = 虚拟键盘 + 功能芯片；
              其余四类（C2）换成各自的表单，提交口与虚拟键盘同一个。 */}
          <div className="min-w-0 flex-1">
            <ActionTypeTabs value={atype} onChange={setAtype} />
            {atype === "shortcut" ? (
              <>
                <VirtualKeyboard
                  mods={mods}
                  main={main}
                  onMod={(bit) => setMods((v) => v ^ bit)}
                  onMain={pressMain}
                />
                {/* 常用功能按键组（原型 ui_03）：虚线分组框横排、放得下就并排（媒体+应用
                    同排），框线即分组；点芯片 = 直接填入当前槽；撞车槽位红闪。 */}
                <FnChips cats={FN_CATEGORIES} onPick={tryCommit} className="mt-5" />
              </>
            ) : (
              <ActionForm type={atype} onCommit={(a) => onCommit(slot, a)} />
            )}
          </div>
          {/* 中栏（原型 ui_03）：当前键读数（无下拉——点右栏实物键切换）+ 三态编辑框
              （点框 = 设为待录入目标，悬停现值出 ✕ 清除）+ 操作提示。 */}
          <div className="flex w-[216px] shrink-0 translate-x-8 -translate-y-12 flex-col gap-3.5">
            <div className="flex items-center justify-center gap-2 text-[var(--sem-ok-text)]">
              {CurIcon && <CurIcon className="h-4 w-4" aria-hidden />}
              <span className="text-base font-semibold">{keyName}</span>
            </div>
            {SLOTS.map((s) => {
              const v = values[s.id];
              const on = s.id === slot;
              return (
                <div key={s.id} className="group/slot flex flex-col gap-1">
                  <div className="relative">
                    <button
                      type="button"
                      aria-pressed={on}
                      {...noSteal}
                      ref={(el) => {
                        if (el) slotBoxRefs.current.set(s.id, el);
                        else slotBoxRefs.current.delete(s.id);
                      }}
                      onMouseEnter={() => setHoverSlot(s.id)}
                      onMouseLeave={() => setHoverSlot(null)}
                      onClick={() => onSlotChange(s.id)}
                      className={`h-[38px] w-full truncate rounded-lg border px-2 text-center text-sm font-bold tabular-nums transition-colors ${
                        on && bad ? "slot-conflict " : ""
                      }${
                        on
                          ? "border-[var(--key-hl)] ring-[3px] ring-[var(--key-hl)]/20"
                          : "border-slate-600 hover:border-slate-400"
                      } ${v ? "text-[var(--sem-ok-text)]" : "font-normal text-ink-4"}`}
                    >
                      {v ? fmtAction(v, macroName) : t("remote.map.slotEmpty")}
                    </button>
                    {v && (
                      <button
                        type="button"
                        aria-label={t("remote.map.slotClear")}
                        {...noSteal}
                        onClick={() => onClear(s.id)}
                        className="invisible absolute right-1 top-1/2 grid h-5 w-5 -translate-y-1/2 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-ink-1 group-hover/slot:visible"
                      >
                        <IconClose className="h-3 w-3" />
                      </button>
                    )}
                  </div>
                  <span className={`text-center text-2xs ${on ? "font-semibold text-ink-1" : "text-ink-3"}`}>
                    {t(s.labelKey)}
                  </span>
                </div>
              );
            })}
            <p className="text-center text-sm font-semibold leading-5 text-ink-3">
              {t(atype === "shortcut" ? "remote.fn.tip" : "remote.act.tip")}
            </p>
          </div>
          {/* 右栏（原型 ui_03）：实物图副本按高定宽（clamp 440~600px），列宽随图，
              与主图同逻辑联动；点键 = 换编辑键。 */}
          <div className="flex shrink-0 flex-col justify-center">
            {/* 实物图副本（用户 2026-11-21 拍板：copy，不是搬走）：与下方主图同一套逻辑
                （灰显高亮/彩斑/悬停/闪烁，只是没连线），当前编辑键高亮、点键 = 换编辑键，
                与虚拟键盘联动。 */}
            <MapCenter
              hasImage={hasImage}
              keys={keys}
              edit
              sel={keyId}
              hover={mirror.hover}
              live={mirror.live}
              held={mirror.held}
              after={mirror.after}
              onSelect={onKeyChange}
              onHover={mirror.onHover}
              regHot={(id, el) => {
                if (el) hotRefs.current.set(id, el);
                else hotRefs.current.delete(id);
              }}
              onMeasure={() => setImgTick((v) => v + 1)}
              // 原型 ui_03：图高 clamp(440px, 60vh, 600px)、宽按比例（列宽随图）。
              imgClassName="h-[clamp(440px,60vh,600px)]"
            />
          </div>
        </div>
    </div>
  );
}

/**
 * 内嵌面板接线（Mapping 画布下方）：fnEdit（键 id + 槽）→ 查键定义与现值、
 * 拼撞车判据，组装 FnPicker。Mapping 只留状态与提交口，布局细节都在这边。
 */
export function FnEditPanel({
  fnEdit,
  config,
  conflictAt,
  onCommit,
  onSlotChange,
  onClear,
  onKeyChange,
  onClose,
  onCollapse,
  mirror,
  hasImage,
  keys,
}: {
  fnEdit: { key: string; slot: SlotKey };
  config: Record<string, KeyConfig>;
  conflictAt: (sc: Shortcut, except: string) => string | null;
  onCommit: (slot: SlotKey, action: Action) => void;
  onSlotChange: (s: SlotKey) => void;
  onClear: (s: SlotKey) => void;
  onKeyChange: (id: string) => void;
  onClose: () => void;
  onCollapse: () => void;
  mirror: RemoteMirror;
  /** 档案有无实物图（队列 22）：无图档案副本走键名网格，不套小米图。 */
  hasImage: boolean;
  keys: KeyDef[];
}) {
  const k = KEYS.find((x) => x.id === fnEdit.key);
  if (!k) return null;
  const kc = config[k.id] ?? DEFAULT_KEY;
  return (
    <FnPicker
      slot={fnEdit.slot}
      onSlotChange={onSlotChange}
      onClear={onClear}
      values={{ click: kc.click, double: kc.double, long: kc.long }}
      init={kc[fnEdit.slot]}
      conflict={(sc, s) => conflictAt(sc, `${k.id}:${s}`) !== null}
      keyId={k.id}
      onKeyChange={onKeyChange}
      mirror={mirror}
      hasImage={hasImage}
      keys={keys}
      keyName={t(k.nameKey)}
      onCommit={onCommit}
      onClose={onClose}
      onCollapse={onCollapse}
    />
  );
}
