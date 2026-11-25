//! Inference engine presets.
//!
//! A backend's `engine` decides three things the rest of the app relies on:
//! - which parameter knobs exist (capability descriptor, DEVELOPMENT-PLAN §5.1)
//! - where its own dashboard lives (Console page, M7)
//! - where its KV-cache telemetry lives (cache-aware routing, §5.5.7)
//!
//! Everything is overridable per backend in the config file / UI.

use crate::types::Protocol;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    Vllm,
    Sglang,
    LlamaCpp,
    Ollama,
    /// oMLX — MLX server with tiered (memory + SSD) KV cache. First-class per D8.
    Omlx,
    /// halogen — ROCm 推理引擎（102 那台机器）。控制台原生接管（SSH 控制面），
    /// 不走 iframe：状态/指标/日志/启停/版本切换由 `services::halctl` 直连。
    Halogen,
    /// gufo — ROCm 10.1 容器化引擎（与 halogen 同机同端口互斥）。控制台同 halogen
    /// 形态原生接管：SSH 控制面走 guctl 机位（/mnt/data/gufo 的 gufo.sh/gu-ver.sh）。
    /// gufo — ROCm 10.1 容器化引擎（与 halogen 同机同端口互斥）。控制台同 halogen
    /// 形态原生接管：SSH 控制面走 guctl 机位（/mnt/data/gufo 的 gufo.sh/gu-ver.sh）。
    Gufo,
    /// Unsloth Studio（llama-server 内核）——自带 Studio API，含**模型级采样默认值**
    /// 端点（`/api/settings/model-sampling-defaults`），参数中心的下发走它
    /// （`services::param_push`）。
    Unsloth,
    LmStudio,
    Custom,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Vllm => "vllm",
            Engine::Sglang => "sglang",
            Engine::LlamaCpp => "llama.cpp",
            Engine::Ollama => "ollama",
            Engine::Omlx => "omlx",
            Engine::Halogen => "halogen",
            Engine::Gufo => "gufo",
            Engine::Unsloth => "unsloth",
            Engine::LmStudio => "lmstudio",
            Engine::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Option<Engine> {
        match s.to_ascii_lowercase().replace([' ', '-', '_'], "").as_str() {
            "vllm" => Some(Engine::Vllm),
            "sglang" => Some(Engine::Sglang),
            "llamacpp" | "llamacppserver" => Some(Engine::LlamaCpp),
            "ollama" => Some(Engine::Ollama),
            "omlx" => Some(Engine::Omlx),
            "halogen" => Some(Engine::Halogen),
            "gufo" => Some(Engine::Gufo),
            "unsloth" | "unslothstudio" => Some(Engine::Unsloth),
            "lmstudio" => Some(Engine::LmStudio),
            "custom" | "" => Some(Engine::Custom),
            _ => None,
        }
    }

    /// 本地自托管推理引擎（D25 费用分档）：本地不计费；`custom` = 在线 API（才计费）。
    pub fn is_local(self) -> bool {
        !matches!(self, Engine::Custom)
    }

    /// Default upstream protocol for this engine (still user-overridable).
    pub fn default_protocol(self) -> Protocol {
        match self {
            Engine::Vllm | Engine::Sglang | Engine::LlamaCpp | Engine::Ollama | Engine::Omlx => {
                Protocol::OpenaiChat
            }
            Engine::LmStudio
            | Engine::Custom
            | Engine::Halogen
            | Engine::Gufo
            | Engine::Unsloth => Protocol::OpenaiChat,
        }
    }

    /// Path of the engine's own web UI, appended to `base_url` when not set.
    pub fn default_webui_path(self) -> Option<&'static str> {
        match self {
            Engine::Vllm => Some("/docs"),
            Engine::Sglang => Some("/server_info"),
            Engine::LlamaCpp => Some("/"),
            Engine::Ollama => Some("/"),
            Engine::Omlx => Some("/"),
            // Unsloth Studio 的 WebUI 就在 API 同端口根路径（8888）。
            Engine::Unsloth => Some("/"),
            Engine::LmStudio => Some("/"),
            // halogen 控制台原生接管（SSH 控制面），不走 iframe，故无默认入口路径。
            Engine::Halogen | Engine::Gufo | Engine::Custom => None,
        }
    }

    /// Metrics endpoint used by the cache-aware router (§5.5.7).
    pub fn telemetry_path(self) -> Option<&'static str> {
        match self {
            Engine::Vllm | Engine::Sglang | Engine::LlamaCpp => Some("/metrics"),
            Engine::Ollama => Some("/api/ps"),
            Engine::Omlx => Some("/api/admin/cache/stats"),
            // halogen 的 /metrics 是 llamacpp 格式，但控制台面板自己经 SSH 读取展示；
            // 这里保持 None，与原先 engine=custom 的路由行为一致，不擅自开启 cache-aware 遥测。
            Engine::LmStudio | Engine::Custom | Engine::Halogen | Engine::Gufo => None,
            // Unsloth Studio 没有 vLLM 那套 /metrics，cache-aware 遥测不擅自接管
            // （与 halogen 同口径）；它的推理状态走 Studio 自己的端点。
            Engine::Unsloth => None,
        }
    }

    /// Whether the engine keeps KV resident beyond GPU memory. Feeds the
    /// "residency depth" bonus in the routing score (§5.5.7).
    pub fn has_disk_kv_tier(self) -> bool {
        matches!(self, Engine::Omlx)
    }

    /// Baseline capability descriptor. Probing may widen it at runtime.
    pub fn capabilities(self) -> Value {
        let common_penalties = json!([
            "repetition_penalty",
            "frequency_penalty",
            "presence_penalty"
        ]);
        match self {
            Engine::Vllm => json!({
                "sampling": ["temperature","top_p","top_k","min_p","seed","stop","logprobs"],
                "penalties": common_penalties,
                "length": ["max_tokens","min_tokens","ctx","truncate_prompt_tokens"],
                "structured": ["json_schema","regex","choice"],
                "vlm": { "supported": true, "image": 1, "video": 0 },
                "speculative": { "algorithm": ["eagle","eagle3","medusa","ngram"], "max_tokens": 8 },
                "runtime": ["gpu_memory_utilization","enable_prefix_caching","chunked_prefill","kv_cache_dtype","enforce_eager"],
                "restart_required": ["ctx","gpu_memory_utilization","tensor_parallel","enforce_eager"]
            }),
            Engine::Sglang => json!({
                "sampling": ["temperature","top_p","top_k","min_p","seed","stop"],
                "penalties": common_penalties,
                "length": ["max_tokens","min_tokens","ctx"],
                "structured": ["json_schema","regex","choice"],
                "vlm": { "supported": true, "image": 4, "video": 1 },
                "speculative": { "algorithm": ["eagle3","nextn","standalone"], "max_tokens": 4 },
                "runtime": ["mem_fraction_static","enable_radix_cache","chunked_prefill_size"],
                "restart_required": ["ctx","mem_fraction_static"]
            }),
            Engine::LlamaCpp => json!({
                "sampling": ["temperature","top_p","top_k","min_p","seed","stop","dry"],
                "penalties": json!(["repeat_penalty","frequency_penalty","presence_penalty","dry_multiplier"]),
                "length": ["max_tokens","ctx","n_keep"],
                "structured": ["json_schema","regex","choice"],
                "vlm": { "supported": true, "image": 1, "video": 0 },
                "speculative": { "algorithm": ["draft_model","lookahead"], "max_tokens": 16 },
                "runtime": ["n_gpu_layers","parallel","cache_type_k","cache_type_v"],
                "restart_required": ["ctx","n_gpu_layers","parallel"]
            }),
            Engine::Ollama => json!({
                "sampling": ["temperature","top_p","top_k","min_p","seed","stop","num_predict"],
                "penalties": json!(["repeat_penalty","presence_penalty","frequency_penalty"]),
                "length": ["num_ctx","num_predict"],
                "structured": ["json_schema","json"],
                "vlm": { "supported": true, "image": 1, "video": 0 },
                "speculative": { "algorithm": [], "max_tokens": 0 },
                "runtime": ["keep_alive","num_gpu","use_mmap"],
                "restart_required": ["num_ctx"]
            }),
            Engine::Omlx => json!({
                "sampling": ["temperature","top_p","top_k","min_p","seed","stop"],
                "penalties": json!(["repetition_penalty","frequency_penalty","presence_penalty"]),
                "length": ["max_tokens","ctx"],
                "structured": ["json_schema"],
                "vlm": { "supported": true, "image": 1, "video": 0 },
                "speculative": { "algorithm": ["mtp"], "max_tokens": 4 },
                "runtime": ["pin_to_memory","ssd_cache","ssd_cache_size","max_context"],
                // oMLX owns its KV/SSD tier: surfaced read-only, edited in oMLX.
                "restart_required": ["ctx","ssd_cache","ssd_cache_size"],
                "kv_tier": { "memory": true, "ssd": true }
            }),
            // halogen 能力表：取自 /health 实测（sampling/structured/vlm/speculative）。
            // runtime/restart_required 留空——halogen 的旋钮在 halogen.env（控制台外管理），
            // 不在参数中心暴露，免得出现改了不生效的假旋钮。
            Engine::Halogen => json!({
                "sampling": ["temperature","top_p","top_k","min_p","seed","stop","logit_bias","logprobs","top_logprobs"],
                "penalties": common_penalties,
                "length": ["max_tokens","ctx"],
                "structured": ["json_schema","json_object"],
                "vlm": { "supported": true, "image": 1, "video": 0 },
                "speculative": { "algorithm": ["ngram","mtp"], "max_tokens": 16 },
                "runtime": [],
                "restart_required": []
            }),
            // gufo 能力表：与 halogen 同源推导——参数旋钮在 gufo.conf（控制台外管理），
            // runtime/restart_required 留空不在参数中心暴露假旋钮；投机只走 MTP
            // （草稿深度即 --draft-tokens 1~7，官方口径；prompt-lookup 由引擎自适应不暴露）。
            Engine::Gufo => json!({
                "sampling": ["temperature","top_p","top_k","min_p","seed","stop","logit_bias","logprobs","top_logprobs"],
                "penalties": common_penalties,
                "length": ["max_tokens","ctx"],
                "structured": ["json_schema","json_object"],
                "vlm": { "supported": true, "image": 1, "video": 0 },
                "speculative": { "algorithm": ["mtp"], "max_tokens": 7 },
                "runtime": [],
                "restart_required": []
            }),
            // Unsloth Studio（llama-server 内核）能力表：采样/惩罚取 Studio 实测口径
            // （`GET /api/inference/sampling-defaults?model=` 返回 temperature/top_p/top_k/
            // min_p/repetition_penalty/presence_penalty 六键，请求侧另支持 frequency_penalty）。
            // runtime/restart_required 只留 ctx——上下文在加载时定，改了要重载。
            Engine::Unsloth => json!({
                "sampling": ["temperature","top_p","top_k","min_p","seed","stop","logprobs"],
                "penalties": common_penalties,
                "length": ["max_tokens","ctx"],
                "structured": ["json_schema","json_object"],
                "vlm": { "supported": true, "image": 1, "video": 0 },
                "speculative": { "algorithm": ["mtp"], "max_tokens": 4 },
                "runtime": [],
                "restart_required": ["ctx"]
            }),
            Engine::LmStudio | Engine::Custom => json!({
                "sampling": ["temperature","top_p","top_k","seed","stop"],
                "penalties": json!(["frequency_penalty","presence_penalty"]),
                "length": ["max_tokens"],
                "structured": [],
                "vlm": { "supported": false },
                "speculative": { "algorithm": [], "max_tokens": 0 },
                "runtime": [],
                "restart_required": []
            }),
        }
    }
}

/// halogen 后端是不是「不许删」的那台。
///
/// 删掉它等于把两块功能打空：速率标定（`gateway::rates` 按 halogen 机位取原生速率）
/// 与 halogen/gufo 控制台（`services::halctl` 的机位名就是 halogen/hal）都以它为锚点。
/// 判据以引擎为主、名字兜底——引擎可能被清空，而 `halogen`/`hal` 是 halctl 认的机位别名。
pub fn is_protected_halogen(name: &str, engine: Option<&str>) -> bool {
    let norm = |s: &str| s.trim().to_ascii_lowercase();
    matches!(engine.map(norm).as_deref(), Some("halogen") | Some("hal"))
        || matches!(norm(name).as_str(), "halogen" | "hal")
}

/// 不许删时给用户的说法（IPC 与 REST 同一句，别两处各写一遍跑偏）。
/// halogen 与 gufo 是同机同端口（8731）的互斥引擎：内存只够一家，
/// 两边同时「启用」只会让流量归到没在跑的那台名下（监控页因此同时挂出两张卡）。
/// 启用开关据此互斥；与后端 `set_backend_enabled` 同一口径。
pub fn is_exclusive_engine(engine: Option<&str>) -> bool {
    matches!(
        engine.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("halogen") | Some("gufo")
    )
}

pub const HALOGEN_PROTECTED: &str = "halogen 后端不能删除：速率标定与 halogen/gufo 控制台都以它为锚点。要停用它请用行上的「停用」，或把引擎切到 gufo。";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halogen_backend_is_protected_by_engine_or_name() {
        // 引擎是 halogen → 保护（名字随便叫，比如用户改叫「102 那台」）。
        assert!(is_protected_halogen("102 那台", Some("halogen")));
        // 名字是 halogen/hal → 也保护（引擎列被清空的存量行）。
        assert!(is_protected_halogen("halogen", None));
        assert!(is_protected_halogen("Hal", Some("custom")));
        // gufo 是同机互斥的另一台，但它不是锚点，允许删。
        assert!(!is_protected_halogen("gufo", Some("gufo")));
        assert!(!is_protected_halogen("Unsloth", Some("unsloth")));
        assert!(!is_protected_halogen("halogen-x", Some("custom")));
    }
    #[test]
    fn halogen_and_gufo_are_one_exclusive_family() {
        // 同端口互斥的只有这一家；别的引擎（omlx/unsloth/ollama…）各自独立，
        // 启用一个不许顺手关掉别人。
        assert!(is_exclusive_engine(Some("halogen")));
        assert!(is_exclusive_engine(Some("GUFO")));
        assert!(is_exclusive_engine(Some(" gufo ")));
        assert!(!is_exclusive_engine(Some("omlx")));
        assert!(!is_exclusive_engine(Some("ollama")));
        assert!(!is_exclusive_engine(None));
    }
}
