// 用量页（M8）：区间五卡 + 趋势 + 客户端环图 + 模型成本/后端两表 + 明细(CSV 导出) + 保留/定价设置。
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card } from "../components/ui";

import { QueryError, errText } from "../components/QueryError";
import { EmptyCell } from "../components/States";
import { Bars, Donut } from "../components/charts";
import { theadCls, useSort, sortRows, SortTh } from "../components/Table";
import type { SortState } from "../components/Table";
import { ipc } from "../lib/ipc";
import { every, POLL } from "../lib/poll";
import { t, ts } from "../lib/i18n";
import type { Cost, DailyRow, Stats } from "./usage/types";

/** 模型 · 成本表的可排序列（ui_01 回灌 U4）。 */
type CostKey = "model" | "scope" | "requests" | "tokens" | "cost";

/** 媒体计量聚合行（usage_media）：张数/秒数 null = 取不到（显示 —，不造假数字）。 */
interface MediaRow {
  model: string | null;
  backend: string | null;
  requests: number;
  images: number | null;
  seconds: number | null;
}

/** 音频时长人话格式：短的用秒、两分钟起换分钟；取不到一律 —。 */
const fmtDur = (s: number | null) =>
  s == null ? "—" : s >= 120 ? `${(s / 60).toFixed(1)} min` : `${s.toFixed(1)} s`;


export const USAGE_RANGES = [
  { key: "today", labelKey: "usage.range.today", days: 1 },
  { key: "7d", labelKey: "usage.range.7d", days: 7 },
  { key: "30d", labelKey: "usage.range.30d", days: 30 },
] as const;

export type UsageRangeKey = (typeof USAGE_RANGES)[number]["key"];

/** range 状态上提到观测页（用户 2026-11-21）：区间选择器并进页签行右侧，不再独占一行。 */
export function UsagePage({ range }: { range: UsageRangeKey }) {
  // UI-14：柱状图点选（选中柱的当日明细显示在图下方）。
  const [selDay, setSelDay] = useState<string | null>(null);
  const spec = USAGE_RANGES.find((r) => r.key === range)!;
  const win =
    spec.days === 1
      ? { from: new Date().setHours(0, 0, 0, 0), to: Date.now() }
      : { from: Date.now() - spec.days * 86400000, to: Date.now() };

  const daily = useQuery({
    queryKey: ["usage-daily", spec.days],
    queryFn: () => ipc<DailyRow[]>("usage_daily", { days: spec.days }),
    refetchInterval: () => every(POLL.slow),
  });
  const stats = useQuery({
    queryKey: ["usage-stats", range],
    queryFn: () => ipc<Stats>("usage_stats", { from: win.from, to: win.to }),
    refetchInterval: () => every(POLL.slow),
  });
  const cost = useQuery({
    queryKey: ["usage-cost", range],
    queryFn: () => ipc<Cost>("usage_cost", { from: win.from, to: win.to }),
    refetchInterval: () => every(POLL.idle),
  });
  const media = useQuery({
    queryKey: ["usage-media", range],
    queryFn: () => ipc<MediaRow[]>("usage_media", { from: win.from, to: win.to }),
    refetchInterval: () => every(POLL.slow),
  });

  const rows = daily.data ?? [];
  // 日聚合到底拿到没有（U5）：没拿到就不拿 0 当数据。
  const dailyOk = !!daily.data && !daily.isError;
  const tot = (k: "requests" | "errors" | "input_tokens" | "output_tokens") =>
    rows.reduce((a, r) => a + r[k], 0);
  const reqs = tot("requests");
  const errs = tot("errors");
  const toks = tot("input_tokens") + tot("output_tokens");
  const s = stats.data;
  const c = cost.data;

  // 模型 · 成本表排序（ui_01 回灌 U4）：列标识→取值的映射交给共享 sortRows。
  const costSort = useSort<CostKey>();
  const costRows = sortRows(c?.by_model ?? [], costSort.sort as SortState<string>, (m, k) => {
    if (k === "tokens") return m.input_tokens + m.output_tokens;
    if (k === "cost") return m.cost_usd;
    if (k === "scope") return m.scope;
    if (k === "requests") return m.requests;
    return m.model;
  });

  return (
    <div className="space-y-4 pt-[var(--content-pad-top)]">
      {/* 三个查询任一失败都要露出来：否则卡片全显示 0 / 「全部成功」，把失败伪装成正常 */}
      {(daily.isError || stats.isError || cost.isError || media.isError) && (
        <QueryError
          title={t("usage.loadFail")}
          details={[
            daily.isError ? ts("usage.errDaily", { e: errText(daily.error) }) : "",
            stats.isError ? ts("usage.errStats", { e: errText(stats.error) }) : "",
            cost.isError ? ts("usage.errCost", { e: errText(cost.error) }) : "",
            media.isError ? ts("usage.errMedia", { e: errText(media.error) }) : "",
          ]}
          onRetry={() => {
            if (daily.isError) daily.refetch();
            if (stats.isError) stats.refetch();
            if (cost.isError) cost.refetch();
            if (media.isError) media.refetch();
          }}
        />
      )}

      {/* 四卡：列数由容器宽 ÷ --flow-kpi 算出来（U9 连续重排），不再看视口断点——
          原先定死两列、到 xl 才跳四列（中间那档 md 干脆是句重复），而断点量的是视口，
          边条收起明明多出 152px，列数却一动不动。下限按下面 U1 那笔 145px 的账定。
          UI-14：缓存命中率卡撤下——网关侧全局开关实际未开（bypass 全量），
          恒显示「—/0」只会误导；等缓存真启用后再回来。
          ui_01 回灌 U1：text-xl 抬到 22px 后实测卡内只有 154px，「1234 / 5678ms」
          要 173px（20px 时已经 157px，本就临界）——单位提到标签走微标注口径
          （与「在线成本 (USD)」同一写法），值只留数字：145px 装得下。 */}
      <div className="kpi-grid gap-3">
        {/* U5：无数据一律 `—`。查询失败或还没回来时不能报「0 请求 / 全部成功」——
            那是把失败伪装成正常，与上面那块红字自相矛盾。 */}
        <Mini
          label={t("usage.kpiReq")}
          value={dailyOk ? reqs.toLocaleString() : "—"}
          sub={dailyOk ? (errs > 0 ? ts("usage.kpiErr", { n: errs }) : t("usage.kpiAllOk")) : undefined}
        />
        <Mini
          label="tokens"
          value={dailyOk ? toks.toLocaleString() : "—"}
          sub={
            dailyOk
              ? ts("usage.kpiInOut", {
                  in: tot("input_tokens").toLocaleString(),
                  out: tot("output_tokens").toLocaleString(),
                })
              : undefined
          }
        />
        <Mini
          label={t("usage.kpiCost")}
          value={c ? `$${c.total_cost_usd.toFixed(4)}` : "—"}
          sub={
            c
              ? ts("usage.kpiSplit", { local: c.local_requests, online: c.online_requests }) +
                (c.unpriced_requests > 0 ? ts("usage.kpiUnpriced", { n: c.unpriced_requests }) : "")
              : undefined
          }
        />
        <Mini
          label="TTFT P50 / P95 (ms)"
          value={s?.ttft_p50_ms != null ? `${Math.round(s.ttft_p50_ms)} / ${Math.round(s.ttft_p95_ms!)}` : "—"}
          tone={reqs > 0 && errs / reqs > 0.05 ? "warn" : undefined}
          sub={reqs > 0 ? ts("usage.kpiErrRate", { n: ((errs / reqs) * 100).toFixed(1) }) : undefined}
        />
      </div>

      {/* 柱图 + 环图：主栏吃掉剩余宽度，侧栏 clamp 连续变化（U9）。
          原先三档栅格 + 柱图占两档，在 900 窗口给环图只剩 212px，图例读不动。 */}
      <div className="split-chart gap-4">
        <Card>
          <h2 className="mb-2 text-base font-semibold">{t("usage.chartTokens")}</h2>
          {/* UI-14：点柱显示该柱用量（再点取消）。U8：读数行收进 Bars——
              悬停 / 键盘聚焦 / 选中三种来源共用同一行，页面不再自己写一句。
              柱值 = 当日实际发生的 token 总量（入+出）：只画输出时，单日几百 M 的
              输入量在图上完全看不见，坐标轴与真实用量对不上（用户实报）。 */}
          <Bars
            unit="tokens"
            hint={t("usage.clickBar")}
            rows={rows.map((r) => ({
              key: r.day,
              value: r.input_tokens + r.output_tokens,
              sub: ts("usage.dayBreak", {
                out: r.output_tokens.toLocaleString(),
                in: r.input_tokens.toLocaleString(),
                req: r.requests,
              }),
            }))}
            onSelect={setSelDay}
            selected={selDay}
          />
        </Card>
        <Card>
          <h2 className="mb-2 text-base font-semibold">{t("usage.byClient")}</h2>
          <Donut
            unit={t("usage.requestsUnit")}
            items={(s?.by_client ?? []).map((b) => ({ label: b.client, value: b.requests }))}
          />
        </Card>
      </div>

      <div>
        {/* 模型成本表（UI-14：后端·流量卡删掉，本卡展宽占满整行） */}
        <Card className="overflow-hidden p-0">
          <h2 className="border-b border-slate-800 px-4 py-2 text-base font-semibold">{t("usage.costTitle")}</h2>
          <table className="w-full text-xs">
            <thead className={theadCls}>
              <tr>
                <SortTh label={t("usage.thModel")} k="model" sort={costSort.sort} onSort={costSort.toggle} className="px-4 py-2" />
                <SortTh label={t("usage.scope")} k="scope" sort={costSort.sort} onSort={costSort.toggle} className="whitespace-nowrap py-2" />
                <SortTh label={t("usage.kpiReq")} k="requests" align="right" sort={costSort.sort} onSort={costSort.toggle} className="py-2" />
                <SortTh label="tokens" k="tokens" align="right" sort={costSort.sort} onSort={costSort.toggle} className="py-2" />
                <SortTh label={t("usage.costTh")} k="cost" align="right" sort={costSort.sort} onSort={costSort.toggle} className="px-4 py-2" />
              </tr>
            </thead>
            <tbody>
              {costRows.map((m) => (
                <tr key={`${m.model}-${m.scope}`} className="border-t border-slate-800">
                  <td className="px-4 py-1.5 font-mono text-ink-2">
                    <span className="whitespace-nowrap">{m.model || "—"}</span>
                    {m.scope === "online" && !m.priced && (
                      <span className="ml-1 rounded bg-slate-700/60 px-1 text-2xs text-ink-3">{t("usage.unpriced")}</span>
                    )}
                  </td>
                  <td className="whitespace-nowrap py-1.5">
                    <span
                      className={`rounded px-1.5 py-px text-2xs ${
                        m.scope === "local"
                          ? "bg-slate-700/50 text-ink-3"
                          : "bg-emerald-500/15 text-[var(--sem-ok-text)]"
                      }`}
                      title={m.scope === "local" ? t("usage.scopeLocalTip") : t("usage.scopeOnlineTip")}
                    >
                      {m.scope === "local" ? t("usage.scopeLocal") : t("usage.scopeOnline")}
                    </span>
                  </td>
                  <td className="py-1.5 text-right font-mono">{m.requests}</td>
                  <td className="py-1.5 text-right font-mono">{(m.input_tokens + m.output_tokens).toLocaleString()}</td>
                  <td className="px-4 py-1.5 text-right font-mono text-[var(--sem-ok-text)]">
                    {m.scope === "local" ? <span className="text-ink-4">—</span> : m.cost_usd.toFixed(4)}
                  </td>
                </tr>
              ))}
              {!c?.by_model.length && <EmptyCell cols={5}>{t("usage.emptyRange")}</EmptyCell>}
            </tbody>
          </table>
        </Card>
      </div>

      {/* 媒体计量（媒体端点透传代理 11d）：张数/时长取自上游响应，取不到显示 —。 */}
      <div>
        <Card className="overflow-hidden p-0">
          <h2
            className="border-b border-slate-800 px-4 py-2 text-base font-semibold"
            title={t("usage.mediaTip")}
          >
            {t("usage.media")}
          </h2>
          <table className="w-full text-xs">
            <thead className={theadCls}>
              <tr>
                <th className="px-4 py-2 text-left">{t("usage.mediaModel")}</th>
                <th className="py-2 text-left">{t("usage.mediaBackend")}</th>
                <th className="py-2 text-right">{t("usage.mediaReqs")}</th>
                <th className="py-2 text-right">{t("usage.mediaImages")}</th>
                <th className="px-4 py-2 text-right">{t("usage.mediaSeconds")}</th>
              </tr>
            </thead>
            <tbody>
              {(media.data ?? []).map((r) => (
                <tr key={`${r.model}-${r.backend}`} className="border-t border-slate-800">
                  <td className="px-4 py-1.5 font-mono text-ink-2">
                    <span className="whitespace-nowrap">{r.model || "—"}</span>
                  </td>
                  <td className="py-1.5 text-ink-3">{r.backend || "—"}</td>
                  <td className="py-1.5 text-right font-mono">{r.requests}</td>
                  <td className="py-1.5 text-right font-mono">
                    {r.images ?? <span className="text-ink-4">—</span>}
                  </td>
                  <td className="px-4 py-1.5 text-right font-mono">
                    {r.seconds == null ? <span className="text-ink-4">—</span> : fmtDur(r.seconds)}
                  </td>
                </tr>
              ))}
              {!media.data?.length && <EmptyCell cols={5}>{t("usage.emptyRange")}</EmptyCell>}
            </tbody>
          </table>
        </Card>
      </div>
      {/* UI-20（用户 2026-11-16，附截图）：「请求明细」表与「运维设置（明细保留）」
          卡整块删掉——明细表与日志页重复；明细保留天数（usage_retention）后端保留，
          UI 入口按用户口径撤下。 */}
    </div>
  );
}



function Mini({ label, value, sub, tone }: { label: string; value: string; sub?: string; tone?: "warn" }) {
  return (
    <Card>
      <div className="text-sm text-ink-4">{label}</div>
      <div className={`mt-1 font-mono text-xl tabular-nums ${tone === "warn" ? "text-[var(--sem-busy-text)]" : "text-ink-1"}`}>{value}</div>
      {sub && <div className="mt-1 text-sm text-ink-4">{sub}</div>}
    </Card>
  );
}
