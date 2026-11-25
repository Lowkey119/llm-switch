// 键列与键行（原型 docs/ui_03/mapping.html 回灌，2026-11-21）：每列一个虚线框，
// 列头「单击/双击/长按」只出现一次，每键一行 = 键名格（92px，朝实物图一侧）+ 三个状态框。
// 录入交互（用户 2026-11-25 终稿）：点槽弹出动作列表直接绑——全部映射统一走动作绑定
// （快捷键/系统功能也都先做成动作再绑），槽位不再直录；悬停 ✕ 清除。
// 数据形状与 Rust actions.rs 对齐（serde snake_case）；冲突判定在 Mapping 父层
// （要看全量 config），这里只管展示与弹层交互。
import { useEffect, useState, type ReactNode } from "react";
import { Button } from "../../components/ui";
import { IconClose } from "../../components/icons";
import { t } from "../../lib/i18n";
import { SLOTS, type KeyDef, type SlotKey } from "./keys";
import { fmtAction, type Action } from "./action";
import { fmtMacro, useMacroLabels, type Macro } from "./macros";

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
 * 一个槽位（一行里的三段之一）：点槽弹出动作列表，挑一条即绑（用户 2026-11-25 拍板：
 * 全部映射统一走动作）；没有动作可选 → 提示先建动作，一键跳到动作序列页签。
 * 有绑定显示格式化读数 + 悬停 ✕ 清除。冲突时 flash=true 红闪且**不保存**（原绑定保留）。
 */
function SlotRow({
  label,
  value,
  tip,
  macros,
  disabled,
  flash,
  left,
  onCommit,
  onClear,
  onGoMacro,
}: {
  label: string;
  value: Action | null;
  /** 这一格定义的功能名（悬停浮出，替代原先摆在键列下方的功能芯片排，用户 2026-11-23 拍板）。 */
  tip?: string;
  /** 动作清单：弹层里挑一条直接绑到本槽。 */
  macros: Macro[];
  disabled: boolean;
  flash: boolean;
  /** 键在左列还是右列：弹层朝列内侧对齐，否则右列最右槽的弹层顶破窗口右沿。 */
  left: boolean;
  onCommit: (a: Action) => void;
  onClear: () => void;
  /** 无动作可挑时：跳到动作序列页签（用户 2026-11-25：先建动作再绑）。 */
  onGoMacro: () => void;
}) {
  const [open, setOpen] = useState(false);
  // 宏动作只显示 id 人看不懂，读数函数从宏库缓存里取名字（同一 query，多键共用一份）。
  const macroName = useMacroLabels();
  // Esc 收弹层（捕获阶段，先于页面级 Esc 退设置态）。
  useEffect(() => {
    if (!open) return;
    const h = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(false);
    };
    window.addEventListener("keydown", h, true);
    return () => window.removeEventListener("keydown", h, true);
  }, [open]);
  return (
    <div
      // tip 类只在有文案时挂：.tip::after 用 attr() 画气泡，没文案会浮一个空框。
      // @container：槽位字号按本格实测宽走（cqw），窗口/Cmd 缩放都跟手（用户 2026-11-24）。
      className={`group/slot relative min-w-0 select-none @container ${tip ? "tip" : ""}`}
      data-tip={tip}
    >
      <button
        type="button"
        aria-expanded={open}
        aria-label={`${label} ${value ? fmtAction(value, macroName) : t("remote.map.slotEmpty")}${tip ? `（${tip}）` : ""}`}
        disabled={disabled}
        onClick={(e) => {
          e.stopPropagation();
          setOpen((v) => !v);
        }}
        className={`min-h-8 w-full overflow-hidden whitespace-nowrap rounded-lg border px-2 py-1 text-center text-[clamp(10px,9.5cqw,14px)] font-bold leading-snug transition-colors ${
          value ? "pr-6" : ""
        } ${
          flash
            ? "slot-conflict border-[var(--sem-danger)] text-[var(--sem-danger-text)]"
            : open
              ? "border-[var(--key-hl)] text-[var(--key-hl)]"
              : value
                ? "border-slate-600 text-[var(--sem-ok-text)] hover:border-slate-400"
                : "border-slate-700 font-normal text-ink-4 hover:border-slate-500"
        } disabled:border-slate-800`}
      >
        {value ? fmtAction(value, macroName) : t("remote.map.slotEmpty")}
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => setOpen(false)} />
          <div
            className={`absolute top-full z-50 mt-1.5 w-[300px] max-w-[80vw] rounded-lg border border-slate-600 bg-slate-900 p-1.5 shadow-2xl ${left ? "left-0" : "right-0"}`}
          >
            {macros.length === 0 ? (
              <div className="p-2">
                <p className="mb-2 text-sm leading-relaxed text-ink-4">{t("remote.map.noMacrosYet")}</p>
                <Button
                  size="sm"
                  onClick={() => {
                    setOpen(false);
                    onGoMacro();
                  }}
                >
                  {t("remote.map.goCreate")}
                </Button>
              </div>
            ) : (
              <ul className="max-h-44 overflow-y-auto">
                {macros.map((m) => (
                  <li key={m.id}>
                    <button
                      type="button"
                      onClick={() => {
                        onCommit({ type: "macro", id: m.id });
                        setOpen(false);
                      }}
                      className="flex w-full items-baseline gap-2 border-b border-slate-800 px-2.5 py-1.5 text-left last:border-b-0 hover:bg-[var(--key-hl)]/10"
                    >
                      <span className="shrink-0 text-sm text-ink-1">{m.name}</span>
                      <span className="min-w-0 flex-1 truncate text-2xs text-ink-3">{fmtMacro(m, macroName)}</span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </>
      )}
      {value !== null && !disabled && !open ? (
        <button
          type="button"
          data-tip={t("remote.map.slotClear")}
          aria-label={t("remote.map.slotClear")}
          onClick={(e) => {
            e.stopPropagation();
            onClear();
          }}
          className="invisible absolute right-0.5 top-1/2 grid h-5 w-5 -translate-y-1/2 shrink-0 place-items-center rounded text-ink-4 hover:bg-slate-800 hover:text-ink-1 group-hover/slot:visible"
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
  onGoMacro,
  setRef,
  onHover,
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
  /** 动作清单：槽位弹层里挑一条直接绑到槽。 */
  macros: Macro[];
  /** 无动作可挑时跳动作序列页签。 */
  onGoMacro: () => void;
  setRef: (el: HTMLDivElement | null) => void;
  /** 悬停上报（父层据此画行↔热点连线，用户 2026-11-20：要连线呀）。 */
  onHover?: (id: string | null) => void;
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
      label={t(s.labelKey)}
      value={kc[s.id]}
      tip={fnTipOf?.(kc[s.id])}
      macros={macros}
      disabled={!edit}
      flash={flash === `${k.id}:${s.id}`}
      left={left}
      onCommit={(a) => onSlot(k.id, s.id, a)}
      onClear={() => onSlot(k.id, s.id, null)}
      onGoMacro={onGoMacro}
    />
  ));
  return (
    <div
      ref={setRef}
      onClick={() => onSelect(k.id)}
      onMouseEnter={() => onHover?.(k.id)}
      onMouseLeave={() => onHover?.(null)}
      className={`cursor-pointer hover:bg-[var(--key-hl)]/10 ${gridCls(left)} items-center gap-x-2.5 border-b border-slate-700 py-1 transition-colors last:border-b-0 ${
        // 选中用有色框、悬停用淡底（用户 2026-11-25）：两者都是淡底就分不出来了。
        live ? "bg-[var(--key-hl)]/25" : selected ? "ring-1 ring-inset ring-[var(--key-hl)]" : ""
      }`}
    >
      {!left && name}
      {slots}
      {left && name}
    </div>
  );
}
