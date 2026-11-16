// 按后端流量卡（切片 17）：每个「当前有流量」的后端一张卡，卡内每个有流量的
// 模型一行——模型名 + 吞吐数据 + 吞吐曲线，有几个模型几行。取代原先的
// 「API 流量」总览卡与「按模型」卡（用户口径：流量看板按后端组织）。
// 排序：内置引擎在前（halogen），用户添加的后端在后（按名）。
import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card, StatusDot } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ipc, type BackendRow } from "../../lib/ipc";
import { backendLabel } from "../../lib/backend";
import { every, POLL } from "../../lib/poll";
import { Sparkline } from "../../components/charts";
import type { ModelRate } from "./model";

/** get_backend_model_rates 出口：ModelRate 全字段 + 后端归属（serde flatten）。 */
export interface BackendModelRate extends ModelRate {
  backend: string;
}

/** 内置引擎卡序：halogen → 其余用户后端。 */
function engineRank(engine: string | null): number {
  return engine === "halogen" ? 0 : 1;
}

interface BackendGroup {
  backend: string;
  engine: string | null;
  rows: BackendModelRate[];
  tps: number;
  rps: number;
  inflight: number;
  errsPerS: number;
}

export function BackendTrafficCards() {
  const rates = useQuery({
    queryKey: ["backend-model-rates"],
    queryFn: () => ipc<BackendModelRate[]>("get_backend_model_rates"),
    refetchInterval: () => every(POLL.live),
  });
  // 引擎归属用来排卡序（内置在前）；清单没回来前按纯名字序兜底。
  const backends = useQuery({
    queryKey: ["backends"],
    queryFn: () => ipc<BackendRow[]>("list_backends"),
    refetchInterval: () => every(POLL.normal),
  });

  const groups = useMemo<BackendGroup[]>(() => {
    const byBackend = new Map<string, BackendModelRate[]>();
    for (const r of rates.data ?? []) {
      // 「有流量」口径 = 最近窗口有完成请求或此刻有在飞流（后端算好的 active）。
      if (!r.active || !r.backend) continue;
      let list = byBackend.get(r.backend);
      if (!list) {
        list = [];
        byBackend.set(r.backend, list);
      }
      list.push(r);
    }
    const engineOf = new Map((backends.data ?? []).map((b) => [b.name, b.engine]));
    // 停用的后端不再出现在看板上：halogen 与 gufo 共用同一个 URL，切了引擎后
    // 旧引擎那张卡还挂着流量（请求其实由在跑的那家服务），看着就是「两个都在跑」。
    const enabledOf = new Map((backends.data ?? []).map((b) => [b.name, b.enabled]));
    const out: BackendGroup[] = [];
    for (const [backend, rows] of byBackend) {
      if (enabledOf.get(backend) === false) continue;
      rows.sort((a, b) => b.tokens_per_s - a.tokens_per_s || a.model.localeCompare(b.model));
      out.push({
        backend,
        engine: engineOf.get(backend) ?? null,
        rows,
        tps: rows.reduce((s, r) => s + r.tokens_per_s, 0),
        rps: rows.reduce((s, r) => s + r.rps, 0),
        inflight: rows.reduce((s, r) => s + r.active_streams, 0),
        errsPerS: rows.reduce((s, r) => s + r.errors_per_s, 0),
      });
    }
    out.sort(
      (a, b) => engineRank(a.engine) - engineRank(b.engine) || a.backend.localeCompare(b.backend),
    );
    return out;
  }, [rates.data, backends.data]);

  if (rates.isError) {
    return (
      <Card>
        <QueryError inline title="后端流量读取失败" error={rates.error} onRetry={() => rates.refetch()} />
      </Card>
    );
  }

  if (groups.length === 0) {
    return (
      <Card>
        <div className="mb-2 flex items-center justify-between">
          <h2 className="text-sm font-semibold">API 流量</h2>
        </div>
        <p className="py-2 text-xs text-ink-4">
          最近 60s 没有流量：有请求进来时按后端自动出现卡片。
        </p>
      </Card>
    );
  }

  const windowSecs = rates.data?.[0]?.window_secs ?? "—";
  return (
    <>
      {groups.map((g) => (
        <Card key={g.backend} className="overflow-hidden p-0">
          <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 px-4 pb-2 pt-3">
            <div className="flex items-center gap-2">
              <StatusDot busy={g.inflight > 0} />
              <h2 className="text-sm font-semibold" title={g.backend}>
                {backendLabel(g.backend)}
              </h2>
              {g.engine && (
                <span className="rounded bg-slate-700/60 px-1.5 py-0.5 text-2xs text-ink-2">
                  {g.engine}
                </span>
              )}
              <span className="text-xs text-ink-4">{g.rows.length} 个模型有流量</span>
            </div>
            <div className="flex flex-wrap items-end gap-x-5 gap-y-1 text-xs">
              <span>
                <span className="text-ink-4">吞吐 </span>
                <span className="font-mono text-cyan-300">
                  {Math.round(g.tps).toLocaleString()} tok/s
                </span>
              </span>
              <span>
                <span className="text-ink-4">请求 </span>
                <span className="font-mono text-ink-2">{g.rps.toFixed(2)} req/s</span>
              </span>
              <span>
                <span className="text-ink-4">在飞 </span>
                <span className="font-mono text-ink-2">{g.inflight}</span>
              </span>
              <span className={g.errsPerS > 0 ? "text-[var(--sem-danger-text)]" : ""}>
                <span className="text-ink-4">错误 </span>
                <span className="font-mono">{g.errsPerS.toFixed(2)}/s</span>
              </span>
              <span className="text-ink-4">曲线窗口 {windowSecs}s</span>
            </div>
          </div>
          {/* 引擎原生速率行已按用户口径移除（速率看板只留网关直接观测的吞吐/请求/在飞/错误）。 */}
          <div className="divide-y divide-slate-800 border-t border-slate-800">
            {g.rows.map((r) => (
              <div key={r.model} className="flex items-center gap-3 px-4 py-2">
                <div className="min-w-0 flex-1">
                  <div className="break-all font-mono text-xs leading-snug text-ink-2" title={r.model}>
                    {r.model}
                  </div>
                  <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-ink-4">
                    <span>{r.rps.toFixed(2)} req/s</span>
                    {r.ttft_p50_ms != null && (
                      <span>
                        TTFT {r.ttft_p50_ms}ms · n={r.ttft_samples}
                      </span>
                    )}
                    {r.active_streams > 0 && <span className="text-cyan-300">在飞 {r.active_streams}</span>}
                    {r.errs > 0 && (
                      <span className="text-[var(--sem-danger-text)]">错 {r.errs}</span>
                    )}
                  </div>
                </div>
                <div className="w-28 shrink-0 text-right">
                  <div className="text-xs tabular-nums text-ink-2">
                    {Math.round(r.tokens_per_s).toLocaleString()}
                    <span className="ml-1 text-xs text-ink-4">tok/s</span>
                  </div>
                  <div className="text-xs text-ink-4">累计 {r.out_tokens.toLocaleString()}</div>
                </div>
                <div className="hidden min-w-0 flex-1 sm:block">
                  <Sparkline points={r.series.map((p) => p.out_tokens)} height={22} />
                </div>
              </div>
            ))}
          </div>
        </Card>
      ))}
    </>
  );
}
