//! 浮条/浮窗的统一出口（M12-C1）：把「此刻在飞的交换」翻译成能直接印的速率。
//!
//! 口径（用户敲定的新口径，**分模型、分后端**）：
//! - **prefill = 请求块内容长度 ÷（decode 首字送达 − 发送起始）× 纠偏系数**；
//! - **decode = 响应流总字节 ÷（块尾送达 − decode 首字）× 纠偏系数**；
//! - 纠偏系数按 (backend, model) 各算各的（[`crate::gateway::metrics::Metrics::pre_coeff`] /
//!   `dec_coeff`，样本 ≥3 才给）——同一模型多后端提供时 tokenizer 与封装都不同，混算不准；
//! - 没系数时 prefill 退回字节启发式估算、decode 退回 SSE 事件到达率（tokbar 旧口径），
//!   即「系数缺位时的先验」，公式形状不变；
//! - 实时流的「块尾」取此刻（与 tokbar 实时口径一致）；封口后由 usage 记录回放同一公式；
//! - 超过该 (backend, model) 历史 prefill P99 三倍 → `outlier=true`，浮条标 `?!` 不抹平；
//! - **已用上下文**在飞期间是估算（输入估算 + 输出估算），封口后由 usage 真值接管。

use crate::gateway::State;
use serde_json::{json, Value};

/// 离群倍数：历史 P99 的三倍以内算正常波动。
const OUTLIER_FACTOR: f64 = 3.0;

/// prefill 实时速率：`请求块字节 × 纠偏系数 ÷ prefill 毫秒`。
/// 没系数时分子退回估算输入（字节启发式，≈ 校准前的老口径）；0 毫秒防除零。
pub fn prefill_rate(req_bytes: u64, est_input: u64, prefill_ms: u64, coeff: Option<f64>) -> f64 {
    if prefill_ms == 0 {
        return 0.0;
    }
    let num = coeff
        .map(|k| req_bytes as f64 * k)
        .unwrap_or(est_input as f64);
    num * 1000.0 / prefill_ms as f64
}

/// decode 实时速率：`响应流字节 × 纠偏系数 ÷ 解码毫秒（块尾 − 首字）`。
/// 没系数时分子退回 SSE 事件数（tokbar 旧口径 ≈ 一事件一 token）；0 毫秒防除零。
pub fn dec_rate(bytes: u64, events: u64, gen_ms: u64, coeff: Option<f64>) -> f64 {
    if gen_ms == 0 {
        return 0.0;
    }
    let num = coeff.map(|k| bytes as f64 * k).unwrap_or(events as f64);
    num * 1000.0 / gen_ms as f64
}

/// decode 毫秒 = 此刻 − decode 首字（实时流的「块尾」= 此刻）。
fn gen_ms(elapsed_ms: u64, ttft_ms: Option<u64>) -> u64 {
    ttft_ms.map(|t| elapsed_ms.saturating_sub(t)).unwrap_or(0)
}

/// 已用上下文（在飞口径）：估算输入 + 已产出的估算输出。
///
/// 上游的 usage 要等流尾才到，在飞期间只能估，而**两段都优先吃纠偏系数**：
/// 输入 = 请求块字节 × pre 系数，输出 = 响应流字节 × dec 系数，与 prefill /
/// decode 速率的分子完全同源。只有系数没攒够时才退回先验（字节/4 的估算输入、
/// 「一个 SSE 事件约一个 token」的输出）——字节启发式对 JSON 重载荷能虚高两三成，
/// 直接拿去跟上下文窗口比就会画出「已用 > 总量」这种看着像 bug 的数。
/// 真值一到，封口那行的 usage 自然接管。
///
/// BUG-8（用户 2026-11-16）：系数估超窗口时（缓存重载荷下 pre 系数会系统性偏高），
/// 退回更保守的先验再比一次——两个都超才由 [`clamp_ctx`] 兜底钳住。
/// 否则浮条会常年画出 `262.1k/262.1k` 的「满窗」，看着就像已用=总量是 bug。
///
/// BUG-8 再修（用户 2026-11-16 复发截图）：先验不再只用字节启发式——
/// claude 系整会话重发的巨型 JSON 载荷下，`est_input`（字节/4）同样系统性虚高，
/// 先验也超窗，clamp 只能画出假满窗。`real_in`（上一请求上游**真报**的输入
/// token，来自 metrics）是最可信的先验：相邻轮次输入几乎相同。三级取最小：
/// 系数估 → 真报先验 → 字节启发式先验。
pub fn used_ctx(
    req_bytes: u64,
    est_input: u64,
    bytes: u64,
    events: u64,
    pre: Option<f64>,
    dec: Option<f64>,
    window: i64,
    real_in: Option<u64>,
) -> u64 {
    let input = pre
        .map(|k| (req_bytes as f64 * k) as u64)
        .unwrap_or(est_input);
    let out = dec.map(|k| (bytes as f64 * k) as u64).unwrap_or(events);
    let est = input + out;
    // 先验：真报优先（上一请求上游 usage），没有才退回字节启发式 + 事件数。
    let prior = real_in.unwrap_or(est_input) + events;
    // 系数估超窗时逐级退保守值：先验（含真报）也超窗才交给 clamp 兜底。
    if window > 0 && est > window as u64 {
        prior.min(est)
    } else {
        est
    }
}

/// 已用上下文钳到窗口：`used > 窗口总量` 这句话本身就不成立——真超了上游早拒了，
/// 轮不到我们显示。在飞期间两段都是估算，估超是常态，所以窗口已知就一律钳住，
/// 宁可比真实值少一点，也不给出物理上不存在的数。窗口未知（0）才原样给。
pub fn clamp_ctx(used: u64, window: i64) -> u64 {
    if window > 0 {
        used.min(window as u64)
    } else {
        used
    }
}

/// 千位缩写：`198200` → `198.2k`，`1234567` → `1.2M`。给浮窗单行用。
pub fn kfmt(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// 一条在飞交换的可打印视图。
pub fn inflight_json(st: &State) -> Vec<Value> {
    let snaps = st.monitor.snapshot();
    // 上下文窗口总量：模型元数据（人工值 > 探测值，已按后端上限钳过）。
    // 元数据自带 TTL 缓存，这里整批只取一次，别一行查一遍。
    let meta = st.store.effective_model_meta_cached().unwrap_or_default();
    snaps
        .iter()
        .map(|s| {
            // 纠偏系数分模型、分后端；样本没攒够时各自退回先验（见模块注释）。
            let pre = st.metrics.pre_coeff(&s.backend, &s.model);
            let dec = st.metrics.dec_coeff(&s.backend, &s.model);
            // 排队阶段还没交给上游，谈不上 prefill 速率。
            let prefill_tps = if s.phase == "prefill" {
                prefill_rate(s.req_bytes, s.est_input_tokens, s.prefill_ms, pre)
            } else {
                0.0
            };
            let dec_tps = if s.phase == "generating" {
                dec_rate(s.bytes, s.data_events, gen_ms(s.elapsed_ms, s.ttft_ms), dec)
            } else {
                0.0
            };
            let p99 = st.metrics.prefill_p99(&s.backend, &s.model);
            let ctx_window = meta.get(s.model.as_str()).and_then(|m| m.ctx).unwrap_or(0);
            // ctx 先验：该 (backend, model) 上一请求上游真报的输入（BUG-8 再修）。
            let real_in = st.metrics.last_real_input(&s.backend, &s.model);
            json!({
                "id": s.id,
                "backend": s.backend,
                "model": s.model,
                "entry": s.entry,
                // 发起方代理（认不出为 null）：单行速率线按它过滤。
                "app": s.app,
                // 发起方进程（认不出为 null）：同代理多实例按 pid 链分流。
                "pid": s.pid,
                "phase": s.phase,
                "elapsed_ms": s.elapsed_ms,
                "queue_ms": s.queue_ms,
                "prefill_ms": s.prefill_ms,
                "ttft_ms": s.ttft_ms,
                "chunks": s.data_events,
                "bytes": s.bytes,
                "req_bytes": s.req_bytes,
                "dec_tps": (dec_tps * 10.0).round() / 10.0,
                "est_input_tokens": s.est_input_tokens,
                // 已用 / 总上下文（单行速率线的第三段）；总 0 = 未知，前端画「—」。
                "used_ctx": clamp_ctx(
                    used_ctx(
                        s.req_bytes,
                        s.est_input_tokens,
                        s.bytes,
                        s.data_events,
                        pre,
                        dec,
                        ctx_window,
                        real_in,
                    ),
                    ctx_window,
                ),
                "ctx_window": ctx_window,
                "prefill_tps": (prefill_tps.round() as u64),
                "calibrated": pre.is_some(),
                "outlier": p99.is_some_and(|p| prefill_tps > p * OUTLIER_FACTOR),
            })
        })
        .collect()
}

/// 浮条一次拉取的全部数据：在飞明细 + 按模型速率 + (backend,model) 纠偏系数 + 全局摘要。
pub fn snapshot(st: &State) -> Value {
    let active = st.monitor.count_by_model();
    json!({
        "inflight": inflight_json(st),
        "models": st.metrics.snapshot_models(&active),
        "pairs": st.metrics.pair_snapshot(),
        "global": st.metrics.snapshot(st.monitor.snapshot().len()),
    })
}

// ── 单行速率线的对外出口（代理速率扩展唯一取数处）───────────────────────────

/// 相位排序：解码中 > prefill > 排队（与前端 `FloatRateLine` 的 `PHASE_RANK` 同口径）。
fn phase_rank(phase: &str) -> u8 {
    match phase {
        "generating" => 0,
        "prefill" => 1,
        "queued" => 2,
        _ => 9,
    }
}

/// 引擎原生速率（backend → 引擎名 → (prefill, decode)）。
///
/// 与前端 `lib/nativeRates.ts::makeNativeResolver` **同一份规则**，只是挪到服务端：
/// 各家代理的速率扩展若各自复刻一遍，口径必分叉（旧版四家脚本各算一套正是前车之鉴）。
/// 认不出引擎 / 引擎没报 → `None`，调用方退回网关校准值。
async fn native_snapshot(st: &State) -> (Value, std::collections::HashMap<String, String>) {
    let engines: std::collections::HashMap<String, String> = st
        .store
        .list_backends()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|b| {
            b.engine
                .filter(|e| !e.trim().is_empty())
                .map(|e| (b.name, e))
        })
        .collect();
    let er = match crate::services::halctl::channel_of("halogen") {
        Ok(ch) => st.engine_rates.snapshot(&st.config.halctl, ch).await,
        // halogen 通道没配（本机没这号引擎）：原生速率整条缺席，全部退回校准值。
        Err(_) => json!({}),
    };
    (er, engines)
}

/// 该 (backend) 此刻的原生 (prefill, decode)；没报的字段是 `None`。
fn native_of(
    er: &Value,
    engines: &std::collections::HashMap<String, String>,
    backend: &str,
) -> (Option<f64>, Option<f64>) {
    match engines.get(backend).map(|s| s.as_str()) {
        Some("halogen") => {
            let h = &er["halogen"];
            if h["ok"].as_bool().unwrap_or(false) {
                (h["prefill_tps"].as_f64(), h["decode_tps"].as_f64())
            } else {
                (None, None)
            }
        }
        _ => (None, None),
    }
}

/// 多实例过滤判定（BUG-1）：行是否属于调用方实例。
/// · 没传链（None/空）→ 全放行（旧扩展纯 app 过滤，行为不变）；
/// · 行 pid 在链上 → 本实例（或其子进程）发的，放行；
/// · 行 pid 认不出（None）→ 放行——归属失败不该让实例看不到自己的流量；
/// · 行 pid 有值但不在链上 → 别的实例，滤掉。
fn pid_owned(row_pid: Option<u64>, chain: Option<&[u32]>) -> bool {
    match chain {
        None | Some([]) => true,
        Some(chain) => match row_pid {
            Some(p) => chain.contains(&(p as u32)),
            None => true,
        },
    }
}

/// 单行速率线的对外视图：`{ ok, app, updated_ms, rows: [...] }`。
///
/// 这是**代理速率扩展唯一的取数出口**（`GET /admin/rates/line?app=<代理>`）：
/// 数值口径与浮窗单行速率线逐字一致——
/// · `decode_tps`：引擎原生优先；原生恒 0（llama.cpp 口径的请求收尾才累计）或没报
///   时退回网关校准值，并用 `source` 标明来源（`native` / `gateway` / `none`）；
/// · `prefill_tps`：同一套规则；
/// · `used_ctx` 已在 `inflight_json` 里钳到窗口总量。
/// 代理名认不出时网关给 `app: null`，这里就退回「全部在飞」——与浮窗同一口径，
/// 宁可不过滤也不猜一家。
///
/// `pids`（BUG-1，多实例分流）：扩展传来的**自身进程祖先链**（自己 → 父 → …）。
/// 同一家代理开多个实例时 `app` 分不出谁是谁，两个实例都看到全部行、各取第一条，
/// 于是"永远显示 1 号槽位的速率"。挂号时已把发起进程 pid 记进在飞流（`origin`
/// 反查），这里按 pid 链再过滤一层：行的 pid ∈ 调用方祖先链 → 是本实例（或它的
/// 子进程，如 Claude Code 的 hook/子代理）发的。没传 `pids` 的旧扩展退回纯 `app`
/// 过滤，行为不变；pid 认不出（None）的行在传了 `pids` 时保留——归属失败不该
/// 让实例看不到自己的流量，宁可多显不可漏显。
pub async fn line_json(st: &State, app: Option<&str>, pids: Option<&[u32]>) -> Value {
    let mut rows: Vec<Value> = inflight_json(st)
        .into_iter()
        .filter(|r| match app {
            Some(a) => r["app"].as_str() == Some(a),
            None => true,
        })
        .filter(|r| pid_owned(r["pid"].as_u64(), pids))
        .collect();
    // 并发序号：同一家代理同时有几条在飞是常事（子代理、并行工具、compact…），而网关
    // **分不出**谁是主谁是子——同进程、同入口、同模型，没有任何可靠信号（`origin` 只到
    // 进程，`request_classifier` 那套 subagent 判定靠正文里的 `__SUBAGENT_MARKER__`，
    // pi 根本不发）。但「谁先挂号」是事实：id 单调递增，按它编号，渲染侧在多于一行时
    // 打 `#N`，让并发的几行彼此分得开。不宣称任何一行是子代理。
    {
        let mut order: Vec<(u64, usize)> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r["id"].as_u64().unwrap_or(0), i))
            .collect();
        order.sort_unstable();
        for (seq, (_, i)) in order.iter().enumerate() {
            rows[*i]["seq"] = json!(seq + 1);
        }
    }
    let (er, engines) = native_snapshot(st).await;
    for row in rows.iter_mut() {
        let backend = row["backend"].as_str().unwrap_or_default().to_string();
        let (pre, dec) = native_of(&er, &engines, &backend);
        // 生成中原生差分恒 0 是 llama.cpp 的计数口径，不是「没在生成」——退回校准值。
        let dec_final = dec.filter(|v| *v > 0.0).or_else(|| row["dec_tps"].as_f64());
        let pre_final = pre.or_else(|| row["prefill_tps"].as_f64());
        let source = if dec.filter(|v| *v > 0.0).is_some() {
            "native"
        } else if dec_final.is_some() {
            "gateway"
        } else {
            "none"
        };
        row["decode_tps"] = dec_final
            .map(|v| json!((v * 10.0).round() / 10.0))
            .unwrap_or(Value::Null);
        row["prefill_tps"] = pre_final
            .map(|v| json!((v * 10.0).round() / 10.0))
            .unwrap_or(Value::Null);
        row["source"] = json!(source);
    }
    // 同相位按挂号先后排（序号在扩展里是稳定标签，别每次刷新换个位置）。
    rows.sort_by_key(|r| {
        (
            phase_rank(r["phase"].as_str().unwrap_or_default()),
            r["id"].as_u64().unwrap_or(0),
        )
    });
    json!({
        "ok": true,
        "app": app,
        "updated_ms": crate::types::now_ms(),
        "rows": rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefill_rate_prefers_coeff_then_heuristic() {
        // 1000B 请求块跑 100ms：无系数 → 用估算 250 tok；有系数 0.5 → 1000×0.5。
        assert!((prefill_rate(1000, 250, 100, None) - 2500.0).abs() < 1e-6);
        assert!((prefill_rate(1000, 250, 100, Some(0.5)) - 5000.0).abs() < 1e-6);
        assert_eq!(prefill_rate(1000, 250, 0, Some(0.5)), 0.0, "0 毫秒防除零");
    }

    #[test]
    fn dec_rate_prefers_coeff_then_event_arrival() {
        // 2000B / 20 事件跑 1000ms：无系数 → 事件到达率 20 t/s；有系数 0.05 → 100 t/s。
        assert!((dec_rate(2000, 20, 1000, None) - 20.0).abs() < 1e-6);
        assert!((dec_rate(2000, 20, 1000, Some(0.05)) - 100.0).abs() < 1e-6);
        assert_eq!(dec_rate(2000, 20, 0, Some(0.05)), 0.0, "0 毫秒防除零");
    }

    #[test]
    fn clamp_ctx_never_exceeds_window() {
        // 估超了也绝不许画出「已用 > 总量」；窗口未知才原样给。
        assert_eq!(clamp_ctx(210_000, 200_000), 200_000);
        assert_eq!(clamp_ctx(190_000, 200_000), 190_000, "没超就不动");
        assert_eq!(clamp_ctx(999, 0), 999, "窗口未知不钳");
    }

    #[test]
    fn used_ctx_prefers_coefficients_over_heuristics() {
        // 没系数：退回估算输入 + 事件数。
        assert_eq!(used_ctx(4000, 1000, 2000, 20, None, None, 0, None), 1020);
        // 两个系数都在：两段都按字节×系数，估算输入根本不上场。
        assert_eq!(used_ctx(4000, 1000, 2000, 20, Some(0.25), Some(0.25), 0, None), 1500);
        // 只有 pre 系数：输入用校准值，输出仍走事件先验。
        assert_eq!(used_ctx(4000, 1000, 2000, 20, Some(0.25), None, 0, None), 1020);
    }

    #[test]
    fn used_ctx_falls_back_to_prior_when_coefficient_overshoots_window() {
        // BUG-8（用户 2026-11-16）：系数把已用估过窗口时退回先验，
        // 不再画出「已用=总量」的假满窗。
        // 系数估 = 4000*0.25 + 2000*0.25 = 1500 > 窗口 1200；先验 = 1000+20 = 1020 < 1500 → 用先验。
        assert_eq!(used_ctx(4000, 1000, 2000, 20, Some(0.25), Some(0.25), 1200, None), 1020);
        // 先验也超窗时不硬拗（留给 clamp_ctx 兜底钳住）。
        assert_eq!(used_ctx(4000, 3000, 2000, 20, Some(0.25), Some(0.25), 1200, None), 1500);
        // 系数估没超窗：原样给，先验不掺和。
        assert_eq!(used_ctx(4000, 1000, 2000, 20, Some(0.25), Some(0.25), 2000, None), 1500);
    }

    #[test]
    fn used_ctx_prefers_real_reported_input_as_prior() {
        // BUG-8 再修（用户 2026-11-16 复发截图）：字节启发式先验对整会话重发的
        // JSON 载荷同样虚高（est_input=3000 也超窗）。上一请求上游**真报**输入
        // real_in=900 是最可信先验：900+20=920 < 窗口 → 画 920，不再假满窗。
        assert_eq!(used_ctx(4000, 3000, 2000, 20, Some(0.25), Some(0.25), 1200, Some(900)), 920);
        // 没超窗时真报先验不掺和（系数估照旧）。
        assert_eq!(used_ctx(4000, 1000, 2000, 20, Some(0.25), Some(0.25), 2000, Some(900)), 1500);
        // 真报先验比系数估还大时不放大（min 退回系数估，超窗部分留给 clamp 兜底）。
        assert_eq!(used_ctx(4000, 1000, 2000, 20, Some(0.25), Some(0.25), 1200, Some(5000)), 1500);
    }

    #[test]
    fn kfmt_abbreviates_thousands() {
        assert_eq!(kfmt(999), "999");
        assert_eq!(kfmt(198_200), "198.2k");
        assert_eq!(kfmt(1_500_000), "1.5M");
    }

    #[test]
    fn gen_span_is_now_minus_first_byte() {
        assert_eq!(gen_ms(5000, Some(2000)), 3000);
        assert_eq!(gen_ms(5000, None), 0);
        assert_eq!(gen_ms(1000, Some(2000)), 0, "时钟不倒挂");
    }

    #[test]
    fn pid_owned_splits_same_app_instances() {
        // 没传链 = 旧行为：全放行。
        assert!(pid_owned(Some(100), None));
        assert!(pid_owned(Some(100), Some(&[])));
        assert!(pid_owned(None, None));
        // 传了链：在链上（本实例或其子进程）放行，不在链上（别的实例）滤掉。
        assert!(pid_owned(Some(100), Some(&[100, 90, 80])));
        assert!(pid_owned(Some(90), Some(&[100, 90, 80])), "父进程也算本实例");
        assert!(!pid_owned(Some(200), Some(&[100, 90, 80])), "别的实例要滤掉");
        // 行 pid 认不出：宁可多显不可漏显，放行。
        assert!(pid_owned(None, Some(&[100, 90])));
    }
}
