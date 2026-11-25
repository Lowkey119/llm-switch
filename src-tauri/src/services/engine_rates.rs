//! 引擎原生速率（主计划 §9 / 切片 11）：速率显示的数据源改为引擎自己的上报，
//! 网关侧的字节估算展示退役（校准机制本身保留——用量页还吃它）。
//!
//! - **halogen**：`halctl::status` 带回的 `/metrics` Prometheus 计数器（与 halogen
//!   面板同源）。`llamacpp:*_total` 是累计量，两次采样差分才得瞬时速率；计数器
//!   回退（引擎重启）时如实报 null，不拿假速率糊脸。
//! - **TTL 缓存**：监控页（3s）与浮窗/速率线（1s）共享同一份快照，halogen 的
//!   SSH 往返每个 TTL 至多一次。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::HalctlCfg;
use crate::services::halctl::{self, Channel};

/// 快照共享 TTL（多消费者合流，见模块头）。
const TTL: Duration = Duration::from_millis(2000);

/// halogen 计数器采样点（差分基线）。全 f64，Prometheus 原样。
/// `at` 是采样时刻：引擎秒数计数器不推进但 token 在动时拿墙钟兜底（见 hal_rates）。
#[derive(Clone, Copy, Debug)]
struct HalSample {
    prompt_tokens: f64,
    prompt_secs: f64,
    pred_tokens: f64,
    pred_secs: f64,
    at: Instant,
}

impl HalSample {
    fn from_metrics(m: &HashMap<String, f64>) -> Self {
        let g = |k: &str| m.get(k).copied().unwrap_or(0.0);
        Self {
            prompt_tokens: g("llamacpp:prompt_tokens_total"),
            prompt_secs: g("llamacpp:prompt_seconds_total"),
            pred_tokens: g("llamacpp:tokens_predicted_total"),
            pred_secs: g("llamacpp:tokens_predicted_seconds_total"),
            at: Instant::now(),
        }
    }
}

/// 两次采样差分 →（prefill, decode）瞬时 tok/s。
/// 计数器回退（引擎重启）→ 两个都 null；没出新 token → 0（不是没数）。
/// 引擎秒数不推进但 token 在动（有的引擎只在请求收尾才累计时间）→ 拿墙钟兜底：
/// Δtoken/墙钟 是真实平均速率，模型在输出却报 0.0 比这更误导。
fn hal_rates(prev: &HalSample, cur: &HalSample) -> (Option<f64>, Option<f64>) {
    let wall = cur.at.duration_since(prev.at).as_secs_f64().max(0.001);
    let one = |dt: f64, dsec: f64| -> Option<f64> {
        if dt < 0.0 || dsec < 0.0 {
            return None; // 计数器回退：基线作废
        }
        if dt <= 0.0 {
            return Some(0.0); // 没出新 token 就是 0，不是没数
        }
        let denom = if dsec > 0.0 { dsec } else { wall };
        Some(dt / denom)
    };
    (
        one(
            cur.prompt_tokens - prev.prompt_tokens,
            cur.prompt_secs - prev.prompt_secs,
        ),
        one(
            cur.pred_tokens - prev.pred_tokens,
            cur.pred_secs - prev.pred_secs,
        ),
    )
}

/// 由 halctl::status 结果构造 halogen 视图；`prev` 是上轮采样（差分基线）。
fn halogen_view(status: &Value, prev: Option<HalSample>) -> (Value, HalSample) {
    let m: HashMap<String, f64> = status["metrics"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_f64().map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default();
    let cur = HalSample::from_metrics(&m);
    let (pf, dec) = match prev {
        Some(p) => hal_rates(&p, &cur),
        None => (None, None), // 首轮没有基线：如实 null
    };
    let g = |k: &str| m.get(k).copied();
    (
        json!({
            "ok": true,
            "running": status["running"].as_bool().unwrap_or(false),
            "prefill_tps": pf,
            "decode_tps": dec,
            "processing": g("llamacpp:requests_processing"),
            "deferred": g("llamacpp:requests_deferred"),
            "kv_ratio": g("llamacpp:kv_cache_usage_ratio"),
            "kv_tokens": g("llamacpp:kv_cache_tokens"),
        }),
        cur,
    )
}

/// 单例状态（挂在 `State.engine_rates`）：halogen 差分基线 + TTL 快照缓存。
#[derive(Default)]
pub struct EngineRates {
    inner: Mutex<Inner>,
    /// 刷新串行锁：两个消费者（监控页 3s / 浮窗 1s）同时撞 TTL 过期时，
    /// 并发 SSH 会互相把差分基线切成半秒小窗口，dt=0 的噪声 0.0 被缓存糊屏。
    /// 拿锁后双检缓存，后来者直接吃现成快照。
    refresh_lock: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Inner {
    hal_prev: Option<HalSample>,
    cached: Option<(Instant, Value)>,
}

impl EngineRates {
    pub fn new() -> Self {
        Self::default()
    }

    /// 聚合快照 `{ halogen: {...} }`；TTL 内直接回缓存。
    pub async fn snapshot(&self, ctl: &HalctlCfg, ch: &'static Channel) -> Value {
        if let Some((at, v)) = self
            .inner
            .lock()
            .expect("engine_rates lock")
            .cached
            .as_ref()
        {
            if at.elapsed() < TTL {
                return v.clone();
            }
        }
        // 刷新串行化：拿锁后双检——排队期间别人已经刷新的直接吃现成快照，
        // 否则两次刷新会互相把差分基线切成半秒窗口，dt=0 的假 0 被缓存糊屏。
        let _refresh = self.refresh_lock.lock().await;
        if let Some((at, v)) = self
            .inner
            .lock()
            .expect("engine_rates lock")
            .cached
            .as_ref()
        {
            if at.elapsed() < TTL {
                return v.clone();
            }
        }
        let hal = match halctl::status(ctl, ch).await {
            Ok(s) => {
                let mut g = self.inner.lock().expect("engine_rates lock");
                let (v, cur) = halogen_view(&s, g.hal_prev);
                g.hal_prev = Some(cur);
                v
            }
            Err(e) => json!({ "ok": false, "error": e }),
        };
        let v = json!({ "halogen": hal });
        self.inner.lock().expect("engine_rates lock").cached = Some((Instant::now(), v.clone()));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics(pf_t: f64, pf_s: f64, dec_t: f64, dec_s: f64) -> Value {
        json!({
            "running": true,
            "metrics": {
                "llamacpp:prompt_tokens_total": pf_t,
                "llamacpp:prompt_seconds_total": pf_s,
                "llamacpp:tokens_predicted_total": dec_t,
                "llamacpp:tokens_predicted_seconds_total": dec_s,
                "llamacpp:requests_processing": 2.0,
                "llamacpp:kv_cache_usage_ratio": 0.4,
            }
        })
    }

    #[test]
    fn hal_rates_diffs_counters() {
        let prev = HalSample {
            prompt_tokens: 1000.0,
            prompt_secs: 1.0,
            pred_tokens: 50.0,
            pred_secs: 2.0,
            at: Instant::now(),
        };
        let cur = HalSample {
            prompt_tokens: 3000.0,
            prompt_secs: 2.0,
            pred_tokens: 70.0,
            pred_secs: 4.0,
            at: Instant::now(),
        };
        let (pf, dec) = hal_rates(&prev, &cur);
        assert_eq!(pf, Some(2000.0)); // Δ2000 tok / Δ1s
        assert_eq!(dec, Some(10.0)); // Δ20 tok / Δ2s
    }

    #[test]
    fn hal_rates_idle_is_zero_and_reset_is_null() {
        let prev = HalSample {
            prompt_tokens: 100.0,
            prompt_secs: 1.0,
            pred_tokens: 10.0,
            pred_secs: 1.0,
            at: Instant::now(),
        };
        // 空闲：计数没动 → 0（不是没数）。
        let (pf, dec) = hal_rates(&prev, &prev);
        assert_eq!(pf, Some(0.0));
        assert_eq!(dec, Some(0.0));
        // 引擎重启（计数器回退）→ 基线作废。
        let after_restart = HalSample {
            prompt_tokens: 5.0,
            prompt_secs: 0.1,
            pred_tokens: 1.0,
            pred_secs: 0.1,
            at: Instant::now(),
        };
        let (pf, dec) = hal_rates(&prev, &after_restart);
        assert_eq!(pf, None);
        assert_eq!(dec, None);
    }

    /// 用户实报「模型在输出但速率线 0.0」：引擎秒数计数器只在请求收尾才累计时，
    /// 解码中 Δtoken>0 而 Δsec=0——旧口径直接报 0，现在拿墙钟兜底出真实平均速率。
    #[test]
    fn hal_rates_wall_clock_fallback_when_engine_secs_frozen() {
        let prev = HalSample {
            prompt_tokens: 0.0,
            prompt_secs: 0.0,
            pred_tokens: 100.0,
            pred_secs: 0.0,
            at: Instant::now(),
        };
        let cur = HalSample {
            pred_tokens: 160.0,
            ..prev
        };
        // 同一瞬间（duration≈0 → 钳到 1ms）：60 tok / ≥0.001s，必须 > 0 而不是 0。
        let (_, dec) = hal_rates(&prev, &cur);
        assert!(dec.unwrap() > 0.0, "token 在动就不许报 0：{dec:?}");
    }

    #[test]
    fn halogen_view_first_round_has_no_rates() {
        let (v, cur) = halogen_view(&metrics(100.0, 1.0, 10.0, 1.0), None);
        assert_eq!(v["ok"], true);
        assert_eq!(v["running"], true);
        assert!(
            v["prefill_tps"].is_null(),
            "首轮没基线必须报 null，不许编速率"
        );
        assert_eq!(v["processing"], 2.0);
        assert_eq!(cur.prompt_tokens, 100.0);
        // 第二轮差分。
        let (v2, _) = halogen_view(&metrics(300.0, 2.0, 30.0, 3.0), Some(cur));
        assert_eq!(v2["prefill_tps"], 200.0);
        assert_eq!(v2["decode_tps"], 10.0);
    }
}
