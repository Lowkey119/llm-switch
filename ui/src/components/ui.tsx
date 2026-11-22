import { forwardRef } from "react";
import * as Switch from "@radix-ui/react-switch";
import { t } from "../lib/i18n";

/**
 * 表单控件基样式（全站单一真相）：边框/底色/字号/焦点态一次定死。
 *
 * 基类刻意不带任何宽度类。Tailwind 产物里 `.w-full` 排在 `.w-64`/`.w-36` 之后，
 * 在控件上叠「基类 + w-64」会被基类的 w-full 顶掉（参数设置页「预设下拉各占
 * 一整行、把表单挤没」的根因）。要定宽就套一层带宽度的外层容器。
 */
// h-7 定高（用户 2026-11-21 拍板全站统一）：原生 select 与 input 默认行高不同，不锁高度就参差。
// 复用于 textarea 时请用 min-h-*（min-height 压过 height），别用 h-*。
export const fieldCls =
  "h-7 rounded-md border border-slate-700 bg-slate-800/60 px-2 py-1 text-sm text-ink-2 placeholder:text-ink-4 focus-visible:border-cyan-500/60";

/** 撑满所在栅格/容器的输入框。 */
export const inputCls = `block w-full ${fieldCls}`;

/** 撑满所在栅格/容器的下拉框。 */
export const selectCls = `block w-full ${fieldCls}`;

/** 弹层/表单里的大一号输入：字号与 `fieldCls` 同档（都是正文/控件档 14），只把内边距松一档给对话框尺度。 */
export const inputLgCls =
  "block w-full rounded-md border border-slate-700 bg-slate-800/60 px-2.5 py-1.5 text-sm focus-visible:border-cyan-500/60";

/** 工具条上的紧凑下拉：底色比卡片深一档、不吃宽度，靠内容或外层定宽。 */
export const barSelectCls = "rounded-md border border-slate-700 bg-slate-900 px-2 py-1.5";

export function Toggle({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <Switch.Root
      checked={checked}
      onCheckedChange={onChange}
      disabled={disabled}
      className="group flex h-5 w-9 shrink-0 items-center rounded-full border px-0.5 transition-colors disabled:opacity-40 data-[state=checked]:border-[var(--sw-on)] data-[state=checked]:bg-[var(--sw-on)] data-[state=unchecked]:border-[var(--sw-off)] data-[state=unchecked]:bg-[var(--sw-off)]"
    >
      <Switch.Thumb className="h-3.5 w-3.5 rounded-full bg-slate-100 transition-transform data-[state=checked]:translate-x-4" />
    </Switch.Root>
  );
}

/**
 * 行内图标动作（UI 重构版式）：平时隐形，所在行（group/row）悬停才显形，
 * 自身悬停才有底色。危险动作用 tone="danger"，只在自身悬停时透红。
 */
export function IconButton({
  children,
  title,
  onClick,
  tone = "neutral",
  disabled,
}: {
  children: React.ReactNode;
  title: string;
  onClick?: () => void;
  tone?: "neutral" | "danger";
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      className={`grid h-7 w-7 place-items-center rounded-md text-ink-3 transition-colors hover:bg-slate-700/60 group-hover/row:text-ink-2 disabled:cursor-not-allowed disabled:text-ink-4 disabled:hover:bg-transparent ${
        tone === "danger" ? "hover:text-[var(--sem-danger-text)]" : "hover:text-ink-1"
      }`}
    >
      {children}
    </button>
  );
}

/** 纯文字主操作（如「+ 增加后端」）：无框无底，hover 变亮。 */
export function TextButton({
  children,
  onClick,
}: {
  children: React.ReactNode;
  onClick?: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="text-sm font-medium text-cyan-300 transition-colors hover:text-cyan-200"
    >
      {children}
    </button>
  );
}

export function Badge({ children }: { children: React.ReactNode }) {
  return (
    <span className="rounded border border-slate-700 bg-slate-800/70 px-1.5 py-0.5 text-2xs text-ink-2">
      {children}
    </span>
  );
}

/**
 * 状态灯（UI-AUDIT P1-6）：颜色之外再叠「形状 + 读屏文本」两路冗余。
 *
 * 只靠颜色的状态灯对色觉障碍和灰度截图等于没有——四态各给一个形状：
 * ● 健康 / ▲ 降级 / ✕ 宕机 / ◐ 在飞（呼吸），读屏另有 sr-only 文案。
 *
 * 呼吸只留给「此刻真有请求在飞」：一直闪的灯等于没有灯，眼睛会自动把它
 * 过滤掉。宕机不参与呼吸——红常亮。
 */
export function StatusDot({
  state,
  busy,
  text,
}: {
  state?: string;
  busy?: boolean;
  /** 覆盖默认文案（形状仍按状态走）：形状对、默认说法不对的场合用，如「实时更新中」。 */
  text?: string;
}) {
  const tone =
    state === "down"
      ? { glyph: "\u2715", cls: "text-[var(--sw-off)]", text: t("dot.down") }
      : busy
        ? { glyph: "\u25D0", cls: "live-breathe text-[var(--sw-live)]", text: t("dot.busy") }
        : state === "degraded"
          ? { glyph: "\u25B2", cls: "text-[var(--sw-live)]", text: t("dot.degraded") }
          : state === "healthy"
            ? { glyph: "\u25CF", cls: "text-[var(--sw-on)]", text: t("dot.healthy") }
            : { glyph: "\u25CB", cls: "text-ink-4", text: t("dot.unknown") };
  return (
    <span
      className="inline-flex h-3 w-3 shrink-0 items-center justify-center"
      title={text ?? tone.text}
    >
      <span aria-hidden className={`text-2xs leading-none ${tone.cls}`}>
        {tone.glyph}
      </span>
      <span className="sr-only">{text ?? tone.text}</span>
    </span>
  );
}

export function Card({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div
      className={`rounded-lg border border-slate-800 bg-slate-900/60 p-4${className ? ` ${className}` : ""}`}
    >
      {children}
    </div>
  );
}

/** 卡头：小标题 + 右侧动作区（按钮/徽标按 Button(sm)/Badge 口径传入）。 */
export function CardHead({
  icon: Icon,
  title,
  children,
}: {
  icon?: React.ComponentType<{ className?: string }>;
  /** 标题：纯文本或可交互节点（如映射页的「设置态」开关）。 */
  title: React.ReactNode;
  /** 右侧动作区：按钮/徽标按 Button(sm)/Badge 口径传入。 */
  children?: React.ReactNode;
}) {
  return (
    <div className="mb-3 flex flex-wrap items-center gap-2">
      {Icon && <Icon className="shrink-0 text-ink-4" />}
      <h3 className="text-xs font-semibold uppercase tracking-wider text-ink-3">{title}</h3>
      {children && <div className="ml-auto flex flex-wrap items-center gap-2">{children}</div>}
    </div>
  );
}

/**
 * 设置行：设置页「左文右控」卡片的唯一形态——左标题+说明（min-w-0
 * 让长说明折行不顶开右侧），右控件簇 shrink-0（状态文字不被挤成竖排）。可选 children
 * 为卡内下沿的补充行（自动带上分隔线）。开关类卡片复用，改一处全站生效。
 */
export function SettingRow({
  title,
  description,
  status,
  control,
  children,
}: {
  title: string;
  description?: React.ReactNode;
  status?: React.ReactNode;
  control: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <Card>
      <div className="flex items-center gap-4">
        <div className="min-w-0 flex-1">
          <h2 className="text-base font-semibold">{title}</h2>
          {description && <p className="mt-1 text-sm leading-relaxed text-ink-4">{description}</p>}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {status}
          {control}
        </div>
      </div>
      {children && <div className="mt-3 border-t border-slate-800 pt-3">{children}</div>}
    </Card>
  );
}

/** 字段标签：label 包控件，hint 走 title 悬浮提示。 */
export function FieldLabel({
  label,
  hint,
  children,
  className,
  row = false,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
  className?: string;
  /** 一行式（用户 2026-11-21 拍板）：标签在左、控件在右；缺省仍是标签在上。 */
  row?: boolean;
}) {
  return (
    <label
      className={`${row ? "flex min-w-0 flex-row items-center gap-2" : "flex min-w-0 flex-col gap-1"} ${className ?? ""}`}
      title={hint}
    >
      <span className={`text-ink-3 ${row ? "min-w-[4.5em] shrink-0" : ""}`}>{label}</span>
      {children}
    </label>
  );
}

/**
 * 骨架行（UI 审查 P1-5）：加载期占住行高，数据到位不整页跳动。
 * 呼吸动画已被全局 prefers-reduced-motion 降级接管，这里不用另写。
 */
export function Skeleton({ className }: { className?: string }) {
  return <div aria-hidden className={`animate-pulse rounded bg-slate-800/70${className ? ` ${className}` : ""}`} />;
}

/** 列表页加载骨架：n 行整行灰条，套在 slab（divide-y 浅底块）里用。 */
export function SkeletonRows({ rows = 3, className }: { rows?: number; className?: string }) {
  return (
    <div aria-busy="true" className={className}>
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className="flex items-center gap-3 px-4 py-3.5">
          <Skeleton className="h-3.5 w-32" />
          <Skeleton className="h-3.5 w-48" />
          <Skeleton className="ml-auto h-3.5 w-16" />
        </div>
      ))}
    </div>
  );
}

/**
 * 全站唯一按钮（UI-AUDIT P1-1）。
 *
 * 样式口径：一律「薄底 + 同族文字」，不用实心彩底配白字——那套在浅色主题上
 * 对比度直接不达标（见 UI-ALIGNMENT §7）。danger 只用于不可逆动作，
 * 且必须配 ConfirmDialog（闸门矩阵 §4）。
 * type 固定 button：这些按钮大量出现在表单里，默认的 submit 会误触发表单提交。
 */
export const Button = forwardRef(function Button(
  {
    children,
    onClick,
    variant = "ghost",
    size = "sm",
    disabled,
    title,
    className,
  }: {
    children: React.ReactNode;
    onClick?: () => void;
    variant?: "primary" | "ghost" | "danger";
    /** 尺寸档只管**疏密**，字号按 U1b 口径统一：xs = 密集行内动作（11px，一行两钮要抢位置的节）；
     *  sm = 默认（工具条/行内，14px 紧边距）；md = 对话框主操作（14px 松边距）。 */
    size?: "xs" | "sm" | "md";
    disabled?: boolean;
    title?: string;
    className?: string;
  },
  ref: React.Ref<HTMLButtonElement>,
) {
  // 禁用态统一压平（审查 ag 2.4）：primary 的 cyan 淡底在暖白主题下近乎浅灰，
  // 与禁用只换字色几乎无差——禁用一律灰底灰字且不带 hover，一眼可辨。
  const tone = disabled
    ? "border-slate-700 bg-slate-800/40 text-ink-4"
    : variant === "primary"
      ? "border-cyan-500/50 bg-cyan-500/15 text-cyan-200 hover:bg-cyan-500/25"
      : variant === "danger"
        ? "border-rose-500/40 bg-rose-500/10 text-[var(--sem-danger-text)] hover:bg-rose-500/20"
        : "border-slate-700 bg-slate-800/60 text-ink-2 hover:bg-slate-700/60";
  return (
    <button
      ref={ref}
      type="button"
      onClick={onClick}
      disabled={disabled}
      title={title}
      className={`rounded-md border transition-colors disabled:cursor-not-allowed disabled:text-ink-4 ${
        size === "md"
          ? "px-3.5 py-1.5 text-sm"
          : size === "xs"
            ? "px-2 py-0.5 text-2xs"
            : "px-2.5 py-1 text-sm"
      } ${tone}${className ? ` ${className}` : ""}`}
    >
      {children}
    </button>
  );
});
