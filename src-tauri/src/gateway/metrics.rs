//! 1s 滑窗速率聚合（M4-A）。
//!
//! 口径（与 docs/ARCHITECTURE.md §6 同源）：
//! - RPS / 错误率：最近 `WINDOW_SECS` 秒完成请求的滑窗均值；
//! - tokens/s：滑窗内 output_tokens 之和 / 窗口秒数；
//! - TTFT P50/P95：滑窗内有 ttft 样本的分位数（线性插值）；
//! - EWMA：每秒推进，α=`EWMA_ALPHA`，用于比瞬时值平滑的趋势线。
//!
//! 桶推进采用惰性轮转（记录/快照时按需补齐），不依赖后台定时器。

use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// 滑窗长度（秒）。
pub const WINDOW_SECS: usize = 300;
/// 序列输出长度（sparkline 用）。
const SERIES_POINTS: usize = 120;
/// EWMA 平滑系数（越大越跟随瞬时值）。
const EWMA_ALPHA: f64 = 0.3;
/// 单次最多补推的秒数（防止长时间空闲后疯狂循环）。
const MAX_CATCHUP_SECS: u64 = 600;
/// 每桶 TTFT 样本上限（防内存膨胀；超出即放弃采样，比例足够）。
const TTFT_SAMPLES_PER_SEC: usize = 256;
/// 按模型的滑窗长度（秒），同时是每模型曲线的窗口与取点上限。
/// 2026-10-16 拍板从 120 拉宽到 300：监控页撤掉全局吞吐图、曲线全在每模型行上，
/// 2 分钟的窗口只剩几根尖刺，看不出趋势（与全局 `WINDOW_SECS` 对齐）。
/// 2026-11-16 用户拍板再拉长到 600：曲线展宽后 300s 仍嫌短，10 分钟整窗更好读。
pub const MODEL_WINDOW_SECS: usize = 600;
/// 「当前在工作」的判定窗口：最近这么多秒内有请求完成即算活跃。
pub const MODEL_ACTIVE_SECS: u64 = 60;
/// 样本攒够这么多次才肯出纠偏系数（宁缺不造）。
const MIN_CALIB_SAMPLES: u64 = 3;
/// 纠偏系数样本的接受域（tok/B）：1kB 才一个 token、或两字节就一个 token，都当脏样本丢。
const COEFF_GUARD: std::ops::RangeInclusive<f64> = 0.001..=2.0;
/// 系数的钳制域：上界防个别幸运样本把速率整体带飞；下界贴守卫下限 0.001——
/// 真实比率可以低于 0.005（实测封口对账：现代 OpenAI 分块 ~250-350B/token
/// ≈ 0.003-0.0047 tok/B，旧下界 0.005 会把中位数顶起来，造成系统性虚高）。
const COEFF_CLAMP: (f64, f64) = (0.001, 1.0);

#[derive(Debug)]
struct Bucket {
    sec: u64,
    reqs: u32,
    errs: u32,
    out_tokens: u64,
    ttfts: Vec<u16>,
}

impl Bucket {
    fn new(sec: u64) -> Self {
        Self {
            sec,
            reqs: 0,
            errs: 0,
            out_tokens: 0,
            ttfts: Vec::new(),
        }
    }
}

/// 按模型的第二套桶（M12-B）。与全局桶同构但窗口更短：
/// 监控页要的是「哪个模型正在被啃」，5 分钟窗口会把刚停下来的模型也拖着不放。
#[derive(Debug)]
struct MBucket {
    sec: u64,
    reqs: u32,
    errs: u32,
    out_tokens: u64,
    /// 实测 decode 跨度累计（毫秒）。与 [`MBucket::dec_tok`] 成对——
    /// 「按模型 tok/s」= Σ输出 token ÷ Σ(末字−首字)，不除以桶数。
    dec_ms: u64,
    /// 计入 decode 速率的输出 token（只有成对样本才累加）。
    dec_tok: u64,
    ttfts: Vec<u16>,
}

#[derive(Debug, Default)]
struct ModelAgg {
    buckets: VecDeque<MBucket>,
    last_sec: u64,
    /// 参与过统计的后端名（同一模型名可能由多台后端提供）。
    backends: std::collections::BTreeSet<String>,
}

/// 记一笔完成事件进桶（按模型与按 (后端, 模型) 两套桶共用，口径同源）。
fn record_completion(
    agg: &mut ModelAgg,
    ok: bool,
    ttft_ms: Option<i64>,
    out_tokens: u64,
    decode_span_ms: Option<u64>,
) {
    if let Some(top) = agg.buckets.back_mut() {
        top.reqs += 1;
        if !ok {
            top.errs += 1;
        }
        top.out_tokens += out_tokens;
        // token 与跨度必须成对入账：没 token 的跨度（无 usage 的流）会把速率拖低，
        // 没跨度的 token 走快照里的墙钟兜底，两边都不造假。
        if out_tokens > 0 {
            if let Some(ms) = decode_span_ms.filter(|ms| *ms > 0) {
                top.dec_tok += out_tokens;
                top.dec_ms += ms;
            }
        }
        if let Some(t) = ttft_ms.and_then(|t| u16::try_from(t.max(0)).ok()) {
            if top.ttfts.len() < TTFT_SAMPLES_PER_SEC {
                top.ttfts.push(t);
            }
        }
    }
}

/// (backend, model) 一对的纠偏聚合：**分模型、分后端**各算各的系数——
/// 同一模型名可能由多台后端提供，tokenizer、出口封装、分块粒度都不一样，
/// 混在一起算就是不准（用户明确要求的口径）。
///
/// 系数 = 近 [`COEFF_RING`] 个样本的**中位数**，不是均值（实测教训：
/// 封口字节与 usage 错位的脏样本（重试、半截流）比率能比真值高几倍，
/// 均值被少数离群拖飞 —— 浮窗速率一度比账本真值高 50%+；中位数免疫）。
#[derive(Debug, Default)]
struct PairAgg {
    /// 近期 prefill 样本（上游真报输入 ÷ 请求块字节，tok/B），环形保留。
    pre: VecDeque<f64>,
    /// prefill 样本累计总数（口径展示用；系数只看环内中位数）。
    pre_n: u64,
    /// 近期 decode 样本（上游真报输出 ÷ 响应流字节，tok/B），环形保留。
    dec: VecDeque<f64>,
    /// decode 样本累计总数。
    dec_n: u64,
    /// 历次封口 prefill 速率（tok/s），离群判定用。
    pre_rates: Vec<f64>,
    /// llama.cpp 引擎 timings 实测 prefill 速率真值环（tok/s）。与系数派生速率
    /// 并排展示——系数偏差一眼可见。真值不需要 ≥3 样本才可信：引擎一次实测即真值。
    eng_pre: VecDeque<f64>,
    /// llama.cpp 引擎 timings 实测 decode 速率真值环（tok/s）。
    eng_dec: VecDeque<f64>,
    /// 喂过引擎真值的请求数（一请求一条）。
    eng_n: u64,
    /// 最近一次封口上游**真报**的输入 token（BUG-8 再修，用户 2026-11-16 截图：
    /// `ctx 262.1k/262.1k` 复发）。claude 系整会话重发时相邻请求的输入几乎相同，
    /// 这是 ctx 估算最可信的先验——字节启发式对整会话 JSON 载荷同样系统性虚高，
    /// 先验也超窗时 clamp 只能画出假满窗。
    last_real_in: Option<u64>,
}

/// 纠偏样本环容量：够覆盖数百请求的近期行为，又让坏样本能被自然冲刷出环。
const COEFF_RING: usize = 64;

/// 喂一个样本进环，超容挤掉最老的。
fn push_sample(ring: &mut VecDeque<f64>, v: f64) {
    ring.push_back(v);
    while ring.len() > COEFF_RING {
        ring.pop_front();
    }
}

/// 环内中位数（偶数取上中位）。环空返回 0 —— 调用方都以 `*_n >= MIN_CALIB_SAMPLES`
/// 把门，n≥3 时环必非空。
fn ring_median(ring: &VecDeque<f64>) -> f64 {
    if ring.is_empty() {
        return 0.0;
    }
    let mut v: Vec<f64> = ring.iter().copied().collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

/// 纠偏样本喂哪一侧。
#[derive(Clone, Copy)]
enum Side {
    Pre,
    Dec,
}

impl ModelAgg {
    fn touch(&mut self, sec: u64) {
        if self.buckets.is_empty() {
            self.buckets.push_back(MBucket {
                sec,
                reqs: 0,
                errs: 0,
                out_tokens: 0,
                dec_ms: 0,
                dec_tok: 0,
                ttfts: Vec::new(),
            });
            self.last_sec = sec;
            return;
        }
        let gap = sec.saturating_sub(self.last_sec);
        if gap > MAX_CATCHUP_SECS {
            self.buckets.clear();
        } else {
            for tick in (self.last_sec + 1)..=sec {
                self.buckets.push_back(MBucket {
                    sec: tick,
                    reqs: 0,
                    errs: 0,
                    out_tokens: 0,
                    dec_ms: 0,
                    dec_tok: 0,
                    ttfts: Vec::new(),
                });
            }
        }
        self.last_sec = sec;
        while self
            .buckets
            .front()
            .is_some_and(|b| sec.saturating_sub(b.sec) >= MODEL_WINDOW_SECS as u64)
        {
            self.buckets.pop_front();
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    buckets: VecDeque<Bucket>,
    last_sec: u64,
    ewma_rps: f64,
    ewma_tps: f64,
    models: std::collections::HashMap<String, ModelAgg>,
    /// (backend, model) → 同一套桶（切片 17：监控页「每后端一卡、卡内模型行」要
    /// 分后端的真实数字——同名模型多后端供给时，按模型聚合的桶分不出是谁扛的）。
    /// 与 `models` 双写同源事件，口径天然一致；互不派生，谁也不给谁兜底。
    by_pair: std::collections::HashMap<(String, String), ModelAgg>,
    /// (backend, model) → 纠偏样本（分模型分后端，见 [`PairAgg`]）。
    pairs: std::collections::HashMap<(String, String), PairAgg>,
}

/// 单个模型的速率快照（监控页「按模型」面板与浮条共用）。
#[derive(Debug, Clone, Serialize)]
pub struct ModelMetrics {
    pub model: String,
    pub backends: Vec<String>,
    /// 滑窗内的请求数与派生速率。
    pub reqs: u32,
    pub errs: u32,
    pub rps: f64,
    pub tokens_per_s: f64,
    pub errors_per_s: f64,
    pub out_tokens: u64,
    pub ttft_p50_ms: Option<u64>,
    /// 近期首响样本数（与 P50 一起下发，UI 标注 `n=` —— 样本少时分位数不可信）。
    pub ttft_samples: usize,
    /// 此刻仍在飞的流数。
    pub active_streams: usize,
    /// 最近 [`MODEL_ACTIVE_SECS`] 秒内有完成请求，或有活跃流 —— 即「当前在工作」。
    pub active: bool,
    /// 逐秒序列（旧→新），画迷你折线用，长度上限 = [`MODEL_WINDOW_SECS`]。
    pub series: Vec<SecondPoint>,
    /// 曲线窗口（秒）= `series` 的覆盖范围，UI 标注窗口用（免得前端写死数字）。
    pub window_secs: usize,
}

/// 一对 (后端, 模型) 的速率快照（切片 17：监控页按后端分卡的数据面）。
/// `metrics.backends` 恒为 `[backend]`——形状与 ModelMetrics 完全一致，
/// 前端两套出口共用同一行组件。
#[derive(Debug, Clone, Serialize)]
pub struct BackendModelMetrics {
    pub backend: String,
    #[serde(flatten)]
    pub metrics: ModelMetrics,
}

/// 单个 ModelAgg 的快照核算（快照即推进：桶是完成时刻才惰性创建的，不在快照时
/// 补齐到现在，空闲组就会一直端着最后一次的旧数据、covered 也永远停在 1）。
/// `model`/`backends` 由调用方给——按模型出口传聚合名单，按 (后端, 模型) 出口传单元素。
fn snapshot_agg(
    agg: &mut ModelAgg,
    sec: u64,
    active_streams: usize,
    model: String,
    backends: Vec<String>,
) -> ModelMetrics {
    agg.touch(sec);
    let mut reqs = 0u32;
    let mut errs = 0u32;
    let mut out_tokens = 0u64;
    let mut dec_ms = 0u64;
    let mut dec_tok = 0u64;
    let mut recent = 0u32;
    let mut ttfts: Vec<u16> = Vec::new();
    let mut ttft_recent: Vec<u16> = Vec::new();
    for b in &agg.buckets {
        reqs += b.reqs;
        errs += b.errs;
        out_tokens += b.out_tokens;
        dec_ms += b.dec_ms;
        dec_tok += b.dec_tok;
        ttfts.extend(b.ttfts.iter().copied());
        if sec.saturating_sub(b.sec) < MODEL_ACTIVE_SECS {
            recent += b.reqs;
            ttft_recent.extend(b.ttfts.iter().copied());
        }
    }
    let covered = (agg.buckets.len().max(1)) as f64;
    let mut scratch = Vec::new();
    ModelMetrics {
        model,
        backends,
        reqs,
        errs,
        rps: reqs as f64 / covered,
        // tok/s 首选实测 decode 跨度（Σtoken ÷ Σ跨度 = 真实解码速度，与全局面板、
        // usage 页 decode_tps 同源）；没有成对样本（非流式、上游无 usage）才退回墙钟吞吐兜底。
        tokens_per_s: if dec_ms > 0 {
            dec_tok as f64 / (dec_ms as f64 / 1000.0)
        } else {
            out_tokens as f64 / covered
        },
        errors_per_s: errs as f64 / covered,
        out_tokens,
        // TTFT 只看近期：老样本会把 P50 拖成历史平均，失去「现在怎么样」的意义。
        ttft_p50_ms: percentile(&mut scratch, &ttft_recent, 0.50)
            .or_else(|| percentile(&mut scratch, &ttfts, 0.50)),
        ttft_samples: ttft_recent.len(),
        active_streams,
        active: recent > 0 || active_streams > 0,
        series: agg
            .buckets
            .iter()
            .rev()
            // 取点上限与桶保留同源（MODEL_WINDOW_SECS），窗口与曲线不会各说各话
            .take(MODEL_WINDOW_SECS)
            .rev()
            .map(|b| SecondPoint {
                sec: b.sec,
                reqs: b.reqs,
                errs: b.errs,
                out_tokens: b.out_tokens,
            })
            .collect(),
        window_secs: MODEL_WINDOW_SECS,
    }
}

/// 活跃优先 → tok/s 降序 → 模型名（监控页第一眼要看到正在忙的）。
fn sort_model_metrics(out: &mut [ModelMetrics]) {
    out.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(
                b.tokens_per_s
                    .partial_cmp(&a.tokens_per_s)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then_with(|| a.model.cmp(&b.model))
    });
}

/// 一对 (backend, model) 的纠偏系数快照（/admin/rates 观测出口与 e2e 断言共用）。
#[derive(Debug, Clone, Serialize)]
pub struct PairMetrics {
    pub backend: String,
    pub model: String,
    /// prefill 纠偏系数（tok/B）；None = 样本 <3，出口退回估算。
    pub pre_coeff: Option<f64>,
    /// decode 纠偏系数（tok/B）；None = 样本 <3，出口退回事件到达率。
    pub dec_coeff: Option<f64>,
    pub pre_n: u64,
    pub dec_n: u64,
    /// llama.cpp 引擎实测 prefill 速率（tok/s，近环中位数）；None = 尚无引擎真值。
    /// 与系数派生速率并排：两者的差就是系数偏差的直观读数。
    pub engine_prefill_tps: Option<f64>,
    /// llama.cpp 引擎实测 decode 速率（tok/s，近环中位数）。
    pub engine_dec_tps: Option<f64>,
    /// 喂过引擎真值的请求数。
    pub engine_n: u64,
}

/// 对外快照（IPC / REST / 事件共用同一形状）。
#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub window_secs: usize,
    pub rps: f64,
    pub errors_per_s: f64,
    pub tokens_per_s: f64,
    pub error_rate: f64,
    pub ttft_p50_ms: Option<u64>,
    pub ttft_p95_ms: Option<u64>,
    pub ewma_rps: f64,
    pub ewma_tokens_per_s: f64,
    pub active_streams: usize,
    /// 最近 `SERIES_POINTS` 秒的逐秒序列（旧→新），供前端画折线。
    pub series: Vec<SecondPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecondPoint {
    pub sec: u64,
    pub reqs: u32,
    pub errs: u32,
    pub out_tokens: u64,
}

#[derive(Default)]
pub struct Metrics {
    inner: Mutex<Inner>,
}

fn now_sec() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 系数出口的钳制：防个别幸运样本把速率整体带飞（下界见 [`COEFF_CLAMP`]）。
fn clamp_coeff(c: f64) -> f64 {
    c.clamp(COEFF_CLAMP.0, COEFF_CLAMP.1)
}

/// 线性插值分位数（输入会被排序，原地复用）。
fn percentile(sorted_scratch: &mut Vec<f64>, values: &[u16], q: f64) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    sorted_scratch.clear();
    sorted_scratch.extend(values.iter().map(|v| *v as f64));
    sorted_scratch.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pos = (sorted_scratch.len() as f64 - 1.0) * q;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        Some(sorted_scratch[lo] as u64)
    } else {
        let frac = pos - lo as f64;
        Some((sorted_scratch[lo] * (1.0 - frac) + sorted_scratch[hi] * frac) as u64)
    }
}

impl Metrics {
    /// 记录一次完成的交换（成功或失败都进来，来自 usage 落库的唯一漏斗）。
    pub fn on_completion(&self, ok: bool, ttft_ms: Option<i64>, out_tokens: u64) {
        let sec = now_sec();
        let mut g = self.inner.lock().expect("metrics lock poisoned");
        if g.buckets.is_empty() {
            g.last_sec = sec;
            g.buckets.push_back(Bucket::new(sec));
        }
        Self::advance(&mut g, sec);
        let ttft = ttft_ms.and_then(|t| u16::try_from(t.max(0)).ok());
        if let Some(top) = g.buckets.back_mut() {
            top.reqs += 1;
            if !ok {
                top.errs += 1;
            }
            top.out_tokens += out_tokens;
            if let Some(t) = ttft {
                if top.ttfts.len() < TTFT_SAMPLES_PER_SEC {
                    top.ttfts.push(t);
                }
            }
        }
    }

    /// 按模型记账（M12-B）。与全局同一漏斗调用，口径天然一致。
    /// `decode_span_ms` = 该请求实测 decode 跨度（首字→末字，ARCHITECTURE §6）；
    /// 「按模型 tok/s」= Σ输出 token ÷ Σdecode 跨度——**不能**除以桶数：
    /// 桶是完成时刻才惰性创建的，刚起步只有 1 个桶，除出来就是「一个请求的全部输出」。
    pub fn on_model_completion(
        &self,
        model: &str,
        backend: &str,
        ok: bool,
        ttft_ms: Option<i64>,
        out_tokens: u64,
        decode_span_ms: Option<u64>,
    ) {
        if model.is_empty() {
            return;
        }
        let sec = now_sec();
        let mut g = self.inner.lock().expect("metrics lock poisoned");
        let agg = g.models.entry(model.to_string()).or_default();
        agg.touch(sec);
        agg.backends.insert(backend.to_string());
        record_completion(agg, ok, ttft_ms, out_tokens, decode_span_ms);
        // (backend, model) 并行桶：同一事件同一笔记账（切片 17 每后端一卡的口径）。
        let pair = g
            .by_pair
            .entry((backend.to_string(), model.to_string()))
            .or_default();
        pair.touch(sec);
        record_completion(pair, ok, ttft_ms, out_tokens, decode_span_ms);
    }

    /// prefill 纠偏样本：上游真报输入 ÷ 请求块字节（**分模型、分后端**）。
    pub fn note_pre_coeff(&self, backend: &str, model: &str, req_bytes: u64, real_in: u64) {
        self.note_coeff(backend, model, req_bytes, real_in, Side::Pre);
    }

    /// decode 纠偏样本：上游真报输出 ÷ 响应流字节（**分模型、分后端**）。
    pub fn note_dec_coeff(&self, backend: &str, model: &str, resp_bytes: u64, real_out: u64) {
        self.note_coeff(backend, model, resp_bytes, real_out, Side::Dec);
    }

    /// 比值离谱（跨 tokenizer、脏 usage）与零值直接丢，别污染系数；均值出口再钳一道。
    fn note_coeff(&self, backend: &str, model: &str, bytes: u64, real: u64, side: Side) {
        if backend.is_empty() || model.is_empty() || bytes == 0 || real == 0 {
            return;
        }
        let ratio = real as f64 / bytes as f64;
        if !COEFF_GUARD.contains(&ratio) {
            return;
        }
        let mut g = self.inner.lock().expect("metrics lock poisoned");
        let agg = g
            .pairs
            .entry((backend.to_string(), model.to_string()))
            .or_default();
        match side {
            Side::Pre => {
                push_sample(&mut agg.pre, ratio);
                agg.pre_n += 1;
                // 同一漏斗顺手记「上一请求真报输入」：ctx 估算的先验（见 PairAgg）。
                agg.last_real_in = Some(real);
            }
            Side::Dec => {
                push_sample(&mut agg.dec, ratio);
                agg.dec_n += 1;
            }
        }
    }

    /// prefill 纠偏系数（样本 ≥3 才给；None = 出口退回字节启发式估算）。
    /// 取环内中位数再钳制——脏样本拖不走。
    pub fn pre_coeff(&self, backend: &str, model: &str) -> Option<f64> {
        let g = self.inner.lock().expect("metrics lock poisoned");
        let a = g.pairs.get(&(backend.to_string(), model.to_string()))?;
        (a.pre_n >= MIN_CALIB_SAMPLES).then(|| clamp_coeff(ring_median(&a.pre)))
    }

    /// 最近一次封口上游真报的输入 token（BUG-8 再修的 ctx 先验）；没有 = None。
    pub fn last_real_input(&self, backend: &str, model: &str) -> Option<u64> {
        let g = self.inner.lock().expect("metrics lock poisoned");
        g.pairs
            .get(&(backend.to_string(), model.to_string()))
            .and_then(|a| a.last_real_in)
    }

    /// decode 纠偏系数（样本 ≥3 才给；None = 出口退回事件到达率）。
    /// 同上：中位数 + 钳制。
    pub fn dec_coeff(&self, backend: &str, model: &str) -> Option<f64> {
        let g = self.inner.lock().expect("metrics lock poisoned");
        let a = g.pairs.get(&(backend.to_string(), model.to_string()))?;
        (a.dec_n >= MIN_CALIB_SAMPLES).then(|| clamp_coeff(ring_median(&a.dec)))
    }

    /// 记一次封口 prefill 速率（tok/s，已乘当刻系数），供浮条做离群标记（分模型分后端）。
    pub fn note_prefill_rate(&self, backend: &str, model: &str, tps: f64) {
        if backend.is_empty() || model.is_empty() || !tps.is_finite() || tps <= 0.0 {
            return;
        }
        let mut g = self.inner.lock().expect("metrics lock poisoned");
        let agg = g
            .pairs
            .entry((backend.to_string(), model.to_string()))
            .or_default();
        agg.pre_rates.push(tps);
        if agg.pre_rates.len() > 64 {
            agg.pre_rates.remove(0);
        }
    }

    /// 历次封口 prefill 速率的 P99（浮条离群判定用，分模型分后端）。
    pub fn prefill_p99(&self, backend: &str, model: &str) -> Option<f64> {
        let g = self.inner.lock().expect("metrics lock poisoned");
        let rates = g
            .pairs
            .get(&(backend.to_string(), model.to_string()))?
            .pre_rates
            .clone();
        if (rates.len() as u64) < MIN_CALIB_SAMPLES {
            return None;
        }
        let mut sorted = rates;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((sorted.len() as f64 * 0.99).floor() as usize).min(sorted.len() - 1);
        Some(sorted[idx])
    }

    /// 喂一条 llama.cpp 引擎真值速率（timings 实测 tok/s，None 侧不喂；一请求一次）。
    /// 与纠偏系数环分池：系数是「字节→token」的换算学习，这里是「引擎自测速率」的
    /// 参照系——两边并排即系数偏差的直观读数（MTP 投机解码下事件先验偏高，靠它盯）。
    pub fn note_engine_rates(
        &self,
        backend: &str,
        model: &str,
        pre_tps: Option<f64>,
        dec_tps: Option<f64>,
    ) {
        if backend.is_empty() || model.is_empty() {
            return;
        }
        let sane = |v: Option<f64>| v.filter(|t| t.is_finite() && *t > 0.0);
        let (pre, dec) = (sane(pre_tps), sane(dec_tps));
        if pre.is_none() && dec.is_none() {
            return;
        }
        let mut g = self.inner.lock().expect("metrics lock poisoned");
        let agg = g
            .pairs
            .entry((backend.to_string(), model.to_string()))
            .or_default();
        if let Some(v) = pre {
            push_sample(&mut agg.eng_pre, v);
        }
        if let Some(v) = dec {
            push_sample(&mut agg.eng_dec, v);
        }
        agg.eng_n += 1;
    }

    /// (backend, model) 纠偏系数一览（稳定排序；/admin/rates 观测出口与 e2e 断言用）。
    pub fn pair_snapshot(&self) -> Vec<PairMetrics> {
        let g = self.inner.lock().expect("metrics lock poisoned");
        let mut out: Vec<PairMetrics> = g
            .pairs
            .iter()
            .map(|((backend, model), a)| PairMetrics {
                backend: backend.clone(),
                model: model.clone(),
                pre_coeff: (a.pre_n >= MIN_CALIB_SAMPLES).then(|| clamp_coeff(ring_median(&a.pre))),
                dec_coeff: (a.dec_n >= MIN_CALIB_SAMPLES).then(|| clamp_coeff(ring_median(&a.dec))),
                pre_n: a.pre_n,
                dec_n: a.dec_n,
                // 引擎真值一条即可信（实测值不是学习值），不设 ≥3 门槛。
                engine_prefill_tps: (!a.eng_pre.is_empty()).then(|| ring_median(&a.eng_pre)),
                engine_dec_tps: (!a.eng_dec.is_empty()).then(|| ring_median(&a.eng_dec)),
                engine_n: a.eng_n,
            })
            .collect();
        out.sort_by(|a, b| {
            a.backend
                .cmp(&b.backend)
                .then_with(|| a.model.cmp(&b.model))
        });
        out
    }

    /// 按模型的速率快照。`active_by_model` 来自活跃流注册表（没流量但有流的模型也算活跃）。
    pub fn snapshot_models(
        &self,
        active_by_model: &std::collections::HashMap<String, usize>,
    ) -> Vec<ModelMetrics> {
        let sec = now_sec();
        let mut g = self.inner.lock().expect("metrics lock poisoned");
        let mut out: Vec<ModelMetrics> = g
            .models
            .iter_mut()
            .map(|(model, agg)| {
                let backends = agg.backends.iter().cloned().collect();
                snapshot_agg(
                    agg,
                    sec,
                    active_by_model.get(model).copied().unwrap_or(0),
                    model.clone(),
                    backends,
                )
            })
            .collect();
        sort_model_metrics(&mut out);
        out
    }

    /// 按 (后端, 模型) 的速率快照（切片 17：监控页「每后端一卡、卡内模型行」）。
    /// `active` 按 (后端, 模型) 计在飞——同名模型多后端供给时，「此刻谁在扛」
    /// 只有这个粒度才分得清。
    pub fn snapshot_backend_models(
        &self,
        active: &std::collections::HashMap<(String, String), usize>,
    ) -> Vec<BackendModelMetrics> {
        let sec = now_sec();
        let mut g = self.inner.lock().expect("metrics lock poisoned");
        let mut out: Vec<BackendModelMetrics> = g
            .by_pair
            .iter_mut()
            .map(|((backend, model), agg)| {
                let streams = active
                    .get(&(backend.clone(), model.clone()))
                    .copied()
                    .unwrap_or(0);
                BackendModelMetrics {
                    backend: backend.clone(),
                    metrics: snapshot_agg(agg, sec, streams, model.clone(), vec![backend.clone()]),
                }
            })
            .collect();
        // 与按模型出口同口径：活跃优先 → tok/s 降序 → 后端名 → 模型名。
        out.sort_by(|a, b| {
            b.metrics
                .active
                .cmp(&a.metrics.active)
                .then(
                    b.metrics
                        .tokens_per_s
                        .partial_cmp(&a.metrics.tokens_per_s)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.backend.cmp(&b.backend))
                .then_with(|| a.metrics.model.cmp(&b.metrics.model))
        });
        out
    }

    /// 把桶推进到 `sec`：裁剪过期桶并按秒推进 EWMA。
    fn advance(g: &mut Inner, sec: u64) {
        // 追平当前秒（空闲很久时封顶，直接快进到当下重建窗口）。
        let gap = sec.saturating_sub(g.last_sec);
        if gap > MAX_CATCHUP_SECS {
            g.buckets.clear();
            g.ewma_rps = 0.0;
            g.ewma_tps = 0.0;
        } else {
            for tick in (g.last_sec + 1)..=sec {
                // tick-1 这一秒已经封口：喂给 EWMA。
                if let Some(done) = g.buckets.back().filter(|b| b.sec + 1 == tick) {
                    g.ewma_rps = EWMA_ALPHA * done.reqs as f64 + (1.0 - EWMA_ALPHA) * g.ewma_rps;
                    g.ewma_tps =
                        EWMA_ALPHA * done.out_tokens as f64 + (1.0 - EWMA_ALPHA) * g.ewma_tps;
                } else {
                    g.ewma_rps *= 1.0 - EWMA_ALPHA;
                    g.ewma_tps *= 1.0 - EWMA_ALPHA;
                }
                g.buckets.push_back(Bucket::new(tick));
            }
        }
        g.last_sec = sec;
        while g
            .buckets
            .front()
            .is_some_and(|b| sec.saturating_sub(b.sec) >= WINDOW_SECS as u64)
        {
            g.buckets.pop_front();
        }
        while g.buckets.len() > WINDOW_SECS {
            g.buckets.pop_front();
        }
    }

    pub fn snapshot(&self, active_streams: usize) -> MetricsSnapshot {
        let sec = now_sec();
        let mut g = self.inner.lock().expect("metrics lock poisoned");
        if !g.buckets.is_empty() {
            Self::advance(&mut g, sec);
        }
        let secs_covered = if g.buckets.is_empty() {
            1.0
        } else {
            // 至少按 1s 计，避免刚起步时除以不足 1 秒放大速率。
            (sec.saturating_sub(g.buckets.front().unwrap().sec) + 1).max(1) as f64
        };
        let mut reqs = 0u64;
        let mut errs = 0u64;
        let mut out_tokens = 0u64;
        let mut ttfts: Vec<u16> = Vec::new();
        for b in &g.buckets {
            reqs += b.reqs as u64;
            errs += b.errs as u64;
            out_tokens += b.out_tokens;
            ttfts.extend(b.ttfts.iter().copied());
        }
        let mut scratch = Vec::new();
        let series: Vec<SecondPoint> = g
            .buckets
            .iter()
            .rev()
            .take(SERIES_POINTS)
            .rev()
            .map(|b| SecondPoint {
                sec: b.sec,
                reqs: b.reqs,
                errs: b.errs,
                out_tokens: b.out_tokens,
            })
            .collect();
        MetricsSnapshot {
            window_secs: WINDOW_SECS,
            rps: reqs as f64 / secs_covered,
            errors_per_s: errs as f64 / secs_covered,
            tokens_per_s: out_tokens as f64 / secs_covered,
            error_rate: if reqs > 0 {
                errs as f64 / reqs as f64
            } else {
                0.0
            },
            ttft_p50_ms: percentile(&mut scratch, &ttfts, 0.50),
            ttft_p95_ms: percentile(&mut scratch, &ttfts, 0.95),
            ewma_rps: g.ewma_rps,
            ewma_tokens_per_s: g.ewma_tps,
            active_streams,
            series,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_is_zero() {
        let m = Metrics::default();
        let s = m.snapshot(0);
        assert_eq!(s.rps, 0.0);
        assert_eq!(s.tokens_per_s, 0.0);
        assert!(s.ttft_p50_ms.is_none());
    }

    #[test]
    fn counts_and_error_rate_within_window() {
        let m = Metrics::default();
        for _ in 0..8 {
            m.on_completion(true, Some(100), 10);
        }
        for _ in 0..2 {
            m.on_completion(false, None, 0);
        }
        let s = m.snapshot(3);
        assert_eq!(s.active_streams, 3);
        assert!((s.error_rate - 0.2).abs() < 1e-9);
        // 同一秒内：secs_covered=1 → rps=10
        assert!((s.rps - 10.0).abs() < 1e-6);
        assert!((s.tokens_per_s - 80.0).abs() < 1e-6);
        assert_eq!(s.ttft_p50_ms, Some(100));
    }

    #[test]
    fn percentile_interpolates() {
        let vals: Vec<u16> = (1..=100).collect();
        let mut scratch = Vec::new();
        assert_eq!(percentile(&mut scratch, &vals, 0.5), Some(50));
        assert_eq!(percentile(&mut scratch, &vals, 0.95), Some(95));
    }

    #[test]
    fn stale_buckets_are_pruned() {
        let m = Metrics::default();
        m.on_completion(true, None, 5);
        {
            let mut g = m.inner.lock().unwrap();
            // 手工把桶挪到远古时间，再记录一次触发裁剪。
            if let Some(b) = g.buckets.front_mut() {
                b.sec = 1;
                g.last_sec = 1;
            }
        }
        m.on_completion(true, None, 5);
        let s = m.snapshot(0);
        // 远古桶被裁掉，只剩当前秒这一次请求（gap 巨大还会清零 EWMA）。
        assert!(s.rps <= 1.0 + 1e-6);
        assert_eq!(s.ewma_rps, 0.0);
    }

    #[test]
    fn coeffs_are_per_backend_and_model() {
        let m = Metrics::default();
        // 同一模型、两台后端：各自攒样本，互不串味。
        for _ in 0..3 {
            m.note_pre_coeff("b1", "m1", 400, 100); // 0.25 tok/B
            m.note_pre_coeff("b2", "m1", 400, 40); // 0.1 tok/B
        }
        assert!((m.pre_coeff("b1", "m1").unwrap() - 0.25).abs() < 1e-9);
        assert!((m.pre_coeff("b2", "m1").unwrap() - 0.1).abs() < 1e-9);
        assert!(
            m.pre_coeff("b1", "m2").is_none(),
            "没样本的模型对不该凭空出系数"
        );
        // decode 侧独立：prefill 攒够不代表 decode 也有。
        assert!(m.dec_coeff("b1", "m1").is_none());
        for _ in 0..3 {
            m.note_dec_coeff("b1", "m1", 2000, 100); // 0.05 tok/B
        }
        assert!((m.dec_coeff("b1", "m1").unwrap() - 0.05).abs() < 1e-9);
        // 快照带全两对，且按 (backend, model) 稳定排序。
        let pairs = m.pair_snapshot();
        assert_eq!(pairs.len(), 2);
        assert_eq!(
            (pairs[0].backend.as_str(), pairs[0].model.as_str()),
            ("b1", "m1")
        );
        assert_eq!(
            (pairs[1].backend.as_str(), pairs[1].model.as_str()),
            ("b2", "m1")
        );
    }

    #[test]
    fn implausible_samples_are_dropped_and_means_clamped() {
        let m = Metrics::default();
        // 脏样本：1 字节报 5000 token → 拒收，攒不出系数。
        for _ in 0..3 {
            m.note_pre_coeff("b1", "m1", 1, 5000);
        }
        assert!(m.pre_coeff("b1", "m1").is_none());
        // 样本不足 3 不出系数。
        m.note_pre_coeff("b2", "m1", 100, 25);
        m.note_pre_coeff("b2", "m1", 100, 25);
        assert!(m.pre_coeff("b2", "m1").is_none());
        // 均值越上界 → 钳到 1.0（1.5 tok/B 不可能，钳住别带飞速率）。
        for _ in 0..3 {
            m.note_dec_coeff("b3", "m1", 2, 3);
        }
        assert_eq!(m.dec_coeff("b3", "m1"), Some(1.0));
    }

    #[test]
    fn coeff_median_is_outlier_proof() {
        // 实测教训（浮窗速率比账本真值高 50%+）：少数脏样本（封口字节与
        // usage 错位的重试/半截流）比率能比真值高几倍，均值系数被拖飞。
        // 8 个正常样本 0.006 + 2 个 0.2 的离群：均值 0.052（近 9 倍虚高），
        // 中位数应钉在正常簇里（0.006 高于钳制下界 0.001，结果可精确断言）。
        let m = Metrics::default();
        for _ in 0..8 {
            m.note_dec_coeff("b1", "m1", 10000, 60); // 0.006 tok/B 真值
        }
        for _ in 0..2 {
            m.note_dec_coeff("b1", "m1", 100, 20); // 0.2 脏样本（守卫域内但离群）
        }
        let c = m.dec_coeff("b1", "m1").unwrap();
        assert!((c - 0.006).abs() < 1e-9, "中位数应=正常样本，got {c}");
        // 环容量：持续喂离群，最老样本被挤出环后中位数跟着翻转（可冲刷）。
        for _ in 0..COEFF_RING {
            m.note_dec_coeff("b1", "m1", 100, 20);
        }
        let c2 = m.dec_coeff("b1", "m1").unwrap();
        assert!(
            (c2 - 0.2).abs() < 1e-9,
            "环被离群刷满后中位数应=0.2, got {c2}"
        );
    }

    #[test]
    fn prefill_p99_is_per_pair() {
        let m = Metrics::default();
        for tps in [100.0, 200.0, 300.0] {
            m.note_prefill_rate("b1", "m1", tps);
        }
        assert_eq!(m.prefill_p99("b1", "m1"), Some(300.0));
        assert!(m.prefill_p99("b2", "m1").is_none(), "P99 也得分后端");
    }

    #[test]
    fn engine_rates_snapshot_is_single_sample_truth_and_per_pair() {
        let m = Metrics::default();
        // 引擎真值不设 ≥3 门槛：一次实测即真值（区别于学习出来的纠偏系数）。
        m.note_engine_rates("unsloth", "q35", Some(2000.0), Some(50.0));
        let pairs = m.pair_snapshot();
        let p = pairs
            .iter()
            .find(|p| p.backend == "unsloth" && p.model == "q35")
            .unwrap();
        assert!((p.engine_prefill_tps.unwrap() - 2000.0).abs() < 1e-9);
        assert!((p.engine_dec_tps.unwrap() - 50.0).abs() < 1e-9);
        assert_eq!(p.engine_n, 1);
        // 单侧喂样（半截流只有 prefill 真值）：另一侧保持 None，计数照加。
        m.note_engine_rates("unsloth", "q35", Some(1800.0), None);
        let p = m
            .pair_snapshot()
            .into_iter()
            .find(|p| p.backend == "unsloth" && p.model == "q35")
            .unwrap();
        assert_eq!(p.engine_n, 2);
        assert!(p.engine_dec_tps.is_some());
        // 中位数抗离群：3 个 50 + 1 个 500 的离群，快照钉在正常簇。
        for _ in 0..2 {
            m.note_engine_rates("unsloth", "q35", None, Some(50.0));
        }
        m.note_engine_rates("unsloth", "q35", None, Some(500.0));
        let p = m
            .pair_snapshot()
            .into_iter()
            .find(|p| p.backend == "unsloth" && p.model == "q35")
            .unwrap();
        assert!((p.engine_dec_tps.unwrap() - 50.0).abs() < 1e-9);
        // 全 None / 非正数不喂：坏数据不配计数。
        m.note_engine_rates("unsloth", "q35", None, None);
        m.note_engine_rates("unsloth", "q35", Some(0.0), Some(-3.0));
        m.note_engine_rates("", "q35", Some(1.0), None);
        let p = m
            .pair_snapshot()
            .into_iter()
            .find(|p| p.backend == "unsloth" && p.model == "q35")
            .unwrap();
        assert_eq!(p.engine_n, 5, "坏样本不该进账");
        // 没引擎真值的对（纯 vLLM 后端）：字段为 None，不影响纠偏系数出口。
        let p2 = m
            .pair_snapshot()
            .into_iter()
            .find(|p| p.backend == "unsloth" && p.model == "q35")
            .unwrap();
        assert!(p2.pre_coeff.is_none(), "引擎环不喂系数池，两池互不串味");
    }

    #[test]
    fn model_tok_per_s_uses_decode_span_not_bucket_count() {
        let m = Metrics::default();
        // 复刻实拍回归：空闲模型第一条完成（只有 1 个桶）吐 3097 token、
        // decode 跨度 331s。旧口径 3097÷1桶 = 3097 tok/s（华为超节点级），
        // 实际 9.4 tok/s —— tok/s 必须是 Σtoken ÷ Σ跨度。
        m.on_model_completion("m1", "b1", true, Some(1196), 3097, Some(331_000));
        let rows = m.snapshot_models(&Default::default());
        let r = rows.iter().find(|x| x.model == "m1").unwrap();
        assert!(
            (r.tokens_per_s - 3097.0 / 331.0).abs() < 0.01,
            "got {}",
            r.tokens_per_s
        );
        assert_eq!(r.out_tokens, 3097, "累计输出照记");
        assert_eq!(r.reqs, 1);
    }

    #[test]
    fn model_tok_per_s_pairs_span_with_tokens_only() {
        let m = Metrics::default();
        // 没 token 的跨度（上游无 usage 的流）不入成对样本，别把速率拖低；
        // 没跨度的 token（非流式）也不硬除——两者都不许伪造速率。
        m.on_model_completion("m1", "b1", true, Some(100), 0, Some(60_000));
        m.on_model_completion("m1", "b1", true, Some(100), 500, None);
        let rows = m.snapshot_models(&Default::default());
        let r = rows.iter().find(|x| x.model == "m1").unwrap();
        // 成对样本为零 → 退回墙钟兜底（500 ÷ covered）而不是 500/60s 或 0/60s。
        assert!(r.tokens_per_s > 0.0, "got {}", r.tokens_per_s);
        assert_eq!(r.out_tokens, 500);
    }

    #[test]
    fn idle_model_buckets_advance_and_prune_at_snapshot() {
        let m = Metrics::default();
        m.on_model_completion("m1", "b1", true, Some(100), 50, Some(5_000));
        {
            let mut g = m.inner.lock().unwrap();
            let agg = g.models.get_mut("m1").unwrap();
            for b in agg.buckets.iter_mut() {
                b.sec -= MODEL_WINDOW_SECS as u64;
            }
            agg.last_sec = agg.last_sec.saturating_sub(MODEL_WINDOW_SECS as u64);
        }
        let rows = m.snapshot_models(&Default::default());
        let r = rows.iter().find(|x| x.model == "m1").unwrap();
        // 快照即推进：远古桶被裁掉，不再把几小时前的完成端着当「现在」。
        assert_eq!(r.reqs, 0);
        assert_eq!(r.out_tokens, 0);
        assert_eq!(r.tokens_per_s, 0.0);
        assert!(!r.active);
    }

    #[test]
    fn backend_model_buckets_separate_backends() {
        // 切片 17：同名模型两台后端供给——按 (后端, 模型) 出口各记各的真实数字，
        // 按模型出口仍是跨后端聚合（两个出口口径互不派生）。
        let m = Metrics::default();
        m.on_model_completion("m1", "hal", true, Some(100), 300, Some(3_000));
        m.on_model_completion("m1", "spl", true, Some(200), 700, Some(7_000));
        let pairs = m.snapshot_backend_models(&Default::default());
        assert_eq!(pairs.len(), 2);
        let hal = pairs.iter().find(|p| p.backend == "hal").unwrap();
        let spl = pairs.iter().find(|p| p.backend == "spl").unwrap();
        assert_eq!(hal.metrics.out_tokens, 300);
        assert_eq!(spl.metrics.out_tokens, 700);
        assert_eq!(hal.metrics.backends, vec!["hal".to_string()]);
        assert!(hal.metrics.active && spl.metrics.active);
        let agg = m.snapshot_models(&Default::default());
        let row = agg.iter().find(|x| x.model == "m1").unwrap();
        assert_eq!(row.reqs, 2, "按模型出口仍是跨后端聚合");
        assert_eq!(row.out_tokens, 1000);
    }

    #[test]
    fn backend_model_active_streams_keyed_by_pair() {
        let m = Metrics::default();
        m.on_model_completion("m1", "hal", true, Some(100), 300, Some(3_000));
        m.on_model_completion("m1", "spl", true, Some(100), 100, Some(1_000));
        // 在飞只算在 spl 上：hal 行不该跟着亮。
        let mut active = std::collections::HashMap::new();
        active.insert(("spl".to_string(), "m1".to_string()), 2usize);
        let pairs = m.snapshot_backend_models(&active);
        let hal = pairs.iter().find(|p| p.backend == "hal").unwrap();
        let spl = pairs.iter().find(|p| p.backend == "spl").unwrap();
        assert_eq!(hal.metrics.active_streams, 0);
        assert_eq!(spl.metrics.active_streams, 2);
    }
}
