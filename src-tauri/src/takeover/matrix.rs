//! 模型矩阵（M12-G）：行 = 聚合暴露清单，列 = 各客户端，格子 = 「这个端用不用它 / 谁是默认」。
//!
//! 为什么要这张表：模型散在各家配置里，切一次要在接管页一家家点开。矩阵把
//! 「哪个模型给哪个端用」变成一页多点几下，而**真值仍然来自配置文件**
//! （[`super::read_selection`]），库里的期望只用来做 diff。
//!
//! 两层的规矩：
//! · `该端实际可见 = 全局暴露 ∩ 该端清单`，清单缺省 = 全暴露（零配置）；
//!   接管后各端配置文件里的清单只写**已分配给它的模型**（+ 它原有的在线 API 模型），
//!   所以矩阵里读到的清单就是真实分配，不再有「全暴露兜底造成的恒亮」；
//! · 默认档必须在可见清单里，否则拒绝，不写一个客户端选不到的默认。
//!
//! claude / codebuddy 特殊：它们没有“清单”概念，只有档位槽（claude 是
//! haiku/sonnet/opus/fable/subagent，codebuddy 是 lite/reasoning，实测
//! `availableModels` 是限制器不是引入器），所以它们的格子语义是**占槽**，
//! 满了就明确拒绝，绝不静默丢弃。

use super::{
    backends_of_models, connection_managed, model_note, read_selection, served_models, tiers_of,
    ClientApp, Endpoint, Selection, TakeoverEnv,
};
use crate::store::Store;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// 档位制客户端的槽位（用户没指定槽位时按这个顺序找空位）。
/// claude 与 codebuddy 都是档位制：`tiers_of` 返回非空即档位制客户端。
fn slots_of(app: ClientApp) -> Option<&'static [&'static str]> {
    let tiers = tiers_of(app);
    (!tiers.is_empty()).then_some(tiers)
}

/// 格子上的三种操作。
#[derive(Debug, Clone)]
pub enum CellOp {
    /// 加入该端可用清单（claude = 占一个档位槽，可指名槽位）
    Add { slot: Option<String> },
    /// 从该端可用清单移出（档位制可指名槽位 = 只摘这一个槽；不指名 = 它占的槽全摘）
    Remove { slot: Option<String> },
    /// 设为该端默认
    SetDefault,
}

impl CellOp {
    pub fn parse(raw: &str) -> Result<CellOp, String> {
        match raw {
            "add" => Ok(CellOp::Add { slot: None }),
            "remove" => Ok(CellOp::Remove { slot: None }),
            "default" => Ok(CellOp::SetDefault),
            other => Err(format!("未知的格子操作: {other}")),
        }
    }
}

/// 该端能占多少个模型位（档位制 = 槽数 + 默认；pi = 主/小两槽；codex = 主/副两槽；gemini = 1；其余不限）。
fn capacity(app: ClientApp) -> Option<usize> {
    let tiers = tiers_of(app);
    match app {
        ClientApp::Gemini => Some(1),
        ClientApp::Pi => Some(Selection::PI_SLOT_KEYS.len()),
        ClientApp::Codex => Some(Selection::CODEX_SLOT_KEYS.len()),
        _ if !tiers.is_empty() => Some(tiers.len() + 1),
        _ => None,
    }
}

/// 矩阵下发给 UI 的槽位键列表：pi = 两个专属槽；codex = 主/副两槽；
/// 档位制 = 默认档 + 档位；其余为空。
/// UI 直接拿这份画档位浮层，不再自己拼 `["main", ...tiers]`。
fn slot_keys(app: ClientApp) -> Vec<String> {
    match app {
        ClientApp::Pi => Selection::PI_SLOT_KEYS
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        ClientApp::Codex => Selection::CODEX_SLOT_KEYS
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        _ if !tiers_of(app).is_empty() => std::iter::once("main".to_string())
            .chain(tiers_of(app).iter().map(|s| (*s).to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// 整张矩阵：给前端一次画完。
pub fn matrix(env: &TakeoverEnv, store: &Store, ep: &Endpoint) -> Result<Value, String> {
    let exposed = served_models(store);
    // 停用的代理（代理 tab 片三）：列与格子都不下发——只隐藏，分配原样保留。
    let disabled: std::collections::BTreeSet<String> =
        store.agents_disabled().into_iter().collect();
    let mut currents: BTreeMap<&'static str, Selection> = BTreeMap::new();
    let mut clients = Vec::new();
    for app in ClientApp::ALL {
        if disabled.contains(app.as_str()) {
            continue;
        }
        // copilot 是网关门面：没有配置文件，真值在 settings 的可见清单，
        // 从门面同一份读取口径合成（read_selection 对它显式报错，别兜底错值）。
        let cur = if app == ClientApp::Copilot {
            Selection {
                allow: store.copilot_allow().map(|v| v.into_iter().collect()),
                ..Selection::default()
            }
        } else {
            read_selection(env, app).unwrap_or_default()
        };
        // 已占模型位：档位制 = 占槽 + 默认；gemini = 默认；pi/codex = 双槽占用；
        // 其余清单制 = 清单长度。UI 画 「已用/容量」，满了的列点空格子弹档位浮层而不是白点一下。
        let visible = cur.visible(&exposed);
        let used = if app == ClientApp::Pi {
            cur.pi_slots().iter().filter(|(_, v)| v.is_some()).count()
        } else if app == ClientApp::Codex {
            cur.codex_slots()
                .iter()
                .filter(|(_, v)| v.is_some())
                .count()
        } else if !tiers_of(app).is_empty() {
            cur.tiers.len() + usize::from(cur.default.is_some())
        } else if app == ClientApp::Gemini {
            usize::from(cur.default.is_some())
        } else {
            visible.len()
        };
        clients.push(json!({
            "app": app.as_str(),
            "label": app.label(),
            "taken_over": connection_managed(env, store, ep, app).unwrap_or(false),
            "tiers": tiers_of(app),
            "slots": slot_keys(app),
            "capacity": capacity(app),
            // pi 的默认档归它自己的 models-store.json 管，copilot 是可见清单
            // 没有默认档概念——这两端 UI 都别摆 ★。
            "default_allowed": !matches!(app, ClientApp::Pi | ClientApp::Copilot),
            // 网关门面（无配置文件可接管）：代理管理不列它，模型矩阵是唯一入口。
            "facade": app == ClientApp::Copilot,
            "used": used,
            "note": model_note(app),
            "visible": visible,
            "default": cur.default,
        }));
        currents.insert(app.as_str(), cur);
    }
    // 队列④：每行标出该模型来自哪些启用后端（矩阵首列灰色后缀标签）。
    // helper 与 served_models 同口径（enabled × exposed_models()）：行集与标注
    // 同源，绝不出现「有行无标」的分叉；接管下拉分组（option_groups）也走它。
    let backends_of = backends_of_models(store);
    let rows: Vec<Value> = exposed
        .iter()
        .map(|model| {
            let mut cells = Map::new();
            for app in ClientApp::ALL {
                if disabled.contains(app.as_str()) {
                    continue;
                }
                let cur = currents
                    .get(app.as_str())
                    .expect("every client inserted above");
                // 槽位之间不再互斥，一个模型能同时占多个槽，所以这里收的是**全部**槽位。
                // `slot` 保留成第一个，给还没升级的调用方兜底（旧 UI 只读它）。
                let slots: Vec<String> = if app == ClientApp::Pi {
                    // pi 是两槽制（主/小模型）：格子亮 = 占了某个槽，不看清单
                    // （清单恒全暴露，按清单画会整列全亮，毫无信息量）。
                    cur.pi_slots()
                        .into_iter()
                        .filter(|(_, v)| *v == Some(model.as_str()))
                        .map(|(k, _)| k.to_string())
                        .collect()
                } else if app == ClientApp::Codex {
                    // codex 也是双槽制（主/子代理模型）：格子亮 = 占了某个槽，
                    // 清单（catalog）退居幕后由 set_model 统一重写。
                    cur.codex_slots()
                        .into_iter()
                        .filter(|(_, v)| *v == Some(model.as_str()))
                        .map(|(k, _)| k.to_string())
                        .collect()
                } else {
                    cur.tiers
                        .iter()
                        .filter(|(_, v)| v.as_str() == model.as_str())
                        .map(|(k, _)| k.clone())
                        .collect()
                };
                let slot = slots.first().cloned();
                // 格子亮的口径 = **这个端真的在用它**，不能再拿「清单缺省 = 全暴露」
                // 兜底：claude / codebuddy 这类档位制端读不到清单（文件里压根没有），
                // `visible` 恒为全集，整列永远是亮的，既看不出分配也误导操作。
                // · pi / codex：看占槽 **或进清单**（BUG-4：槽位只约束档位槽，
                //   清单追加的模型格子同样要亮）；
                // · 档位制（claude / codebuddy）：看占档或默认；
                // · gemini：只有一个模型位 = 默认；
                // · 清单制（opencode / DevEco / copilot）：看清单，且清单现在
                //   只含已分配模型，读回来是什么就画什么。
                let in_list = if app == ClientApp::Pi || app == ClientApp::Codex {
                    slot.is_some() || cur.allow.as_ref().is_some_and(|a| a.contains(model))
                } else if !tiers_of(app).is_empty() {
                    slot.is_some() || cur.default.as_deref() == Some(model.as_str())
                } else if app == ClientApp::Gemini {
                    cur.default.as_deref() == Some(model.as_str())
                } else {
                    slot.is_some() || cur.visible(&exposed).iter().any(|v| v == model)
                };
                cells.insert(
                    app.as_str().to_string(),
                    json!({
                        "in_list": in_list,
                        "is_default": cur.default.as_deref() == Some(model.as_str()),
                        "slot": slot,
                        "slots": slots,
                    }),
                );
            }
            json!({
                "model": model,
                // 队列④：来自哪些启用后端（列表序 = 后端名字典序）；空数组 = 前端不渲染标签
                "backends": backends_of
                    .get(model.as_str())
                    .cloned()
                    .unwrap_or_default(),
                "cells": Value::Object(cells),
            })
        })
        .collect();
    Ok(json!({ "clients": clients, "rows": rows }))
}

/// 以**文件真值**为基准构造新的期望：库里那份可能已经被手工编辑甩在后面。
fn base_selection(
    env: &TakeoverEnv,
    app: ClientApp,
    exposed: &[String],
) -> Result<Selection, String> {
    let mut cur = read_selection(env, app)?;
    // 一旦在矩阵里动手，就是我们在这张表上做增删了（claude 才敢清理空槽；
    // pi/codex 的槽位同理——矩阵写入是显式动过手，不该被后续无关写入洗掉）
    cur.manage_tiers = true;
    cur.manage_pi = true;
    cur.manage_codex = true;
    // 读回来的清单若是暴露全集，等价于“没特别限定”，归一成 None，省得白写一堆名字
    if let Some(allow) = &cur.allow {
        if allow.len() == exposed.len() && exposed.iter().all(|m| allow.contains(m)) {
            cur.allow = None;
        }
    }
    Ok(cur)
}

/// codex 的“清单”落地成 catalog 文件；没有模板就生不成，这时必须报错，
/// 不能让格子看起来改成功了却什么都没写（codex 那边压根看不到变化）。
fn require_catalog(env: &TakeoverEnv, app: ClientApp) -> Result<(), String> {
    if app == ClientApp::Codex && !super::codex::has_catalog_template(env) {
        return Err(
            "codex 的模型清单要写进 catalog 文件，但它没有可参考的模板条目；\
             请先在 ~/.codex 里放一份含 models 条目的 catalog，或让 codex 自己生成一份"
                .to_string(),
        );
    }
    Ok(())
}

/// 落盘前的公共校验：默认档必须在该端看得见的范围内。
fn ensure_default_visible(
    sel: &Selection,
    exposed: &[String],
    app: ClientApp,
) -> Result<(), String> {
    if let Some(def) = &sel.default {
        // 默认档根本不在暴露清单里 = 那是用户自己的模型，不归我们管，别拦
        if exposed.iter().any(|m| m == def) && !sel.visible(exposed).iter().any(|m| m == def) {
            return Err(format!(
                "{} 的默认模型 {def} 不在它的可用清单里",
                app.label()
            ));
        }
    }
    Ok(())
}

/// 改一个格子。写完立刻落盘（复用接管的快照与原子写），返回新矩阵里那一行的样子。
pub fn apply_cell(
    env: &TakeoverEnv,
    store: &Store,
    ep: &Endpoint,
    app: ClientApp,
    model: &str,
    op: CellOp,
) -> Result<Value, String> {
    /* 闸门改了口径（用户 2026-11-17）：**分配即接管**——未接管的端照样能在矩阵里点，
    写下去由 `set_model` 末尾的 `apply` 顺手完成接管（先快照原件、再写连接、再落态）。
    只有「取消分配」在未接管时是真没东西可取消，明确报错，不许静默成功。 */
    if !connection_managed(env, store, ep, app)? && matches!(op, CellOp::Remove { .. }) {
        return Err(format!(
            "{} 尚未接管，没有分配给它的模型可取消",
            app.label()
        ));
    }
    let exposed = served_models(store);
    if !exposed.iter().any(|m| m == model) {
        return Err(format!("模型 {model} 未暴露，无法分配"));
    }
    if app == ClientApp::Copilot {
        // copilot 是网关门面（Ollama 下游）：真值在 settings，没有配置文件可写，
        // 不走 base_selection/set_model 那套文件链。格子 = 可见清单的增/删。
        let add = match op {
            CellOp::Add { .. } => true,
            CellOp::Remove { .. } => false,
            CellOp::SetDefault => return Err("copilot 是可见清单，没有默认档概念".into()),
        };
        let allow = super::copilot_assign(store, model, add)?;
        return Ok(cell_report(
            model,
            &json!({
                "app": app.as_str(),
                "changed": true,
                "backup_id": Value::Null,
                "current": allow["allow"].clone(),
            }),
        ));
    }
    let mut sel = base_selection(env, app, &exposed)?;
    if app == ClientApp::Pi {
        // pi 是两槽制（主/小模型），语义与清单制/档位制都不同，单独一支。
        return apply_cell_pi(env, store, ep, &mut sel, model, op);
    }
    if app == ClientApp::Codex {
        // codex 也是双槽制（主/子代理模型），语义与清单制/档位制都不同，单独一支。
        return apply_cell_codex(env, store, ep, &mut sel, model, op, &exposed);
    }
    let visible: BTreeSet<String> = sel.visible(&exposed).into_iter().collect();

    match op {
        CellOp::SetDefault => match app {
            ClientApp::Pi => {
                return Err("pi 的默认模型由 pi 自己管理（models-store.json），我们不强改".into())
            }
            _ => {
                // 槽位之间不再互斥：设默认不动它已占的档位。同一个模型完全可以
                // 既是默认兜底、又占着 sonnet（用户明确要求，10-18）。
                sel.default = Some(model.to_string());
                // 清单制端（opencode / DevEco）：设为默认必须同时进清单，否则
                // `ensure_default_visible` 会拒（「默认模型不在可用清单里」）。
                // 用户点灯的心智是「让这个代理用它」，不该要求他先去加清单。
                // 只在已有显式清单时补——allow 为 None 表示全暴露可见，本就含它。
                if let Some(allow) = &sel.allow {
                    let mut next = allow.clone();
                    next.insert(model.to_string());
                    sel.allow = Some(next);
                }
            }
        },
        CellOp::Add { slot } => {
            if app == ClientApp::Gemini {
                return Err("gemini 只有一个模型位，请直接设为默认".into());
            }
            if let Some(slots) = slots_of(app) {
                let want = slot.filter(|s| slots.contains(&s.as_str()));
                if let Some(slot) = want {
                    // 指名档位 = 只动这一个槽：目标档有人就直接顶替，别处照旧不动。
                    // 槽位之间不再互斥——同一模型允许同时占多个槽。
                    if sel.tiers.get(&slot).map(String::as_str) == Some(model) {
                        return Err(format!("{model} 已经在 {slot} 档"));
                    }
                    sel.tiers.insert(slot.to_string(), model.to_string());
                } else {
                    // 没指名槽位 = 找个空槽塞进去。重复占槽是允许的，所以不再拦
                    // 「已经在用」，只有模型位真的满了才拒。
                    let used = sel.tiers.len() + usize::from(sel.default.is_some());
                    if let Some(cap) = capacity(app) {
                        if used >= cap {
                            return Err(format!(
                                "{} 的 {cap} 个模型位已满：点格子选择要顶替哪个槽位",
                                app.label()
                            ));
                        }
                    }
                    if sel.default.is_none() {
                        sel.default = Some(model.to_string());
                    } else {
                        let free = slots
                            .iter()
                            .find(|s| !sel.tiers.contains_key::<str>(*s))
                            .ok_or_else(|| format!("{} 的档位都占满了", app.label()))?;
                        sel.tiers.insert(free.to_string(), model.to_string());
                    }
                }
            } else {
                self::require_catalog(env, app)?;
                let mut allow = visible;
                allow.insert(model.to_string());
                sel.allow = Some(allow);
            }
        }
        CellOp::Remove { slot } => {
            if app == ClientApp::Gemini {
                return Err("gemini 只有一个模型位，请用「设为默认」切换".into());
            }
            if slots_of(app).is_some() {
                let tiers = slots_of(app).unwrap();
                let held: Vec<String> = sel
                    .tiers
                    .iter()
                    .filter(|(_, v)| *v == model)
                    .map(|(k, _)| k.clone())
                    .collect();
                // 一个模型现在能同时占多个槽，所以「摘哪一个」必须说清楚：
                // 档位键 = 只摘这一档；"main"（UI 里 = 默认档，见 slot_keys）= 只清默认；
                // 不指名 = 它占的槽全摘 + 清默认（矩阵格子的「取消分配」）。
                let (targets, clear_default) = match slot.as_deref() {
                    Some("main") => (Vec::new(), sel.default.as_deref() == Some(model)),
                    Some(s) if tiers.contains(&s) => {
                        (held.into_iter().filter(|h| h == s).collect(), false)
                    }
                    Some(_) => (Vec::new(), false),
                    None => (held, sel.default.as_deref() == Some(model)),
                };
                // 档位制端没有「清单」概念，格子恒亮：移走一个没占槽的模型是空操作。
                // 以前这里静默成功（UI 提示已写入但什么都没变），现在明确拒绝。
                if targets.is_empty() && !clear_default {
                    return Err(match slot.as_deref() {
                        Some(s) if tiers.contains(&s) => {
                            format!("{model} 没有占用 {} 的 {s} 档", app.label())
                        }
                        _ => format!(
                            "{model} 没有占用 {} 的任何档位，请改用「占档」选择槽位",
                            app.label()
                        ),
                    });
                }
                for s in targets {
                    sel.tiers.remove(&s);
                }
                if clear_default {
                    sel.default = None;
                }
            } else {
                self::require_catalog(env, app)?;
                let mut allow = visible;
                allow.remove(model);
                // 删到空不再报错（用户 2026-11-17）：清空该端的分配 = 自动取消接管，
                // 由 `set_model` 收尾还原接管前原件。
                if sel.default.as_deref() == Some(model) {
                    sel.default = allow.iter().next().cloned();
                }
                sel.allow = Some(allow);
            }
        }
    }

    ensure_default_visible(&sel, &exposed, app)?;
    let report = super::set_model(env, store, ep, app, sel)?;
    Ok(cell_report(model, &report))
}

/// 格子操作的统一回执（`apply_cell` 与其 pi 支路共用）。
fn cell_report(model: &str, report: &Value) -> Value {
    json!({
        "app": report["app"].clone(),
        "model": model,
        "changed": report["changed"].clone(),
        "backup_id": report["backup_id"].clone(),
        "current": report["current"].clone(),
    })
}

/// pi 的两个专属槽位（主/小模型）怎么改格子。
///
/// 槽位语义：格子亮 = 占了 `main`/`small` 之一或进了清单；两槽互斥性已放开
/// ——同一模型可以既当主模型又当小模型，指名槽位时目标槽有主直接顶替。
/// 清单语义（BUG-4，用户 2026-11-16）：槽位制只约束主/小两个档位槽，
/// `models.json` 的 models 数组允许多模型清单——不指名槽位的 add/remove 就是
/// 清单的追加/取消，由用户在 pi 侧选择使用。
fn apply_cell_pi(
    env: &TakeoverEnv,
    store: &Store,
    ep: &Endpoint,
    sel: &mut Selection,
    model: &str,
    op: CellOp,
) -> Result<Value, String> {
    match op {
        CellOp::SetDefault => {
            return Err("pi 的默认模型由 pi 自己管理（models-store.json），我们不强改".into())
        }
        CellOp::Add { slot } => {
            let Some(slot) = slot.filter(|s| Selection::PI_SLOT_KEYS.contains(&s.as_str())) else {
                // 没指名槽位 = 追加进清单（BUG-4）。清单缺省（None）= 全暴露，
                // 本就含它，无需落盘——回执如实说明。
                match &mut sel.allow {
                    None => {
                        return Err("pi 当前对全部暴露模型可见，无需追加".into());
                    }
                    Some(allow) => {
                        if !allow.insert(model.to_string()) {
                            return Err(format!("{model} 已在 pi 的清单里"));
                        }
                    }
                }
                let report = super::set_model(env, store, ep, ClientApp::Pi, sel.clone())?;
                return Ok(cell_report(model, &report));
            };
            let cur = sel
                .pi_slots()
                .iter()
                .find(|(k, _)| *k == slot)
                .and_then(|(_, v)| *v);
            if cur == Some(model) {
                return Err(format!("{model} 已经在 {slot} 槽"));
            }
            // 两槽不再互斥：主/小模型可以是同一个模型，这里只动指名的那一个。
            match slot.as_str() {
                "main" => sel.pi_main = Some(model.to_string()),
                _ => sel.pi_small = Some(model.to_string()),
            }
            // 占槽的模型必须在 pi 的可用清单里（清单缺省 = 全暴露，无需动）
            if let Some(allow) = &mut sel.allow {
                allow.insert(model.to_string());
            }
        }
        CellOp::Remove { slot } => {
            // 指名槽位只摘那一个；不指名 = 槽 + 清单一起摘（彻底取消分配）。
            let want = slot.filter(|s| Selection::PI_SLOT_KEYS.contains(&s.as_str()));
            let clear_main = match want.as_deref() {
                Some(s) => s == "main",
                None => sel.pi_main.as_deref() == Some(model),
            };
            let clear_small = match want.as_deref() {
                Some(s) => s != "main",
                None => sel.pi_small.as_deref() == Some(model),
            };
            let in_slot = (clear_main && sel.pi_main.as_deref() == Some(model))
                || (clear_small && sel.pi_small.as_deref() == Some(model));
            let in_list = sel.allow.as_ref().is_some_and(|a| a.contains(model));
            if !in_slot && !in_list {
                return Err(match &want {
                    Some(s) => format!("{model} 没有占用 pi 的 {s} 槽"),
                    None => format!("{model} 没有分配给 pi"),
                });
            }
            if want.is_some() && !in_slot {
                return Err(format!("{model} 没有占用 pi 的 {want:?} 槽"));
            }
            if clear_main {
                sel.pi_main = None;
            }
            if clear_small {
                sel.pi_small = None;
            }
            if want.is_none() {
                if let Some(allow) = &mut sel.allow {
                    allow.remove(model);
                }
            }
        }
    }
    let report = super::set_model(env, store, ep, ClientApp::Pi, sel.clone())?;
    Ok(cell_report(model, &report))
}

/// codex 的两个专属槽位（主模型 / 子代理模型）怎么改格子。
///
/// 主模型 = ★ 默认档，写顶层 `model`；副槽写 `[agents] default_subagent_model`
/// （codex 自主 spawn 的子代理用它，官方 subagents 文档）。两槽互斥：同一模型
/// 不能既是主模型又是子代理模型（指名槽位时先腾另一个槽，与 pi/claude 的
/// 「连根拔移入」同一语义）。主模型不可直接移除——codex 必须有个默认模型。
fn apply_cell_codex(
    env: &TakeoverEnv,
    store: &Store,
    ep: &Endpoint,
    sel: &mut Selection,
    model: &str,
    op: CellOp,
    exposed: &[String],
) -> Result<Value, String> {
    /// 占主槽：两槽不再互斥，副槽照旧——主模型完全可以同时就是子代理模型。
    fn set_main(sel: &mut Selection, model: &str) {
        sel.default = Some(model.to_string());
        // codex 的清单读回来是 catalog 里的 slug（= 上一次写盘的槽位集合），
        // 新主模型不在其中就会被 `ensure_default_visible` 拒掉。清单是过程量
        // （catalog 按槽位重写），这里先补上新主模型，校验与落盘口径才一致。
        if let Some(allow) = &mut sel.allow {
            allow.insert(model.to_string());
        }
    }
    match op {
        CellOp::SetDefault => set_main(sel, model),
        CellOp::Add { slot } => {
            let Some(slot) = slot.filter(|s| Selection::CODEX_SLOT_KEYS.contains(&s.as_str()))
            else {
                // 没指名槽位 = 追加进清单（BUG-4）：codex 的清单落点是我们生成的
                // catalog（Select Model 面板列的就是它）。清单缺省 = 全暴露，本就含它。
                match &mut sel.allow {
                    None => {
                        return Err("codex 当前对全部暴露模型可见，无需追加".into());
                    }
                    Some(allow) => {
                        require_catalog(env, ClientApp::Codex)?;
                        if !allow.insert(model.to_string()) {
                            return Err(format!("{model} 已在 codex 的清单里"));
                        }
                    }
                }
                ensure_default_visible(sel, exposed, ClientApp::Codex)?;
                let report = super::set_model(env, store, ep, ClientApp::Codex, sel.clone())?;
                return Ok(cell_report(model, &report));
            };
            match slot.as_str() {
                "main" => {
                    if sel.default.as_deref() == Some(model) {
                        return Err(format!("{model} 已经是 codex 的主模型"));
                    }
                    set_main(sel, model);
                }
                _ => {
                    if sel.codex_sub.as_deref() == Some(model) {
                        return Err(format!("{model} 已经在 sub 槽"));
                    }
                    // 副槽不再排斥主模型：同一个模型可以同时占两个槽。
                    // 副槽模型必须进得了 codex 的清单（清单缺省 = 全暴露，无需动）
                    if let Some(allow) = &mut sel.allow {
                        require_catalog(env, ClientApp::Codex)?;
                        allow.insert(model.to_string());
                    }
                    sel.codex_sub = Some(model.to_string());
                }
            }
        }
        CellOp::Remove { slot } => {
            // 副槽可以直接摘；主槽不能摘（codex 必须有主模型），只能改设。
            // 两个槽现在可能都是同一个模型，所以指名槽位时只动那一个。
            let want = slot.filter(|s| Selection::CODEX_SLOT_KEYS.contains(&s.as_str()));
            match want.as_deref() {
                Some("main") => {
                    return Err(format!(
                        "{model} 是 codex 的主模型，不能直接移除；用 ★ 或槽位浮层改设其它模型"
                    ))
                }
                Some(_) => {
                    if sel.codex_sub.as_deref() != Some(model) {
                        return Err(format!("{model} 没有占用 codex 的 sub 槽"));
                    }
                    sel.codex_sub = None;
                }
                None => {
                    if sel.codex_sub.as_deref() == Some(model) {
                        sel.codex_sub = None;
                    } else if sel.default.as_deref() == Some(model) {
                        return Err(format!(
                            "{model} 是 codex 的主模型，不能直接移除；用 ★ 或槽位浮层改设其它模型"
                        ));
                    } else if sel.allow.as_mut().is_some_and(|a| a.remove(model)) {
                        // 没占槽但在清单里 = 取消清单分配（BUG-4）。
                    } else {
                        return Err(format!("{model} 没有分配给 codex"));
                    }
                }
            }
        }
    }
    ensure_default_visible(sel, exposed, ClientApp::Codex)?;
    let report = super::set_model(env, store, ep, ClientApp::Codex, sel.clone())?;
    Ok(cell_report(model, &report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takeover::{Endpoint, TakeoverEnv};
    use tempfile::TempDir;

    struct Fix {
        env: TakeoverEnv,
        store: Store,
        ep: Endpoint,
        _tmp: TempDir,
    }

    fn fix() -> Fix {
        let tmp = TempDir::new().unwrap();
        let env = TakeoverEnv::with_roots(tmp.path().to_path_buf(), tmp.path().join("backups"));
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "b".into(),
                base_url: "http://x".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                weight: 1,
                models: vec!["m-one".into(), "m-two".into(), "m-three".into()],
                ..Default::default()
            })
            .unwrap();
        let ep = Endpoint {
            meta: Default::default(),
            base_root: "http://127.0.0.1:8787".into(),
            api_root: "http://127.0.0.1:8787/v1".into(),
            token: "tk".into(),
            models: vec!["m-one".into(), "m-two".into(), "m-three".into()],
            codex_wire_api: "chat".into(),
            direct: false,
        };
        Fix {
            env,
            store,
            ep,
            _tmp: tmp,
        }
    }

    fn take_over(f: &Fix, app: ClientApp) {
        super::super::apply(&f.env, &f.store, &f.ep, app).unwrap();
    }

    /// 清单制端（opencode / DevEco）：设默认必须顺带把模型补进清单。
    /// 用户点模型行那盏灯的心智是「让这个代理用它」，不是「只加进可见清单」；
    /// 若只设 default 而模型不在清单里，`ensure_default_visible` 会直接拒，
    /// 用户看到的就是「点了没反应，还得去代理页换模型」。
    #[test]
    fn set_default_on_catalog_client_extends_its_list() {
        let f = fix();
        take_over(&f, ClientApp::OpenCode);
        // 接管后清单只写已分配的模型：先只分一个 m-one，做出「显式清单里没有 m-two」。
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::OpenCode,
            "m-one",
            CellOp::Add { slot: None },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::OpenCode).unwrap();
        assert_eq!(
            sel.allow.as_ref().map(|a| a.len()),
            Some(1),
            "期望清单只剩 m-one"
        );
        // 把不在清单里的 m-two 设为默认：清单必须自动补上它，而不是报错。
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::OpenCode,
            "m-two",
            CellOp::SetDefault,
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::OpenCode).unwrap();
        assert_eq!(sel.default.as_deref(), Some("m-two"));
        assert!(
            sel.allow.map(|a| a.contains("m-two")).unwrap_or(false),
            "设默认没把模型补进清单"
        );
    }

    /// 槽位之间不再互斥（用户明确要求，10-18）：同一个模型可以占满 claude 所有位。
    #[test]
    fn one_model_can_fill_every_slot() {
        let f = fix();
        take_over(&f, ClientApp::Claude);
        for slot in tiers_of(ClientApp::Claude) {
            apply_cell(
                &f.env,
                &f.store,
                &f.ep,
                ClientApp::Claude,
                "m-one",
                CellOp::Add {
                    slot: Some(slot.to_string()),
                },
            )
            .unwrap();
        }
        // 默认兜底也可以是它：设默认不腾档位，占槽也不清默认。
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::SetDefault,
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Claude).unwrap();
        assert_eq!(sel.default.as_deref(), Some("m-one"));
        assert_eq!(
            sel.tiers.len(),
            tiers_of(ClientApp::Claude).len(),
            "五个档位都该是它"
        );
        assert!(sel.tiers.values().all(|v| v == "m-one"));
        // 摘单个槽只动那一个：其余槽仍是它。
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::Remove {
                slot: Some("sonnet".into()),
            },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Claude).unwrap();
        assert!(!sel.tiers.contains_key("sonnet"), "只该摘 sonnet");
        assert_eq!(sel.tiers.len(), tiers_of(ClientApp::Claude).len() - 1);
    }

    #[test]
    fn cell_ops_drive_claude_slots_and_reject_overflow() {
        let f = fix();
        take_over(&f, ClientApp::Claude);
        // 默认未设 → 第一次 Add 占默认位
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::Add { slot: None },
        )
        .unwrap();
        assert_eq!(
            read_selection(&f.env, ClientApp::Claude)
                .unwrap()
                .default
                .as_deref(),
            Some("m-one")
        );
        // 之后按 haiku→sonnet→… 找空槽
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-two",
            CellOp::Add { slot: None },
        )
        .unwrap();
        assert_eq!(
            read_selection(&f.env, ClientApp::Claude)
                .unwrap()
                .tiers
                .get("haiku")
                .map(String::as_str),
            Some("m-two")
        );
        // UI 重设计决策：指名槽位被占 = 直接顶替（连根拔移入，与角色表一致）
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-three",
            CellOp::Add {
                slot: Some("haiku".into()),
            },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Claude).unwrap();
        assert_eq!(sel.tiers.get("haiku").map(String::as_str), Some("m-three"));
        assert!(
            !sel.tiers.values().any(|v| v == "m-two"),
            "被顶替的 m-two 应已离槽"
        );
        // 移走占着槽的 m-three 后，档位就空了
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-three",
            CellOp::Remove { slot: None },
        )
        .unwrap();
        assert!(read_selection(&f.env, ClientApp::Claude)
            .unwrap()
            .tiers
            .is_empty());
    }

    /// codebuddy 与 claude 同款档位语义（lite/reasoning + 默认 = 3 个模型位）。
    #[test]
    fn codebuddy_slots_mirror_claude_semantics() {
        let f = fix();
        take_over(&f, ClientApp::CodeBuddy);
        // 首个 Add 占默认位
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::CodeBuddy,
            "m-one",
            CellOp::Add { slot: None },
        )
        .unwrap();
        // 次个 Add 按 lite→reasoning 找空槽
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::CodeBuddy,
            "m-two",
            CellOp::Add { slot: None },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::CodeBuddy).unwrap();
        assert_eq!(sel.tiers.get("lite").map(String::as_str), Some("m-two"));
        assert_eq!(sel.default.as_deref(), Some("m-one"));
        // 3 位全满后再加 → 明确拒绝（互斥靠容量卡住，不静默丢弃）
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::CodeBuddy,
            "m-three",
            CellOp::Add { slot: None },
        )
        .unwrap();
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::CodeBuddy,
            "m-one",
            CellOp::Add { slot: None },
        )
        .unwrap_err();
        assert!(
            err.contains("已满"),
            "满员后重复 Add 应被容量拦住（槽位之间不再互斥）: {err}"
        );
        // 指名 reasoning 档 = 顶替 m-three
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::CodeBuddy,
            "m-one",
            CellOp::Add {
                slot: Some("reasoning".into()),
            },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::CodeBuddy).unwrap();
        assert_eq!(
            sel.tiers.get("reasoning").map(String::as_str),
            Some("m-one")
        );
        // 槽位之间不再互斥：m-one 去占 reasoning 档，默认档照旧是它。
        assert_eq!(sel.default.as_deref(), Some("m-one"), "占槽不该清默认档");
    }

    /// 档位制端格子恒亮（无清单概念），「移走一个没占槽的模型」是空操作——
    /// 以前静默成功（UI 提示已写入但文件没变），现在明确拒绝。
    #[test]
    fn remove_refuses_model_not_occupying_any_slot() {
        let f = fix();
        take_over(&f, ClientApp::Claude);
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::Remove { slot: None },
        )
        .unwrap_err();
        assert!(err.contains("没有占用"), "{err}");
    }

    /// 清默认档（取消分配/连根拔占槽）必须真删文件里的键——以前只在有默认档时
    /// 写入，清掉后残留旧值，读回来默认档还在（UI 点击没反应的另一处根因）。
    #[test]
    fn clearing_default_really_removes_claude_keys() {
        let f = fix();
        take_over(&f, ClientApp::Claude);
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::Add { slot: None },
        )
        .unwrap();
        assert_eq!(
            read_selection(&f.env, ClientApp::Claude)
                .unwrap()
                .default
                .as_deref(),
            Some("m-one")
        );
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::Remove { slot: None },
        )
        .unwrap();
        assert_eq!(
            read_selection(&f.env, ClientApp::Claude).unwrap().default,
            None,
            "取消分配后默认档应从文件里真删"
        );
    }

    /// 矩阵下发给 UI 的列元数据：容量/已用/默认档可设性（pi 不许设默认）。
    #[test]
    fn matrix_clients_carry_capacity_used_and_default_allowed() {
        let f = fix();
        take_over(&f, ClientApp::Claude);
        take_over(&f, ClientApp::CodeBuddy);
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        let clients = m["clients"].as_array().unwrap();
        let claude = clients
            .iter()
            .find(|c| c["app"] == json!("claude"))
            .unwrap();
        assert_eq!(claude["tiers"].as_array().unwrap().len(), 5);
        assert_eq!(claude["capacity"], json!(6), "claude = 5 档位 + 默认");
        assert_eq!(claude["used"], json!(0));
        assert_eq!(claude["default_allowed"], json!(true));
        let cb = clients
            .iter()
            .find(|c| c["app"] == json!("codebuddy"))
            .unwrap();
        assert_eq!(
            cb["capacity"],
            json!(3),
            "codebuddy = lite/reasoning + 默认"
        );
        let codex = clients.iter().find(|c| c["app"] == json!("codex")).unwrap();
        assert_eq!(codex["capacity"], json!(2), "codex = 主/副两槽");
        assert_eq!(
            codex["slots"],
            json!(["main", "sub"]),
            "codex 槽位键下发给 UI 画浮层"
        );
        let pi = clients.iter().find(|c| c["app"] == json!("pi")).unwrap();
        assert_eq!(pi["default_allowed"], json!(false), "pi 默认档自理");
        let gemini = clients
            .iter()
            .find(|c| c["app"] == json!("gemini"))
            .unwrap();
        assert_eq!(gemini["capacity"], json!(1));
        // claude 占默认位后 used=1
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::Add { slot: None },
        )
        .unwrap();
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        let claude = m["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["app"] == json!("claude"))
            .unwrap();
        assert_eq!(claude["used"], json!(1));
    }

    #[test]
    fn cell_ops_on_list_based_clients_materialize_allow_list() {
        let f = fix();
        take_over(&f, ClientApp::OpenCode);
        // 接管后清单只写**已分配**给它的模型：一个都没分 → 清单空，格子全暗。
        let cur = read_selection(&f.env, ClientApp::OpenCode).unwrap();
        assert_eq!(cur.visible(&f.ep.models).len(), 0, "没分配就不该有清单");
        // 分两个（清单变成显式两项），默认档设给其中一个
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::OpenCode,
            "m-one",
            CellOp::Add { slot: None },
        )
        .unwrap();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::OpenCode,
            "m-two",
            CellOp::Add { slot: None },
        )
        .unwrap();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::OpenCode,
            "m-one",
            CellOp::SetDefault,
        )
        .unwrap();
        let cur = read_selection(&f.env, ClientApp::OpenCode).unwrap();
        assert_eq!(cur.visible(&f.ep.models).len(), 2);
        assert_eq!(cur.default.as_deref(), Some("m-one"));
        // 移走默认档：默认跟着挪走，不会留一个选不到的默认
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::OpenCode,
            "m-one",
            CellOp::Remove { slot: None },
        )
        .unwrap();
        let cur = read_selection(&f.env, ClientApp::OpenCode).unwrap();
        assert_eq!(cur.visible(&f.ep.models).len(), 1);
        assert_eq!(cur.default.as_deref(), Some("m-two"), "默认档得跟着挪走");
        // 删最后一个不再报错（用户 2026-11-17）：清空该端的分配 = 自动取消接管。
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::OpenCode,
            "m-two",
            CellOp::Remove { slot: None },
        )
        .unwrap();
        assert!(
            !super::super::taken_over(&f.store, ClientApp::OpenCode),
            "分配清空后必须自动取消接管"
        );
    }

    /// 格子亮 = 这个端**真的在用**它。档位制端（claude）读不到清单，以前 `visible`
    /// 兜底成全暴露 → 整列恒亮，现在看占槽/默认；清单制（opencode）看清单（清单只含
    /// 已分配模型）；pi 看占槽；gemini 看默认档。
    #[test]
    fn cells_light_up_only_for_models_actually_assigned() {
        let f = fix();
        take_over(&f, ClientApp::Claude);
        take_over(&f, ClientApp::OpenCode);
        take_over(&f, ClientApp::Pi);
        let in_list = |app: &str, model: &str| {
            let m = matrix(&f.env, &f.store, &f.ep).unwrap();
            m["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["model"] == json!(model))
                .expect("矩阵行")["cells"][app]["in_list"]
                .as_bool()
                .expect("in_list 是布尔")
        };
        // 一个都没分配：整列全暗（以前 claude 这一列永远亮着，毫无信息量）
        assert!(!in_list("claude", "m-one"));
        assert!(!in_list("claude", "m-two"));
        assert!(!in_list("opencode", "m-one"));
        assert!(!in_list("pi", "m-one"));
        assert!(!in_list("gemini", "m-three"));

        // 占槽/占清单后只有那个模型亮，同列别的格子照旧暗
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-two",
            CellOp::Add {
                slot: Some("haiku".into()),
            },
        )
        .unwrap();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::OpenCode,
            "m-one",
            CellOp::Add { slot: None },
        )
        .unwrap();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-one",
            CellOp::Add {
                slot: Some("main".into()),
            },
        )
        .unwrap();
        assert!(in_list("claude", "m-two"));
        assert!(!in_list("claude", "m-one"), "没占槽的格子必须暗");
        assert!(in_list("opencode", "m-one"));
        assert!(!in_list("opencode", "m-two"));
        assert!(in_list("pi", "m-one"));
        assert!(!in_list("pi", "m-two"));

        // 默认档也算「在用」（claude 没有清单，默认档就是它的主模型位）
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::SetDefault,
        )
        .unwrap();
        assert!(in_list("claude", "m-one"));
    }

    /// copilot 门面列：矩阵里恒接管态（真值在 settings）、格子改可见清单、
    /// 无默认档概念、清空拒绝、恰好全暴露时归一回 null。
    #[test]
    fn copilot_facade_column_and_cell_ops_write_settings() {
        let f = fix();
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        let col = m["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["app"] == json!("copilot"))
            .unwrap()
            .clone();
        assert_eq!(
            col["taken_over"],
            json!(true),
            "门面恒就绪，没有外部文件可评估"
        );
        assert_eq!(col["capacity"], Value::Null, "清单制无容量上限");
        assert_eq!(col["slots"].as_array().unwrap().len(), 0, "没有槽位");
        assert_eq!(col["default_allowed"], json!(false), "没有默认档");
        assert_eq!(col["facade"], json!(true));
        assert!(col["note"].as_str().unwrap().contains("门面"), "{col:?}");
        // 缺省全暴露：格子全亮
        let row_of = |m: &Value, model: &str| {
            m["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["model"] == json!(model))
                .unwrap()
                .clone()
        };
        assert_eq!(
            row_of(&m, "m-one")["cells"]["copilot"]["in_list"],
            json!(true)
        );
        // 摘除 → settings 里落显式清单，矩阵格子随之熄灭
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Copilot,
            "m-three",
            CellOp::Remove { slot: None },
        )
        .unwrap();
        assert_eq!(
            f.store.copilot_allow().unwrap(),
            vec!["m-one".to_string(), "m-two".to_string()]
        );
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        assert_eq!(
            row_of(&m, "m-three")["cells"]["copilot"]["in_list"],
            json!(false)
        );
        // 挂回 → 恰好全暴露，归一成缺省 null（与门面零配置默认态同口径）
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Copilot,
            "m-three",
            CellOp::Add { slot: None },
        )
        .unwrap();
        assert!(f.store.copilot_allow().is_none());
        // 摘到只剩一个后再拒绝摘最后一个
        for model in ["m-one", "m-two"] {
            apply_cell(
                &f.env,
                &f.store,
                &f.ep,
                ClientApp::Copilot,
                model,
                CellOp::Remove { slot: None },
            )
            .unwrap();
        }
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Copilot,
            "m-three",
            CellOp::Remove { slot: None },
        )
        .unwrap_err();
        assert!(err.contains("至少"), "{err}");
        // 没有默认档概念；未暴露模型也照旧拒绝
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Copilot,
            "m-one",
            CellOp::SetDefault,
        )
        .unwrap_err();
        assert!(err.contains("默认档"), "{err}");
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Copilot,
            "ghost",
            CellOp::Add { slot: None },
        )
        .unwrap_err();
        assert!(err.contains("未暴露"), "{err}");
    }

    /// 未接管的端不再被闸门挡住（用户 2026-11-17：**分配即接管**）——点格子就把它
    /// 接管上，原件先立备份；未暴露的模型照旧拒绝，那是数据问题不是接管问题。
    #[test]
    fn assigning_to_an_untaken_over_client_takes_it_over() {
        let f = fix();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::SetDefault,
        )
        .unwrap();
        assert!(
            super::super::taken_over(&f.store, ClientApp::Claude),
            "分配一个模型就该自动接管"
        );
        assert!(
            super::super::list_backups(&f.env)
                .unwrap()
                .iter()
                .any(|b| b.app == "claude"),
            "自动接管也必须先立原件备份"
        );
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "ghost",
            CellOp::SetDefault,
        )
        .unwrap_err();
        assert!(err.contains("未暴露"), "{err}");
    }

    /// 清空该端的全部分配 = 自动取消接管（用户 2026-11-17）：还原原件、清接管态。
    #[test]
    fn clearing_every_assignment_releases_takeover() {
        let f = fix();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::SetDefault,
        )
        .unwrap();
        assert!(super::super::taken_over(&f.store, ClientApp::Claude));
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::Remove {
                slot: Some("main".into()),
            },
        )
        .unwrap();
        assert!(
            !super::super::taken_over(&f.store, ClientApp::Claude),
            "最后一个分配被摘走就必须自动取消接管"
        );
    }

    /// 未接管时点「取消分配」没有东西可取消，必须明确报错，不许静默成功。
    #[test]
    fn remove_on_an_untaken_over_client_is_refused() {
        let f = fix();
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Claude,
            "m-one",
            CellOp::Remove { slot: None },
        )
        .unwrap_err();
        assert!(err.contains("尚未接管"), "{err}");
    }

    #[test]
    fn matrix_reports_truth_from_files() {
        let f = fix();
        take_over(&f, ClientApp::Codex);
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        assert_eq!(m["rows"].as_array().unwrap().len(), 3);
        let codex_col = &m["clients"].as_array().unwrap()[1];
        assert_eq!(codex_col["taken_over"], json!(true));
        // codex 列已改双槽制：没占槽的格子不再整列默认亮起（旧清单制语义已退役）
        assert_eq!(m["rows"][0]["cells"]["codex"]["in_list"], json!(false));
        assert_eq!(codex_col["used"], json!(0));
    }

    /// 队列④：行带后端标注——与 served_models 同源（enabled × exposed）：
    /// 共享模型标多家（字典序），停用后端不列。
    #[test]
    fn matrix_rows_carry_backend_annotation() {
        let f = fix();
        f.store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "b2".into(),
                base_url: "http://y".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                weight: 1,
                models: vec!["m-one".into()],
                ..Default::default()
            })
            .unwrap();
        f.store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "b-off".into(),
                base_url: "http://z".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: false,
                weight: 1,
                models: vec!["m-two".into()],
                ..Default::default()
            })
            .unwrap();
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        let rows = m["rows"].as_array().unwrap();
        let backends_of =
            |model: &str| rows.iter().find(|r| r["model"] == model).unwrap()["backends"].clone();
        // b 与 b2 共享 m-one（list_backends ORDER BY name → b 在前）
        assert_eq!(backends_of("m-one"), json!(["b", "b2"]));
        // 停用的 b-off 不进标注（m-two 行仍在——由启用的 b 提供）
        assert_eq!(backends_of("m-two"), json!(["b"]));
        assert_eq!(backends_of("m-three"), json!(["b"]));
    }

    /// codex 主/副双槽：占副槽写 [agents] default_subagent_model，主/副互斥，
    /// 主模型不可移除，副槽可单独腾出。
    #[test]
    fn codex_main_sub_slots_roundtrip_through_matrix() {
        let f = fix();
        take_over(&f, ClientApp::Codex);
        // 占副槽：落盘进 [agents].default_subagent_model
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-two",
            CellOp::Add {
                slot: Some("sub".into()),
            },
        )
        .unwrap();
        let text = std::fs::read_to_string(super::super::codex::config_path(&f.env)).unwrap();
        assert!(
            text.contains("default_subagent_model = \"m-two\""),
            "{text}"
        );
        assert_eq!(
            read_selection(&f.env, ClientApp::Codex)
                .unwrap()
                .codex_sub
                .as_deref(),
            Some("m-two")
        );
        // 主模型占 sub 被拒（已先占了 m-two 之外的主模型再试）
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-one",
            CellOp::SetDefault,
        )
        .unwrap();
        // 两槽不再互斥：主模型可以同时占副槽（用户明确要求，10-18）。
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-one",
            CellOp::Add {
                slot: Some("sub".into()),
            },
        )
        .unwrap();
        let both = read_selection(&f.env, ClientApp::Codex).unwrap();
        assert_eq!(both.default.as_deref(), Some("m-one"), "占副槽不该动主模型");
        assert_eq!(both.codex_sub.as_deref(), Some("m-one"));
        // 主模型改设别家：副槽照旧留着，槽位之间互不牵连。
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-two",
            CellOp::SetDefault,
        )
        .unwrap();
        let cur = read_selection(&f.env, ClientApp::Codex).unwrap();
        assert_eq!(cur.default.as_deref(), Some("m-two"));
        assert_eq!(cur.codex_sub.as_deref(), Some("m-one"));
        // 主模型不能直接移除
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-two",
            CellOp::Remove { slot: None },
        )
        .unwrap_err();
        assert!(err.contains("主模型"), "{err}");
        // 没占槽也没进清单的模型移除被拒（BUG-4 后：没分配就是没分配）；
        // m-one 现在是副槽，改用没上槽的 m-three。
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-three",
            CellOp::Remove { slot: None },
        )
        .unwrap_err();
        assert!(err.contains("没有分配"), "{err}");
        // 副槽单独腾出：只动副槽，主模型不受影响
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-three",
            CellOp::Add {
                slot: Some("sub".into()),
            },
        )
        .unwrap();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-three",
            CellOp::Remove { slot: None },
        )
        .unwrap();
        let cur = read_selection(&f.env, ClientApp::Codex).unwrap();
        assert_eq!(cur.codex_sub, None);
        assert_eq!(cur.default.as_deref(), Some("m-two"));
        // 矩阵回执：used 只算主模型一个，m-two 格子亮且带 ★（按模型名找行，不假设行序）
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        let codex_col = m["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["app"] == json!("codex"))
            .unwrap();
        assert_eq!(codex_col["used"], json!(1));
        let row = m["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["model"] == json!("m-two"))
            .unwrap();
        let cell = &row["cells"]["codex"];
        assert_eq!(cell["in_list"], json!(true));
        assert_eq!(cell["is_default"], json!(true));
        assert_eq!(cell["slot"], json!("main"));
    }

    /// 只改默认档的写入不洗副槽（manage_codex 没置真时以文件真值为准）。
    #[test]
    fn codex_sub_slot_survives_unmanaged_writes() {
        let f = fix();
        take_over(&f, ClientApp::Codex);
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-two",
            CellOp::Add {
                slot: Some("sub".into()),
            },
        )
        .unwrap();
        // 面板/REST 只改默认档的老式写入（不带 manage_codex）
        super::super::set_model(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            Selection {
                default: Some("m-one".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let cur = read_selection(&f.env, ClientApp::Codex).unwrap();
        assert_eq!(cur.default.as_deref(), Some("m-one"));
        assert_eq!(
            cur.codex_sub.as_deref(),
            Some("m-two"),
            "副槽没被无关写入洗掉"
        );
    }

    /// 代理启停（代理 tab 片三）：停用 = 从矩阵消失（列与格子都不下发），
    /// 分配原样保留在文件里，启用即恢复。
    #[test]
    fn disabled_agent_drops_from_matrix_columns() {
        let f = fix();
        take_over(&f, ClientApp::Claude);
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        assert_eq!(
            m["clients"].as_array().unwrap().len(),
            ClientApp::ALL.len(),
            "默认全员在列"
        );

        f.store.set_agent_enabled("claude", false).unwrap();
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        let clients = m["clients"].as_array().unwrap();
        assert_eq!(clients.len(), ClientApp::ALL.len() - 1);
        assert!(clients.iter().all(|c| c["app"] != json!("claude")));
        for row in m["rows"].as_array().unwrap() {
            assert!(row["cells"].get("claude").is_none(), "停用端的格子也不下发");
        }

        f.store.set_agent_enabled("claude", true).unwrap();
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        assert_eq!(m["clients"].as_array().unwrap().len(), ClientApp::ALL.len());
    }

    /// pi 两槽制：占槽/顶替/互斥/腾槽，格子真值与列元数据（容量 2、slots 下发）。
    #[test]
    fn pi_two_slots_assign_and_can_share_one_model() {
        let f = fix();
        take_over(&f, ClientApp::Pi);

        // 不指名槽位 = 清单追加（BUG-4）；清单缺省（全暴露）时无处可加，如实拒绝。
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-one",
            CellOp::Add { slot: None },
        )
        .unwrap_err();
        assert!(err.contains("无需追加"), "{err}");

        // 指名槽位：main / small 各占一个
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-one",
            CellOp::Add {
                slot: Some("main".into()),
            },
        )
        .unwrap();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-two",
            CellOp::Add {
                slot: Some("small".into()),
            },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Pi).unwrap();
        assert_eq!(sel.pi_main.as_deref(), Some("m-one"));
        assert_eq!(sel.pi_small.as_deref(), Some("m-two"));

        // 两槽不再互斥：m-one 已在 main，再占 small = 两个槽都是它，main 不腾空。
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-one",
            CellOp::Add {
                slot: Some("small".into()),
            },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Pi).unwrap();
        assert_eq!(
            sel.pi_main.as_deref(),
            Some("m-one"),
            "占 small 不该腾空 main"
        );
        assert_eq!(sel.pi_small.as_deref(), Some("m-one"));
        // 同槽重复占 = 拒绝（同一个槽里已经是它，没有第二份可占）
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-one",
            CellOp::Add {
                slot: Some("small".into()),
            },
        )
        .unwrap_err();
        assert!(err.contains("已经在 small 槽"), "{err}");

        // 指名槽位摘除：只摘 small，main 留着同一个模型。
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-one",
            CellOp::Remove {
                slot: Some("small".into()),
            },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Pi).unwrap();
        assert_eq!(sel.pi_main.as_deref(), Some("m-one"));
        assert_eq!(sel.pi_small, None, "只该摘 small");

        // 顶替：m-three 占 main 顶掉…（main 现为空）；再让 m-three 挤掉 small 的 m-one
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-three",
            CellOp::Add {
                slot: Some("small".into()),
            },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Pi).unwrap();
        assert_eq!(
            sel.pi_small.as_deref(),
            Some("m-three"),
            "目标槽有人直接顶替"
        );

        // 移走没占槽也没进清单的模型 = 明确拒绝（BUG-4 后口径：没分配就是没分配）；
        // 占槽的移走 = 腾空。
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-two",
            CellOp::Remove { slot: None },
        )
        .unwrap_err();
        assert!(err.contains("没有分配"), "{err}");
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-three",
            CellOp::Remove { slot: None },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Pi).unwrap();
        assert_eq!(sel.pi_small, None);

        // pi 没有默认档，设默认 = 拒绝
        let err = apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-one",
            CellOp::SetDefault,
        )
        .unwrap_err();
        assert!(err.contains("默认模型由 pi 自己管理"), "{err}");
    }

    /// BUG-4 回归：pi/codex 的格子不指名槽位 = 清单追加/取消（槽位只约束档位槽）。
    #[test]
    fn pi_and_codex_cells_toggle_model_list_without_slot() {
        let mut f = fix();
        // 加第四个暴露模型：清单 = 暴露全集时归一回「全暴露」，没第四个就测不出显式清单。
        f.store
            .upsert_backend(&crate::types::BackendUpsert {
                name: "b".into(),
                base_url: "http://x".into(),
                protocol: crate::types::Protocol::OpenaiChat,
                enabled: true,
                weight: 1,
                models: vec![
                    "m-one".into(),
                    "m-two".into(),
                    "m-three".into(),
                    "m-four".into(),
                ],
                ..Default::default()
            })
            .unwrap();
        f.ep.models.push("m-four".into());
        take_over(&f, ClientApp::Pi);
        // 先占两个槽，让清单落到显式态（Some）
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-one",
            CellOp::Add {
                slot: Some("main".into()),
            },
        )
        .unwrap();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-two",
            CellOp::Add {
                slot: Some("small".into()),
            },
        )
        .unwrap();
        // 不指名槽位 = 追加进清单：m-three 进 models 清单，不占任何槽
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-three",
            CellOp::Add { slot: None },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Pi).unwrap();
        assert!(
            sel.allow.as_ref().is_some_and(|a| a.contains("m-three")),
            "m-three 应进清单"
        );
        assert_eq!(sel.pi_main.as_deref(), Some("m-one"), "清单追加不动槽位");
        // 再点 = 取消清单分配
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-three",
            CellOp::Remove { slot: None },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Pi).unwrap();
        assert!(
            !sel.allow.as_ref().is_some_and(|a| a.contains("m-three")),
            "应已从清单摘掉"
        );

        // codex：同样口径（清单落点 = catalog，要有模板才生成得出来）。
        // 模板要在接管**之后**写：接管本身会按占槽重写 catalog，把模板位洗掉。
        take_over(&f, ClientApp::Codex);
        std::fs::write(
            super::super::codex::config_path(&f.env)
                .parent()
                .unwrap()
                .join(super::super::codex::CATALOG_FILE),
            r#"{"models":[{"slug":"m-one","display_name":"m-one","description":"模板条目"}]}"#,
        )
        .unwrap();
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-three",
            CellOp::Add { slot: None },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Codex).unwrap();
        assert!(
            sel.allow.as_ref().is_some_and(|a| a.contains("m-three")),
            "m-three 应进 codex catalog"
        );
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Codex,
            "m-three",
            CellOp::Remove { slot: None },
        )
        .unwrap();
        let sel = read_selection(&f.env, ClientApp::Codex).unwrap();
        assert!(
            !sel.allow.as_ref().is_some_and(|a| a.contains("m-three")),
            "应已从 catalog 摘掉"
        );
    }

    /// 矩阵下发的 pi 列：容量 2、slots=[main,small]、格子亮 = 占槽、used = 已占数。
    #[test]
    fn matrix_pi_column_carries_slots_capacity_and_lit_cells() {
        let f = fix();
        take_over(&f, ClientApp::Pi);
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        let pi = m["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["app"] == json!("pi"))
            .unwrap();
        assert_eq!(pi["capacity"], json!(2), "pi = 主/小两个槽");
        assert_eq!(pi["slots"], json!(["main", "small"]));
        assert_eq!(pi["used"], json!(0));
        assert_eq!(pi["default_allowed"], json!(false));
        // 没占槽时格子全灭（清单制的「全暴露恒亮」不适用于 pi 的格子语义）
        for row in m["rows"].as_array().unwrap() {
            assert_eq!(row["cells"]["pi"]["in_list"], json!(false), "{row}");
            assert_eq!(row["cells"]["pi"]["slot"], Value::Null, "{row}");
        }

        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-two",
            CellOp::Add {
                slot: Some("small".into()),
            },
        )
        .unwrap();
        let m = matrix(&f.env, &f.store, &f.ep).unwrap();
        let pi = m["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["app"] == json!("pi"))
            .unwrap();
        assert_eq!(pi["used"], json!(1));
        for row in m["rows"].as_array().unwrap() {
            let cell = &row["cells"]["pi"];
            if row["model"] == json!("m-two") {
                assert_eq!(cell["in_list"], json!(true), "{row}");
                assert_eq!(cell["slot"], json!("small"), "{row}");
            } else {
                assert_eq!(cell["in_list"], json!(false), "{row}");
            }
        }
    }

    /// 面板/REST 的普通写入（没动过槽位）不得洗掉已配好的 pi 槽位。
    #[test]
    fn plain_model_set_preserves_pi_slots() {
        let f = fix();
        take_over(&f, ClientApp::Pi);
        apply_cell(
            &f.env,
            &f.store,
            &f.ep,
            ClientApp::Pi,
            "m-one",
            CellOp::Add {
                slot: Some("main".into()),
            },
        )
        .unwrap();
        // 面板语义：只带 default/tiers 的 Selection（槽位字段全空、manage_pi=false）
        super::super::set_model(&f.env, &f.store, &f.ep, ClientApp::Pi, Selection::default())
            .unwrap();
        let sel = read_selection(&f.env, ClientApp::Pi).unwrap();
        assert_eq!(sel.pi_main.as_deref(), Some("m-one"), "槽位没被洗掉");
    }
}
