//! 参数目录（M5-B 定义、M5-A 消费）：分组、已知键全集与默认策略。
//!
//! 「已知键」是能力门控的前提：目录内的键受后端能力表约束；目录外的
//! 自定义键（各家引擎私有参数）不受限，直接放行——逃生舱语义。

/// 一个目录条目：键名 + 所属组 + 默认策略。
pub struct Entry {
    pub key: &'static str,
    pub group: &'static str,
    /// 默认策略：一般 `fill`；`max_tokens` 用 `clamp`（护栏语义）。
    pub default_policy: &'static str,
}

pub const ENTRIES: &[Entry] = &[
    // 采样
    e("temperature", "sampling", "fill"),
    e("top_p", "sampling", "fill"),
    e("top_k", "sampling", "fill"),
    e("min_p", "sampling", "fill"),
    e("seed", "sampling", "fill"),
    e("stop", "sampling", "fill"),
    e("logprobs", "sampling", "fill"),
    // 惩罚
    e("repetition_penalty", "penalties", "fill"),
    e("frequency_penalty", "penalties", "fill"),
    e("presence_penalty", "penalties", "fill"),
    e("best_of", "penalties", "fill"),
    e("beam_width", "penalties", "fill"),
    // 长度
    e("max_tokens", "length", "clamp"),
    e("max_completion_tokens", "length", "clamp"),
    e("min_tokens", "length", "fill"),
    e("ctx", "length", "fill"),
    e("truncate_prompt_tokens", "length", "fill"),
    // 结构化输出
    e("response_format", "structured", "fill"),
    e("guided_json", "structured", "fill"),
    e("guided_regex", "structured", "fill"),
    e("guided_choice", "structured", "fill"),
    e("strict_tools", "structured", "fill"),
    // VLM 多模态
    e("limit_mm_per_prompt", "vlm", "fill"),
    e("mm_processor_kwargs", "vlm", "fill"),
    e("image_detail", "vlm", "fill"),
    e("allowed_media", "vlm", "fill"),
    e("video_fps", "vlm", "fill"),
    e("video_max_frames", "vlm", "fill"),
    // 投机解码 / MTP
    e("speculative_algorithm", "speculative", "fill"),
    e("num_speculative_tokens", "speculative", "fill"),
    e("speculative_draft_model", "speculative", "fill"),
    e("mtp_layers", "speculative", "fill"),
    e("include_draft_logits", "speculative", "fill"),
    // reasoning
    e("thinking_budget", "reasoning", "fill"),
    e("reasoning_effort", "reasoning", "fill"),
    e("strip_inline_think", "reasoning", "fill"),
    // 引擎运行时（只读展示，代理不下发；列在这里只为 UI 渲染与灰显）
    e("gpu_memory_utilization", "runtime", "fill"),
    e("swap_space", "runtime", "fill"),
    e("tensor_parallel", "runtime", "fill"),
    e("enable_prefix_caching", "runtime", "fill"),
    e("chunked_prefill", "runtime", "fill"),
    e("kv_cache_dtype", "runtime", "fill"),
    e("enforce_eager", "runtime", "fill"),
];

const fn e(key: &'static str, group: &'static str, default_policy: &'static str) -> Entry {
    Entry {
        key,
        group,
        default_policy,
    }
}

/// 目录内已知键（能力门控的作用域）。
pub fn is_known_key(key: &str) -> bool {
    ENTRIES.iter().any(|en| en.key == key)
}

/// 数值键（值形状守卫）：这些键的值必须是数字——空串/垃圾字符串一旦下发，
/// 上游直接 400（`top_p: ""` 事故）。目录外的自定义键不算（未知即未知）。
pub fn is_numeric_key(key: &str) -> bool {
    matches!(
        key,
        // 采样
        "temperature"
            | "top_p"
            | "top_k"
            | "min_p"
            | "seed"
            | "logprobs"
        // 惩罚
            | "repetition_penalty"
            | "frequency_penalty"
            | "presence_penalty"
            | "best_of"
            | "beam_width"
        // 长度
            | "max_tokens"
            | "max_completion_tokens"
            | "min_tokens"
            | "ctx"
            | "truncate_prompt_tokens"
        // VLM
            | "video_fps"
            | "video_max_frames"
        // 投机解码
            | "num_speculative_tokens"
            | "mtp_layers"
        // reasoning
            | "thinking_budget"
        // 引擎运行时（只读展示，但仍是数值）
            | "gpu_memory_utilization"
            | "swap_space"
            | "tensor_parallel"
    )
}

/// 长度量纲键（片2 R4：0=继承**只对这些键**成立；采样等键的显式 0 是真值）。
/// 目录外的自定义键不算——未知即未知，不按 0=继承解释。
pub fn is_length_key(key: &str) -> bool {
    ENTRIES
        .iter()
        .any(|en| en.key == key && en.group == "length")
}

/// 某组的键集合（UI 分组渲染 / 能力预设构造用）。
pub fn keys_of_group(group: &str) -> Vec<&'static str> {
    ENTRIES
        .iter()
        .filter(|en| en.group == group)
        .map(|en| en.key)
        .collect()
}

/// 目录默认策略；未知键默认 `fill`。
pub fn default_policy(key: &str) -> &'static str {
    ENTRIES
        .iter()
        .find(|en| en.key == key)
        .map(|en| en.default_policy)
        .unwrap_or("fill")
}
