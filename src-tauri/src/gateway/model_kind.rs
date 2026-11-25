//! 模型工种类（媒体端点透传代理 11a）。
//!
//! 工种与 chat 并列、**互斥归类**：一个模型要么是聊天模型，要么是某种媒体模型，
//! 绝不兼职——媒体工种永不进聊天择路与分配格子，反向同理（焊死在类型上，不靠人记）。
//!
//! 判定源（用户 2026-11-23 拍板：**取消人工覆盖，完全自动识别**）：
//! 1. **名字启发式**（请求期唯一判定源）：TTS / ASR / flux / SDXL 等确认族模式，
//!    暴露名 ⊕ 上游真名都试；
//! 2. **声明源**（`resolve_kind` 探活侧保留）：halogen `/health` 的 npu 段直接报工种
//!    （qwen3-tts-* → tts、qwen3-asr-* → asr、flux2-klein* → image_gen），照单全收；
//! 3. `backend_models.kind` 人工声明列：历史遗留，**不再参与判定**（误点写入一并失效）。
//!
//! 都没有 = `Chat`。未知默认聊天——猜错代价不对称：把生图当聊天，客户端撞一次墙就
//! 暴露；把聊天当生图，好模型直接失踪。宁可放行。
//!
//! 11a 只落数据层（枚举 + 判定 + 存取口）；路由择路在 11b、UI 分组在 11d。

use serde::Serialize;

/// 模型工种类。`Chat` 是缺省值（未知默认聊天），媒体工种与 chat 互斥。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub enum ModelKind {
    /// 缺省值：未知默认聊天（宁可放行）。
    #[default]
    Chat,
    ImageGen,
    Tts,
    Asr,
    VideoGen,
}

impl ModelKind {
    /// 存库 / IPC 传输用的稳定字符串（小写蛇形，永远只增不改）。
    pub fn as_str(self) -> &'static str {
        match self {
            ModelKind::Chat => "chat",
            ModelKind::ImageGen => "image_gen",
            ModelKind::Tts => "tts",
            ModelKind::Asr => "asr",
            ModelKind::VideoGen => "video_gen",
        }
    }

    /// 解析库里的 kind 列 / IPC 入参。`None`（NULL/空）= 未声明，回落判定链。
    /// 认不出的值**不猜**：按未声明处理（跟「Unknown 放行」同哲学，宁可放行）。
    pub fn parse(value: Option<&str>) -> Option<ModelKind> {
        let v = value?.trim();
        if v.is_empty() {
            return None;
        }
        Some(match v {
            "chat" => ModelKind::Chat,
            "image_gen" => ModelKind::ImageGen,
            "tts" => ModelKind::Tts,
            "asr" => ModelKind::Asr,
            "video_gen" => ModelKind::VideoGen,
            _ => return None,
        })
    }
}

/// 名字启发式：给自动**建议**（不落库、UI 标「（建议）」）。
/// 模式取小写包含匹配；刻意又窄又保守——只收确认过的命名族，
/// 新后缀不因长得像就被继承（同 `is_confirmed_text_only_model` 的教训）。
pub fn suggested_kind_by_name(model: &str) -> Option<ModelKind> {
    let m = model.trim().to_ascii_lowercase();
    let tail = m.rsplit('/').next().unwrap_or(&m);
    // 视频：确认族 + OpenAI Videos API 的命名惯例（模型名里带 video / t2v / i2v / s2v）。
    // **必须排在生图分支之前**：`wan-video`、`wan2.2-i2v-plus` 这类名字同时含 `wan-`，
    // 先判生图就会把视频吞成 image_gen，视频端点从此择不到后端。
    // 11-23 取消人工覆盖后这条是请求期唯一判定源，缺它等于视频代理整体失效
    // （`e2e_video_proxy` 就是这么抓到的，不是测试写错）。
    if ["video", "t2v", "i2v", "s2v", "sora"]
        .iter()
        .any(|p| tail.contains(p))
    {
        return Some(ModelKind::VideoGen);
    }
    // 生图：flux / SDXL / Z-Image / qwen-image 等确认族。
    if [
        "flux",
        "sdxl",
        "z-image",
        "qwen-image",
        "stable-diffusion",
        "wan-",
    ]
    .iter()
    .any(|p| tail.contains(p))
    {
        return Some(ModelKind::ImageGen);
    }
    // TTS / ASR：halogen 首批是 qwen3-tts-* / qwen3-asr-*，whisper 是转写常客。
    if tail.contains("tts") || tail.contains("speech") {
        return Some(ModelKind::Tts);
    }
    if tail.contains("asr") || tail.contains("whisper") {
        return Some(ModelKind::Asr);
    }
    None
}

/// 从 `/health` JSON 里解析某模型的声明工种（声明源，优先级高于名字启发式）。
///
/// npu 段的真实结构以 102 宿主机实际部署为准（当前 NPU 模式未部署，拿不到样例），
/// 所以解析**宽容取形**，不赌单一形状：
/// - `{"npu": {"models": {"<模型名>": "tts", …}}}`（对象映射）；
/// - `{"npu": {"models": [{"name"/"model"/"id": "<模型名>", "kind"/"type": "tts"}, …]}}`（条目数组）；
/// - `{"npu": {"<模型名>": "tts", …}}`（直接映射）。
///
/// 工种值认 `as_str` 的五种写法（大小写宽容），其余忽略——不猜。
pub fn declared_kind_from_health(health: &serde_json::Value, model: &str) -> Option<ModelKind> {
    let npu = health.get("npu")?;
    let want = model.trim().to_ascii_lowercase();
    let parse_kind = |v: &serde_json::Value| -> Option<ModelKind> { ModelKind::parse(v.as_str()) };
    // 条目数组形状：{"name"|"model"|"id": <名>, "kind"|"type": <工种>}
    if let Some(arr) = npu.get("models").and_then(|m| m.as_array()) {
        for item in arr {
            let name = ["name", "model", "id"]
                .iter()
                .find_map(|k| item.get(*k).and_then(|v| v.as_str()));
            let matches = name.is_some_and(|n| n.trim().to_ascii_lowercase() == want);
            if matches {
                for k in ["kind", "type"] {
                    if let Some(kind) = item.get(k).and_then(parse_kind) {
                        return Some(kind);
                    }
                }
            }
        }
    }
    // 对象映射形状（models 层或 npu 层）：{"<模型名>": "<工种>"}
    for obj in [npu.get("models"), Some(npu)] {
        let Some(obj) = obj.and_then(|o| o.as_object()) else {
            continue;
        };
        for (name, v) in obj {
            if name.trim().to_ascii_lowercase() == want {
                if let Some(kind) = parse_kind(v) {
                    return Some(kind);
                }
            }
        }
    }
    None
}

/// 判定链入口（优先级：人工声明 > /health 声明 > 名字建议 > 缺省 Chat）。
pub fn resolve_kind(
    declared: Option<&str>,
    health: Option<&serde_json::Value>,
    model: &str,
) -> ModelKind {
    if let Some(k) = ModelKind::parse(declared) {
        return k;
    }
    if let Some(h) = health {
        if let Some(k) = declared_kind_from_health(h, model) {
            return k;
        }
    }
    suggested_kind_by_name(model).unwrap_or(ModelKind::Chat)
}

/// 请求期工种类（转发热路径专用）：**完全自动识别（用户 2026-11-23 拍板，取消人工覆盖）**——
/// 名字启发式（暴露名 ⊕ 上游真名都试）> 缺省 Chat。`kind` 列不再参与判定：
/// 历史上误点写入的声明一并失效，判定只看名字特征，行为可预期。
/// 名字两边都试是改名/别名的现实：暴露名叫 `tts-fast`、上游真名
/// `qwen3-tts-customvoice`，只认一边就会漏判。
pub fn request_kind(flags: &[crate::types::ModelFlag], requested: &str) -> ModelKind {
    if let Some(k) = suggested_kind_by_name(requested) {
        return k;
    }
    if let Some(f) = flags.iter().find(|f| f.public_name == requested) {
        if let Some(k) = suggested_kind_by_name(&f.upstream_name) {
            return k;
        }
    }
    ModelKind::Chat
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_roundtrip_and_unknown_fails_open() {
        for k in [
            ModelKind::Chat,
            ModelKind::ImageGen,
            ModelKind::Tts,
            ModelKind::Asr,
            ModelKind::VideoGen,
        ] {
            assert_eq!(ModelKind::parse(Some(k.as_str())), Some(k));
        }
        // NULL / 空 = 未声明；认不出的值不猜（None → 回落判定链）。
        assert_eq!(ModelKind::parse(None), None);
        assert_eq!(ModelKind::parse(Some("")), None);
        assert_eq!(ModelKind::parse(Some("  ")), None);
        assert_eq!(ModelKind::parse(Some("video")), None);
    }

    #[test]
    fn name_heuristics_only_match_confirmed_families() {
        assert_eq!(
            suggested_kind_by_name("flux2-klein-4b"),
            Some(ModelKind::ImageGen)
        );
        assert_eq!(
            suggested_kind_by_name("halogen/Z-Image-Turbo"),
            Some(ModelKind::ImageGen)
        );
        assert_eq!(
            suggested_kind_by_name("qwen3-tts-customvoice"),
            Some(ModelKind::Tts)
        );
        assert_eq!(suggested_kind_by_name("qwen3-asr-2b"), Some(ModelKind::Asr));
        assert_eq!(
            suggested_kind_by_name("whisper-large-v3"),
            Some(ModelKind::Asr)
        );
        // 聊天模型一个都不许误伤（前缀相似也不行）。
        for chat in [
            "qwen3.8-flash",
            "qwen3-coder-plus",
            "glm-5.2",
            "deepseek-chat",
        ] {
            assert_eq!(suggested_kind_by_name(chat), None, "{chat} 是聊天模型");
        }
    }

    #[test]
    fn health_declaration_wins_over_heuristics() {
        let health = json!({
            "npu": { "models": { "flux2-klein-4b": "image_gen" } }
        });
        assert_eq!(
            declared_kind_from_health(&health, "flux2-klein-4b"),
            Some(ModelKind::ImageGen)
        );
        // 条目数组形状也认。
        let health2 = json!({
            "npu": { "models": [
                { "name": "qwen3-tts-customvoice", "kind": "tts" },
                { "model": "qwen3-asr-2b", "type": "asr" }
            ]}
        });
        assert_eq!(
            declared_kind_from_health(&health2, "qwen3-tts-customvoice"),
            Some(ModelKind::Tts)
        );
        assert_eq!(
            declared_kind_from_health(&health2, "qwen3-asr-2b"),
            Some(ModelKind::Asr)
        );
        // 没报的模型 / 未知工种值 → None（不猜）。
        assert_eq!(declared_kind_from_health(&health, "qwen3.8-flash"), None);
        let bad = json!({ "npu": { "models": { "m": "video" } } });
        assert_eq!(declared_kind_from_health(&bad, "m"), None);
    }

    #[test]
    fn resolution_order_manual_then_health_then_name_then_chat() {
        let health = json!({ "npu": { "models": { "m1": "tts" } } });
        // 人工声明最高（人工把 TTS 模型改标成生图，照改）。
        assert_eq!(
            resolve_kind(Some("image_gen"), Some(&health), "m1"),
            ModelKind::ImageGen
        );
        // 无人工 → /health 声明。
        assert_eq!(resolve_kind(None, Some(&health), "m1"), ModelKind::Tts);
        // 无声明无 health → 名字建议。
        assert_eq!(
            resolve_kind(None, None, "flux2-klein-4b"),
            ModelKind::ImageGen
        );
        // 全没有 → 默认聊天（宁可放行）。
        assert_eq!(resolve_kind(None, None, "qwen3.8-flash"), ModelKind::Chat);
        // 显式 chat 声明同缺省。
        assert_eq!(
            resolve_kind(Some("chat"), None, "qwen3.8-flash"),
            ModelKind::Chat
        );
    }

    fn flag(public: &str, upstream: &str, kind: Option<&str>) -> crate::types::ModelFlag {
        crate::types::ModelFlag {
            public_name: public.to_string(),
            upstream_name: upstream.to_string(),
            exposed: true,
            accept_upstream: false,
            kind: kind.map(str::to_string),
        }
    }

    #[test]
    fn video_families_win_over_image_prefix() {
        // 取消人工覆盖后（11-23），视频只能靠名字判出来——这条判定链断了，
        // `/v1/videos` 就再也择不到后端（`e2e_video_proxy` 的 400 即此）。
        for name in [
            "wan-video",
            "comfy-video",
            "wan2.2-i2v-plus",
            "hunyuan-video-13b",
            "cogvideox-5b",
            "sora-2",
            "ltx-2.3-t2v",
        ] {
            assert_eq!(
                suggested_kind_by_name(name),
                Some(ModelKind::VideoGen),
                "{name} 该判视频"
            );
        }
        // `wan-` 前缀本身仍是生图那一支，只有带视频特征才被抢走。
        assert_eq!(
            suggested_kind_by_name("wan-t2i-turbo"),
            Some(ModelKind::ImageGen)
        );
        // 名字毫无特征仍是聊天（宁可放行）。
        assert_eq!(suggested_kind_by_name("qwen3-32b"), None);
    }

    #[test]
    fn request_kind_is_name_only_and_tries_upstream_name() {
        let flags = [
            flag("tts-fast", "qwen3-tts-customvoice", None),
            flag("m-chat", "m-chat", Some("chat")),
            flag("renamed-gen", "flux2-klein-4b", Some("image_gen")),
        ];
        // 暴露名没特征 → 上游真名兜住（改名/别名场景）。
        assert_eq!(request_kind(&flags, "tts-fast"), ModelKind::Tts);
        // 人工声明列不参与判定（用户 11-23 拍板取消覆盖）：按名字特征算。
        assert_eq!(request_kind(&flags, "renamed-gen"), ModelKind::ImageGen);
        assert_eq!(request_kind(&flags, "m-chat"), ModelKind::Chat);
        // 完全没见过的模型 → 缺省聊天（宁可放行）。
        assert_eq!(request_kind(&flags, "glm-5.2"), ModelKind::Chat);
        // 上游原名（accept_upstream 过渡期）也认得工种。
        assert_eq!(
            request_kind(&flags, "qwen3-tts-customvoice"),
            ModelKind::Tts
        );
        // 声明列与名字打架时听名字：kind=tts 但名字毫无特征 → Chat。
        let flags2 = [flag("plain-chat", "plain-chat", Some("tts"))];
        assert_eq!(request_kind(&flags2, "plain-chat"), ModelKind::Chat);
    }
}
