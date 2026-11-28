import type { ComponentType, ReactNode, SVGProps } from "react";
import { IconInfo } from "./icons";

/**
 * 三态统一呈现（ui_01 原型第 5 条回灌，U5）。
 *
 * 空态此前在八处各写一遍——`px-4 py-6 text-xs`、`py-12 text-center text-sm`、
 * 裸 `text-xs`、`px-3 py-6 text-center`——同一个状态五种长相，而且都只有一句话：
 * 「暂无数据」四个字把用户打发走，既不区分「真没有」和「被筛没了」，也不给下一步。
 * 这里收成两个形状，与既有的错误态、骨架共用同一套留白与居中，切换时不跳版：
 *
 *   EmptyState —— 卡片 / 页面级：图标 + 标题 + 一句说明 + 可选动作（原型 `.state`）
 *   EmptyCell  —— 表格里的紧凑版（原型 `.empty-cell`），只有一行，不塞图标
 *
 * 错误态在 `components/QueryError.tsx`（「读取失败 ≠ 空态」是 UI-AUDIT P0-4 的底线，
 * 不并到这里来）；骨架在 `components/ui.tsx` 的 `SkeletonRows`。
 *
 * 字号说明：原型 `.state h4` 用 `--fs-base`(14) → 我们的 `text-sm`；说明行原型用
 * `--fs-sm`(13)，本仓没有 13 这一档——U1b 分流时按「整句正文 = `text-sm`」并进了 14，
 * 没有为它单开一个 13px 死令牌（全站只剩这一处要它，建了就是没人吃的档）。
 */

/** 空态图标默认用信息符；调用方按域传（后端页传 IconBackend、筛空传 IconSearch…）。 */
type IconType = ComponentType<SVGProps<SVGSVGElement>>;

export function EmptyState({
  title,
  desc,
  icon: Icon = IconInfo,
  action,
  className = "",
}: {
  /** 说清「这里为什么是空的」，不要只写「暂无数据」。 */
  title: ReactNode;
  /** 下一步做什么。可省，但省之前先想清楚用户是否知道该干嘛。 */
  desc?: ReactNode;
  icon?: IconType;
  /** 一个动作按钮（调用方自带 Button 与 onClick）；空态最多一个动作，多了就是没想清楚。 */
  action?: ReactNode;
  className?: string;
}) {
  return (
    <div className={`flex flex-col items-center gap-2 px-4 py-10 text-center ${className}`}>
      <Icon width={26} height={26} className="shrink-0 text-ink-4" aria-hidden="true" />
      {/* 卡头是 h2，空态是卡内内容，取 h3；
          写死 h4 会跳过一级，impeccable 报 skipped-heading。 */}
      <h3 className="text-sm font-semibold text-ink-2">{title}</h3>
      {desc && <p className="max-w-[46ch] text-sm leading-relaxed text-ink-4">{desc}</p>}
      {action}
    </div>
  );
}

/**
 * 表格空态（原型 `.empty-cell`）：整行合并居中一行字。
 * 表格里塞图标块会把一行撑成卡片，读起来像出了错，所以这里刻意不给图标。
 */
export function EmptyCell({ cols, children }: { cols: number; children: ReactNode }) {
  return (
    <tr>
      <td colSpan={cols} className="px-4 py-10 text-center text-sm text-ink-4">
        {children}
      </td>
    </tr>
  );
}
