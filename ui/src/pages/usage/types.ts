// 用量页共用的后端返回结构（计划 §6.1 自 Usage.tsx 拆出）。
export interface UsageRec {
  request_id: string;
  ts: number;
  backend: string | null;
  model: string | null;
  entry_protocol: string | null;
  client: string | null;
  status: number;
  ok: boolean;
  stream: boolean;
  ttft_ms: number | null;
  duration_ms: number | null;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  decode_tps: number | null;
  estimated: boolean;
  cache_status: string | null;
  retry_trace: string | null;
  error: string | null;
}

export interface DailyRow {
  day: string;
  requests: number;
  errors: number;
  input_tokens: number;
  output_tokens: number;
  avg_ttft_ms: number | null;
}

export interface Stats {
  ttft_p50_ms: number | null;
  ttft_p95_ms: number | null;
  cache: { hit: number; miss: number; bypass: number; hit_rate: number };
  by_client: { client: string; requests: number; input_tokens: number; output_tokens: number }[];
  series: { backend: SeriesRow[]; model: SeriesRow[]; client: SeriesRow[] };
}
export interface SeriesRow {
  day: string;
  key: string;
  requests: number;
  tokens: number;
}

export interface Cost {
  total_cost_usd: number;
  local_requests: number;
  online_requests: number;
  priced_requests: number;
  unpriced_requests: number;
  by_model: {
    model: string;
    /** local = 本地引擎不计费；online = 在线（手工单价，未设 = 不算）。 */
    scope: "local" | "online";
    requests: number;
    input_tokens: number;
    output_tokens: number;
    cache_read_tokens: number;
    cost_usd: number;
    priced: boolean;
  }[];
}
