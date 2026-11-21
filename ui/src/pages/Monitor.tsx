// 实时监控页：流量看板按后端组织——每个当前有流量的后端一张卡，
// 卡内每个有流量的模型一行（名称 + 吞吐 + 曲线），见 monitor/BackendTrafficCards。
// 「按模型」卡已被按后端分卡吸收；引擎原生速率并入 halogen 后端卡头部展示。
// 保留：上游 KV 遥测卡、页脚状态行、在飞流明细表（StreamsTable）。
import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card } from "../components/ui";
import { t, ts } from "../lib/i18n";
import { QueryError } from "../components/QueryError";
import { ipc } from "../lib/ipc";
import { every, POLL } from "../lib/poll";
import { BackendTrafficCards } from "./monitor/BackendTrafficCards";
import { StreamsTable } from "./monitor/StreamsTable";
import { fmtBytes, type InflightRow, type KvRow, type MetricsSnapshot } from "./monitor/model";


export function MonitorPage() {
  const [streams, setStreams] = useState<InflightRow[]>([]);
  const [metrics, setMetrics] = useState<MetricsSnapshot | null>(null);
  const [lastPush, setLastPush] = useState<number | null>(null);
  // 后端 KV 遥测（M5.5-D/E）：维护循环刷新的快照，这里只读展示。
  const kv = useQuery({
    queryKey: ["kv-telemetry"],
    queryFn: () => ipc<KvRow[]>("get_backend_telemetry"),
    refetchInterval: () => every(POLL.status),
  });

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let unlistenMetrics: (() => void) | undefined;
    let cancelled = false;
    import("@tauri-apps/api/event").then(({ listen }) =>
      listen<InflightRow[]>("monitor://streams", (ev) => {
        if (!cancelled) {
          setStreams(ev.payload);
          setLastPush(Date.now());
        }
      }),
    ).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    import("@tauri-apps/api/event").then(({ listen }) =>
      listen<MetricsSnapshot>("metrics://tick", (ev) => {
        if (!cancelled) setMetrics(ev.payload);
      }),
    ).then((fn) => {
      if (cancelled) fn();
      else unlistenMetrics = fn;
    });
    // 非 GUI 环境（纯浏览器调试）退回 2s 轮询 REST。
    if (!("__TAURI_INTERNALS__" in window)) {
      const poll = () =>
        fetch("/admin/metrics")
          .then((r) => r.json())
          .then((j) => !cancelled && setMetrics(j))
          .catch(() => {});
      poll();
      const t = setInterval(poll, 2000);
      return () => {
        cancelled = true;
        clearInterval(t);
      };
    }
    return () => {
      cancelled = true;
      unlisten?.();
      unlistenMetrics?.();
    };
  }, []);

  const totals = streams.reduce(
    (acc, s) => ({
      bytes: acc.bytes + s.bytes,
      events: acc.events + s.chunks,
    }),
    { bytes: 0, events: 0 },
  );

  return (
    <div className="space-y-4 pt-[var(--content-pad-top)]">
      {/* 按后端流量卡（切片 17）：数据 = get_backend_model_rates 每秒轮询，
          全部是网关直接观测（完成请求滑窗 + 在飞注册表），无估算。 */}
      <BackendTrafficCards />

      {/* kv 遥测失败时也别默默藏起来：与「缺端点 = 不展示」的空态区分开 */}
      {kv.isError && (
        <Card>
          <QueryError inline title={t("mon.kvError")} error={kv.error} onRetry={() => kv.refetch()} />
        </Card>
      )}
      {!kv.isError && (kv.data?.length ?? 0) > 0 && (
        <Card>
          <div className="mb-2 flex items-center justify-between">
            <h2 className="text-base font-semibold">{t("mon.kvTitle")}</h2>
            <span className="text-sm text-ink-4">{t("mon.kvDesc")}</span>
          </div>
          <div className="flex flex-wrap gap-2">
            {(kv.data ?? []).map((k) => (
              <span
                key={k.backend}
                className="flex items-center gap-2 rounded-md border border-slate-800 bg-slate-800/40 px-2.5 py-1.5 text-sm text-ink-2"
              >
                <span className="font-medium text-ink-2">{k.backend}</span>
                {k.engine && <span className="text-ink-4">{k.engine}</span>}
                {k.kv_usage != null && <span>KV {(k.kv_usage * 100).toFixed(0)}%</span>}
                {k.prefix_hit_rate != null && <span>{ts("mon.prefixHit", { n: (k.prefix_hit_rate * 100).toFixed(0) })}</span>}
                {k.waiting != null && k.waiting > 0 && <span className="text-[var(--sem-busy-text)]">{ts("mon.waiting", { n: k.waiting })}</span>}
                {k.has_ssd_kv && (
                  <span className="text-cyan-300">
                    SSD{k.ssd_hit_rate != null ? ` ${(k.ssd_hit_rate * 100).toFixed(0)}%` : ""}
                  </span>
                )}
              </span>
            ))}
          </div>
        </Card>
      )}

      <div className="flex items-center gap-4 text-sm text-ink-3">
        <span>{ts("mon.sumBytes", { v: fmtBytes(totals.bytes) })}</span>
        <span>{ts("mon.chunksN", { n: totals.events })}</span>
        {metrics?.cache && (
          <span className="text-ink-3">
            {ts("mon.cacheHit", { n: (metrics.cache.hit_rate * 100).toFixed(0) })} ·{" "}
            {ts("mon.savedInput", { n: metrics.cache.saved_input_tokens.toLocaleString() })}
          </span>
        )}
        <span className="ml-auto text-ink-4">
          {lastPush ? ts("mon.lastPush", { time: new Date(lastPush).toLocaleTimeString() }) : t("mon.waitingPush")}
        </span>
      </div>

      <StreamsTable streams={streams} />
    </div>
  );
}
