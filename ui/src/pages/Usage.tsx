// 用量页（M8）：区间五卡 + 趋势 + 客户端环图 + 模型成本/后端两表 + 明细(CSV 导出) + 保留/定价设置。
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card } from "../components/ui";
import { QueryError, errText } from "../components/QueryError";
import { Tabs } from "../components/Tabs";
import { Bars, Donut } from "../components/charts";
import { theadCls } from "../components/Table";
import { ipc } from "../lib/ipc";
import { every, POLL } from "../lib/poll";
import type { Cost, DailyRow, Stats } from "./usage/types";


const RANGES = [
  { key: "today", label: "今日", days: 1 },
  { key: "7d", label: "近 7 天", days: 7 },
  { key: "30d", label: "近 30 天", days: 30 },
] as const;

type RangeKey = (typeof RANGES)[number]["key"];

export function UsagePage() {
  const [range, setRange] = useState<RangeKey>("today");
  // UI-14：柱状图点选（选中柱的当日明细显示在图下方）。
  const [selDay, setSelDay] = useState<string | null>(null);
  const spec = RANGES.find((r) => r.key === range)!;
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

  const rows = daily.data ?? [];
  const tot = (k: "requests" | "errors" | "input_tokens" | "output_tokens") =>
    rows.reduce((a, r) => a + r[k], 0);
  const reqs = tot("requests");
  const errs = tot("errors");
  const toks = tot("input_tokens") + tot("output_tokens");
  const s = stats.data;
  const c = cost.data;

  return (
    <div className="space-y-4">
      {/* 三个查询任一失败都要露出来：否则卡片全显示 0 / 「全部成功」，把失败伪装成正常 */}
      {(daily.isError || stats.isError || cost.isError) && (
        <QueryError
          title="读取用量数据失败"
          error={[
            daily.isError ? `日聚合：${errText(daily.error)}` : null,
            stats.isError ? `统计：${errText(stats.error)}` : null,
            cost.isError ? `成本：${errText(cost.error)}` : null,
          ]
            .filter(Boolean)
            .join("；")}
          onRetry={() => {
            if (daily.isError) daily.refetch();
            if (stats.isError) stats.refetch();
            if (cost.isError) cost.refetch();
          }}
        />
      )}

      {/* 区间切换 */}
      <div className="flex items-center gap-2">
        <Tabs
          variant="segmented"
          ariaLabel="统计区间"
          value={range}
          onChange={(id) => setRange(id as typeof range)}
          items={RANGES.map((r) => ({ id: r.key, label: r.label }))}
        />
      </div>

      {/* 四卡：列数随窗口自适应。主窗最小 900px、内容列上限 780px，
          定死 5 列时每列仅约 146px，value 是 text-xl 等宽数字、sub 是长文案，
          窄窗会挤成折行；改成 2→3→4 阶梯，窄窗折行、宽窗铺开。
          UI-14：缓存命中率卡撤下——网关侧全局开关实际未开（bypass 全量），
          恒显示「—/0」只会误导；等缓存真启用后再回来。 */}
      <div className="grid grid-cols-2 gap-3 md:grid-cols-2 xl:grid-cols-4">
        <Mini label="请求" value={reqs.toLocaleString()} sub={errs > 0 ? `错误 ${errs}` : "全部成功"} />
        <Mini label="tokens" value={toks.toLocaleString()} sub={`入 ${tot("input_tokens").toLocaleString()} · 出 ${tot("output_tokens").toLocaleString()}`} />
        <Mini
          label="在线成本 (USD)"
          value={c ? `$${c.total_cost_usd.toFixed(4)}` : "—"}
          sub={c ? `本地 ${c.local_requests} · 在线 ${c.online_requests} 请求${c.unpriced_requests > 0 ? ` · 未计价 ${c.unpriced_requests}` : ""}` : undefined}
        />
        <Mini
          label="TTFT P50 / P95"
          value={s?.ttft_p50_ms != null ? `${Math.round(s.ttft_p50_ms)} / ${Math.round(s.ttft_p95_ms!)}ms` : "—"}
          tone={reqs > 0 && errs / reqs > 0.05 ? "warn" : undefined}
          sub={reqs > 0 ? `错误率 ${((errs / reqs) * 100).toFixed(1)}%` : undefined}
        />
      </div>

      <div className="grid gap-4 md:grid-cols-3">
        <Card className="md:col-span-2">
          <h2 className="mb-2 text-sm font-semibold">输出 tokens / 日</h2>
          <Bars
            unit="tokens"
            rows={rows.map((r) => ({
              key: r.day,
              value: r.output_tokens,
              title: `${r.day}: ${r.output_tokens} out · ${r.requests} req`,
            }))}
            onSelect={setSelDay}
            selected={selDay}
          />
          {/* UI-14：点柱显示该柱用量（再点取消）。 */}
          <div className="mt-1 min-h-[1.25rem] text-xs text-ink-3">
            {selDay
              ? (() => {
                  const r = rows.find((x) => x.day === selDay);
                  return r
                    ? `${r.day}：入 ${r.input_tokens.toLocaleString()} · 出 ${r.output_tokens.toLocaleString()} tokens · ${r.requests} 请求`
                    : "";
                })()
              : "点击柱子查看当日用量"}
          </div>
        </Card>
        <Card>
          <h2 className="mb-2 text-sm font-semibold">按客户端</h2>
          <Donut
            items={(s?.by_client ?? []).map((b) => ({ label: b.client, value: b.requests }))}
          />
        </Card>
      </div>

      <div>
        {/* 模型成本表（UI-14：后端·流量卡删掉，本卡展宽占满整行） */}
        <Card className="overflow-hidden p-0">
          <h2 className="border-b border-slate-800 px-4 py-2 text-sm font-semibold">模型 · 成本</h2>
          <table className="w-full text-xs">
            <thead className={theadCls}>
              <tr>
                <th className="px-4 py-2 text-left">模型</th>
                <th className="whitespace-nowrap py-2 text-left">分档</th>
                <th className="py-2 text-right">请求</th>
                <th className="py-2 text-right">tokens</th>
                <th className="px-4 py-2 text-right">成本 $</th>
              </tr>
            </thead>
            <tbody>
              {(c?.by_model ?? []).map((m) => (
                <tr key={`${m.model}-${m.scope}`} className="border-t border-slate-800">
                  <td className="px-4 py-1.5 font-mono text-ink-2">
                    <span className="whitespace-nowrap">{m.model || "—"}</span>
                    {m.scope === "online" && !m.priced && (
                      <span className="ml-1 rounded bg-slate-700/60 px-1 text-2xs text-ink-3">未计价</span>
                    )}
                  </td>
                  <td className="whitespace-nowrap py-1.5">
                    <span
                      className={`rounded px-1.5 py-px text-2xs ${
                        m.scope === "local"
                          ? "bg-slate-700/50 text-ink-3"
                          : "bg-emerald-500/15 text-[var(--sem-ok-text)]"
                      }`}
                      title={m.scope === "local" ? "本地引擎（自托管），不计费" : "在线 API：按手工单价计价"}
                    >
                      {m.scope === "local" ? "本地" : "在线"}
                    </span>
                  </td>
                  <td className="py-1.5 text-right font-mono">{m.requests}</td>
                  <td className="py-1.5 text-right font-mono">{(m.input_tokens + m.output_tokens).toLocaleString()}</td>
                  <td className="px-4 py-1.5 text-right font-mono text-[var(--sem-ok-text)]">
                    {m.scope === "local" ? <span className="text-ink-4">—</span> : m.cost_usd.toFixed(4)}
                  </td>
                </tr>
              ))}
              {!c?.by_model.length && <Empty cols={5} />}
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

function Empty({ cols }: { cols: number }) {
  return (
    <tr>
      <td colSpan={cols} className="px-4 py-8 text-center text-ink-4">
        区间内暂无数据
      </td>
    </tr>
  );
}



function Mini({ label, value, sub, tone }: { label: string; value: string; sub?: string; tone?: "warn" }) {
  return (
    <Card>
      <div className="text-xs text-ink-4">{label}</div>
      <div className={`mt-1 font-mono text-xl ${tone === "warn" ? "text-[var(--sem-busy-text)]" : "text-ink-1"}`}>{value}</div>
      {sub && <div className="mt-1 text-xs text-ink-4">{sub}</div>}
    </Card>
  );
}
