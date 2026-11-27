//! 宏库（C3，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）：命名动作序列 + 独立存储。
//!
//! 纯逻辑，无 cfg、无 FFI（默认 `cargo test` 就编到并全测）。存储对齐 schemes 口径：
//! `<data_dir>/remote/macros.<profile>.json`，pretty JSON，按档案分仓，导入导出就是拷文件。
//!
//! 键槽里存的是 `Action::Macro { id }` 引用，**不是步骤副本**：宏改一次所有绑点同时生效，
//! 删除宏时跨「当前映射 + 全部方案」清悬空引用（SayAll `removeMacroReferences` 口径）。
//! 副本方案做不到这两件事，故计划原文的 `Action::Sequence{steps,…}` 落成「引用 + 宏库」，
//! `steps`/`stop_on_error` 原样住在 `Macro` 上。

use crate::services::remote::actions::{self, Action, ButtonConfig, Slot};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 单步延迟上限（计划 §三：0–5000ms 封顶）。超了按上限截不报错——
/// 手滑多打一个 0 不该让整条宏存不下去。
pub const MAX_DELAY_MS: u32 = 5000;

/// 序列步骤：一个动作，或一段等待。
/// 步骤里不许再套宏（宏套宏没有终止保证，UI 也不给这个入口），`normalize` 兜住。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    Action { action: Action },
    Delay { ms: u32 },
}

impl Step {
    /// 日志/报错用的一行读数（后端兜底文案，前端有自己的 i18n）。
    pub fn label(&self) -> String {
        match self {
            Self::Action { action } => action.label(),
            Self::Delay { ms } => format!("等待 {ms}ms"),
        }
    }

    /// 就地归一：延迟封顶到 `MAX_DELAY_MS`（负数不存在，u32 天然非负）。
    pub fn clamp(&mut self) {
        if let Self::Delay { ms } = self {
            *ms = (*ms).min(MAX_DELAY_MS);
        }
    }
}

/// 一条宏：命名 + 是否遇错即停 + 步骤 + 用户排定的次序。
/// `order` 是「动作列表整组挪位置」（计划 2026-11-25 片③）落下来的唯一真相：
/// 存储仍是 id→map（执行按 id 解析、删宏要跨方案清引用，都靠这个形状），
/// 列表顺序单独用一个数存，不动 map 形状。**旧库没这字段 → 全 0 → 退回按名排**，行为不变。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Macro {
    /// 稳定 id（宏库的键）。新建时留空由后端生成，前端不必造 id。
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub stop_on_error: bool,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// 列表次序（`reorder` 写入）。同值（旧库全 0）时按名排。
    #[serde(default)]
    pub order: u32,
}

impl Macro {
    /// 校验 + 就地归一（名称去空白、延迟封顶）。返回给命令层的错必须能直接给人看。
    pub fn normalize(&mut self) -> Result<(), String> {
        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            return Err("宏名称不能为空".to_string());
        }
        if self.steps.is_empty() && !special::is_special(&self.id) {
            return Err("宏至少要有一个步骤".to_string());
        }
        for s in &self.steps {
            if let Step::Action {
                action: Action::Macro { .. },
            } = s
            {
                return Err("宏步骤里不能再套宏".to_string());
            }
        }
        self.steps.iter_mut().for_each(Step::clamp);
        Ok(())
    }
}

/// 宏库文件（`remote/macros.<profile>.json`）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MacroFile {
    #[serde(default = "default_format_version")]
    pub format_version: u32,
    #[serde(default)]
    pub macros: BTreeMap<String, Macro>,
}

fn default_format_version() -> u32 {
    1
}

pub fn macros_path(data_dir: &Path, profile_id: &str) -> PathBuf {
    actions::config_dir(data_dir).join(format!("macros.{profile_id}.json"))
}

/// 读宏库：文件缺失/坏档回空库（与 schemes 同口径，坏档不静默吞——下次保存覆盖它）。
pub fn load(data_dir: &Path, profile_id: &str) -> MacroFile {
    let mut file = std::fs::read_to_string(macros_path(data_dir, profile_id))
        .ok()
        .and_then(|raw| serde_json::from_str::<MacroFile>(&raw).ok())
        .unwrap_or_default();
    // 特殊宏补齐（计划 §三.2）：面板表要引用它们、键位界面要显示功能名。
    // 用 or_insert——用户改了名字/删过就不再塞回去，不跟用户较劲。
    for m in special::all() {
        file.macros.entry(m.id.clone()).or_insert(m);
    }
    file
}

/// 保存前按「归一化后的宏名」去重（计划 2026-11-26「备注字段与去重」：动作库照搬快速面板口径，
/// 归一函数同源 `quickpanel::normalize_text`，剔零宽/控制符 + 去首尾空白，不改内容本身）。
/// 「保留第一条」按用户看到的列表顺序（order → 名 → id）判，与 `list()` 同口径；
/// 只删重复条目，宏本体一个字段不动。
fn dedupe_by_name(file: &MacroFile) -> MacroFile {
    let mut items: Vec<&Macro> = file.macros.values().collect();
    items.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut seen: Vec<String> = Vec::with_capacity(items.len());
    let mut macros: BTreeMap<String, Macro> = BTreeMap::new();
    for m in items {
        let k = crate::services::quickpanel::normalize_text(&m.name);
        if seen.contains(&k) {
            continue;
        }
        seen.push(k);
        macros.insert(m.id.clone(), m.clone());
    }
    MacroFile {
        format_version: file.format_version,
        macros,
    }
}

fn write(data_dir: &Path, profile_id: &str, file: &MacroFile) -> Result<(), String> {
    let file = dedupe_by_name(file);
    let path = macros_path(data_dir, profile_id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("宏目录创建失败：{e}"))?;
    }
    let raw = serde_json::to_string_pretty(&file).map_err(|e| format!("宏序列化失败：{e}"))?;
    std::fs::write(&path, raw).map_err(|e| format!("宏文件写入失败：{e}"))
}

/// 宏列表（UI 列表顺序）：先按用户排的 `order`，同序（旧库全是 0）再按名。
pub fn list(data_dir: &Path, profile_id: &str) -> Vec<Macro> {
    let mut out: Vec<Macro> = load(data_dir, profile_id).macros.into_values().collect();
    out.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// 按给定 id 顺序重排宏库（片③：动作列表整组挪位置）。
///
/// 口径：`ids` 里第 k 条就拿到 order=k；**没点到的宏按原序接在后面**，
/// 不会凭空窜到列表最前（前端只发可见那批时也不丢宏）。id 不存在直接报错，
/// 不静默跳过——静默跳过的表现是「拖了没反应」，人无从知道哪条没了。
pub fn reorder(data_dir: &Path, profile_id: &str, ids: &[String]) -> Result<(), String> {
    let mut file = load(data_dir, profile_id);
    if ids.is_empty() {
        return Err("重排至少要给一条宏".to_string());
    }
    for id in ids {
        if !file.macros.contains_key(id.as_str()) {
            return Err(format!("宏「{id}」不存在，重排没生效"));
        }
    }
    let mut next = 0u32;
    for id in ids {
        if let Some(m) = file.macros.get_mut(id) {
            m.order = next;
        }
        next += 1;
    }
    // 没点到的按旧 order、再按名接在后面（与 list() 同口径）。
    let mut rest: Vec<(u32, String, String)> = file
        .macros
        .iter()
        .filter(|(id, _)| !ids.iter().any(|x| x == *id))
        .map(|(id, m)| (m.order, m.name.clone(), id.clone()))
        .collect();
    rest.sort();
    for (_, _, id) in rest {
        if let Some(m) = file.macros.get_mut(&id) {
            m.order = next;
        }
        next += 1;
    }
    write(data_dir, profile_id, &file)
}

/// 取一条宏（按键执行时按 id 解析）。
pub fn get(data_dir: &Path, profile_id: &str, id: &str) -> Option<Macro> {
    load(data_dir, profile_id).macros.remove(id)
}

/// 新增/覆盖一条宏（同 id = 覆盖，改名也走这里）。id 为空即生成新 id，回存好的整条。
pub fn upsert(data_dir: &Path, profile_id: &str, mut mac: Macro) -> Result<Macro, String> {
    mac.normalize()?;
    let mut file = load(data_dir, profile_id);
    let fresh = mac.id.trim().is_empty() || !file.macros.contains_key(&mac.id);
    if mac.id.trim().is_empty() {
        mac.id = uuid::Uuid::new_v4().simple().to_string();
    }
    if fresh && mac.order == 0 {
        // 新建（库里还没有这条）才要定序：库里已有人显式排过序（order>0）就追加到底；
        // 全库都是 0（旧库常态、还没人排过序）就留 0，继续按名排——与旧库行为一致。
        if file.macros.values().any(|m| m.order > 0) {
            mac.order = file.macros.values().map(|m| m.order).max().unwrap_or(0) + 1;
        }
    }
    file.macros.insert(mac.id.clone(), mac.clone());
    write(data_dir, profile_id, &file)?;
    Ok(mac)
}

/// 删除宏，并跨「当前映射 + 全部方案」清掉引用它的槽位。回清掉的引用条数。
pub fn delete(data_dir: &Path, profile_id: &str, id: &str) -> Result<usize, String> {
    let mut file = load(data_dir, profile_id);
    if file.macros.remove(id).is_none() {
        return Err(format!("宏「{id}」不存在"));
    }
    write(data_dir, profile_id, &file)?;
    Ok(remove_references(data_dir, profile_id, id))
}

/// 清悬空引用（SayAll `removeMacroReferences` 口径）：当前映射文件 + 该档案全部方案。
/// 只在删除宏时调用；执行时引用不到宏另有明确报错，不静默 no-op。
pub fn remove_references(data_dir: &Path, profile_id: &str, id: &str) -> usize {
    let mut cleared = 0;
    // 当前映射：文件不存在就别凭空造一个（没配过键位的档案是常态）。
    let path = actions::config_path(data_dir, profile_id);
    if let Ok(raw) = std::fs::read_to_string(&path) {
        let cfg = actions::parse(&raw);
        let (cleaned, n) = strip(&cfg, id);
        if n > 0 {
            if let Err(e) = actions::save(data_dir, profile_id, &cleaned) {
                tracing::warn!(target: "llm_switch::remote", err = %e, "清宏引用：写回当前映射失败");
            } else {
                cleared += n;
            }
        }
    }
    let mut file = actions::load_schemes(data_dir, profile_id);
    let mut dirty = false;
    for cfg in file.schemes.values_mut() {
        let (cleaned, n) = strip(cfg, id);
        if n > 0 {
            *cfg = cleaned;
            dirty = true;
            cleared += n;
        }
    }
    if dirty {
        if let Err(e) = actions::write_schemes(data_dir, profile_id, &file) {
            tracing::warn!(target: "llm_switch::remote", err = %e, "清宏引用：写回方案文件失败");
        }
    }
    cleared
}

/// 从一份映射里摘掉引用该宏的槽。回（新映射，摘掉的槽数）。
pub fn strip(cfg: &ButtonConfig, id: &str) -> (ButtonConfig, usize) {
    let mut out = cfg.clone();
    let mut n = 0;
    for kc in out.keys.values_mut() {
        for slot in Slot::ALL {
            if matches!(kc.slot(slot), Some(Action::Macro { id: m }) if m == id) {
                kc.set_slot(slot, None);
                n += 1;
            }
        }
    }
    (out, n)
}

/// 单步执行结果（调用方给的）。`Abort` = 整条序列别再往下跑（录制开始、通道关了这类
/// 上下文变化），与 `Fail`（这步没做成）分开：遇错继续跑是用户显式选的，上下文没了不是。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Ok,
    Fail(String),
    Abort(String),
}

/// 整条序列的跑完读数。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SeqOutcome {
    /// 成功跑完的动作步数（延迟不计）。
    pub ran: usize,
    /// 失败明细：(步骤下标, 原因)。`stop_on_error = true` 时最多一条。
    pub errors: Vec<(usize, String)>,
    /// 是否中途停下（遇错即停，或被 Abort）。
    pub stopped: bool,
}

/// 跑一条宏（计划 §三：执行器逐步跑、前步失败按 `stop_on_error` 决定中断）。
/// 线程调度由调用方决定（键盘注入必须在主线程，见 `hid.rs`），这里只管顺序与卡点。
pub fn run_sequence<F>(steps: &[Step], stop_on_error: bool, exec: &mut F) -> SeqOutcome
where
    F: FnMut(&Action) -> StepOutcome,
{
    run_sequence_with(steps, stop_on_error, exec, |ms| {
        std::thread::sleep(std::time::Duration::from_millis(u64::from(ms)))
    })
}

/// 把 sleep 也交出去的可测版本（单测不许真等 5 秒）。
pub fn run_sequence_with<F, S>(
    steps: &[Step],
    stop_on_error: bool,
    exec: &mut F,
    mut sleep: S,
) -> SeqOutcome
where
    F: FnMut(&Action) -> StepOutcome,
    S: FnMut(u32),
{
    let mut out = SeqOutcome::default();
    for (i, step) in steps.iter().enumerate() {
        match step {
            Step::Delay { ms } => sleep(*ms),
            Step::Action { action } => match exec(action) {
                StepOutcome::Ok => out.ran += 1,
                StepOutcome::Fail(e) => {
                    out.errors.push((i, e));
                    if stop_on_error {
                        out.stopped = true;
                        break;
                    }
                }
                StepOutcome::Abort(e) => {
                    out.errors.push((i, e));
                    out.stopped = true;
                    break;
                }
            },
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::remote::buttons::RemoteButton;
    use actions::{ButtonConfig, KeyConfig, Shortcut};

    /// 只看用户宏。特殊宏（`sys:`）按计划常驻宏库（键位界面要见名知意），
    /// 会把下面这些「列表就该是这几条」的断言全污染掉，故统一先滤掉。
    fn user_list(dir: &Path, prof: &str) -> Vec<Macro> {
        list(dir, prof)
            .into_iter()
            .filter(|m| !special::is_special(&m.id))
            .collect()
    }

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lls-macros-{}-{}-{}",
            tag,
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn act(mods: u8, key: u16) -> Action {
        Action::Shortcut {
            shortcut: Shortcut {
                mods,
                key,
                ..Default::default()
            },
        }
    }

    fn macro_with(name: &str, steps: Vec<Step>) -> Macro {
        Macro {
            id: String::new(),
            name: name.to_string(),
            stop_on_error: true,
            steps,
            order: 0,
        }
    }

    /// 键位配置：某键某槽绑一条宏。
    fn cfg_with_macro(id: &str, button: RemoteButton, slot: Slot) -> ButtonConfig {
        let mut cfg = ButtonConfig::default();
        let mut kc = KeyConfig::default();
        kc.set_slot(slot, Some(Action::Macro { id: id.to_string() }));
        cfg.keys.insert(button.id().to_string(), kc);
        cfg
    }

    #[test]
    fn serde_shape_is_stable() {
        let m = macro_with(
            "截图",
            vec![
                Step::Action {
                    action: act(10, 23),
                },
                Step::Delay { ms: 500 },
            ],
        );
        let raw = serde_json::to_string(&m).unwrap();
        assert!(raw.contains(r#"{"type":"delay","ms":500}"#), "{raw}");
        assert!(raw.contains(r#""type":"action""#), "{raw}");
        let back: Macro = serde_json::from_str(&raw).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn normalize_trims_name_and_caps_delay() {
        // 计划 §三：延迟 0–5000ms 封顶——手滑多打一个 0 按上限截，不整条存不下去。
        let mut m = macro_with("  截图  ", vec![Step::Delay { ms: 99_999 }]);
        m.normalize().unwrap();
        assert_eq!(m.name, "截图");
        assert_eq!(m.steps, vec![Step::Delay { ms: MAX_DELAY_MS }]);
        // 空名 / 零步骤 / 宏套宏都是拒绝条件。
        assert!(macro_with("   ", vec![Step::Delay { ms: 1 }])
            .normalize()
            .is_err());
        assert!(macro_with("空", vec![]).normalize().is_err());
        let mut nested = macro_with(
            "套娃",
            vec![Step::Action {
                action: Action::Macro { id: "x".into() },
            }],
        );
        assert!(nested.normalize().is_err());
    }

    /// 片③：动作列表整组挪位置。旧库（没 order 字段）必须仍按名排，
    /// 排过一次之后必须按人排的序，且新建的追加到底、不再插进名字堆里。
    #[test]
    fn reorder_persists_and_keeps_legacy_name_order() {
        let dir = tmp("reorder");
        let prof = "xiaomi_rc00x";
        let one = || Step::Action { action: act(0, 8) };
        let a = upsert(&dir, prof, macro_with("乙", vec![one()])).unwrap();
        let b = upsert(&dir, prof, macro_with("甲", vec![one()])).unwrap();
        // 没排过序：按名（甲 < 乙 的 Unicode 序），与加字段前完全一致。
        let before: Vec<String> = user_list(&dir, prof).into_iter().map(|m| m.name).collect();
        assert_eq!(before.len(), 2);
        // 排一次：把 a 放到 b 后面。
        reorder(&dir, prof, &[b.id.clone(), a.id.clone()]).unwrap();
        let after: Vec<String> = user_list(&dir, prof).into_iter().map(|m| m.id).collect();
        assert_eq!(after, vec![b.id.clone(), a.id.clone()]);
        // 已排过序的库里新建 → 追加到底，不插回名字堆。
        let c = upsert(&dir, prof, macro_with("丙", vec![one()])).unwrap();
        let tail: Vec<String> = user_list(&dir, prof).into_iter().map(|m| m.id).collect();
        assert_eq!(tail, vec![b.id.clone(), a.id.clone(), c.id.clone()]);
        // 不存在的 id 必须报错，不能静默少一条（静默的表现就是「拖了没反应」）。
        assert!(reorder(&dir, prof, &["没有这条".into()]).is_err());
        assert!(reorder(&dir, prof, &[]).is_err());
    }

    #[test]
    fn upsert_generates_id_and_roundtrips_file() {
        let dir = tmp("upsert");
        let saved = upsert(
            &dir,
            "xiaomi_rc00x",
            macro_with(
                "截图",
                vec![Step::Action {
                    action: act(10, 23),
                }],
            ),
        )
        .unwrap();
        assert!(!saved.id.is_empty());
        assert_eq!(get(&dir, "xiaomi_rc00x", &saved.id).unwrap(), saved);
        // 同 id 再存 = 覆盖（改名走同一条路）。
        let mut renamed = saved.clone();
        renamed.name = "截图2".into();
        let again = upsert(&dir, "xiaomi_rc00x", renamed).unwrap();
        assert_eq!(again.id, saved.id);
        assert_eq!(user_list(&dir, "xiaomi_rc00x").len(), 1);
        assert_eq!(user_list(&dir, "xiaomi_rc00x")[0].name, "截图2");
        // 按档案分仓：别的档案看不到这条。
        assert!(user_list(&dir, "other").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_clears_dangling_references_everywhere() {
        // 计划 §三验收：删宏要跨全部方案清悬空引用（SayAll removeMacroReferences 口径）。
        let dir = tmp("dangling");
        let profile = "xiaomi_rc00x";
        let m = upsert(
            &dir,
            profile,
            macro_with(
                "截图",
                vec![Step::Action {
                    action: act(10, 23),
                }],
            ),
        )
        .unwrap();
        let other = upsert(
            &dir,
            profile,
            macro_with("别的", vec![Step::Delay { ms: 10 }]),
        )
        .unwrap();

        // 当前映射：Home 单击引用它 + Back 长按引用别的宏。
        let mut cfg = cfg_with_macro(&m.id, RemoteButton::Home, Slot::Click);
        cfg.set_slot_action(
            RemoteButton::Back,
            Slot::Long,
            Some(Action::Macro {
                id: other.id.clone(),
            }),
        );
        actions::save(&dir, profile, &cfg).unwrap();
        // 两个方案各引用一次（其中一个槽与另一个宏同键不同槽）。
        actions::save_scheme(&dir, profile, "工作", &cfg).unwrap();
        let scheme2 = cfg_with_macro(&m.id, RemoteButton::Power, Slot::Double);
        actions::save_scheme(&dir, profile, "居家", &scheme2).unwrap();

        let cleared = delete(&dir, profile, &m.id).unwrap();
        assert_eq!(cleared, 3, "当前映射 1 + 方案 2");
        assert!(get(&dir, profile, &m.id).is_none());
        // 当前映射里那条宏引用被摘掉，别的宏引用原样保留。
        let after =
            actions::parse(&std::fs::read_to_string(actions::config_path(&dir, profile)).unwrap());
        assert_eq!(after.slot_action(RemoteButton::Home, Slot::Click), None);
        assert_eq!(
            after.slot_action(RemoteButton::Back, Slot::Long),
            Some(&Action::Macro {
                id: other.id.clone()
            })
        );
        let schemes = actions::load_schemes(&dir, profile);
        assert_eq!(
            schemes.schemes["工作"].slot_action(RemoteButton::Ok, Slot::Click),
            None
        );
        assert_eq!(
            schemes.schemes["工作"].slot_action(RemoteButton::Back, Slot::Long),
            Some(&Action::Macro { id: other.id })
        );
        assert_eq!(
            schemes.schemes["居家"].slot_action(RemoteButton::Power, Slot::Double),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_missing_macro_errors_and_keeps_file() {
        let dir = tmp("missing");
        assert!(delete(&dir, "xiaomi_rc00x", "nope").is_err());
        assert!(user_list(&dir, "xiaomi_rc00x").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 记录调用轨迹的假执行器：按预设结果表逐步回，顺带钉住「第几步没跑」。
    struct FakeExec {
        results: Vec<StepOutcome>,
        calls: Vec<usize>,
        nth: std::cell::Cell<usize>,
    }

    impl FakeExec {
        fn new(results: Vec<StepOutcome>) -> Self {
            Self {
                results,
                calls: Vec::new(),
                nth: std::cell::Cell::new(0),
            }
        }
        fn call(&mut self, _a: &Action) -> StepOutcome {
            let i = self.nth.get();
            self.nth.set(i + 1);
            self.calls.push(i);
            self.results.get(i).cloned().unwrap_or(StepOutcome::Ok)
        }
    }

    fn steps(n: usize) -> Vec<Step> {
        (0..n).map(|_| Step::Action { action: act(0, 5) }).collect()
    }

    #[test]
    fn sequence_stops_at_first_error_when_asked() {
        // 计划 §三验收：前步失败按 stop_on_error 决定中断。
        let s = steps(4);
        let mut ex = FakeExec::new(vec![
            StepOutcome::Ok,
            StepOutcome::Fail("注入失败".into()),
            StepOutcome::Ok,
            StepOutcome::Ok,
        ]);
        let out = run_sequence(&s, true, &mut |a| ex.call(a));
        assert_eq!(ex.calls, vec![0, 1], "遇错即停：第 3、4 步不该跑");
        assert_eq!(out.ran, 1);
        assert!(out.stopped);
        assert_eq!(out.errors, vec![(1, "注入失败".to_string())]);
    }

    #[test]
    fn sequence_continues_and_collects_every_error() {
        let s = steps(3);
        let mut ex = FakeExec::new(vec![
            StepOutcome::Ok,
            StepOutcome::Fail("第 2 步错".into()),
            StepOutcome::Fail("第 3 步错".into()),
        ]);
        let out = run_sequence(&s, false, &mut |a| ex.call(a));
        assert_eq!(ex.calls, vec![0, 1, 2], "遇错继续：三步都该跑");
        assert_eq!(out.ran, 1);
        assert!(!out.stopped);
        assert_eq!(out.errors.len(), 2);
    }

    #[test]
    fn abort_stops_regardless_of_stop_on_error() {
        // 录制开始/通道关闭这类上下文变化：不管用户怎么设，整条立刻别跑。
        let s = steps(3);
        let mut ex = FakeExec::new(vec![
            StepOutcome::Ok,
            StepOutcome::Abort("录制中，序列中断".into()),
            StepOutcome::Ok,
        ]);
        let out = run_sequence(&s, false, &mut |a| ex.call(a));
        assert_eq!(ex.calls, vec![0, 1]);
        assert!(out.stopped);
    }

    #[test]
    fn delays_are_handed_to_the_sleeper_in_order() {
        // 延迟步骤不占动作计数，但必须按序真等（这里只验交给 sleep 的值）。
        let s = vec![
            Step::Delay { ms: 120 },
            Step::Action { action: act(0, 5) },
            Step::Delay { ms: MAX_DELAY_MS },
        ];
        let mut slept = Vec::new();
        let mut ex = FakeExec::new(vec![StepOutcome::Ok]);
        let out = run_sequence_with(&s, true, &mut |a| ex.call(a), |ms| slept.push(ms));
        assert_eq!(slept, vec![120, MAX_DELAY_MS]);
        assert_eq!((out.ran, out.stopped), (1, false));
        assert!(out.errors.is_empty());
    }
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;

    fn m(id: &str, name: &str, order: u32) -> Macro {
        Macro {
            id: id.into(),
            name: name.into(),
            stop_on_error: false,
            steps: vec![],
            order,
        }
    }

    /// 计划 2026-11-26「备注字段与去重」：宏库保存按归一化名去重，保留列表顺序里的第一条。
    #[test]
    fn 同名宏保存时只留第一条_按列表顺序判() {
        let mut file = MacroFile::default();
        file.macros.insert("a".into(), m("a", "发送", 1));
        file.macros.insert("b".into(), m("b", "发送", 0)); // order 更小 → 用户看到的更前面
        file.macros.insert("c".into(), m("c", "缩放", 2));
        let out = dedupe_by_name(&file);
        assert_eq!(out.macros.len(), 2, "同名只留一条");
        assert!(
            out.macros.contains_key("b"),
            "保留的是列表第一条（order 小的 b）"
        );
        assert!(!out.macros.contains_key("a"));
        assert!(out.macros.contains_key("c"), "不同名不动");
    }

    /// 归一口径与快速面板同源：首尾空白与零宽字符不算差异，内容本身不改。
    #[test]
    fn 首尾空白与零宽字符算同名() {
        let mut file = MacroFile::default();
        file.macros.insert("x".into(), m("x", "  发送 ", 0));
        file.macros.insert("y".into(), m("y", "发送\u{200b}", 1));
        file.macros.insert("z".into(), m("z", "发 送", 2));
        let out = dedupe_by_name(&file);
        assert_eq!(
            out.macros.len(),
            2,
            "空白/零宽差异视为同名，中间空格是内容不算同名"
        );
        assert_eq!(
            out.macros.get("x").unwrap().name,
            "  发送 ",
            "原文一个字段都不改"
        );
    }
}

/// 面板态专属「特殊宏」（计划 2026-11-26 §三.2）。
///
/// 两张映射表统一用**动作（宏引用）**映射，键位界面才见名知意（显示「切换应用 (⌘Tab)」
/// 而不是一串裸快捷键）。面板态的新功能因此做成宏库里 **id 固定**的宏：
/// 执行时由面板侧按这些 id 拦下来路由到专属逻辑（`promptfloat::run_special`），
/// 不当按键序列跑。步骤留空是故意的——它们不是按键序列，`normalize` 对特殊宏放行空步骤。
pub mod special {
    use super::*;

    /// id 前缀：见到就知道是系统专属宏，不是用户录的。
    pub const PREFIX: &str = "sys:";
    /// 菜单键：调系统切换器（mac ⌘Tab / Win Alt+Tab）切前台 = 选目标方式之一。
    pub const SWITCH_APP: &str = "sys:switch-app";
    /// Home 键：在「运行中且可注入」列表里循环换目标，**不切前台**（选目标方式之二）。
    pub const CYCLE_TARGET: &str = "sys:cycle-target";
    /// TV 键：显示/隐藏快速面板。
    pub const TOGGLE_PANEL: &str = "sys:toggle-panel";
    /// 电源键：把常用表复制到面板表，只补面板表未映射的键。
    pub const COPY_COMMON_MAP: &str = "sys:copy-common-map";
    /// 音量 +：面板缩放 +1 档（1 → 1.5 → 2 循环）。
    pub const ZOOM_UP: &str = "sys:zoom-up";
    /// 音量 −：面板缩放 −1 档。
    pub const ZOOM_DOWN: &str = "sys:zoom-down";
    /// back 键单击：向前删一个字符（语音编辑态用）。
    pub const BACKSPACE: &str = "sys:backspace";
    /// 清空语音回显编辑框（计划 §语音编辑态：Esc 保留内容，长按返回键给个清空入口）。
    pub const CLEAR_VOICE: &str = "sys:clear-voice";

    fn m(id: &str, name: &str) -> Macro {
        Macro {
            id: id.into(),
            name: name.into(),
            stop_on_error: false,
            steps: vec![],
            order: 0,
        }
    }

    /// 全部特殊宏。宏库加载时补齐（`load` 里 or_insert），键位界面因此能显示功能名。
    pub fn all() -> Vec<Macro> {
        vec![
            m(SWITCH_APP, "切换应用 (⌘Tab)"),
            m(CYCLE_TARGET, "循环切目标"),
            m(TOGGLE_PANEL, "显示/隐藏面板"),
            m(COPY_COMMON_MAP, "复制常用映射表"),
            m(ZOOM_UP, "面板放大一档"),
            m(ZOOM_DOWN, "面板缩小一档"),
            m(BACKSPACE, "删一个字符"),
            m(CLEAR_VOICE, "清空语音编辑框"),
        ]
    }

    pub fn is_special(id: &str) -> bool {
        id.starts_with(PREFIX)
    }
}

#[cfg(test)]
mod special_tests {
    use super::*;

    /// 特殊宏按计划常驻宏库（键位界面要见名知意），且空步骤是合法的——它们不当按键序列跑。
    #[test]
    fn 特殊宏自动补齐且空步骤过校验() {
        let dir = std::env::temp_dir().join(format!("lls-special-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let ids: Vec<String> = list(&dir, "xiaomi_rc00x")
            .into_iter()
            .filter(|m| special::is_special(&m.id))
            .map(|m| m.id)
            .collect();
        assert_eq!(ids.len(), special::all().len(), "特殊宏一条不少");
        for m in special::all() {
            let mut m = m;
            assert!(m.normalize().is_ok(), "特殊宏空步骤必须过校验：{}", m.name);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 用户改过名字就不再塞回去——不跟用户较劲。
    #[test]
    fn 用户改过名的特殊宏不被覆盖() {
        let dir = std::env::temp_dir().join(format!("lls-special2-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut mine = special::all().remove(0);
        mine.name = "我叫切换应用".into();
        let mut file = MacroFile::default();
        file.macros.insert(mine.id.clone(), mine);
        write(&dir, "xiaomi_rc00x", &file).unwrap();
        let got = get(&dir, "xiaomi_rc00x", special::SWITCH_APP).unwrap();
        assert_eq!(got.name, "我叫切换应用");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
