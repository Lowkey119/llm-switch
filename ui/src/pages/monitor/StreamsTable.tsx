// 进行中的流式交换明细表（计划 §6.1 自 Monitor.tsx 拆出，行为不变）。
import { Card } from "../../components/ui";
import { EmptyState } from "../../components/States";
import { IconMonitor } from "../../components/icons";
import { t, ts } from "../../lib/i18n";
import { tableCls, theadCls } from "../../components/Table";
import { PHASE, fmtBytes, type InflightRow } from "./model";

export function StreamsTable({ streams }: { streams: InflightRow[] }) {
  return (
    <>
      {streams.length === 0 ? (
        <Card>
          <EmptyState
            icon={IconMonitor}
            title={t("mon.emptyStreamsTitle")}
            desc={t("mon.emptyStreamsDesc")}
          />
        </Card>
      ) : (
        <Card className="max-h-[420px] overflow-auto p-0">
          {/* 内滚容器 + sticky 表头（UI 审查 P1-4）：在飞流可能很长，限高内滚
              比整页拉长好盯；px 定高不吃 zoom 的 vh 放大坑（见 styles.css 注释）。 */}
          <table className={tableCls}>
            <thead className={theadCls}>
              <tr>
                <th className="px-4 py-2">{t("mon.th.model")}</th>
                <th className="px-4 py-2">{t("mon.th.entry")}</th>
                <th className="px-4 py-2">{t("mon.th.agent")}</th>
                <th className="px-4 py-2">{t("mon.th.phase")}</th>
                <th className="px-4 py-2 text-right">{t("mon.th.elapsed")}</th>
                <th className="px-4 py-2 text-right">prefill</th>
                <th className="px-4 py-2 text-right">TTFT</th>
                <th className="px-4 py-2 text-right">dec</th>
                <th className="px-4 py-2 text-right">{t("mon.th.bytes")}</th>
              </tr>
            </thead>
            <tbody>
              {streams.map((s) => (
                <tr key={s.id} className="border-t border-slate-800">
                  <td className="px-4 py-2">
                    <span className="font-mono text-ink-2">{s.model}</span>
                    <span className="ml-2 text-ink-4">@{s.backend}</span>
                  </td>
                  <td className="px-4 py-2 text-ink-3">{s.entry}</td>
                  {/* 归属可核对：速率线张冠李戴时，拿这个 pid 对 `ps` 就能分清是
                      lsof 认错进程还是判定串味，不用再靠猜。 */}
                  <td className="px-4 py-2">
                    <span className="text-cyan-300">{s.app ?? t("common.unknown")}</span>
                    <span className="ml-1 font-mono text-2xs text-ink-4">
                      {s.pid ? `#${s.pid}` : ""}
                    </span>
                  </td>
                  <td className="px-4 py-2">
                    <span className={`rounded px-1.5 py-0.5 text-2xs ${PHASE[s.phase].cls}`}>
                      {t(PHASE[s.phase].labelKey)}
                    </span>
                  </td>
                  <td className="px-4 py-2 text-right font-mono">
                    {(s.elapsed_ms / 1000).toFixed(1)}s
                    {s.queue_ms > 0 && (
                      <span className="ml-1 text-2xs text-ink-4">
                        {ts("mon.queuedSec", { s: (s.queue_ms / 1000).toFixed(1) })}
                      </span>
                    )}
                  </td>
                  <td className="px-4 py-2 text-right font-mono">
                    {s.phase === "prefill" ? (
                      <span className={s.outlier ? "text-[var(--sem-danger-text)]" : "text-[var(--sem-busy-text)]"}>
                        ~{Math.round(s.prefill_tps).toLocaleString()} tok/s{s.outlier ? " ?!" : ""}
                        <span className="ml-1 text-2xs text-ink-4">
                          {(s.prefill_ms / 1000).toFixed(1)}s
                        </span>
                      </span>
                    ) : s.phase === "queued" ? (
                      <span className="text-ink-4">…</span>
                    ) : (
                      <span className="text-ink-4">
                        {(s.prefill_ms / 1000).toFixed(1)}s{s.calibrated ? "" : "*"}
                      </span>
                    )}
                  </td>
                  <td className="px-4 py-2 text-right font-mono">
                    {s.ttft_ms === null ? "…" : `${s.ttft_ms}ms`}
                  </td>
                  <td className="px-4 py-2 text-right font-mono text-cyan-300">
                    ~{s.dec_tps.toFixed(1)}
                    <span className="ml-1 text-2xs text-ink-4">{ts("mon.chunksShort", { n: s.chunks })}</span>
                  </td>
                  <td className="px-4 py-2 text-right font-mono">{fmtBytes(s.bytes)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
    </>
  );
}
