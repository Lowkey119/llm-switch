//! 模型图文能力判定（P0-b）。
//!
//! 见 `docs/NOTICE-upstream.md`）。改动：`crate::claude_desktop_config::ONE_M_CONTEXT_MARKER`
//! 换成本地常量 `[1M]`；对外函数放开到 `pub`；其余逻辑与测试原样沿用。
//!
//! 为什么值得搬：它把一件容易做错的事定得很稳——
//! **声明优先，其次查「已确认纯文本」名单，都没有就是 Unknown，而 Unknown 一律放行**。
//! 猜错方向的代价不对称：把会看图的说成不会看 = 白白剥掉图片；
//! 把不会图的说成会看 = 上游回个 400，至少是真话。所以宁可放行。

use serde_json::Value;

/// Claude Code 用来声明 1M 上下文的模型名后缀（习面写作 `GLM-5.2[1M]`）。
/// 这里存小写：`normalize_model_id` 先转小写再剔后缀，上游也是 `[1m]`。
pub const ONE_M_CONTEXT_MARKER: &str = "[1m]";

/// 图片输入能力。`Unknown` 刻意区别于 `Unsupported`：
/// 调用方可以据此选不同策略，而不必各自再抄一份模型名单。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageInputCapability {
    Supported,
    Unsupported,
    Unknown,
}

/// 先认显式声明，没有声明再看已确认纯文本网状表（调用方决定是否启用后者）。
pub fn resolve_image_input_capability(
    model: &str,
    declared_support: Option<bool>,
    use_confirmed_registry: bool,
) -> ImageInputCapability {
    match declared_support {
        Some(true) => ImageInputCapability::Supported,
        Some(false) => ImageInputCapability::Unsupported,
        None if use_confirmed_registry && is_confirmed_text_only_model(model) => {
            ImageInputCapability::Unsupported
        }
        None => ImageInputCapability::Unknown,
    }
}

/// 从供应商设置里几种常见形状里读某模型的显式图片声明。
pub fn image_input_capability_from_settings(
    settings: &Value,
    model: &str,
    use_confirmed_registry: bool,
) -> ImageInputCapability {
    resolve_image_input_capability(
        model,
        declared_model_image_support(settings, model),
        use_confirmed_registry,
    )
}

/// 把目录行里的模态列表翻译成统一的能力表示，缺省回落已确认名单。
pub fn image_input_capability_from_modalities(
    model: &str,
    modalities: Option<&[String]>,
) -> ImageInputCapability {
    let declared_support = modalities.map(|items| {
        items
            .iter()
            .any(|item| item.trim().eq_ignore_ascii_case("image"))
    });
    resolve_image_input_capability(model, declared_support, true)
}

/// 我们敢当成「纯文本」的模型名单。
///
/// 故意做得又窄又保守：新后缀**不会**因为长得像就被继承——
/// 在它的能力被确认之前一律按会看图处理，免得某个未来的 `-vision` / `-vl`
/// 变体在请求还没出门时就被掐死。
pub fn is_confirmed_text_only_model(model: &str) -> bool {
    let normalized = normalize_model_id(model);
    let tail = normalized.rsplit('/').next().unwrap_or(normalized.as_str());

    const CONFIRMED_TAILS: &[&str] = &[
        "ark-code-latest",
        "deepseek-chat",
        "deepseek-reasoner",
        // `deepseek-v4-flash` 故意不收：它是厂商仍在受理的旧别名，实际路由到会看图的
        // deepseek-flash，必须放行。`deepseek-v4-pro` 同理留在门外。
        "glm-5.1",
        // 精确匹配而非前缀：GLM 的视觉型号带 `v` 后缀（glm-5.2v），必须保持会看图。
        "glm-5.2",
        "glm-5.3",
        "kat-coder",
        "kat-coder-pro",
        "kat-coder-pro v1",
        "kat-coder-pro v2",
        "kat-coder-pro-v1",
        "kat-coder-pro-v2",
        "ling-2.5-1t",
        // Ant Ling 的视觉是独立的 `-VL` 型号；Ling-2.6-1T 确认纯文本。
        "ling-2.6-1t",
        "longcat-2.0",
        "longcat-flash-chat",
        "minimax-m2.7",
        "minimax-m2.7-highspeed",
        "mimo-v2.5-pro",
        "qwen3-coder-480b",
        "qwen3-coder-480b-a35b-instruct",
        "qwen3-coder-flash",
        "qwen3-coder-next",
        "qwen3-coder-plus",
        "step-3.5-flash",
        "step-3.5-flash-2603",
        "us.deepseek.r1-v1",
    ];

    CONFIRMED_TAILS.contains(&tail)
}

/// 确认只吃文本的模型遇到图片块：就地换成文字标记，别让上游回一个谁也看不懂的 400。
/// `declared` = 模型元数据里的 VLM 声明（人工 > 探测；None = 未知，回落已确认名单）。
/// 返回剥掉的图片块数量（没剥就是 0）。
pub fn rectify_image_blocks(body: &mut Value, model: &str, declared: Option<bool>) -> usize {
    if resolve_image_input_capability(model, declared, true) != ImageInputCapability::Unsupported {
        return 0;
    }
    if !crate::gateway::media_sanitizer::contains_image_blocks(body) {
        return 0;
    }
    crate::gateway::media_sanitizer::replace_image_blocks_with_marker(body)
}

fn declared_model_image_support(settings: &Value, model: &str) -> Option<bool> {
    [
        settings
            .get("modelCatalog")
            .and_then(|catalog| catalog.get("models")),
        settings.get("modelCatalog"),
        settings.get("models"),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| declared_model_image_support_in_value(value, model))
}

fn declared_model_image_support_in_value(value: &Value, model: &str) -> Option<bool> {
    if let Some(models) = value.as_array() {
        return models.iter().find_map(|entry| {
            model_entry_matches(entry, None, model).then(|| explicit_image_support(entry))?
        });
    }

    let object = value.as_object()?;
    object.iter().find_map(|(key, entry)| {
        model_entry_matches(entry, Some(key), model).then(|| explicit_image_support(entry))?
    })
}

fn explicit_image_support(entry: &Value) -> Option<bool> {
    if let Some(value) = entry
        .get("supportsImage")
        .or_else(|| entry.get("supports_image"))
        .or_else(|| entry.get("vision"))
        .and_then(Value::as_bool)
    {
        return Some(value);
    }

    [
        entry.get("input"),
        entry.pointer("/modalities/input"),
        entry.get("input_modalities"),
        entry.get("inputModalities"),
    ]
    .into_iter()
    .flatten()
    .find_map(input_modalities_support_image)
}

fn input_modalities_support_image(value: &Value) -> Option<bool> {
    let modalities = value.as_array()?;
    Some(modalities.iter().any(|item| {
        item.as_str()
            .map(str::trim)
            .is_some_and(|item| item.eq_ignore_ascii_case("image"))
    }))
}

fn model_entry_matches(entry: &Value, key: Option<&str>, model: &str) -> bool {
    key.is_some_and(|key| model_ids_match(key, model))
        || ["model", "id", "name"]
            .into_iter()
            .filter_map(|field| entry.get(field).and_then(Value::as_str))
            .any(|candidate| model_ids_match(candidate, model))
}

fn model_ids_match(candidate: &str, model: &str) -> bool {
    let candidate = normalize_model_id(candidate);
    let model = normalize_model_id(model);
    if candidate.is_empty() || model.is_empty() {
        return false;
    }
    if candidate == model {
        return true;
    }

    let candidate_tail = candidate.rsplit('/').next().unwrap_or(candidate.as_str());
    let model_tail = model.rsplit('/').next().unwrap_or(model.as_str());
    candidate_tail == model_tail || candidate == model_tail || candidate_tail == model
}

fn normalize_model_id(value: &str) -> String {
    let mut normalized = value
        .trim()
        .trim_start_matches("models/")
        .trim()
        .to_ascii_lowercase();
    if let Some(stripped) = normalized.strip_suffix(ONE_M_CONTEXT_MARKER) {
        normalized = stripped.trim().to_string();
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gpt_and_unknown_models_remain_unknown_without_declarations() {
        for model in ["gpt-5.4", "gpt-5.5", "gpt-5.6-sol", "custom-alias"] {
            assert_eq!(
                resolve_image_input_capability(model, None, true),
                ImageInputCapability::Unknown,
                "{model} 必须放行"
            );
        }
    }

    #[test]
    fn confirmed_text_only_registry_normalizes_namespaces_and_context_markers() {
        assert!(is_confirmed_text_only_model("deepseek/deepseek-chat"));
        assert!(!is_confirmed_text_only_model("deepseek/deepseek-v4-flash"));
        assert!(!is_confirmed_text_only_model("deepseek/deepseek-v4-pro"));
        assert!(is_confirmed_text_only_model("GLM-5.2[1M]"));
        assert!(is_confirmed_text_only_model("qwen/qwen3-coder-plus"));
        assert!(is_confirmed_text_only_model(
            "Qwen/Qwen3-Coder-480B-A35B-Instruct"
        ));
        assert!(!is_confirmed_text_only_model("glm-5.2v"));
    }

    #[test]
    fn unconfirmed_family_suffixes_fail_open() {
        for model in [
            "minimax-m2.7-vision",
            "qwen3-coder-ultra",
            "qwen3-coder-vl",
            "step-3.5-flash-vision",
        ] {
            assert!(
                !is_confirmed_text_only_model(model),
                "未确认的变体 {model} 不能被硬掐"
            );
        }
    }

    #[test]
    fn explicit_capability_overrides_the_registry() {
        assert_eq!(
            resolve_image_input_capability("qwen3-coder-plus", Some(true), true),
            ImageInputCapability::Supported
        );
        assert_eq!(
            resolve_image_input_capability("gpt-5.4", Some(false), true),
            ImageInputCapability::Unsupported
        );
    }

    #[test]
    fn provider_settings_support_multiple_capability_shapes() {
        let settings = json!({
            "modelCatalog": {
                "models": [
                    { "model": "vision", "modalities": { "input": ["text", "image"] } },
                    { "model": "text", "inputModalities": ["text"] }
                ]
            }
        });
        assert_eq!(
            image_input_capability_from_settings(&settings, "vision", true),
            ImageInputCapability::Supported
        );
        assert_eq!(
            image_input_capability_from_settings(&settings, "text", true),
            ImageInputCapability::Unsupported
        );
    }

    #[test]
    fn rectify_strips_images_only_for_confirmed_text_only_models() {
        let body_with_image = || {
            json!({
                "model": "m",
                "messages": [{
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "看看这张图" },
                        { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }
                    ]
                }]
            })
        };
        // 会看图 / 不知道 → 一个字都不动
        let mut keep = body_with_image();
        assert_eq!(rectify_image_blocks(&mut keep, "gpt-5.4", None), 0);
        assert!(keep["messages"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["type"] == json!("image_url")));

        // 确认纯文本 → 换成文字标记，文本块留着
        let mut strip = body_with_image();
        assert_eq!(
            rectify_image_blocks(&mut strip, "qwen3-coder-plus", None),
            1
        );
        assert!(!strip["messages"][0]["content"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["type"] == json!("image_url")));
    }
}
