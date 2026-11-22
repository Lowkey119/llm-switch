// 键列与键行（原型 docs/ui_03/mapping.html 回灌，2026-11-21）：每列一个虚线框，
// 列头「单击/双击/长按」只出现一次，每键一行 = 键名格（92px，朝实物图一侧）+ 三个状态框。
// 录入交互不变：聚焦直接敲组合、双击进录制、录制中「功能」钮开面板、悬停 ✕ 清除。
// 数据形状与 Rust actions.rs 对齐（serde snake_case）；冲突判定在 Mapping 父层
// （要看全量 config），这里只管展示与录入交互。功能编辑面板内嵌在 Mapping 画布下方（FnPicker.tsx）。
import { useRef, useState, type ReactNode } from "react";
import { IconClose } from "../../components/icons";
import { t } from "../../lib/i18n";
import { ipc } from "../../lib/ipc";
import { SLOTS, type KeyDef, type SlotKey } from "./keys";
import { eventToShortcut, fmtShortcut, type Shortcut } from "./shortcut";

/** 与后端 KeyConfig 对齐：缺键 = 默认（启用、三槽未绑=原生、不连按）。 */
export interface KeyConfig {
  enabled: boolean;
  click: Shortcut | null;
  double: Shortcut | null;
  long: Shortcut | null;
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
    <div className="relative z-10 rounded-[10px] border border-dashed border-slate-600 px-3.5 py-1.5">
      <div className={`${gridCls(left)} items-center gap-x-2.5 border-b border-slate-700 pb-1.5`}>
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
  disabled,
  flash,
  onCommit,
  onClear,
  reg,
  onFocusSlot,
  onFn,
}: {
  slotId: string;
  label: string;
  value: Shortcut | null;
  disabled: boolean;
  flash: boolean;
  onCommit: (sc: Shortcut) => void;
  onClear: () => void;
  reg: (el: HTMLButtonElement | null) => void;
  /** 聚焦槽上报父层记账：captured 事件据此提交，不赌 document.activeElement（WKWebView 不稳）。 */
  onFocusSlot?: (slotId: string | null) => void;
  /** 打开功能编辑面板（面板内嵌在 Mapping 画布下方，见 FnPicker）。 */
  onFn: () => void;
}) {
  const [rec, setRec] = useState(false);
  const btnRef = useRef<HTMLButtonElement | null>(null);
  return (
    // 单击热区（用户 2026-11-20 终稿）：点槽即聚焦开录，**再点同一槽 = 退出录制**
    // （不与双击/键盘打架；Esc 留给映射，取消就再点一下或点标题退出设置态）。
    // WKWebView 点按钮不给焦点（Safari 内核口径），所以手动 focus——
    // 焦点不在槽上，按键事件根本到不了 onKeyDown。
    <div
      className="group/slot relative min-w-0 select-none"
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
        aria-label={`${label} ${value ? fmtShortcut(value) : t("remote.map.slotEmpty")}`}
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
          onCommit(sc);
          e.currentTarget.blur();
        }}
        className={`h-[34px] w-full truncate rounded-lg border px-2 text-center text-sm font-bold tabular-nums transition-colors focus-visible:ring-1 focus-visible:ring-[var(--key-hl)] ${
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
        {rec ? t("remote.map.slotRecording") : value ? fmtShortcut(value) : t("remote.map.slotEmpty")}
      </button>
      {/* 录制态「功能」入口（用户 2026-11-20 拍板，参照 SayAll）：mousedown preventDefault
          保住槽位焦点——闸门不关、录制态不丢，面板里选一条即填入。录制中不显 ✕：
          值马上被覆盖，两个钮挤一个小框反而误触。 */}
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
  onSlot: (id: string, slot: SlotKey, sc: Shortcut | null) => void;
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
  const name = (
    <span
      className={`flex min-w-0 items-center gap-1.5 whitespace-nowrap font-bold ${left ? "justify-end" : ""} ${
        selected || live ? "text-[var(--key-hl)]" : "text-[var(--sem-ok-text)]"
      }`}
    >
      <Icon className="h-3.5 w-3.5 shrink-0" />
      <span className="truncate text-sm">{t(k.nameKey)}</span>
    </span>
  );
  const slots = SLOTS.map((s) => (
    <SlotRow
      key={s.id}
      slotId={`${k.id}:${s.id}`}
      label={t(s.labelKey)}
      value={kc[s.id]}
      disabled={!edit}
      flash={flash === `${k.id}:${s.id}`}
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
      className={`${gridCls(left)} items-center gap-x-2.5 border-b border-slate-700 py-2 transition-colors last:border-b-0 ${
        live ? "bg-[var(--key-hl)]/25" : selected ? "bg-[var(--key-hl)]/10" : ""
      }`}
    >
      {!left && name}
      {slots}
      {left && name}
    </div>
  );
}
