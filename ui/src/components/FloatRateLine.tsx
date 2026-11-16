// 单行速率线：一个代理一个窗口（标签 `rate-<app>`），各自可拖、各自记位置。
// 一条与终端状态线同高的细条，三段固定——prefill 速率 / decode 速率 /
// 已用与总上下文。一秒一跳；速率切片 11 起改吃引擎原生速率（get_engine_rates，
// 经 backend→engine 映射解析），网关字节估算展示退役——引擎没报就显示 —。
//
// 上下文在飞期间是估算（上游 usage 要等流尾才真报），所以一律带 `~`。
// 抽成独立文件是因为浮窗本体已经够挤了，这两件事本来也不该一起改。

import { useCallback, useEffect, useMemo, useRef } from "react";
import { useQuery } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ipc } from "../lib/ipc";
import { every, POLL } from "../lib/poll";
import { shortModel } from "../lib/modelprefix";
import { useZoom } from "../lib/zoom";
import {
  engineMap,
  makeNativeResolver,
  useBackendEngines,
  useEngineRates,
  type NativeRate,
} from "../lib/nativeRates";

/** 在飞交换的视图（gateway/rates.rs `inflight_json` 的前端形状）。 */
export interface Inflight {
  id: string;
  backend: string;
  model: string | null;
  entry: string;
  /** 发起方代理（网关从 socket 反查）；null = 认不出。单行速率线按它过滤。 */
  app: string | null;
  /** 已用上下文（在飞期间是估算：输入估算 + 输出估算）。 */
  used_ctx: number;
  /** 该模型的上下文窗口总量；0 = 未知。 */
  ctx_window: number;
  phase: string;
  elapsed_ms: number;
  queue_ms: number;
  prefill_ms: number | null;
  ttft_ms: number | null;
  chunks: number;
  bytes: number;
  dec_tps: number | null;
  est_input_tokens: number;
  prefill_tps: number | null;
  calibrated: boolean;
  outlier: boolean;
}

/** 毫秒 → 「8.6s」；一分钟以上换成分，避免浮窗里出现 61234ms 这种东西。 */
export function secs(ms: number | null | undefined): string {
  if (ms == null) return "—";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = ms / 1000;
  if (s < 60) return s.toFixed(1) + "s";
  return `${Math.floor(s / 60)}m${String(Math.round(s % 60)).padStart(2, "0")}s`;
}

export function rate(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return "—";
  return n >= 100 ? String(Math.round(n)) : n.toFixed(1);
}

/** 千位缩写（198200 → 198.2k），与 Rust 侧 rates::kfmt 同口径。 */
function kfmt(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1000) return (n / 1000).toFixed(1) + "k";
  return String(n);
}

/** 同时多条时按活跃度排：解码中 > prefill > 排队（单行代表、多行展开共用）。 */
const PHASE_RANK: Record<string, number> = { generating: 0, prefill: 1, queued: 2 };

function byPhase(a: Inflight, b: Inflight): number {
  return (PHASE_RANK[a.phase] ?? 9) - (PHASE_RANK[b.phase] ?? 9);
}

/**
 * 单行速率线：一条与终端状态线同高的细条，三段固定——
 * prefill 速率 / decode 速率 / 已用与总上下文。一秒一跳（在飞走 get_rates，
 * 速率走引擎原生速率 nativeFor）。上下文在飞期间是估算（上游 usage 要等流尾
 * 才真报），所以一律带 `~`。
 *
 * **多子代理并行时自动竖排**（用户口径）：多条在飞各占一行（按活跃度排序，
 * 解码中的在最上），代理名只在首行显形、其余行 invisible 占位对齐；窗口
 * 高度由 RateLineWindow 的 refit 随 rows.length 自动收。
 */
export function RateLine({
  rows,
  app,
  nativeFor,
}: {
  rows: Inflight[];
  app: string;
  nativeFor: (backend: string, model: string | null) => NativeRate | null;
}) {
  if (rows.length === 0) {
    return (
      <div className="flex items-center gap-2 whitespace-nowrap text-2xs leading-none tabular-nums">
        <span className="text-cyan-300">{app}</span>
        <span className="text-ink-4">空闲</span>
      </div>
    );
  }
  const ordered = [...rows].sort(byPhase);
  // 并发序号：同一家代理同时有几条在飞是常事（子代理、并行工具、compact…），而网关
  // 分不出谁是主谁是子（同进程、同入口、同模型），只按挂号先后（id 单调递增）编号，
  // 让并发的几行彼此分得开。只有一条时不挂——`#1` 是纯噪音。
  const seqOf = new Map(
    [...rows]
      .sort((a, b) => Number(a.id) - Number(b.id))
      .map((r, i) => [r.id, i + 1]),
  );
  const many = rows.length > 1;
  return (
    <div className="flex flex-col gap-1">
      {ordered.map((s, i) => (
        <RateLineRow
          key={s.id}
          s={s}
          app={app}
          appShown={i === 0}
          seq={many ? seqOf.get(s.id) : undefined}
          nativeFor={nativeFor}
        />
      ))}
    </div>
  );
}

function RateLineRow({
  s,
  app,
  appShown,
  seq,
  nativeFor,
}: {
  s: Inflight;
  app: string;
  appShown: boolean;
  /** 并发序号（见 RateLine）；undefined = 此刻只有一条，不挂标。 */
  seq?: number;
  nativeFor: (backend: string, model: string | null) => NativeRate | null;
}) {
  const native = nativeFor(s.backend, s.model);
  // 生成中原生差分恒 0（引擎计数器请求收尾才累计）→ 退回网关校准速率，与浮窗同源。
  const decShown =
    native?.decode != null && native.decode > 0 ? native.decode : (s.dec_tps ?? null);
  // 上下文与多行浮窗同源同算法（同一份 get_rates），只是先按代理过滤。
  // 在飞期间的 used 是估算（输入估算 + 输出估算），估超是常态；而「已用 > 窗口总量」
  // 这句话本身就是错的——真超了上游早拒了，轮不到这里显示。所以只要窗口已知就一律
  // 钳到窗口，宁可比真实值少一点，也不给出物理上不存在的数。
  const used = s.ctx_window > 0 ? Math.min(s.used_ctx, s.ctx_window) : s.used_ctx;
  return (
    <div className="flex items-center gap-2 whitespace-nowrap text-2xs leading-none tabular-nums">
      {/* 多行时非首行用 invisible 占位：列对齐不塌，代理名不重复灌满整窗。 */}
      <span className={appShown ? "text-cyan-300" : "invisible text-cyan-300"}>{app}</span>
      {/* 序号只在多条并发时给（给就每行都给，列不会塌）；单条时不估位，
          不然空闲行白留一格空位。 */}
      {seq != null && <span className="text-ink-4">#{seq}</span>}
      <span className="max-w-[16rem] truncate text-ink-3" title={s.model ?? "未知模型"}>
        {s.model ? shortModel(s.model) : "未知模型"}
      </span>
      <span
        className={s.phase === "prefill" ? "text-cyan-400" : "text-ink-4"}
        title="prefill 引擎原生速率（halogen 为引擎级聚合）"
      >
        ◐ {rate(native?.prefill)}
      </span>
      <span
        className={s.phase === "generating" ? "text-[var(--sem-ok-text)]" : "text-ink-4"}
        title="decode 速率：引擎原生优先，生成中原生恒 0 时退回网关校准值"
      >
        ▶ {rate(decShown)}
      </span>
      <span className="text-ink-2">
        ctx {kfmt(used)}
        {s.ctx_window > 0 ? `/${kfmt(s.ctx_window)}` : ""}
      </span>
    </div>
  );
}

/**
 * 一行的主体文案：相位决定说什么，别的都不说。速率优先引擎原生值（切片 11）：
 * native 由 makeNativeResolver 解析，字段为 null = 引擎没报，显示 —。
 * 例外：llama.cpp 口径的计数器在请求收尾才累计，生成中原生差分恒 0——块数在涨
 * 就是 token 在动，此时 0.0 是误导，退回网关校准速率（带 ~ 标估算口径）。
 */
export function headline(
  s: Inflight,
  native: NativeRate | null,
): { icon: string; text: string } {
  if (s.phase === "queued") return { icon: "⏸", text: `排队 ${secs(s.elapsed_ms)}` };
  if (s.phase === "prefill") {
    const speed = ` · ${rate(native?.prefill)} tok/s`;
    const q = s.queue_ms > 1 ? ` · 排队 ${secs(s.queue_ms)}` : "";
    return { icon: "◐", text: `prefill ${secs(s.elapsed_ms)}${speed}${q}` };
  }
  const dec =
    native?.decode != null && native.decode > 0
      ? `dec ${rate(native.decode)} t/s`
      : s.dec_tps != null && s.dec_tps > 0
        ? `dec ~${rate(s.dec_tps)} t/s`
        : "dec —";
  return {
    icon: "▶",
    text: `${secs(s.elapsed_ms)} · ${dec} · ${s.chunks.toLocaleString()} 块`,
  };
}


interface RatesSnapshot {
  inflight: Inflight[];
}

/**
 * 速率线的窗口外壳：只吃本代理的在飞请求，把窗口收到内容大小，
 * 整条可拖（按下不动就不开拖，理由同浮窗——原生拖拽会吞掉双击的第二下），
 * 双击或 Esc 关掉自己。位置由 Rust 侧记在 settings，拖到哪下次还在哪。
 */
export function RateLineWindow({ app }: { app: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const lastPress = useRef(0);
  const armed = useRef<{ x: number; y: number; moved: boolean } | null>(null);

  const rates = useQuery<RatesSnapshot>({
    queryKey: ["rateline-rates"],
    queryFn: () => ipc<RatesSnapshot>("get_rates"),
    refetchInterval: () => every(POLL.live),
  });
  // 引擎原生速率（切片 11）：与浮窗/监控页同源，后端 TTL 合流不额外加 SSH。
  const er = useEngineRates(POLL.live);
  const engines = useBackendEngines();
  const nativeFor = useMemo(
    () => makeNativeResolver(er.data, engineMap(engines.data)),
    [er.data, engines.data],
  );
  const rows = (rates.data?.inflight ?? []).filter((r) => r.app === app);
  const busy = rows.length > 0;

  const refit = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    if (r.width < 8 || r.height < 8) return;
    void ipc("rateline_fit", {
      width: Math.ceil(r.width) + 2,
      height: Math.ceil(r.height) + 2,
    }).catch(() => {});
  }, []);

  // 缩放与浮窗同一套（Cmd+滚轮 / Cmd±0），倍率按代理各存各的：
  // 大终端旁边那条放大些、小窗旁边那条缩着，互不牵连。
  useZoom(`rate-${app}`, () => refit());

  // 内容宽度随模型名长度变，量完就收一次窗口。
  useEffect(() => {
    refit();
  }, [refit, app, rows.length, busy]);

  const close = useCallback(() => {
    void ipc("rateline_set", { target: app, on: false }).catch(() => {});
  }, [app]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [close]);

  const press = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const now = performance.now();
    if (e.detail >= 2 || now - lastPress.current < 420) {
      lastPress.current = 0;
      armed.current = null;
      close();
      return;
    }
    lastPress.current = now;
    armed.current = { x: e.clientX, y: e.clientY, moved: false };
  };

  const drift = (e: React.PointerEvent) => {
    const a = armed.current;
    if (!a || a.moved) return;
    if (Math.hypot(e.clientX - a.x, e.clientY - a.y) < 4) return;
    a.moved = true;
    lastPress.current = 0;
    void getCurrentWindow()
      .startDragging()
      .catch(() => {});
  };

  return (
    <div
      ref={ref}
      className={`float-line select-none ${busy ? "" : "float-idle"}`}
      onPointerDown={press}
      onPointerMove={drift}
      onPointerUp={() => {
        armed.current = null;
      }}
      onPointerCancel={() => {
        armed.current = null;
      }}
      title={`llm-switch 速率线 · ${app}：拖动可移动，双击或 Esc 关闭`}
    >
      <RateLine rows={rows} app={app} nativeFor={nativeFor} />
    </div>
  );
}
