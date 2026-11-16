import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { POLL, setPollPaused, usePollPaused } from "../lib/poll";

/**
 * 数据新鲜度控件（计划 §6.3）：一眼看出屏上的数据有多新，并且能叫停刷新。
 *
 * 「最后更新」取的是整个 QueryCache 里最近一次成功写入的时刻，而不是某一页自己的时间戳 ——
 * 顶栏是全窗共用的，跟着当前页换控件会让位置跳动。
 *
 * 暂停的是轮询节奏（lib/poll 的开关），不是请求本身：手动重试、切页首屏照样能拉。
 * 恢复时补一次全量 refetch，把暂停期间停掉的定时器重新点着，顺带把落下的真值补回来。
 */

/** 订阅 QueryCache，返回最近一次任意查询成功写入的时刻（毫秒）。 */
function useLastUpdated(): number {
  const qc = useQueryClient();
  const [ts, setTs] = useState(() => latest(qc));
  useEffect(() => {
    // 查询状态一变就重算：只关心 dataUpdatedAt 变新的那些事件，避免无谓重渲染。
    let last = latest(qc);
    const unsub = qc.getQueryCache().subscribe(() => {
      const next = latest(qc);
      if (next !== last) {
        last = next;
        setTs(next);
      }
    });
    return unsub;
  }, [qc]);
  return ts;
}

function latest(qc: ReturnType<typeof useQueryClient>): number {
  let max = 0;
  for (const q of qc.getQueryCache().getAll()) {
    if (q.state.dataUpdatedAt > max) max = q.state.dataUpdatedAt;
  }
  return max;
}

function ago(ms: number, now: number): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 5) return "刚刚";
  if (s < 60) return `${s} 秒前`;
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  return `${Math.floor(s / 3600)} 小时前`;
}

export function Freshness() {
  const qc = useQueryClient();
  const ts = useLastUpdated();
  const [now, setNow] = useState(() => Date.now());
  const paused = usePollPaused();

  // 只用来把「N 秒前」往前推，节奏比最慢的轮询还慢一档就够读了。
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), POLL.tail);
    return () => window.clearInterval(id);
  }, []);

  const toggle = () => {
    setPollPaused(!paused);
    // 恢复：定时器已经不在排班了，靠一次主动 refetch 重新点着，顺带补回暂停期间的变化。
    if (paused) qc.refetchQueries({ type: "active" });
  };

  if (!ts) return null;
  return (
    <button
      onClick={toggle}
      aria-pressed={paused}
      title={paused ? "恢复自动刷新" : "暂停自动刷新（停在当前读数上核对）"}
      className="flex h-8 items-center gap-1.5 rounded-md border border-slate-700 bg-slate-800/60 px-2 text-2xs text-ink-3 transition-colors hover:border-cyan-500/50 hover:text-ink-2"
    >
      {/* 暂停时灯灭成灰、恢复后跟着最新一次写入呼吸，状态一眼可辨 */}
      <span
        className={`inline-block h-1.5 w-1.5 rounded-full ${
          paused ? "bg-slate-500" : "bg-[var(--sw-on)]"
        }`}
      />
      {paused ? "刷新已暂停" : `更新于 ${ago(ts, now)}`}
    </button>
  );
}
