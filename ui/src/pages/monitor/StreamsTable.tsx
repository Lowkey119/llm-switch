// 进行中的流式交换明细表（计划 §6.1 自 Monitor.tsx 拆出，行为不变）。
import { Card } from "../../components/ui";
import { tableCls, theadCls } from "../../components/Table";
import { PHASE, fmtBytes, type InflightRow } from "./model";

export function StreamsTable({ streams }: { streams: InflightRow[] }) {
  return (
    <>
      {streams.length === 0 ? (
        <Card>
          <div className="py-12 text-center text-sm text-ink-4">
            当前没有进行中的请求。发起一个对话后这里会每秒刷新。
          </div>
        </Card>
      ) : (
        <Card className="max-h-[420px] overflow-auto p-0">
          {/* 内滚容器 + sticky 表头（UI 审查 P1-4）：在飞流可能很长，限高内滚
              比整页拉长好盯；px 定高不吃 zoom 的 vh 放大坑（见 styles.css 注释）。 */}
          <table className={tableCls}>
            <thead className={theadCls}>
              <tr>
                <th className="px-4 py-2">模型 · 后端</th>
                <th className="px-4 py-2">入口</th>
                <th className="px-4 py-2">相位</th>
                <th className="px-4 py-2 text-right">时长</th>
                <th className="px-4 py-2 text-right">prefill</th>
                <th className="px-4 py-2 text-right">TTFT</th>
                <th className="px-4 py-2 text-right">dec</th>
                <th className="px-4 py-2 text-right">字节</th>
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
                  <td className="px-4 py-2">
                    <span className={`rounded px-1.5 py-0.5 text-2xs ${PHASE[s.phase].cls}`}>
                      {PHASE[s.phase].label}
                    </span>
                  </td>
                  <td className="px-4 py-2 text-right font-mono">
                    {(s.elapsed_ms / 1000).toFixed(1)}s
                    {s.queue_ms > 0 && (
                      <span className="ml-1 text-2xs text-ink-4">
                        (排队 {(s.queue_ms / 1000).toFixed(1)}s)
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
                    <span className="ml-1 text-2xs text-ink-4">{s.chunks}块</span>
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
