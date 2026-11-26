// 「谁在干活」的唯一出口：把在飞请求按后端/模型归堆，给各处的小圆点用。
//
// 为什么不各页自己拉：同一个 queryKey 在 react-query 里只会有一份轮询，
// 后端页、模型页、监控页共用也不会变成每秒三次 IPC。
import { useQuery } from "@tanstack/react-query";
import { ipc } from "./ipc";
import { every, POLL } from "./poll";

interface LiveRow {
  backend?: string | null;
  model?: string | null;
}

interface RatesLike {
  inflight?: LiveRow[];
}

/** 在飞请求的归属集合 + 总数。 */
export function useLive(refetchMs: number = POLL.live) {
  const { data } = useQuery<RatesLike>({
    queryKey: ["live"],
    queryFn: () => ipc<RatesLike>("get_rates"),
    refetchInterval: () => every(refetchMs),
    refetchOnWindowFocus: false,
  });
  const rows = data?.inflight ?? [];
  const byBackend = new Set(
    rows.map((r) => r.backend).filter((b): b is string => !!b),
  );
  const byModel = new Set(rows.map((r) => r.model).filter((m): m is string => !!m));
  return { byBackend, byModel, total: rows.length };
}
