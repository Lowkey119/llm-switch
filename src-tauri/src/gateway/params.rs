//! 参数中心（M2 核心切片）：多级覆盖，链 = **请求层 > 模型层 > 全局层 > 后端层**。
//!
//! 出站前单点应用到入口形状的请求体上。约定（片2 定案）：
//! - **请求层绝对最高**：`apply` 入口快照请求侧已有键，任何策略都不覆盖；
//!   长度量纲键（catalog `group=length`）的显式 0 = 「未给」，直接从请求剥掉
//!   交继承链接管（0 token 上游必报错），链上无人就不下发。
//! - 层间按 **模型 → 全局 → 后端** 遍历、先到先得（与前端预览同口径）；
//!   后端层（scope=backend）只在项目没配同键时兜底。
//! - `fill`（默认）与 `override` 同效：都只在键缺席时写（override=历史兼容标签，
//!   用户已拍板「override 也不压请求层」）。
//! - `clamp` 护栏批末统一跑：先按优先级定值，最后把最终值（含请求层给的）
//!   钳到 `[min, max]`，值形状 `{"v":X,"min":A,"max":B}`
//!   （缺 `v` 或长度键 `v:0` = 只钳不填）。
//! - 配置层长度键值 0 = 该层未声明（跳过，审计 applied=false）；
//!   采样等其余键的 0 是真值，正常写入/穿透。
//! - `raw_override` 最后跑：无条件压配置层，但**不碰请求层键**；
//!   批内后端 → 全局 → 模型（后写赢 = 模型层最终赢）。
//!
//! 键名按入口协议字面应用（如 anthropic 入口用 `max_tokens`，
//! chat 入口用 `max_tokens`/`max_completion_tokens` 时请按需各配一条）。

use crate::types::ParamOverride;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Indexed view of the `param_overrides` table, hot-swappable in `State`.
#[derive(Default, Clone)]
pub struct ParamTable {
    global: Vec<ParamOverride>,
    backend: HashMap<String, Vec<ParamOverride>>,
    model: HashMap<String, Vec<ParamOverride>>,
}

/// One applied injection: (key, layer, policy, applied).
/// `applied=false` 表示被跳过、审计可见不落 body，三类：
/// 后端能力不支持 / 长度键 0=继承（该层未声明）/ 请求层已占用（raw 撞客户端键）。
pub struct Injection {
    pub key: String,
    pub layer: &'static str,
    pub policy: String,
    pub applied: bool,
}

fn push(map: &mut HashMap<String, Vec<ParamOverride>>, ref_id: String, o: ParamOverride) {
    map.entry(ref_id).or_default().push(o);
}

impl ParamTable {
    pub fn from_rows(rows: Vec<ParamOverride>) -> Self {
        let mut table = Self::default();
        for o in rows {
            match o.scope.as_str() {
                "global" => table.global.push(o),
                "backend" => push(&mut table.backend, o.ref_id.clone(), o),
                "model" => push(&mut table.model, o.ref_id.clone(), o),
                _ => {}
            }
        }
        table
    }

    /// Apply the stored layers onto the (entry-shape) request body.
    /// 链：请求层 > 模型 > 全局 > 后端；请求层键任何策略都不覆盖。
    pub fn apply(
        &self,
        backend: &str,
        model: &str,
        body: &mut Map<String, Value>,
        caps: Option<&crate::gateway::capability::Caps>,
    ) -> Vec<Injection> {
        self.apply_for(backend, &[model], body, caps)
    }

    /// 同 `apply`，但模型层接受多个候选键：**客户端请求的暴露名在前、上游真名
    /// 在后**。模型层参数的 ref_id 按暴露名存（UI 口径），而转发时 body 的
    /// model 已被改写成上游真名——只认真名会让改名模型的模型层参数静默失效。
    pub fn apply_for(
        &self,
        backend: &str,
        models: &[&str],
        body: &mut Map<String, Value>,
        caps: Option<&crate::gateway::capability::Caps>,
    ) -> Vec<Injection> {
        let mut audit = Vec::new();

        // ① 请求层快照：进参数中心前已有的键 = 请求侧显式给的（客户端意图，
        //    含入口转换产物——fill 一直以来就是这个口径）。
        let mut client_keys: std::collections::HashSet<String> = body.keys().cloned().collect();
        // 长度量纲键的显式 0 = 「未给」：从请求剥掉并撤出快照，
        // 交继承链接管（fill/raw 都能补），链上无人就不下发。
        let zeros: Vec<String> = body
            .iter()
            .filter(|(k, v)| crate::gateway::catalog::is_length_key(k) && v.as_f64() == Some(0.0))
            .map(|(k, _)| k.clone())
            .collect();
        for k in &zeros {
            body.remove(k);
            client_keys.remove(k);
        }

        let model_rows: &[ParamOverride] = models
            .iter()
            .filter_map(|m| self.model.get(&format!("{backend}:{m}")))
            .find(|rows| !rows.is_empty())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let backend_rows: &[ParamOverride] =
            self.backend.get(backend).map(Vec::as_slice).unwrap_or(&[]);
        // 层间先到先得：模型 → 全局 → 后端（与前端 buildChains 同口径）。
        let layers: [(&str, &[ParamOverride]); 3] = [
            ("model", model_rows),
            ("global", &self.global),
            ("backend", backend_rows),
        ];

        // ② 主批：fill/override（同效）按层序「缺失才写」；clamp 批末统一跑。
        for (layer, rows) in layers {
            for o in rows.iter().filter(|o| o.key != "raw_override") {
                let policy = resolve_policy(o);
                if policy == "clamp" {
                    continue;
                }
                if !gate(o, layer, policy, caps, &mut audit) {
                    continue;
                }
                // 值形状守卫：空串/数值键垃圾拒发（记审计），数字串纠正为数字。
                let Ok(value) = normalize_value(&o.key, policy, &o.value) else {
                    audit.push(Injection {
                        key: o.key.clone(),
                        layer,
                        policy: policy.to_string(),
                        applied: false,
                    });
                    continue;
                };
                // 配置层长度键 0 = 该层未声明。
                if crate::gateway::catalog::is_length_key(&o.key) && value.as_f64() == Some(0.0) {
                    audit.push(Injection {
                        key: o.key.clone(),
                        layer,
                        policy: policy.to_string(),
                        applied: false,
                    });
                    continue;
                }
                if apply_one(o, &value, policy, body) {
                    audit.push(Injection {
                        key: o.key.clone(),
                        layer,
                        policy: policy.to_string(),
                        applied: true,
                    });
                }
            }
        }

        // ③ clamp 护栏批末：先按优先级定值，最后统一钳（含请求层给的值——
        //    clamp 是边界保护不是取值覆盖，用户已拍板保留）。
        for (layer, rows) in layers {
            for o in rows.iter().filter(|o| o.key != "raw_override") {
                if resolve_policy(o) != "clamp" {
                    continue;
                }
                if !gate(o, layer, "clamp", caps, &mut audit) {
                    continue;
                }
                // clamp 值形状守卫：必须是 {v,min,max} 对象，v 得是数字。
                let Ok(value) = normalize_value(&o.key, "clamp", &o.value) else {
                    audit.push(Injection {
                        key: o.key.clone(),
                        layer,
                        policy: "clamp".to_string(),
                        applied: false,
                    });
                    continue;
                };
                if apply_one(o, &value, "clamp", body) {
                    audit.push(Injection {
                        key: o.key.clone(),
                        layer,
                        policy: "clamp".to_string(),
                        applied: true,
                    });
                }
            }
        }

        // ④ 逃生舱：raw_override 无条件压配置层，但**不碰请求层键**；
        //    不受能力门控（语义即「我知道我在干什么」）。
        //    批内后端 → 全局 → 模型，后写赢 = 模型层最终赢（同主批链）。
        for (layer, rows) in [
            ("backend", backend_rows),
            ("global", &self.global),
            ("model", model_rows),
        ] {
            for o in rows.iter().filter(|o| o.key == "raw_override") {
                let Some(obj) = o.value.as_object() else {
                    continue;
                };
                let mut wrote = false;
                for (k, v) in obj {
                    if client_keys.contains(k) {
                        audit.push(Injection {
                            key: k.clone(),
                            layer,
                            policy: "merge".into(),
                            applied: false,
                        });
                        continue;
                    }
                    // 值形状守卫：raw 也拒空串/数值键垃圾（同样的 400 事故口子）。
                    let Ok(v2) = normalize_value(k, "fill", v) else {
                        audit.push(Injection {
                            key: k.clone(),
                            layer,
                            policy: "merge".into(),
                            applied: false,
                        });
                        continue;
                    };
                    // 长度键 0 = 未声明，同配置层（绝不下发 0 token）。
                    if crate::gateway::catalog::is_length_key(k) && v2.as_f64() == Some(0.0) {
                        audit.push(Injection {
                            key: k.clone(),
                            layer,
                            policy: "merge".into(),
                            applied: false,
                        });
                        continue;
                    }
                    body.insert(k.clone(), v2);
                    wrote = true;
                }
                if wrote {
                    audit.push(Injection {
                        key: "raw_override".into(),
                        layer,
                        policy: "merge".into(),
                        applied: true,
                    });
                }
            }
        }
        audit
    }
}

/// 空/`auto` 策略 → 目录默认（max_tokens 类默认 clamp，其余 fill）。
fn resolve_policy(o: &ParamOverride) -> &str {
    match o.policy.as_str() {
        "" | "auto" => crate::gateway::catalog::default_policy(&o.key),
        other => other,
    }
}

/// 数字规整：数字原样；数字串纠正（整数串回整数，免得上游 int 校验翻脸）。
fn coerce_num(v: &Value) -> Option<Value> {
    match v {
        Value::Number(_) => Some(v.clone()),
        Value::String(s) => {
            let t = s.trim();
            t.parse::<i64>()
                .map(Value::from)
                .ok()
                .or_else(|| t.parse::<f64>().ok().map(Value::from))
        }
        _ => None,
    }
}

/// 值形状校验（保存口与注入口共用的同一口径）：
/// - **任何键**：空串 / `null` → 拒（清空 = 删除覆盖，不许当值存/发）；
/// - **数值键**：必须是数字，数字串纠正成数字，其余（`"abc"`/bool/对象）拒；
/// - `clamp` 策略：值必须是 `{v,min,max}` 对象，`v` 给了就得是数字；
/// - 其余键非空原样放行（stop/response_format 等对象、数组、字符串）。
///
/// 注入口拿 `Err` 当「跳过并记审计」，宁可不带参数也不能把请求打挂。
pub fn normalize_value(key: &str, policy: &str, value: &Value) -> Result<Value, String> {
    if policy == "clamp" {
        let mut obj = value
            .as_object()
            .cloned()
            .ok_or_else(|| "clamp 的值必须是 {v,min,max} 对象".to_string())?;
        if let Some(v) = obj.get("v").cloned() {
            if !v.is_null() {
                let n = coerce_num(&v).ok_or_else(|| format!("clamp 的 v 必须是数字（{key}）"))?;
                obj.insert("v".into(), n);
            }
        }
        return Ok(Value::Object(obj));
    }
    match value {
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Err("值为空：清空输入即删除覆盖，不下发空值".into());
            }
            if crate::gateway::catalog::is_numeric_key(key) {
                coerce_num(value).ok_or_else(|| format!("{key} 需要数字，收到 {s:?}"))
            } else {
                Ok(value.clone())
            }
        }
        Value::Number(_) => Ok(value.clone()),
        Value::Null => Err("值为空（null）：要恢复默认请删除覆盖".into()),
        _ if crate::gateway::catalog::is_numeric_key(key) => {
            Err(format!("{key} 需要数字，收到 {value}"))
        }
        _ => Ok(value.clone()),
    }
}

/// 能力门控：目录内已知键且后端不支持 → 跳过（审计记 skipped）。true = 放行。
fn gate(
    o: &ParamOverride,
    layer: &'static str,
    policy: &str,
    caps: Option<&crate::gateway::capability::Caps>,
    audit: &mut Vec<Injection>,
) -> bool {
    if crate::gateway::catalog::is_known_key(&o.key) {
        if let Some(c) = caps {
            if !c.supports(&o.key) {
                audit.push(Injection {
                    key: o.key.clone(),
                    layer,
                    policy: policy.to_string(),
                    applied: false,
                });
                return false;
            }
        }
    }
    true
}

fn clamp_number(v: &Value, min: Option<&Value>, max: Option<&Value>) -> Value {
    let mut n = match v.as_f64() {
        Some(n) => n,
        None => return v.clone(),
    };
    if let Some(m) = min.and_then(Value::as_f64) {
        n = n.max(m);
    }
    if let Some(m) = max.and_then(Value::as_f64) {
        n = n.min(m);
    }
    json_number(n, v)
}

/// Keep integers integral after clamping.
fn json_number(n: f64, orig: &Value) -> Value {
    if orig.is_i64() || orig.is_u64() {
        Value::from(n.round() as i64)
    } else {
        Value::from(n)
    }
}

/// 单行落点。`policy` 由调用方先 resolve；`value` 是 `normalize_value` 校验过的值。
/// 返回是否真的改了 body。
fn apply_one(
    o: &ParamOverride,
    value: &Value,
    policy: &str,
    body: &mut Map<String, Value>,
) -> bool {
    match policy {
        "clamp" => {
            let (spec_min, spec_max) = (value.get("min"), value.get("max"));
            // 长度键的 `v:0` = 继承：不拿它当默认值，但护栏照立。
            let spec_val = value.get("v").cloned().filter(|v| {
                !(crate::gateway::catalog::is_length_key(&o.key) && v.as_f64() == Some(0.0))
            });
            let present = body.contains_key(&o.key);
            let mut changed = false;
            if let Some(v) = spec_val {
                if !present {
                    body.insert(o.key.clone(), v);
                    changed = true;
                }
            }
            if let Some(cur) = body.get(&o.key).cloned() {
                let clamped = clamp_number(&cur, spec_min, spec_max);
                if clamped != cur {
                    body.insert(o.key.clone(), clamped);
                    changed = true;
                }
            }
            changed
        }
        // `fill` 与 `override` 同效（用户拍板「override 也不压请求层」）：
        // 请求层最高 + 层间先到先得 → 都「缺失才写」；override 是历史兼容标签。
        _ => {
            if body.contains_key(&o.key) {
                false
            } else {
                body.insert(o.key.clone(), value.clone());
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn o(scope: &str, ref_id: &str, key: &str, value: Value, policy: &str) -> ParamOverride {
        ParamOverride {
            id: 0,
            scope: scope.into(),
            ref_id: ref_id.into(),
            key: key.into(),
            value,
            policy: policy.into(),
        }
    }

    #[test]
    fn request_layer_beats_every_policy() {
        // 片2 定案：请求层绝对最高，fill/override 都只「缺失才写」。
        let table = ParamTable::from_rows(vec![
            o("global", "", "temperature", json!(0.1), "fill"),
            o("global", "", "top_p", json!(0.5), "override"),
        ]);
        let mut body = json!({"temperature": 0.9, "top_p": 0.9})
            .as_object()
            .unwrap()
            .clone();
        table.apply("b", "m", &mut body, None);
        assert_eq!(body["temperature"], 0.9, "fill 不压请求层");
        assert_eq!(body["top_p"], 0.9, "override 也不压请求层");
    }

    /// 改名模型（暴露名 ≠ 上游真名）的模型层参数必须还能查到：ref_id 按暴露名存，
    /// 转发时 body.model 已是上游真名——apply_for 双候选键（暴露名在前）修复此断链。
    #[test]
    fn apply_for_finds_model_layer_by_exposed_name() {
        let table = ParamTable::from_rows(vec![
            o(
                "model",
                "b1:exposed-name",
                "temperature",
                json!(0.7),
                "fill",
            ),
            o("global", "", "temperature", json!(0.1), "fill"),
        ]);
        // 候选 = [暴露名, 上游真名]：模型层（按暴露名存）赢过全局层。
        let mut body = Map::new();
        table.apply_for("b1", &["exposed-name", "upstream-real"], &mut body, None);
        assert_eq!(body["temperature"], 0.7);
        // 两个候选都没配模型层 → 落全局层，不误伤。
        let mut body = Map::new();
        table.apply_for("b1", &["other", "none"], &mut body, None);
        assert_eq!(body["temperature"], 0.1);
    }

    #[test]
    fn layer_chain_model_then_global_then_backend() {
        let table = ParamTable::from_rows(vec![
            o("global", "", "temperature", json!(0.1), "fill"),
            o("backend", "b1", "temperature", json!(0.2), "fill"),
            o("model", "b1:m1", "temperature", json!(0.3), "fill"),
        ]);
        // 模型层赢。
        let mut body = Map::new();
        table.apply("b1", "m1", &mut body, None);
        assert_eq!(body["temperature"], 0.3);
        // 没配模型层 → 全局层赢（全局压后端层）。
        let mut body = Map::new();
        table.apply("b1", "m9", &mut body, None);
        assert_eq!(body["temperature"], 0.1);
        // 模型+全局都没配 → 后端层兜底。
        let table2 =
            ParamTable::from_rows(vec![o("backend", "b1", "temperature", json!(0.2), "fill")]);
        let mut body = Map::new();
        table2.apply("b1", "m1", &mut body, None);
        assert_eq!(body["temperature"], 0.2);
        // 每个请求只填一次（先到先得）。
        let mut body = Map::new();
        let audit = table.apply("b1", "m9", &mut body, None);
        assert_eq!(
            audit
                .iter()
                .filter(|i| i.key == "temperature" && i.applied)
                .count(),
            1
        );
    }

    #[test]
    fn raw_override_merges_last_and_wins() {
        let table = ParamTable::from_rows(vec![
            o("global", "", "temperature", json!(0.1), "override"),
            o(
                "backend",
                "b",
                "raw_override",
                json!({"temperature": 0.77, "vendor_x": 1}),
                "fill",
            ),
        ]);
        let mut body = Map::new();
        table.apply("b", "m", &mut body, None);
        assert_eq!(body["temperature"], 0.77);
        assert_eq!(body["vendor_x"], 1);
    }

    #[test]
    fn empty_policy_uses_catalog_default() {
        // max_tokens 目录默认 clamp；temperature 默认 fill。
        let table = ParamTable::from_rows(vec![
            o(
                "global",
                "",
                "max_tokens",
                json!({"min": 1, "max": 1024}),
                "",
            ),
            o("global", "", "temperature", json!(0.4), "auto"),
        ]);
        let mut body = json!({"max_tokens": 99999}).as_object().unwrap().clone();
        table.apply("b", "m", &mut body, None);
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["temperature"], 0.4);
    }

    #[test]
    fn capability_gate_skips_unsupported_keys() {
        let table = ParamTable::from_rows(vec![
            o("global", "", "temperature", json!(0.3), "fill"),
            o(
                "global",
                "",
                "limit_mm_per_prompt",
                json!({"image": 1}),
                "fill",
            ),
            o("global", "", "vendor_private_knob", json!(7), "fill"), // 目录外键放行
        ]);
        let caps = crate::gateway::capability::effective(Some("custom"), &serde_json::json!({}));
        let mut body = Map::new();
        let audit = table.apply("b", "m", &mut body, Some(&caps));
        assert_eq!(body["temperature"], 0.3);
        assert_eq!(body["vendor_private_knob"], 7);
        assert!(
            body.get("limit_mm_per_prompt").is_none(),
            "vlm key must be gated"
        );
        let skipped: Vec<_> = audit.iter().filter(|i| !i.applied).collect();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].key, "limit_mm_per_prompt");
    }

    #[test]
    fn clamp_bounds_client_and_filled_values() {
        let table = ParamTable::from_rows(vec![
            o(
                "global",
                "",
                "max_tokens",
                json!({"min": 1, "max": 4096}),
                "clamp",
            ),
            o(
                "model",
                "b:m",
                "temperature",
                json!({"v": 0.5, "min": 0.0, "max": 1.0}),
                "clamp",
            ),
        ]);
        let mut body = json!({"max_tokens": 99999}).as_object().unwrap().clone();
        table.apply("b", "m", &mut body, None);
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn length_zero_means_inherit_everywhere() {
        // R4：长度键 0=继承——请求侧 0 被剥、配置层 0 该层未声明、链上无人不下发。
        let table = ParamTable::from_rows(vec![
            o("model", "b:m", "max_tokens", json!(0), "fill"),
            o("global", "", "max_tokens", json!(4096), "fill"),
            o("global", "", "min_tokens", json!(0), "fill"),
        ]);
        let mut body = json!({"max_tokens": 0}).as_object().unwrap().clone();
        let audit = table.apply("b", "m", &mut body, None);
        assert_eq!(
            body["max_tokens"], 4096,
            "客户端 0 剥掉 → 模型层 0 跳过 → 全局层填入"
        );
        assert!(body.get("min_tokens").is_none(), "链上无人 = 不下发");
        assert!(
            audit
                .iter()
                .any(|i| i.key == "max_tokens" && i.layer == "model" && !i.applied),
            "模型层 0=继承应记审计"
        );

        // 链上全无声明 → 0 剥掉后不补，交后端默认。
        let table = ParamTable::default();
        let mut body = json!({"max_tokens": 0}).as_object().unwrap().clone();
        table.apply("b", "m", &mut body, None);
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn sampling_zero_is_a_real_value() {
        // 采样键不在量纲范围：显式 0 是真值，穿透且不被层覆盖。
        let table = ParamTable::from_rows(vec![o("global", "", "temperature", json!(0.7), "fill")]);
        let mut body = json!({"temperature": 0}).as_object().unwrap().clone();
        table.apply("b", "m", &mut body, None);
        assert_eq!(body["temperature"], 0, "客户端显式 0 原样保留");
        let mut body = Map::new();
        table.apply("b", "m", &mut body, None);
        assert_eq!(body["temperature"], 0.7, "没给才填");
    }

    #[test]
    fn raw_override_respects_request_layer() {
        let table = ParamTable::from_rows(vec![o(
            "global",
            "",
            "raw_override",
            json!({"temperature": 0.1, "vendor_x": 1, "max_tokens": 0}),
            "merge",
        )]);
        let mut body = json!({"temperature": 0.9}).as_object().unwrap().clone();
        let audit = table.apply("b", "m", &mut body, None);
        assert_eq!(body["temperature"], 0.9, "raw 不碰请求层键");
        assert_eq!(body["vendor_x"], 1, "请求没给的键照样补");
        assert!(body.get("max_tokens").is_none(), "raw 的长度键 0 也不下发");
        let skipped: Vec<_> = audit.iter().filter(|i| !i.applied).collect();
        assert!(skipped.iter().any(|i| i.key == "temperature"));
        assert!(skipped.iter().any(|i| i.key == "max_tokens"));
    }

    #[test]
    fn empty_or_garbage_values_never_injected() {
        // 值形状守卫（top_p:"" 事故的最后防线）：空串/数值键垃圾拒发（审计记
        // skipped），数字串纠正为数字——宁可不带参数也不能把请求打挂。
        let table = ParamTable::from_rows(vec![
            o("model", "b:m", "top_p", json!(""), "fill"),
            o("model", "b:m", "temperature", json!("abc"), "fill"),
            o("model", "b:m", "stop", json!("  "), "fill"), // 非数值键的空白串同样拒
            o("model", "b:m", "seed", json!("42"), "fill"), // 数字串纠正
        ]);
        let mut body = Map::new();
        let audit = table.apply("b", "m", &mut body, None);
        assert!(body.get("top_p").is_none(), "空串不下发");
        assert!(body.get("temperature").is_none(), "垃圾串不下发");
        assert!(body.get("stop").is_none(), "空白串不下发");
        assert_eq!(body["seed"], json!(42), "数字串纠正为数字");
        let skipped: Vec<&str> = audit
            .iter()
            .filter(|i| !i.applied)
            .map(|i| i.key.as_str())
            .collect();
        assert_eq!(skipped.len(), 3, "三条坏值都应记审计：{skipped:?}");
    }

    #[test]
    fn normalize_value_guards() {
        // 保存口与注入口共用同一口径。
        assert!(normalize_value("top_p", "fill", &json!("")).is_err());
        assert!(normalize_value("top_p", "fill", &json!(null)).is_err());
        assert!(normalize_value("top_p", "fill", &json!("abc")).is_err());
        assert!(normalize_value("top_p", "fill", &json!(true)).is_err());
        assert_eq!(
            normalize_value("top_p", "fill", &json!("0.9")).unwrap(),
            json!(0.9)
        );
        assert_eq!(
            normalize_value("seed", "fill", &json!("42")).unwrap(),
            json!(42)
        );
        assert!(normalize_value("top_p", "fill", &json!(0.9)).is_ok());
        assert!(normalize_value("stop", "fill", &json!("END")).is_ok());
        assert!(normalize_value("stop", "fill", &json!(["a", "b"])).is_ok());
        assert!(
            normalize_value("response_format", "fill", &json!({"type": "json_object"})).is_ok()
        );
        // clamp：必须是 {v,min,max} 对象，v 给了就得是数字（数字串纠正）。
        assert!(normalize_value("max_tokens", "clamp", &json!({"min": 1, "max": 10})).is_ok());
        assert!(normalize_value("max_tokens", "clamp", &json!("")).is_err());
        assert!(normalize_value("max_tokens", "clamp", &json!({"v": "abc"})).is_err());
        assert_eq!(
            normalize_value("max_tokens", "clamp", &json!({"v": "4096"})).unwrap(),
            json!({"v": 4096})
        );
    }
}
