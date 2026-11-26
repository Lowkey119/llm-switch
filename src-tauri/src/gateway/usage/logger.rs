//! Thin writer turning a finished (or failed) proxy exchange into a
//! `UsageRecord` row. Deliberately dumb: pricing/costs land in a later slice.

use super::parser::TokenUsage;
use crate::store::Store;
use crate::types::{now_ms, UsageRecord};
use uuid::Uuid;

/// Everything a completed exchange contributes to the usage ledger.
#[derive(Debug, Clone, Default)]
pub struct UsageContext {
    pub backend: String,
    pub model: String,
    pub entry_protocol: String,
    pub stream: bool,
    pub status: i64,
    pub ok: bool,
    pub ttft_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    /// JSON array of prior failed attempts (retry trajectory).
    pub retry_trace: Option<String>,
    /// Measured decode speed (stream: out_tokens / (last−first content)).
    pub decode_tps: Option<f64>,
    /// 实测 decode 跨度毫秒（首字→末字，与 `decode_tps` 同源）；
    /// 「按模型 tok/s」的分母（None = 没锚点，不入成对样本）。
    pub decode_span_ms: Option<u64>,
    /// Tokens are estimates (upstream reported no usage).
    pub estimated: bool,
    /// Estimated output tokens when `usage` is absent (paired with `estimated`).
    pub estimated_output: Option<i64>,
    /// 本请求输入 token 的字节启发式估算（M12-C1；0 = 未知，也是 prefill 速率的兜底分子）。
    pub est_input_tokens: u64,
    /// 纠偏键：挂号槽看到的模型名（与浮窗行同源）；None = 无槽（如缓存回放），退回 `model`。
    pub rate_model: Option<String>,
    /// 请求块字节（prefill 纠偏样本的分母；0 = 没测量，不喂样）。
    pub req_bytes: u64,
    /// 响应流字节（decode 纠偏样本的分母；非流式恒 0 —— 封装不同不与流式混池）。
    pub resp_bytes: u64,
    /// 封口 prefill 毫秒（发送起始 → decode 首字）；None = 没锚点，不记速率样本。
    pub prefill_ms: Option<u64>,
    /// 响应缓存口径（M5.5）：`hit` | `miss` | `bypass:<reason>`；未启用时为 None。
    pub cache_status: Option<String>,
    /// 发起方代理名（`origin` 从 socket 反查）；认不出为 None。
    pub client: Option<String>,
    /// llama.cpp 引擎自测计时（timings 块）；None = 上游不是 llama.cpp 系或没带。
    /// 喂引擎真值速率环（并行参照系，不改用户敲定的线口径公式）。
    pub timings: Option<super::parser::EngineTimings>,
    /// 媒体计量（11c）：生成图像张数；None = 非媒体行或取不到（UI 显示 —）。
    pub media_images: Option<i64>,
    /// 媒体计量（11c）：音频秒数；None = 非媒体行或取不到（—）。
    pub media_seconds: Option<f64>,
}

/// Persist one exchange. `usage` may be `None` when the upstream never
/// reported tokens (failed calls, usage-less streams) — the row is still
/// written so request counts stay honest.
pub fn record(
    store: &Store,
    metrics: Option<&crate::gateway::metrics::Metrics>,
    ctx: &UsageContext,
    usage: Option<&TokenUsage>,
) {
    // 速率聚合与用量落库同一漏斗：一处喂数，两处口径天然一致。
    if let Some(m) = metrics {
        let out_tokens = usage.map(|u| u.output_tokens as u64).unwrap_or(0);
        m.on_completion(ctx.ok, ctx.ttft_ms, out_tokens);
        // 按模型的第二套桶（M12-B）：同一处喂数，两处口径天然一致。
        // decode 跨度随行：模型 tok/s 用 Σtoken ÷ Σ跨度算，除以桶数会把
        // 「刚起步只有 1 个桶」的第一次完成放大成整个请求的输出量。
        // 键必须用挂号别名（与在飞槽、纠偏键同源）：ctx.model 是上游回报的真实 id，
        // 用它记账会让同一模型在流量卡上裂成「在飞行=别名 + 完成行=上游id」两行。
        let key = ctx.rate_model.as_deref().unwrap_or(&ctx.model);
        m.on_model_completion(
            key,
            &ctx.backend,
            ctx.ok,
            ctx.ttft_ms,
            out_tokens,
            ctx.decode_span_ms,
        );
        // 纠偏样本（**分模型、分后端**）：真报 token 与实测字节成对喂样，
        // 离谱比值与零值由 note_* 内部丢弃，这里只管送对键、送对字节。
        if ctx.ok {
            // prefill：上游真报输入 ÷ 请求块字节。
            if ctx.req_bytes > 0 {
                if let Some(u) = usage.filter(|u| u.input_tokens > 0) {
                    m.note_pre_coeff(&ctx.backend, key, ctx.req_bytes, u.input_tokens as u64);
                }
            }
            // decode：上游真报输出 ÷ 响应流字节（只有流式才有响应流测量）。
            if ctx.resp_bytes > 0 {
                if let Some(u) = usage.filter(|u| u.output_tokens > 0) {
                    m.note_dec_coeff(&ctx.backend, key, ctx.resp_bytes, u.output_tokens as u64);
                }
            }
            // 封口 prefill 速率样本：同一公式回放（有系数乘系数，否则用估算先验）。
            if let Some(ms) = ctx.prefill_ms.filter(|ms| *ms > 0) {
                if ctx.req_bytes > 0 {
                    let coeff = m.pre_coeff(&ctx.backend, key);
                    let tps = crate::gateway::rates::prefill_rate(
                        ctx.req_bytes,
                        ctx.est_input_tokens,
                        ms,
                        coeff,
                    );
                    m.note_prefill_rate(&ctx.backend, key, tps);
                }
            }
            // 引擎真值速率（llama.cpp timings）：与系数派生速率并排的参照系，
            // 只喂环、不改上面任何线口径公式。
            if let Some(t) = ctx.timings {
                m.note_engine_rates(&ctx.backend, key, t.prefill_tps(), t.decode_tps());
            }
        }
    }
    let request_id = usage
        .and_then(|u| u.message_id.as_ref())
        .map(|mid| format!("session:{mid}"))
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let row = UsageRecord {
        request_id,
        ts: now_ms(),
        backend: Some(ctx.backend.clone()),
        model: Some(
            usage
                .and_then(|u| u.model.as_deref())
                .unwrap_or(&ctx.model)
                .to_string(),
        ),
        entry_protocol: Some(ctx.entry_protocol.clone()),
        client: ctx.client.clone(),
        status: ctx.status,
        ok: ctx.ok,
        stream: ctx.stream,
        ttft_ms: ctx.ttft_ms,
        duration_ms: ctx.duration_ms,
        input_tokens: usage.map(|u| u.input_tokens as i64).unwrap_or(0),
        output_tokens: usage
            .map(|u| u.output_tokens as i64)
            .or(ctx.estimated_output)
            .unwrap_or(0),
        cache_read_tokens: usage.map(|u| u.cache_read_tokens as i64).unwrap_or(0),
        cache_creation_tokens: usage.map(|u| u.cache_creation_tokens as i64).unwrap_or(0),
        decode_tps: ctx.decode_tps,
        estimated: ctx.estimated,
        cache_status: ctx.cache_status.clone(),
        retry_trace: ctx.retry_trace.clone(),
        error: ctx.error.clone(),
        media_images: ctx.media_images,
        media_seconds: ctx.media_seconds,
    };

    if let Err(e) = store.record_usage(&row) {
        tracing::warn!(target: "llm_switch::usage", error = %e, "failed to record usage row");
    }
}
