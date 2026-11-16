//! 片4 · 成本计价：按引擎分档（本地不计费 / 在线手工单价），查询期实时计算（不落列不回填）。
//!
//! 口径（D25 + 片4 拍板）：
//! · 分档按后端引擎自动判定——vllm/sglang/llama.cpp/ollama/omlx/lmstudio = 本地不计费，
//!   `custom` / 未填 / 未知 = 在线（才计费）；
//! · 单价纯手工进 `model_meta`（USD / 百万 token，未设 = 不算），内置 `model_prices.json` 已废除；
//! · 精确键匹配（单价挂在模型名上，无前缀猜测）；
//! · 同一模型可同时被本地与在线后端服务，按（模型 × 引擎）拆行分别计。

use crate::engine::Engine;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 单模型四维单价（USD / 百万 token）。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_creation: f64,
}

/// 本地 / 在线分档（Usage 页分列口径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Local,
    Online,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Local => "local",
            Scope::Online => "online",
        }
    }
}

/// 按后端引擎判定分档：本地自托管 = `Local`（不计费）；
/// `custom` / 未填（NULL）/ 未知字符串 = `Online`（才计费，未设单价 = 不算）。
pub fn scope_of(engine: Option<&str>) -> Scope {
    match engine.and_then(Engine::parse) {
        Some(e) if e.is_local() => Scope::Local,
        _ => Scope::Online,
    }
}

/// 定价表 = `model_meta` 手工单价（每次查询现场装载，调价即时生效于历史区间的汇总）。
pub struct PriceTable {
    entries: Vec<(String, Price)>,
}

impl PriceTable {
    pub fn load(store: &Store) -> Self {
        let map = store.model_prices().unwrap_or_default();
        // 键降序：仅为确定性排序，匹配只认精确名。
        let mut entries: Vec<(String, Price)> = map.into_iter().collect();
        entries.sort_by(|a, b| b.0.cmp(&a.0));
        Self { entries }
    }

    /// 精确匹配（大小写不敏感、去空白）；无前缀猜测。
    pub fn match_price(&self, model: &str) -> Option<(&str, &Price)> {
        let m = model.trim().to_lowercase();
        if m.is_empty() {
            return None;
        }
        self.entries
            .iter()
            .find(|(k, _)| k.trim().to_lowercase() == m)
            .map(|(k, p)| (k.as_str(), p))
    }
}

/// 一次算成本（USD）。token 数为 usage_records 分列口径。
pub fn cost_usd(p: &Price, input: i64, output: i64, cache_read: i64, cache_creation: i64) -> f64 {
    (input as f64 * p.input
        + output as f64 * p.output
        + cache_read as f64 * p.cache_read
        + cache_creation as f64 * p.cache_creation)
        / 1_000_000.0
}

/// 区间成本汇总：按（模型 × 引擎）分组 → 分档 → 计价（在线才查单价）。
///
/// `total_cost_usd` 只含在线部分；本地行 `priced = false` 但**不**计入
/// `unpriced_requests`（本地不计费是判定，不是漏配）。
pub fn usage_cost(store: &Store, from: i64, to: i64) -> crate::store::Result<Value> {
    let agg = store.usage_model_totals(from, to)?;
    let table = PriceTable::load(store);
    let mut rows: Vec<Value> = Vec::new();
    let mut total = 0.0_f64;
    let mut priced_reqs = 0_i64;
    let mut online_reqs = 0_i64;
    let mut local_reqs = 0_i64;
    for r in &agg {
        let scope = scope_of(r.engine.as_deref());
        let matched = match scope {
            Scope::Local => None,
            Scope::Online => table.match_price(&r.model),
        };
        let cost = matched.map_or(0.0, |(_, p)| {
            cost_usd(p, r.input, r.output, r.cache_read, r.cache_creation)
        });
        let requests = match scope {
            Scope::Local => {
                local_reqs += r.requests;
                0.0
            }
            Scope::Online => {
                online_reqs += r.requests;
                if matched.is_some() {
                    priced_reqs += r.requests;
                }
                total += cost;
                cost
            }
        };
        rows.push(serde_json::json!({
            "model": r.model,
            "scope": scope.as_str(),
            "requests": r.requests,
            "input_tokens": r.input,
            "output_tokens": r.output,
            "cache_read_tokens": r.cache_read,
            "cache_creation_tokens": r.cache_creation,
            "priced": matched.is_some(),
            "cost_usd": (requests * 1e6).round() / 1e6,
        }));
    }
    Ok(serde_json::json!({
        "total_cost_usd": (total * 1e6).round() / 1e6,
        "local_requests": local_reqs,
        "online_requests": online_reqs,
        "priced_requests": priced_reqs,
        "unpriced_requests": online_reqs - priced_reqs,
        "by_model": rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::types::{BackendUpsert, Protocol, UsageRecord};

    fn store() -> Store {
        Store::in_memory().unwrap()
    }

    /// 建后端（engine：None = 未填 = 在线）。
    fn backend(s: &Store, name: &str, engine: Option<&str>, models: &[&str]) {
        s.upsert_backend(&BackendUpsert {
            name: name.into(),
            base_url: "http://x".into(),
            protocol: Protocol::OpenaiChat,
            enabled: true,
            weight: 1,
            engine: engine.map(Into::into),
            models: models.iter().map(|m| m.to_string()).collect(),
            ..Default::default()
        })
        .unwrap();
    }

    fn rec(backend: &str, model: &str, input: i64, output: i64, cr: i64) -> UsageRecord {
        UsageRecord {
            request_id: format!("req-{model}-{}", rand_suffix()),
            ts: chrono_now(),
            backend: Some(backend.into()),
            model: Some(model.into()),
            entry_protocol: Some("openai".into()),
            client: None,
            status: 200,
            ok: true,
            stream: false,
            ttft_ms: None,
            duration_ms: Some(10),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cr,
            cache_creation_tokens: 0,
            decode_tps: None,
            estimated: false,
            cache_status: None,
            retry_trace: None,
            error: None,
        }
    }

    static SEQ: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    fn rand_suffix() -> i64 {
        SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
    fn chrono_now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }
    fn window() -> (i64, i64) {
        (chrono_now() - 60_000, chrono_now() + 1000)
    }
    fn price<'a>(p: &'a Value, model: &str) -> &'a Value {
        p["by_model"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["model"] == model)
            .unwrap()
    }

    #[test]
    fn scope_by_engine() {
        assert_eq!(scope_of(Some("vllm")), Scope::Local);
        assert_eq!(scope_of(Some("omlx")), Scope::Local);
        assert_eq!(scope_of(Some("lm-studio")), Scope::Local);
        assert_eq!(scope_of(Some("custom")), Scope::Online);
        assert_eq!(scope_of(None), Scope::Online);
        assert_eq!(scope_of(Some("anthropic")), Scope::Online);
    }

    #[test]
    fn exact_match_only_no_prefix() {
        let s = store();
        backend(&s, "b", None, &["gpt-4o"]);
        s.set_model_price(
            "gpt-4o",
            Some(Price {
                input: 2.5,
                output: 10.0,
                cache_read: 0.0,
                cache_creation: 0.0,
            }),
        )
        .unwrap();
        let t = PriceTable::load(&s);
        assert_eq!(t.match_price("gpt-4o").unwrap().0, "gpt-4o");
        assert!(t.match_price("gpt-4o-mini").is_none());
        assert!(t.match_price("  GPT-4O ").is_some());
    }

    #[test]
    fn local_engine_never_costed() {
        let s = store();
        backend(&s, "local", Some("vllm"), &["m"]);
        // 即便挂了单价：本地分档直接跳过，不查价、不计入未计价。
        s.set_model_price(
            "m",
            Some(Price {
                input: 10.0,
                output: 10.0,
                cache_read: 0.0,
                cache_creation: 0.0,
            }),
        )
        .unwrap();
        s.record_usage(&rec("local", "m", 1000, 1000, 0)).unwrap();
        let (from, to) = window();
        let v = usage_cost(&s, from, to).unwrap();
        assert_eq!(v["total_cost_usd"].as_f64().unwrap(), 0.0);
        assert_eq!(v["local_requests"], 1);
        assert_eq!(v["online_requests"], 0);
        assert_eq!(v["unpriced_requests"], 0);
        assert_eq!(price(&v, "m")["scope"], "local");
        assert_eq!(price(&v, "m")["priced"], false);
    }

    #[test]
    fn online_costed_by_manual_price() {
        let s = store();
        backend(&s, "api", None, &["claude-x"]);
        s.set_model_price(
            "claude-x",
            Some(Price {
                input: 3.0,
                output: 15.0,
                cache_read: 0.3,
                cache_creation: 0.0,
            }),
        )
        .unwrap();
        s.record_usage(&rec("api", "claude-x", 1000, 500, 200))
            .unwrap();
        let (from, to) = window();
        let v = usage_cost(&s, from, to).unwrap();
        let expect = (1000.0 * 3.0 + 500.0 * 15.0 + 200.0 * 0.3) / 1e6;
        assert!((v["total_cost_usd"].as_f64().unwrap() - expect).abs() < 1e-6);
        assert_eq!(v["priced_requests"], 1);
        assert_eq!(v["unpriced_requests"], 0);
        assert_eq!(price(&v, "claude-x")["scope"], "online");
    }

    #[test]
    fn online_without_price_is_unpriced() {
        let s = store();
        backend(&s, "api", None, &["mystery-model"]);
        s.record_usage(&rec("api", "mystery-model", 100, 100, 0))
            .unwrap();
        let (from, to) = window();
        let v = usage_cost(&s, from, to).unwrap();
        assert_eq!(v["online_requests"], 1);
        assert_eq!(v["unpriced_requests"], 1);
        assert_eq!(v["total_cost_usd"].as_f64().unwrap(), 0.0);
    }

    #[test]
    fn same_model_local_and_online_split_rows() {
        let s = store();
        backend(&s, "local", Some("omlx"), &["shared"]);
        backend(&s, "api", Some("custom"), &["shared"]);
        s.set_model_price(
            "shared",
            Some(Price {
                input: 1.0,
                output: 2.0,
                cache_read: 0.0,
                cache_creation: 0.0,
            }),
        )
        .unwrap();
        s.record_usage(&rec("local", "shared", 1000, 1000, 0))
            .unwrap();
        s.record_usage(&rec("api", "shared", 1000, 1000, 0))
            .unwrap();
        let (from, to) = window();
        let v = usage_cost(&s, from, to).unwrap();
        assert_eq!(v["local_requests"], 1);
        assert_eq!(v["online_requests"], 1);
        // 只有在线那行计费：1000×1 + 1000×2。
        let expect = (1000.0 * 1.0 + 1000.0 * 2.0) / 1e6;
        assert!((v["total_cost_usd"].as_f64().unwrap() - expect).abs() < 1e-6);
        let rows = v["by_model"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        // by_model 求和 == total。
        let sum: f64 = rows.iter().map(|r| r["cost_usd"].as_f64().unwrap()).sum();
        assert!((sum - v["total_cost_usd"].as_f64().unwrap()).abs() < 1e-6);
    }

    #[test]
    fn price_row_survives_meta_reset() {
        let s = store();
        backend(&s, "api", None, &["m"]);
        s.set_model_price(
            "m",
            Some(Price {
                input: 1.0,
                output: 1.0,
                cache_read: 0.0,
                cache_creation: 0.0,
            }),
        )
        .unwrap();
        // 其余声明回默认 = 删行——但价格还挂着，不能连带删。
        s.set_model_meta("m", 0, 0, 0, 0).unwrap();
        assert!(s.model_prices().unwrap().contains_key("m"));
        // 清价后行才消失。
        assert!(!s.set_model_price("m", None).unwrap());
        assert!(s.model_prices().unwrap().is_empty());
    }

    #[test]
    fn cost_math() {
        let p = Price {
            input: 3.0,
            output: 15.0,
            cache_read: 0.3,
            cache_creation: 3.75,
        };
        // 1M in + 1M out + 1M cache read = 3 + 15 + 0.3
        let c = cost_usd(&p, 1_000_000, 1_000_000, 1_000_000, 0);
        assert!((c - 18.3).abs() < 1e-9);
    }

    #[test]
    fn price_requires_served_model_and_non_negative() {
        let s = store();
        assert!(s
            .set_model_price(
                "ghost",
                Some(Price {
                    input: 1.0,
                    output: 1.0,
                    cache_read: 0.0,
                    cache_creation: 0.0
                })
            )
            .is_err());
        backend(&s, "api", None, &["m"]);
        assert!(s
            .set_model_price(
                "m",
                Some(Price {
                    input: -1.0,
                    output: 1.0,
                    cache_read: 0.0,
                    cache_creation: 0.0
                })
            )
            .is_err());
    }
}
