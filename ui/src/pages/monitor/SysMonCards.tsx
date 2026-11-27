// 系统与引擎卡（切片 12）。展示形态借鉴 unsloth studio 的监控界面——
// 只借鉴思路，未复制任何代码（AGPL 红线）。
//
// 数据只有一个来源：get_sys_stats（services/sysmon.rs，TTL 缓存在后端）。
// 红线：字段为 null = 拿不到，一律显示 —，绝不编数。
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Card, StatusDot, Toggle } from "../../components/ui";
import { QueryError } from "../../components/QueryError";
import { ipc } from "../../lib/ipc";
import { t, ts } from "../../lib/i18n";
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
  /** 只有 Apple Silicon 统一内存为 true；独显机器 false 并给出下面两个读数。 */
  vram_shared: boolean;
  /** 独显显存已用/总量（nvidia-smi）；共享内存机器为 null。 */
  vram_used?: number | null;
  vram_total?: number | null;
  /** 无设备名时的标题兜底（平台 + 架构），由后端给，不在前端猜。 */
  machine?: string | null;
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
    <div className="tiles gap-2">
      <Tile
        label="CPU"
        value={l.cpu_pct != null ? `${l.cpu_pct.toFixed(0)}%` : "—"}
        sub={
          [
            l.cpu_cores != null ? ts("sysmon.coresN", { n: l.cpu_cores }) : null,
            l.load ? `load ${l.load.map((n) => n.toFixed(1)).join(" / ")}` : null,
          ]
            .filter(Boolean)
            .join(" · ") || undefined
        }
      />
      <Tile
        label={t("sysmon.mem")}
        value={`${fmtSize(l.mem_used)} / ${fmtSize(l.mem_total)}`}
        sub={l.mem_available != null ? ts("sysmon.avail", { v: fmtSize(l.mem_available) }) : undefined}
        bar={pct(l.mem_used, l.mem_total)}
      />
      <Tile
        label="GPU"
        value={l.gpu_pct != null ? `${l.gpu_pct.toFixed(0)}%` : "—"}
        /* 独显机器（Windows / Linux + N 卡）显示显存已用/总量；
           Apple Silicon 统一内存才说「显存与内存共享」（vram_shared 由后端按平台给）。 */
        sub={
          l.vram_shared
            ? t("sysmon.vramShared")
            : l.vram_used != null && l.vram_total != null
              ? ts("sysmon.vram", { u: fmtSize(l.vram_used), t: fmtSize(l.vram_total) })
              : undefined
        }
        bar={pct(l.vram_used, l.vram_total)}
        title={l.gpu_name ?? l.machine ?? undefined}
      />
      <Tile
        label={t("sysmon.temp")}
        value={`${l.cpu_temp != null ? `${l.cpu_temp.toFixed(0)}°` : "—"} / ${l.gpu_temp != null ? `${l.gpu_temp.toFixed(0)}°` : "—"}`}
      />
      <Tile label={t("sysmon.power")} value={l.power_w != null ? `${l.power_w.toFixed(0)} W` : "—"} />
      <Tile
        label={t("sysmon.disk")}
        value={`${fmtSize(diskUsed)} / ${fmtSize(l.disk_total)}`}
        sub={l.disk_free != null ? ts("sysmon.avail", { v: fmtSize(l.disk_free) }) : undefined}
        bar={pct(diskUsed, l.disk_total)}
      />
    </div>
  );
}

function RemoteTiles({ r }: { r: SysRemote }) {
  if (!r.ok) {
    return (
      <p className="rounded-md border border-slate-800 bg-slate-800/40 px-2.5 py-2 text-sm text-[var(--sem-danger-text)]" title={r.error}>
        {t("sysmon.sshFail")}
        {r.error ? `${t("common.colon")}${r.error}` : ""}
      </p>
    );
  }
  return (
    <div className="tiles gap-2">
      <Tile
        label="CPU"
        value={r.cpu_cores != null ? ts("sysmon.coresN", { n: r.cpu_cores }) : "—"}
        sub={r.load ? `load ${r.load.map((n) => n.toFixed(1)).join(" / ")}` : undefined}
        title={t("sysmon.loadTitle")}
      />
      <Tile label={t("sysmon.mem")} value={`${fmtSize(r.mem_used)} / ${fmtSize(r.mem_total)}`} bar={pct(r.mem_used, r.mem_total)} />
      <Tile
        label="GPU"
        value={r.gpu_pct != null ? `${r.gpu_pct.toFixed(0)}%` : "—"}
        sub={
          r.vram_used != null || r.vram_total != null
            ? ts("sysmon.vram", { u: fmtSize(r.vram_used), t: fmtSize(r.vram_total) })
            : undefined
        }
      />
      <Tile
        label={t("sysmon.temp")}
        value={`${r.cpu_temp != null ? `${r.cpu_temp.toFixed(0)}°` : "—"} / ${r.gpu_temp != null ? `${r.gpu_temp.toFixed(0)}°` : "—"}`}
        sub="sensors Tctl / edge"
      />
      <Tile label={t("sysmon.power")} value={r.power_w != null ? `${r.power_w.toFixed(0)} W` : "—"} sub={t("sysmon.pptSub")} />
      <Tile
        label={t("sysmon.diskRoot")}
        value={r.disk_total != null ? `${fmtSize(r.disk_total - (r.disk_free ?? 0))} / ${fmtSize(r.disk_total)}` : "—"}
        sub={r.disk_free != null ? ts("sysmon.avail", { v: fmtSize(r.disk_free) }) : undefined}
        bar={pct((r.disk_total ?? 0) - (r.disk_free ?? 0), r.disk_total)}
      />
    </div>
  );
}

/**
 * 「实时更新中」标记（U10）：原先只有一颗绿点，色觉障碍与灰度截图等于没有——
 * 换成状态灯的形状（呼吸的 ◐）+ 读屏文案，与全站状态灯同一套语言。
 */
const LiveMark = () => (
  <span className="ml-1.5 inline-flex align-middle">
    <StatusDot busy text={t("sysmon.live")} />
  </span>
);

/** 「系统与引擎」：一台机器一张卡（用户 2026-11-19 拍板）+ 实时更新开关。 */
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
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-base font-semibold">{t("sysmon.title")}</h2>
        <label className="flex shrink-0 cursor-pointer items-center gap-2 text-sm text-ink-3">
          {t("sysmon.liveToggle")}
          <Toggle checked={live} onChange={setLive} />
        </label>
      </div>
      {q.isError ? (
        <Card>
          <QueryError inline title={t("sysmon.loadFail")} error={q.error} onRetry={() => q.refetch()} />
        </Card>
      ) : !s ? (
        <Card>
          <p className="py-4 text-center text-sm text-ink-4">
            {live ? t("sysmon.collecting") : t("sysmon.liveOff")}
          </p>
        </Card>
      ) : (
        // 两台机器并列（U9）：列数看这块还剩多宽，不看视口——900 的窗口收起边条就该
        // 并排，原先那一档 lg 断点只认视口，收起边条多出来的 152px 它根本不知道。
        <div className="cards-2 gap-3">
          <Card className="min-w-0">
            <h3 className="mb-2 text-sm font-medium text-ink-2" title={s.local.gpu_name ?? s.local.machine ?? t("sysmon.local")}>
              {t("sysmon.local")}{live && <LiveMark />}
              {/* 无设备名（非 mac 且无 N 卡）时退回平台名，标题不留空（用户 2026-11-16） */}
              {(s.local.gpu_name ?? s.local.machine) && (
                <span className="ml-1.5 font-normal text-ink-4">{s.local.gpu_name ?? s.local.machine}</span>
              )}
            </h3>
            <MachineTiles l={s.local} />
          </Card>
          <Card className="min-w-0">
            <h3 className="mb-2 text-sm font-medium text-ink-2" title={s.remote.hostname ?? "102"}>
              102 · halogen
              {live && s.remote.ok && <LiveMark />}
              {s.remote.hostname && <span className="ml-1.5 font-normal text-ink-4">{s.remote.hostname}</span>}
            </h3>
            <RemoteTiles r={s.remote} />
          </Card>
        </div>
      )}
    </div>
  );
}
