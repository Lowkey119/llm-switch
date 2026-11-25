//! 上游 KV 遥测采集 + 缓存感知打分（M5.5-D，计划 §5.5.7）。
//!
//! 边界（D11/既定决策）：上游引擎的 KV 缓存本体不可搬运，这里只做
//! 「遥测打分 + 亲和」——周期拉各引擎自己的指标端点，产出一个决胜分数，
//! 让 `cache_aware` 路由把同分候选按「谁的缓存更可能帮到我们」排序。
//! 任何端点缺失/超时都优雅退化（该信号缺席，分数照常可算）。

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

/// 单个后端的 KV 遥测快照（缺项 = None，打分时取中性值）。
#[derive(Clone, Debug, Default, Serialize)]
pub struct KvTelemetry {
    /// KV 池占用率 0..1（越空越不容易把我们的前缀挤掉）。
    pub kv_usage: Option<f64>,
    /// 前缀缓存命中率 0..1（累计口径）。
    pub prefix_hit_rate: Option<f64>,
    /// 排队请求数（负载惩罚项）。
    pub waiting: Option<f64>,
    /// 是否有 KV 磁盘存活层（仅 omlx：前缀能保留得更久）。
    pub has_ssd_kv: bool,
    /// SSD 层命中率（omlx）。
    pub ssd_hit_rate: Option<f64>,
    pub fetched_at_ms: i64,
}

/// 极简 Prometheus 文本解析：`name{labels} value` → name→首个数值。
pub fn parse_prometheus(text: &str) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // name{label="v"} 1.23 或 name 1.23
        let Some(sp) = line.find([' ', '\t']) else {
            continue;
        };
        let (name_part, rest) = line.split_at(sp);
        let Ok(value) = rest.trim().parse::<f64>() else {
            continue;
        };
        let name = name_part.split('{').next().unwrap_or(name_part);
        out.entry(name.to_string()).or_insert(value);
    }
    out
}

fn prom_get(m: &HashMap<String, f64>, names: &[&str]) -> Option<f64> {
    names.iter().find_map(|n| m.get(*n).copied())
}

fn ratio(hits: Option<f64>, queries: Option<f64>) -> Option<f64> {
    match (hits, queries) {
        (Some(h), Some(q)) if q > 0.0 => Some((h / q).clamp(0.0, 1.0)),
        _ => None,
    }
}

/// 传输错误或 HTTP 非 2xx 都视为「该端点不存在」（优雅退化的入口）。
async fn ok_text(fut: Result<reqwest::Response, reqwest::Error>) -> Option<String> {
    let resp = fut.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().await.ok()
}

/// 深度搜索 JSON 里第一个名为 key 的非负数值（omlx 载荷层级深且不稳定）。
pub fn json_find_number(v: &Value, key: &str) -> Option<f64> {
    match v {
        Value::Object(map) => {
            if let Some(num) = map.get(key).and_then(|x| x.as_f64().filter(|f| *f >= 0.0)) {
                return Some(num);
            }
            map.values().find_map(|child| json_find_number(child, key))
        }
        Value::Array(arr) => arr.iter().find_map(|child| json_find_number(child, key)),
        _ => None,
    }
}

/// 按引擎预设拉一次遥测；一切失败都返回 None（优雅退化）。
pub async fn collect(
    client: &reqwest::Client,
    base_url: &str,
    engine: Option<&str>,
) -> Option<KvTelemetry> {
    let base = base_url.trim_end_matches('/');
    let now = crate::types::now_ms();
    let mut t = KvTelemetry {
        fetched_at_ms: now,
        ..Default::default()
    };
    match engine {
        Some("vllm") => {
            let text = ok_text(client.get(format!("{base}/metrics")).send().await).await?;
            let m = parse_prometheus(&text);
            t.kv_usage = prom_get(&m, &["vllm:gpu_cache_usage_perc", "gpu_cache_usage_perc"]);
            t.prefix_hit_rate = ratio(
                prom_get(
                    &m,
                    &[
                        "vllm:prefix_cache_hits_total",
                        "vllm:prefix_cache_hits",
                        "prefix_cache_hits_total",
                    ],
                ),
                prom_get(
                    &m,
                    &[
                        "vllm:prefix_cache_queries_total",
                        "vllm:prefix_cache_queries",
                        "prefix_cache_queries_total",
                    ],
                ),
            );
            t.waiting = prom_get(&m, &["vllm:num_requests_waiting", "num_requests_waiting"]);
        }
        Some("sglang") => {
            let text = ok_text(client.get(format!("{base}/metrics")).send().await).await?;
            let m = parse_prometheus(&text);
            t.kv_usage = prom_get(&m, &["sglang:token_usage", "sglang:token_usage_percent"]);
            t.prefix_hit_rate = prom_get(&m, &["sglang:cache_hit_rate"]);
            t.waiting = prom_get(
                &m,
                &["sglang:num_waiting_reqs", "sglang:num_waiting_requests"],
            );
        }
        Some("llama.cpp") => {
            // llama.cpp 的指标命名不稳定：抓到就算，抓不到中性。
            if let Some(text) = ok_text(client.get(format!("{base}/metrics")).send().await).await {
                let m = parse_prometheus(&text);
                if let Some(u) = prom_get(&m, &["llama:cache_usage", "cache_usage_perc"]) {
                    t.kv_usage = Some(u);
                }
                if let Some(h) = prom_get(&m, &["llama:cache_hit_rate", "cache_hit_rate"]) {
                    t.prefix_hit_rate = Some(h);
                }
                if let Some(w) = prom_get(&m, &["llama:requests_waiting", "requests_waiting"]) {
                    t.waiting = Some(w);
                }
            }
        }
        Some("ollama") => {
            // /api/ps 只说明「活着且有模型驻留」，无数值信号。
            ok_text(client.get(format!("{base}/api/ps")).send().await).await?;
        }
        Some("omlx") => {
            let resp = client.get(format!("{base}/api/stats")).send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let v: Value = resp.json().await.ok()?;
            t.has_ssd_kv = v.get("runtime_cache").is_some();
            t.ssd_hit_rate =
                json_find_number(&v, "ssd_hit_rate").or_else(|| json_find_number(&v, "hit_rate"));
            t.kv_usage = json_find_number(&v, "utilization");
            t.prefix_hit_rate = json_find_number(&v, "prefix_hit_rate");
        }
        _ => return None,
    }
    Some(t)
}

/// 缓存感知决胜分数（越大越优）。各项定义见 §5.5.7：
/// w1 前缀亲和置信度（已含时间衰减）｜w2 (1-KV占用)｜w3 前缀命中率｜
/// w4 存活层深度（SSD）｜w5 排队负载惩罚。
pub fn score(t: Option<&KvTelemetry>, affinity_conf: f64) -> f64 {
    // w1 故意大于其它项之和：「谁曾服务过这个前缀」是最强信号（计划意图）。
    const W1: f64 = 2.0;
    const W2: f64 = 0.8;
    const W3: f64 = 0.6;
    const W4: f64 = 0.3;
    const W5: f64 = 0.5;
    let neutral = KvTelemetry::default();
    let t = t.unwrap_or(&neutral);
    let free = t.kv_usage.map(|u| 1.0 - u.clamp(0.0, 1.0)).unwrap_or(0.5);
    let hit = t.prefix_hit_rate.unwrap_or(0.0).clamp(0.0, 1.0);
    let depth = if t.has_ssd_kv {
        0.5 + 0.5 * t.ssd_hit_rate.unwrap_or(0.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let load = (t.waiting.unwrap_or(0.0) / 10.0).clamp(0.0, 1.0);
    W1 * affinity_conf.max(0.0) + W2 * free + W3 * hit + W4 * depth - W5 * load
}

/// 监控面快照：后端名 + KV 遥测（REST /admin/telemetry 与 IPC 共用，M5.5-E）。
pub fn snapshot_json(st: &crate::gateway::State) -> Vec<serde_json::Value> {
    let backends: HashMap<i64, (String, Option<String>)> = st
        .store
        .list_backends()
        .unwrap_or_default()
        .into_iter()
        .map(|b| (b.id, (b.name, b.engine)))
        .collect();
    let tm = st.telemetry.read().expect("telemetry lock poisoned");
    let now = crate::types::now_ms();
    let mut out: Vec<serde_json::Value> = tm
        .iter()
        .filter_map(|(id, t)| {
            let (name, engine) = backends.get(id)?.clone();
            Some(serde_json::json!({
                "backend": name,
                "engine": engine,
                "kv_usage": t.kv_usage,
                "prefix_hit_rate": t.prefix_hit_rate,
                "waiting": t.waiting,
                "has_ssd_kv": t.has_ssd_kv,
                "ssd_hit_rate": t.ssd_hit_rate,
                "age_ms": now - t.fetched_at_ms,
            }))
        })
        .collect();
    out.sort_by(|a, b| {
        a["backend"]
            .as_str()
            .unwrap_or("")
            .cmp(b["backend"].as_str().unwrap_or(""))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prom_parses_labels_and_first_wins() {
        let text = "# HELP x\n# TYPE x gauge\nvllm:gpu_cache_usage_perc{model=\"a\"} 0.42\nvllm:gpu_cache_usage_perc 0.9\nbad line\nvllm:num_requests_waiting 3";
        let m = parse_prometheus(text);
        assert_eq!(m.get("vllm:gpu_cache_usage_perc"), Some(&0.42));
        assert_eq!(m.get("vllm:num_requests_waiting"), Some(&3.0));
    }

    #[test]
    fn hit_ratio_guards_zero_division() {
        assert_eq!(ratio(Some(3.0), Some(0.0)), None);
        assert_eq!(ratio(Some(3.0), Some(6.0)), Some(0.5));
    }

    #[test]
    fn json_deep_search_finds_nested_numbers() {
        let v = json!({"runtime_cache": {"models": [{"runtime": {"hit_rate": 0.7}}]}, "x": {"utilization": 0.3}});
        assert_eq!(json_find_number(&v, "hit_rate"), Some(0.7));
        assert_eq!(json_find_number(&v, "utilization"), Some(0.3));
        assert_eq!(json_find_number(&v, "nope"), None);
    }

    #[test]
    fn score_prefers_free_and_ssd_backends() {
        let busy = KvTelemetry {
            kv_usage: Some(0.95),
            waiting: Some(8.0),
            ..Default::default()
        };
        let free = KvTelemetry {
            kv_usage: Some(0.05),
            waiting: Some(0.0),
            prefix_hit_rate: Some(0.4),
            ..Default::default()
        };
        assert!(score(Some(&free), 0.0) > score(Some(&busy), 0.0));
        // 遥测缺席 → 中性分，不报错也不主导。
        let neutral = score(None, 0.0);
        assert!(neutral > score(Some(&busy), 0.0));
        // 亲和权重最大：钉定的后端即使忙也应赢过空闲陌生后端。
        assert!(score(Some(&busy), 1.0) > score(Some(&free), 0.0));
        // SSD 存活层加分。
        let ssd = KvTelemetry {
            has_ssd_kv: true,
            ssd_hit_rate: Some(0.8),
            ..Default::default()
        };
        assert!(score(Some(&ssd), 0.0) > score(Some(&KvTelemetry::default()), 0.0));
    }
}
