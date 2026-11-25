// 键列与键行（原型 docs/ui_03/mapping.html 回灌，2026-11-21）：每列一个虚线框，
// 列头「单击/双击/长按」只出现一次，每键一行 = 键名格（92px，朝实物图一侧）+ 三个状态框。
// 录入交互不变：聚焦直接敲组合、双击进录制、录制中「功能」钮开面板、悬停 ✕ 清除。
// 数据形状与 Rust actions.rs 对齐（serde snake_case）；冲突判定在 Mapping 父层
// （要看全量 config），这里只管展示与录入交互。功能编辑面板内嵌在 Mapping 画布下方（FnPicker.tsx）。
import { useRef, useState, type ReactNode } from "react";
import { FN_CATEGORIES } from "./FnPicker";
import { IconClose } from "../../components/icons";
import { t } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";
import { SLOTS, type KeyDef, type SlotKey } from "./keys";
import { asShortcut, fmtAction, type Action } from "./action";
import { fmtMacro, type Macro } from "./macros";
import { useMacroLabels } from "./macros";
import { eventToShortcut } from "./shortcut";

/** 与后端 KeyConfig 对齐：缺键 = 默认（启用、三槽未绑=原生、不连按）。
 *  C2 起槽值是动作（快捷键只是其中一种），旧配置由后端读入时迁移。 */
export interface KeyConfig {
  enabled: boolean;
  click: Action | null;
  double: Action | null;
  long: Action | null;
  rapid: boolean;
}

export const DEFAULT_KEY: KeyConfig = { enabled: true, click: null, double: null, long: null, rapid: false };

/** 列网格口径：左列键名在行尾（靠实物图）、右列在行首；92px 名格 + 三等宽状态框。 */
const gridCls = (left: boolean) =>
  left ? "grid grid-cols-[repeat(3,minmax(0,1fr))_92px]" : "grid grid-cols-[92px_repeat(3,minmax(0,1fr))]";

/** 键列（原型 keycol）：虚线框 + 列头（单击/双击/长按只出现一次）+ 每键一行。
 *  顶对齐由 Mapping 的列包装层 self-start 负责（贴遥控器顶不越界，用户 2026-11-21）。 */
export function KeyColumn({ side, children }: { side: "l" | "r"; children: ReactNode }) {
  const left = side === "l";
  const labels = SLOTS.map((s) => t(s.labelKey));
  // 表头格与行格同序：左列 = 三槽头 + 空名格；右列 = 空名格 + 三槽头。
  const cells = left ? [...labels, ""] : ["", ...labels];
  return (
    <div className="relative z-10 rounded-[10px] border border-dashed border-slate-600 px-3 py-1">
      <div className={`${gridCls(left)} items-center gap-x-2.5 border-b border-slate-700 pb-1`}>
        {cells.map((c, i) => (
          <span key={i} className="text-center text-2xs text-ink-3">
            {c}
          </span>
        ))}
      </div>
      {children}
    </div>
  );
}

/**
 * 一个槽位（一行里的三段之一）：聚焦直接敲组合；双击槽位进录制态；
 * 有绑定显示 ⌃⌥⇧⌘ 格式化 + 悬停 ✕ 清除。冲突时 flash=true 红闪且**不保存**（原绑定保留）。
 * 槽名不再逐槽显示——列头出现一次（原型口径）。
 */
function SlotRow({
  slotId,
  label,
  value,
  tip,
  macros,
  disabled,
  flash,
  left,
  onCommit,
  onClear,
  reg,
  onFocusSlot,
  onFn,
}: {
  slotId: string;
  label: string;
  value: Action | null;
  /** 这一格定义的功能名（悬停浮出，替代原先摆在键列下方的功能芯片排，用户 2026-11-23 拍板）。 */
  tip?: string;
  disabled: boolean;
  flash: boolean;
  /** 键在左列还是右列：录制浮层朝列内侧对齐（左列 left-0 / 右列 right-0），
   *  否则右列最右槽的浮层会顶破窗口右沿（用户 2026-11-24 缩放验证抓）。 */
  left: boolean;
  onCommit: (a: Action) => void;
  onClear: () => void;
  reg: (el: HTMLButtonElement | null) => void;
  /** 聚焦槽上报父层记账：captured 事件据此提交，不赌 document.activeElement（WKWebView 不稳）。 */
  onFocusSlot?: (slotId: string | null) => void;
  /** 打开功能编辑面板（面板内嵌在 Mapping 画布下方，见 FnPicker）。 */
  onFn: () => void;
  /** 宏清单（录制浮层里挑一条直接绑到本槽）。 */
  macros?: Macro[];
}) {
  const [rec, setRec] = useState(false);
  // 录制浮层的挑选页签（用户 2026-11-24 定稿）：三条绑定路——
  // ① 物理键直录（槽位 keydown / 后端 captured 事件，本组件不变）；
  // ② 从「我的动作」清单挑一条宏；③ 从「系统功能」挑一条内置组合。
  const [pickTab, setPickTab] = useState<"mine" | "sys">("mine");
  // 宏动作只显示 id 人看不懂，读数函数从宏库缓存里取名字（同一 query，多键共用一份）。
  const macroName = useMacroLabels();
  const btnRef = useRef<HTMLButtonElement | null>(null);
  return (
    // 单击热区（用户 2026-11-20 终稿）：点槽即聚焦开录，**再点同一槽 = 退出录制**
    // （不与双击/键盘打架；Esc 留给映射，取消就再点一下或点标题退出设置态）。
    // WKWebView 点按钮不给焦点（Safari 内核口径），所以手动 focus——
    // 焦点不在槽上，按键事件根本到不了 onKeyDown。
    <div
      // tip 类只在有文案时挂：.tip::after 用 attr() 画气泡，没文案会浮一个空框。
      // @container：槽位字号按本格实测宽走（cqw），窗口/Cmd 缩放都跟手（用户 2026-11-24：
      // 缩放禁换行、缩字号）。
      className={`group/slot relative min-w-0 select-none @container ${tip ? "tip" : ""}`}
      data-tip={tip}
      onDoubleClick={() => {
        if (disabled) return;
        btnRef.current?.focus();
      }}
    >
      <button
        type="button"
        data-slot={slotId}
        ref={(el) => {
          btnRef.current = el;
          reg(el);
        }}
        disabled={disabled}
        aria-label={`${label} ${value ? fmtAction(value, macroName) : t("remote.map.slotEmpty")}${tip ? `（${tip}）` : ""}`}
        onClick={(e) => {
          e.stopPropagation();
          const btn = e.currentTarget;
          // 再点正在录制的槽 = 退出录制（blur 统一关闸复位）。
          if (document.activeElement === btn) btn.blur();
          else btn.focus();
        }}
        // 聚焦即开录制闸门：后端 tap 拦下全部键盘输入（含遥控按键）转交本槽，
        // 不透传不触发别的响应（用户 2026-11-20 拍板）；失焦关闸。旧后端无此命令，catch 吞掉走 keydown 兜底。
        onFocus={() => {
          // 录制态以焦点为准（用户 2026-11-20：已有值的槽位进不了录制态）：
          // 聚焦即开闸 + 亮「录制中」，已有映射的槽位也一样——点槽按新组合即覆盖。
          setRec(true);
          onFocusSlot?.(slotId);
          void ipc("remote_recording_set", { active: true }).catch(() => {});
        }}
        onBlur={() => {
          setRec(false);
          onFocusSlot?.(null);
          void ipc("remote_recording_set", { active: false }).catch(() => {});
        }}
        onKeyDown={(e) => {
          // Esc 不设特判（用户 2026-11-20：Esc 要能录）：和别的键一样录进去；
          // 不想录了就点别处失焦关闸。
          e.preventDefault();
          const sc = eventToShortcut(e.nativeEvent);
          if (!sc) return; // 修饰键等：先按住不算组合，继续等
          e.stopPropagation();
          setRec(false);
          onCommit(asShortcut(sc));
          e.currentTarget.blur();
        }}
        className={`min-h-8 w-full overflow-hidden whitespace-nowrap rounded-lg border px-2 py-1 text-center text-[clamp(10px,9.5cqw,14px)] font-bold leading-snug transition-colors focus-visible:ring-1 focus-visible:ring-[var(--key-hl)] ${
          rec || value ? "pr-6" : ""
        } ${
          flash
            ? "slot-conflict border-[var(--sem-danger)] text-[var(--sem-danger-text)]"
            : rec
              ? "border-[var(--key-hl)] font-normal text-[var(--key-hl)]"
              : value
                ? "border-slate-600 text-[var(--sem-ok-text)] hover:border-slate-400"
                : "border-slate-700 font-normal text-ink-4 hover:border-slate-500"
        } disabled:border-slate-800`}
      >
        {rec ? t("remote.map.slotRecording") : value ? fmtAction(value, macroName) : t("remote.map.slotEmpty")}
      </button>
      {/* 录制态「功能」入口（用户 2026-11-20 拍板，参照 SayAll）：mousedown preventDefault
          保住槽位焦点——闸门不关、录制态不丢，面板里选一条即填入。录制中不显 ✕：
          值马上被覆盖，两个钮挤一个小框反而误触。 */}
      {rec ? (
        <div
          // 录制态浮层（用户 2026-11-24 定稿）：三条绑定路同层并存——
          // ① 物理键直录：焦点在槽上，keydown / 后端 captured 事件照旧直接成组合；
          // ② 「我的动作」清单挑一条宏；③ 「系统功能」挑一条内置组合。
          // 虚拟键盘撤了（用户拍板）：拼组合要么真键盘，要么去下方动作库编辑器里拼。
          // mousedown 一律不吃：焦点必须留在槽上，否则闸门一关、这层自己就没了。
          onMouseDown={(e) => e.preventDefault()}
          className={`absolute top-full z-50 mt-1.5 w-[420px] max-w-[80vw] rounded-lg border border-slate-600 bg-slate-900 p-3 shadow-2xl ${left ? "left-0" : "right-0"}`}
        >
          <p className="mb-2 text-sm leading-relaxed text-ink-4">{t("remote.map.recHint")}</p>
          <div role="tablist" aria-label={t("remote.map.recHint")} className="mb-1.5 flex items-center gap-1.5">
            {(
              [
                ["mine", "remote.macro.mineTitle"],
                ["sys", "remote.macro.sysTitle"],
              ] as const
            ).map(([id, labelKey]) => (
              <button
                key={id}
                type="button"
                role="tab"
                aria-selected={pickTab === id}
                onClick={() => setPickTab(id)}
                className={`rounded-md px-2 py-1 text-sm transition-colors ${
                  pickTab === id
                    ? "bg-[var(--key-hl)]/15 text-[var(--key-hl)]"
                    : "text-ink-3 hover:bg-slate-800 hover:text-ink-1"
                }`}
              >
                {t(labelKey)}
              </button>
            ))}
          </div>
          <div className="max-h-44 overflow-y-auto rounded-lg border border-slate-800">
            {pickTab === "mine" ? (
              !macros || macros.length === 0 ? (
                <p className="px-2.5 py-2 text-sm text-ink-4">{t("remote.macro.empty")}</p>
              ) : (
                macros.map((m) => (
                  <button
                    key={m.id}
                    type="button"
                    onClick={() => {
                      onCommit({ type: "macro", id: m.id });
                      btnRef.current?.blur();
                    }}
                    className="flex w-full items-baseline gap-2 border-b border-slate-800 px-2.5 py-1.5 text-left last:border-b-0 hover:bg-[var(--key-hl)]/10"
                  >
                    <span className="shrink-0 text-sm text-ink-1">{m.name}</span>
                    <span className="min-w-0 flex-1 truncate text-2xs text-ink-3">{fmtMacro(m)}</span>
                  </button>
                ))
              )
            ) : (
              <div className="px-2.5 py-2">
                {FN_CATEGORIES.map((c) => (
                  <div key={c.nameKey} className="mb-2 last:mb-0">
                    <p className="mb-1 text-2xs text-ink-4">{t(c.nameKey)}</p>
                    <div className="flex flex-wrap gap-1.5">
                      {c.items.map((it) => (
                        <button
                          key={it.nameKey}
                          type="button"
                          onClick={() => {
                            onCommit(asShortcut(it.sc));
                            btnRef.current?.blur();
                          }}
                          className="rounded-md border border-slate-700 px-2 py-1 text-sm text-ink-1 transition-colors hover:border-[var(--key-hl)] hover:text-[var(--key-hl)]"
                        >
                          {t(it.nameKey)}
                        </button>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      ) : null}
      {rec ? (
        <button
          type="button"
          data-tip={t("remote.map.fn")}
          aria-label={t("remote.map.fn")}
          onMouseDown={(e) => {
            e.preventDefault();
            // 开面板前把键盘交出去：blur 槽位 → 后端录制闸门关闭，物理键不再被
            // 槽位直录，全部进面板虚拟键盘拼组合（用户 2026-11-20 草图）。
            btnRef.current?.blur();
            onFn();
          }}
          className="absolute right-1 top-1/2 h-5 -translate-y-1/2 shrink-0 rounded border border-slate-600 px-1 text-2xs font-normal text-ink-3 hover:border-slate-500 hover:text-ink-1"
        >
          {t("remote.map.fn")}
        </button>
      ) : value !== null && !disabled ? (
        <button
          type="button"
          data-tip={t("remote.map.slotClear")}
          aria-label={t("remote.map.slotClear")}
          onClick={(e) => {
            e.stopPropagation();
            onClear();
          }}
          onDoubleClick={(e) => e.stopPropagation()}
          className="invisible absolute right-0.5 top-1/2 grid h-5 w-5 -translate-y-1/2 shrink-0 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-ink-1 group-hover/slot:visible group-focus-within/slot:visible"
        >
          <IconClose className="h-3 w-3" />
        </button>
      ) : null}
    </div>
  );
}

/** 一行键（原型 krow）：键名格 + 三状态框；整行点选/悬停（连线、滚动定位都挂在行上）。 */
export function KeyCard({
  k,
  kc,
  selected,
  live,
  edit,
  flash,
  onSelect,
  onSlot,
  fnTipOf,
  macros,
  regSlot,
  setRef,
  onHover,
  onFocusSlot,
  onFn,
}: {
  k: KeyDef;
  kc: KeyConfig;
  selected: boolean;
  live: boolean;
  edit: boolean;
  flash: string | null;
  onSelect: (id: string) => void;
  onSlot: (id: string, slot: SlotKey, action: Action | null) => void;
  /** 槽动作 → 功能名读数（父层从功能芯片目录反查；对不上回 undefined）。 */
  fnTipOf?: (action: Action | null) => string | undefined;
  /** 宏清单（录制浮层里挑一条直接绑到槽）。 */
  macros?: Macro[];
  regSlot: (key: string, el: HTMLButtonElement | null) => void;
  setRef: (el: HTMLDivElement | null) => void;
  /** 悬停上报（父层据此画行↔热点连线，用户 2026-11-20：要连线呀）。 */
  onHover?: (id: string | null) => void;
  /** 聚焦槽上报（父层记账，captured 事件据此提交）。 */
  onFocusSlot?: (slotId: string | null) => void;
  /** 打开功能编辑面板（面板内嵌在 Mapping 画布下方，用户 2026-11-20 拍板不再弹层）。 */
  onFn: (slot: SlotKey) => void;
}) {
  const Icon = k.icon;
  const left = k.col === "l";
  // 键名格 = 选行的键盘入口（法条：禁 div 冒充按钮）：整行 onClick 留给鼠标 convenience，
  // 键盘用户 Tab 到这个钮 Enter/Space 选中。stopPropagation 防双触发——
  // 行 onClick 也在冒泡链上，不拦的话 toggleKey 会被点两次（选中又立刻取消）。
  // 运行态点选无效（toggleKey 直接 return），钮跟着 disabled：别给键盘一个按了没反应的焦点位。
  const name = (
    <button
      type="button"
      disabled={!edit}
      aria-pressed={selected}
      onClick={(e) => {
        e.stopPropagation();
        onSelect(k.id);
      }}
      className={`flex min-w-0 items-center gap-1.5 whitespace-nowrap rounded px-1 -mx-1 font-bold transition-colors hover:bg-slate-800 disabled:cursor-not-allowed disabled:hover:bg-transparent ${left ? "justify-end" : ""} ${
        selected || live ? "text-[var(--key-hl)]" : "text-[var(--sem-ok-text)]"
      }`}
    >
      <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden />
      <span className="truncate text-sm">{t(k.nameKey)}</span>
    </button>
  );
  const slots = SLOTS.map((s) => (
    <SlotRow
      key={s.id}
      slotId={`${k.id}:${s.id}`}
      label={t(s.labelKey)}
      value={kc[s.id]}
      tip={fnTipOf?.(kc[s.id])}
      macros={macros}
      disabled={!edit}
      flash={flash === `${k.id}:${s.id}`}
      left={left}
      onCommit={(sc) => onSlot(k.id, s.id, sc)}
      onClear={() => onSlot(k.id, s.id, null)}
      reg={(el) => regSlot(`${k.id}:${s.id}`, el)}
      onFocusSlot={onFocusSlot}
      onFn={() => onFn(s.id)}
    />
  ));
  return (
    <div
      ref={setRef}
      onClick={() => onSelect(k.id)}
      onMouseEnter={() => onHover?.(k.id)}
      onMouseLeave={() => onHover?.(null)}
      className={`cursor-pointer ${gridCls(left)} items-center gap-x-2.5 border-b border-slate-700 py-1 transition-colors last:border-b-0 ${
        live ? "bg-[var(--key-hl)]/25" : selected ? "bg-[var(--key-hl)]/10" : ""
      }`}
    >
      {!left && name}
      {slots}
      {left && name}
    </div>
  );
}
