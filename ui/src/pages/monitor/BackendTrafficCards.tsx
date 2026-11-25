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
import { t, ts } from "../../lib/i18n";
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
  /** 直连标记：切直连后网关看不到新流量，卡上钉「不经网关」（计划决策 5）。 */
  direct: boolean;
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
    const directOf = new Map((backends.data ?? []).map((b) => [b.name, b.direct]));
    const out: BackendGroup[] = [];
    for (const [backend, rows] of byBackend) {
      if (enabledOf.get(backend) === false) continue;
      rows.sort((a, b) => b.tokens_per_s - a.tokens_per_s || a.model.localeCompare(b.model));
      out.push({
        backend,
        engine: engineOf.get(backend) ?? null,
        direct: directOf.get(backend) ?? false,
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
        <QueryError inline title={t("mon.ratesError")} error={rates.error} onRetry={() => rates.refetch()} />
      </Card>
    );
  }

  if (groups.length === 0) {
    return (
      <Card>
        <div className="mb-2 flex items-center justify-between">
          <h2 className="text-base font-semibold">{t("mon.apiTraffic")}</h2>
        </div>
        <p className="py-2 text-sm text-ink-4">{t("mon.noTraffic")}</p>
      </Card>
    );
  }

  return (
    <>
      {groups.map((g) => (
        // U8：卡上的 overflow-hidden 撤了——它会把迷你曲线的悬停读数（.tip）连根裁掉。
        <Card key={g.backend} className="p-0">
          {/* 每个后端固定两行（用户 2026-11-23 截图拍板）：
              第 1 行 = 后端名（固定不滚）+ 去重后的信息，一行放不下时其余横向滚动；
              第 2 行 = 吞吐曲线（每有流量的模型一条）。
              旧版三段（标题行 / 统计行 / 曲线行）里的重复项一并删：
              引擎徽标 ≈ 后端名、逐模型「在飞」≈ 卡头在飞、曲线左标 tok/s ≈ 卡头吞吐、
              「曲线窗口 600s」无意义——全部去掉，只留一份。 */}
          <div className="flex flex-col gap-1.5 px-4 py-2.5">
            <div className="flex items-center gap-3">
              <div className="flex shrink-0 items-center gap-2">
                <StatusDot busy={g.inflight > 0} />
                {/* 状态灯必须配文字（法条）：灯旁钉「在飞 N / 空闲」，颜色只是加强。 */}
                <span className="shrink-0 text-2xs text-ink-3">
                  {g.inflight > 0 ? `${t("mon.labelInflight")} ${g.inflight}` : t("mon.idle")}
                </span>
                <h2 className="whitespace-nowrap text-base font-semibold" title={g.backend}>
                  {backendLabel(g.backend)}
                </h2>
                {g.direct && (
                  /* 切直连后旧流量还在 60s 窗口里的过渡期：卡照旧出现，但钉一条「不经网关」，
                     免得用户对着不再更新的数字发懵（计划决策 5）。 */
                  <span className="rounded border border-slate-700 px-1.5 py-0.5 text-2xs text-ink-3">
                    {t("mon.directTag")}
                  </span>
                )}
                {/* 引擎徽标只在与后端名不同才挂（用户 2026-11-23：同名时 1、2 重复，删一个）。
                    大小写不敏感：后端名 "Unslow" 与引擎 "unslow" 是同一个，别挂两遍。 */}
                {g.engine && g.engine.toLowerCase() !== g.backend.toLowerCase() && (
                  <span className="rounded bg-slate-700/60 px-1.5 py-0.5 text-2xs text-ink-2">
                    {g.engine}
                  </span>
                )}
              </div>
              {/* 横滚区可聚焦（UI-AUDIT-5 P1-2）：窄窗读数被裁时，键盘 Tab 进来能左右滚。 */}
              <div className="min-w-0 flex-1 overflow-x-auto" tabIndex={0} role="group" aria-label={t("mon.readoutsAria")}>
                <div className="flex items-center gap-x-4 whitespace-nowrap text-sm">
                  <span className="text-ink-4">{ts("mon.modelsActive", { n: g.rows.length })}</span>
                  {g.rows.map((r) => (
                    <span key={r.model} className="tip font-mono text-xs text-ink-2" data-tip={r.model}>
                      {r.model}
                    </span>
                  ))}
                  <span>
                    <span className="text-ink-4">{t("mon.labelTps")} </span>
                    <span className="font-mono text-cyan-300">
                      {Math.round(g.tps).toLocaleString()} tok/s
                    </span>
                  </span>
                  <span>
                    <span className="text-ink-4">{t("mon.labelRps")} </span>
                    <span className="font-mono text-ink-2">{g.rps.toFixed(2)} req/s</span>
                  </span>
                  <span className={g.errsPerS > 0 ? "text-[var(--sem-danger-text)]" : ""}>
                    <span className="text-ink-4">{t("mon.labelErrs")} </span>
                    <span className="font-mono">{g.errsPerS.toFixed(2)}/s</span>
                  </span>
                  {g.rows.map((r) =>
                    r.ttft_p50_ms != null ? (
                      <span key={r.model} className="text-ink-4">
                        TTFT {r.ttft_p50_ms}ms · n={r.ttft_samples}
                      </span>
                    ) : null,
                  )}
                </div>
              </div>
            </div>
            {g.rows.map((r) => (
              <div key={r.model} className="flex items-center gap-3">
                <div className="w-24 shrink-0 text-right text-sm text-ink-4">
                  {ts("mon.totalOut", { n: r.out_tokens.toLocaleString() })}
                </div>
                <div className="min-w-0 flex-1">
                  <Sparkline points={r.series.map((p) => p.out_tokens)} height={22} unit="tok/s" />
                </div>
              </div>
            ))}
          </div>
        </Card>
      ))}
    </>
  );
}
