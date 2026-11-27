//! 引擎能力表（M5-A）：内置预设 ⊕ 后端声明 → 生效能力集。
//!
//! 生效规则：先取 `engine` 预设键集，再用后端声明（yaml `capabilities`
//! 或 UI 手工勾选，存 backends.capabilities 列）做**增量覆盖**：
//! 声明里出现的组以其内容为准（可与预设并集 `"+key"` 前缀暂不支持，
//! 语义就是替换）。VLM / 投机两组用对象声明（`{"supported":true}` /
//! `{"algorithm":[...]}`），命中即把该组全部键并入。
//!
//! 出站注入只对 `catalog::is_known_key` 的键做门控；目录外自定义键放行。

use crate::gateway::catalog;
use serde_json::Value;
use std::collections::HashSet;

/// 后端生效能力：支持的目录键集合。
#[derive(Debug, Clone, Default)]
pub struct Caps {
    keys: HashSet<String>,
    /// 是否有任何来源（预设或声明）参与——空集时门控退化为「全放行」。
    constrained: bool,
}

impl Caps {
    pub fn supports(&self, key: &str) -> bool {
        !self.constrained || self.keys.contains(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    /// 序列化回 JSON（探活合并结果写回 capabilities 列时用）。
    pub fn to_json(&self) -> Value {
        let mut sorted: Vec<&str> = self.keys.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        Value::Array(
            sorted
                .iter()
                .map(|k| Value::String((*k).to_string()))
                .collect(),
        )
    }
}

/// 引擎预设的组→键映射。未知/custom 走保守全集（不含 vlm/speculative）。
fn preset_groups(engine: Option<&str>) -> Vec<(&'static str, Vec<&'static str>)> {
    let basic_sampling = catalog::keys_of_group("sampling");
    let basic_penalties = catalog::keys_of_group("penalties");
    let basic_length = catalog::keys_of_group("length");
    match engine.unwrap_or("custom") {
        "vllm" => vec![
            ("sampling", basic_sampling),
            ("penalties", basic_penalties),
            ("length", basic_length),
            (
                "structured",
                vec![
                    "response_format",
                    "guided_json",
                    "guided_regex",
                    "guided_choice",
                ],
            ),
            ("vlm", catalog::keys_of_group("vlm")),
            (
                "speculative",
                vec!["speculative_algorithm", "num_speculative_tokens"],
            ),
            ("reasoning", vec!["reasoning_effort"]),
        ],
        "sglang" => vec![
            ("sampling", basic_sampling),
            ("penalties", basic_penalties),
            ("length", basic_length),
            (
                "structured",
                vec![
                    "response_format",
                    "guided_json",
                    "guided_regex",
                    "guided_choice",
                    "strict_tools",
                ],
            ),
            ("vlm", catalog::keys_of_group("vlm")),
            ("speculative", catalog::keys_of_group("speculative")),
            ("reasoning", vec!["reasoning_effort", "thinking_budget"]),
        ],
        "llama.cpp" => vec![
            ("sampling", basic_sampling),
            (
                "penalties",
                vec!["frequency_penalty", "presence_penalty", "min_p"],
            ),
            ("length", vec!["max_tokens", "max_completion_tokens", "ctx"]),
            ("structured", vec!["response_format"]),
            ("vlm", vec!["image_detail", "allowed_media"]),
            ("reasoning", vec!["reasoning_effort"]),
        ],
        "ollama" => vec![
            (
                "sampling",
                vec!["temperature", "top_p", "top_k", "seed", "stop"],
            ),
            ("length", vec!["max_tokens", "ctx"]),
            ("structured", vec!["response_format"]),
        ],
        "lmstudio" => vec![
            (
                "sampling",
                vec!["temperature", "top_p", "top_k", "seed", "stop"],
            ),
            ("length", vec!["max_tokens", "max_completion_tokens"]),
            ("structured", vec!["response_format"]),
            ("vlm", vec!["image_detail", "allowed_media"]),
        ],
        // custom / 未知引擎：开放常规三组 + response_format，不开 vlm/投机。
        _ => vec![
            ("sampling", basic_sampling),
            ("penalties", basic_penalties),
            ("length", basic_length),
            ("structured", vec!["response_format"]),
        ],
    }
}

/// 由 engine 预设 + 声明 JSON 计算生效能力。
///
/// 声明 JSON 兼容两种形状：
/// 1. 计划形态：`{"sampling":[...],"vlm":{"supported":true},"speculative":{...}}`
/// 2. 扁平数组（本模块 `to_json` 产出，探活写回）：`["temperature", ...]`
pub fn effective(engine: Option<&str>, declared: &Value) -> Caps {
    let mut keys: HashSet<String> = HashSet::new();
    for (_group, ks) in preset_groups(engine) {
        for k in ks {
            keys.insert(k.to_string());
        }
    }
    if let Some(arr) = declared.as_array() {
        // 扁平数组 = 权威键集，直接替换。
        keys.clear();
        for k in arr.iter().filter_map(Value::as_str) {
            keys.insert(k.to_string());
        }
    } else if let Some(obj) = declared.as_object() {
        for group in ["sampling", "penalties", "length", "structured", "runtime"] {
            if let Some(arr) = obj.get(group).and_then(Value::as_array) {
                // 组内增量：预设该组先剔除，再按声明加入（替换语义）。
                for (_, ks) in preset_groups(engine).iter().filter(|(g, _)| *g == group) {
                    for k in ks {
                        keys.remove(*k);
                    }
                }
                for k in arr.iter().filter_map(Value::as_str) {
                    keys.insert(k.to_string());
                }
            }
        }
        if let Some(vlm) = obj.get("vlm") {
            if vlm.get("supported").and_then(Value::as_bool) == Some(true) {
                for k in catalog::keys_of_group("vlm") {
                    keys.insert(k.to_string());
                }
            } else {
                // 显式关闭（或未给 supported）：把预设并入的 vlm 组剔掉。
                for k in catalog::keys_of_group("vlm") {
                    keys.remove(k);
                }
            }
        }
        if obj.contains_key("speculative") {
            for k in catalog::keys_of_group("speculative") {
                keys.insert(k.to_string());
            }
        }
    }
    Caps {
        keys,
        constrained: true,
    }
}

/// 尽力自动探测引擎类型（M5-E）：探活成功后对空白后端调用。
/// 只识别有独特指纹的端点；探不到返回 None（保持 engine 空白 → custom 预设）。
pub async fn detect_engine(client: &reqwest::Client, base: &str) -> Option<&'static str> {
    let base = base.trim_end_matches('/');
    // ollama：/api/version 独有。
    if let Ok(resp) = client.get(format!("{base}/api/version")).send().await {
        if resp.status().is_success() {
            if let Ok(j) = resp.json::<Value>().await {
                if j.get("version").is_some() {
                    return Some("ollama");
                }
            }
        }
    }
    // vLLM / SGLang 都有 /server_info，指纹字段不同。
    if let Ok(resp) = client.get(format!("{base}/server_info")).send().await {
        if resp.status().is_success() {
            if let Ok(j) = resp.json::<Value>().await {
                if j.get("vllm_config").is_some() {
                    return Some("vllm");
                }
                if j.get("internal_states").is_some() || j.get("disaggregation_mode").is_some() {
                    return Some("sglang");
                }
            }
        }
    }
    // llama.cpp 的 /props 带 default_generation_settings；SGLang 的带 max_total_num_tokens。
    if let Ok(resp) = client.get(format!("{base}/props")).send().await {
        if resp.status().is_success() {
            if let Ok(j) = resp.json::<Value>().await {
                if j.get("default_generation_settings").is_some() {
                    return Some("llama.cpp");
                }
                if j.get("max_total_num_tokens").is_some() {
                    return Some("sglang");
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sglang_preset_has_speculative_and_vlm() {
        let caps = effective(Some("sglang"), &json!({}));
        assert!(caps.supports("speculative_algorithm"));
        assert!(caps.supports("limit_mm_per_prompt"));
        assert!(caps.supports("temperature"));
    }

    #[test]
    fn custom_preset_blocks_vlm_and_speculative() {
        let caps = effective(Some("custom"), &json!({}));
        assert!(caps.supports("temperature"));
        assert!(!caps.supports("limit_mm_per_prompt"));
        assert!(!caps.supports("speculative_algorithm"));
    }

    #[test]
    fn declared_group_replaces_preset_group() {
        let caps = effective(
            Some("vllm"),
            &json!({"penalties": ["repetition_penalty"], "vlm": {"supported": false}}),
        );
        assert!(caps.supports("repetition_penalty"));
        assert!(!caps.supports("frequency_penalty")); // 组替换
        assert!(!caps.supports("limit_mm_per_prompt")); // vlm 显式关闭（预设并入后被替换组不影响，需重新计算）
    }

    #[test]
    fn flat_array_is_authoritative() {
        let caps = effective(Some("vllm"), &json!(["temperature", "max_tokens"]));
        assert!(caps.supports("temperature"));
        assert!(!caps.supports("top_p"));
    }
}
