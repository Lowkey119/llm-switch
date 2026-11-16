// 系统与引擎卡（切片 12）。展示形态借鉴 unsloth studio 的监控界面——
// 只借鉴思路，未复制任何代码（AGPL 红线）。
//
// 数据只有一个来源：get_sys_stats（services/sysmon.rs，TTL 缓存在后端）。
// 红线：字段为 null = 拿不到，一律显示 —，绝不编数。
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card, Toggle } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import { every, POLL } from "../../lib/poll";
import { fmtSize } from "./model";

/** 本机快照（sysmon.rs local_snapshot 的前端形状；资源小窗同源复用）。 */
export interface SysLocal {
  ok: boolean;
  cpu_pct: number | null;
  cpu_cores: number | null;
  load: [number, number, number] | null;
  mem_used: number | null;
  mem_total: number | null;
  mem_available: number | null;
  gpu_name: string | null;
  gpu_pct: number | null;
  vram_shared: boolean;
  cpu_temp: number | null;
  gpu_temp: number | null;
  power_w: number | null;
  disk_total: number | null;
  disk_free: number | null;
}

/** 102 快照（remote_view 的前端形状）；ok=false 时只有 error。 */
export interface SysRemote {
  ok: boolean;
  error?: string;
  hostname?: string | null;
  cpu_temp?: number | null;
  gpu_temp?: number | null;
  power_w?: number | null;
  mem_total?: number | null;
  mem_used?: number | null;
  load?: [number, number, number] | null;
  cpu_cores?: number | null;
  disk_total?: number | null;
  disk_free?: number | null;
  gpu_pct?: number | null;
  vram_used?: number | null;
  vram_total?: number | null;
}

export interface SysStats {
  local: SysLocal;
  remote: SysRemote;
}

/** 占用比 0–100；分母缺失/为零 → null（显示 —，不猜）。 */
function pct(used: number | null | undefined, total: number | null | undefined): number | null {
  if (used == null || total == null || total <= 0) return null;
  return Math.min(100, Math.max(0, (used / total) * 100));
}

/** 瓦片：小标签 + 主值 + 副值 + 可选进度条。缺值一律 —。 */
function Tile({
  label,
  value,
  sub,
  bar,
  title,
}: {
  label: string;
  value: string;
  sub?: string;
  bar?: number | null;
  title?: string;
}) {
  return (
    <div className="min-w-0 rounded-md border border-slate-800 bg-slate-800/40 px-2.5 py-2" title={title}>
      <div className="text-2xs uppercase tracking-wide text-ink-4">{label}</div>
      <div className="break-words font-mono text-sm leading-snug text-ink-2">{value}</div>
      {sub && <div className="break-words text-2xs leading-snug text-ink-4">{sub}</div>}
      {bar != null && (
        <div className="mt-1 h-1 overflow-hidden rounded bg-slate-700/60">
          <div className="h-full rounded bg-cyan-500/80" style={{ width: `${bar}%` }} />
        </div>
      )}
    </div>
  );
}

/** 一台机器的瓦片组。字段缺失（null）一律显示 —。 */
function MachineTiles({ l }: { l: SysLocal }) {
  const diskUsed =
    l.disk_total != null && l.disk_free != null ? l.disk_total - l.disk_free : null;
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
      <Tile
        label="CPU"
        value={l.cpu_pct != null ? `${l.cpu_pct.toFixed(0)}%` : "—"}
        sub={
          [
            l.cpu_cores != null ? `${l.cpu_cores} 核` : null,
            l.load ? `load ${l.load.map((n) => n.toFixed(1)).join(" / ")}` : null,
          ]
            .filter(Boolean)
            .join(" · ") || undefined
        }
      />
      <Tile
        label="内存"
        value={`${fmtSize(l.mem_used)} / ${fmtSize(l.mem_total)}`}
        sub={l.mem_available != null ? `可用 ${fmtSize(l.mem_available)}` : undefined}
        bar={pct(l.mem_used, l.mem_total)}
      />
      <Tile
        label="GPU"
        value={l.gpu_pct != null ? `${l.gpu_pct.toFixed(0)}%` : "—"}
        sub={l.vram_shared ? "显存与内存共享" : undefined}
        title={l.gpu_name ?? undefined}
      />
      <Tile
        label="温度 CPU / GPU"
        value={`${l.cpu_temp != null ? `${l.cpu_temp.toFixed(0)}°` : "—"} / ${l.gpu_temp != null ? `${l.gpu_temp.toFixed(0)}°` : "—"}`}
      />
      <Tile label="功率" value={l.power_w != null ? `${l.power_w.toFixed(0)} W` : "—"} />
      <Tile
        label="磁盘"
        value={`${fmtSize(diskUsed)} / ${fmtSize(l.disk_total)}`}
        sub={l.disk_free != null ? `可用 ${fmtSize(l.disk_free)}` : undefined}
        bar={pct(diskUsed, l.disk_total)}
      />
    </div>
  );
}

function RemoteTiles({ r }: { r: SysRemote }) {
  if (!r.ok) {
    return (
      <p className="rounded-md border border-slate-800 bg-slate-800/40 px-2.5 py-2 text-xs text-[var(--sem-danger-text)]" title={r.error}>
        SSH 采集失败{r.error ? `：${r.error}` : ""}
      </p>
    );
  }
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
      <Tile
        label="CPU"
        value={r.cpu_cores != null ? `${r.cpu_cores} 核` : "—"}
        sub={r.load ? `load ${r.load.map((n) => n.toFixed(1)).join(" / ")}` : undefined}
        title="负载均值（1/5/15 分钟）；该机未采 CPU 占用百分比"
      />
      <Tile label="内存" value={`${fmtSize(r.mem_used)} / ${fmtSize(r.mem_total)}`} bar={pct(r.mem_used, r.mem_total)} />
      <Tile
        label="GPU"
        value={r.gpu_pct != null ? `${r.gpu_pct.toFixed(0)}%` : "—"}
        sub={r.vram_used != null || r.vram_total != null ? `VRAM ${fmtSize(r.vram_used)} / ${fmtSize(r.vram_total)}` : undefined}
      />
      <Tile
        label="温度 CPU / GPU"
        value={`${r.cpu_temp != null ? `${r.cpu_temp.toFixed(0)}°` : "—"} / ${r.gpu_temp != null ? `${r.gpu_temp.toFixed(0)}°` : "—"}`}
        sub="sensors Tctl / edge"
      />
      <Tile label="功率" value={r.power_w != null ? `${r.power_w.toFixed(0)} W` : "—"} sub="sensors PPT（封装）" />
      <Tile
        label="磁盘 /"
        value={r.disk_total != null ? `${fmtSize(r.disk_total - (r.disk_free ?? 0))} / ${fmtSize(r.disk_total)}` : "—"}
        sub={r.disk_free != null ? `可用 ${fmtSize(r.disk_free)}` : undefined}
        bar={pct((r.disk_total ?? 0) - (r.disk_free ?? 0), r.disk_total)}
      />
    </div>
  );
}

/** 「系统与引擎」卡：两台机器分组并列 + 实时更新开关（借鉴 unsloth 面板设计）。 */
export function SysEngineCard() {
  // 实时更新默认开；关掉即停止轮询（后端 TTL 缓存也就不用再打了）。
  const [live, setLive] = useState(true);
  const q = useQuery({
    queryKey: ["sys-stats"],
    queryFn: () => ipc<SysStats>("get_sys_stats"),
    refetchInterval: () => (live ? every(POLL.fast) : false),
  });
  const s = q.data;
  return (
    <Card>
      <div className="mb-2 flex items-center justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold">系统与引擎</h2>
        </div>
        <label className="flex shrink-0 cursor-pointer items-center gap-2 text-xs text-ink-3">
          实时更新
          <Toggle checked={live} onChange={setLive} />
        </label>
      </div>
      {q.isError ? (
        <QueryError inline title="系统指标读取失败" error={q.error} onRetry={() => q.refetch()} />
      ) : !s ? (
        <p className="py-4 text-center text-xs text-ink-4">
          {live ? "采集中…" : "实时更新已关闭"}
        </p>
      ) : (
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          <div className="min-w-0">
            <h3 className="mb-1.5 text-xs font-medium text-ink-2" title={s.local.gpu_name ?? "本机"}>
              本机{live && <span className="ml-1.5 inline-block h-1.5 w-1.5 rounded-full bg-[var(--sem-ok-text)] align-middle" />}
              {s.local.gpu_name && <span className="ml-1.5 font-normal text-ink-4">{s.local.gpu_name}</span>}
            </h3>
            <MachineTiles l={s.local} />
          </div>
          <div className="min-w-0">
            <h3 className="mb-1.5 text-xs font-medium text-ink-2" title={s.remote.hostname ?? "102"}>
              102 · halogen
              {live && s.remote.ok && <span className="ml-1.5 inline-block h-1.5 w-1.5 rounded-full bg-[var(--sem-ok-text)] align-middle" />}
              {s.remote.hostname && <span className="ml-1.5 font-normal text-ink-4">{s.remote.hostname}</span>}
            </h3>
            <RemoteTiles r={s.remote} />
          </div>
        </div>
      )}
    </Card>
  );
}
