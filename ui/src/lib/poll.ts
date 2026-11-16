// 轮询节奏表 + 自动刷新暂停开关（UI-OPTIMIZATION-PLAN §6.2 / §6.3）。
import { useSyncExternalStore } from "react";

export const POLL = {
  /** 在飞/实时：监控流、在飞集合、展开行的模型探活。 */
  live: 1000,
  /** 跟随式尾读：日志 tail、halogen 日志。 */
  tail: 2000,
  /** 快档：状态灯类（halogen 状态、缓存命中）。 */
  fast: 3000,
  /** 内核状态与浮窗可见性：变化不频繁，但用户会盯着顶栏胶囊。 */
  status: 5000,
  /** 慢档：清单类（矩阵、监控汇总）。 */
  normal: 10000,
  /** 更慢：历史与统计（日志、用量）。 */
  slow: 30000,
  /** 几乎不动：版本清单、保留策略。 */
  idle: 60000,
} as const;

// 此前 19 处 refetchInterval 各写各的毫秒数，没人说得清「为什么这里是 5 秒那里是 3 秒」，
// 想整体降频只能一处个数。档位收成上面的命名常量：加新查询时从表里挑一档，
// 别再往代码里塞裸数字。

let paused = false;
const subs = new Set<() => void>();

/**
 * 暂停/恢复自动刷新。
 *
 * 为什么开关做在这一层而不是 react-query 上：v5 把定时器暂停收进了内部（给
 * networkMode 离线用），QueryCache 上没有公开的 pause/resume，外部唯一能让
 * 「所有轮询都不再排下一次」的入口就是 refetchInterval 自己返回 false。
 * 于是所有查询写成 `refetchInterval: () => every(POLL.x)`，暂停时统一返回 false。
 *
 * 两个已知代价，都是刻意的：暂停那一刻已经排好的那一轮定时器仍会跑完一次才停；
 * 暂停期间切到新页面仍会拉一次首屏数据（那是导航带来的，不是后台轮询），
 * 恢复时由控件补一次 refetchQueries 把循环重新点着，见 components/Freshness.tsx。
 */
export function setPollPaused(v: boolean) {
  if (paused === v) return;
  paused = v;
  for (const fn of subs) fn();
}

export function isPollPaused(): boolean {
  return paused;
}

/** 交给 refetchInterval：暂停时返回 false，让定时器不再排下一次。 */
export function every(ms: number): number | false {
  return paused ? false : ms;
}

function subscribe(fn: () => void): () => void {
  subs.add(fn);
  return () => {
    subs.delete(fn);
  };
}

/** 暂停状态的 React 视图（顶栏控件用它决定灯色与文案）。 */
export function usePollPaused(): boolean {
  return useSyncExternalStore(subscribe, isPollPaused, isPollPaused);
}
