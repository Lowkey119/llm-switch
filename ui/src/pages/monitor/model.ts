// 监控页共用的后端/事件结构与相位口径（计划 §6.1 自 Monitor.tsx 拆出）。
import { t } from "../../lib/i18n";

export interface SecondPoint {
  sec: number;
  reqs: number;
  errs: number;
  out_tokens: number;
}

export interface MetricsSnapshot {
  window_secs: number;
  rps: number;
  errors_per_s: number;
  tokens_per_s: number;
  error_rate: number;
  ttft_p50_ms: number | null;
  ttft_p95_ms: number | null;
  ewma_rps: number;
  ewma_tokens_per_s: number;
  active_streams: number;
  series: SecondPoint[];
  cache?: { hits: number; misses: number; hit_rate: number; saved_input_tokens: number; saved_output_tokens: number };
}

export interface KvRow {
  backend: string;
  engine: string | null;
  kv_usage: number | null;
  prefix_hit_rate: number | null;
  waiting: number | null;
  has_ssd_kv: boolean;
  ssd_hit_rate: number | null;
  age_ms: number;
}

export interface ModelRate {
  model: string;
  backends: string[];
  reqs: number;
  errs: number;
  rps: number;
  tokens_per_s: number;
  errors_per_s: number;
  out_tokens: number;
  ttft_p50_ms: number | null;
  /** 近期首响样本数（与 P50 一起标——样本少时分位数不可信）。 */
  ttft_samples: number;
  active_streams: number;
  active: boolean;
  series: SecondPoint[];
  /** 曲线窗口（秒）——后端 MODEL_WINDOW_SECS 下发，UI 标注免写死。 */
  window_secs: number;
}

/// 在飞交换（与后端 rates 出口同形：浮条与本页吃同一份数据）。
export interface InflightRow {
  id: number;
  backend: string;
  model: string;
  entry: string;
  /** 发起方代理（lsof 反查，认不出为 null）：速率线归属就按它分。 */
  app: string | null;
  /** 发起方进程号：归属出问题时拿它对 `ps`，不用猜。 */
  pid: number | null;
  phase: "queued" | "prefill" | "generating";
  elapsed_ms: number;
  queue_ms: number;
  prefill_ms: number;
  ttft_ms: number | null;
  chunks: number;
  bytes: number;
  dec_tps: number;
  est_input_tokens: number;
  prefill_tps: number;
  calibrated: boolean;
  outlier: boolean;
}

export interface ModelRow {
  model: string;
  backends: string[];
  hidden?: boolean;
  metric?: ModelRate;
}

// 相位标签存 i18n key：模块级常量在加载时定不了语言（lang 还是默认值），渲染时取词。
export const PHASE: Record<InflightRow["phase"], { labelKey: string; cls: string }> = {
  queued: { labelKey: "mon.phase.queued", cls: "bg-slate-700/60 text-ink-2" },
  prefill: { labelKey: "mon.phase.prefill", cls: "bg-amber-500/15 text-[var(--sem-busy-text)]" },
  generating: { labelKey: "mon.phase.generating", cls: "bg-emerald-500/15 text-[var(--sem-ok-text)]" },
};

/** 相位计数（拍板：全局速率卡撤掉后，「在飞」信息落到每模型行与页脚两处）。 */
export type PhaseCounts = Partial<Record<InflightRow["phase"], number>>;
export const PHASE_ORDER: InflightRow["phase"][] = ["queued", "prefill", "generating"];

/** 计数 → 展示段：只列非零，顺序固定（排队 → prefill → 生成中），如 `["生成中 1"]`。 */
export function phaseSegs(counts: PhaseCounts | undefined): string[] {
  if (!counts) return [];
  return PHASE_ORDER.filter((p) => (counts[p] ?? 0) > 0).map(
    (p) => `${t(PHASE[p].labelKey)} ${counts[p]}`,
  );
}

/** 大字节数可读化（磁盘/内存/显存，B→PB）；null → —（编数是红线）。 */
export function fmtSize(n: number | null | undefined): string {
  if (n == null) return "—";
  const u = ["B", "KB", "MB", "GB", "TB", "PB"];
  const i = Math.min(Math.floor(Math.log(Math.max(n, 1)) / Math.log(1024)), u.length - 1);
  return `${(n / 1024 ** i).toFixed(i ? 1 : 0)} ${u[i]}`;
}

/** 字节数可读化：监控表里的请求/响应体积。 */
export function fmtBytes(n: number): string {
  if (n < 1024) return `${n}B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${(n / 1024 / 1024).toFixed(2)}MB`;
}
