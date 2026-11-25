// 资源悬浮小窗（切片 13，主计划 §10①）：置顶、无边框、可拖可钉的实时监控。
// 展示借鉴 unsloth 悬浮监控窗——只借鉴思路，未复制任何代码（AGPL 红线）。
//
// 数据与监控页同源：get_sys_stats（sysmon TTL 合流，多窗口共享一次 SSH）。
// 红线照旧：字段 null = 拿不到，一律显示 —，绝不编数。
// 窗口几何（位置/尺寸/钉住/显隐偏好）全在 gui/sysfloat.rs，这里只画内容、
// 量内容回报 fit、整窗拖拽（起手 >4px 才开原生拖，macOS startDragging 吞
// mouseUp 的坑 FloatPanel 已踩过，这里没有双击所以简单些）。
import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ipc } from "../lib/ipc";
import { t, ts } from "../lib/i18n";
import { IconClose } from "./icons";
import { every, POLL } from "../lib/poll";
import { useZoom } from "../lib/zoom";
import { fmtSize } from "../pages/monitor/model";
import type { SysLocal, SysRemote } from "../pages/monitor/SysMonCards";

/** 可显隐的指标（设置页勾选与小窗渲染共用这一份清单）。 */
export const METRIC_KEYS = [
  { key: "cpu", labelKey: "set.metric.cpu" },
  { key: "mem", labelKey: "set.metric.mem" },
  { key: "vram", labelKey: "set.metric.vram" },
  { key: "gpu", labelKey: "set.metric.gpu" },
  { key: "temp", labelKey: "set.metric.temp" },
  { key: "power", labelKey: "set.metric.power" },
] as const;

export type MetricKey = (typeof METRIC_KEYS)[number]["key"];

interface SysStats {
  local: SysLocal;
  remote: SysRemote;
}

function pct(used: number | null | undefined, total: number | null | undefined): number | null {
  if (used == null || total == null || total <= 0) return null;
  return Math.min(100, Math.max(0, (used / total) * 100));
}

/** 一行指标：标签 + 值（+ 可选细进度条）。 */
function Row({ label, value, bar }: { label: string; value: string; bar?: number | null }) {
  return (
    <div className="flex items-center gap-1.5 leading-tight">
      <span className="w-10 shrink-0 text-2xs text-ink-4">{label}</span>
      <span className="min-w-0 flex-1 truncate font-mono text-2xs text-ink-2">{value}</span>
      {bar != null && (
        <span className="block h-1 w-10 shrink-0 overflow-hidden rounded bg-slate-700/60">
          <span className="block h-full rounded bg-cyan-500/80" style={{ width: `${bar}%` }} />
        </span>
      )}
    </div>
  );
}

/** 一台机器的指标行（勾选显隐在这里生效；缺值一律 —）。 */
function MachineRows({ m, show, remote }: { m: SysLocal | SysRemote; show: Set<string>; remote: boolean }) {
  const bad = !m.ok;
  const rows: { key: MetricKey; el: React.ReactNode }[] = [];
  if (!bad) {
    if (show.has("cpu")) {
      const load = m.load ? m.load.map((n) => n.toFixed(1)).join(" ") : "—";
      const l = m as SysLocal;
      rows.push({
        key: "cpu",
        el: (
          <Row
            label="CPU"
            value={
              !remote && l.cpu_pct != null
                ? `${l.cpu_pct.toFixed(0)}% · ${load}`
                : `load ${load}${m.cpu_cores != null ? ` · ${ts("sf.cores", { n: m.cpu_cores })}` : ""}`
            }
          />
        ),
      });
    }
    if (show.has("mem")) {
      rows.push({
        key: "mem",
        el: (
          <Row
            label={t("sf.mem")}
            value={`${fmtSize(m.mem_used)} / ${fmtSize(m.mem_total)}`}
            bar={pct(m.mem_used, m.mem_total)}
          />
        ),
      });
    }
    if (show.has("vram")) {
      const l = m as SysLocal;
      const r = m as SysRemote;
      rows.push({
        key: "vram",
        el: remote ? (
          <Row
            label="VRAM"
            value={`${fmtSize(r.vram_used)} / ${fmtSize(r.vram_total)}`}
            bar={pct(r.vram_used, r.vram_total)}
          />
        ) : (
          <Row
            label="VRAM"
            value={
              l.vram_shared
                ? ts("sf.sharedMem", { v: fmtSize(l.mem_available) })
                : l.vram_used != null && l.vram_total != null
                  ? `${fmtSize(l.vram_used)} / ${fmtSize(l.vram_total)}`
                  : "—"
            }
          />
        ),
      });
    }
    if (show.has("gpu")) {
      const l = m as SysLocal;
      rows.push({
        key: "gpu",
        el: (
          <Row
            label="GPU"
            value={`${m.gpu_pct != null ? `${m.gpu_pct.toFixed(0)}%` : "—"}${!remote && l.gpu_name ? ` · ${l.gpu_name}` : ""}`}
          />
        ),
      });
    }
    if (show.has("temp")) {
      rows.push({
        key: "temp",
        el: (
          <Row
            label={t("sf.temp")}
            value={`${m.cpu_temp != null ? `${m.cpu_temp.toFixed(0)}°` : "—"} / ${m.gpu_temp != null ? `${m.gpu_temp.toFixed(0)}°` : "—"}`}
          />
        ),
      });
    }
    if (show.has("power")) {
      rows.push({
        key: "power",
        el: <Row label={t("sf.power")} value={m.power_w != null ? `${m.power_w.toFixed(0)} W` : "—"} />,
      });
    }
  }
  return (
    <div className="space-y-0.5">
      {rows.map((r) => (
        <div key={r.key}>{r.el}</div>
      ))}
      {bad && (
        <div className="text-2xs text-[var(--sem-danger-text)]" title={(m as SysRemote).error}>
          {t("sf.collectFail")}
        </div>
      )}
    </div>
  );
}

export default function SysFloatWindow() {
  const rootRef = useRef<HTMLDivElement>(null);
  const armed = useRef<{ x: number; y: number; moved: boolean } | null>(null);
  // 圆钮态：拖到屏幕左右边收成 54px 圆钮（浮窗同款手感），点圆钮展开。
  const [orb, setOrb] = useState(false);
  const orbRef = useRef(false);
  orbRef.current = orb;

  useEffect(() => {
    void ipc<{ orb: boolean }>("sysfloat_shape_get")
      .then((s) => setOrb(Boolean(s.orb)))
      .catch(() => {});
    const un = listen<{ orb: boolean }>("sysfloat://shape", (e) => setOrb(Boolean(e.payload.orb)));
    return () => {
      void un.then((f) => f());
    };
  }, []);

  const stats = useQuery({
    queryKey: ["sys-stats"],
    queryFn: () => ipc<SysStats>("get_sys_stats"),
    refetchInterval: () => every(POLL.fast),
  });
  const metrics = useQuery({
    queryKey: ["sysfloat-metrics"],
    queryFn: () => ipc<string[]>("sysfloat_metrics_get"),
  });
  const pin = useQuery({
    queryKey: ["sysfloat-pin"],
    queryFn: () => ipc<boolean>("sysfloat_pin_get"),
  });

  // 量内容收窗口（左上角不动）；ResizeObserver 让勾选变化/数据变宽都跟得上。
  const refit = useCallback(() => {
    const el = rootRef.current;
    if (!el || orbRef.current) return;
    const r = el.getBoundingClientRect();
    if (r.width < 8 || r.height < 8) return;
    void ipc("sysfloat_fit", {
      width: Math.ceil(r.width) + 2,
      height: Math.ceil(r.height) + 2,
    }).catch(() => {});
  }, []);
  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => refit());
    ro.observe(el);
    return () => ro.disconnect();
  }, [refit]);

  // 缩放（Cmd+滚轮 / Cmd±0，与速率线同款）：zoom 改的是 CSS 倍率，getBoundingClientRect
  // 已含倍率，refit 回报的就是缩放后的真实尺寸；Rust 侧 fit 落库后还会重做一次贴边吸附。
  useZoom("sysfloat", () => refit());

  // Esc 收起（与多行浮窗同手感）。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void ipc("sysfloat_set_visible", { visible: false }).catch(() => {});
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // 整窗拖拽：起手 >4px 才交给原生拖（FloatPanel 同款约束）。
  const press = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    armed.current = { x: e.clientX, y: e.clientY, moved: false };
  };
  const drift = (e: React.PointerEvent) => {
    const a = armed.current;
    if (!a || a.moved) return;
    if (Math.hypot(e.clientX - a.x, e.clientY - a.y) < 4) return;
    a.moved = true;
    void getCurrentWindow()
      .startDragging()
      .catch(() => {});
  };

  // 圆钮态：54px 圆钮（.float-orb 与浮窗/速率线同款，跟主题走）。
  // 没拖动就松手 = 点击 → 展开回面板；拖动照常整窗拖。
  if (orb) {
    return (
      <div
        className="float-orb select-none text-2xs font-medium"
        title={t("sf.orbTip")}
        onPointerDown={press}
        onPointerMove={drift}
        onPointerUp={() => {
          const a = armed.current;
          armed.current = null;
          if (a && !a.moved) void ipc("sysfloat_expand").catch(() => {});
        }}
        onPointerCancel={() => {
          armed.current = null;
        }}
      >
        {t("sf.orbChar")}
      </div>
    );
  }

  const show = new Set(metrics.data ?? []);
  const s = stats.data;
  return (
    <div
      ref={rootRef}
      className="sysfloat-card select-none"
      onPointerDown={press}
      onPointerMove={drift}
      onPointerUp={() => {
        armed.current = null;
      }}
      onPointerCancel={() => {
        armed.current = null;
      }}
    >
      <div className="mb-1 flex items-center justify-between text-2xs leading-none text-ink-3">
        <span className="tracking-wide">{t("sf.title")}</span>
        <span className="flex items-center gap-1.5">
          <button
            title={pin.data ? t("sf.pinnedTip") : t("sf.pinTip")}
            className={`px-1 ${pin.data ? "text-cyan-300" : "text-ink-4 hover:text-ink-2"}`}
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() =>
              void ipc("sysfloat_pin_set", { on: !pin.data })
                .then(() => pin.refetch())
                .catch(() => {})
            }
          >
            {t("sf.pin")}
          </button>
          <button
            title={t("sf.collapseTip")}
            aria-label={t("sf.collapseTip")}
            className="grid h-6 w-6 place-items-center rounded text-ink-4 hover:text-ink-2"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={() => void ipc("sysfloat_set_visible", { visible: false }).catch(() => {})}
          >
            <IconClose className="h-3 w-3" />
          </button>
        </span>
      </div>
      {!s ? (
        <div className="py-3 text-center text-2xs text-ink-4">
          {stats.isError ? t("sf.noStats") : t("sf.collecting")}
        </div>
      ) : (
        <div className="space-y-1.5">
          <div>
            <div
              className="mb-0.5 truncate text-2xs font-medium text-ink-2"
              title={s.local.gpu_name ?? s.local.machine ?? t("sf.local")}
            >
              {t("sf.local")}{(s.local.gpu_name ?? s.local.machine) ? ` · ${s.local.gpu_name ?? s.local.machine}` : ""}
            </div>
            <MachineRows m={s.local} show={show} remote={false} />
          </div>
          <div>
            <div className="mb-0.5 truncate text-2xs font-medium text-ink-2" title={s.remote.hostname ?? "102"}>
              102 · halogen{s.remote.hostname ? ` · ${s.remote.hostname}` : ""}
            </div>
            <MachineRows m={s.remote} show={show} remote />
          </div>
        </div>
      )}
    </div>
  );
}
