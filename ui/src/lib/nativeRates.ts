// 引擎原生速率（切片 11）：监控页与浮窗共用的前端解析层。
//
// 数据源只有一个：`get_engine_rates`（halogen 指标接口计数器差分，
// 后端 TTL 2s 合流，多窗口共享一次 SSH）。网关的字节估算速率展示
// 已按主计划 §9 退役——这里解析不出原生值就返回 null，UI 显示 —，
// 拿估算补位是红线。
import { useQuery } from "@tanstack/react-query";
import { ipc, type BackendRow } from "./ipc";
import { every, POLL } from "./poll";

/** halogen 视图（services/engine_rates.rs `halogen_view` 的前端形状）。 */
export interface HalogenNative {
  ok: boolean;
  error?: string;
  running?: boolean;
  prefill_tps?: number | null;
  decode_tps?: number | null;
  processing?: number | null;
  deferred?: number | null;
  kv_ratio?: number | null;
  kv_tokens?: number | null;
}

export interface EngineRates {
  halogen: HalogenNative;
}

/** 一条交换的原生速率；null 字段 = 引擎没报（显示 —）。 */
export interface NativeRate {
  prefill: number | null;
  decode: number | null;
}

/** 引擎原生速率查询（监控页 3s / 浮窗 1s，后端 TTL 合流不打穿引擎）。 */
export function useEngineRates(ms: number = POLL.fast) {
  return useQuery({
    queryKey: ["engine-rates"],
    queryFn: () => ipc<EngineRates>("get_engine_rates"),
    refetchInterval: () => every(ms),
  });
}

/** 后端名 → 引擎名映射（backend.engine 认识的引擎才参与解析）。 */
export function useBackendEngines() {
  return useQuery({
    queryKey: ["backends"],
    queryFn: () => ipc<BackendRow[]>("list_backends"),
    refetchInterval: () => every(POLL.normal),
  });
}

export function engineMap(rows: BackendRow[] | undefined): Map<string, string> {
  return new Map(
    (rows ?? [])
      .filter((b) => b.engine)
      .map((b) => [b.name, b.engine as string]),
  );
}

/**
 * 解析器：在飞交换 (backend, model) → 引擎原生速率。
 * - halogen：引擎级聚合速率（全引擎一份，多流并发时各行显示同一聚合值——
 *   这是指标接口的真实粒度，不装出逐流精度）。
 * 认不出引擎 / 引擎没报 → null，UI 显示 —。
 */
export function makeNativeResolver(
  er: EngineRates | null | undefined,
  engines: Map<string, string>,
): (backend: string, model: string | null) => NativeRate | null {
  return (backend) => {
    if (!er) return null;
    switch (engines.get(backend)) {
      case "halogen": {
        const h = er.halogen;
        return h.ok
          ? { prefill: h.prefill_tps ?? null, decode: h.decode_tps ?? null }
          : null;
      }
      default:
        return null;
    }
  };
}

/** tok/s 展示：整数千分位；null → —。 */
export function fmtTps(n: number | null | undefined): string {
  return n == null ? "—" : Math.round(n).toLocaleString();
}
