//! 预设配置一键还原（preset）：把存档目录里的各家代理配置
//! 复制回各代理的真实配置文件路径。
//!
//! 与接管备份（takeover backup）的分工：接管备份管的是「连接键」这一层，
//! 预设还原管的是「整套模型搭配」这一层（档位、视觉标志、上下文窗口等）。
//! 预设目录里放什么由 `manifest.json` 决定，代码不猜路径。
//!
//! 两个目录两个身份（别混）：`~/.llm-switch/agent-configs` 是**运行时**存档，
//! 跟着装好的应用走；`~/myllm/agent-configs` 是**开发期**存档，跟着源码走 git。
//! 运行时那份有清单就用运行时那份，没有才回落到开发期那份——
//! 这样仓库里的存档在开发时照样能一键还原，装好的应用又不会去翻源码树。
//!
//! 安全边界（与接管同一套铁律）：
//! - `source` 必须落在预设目录内（清单被改也不能指到别处读文件）；
//! - `target` 必须在家目录内（家目录外一律拒绝）；
//! - 覆盖前把现行文件快照到 `<预设目录>/.restore-backups/<毫秒>/`，误还原可人工找回。

use super::TakeoverEnv;
use crate::types::now_ms;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 运行时预设目录：数据目录（`~/.llm-switch`）下的一份，跟着装机走。
pub const PRESET_DIR: &str = "~/.llm-switch/agent-configs";
/// 开发期预设目录：随源码走 git 的那份存档。运行时那份没有清单时才回落到这里。
pub const PRESET_DIR_DEV: &str = "~/myllm/agent-configs";
/// 预设清单文件名。
const MANIFEST: &str = "manifest.json";

/// 一条还原映射：某代理的一个配置文件。
pub struct Entry {
    pub app: String,
    pub target: PathBuf,
    pub source: PathBuf,
}

/// 把 `~/…` 展开成家目录下的绝对路径；绝对路径原样返回。
fn expand(home: &Path, raw: &str) -> PathBuf {
    let raw = raw.trim();
    if raw == "~" {
        home.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    }
}

/// 预设目录的绝对路径：运行时优先，缺清单才认开发期那份。
///
/// 认「有没有 manifest.json」而不是认目录存不存在：空目录（刚建好还没放东西）
/// 不该把开发期存档抢过来，否则用户以为在编辑运行时那份、其实写进了源码树。
pub fn preset_dir(env: &TakeoverEnv) -> PathBuf {
    let run = expand(&env.home, PRESET_DIR);
    if run.join(MANIFEST).is_file() {
        return run;
    }
    let dev = expand(&env.home, PRESET_DIR_DEV);
    if dev.join(MANIFEST).is_file() {
        return dev;
    }
    run
}

/// 读清单并做边界校验。清单缺条目 / 路径越界都在这里拦下。
fn load_entries(env: &TakeoverEnv) -> Result<Vec<Entry>, String> {
    let dir = preset_dir(env);
    let raw = std::fs::read(dir.join(MANIFEST))
        .map_err(|e| format!("读取预设清单失败 {}：{e}", dir.join(MANIFEST).display()))?;
    let doc: Value =
        serde_json::from_slice(&raw).map_err(|e| format!("预设清单不是合法 JSON：{e}"))?;
    let items = doc
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "预设清单缺少 entries 数组".to_string())?;
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let app = item
            .get("app")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("entries[{i}] 缺 app"))?
            .to_string();
        let target = item
            .get("target")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("entries[{i}] 缺 target"))?;
        let source = item
            .get("source")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("entries[{i}] 缺 source"))?;
        let target = expand(&env.home, target);
        let source = expand(&env.home, source);
        // 预设目录有两份（运行时 / 开发期归档），整份拷贝是最常见的迁移方式，
        // 但清单里的 source 写死了来源目录，一拷就「越出预设目录」。把那段前缀
        // 换成当前生效目录，两份目录就能互为模板。
        let source = match [PRESET_DIR, PRESET_DIR_DEV]
            .iter()
            .map(|p| expand(&env.home, p))
            .find(|other| *other != dir && source.starts_with(other))
        {
            Some(other) => dir.join(source.strip_prefix(&other).unwrap()),
            None => source,
        };
        // source 必须在预设目录内：清单被恶意/误改也读不到目录外的文件。
        if !source.starts_with(&dir) {
            return Err(format!(
                "entries[{i}] 的 source 越出预设目录，已拒绝: {}",
                source.display()
            ));
        }
        // target 必须在家目录内：与接管备份还原同一条边界。
        if !super::path_under_home(env, &target) {
            return Err(format!(
                "entries[{i}] 的 target 越出家目录，已拒绝: {}",
                target.display()
            ));
        }
        out.push(Entry {
            app,
            target,
            source,
        });
    }
    if out.is_empty() {
        return Err("预设清单是空的".to_string());
    }
    Ok(out)
}

/// 预设目录与每条映射的现状：给前端列表用。
pub fn status(env: &TakeoverEnv) -> Result<Value, String> {
    let dir = preset_dir(env);
    let entries = load_entries(env)?;
    let items: Vec<Value> = entries
        .iter()
        .map(|e| {
            let src = std::fs::read(&e.source);
            let tgt = std::fs::read(&e.target);
            let differs = match (&src, &tgt) {
                (Ok(s), Ok(t)) => s != t,
                _ => true,
            };
            json!({
                "app": e.app,
                "target": e.target.display().to_string(),
                "source": e.source.display().to_string(),
                "source_exists": src.is_ok(),
                "target_exists": tgt.is_ok(),
                "differs": differs,
            })
        })
        .collect();
    Ok(json!({ "dir": dir.display().to_string(), "entries": items }))
}

/// 一键还原：`app` 为 None 时全量，否则只还原该代理的条目。
/// 覆盖前把现行文件快照到 `<预设目录>/.restore-backups/<毫秒>/`。
pub fn restore(env: &TakeoverEnv, app: Option<&str>) -> Result<Value, String> {
    let dir = preset_dir(env);
    let entries = load_entries(env)?;
    let selected: Vec<&Entry> = match app {
        Some(a) => entries.iter().filter(|e| e.app == a).collect(),
        None => entries.iter().collect(),
    };
    if selected.is_empty() {
        return Err(format!("预设清单里没有代理为「{app:?}」的条目"));
    }
    let stamp = now_ms();
    let snapshot = dir.join(".restore-backups").join(stamp.to_string());
    let mut restored = Vec::new();
    let mut skipped = Vec::new();
    for e in &selected {
        let bytes = match std::fs::read(&e.source) {
            Ok(b) => b,
            // 源缺失 ≠ 全量失败：跳过并如实上报，让用户先补存档。
            Err(_) => {
                skipped.push(
                    json!({ "app": e.app, "target": e.target.display().to_string(),
                    "reason": "预设文件不存在" }),
                );
                continue;
            }
        };
        // 快照现行文件（存在才有得快照）。
        if let Ok(cur) = std::fs::read(&e.target) {
            let slot = snapshot.join(&e.app);
            std::fs::create_dir_all(&slot).map_err(|e| format!("创建快照目录失败: {e}"))?;
            let name = e
                .target
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".into());
            std::fs::write(slot.join(name), cur).map_err(|e| format!("写入快照失败: {e}"))?;
        }
        if let Some(parent) = e.target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目标目录失败 {}: {e}", parent.display()))?;
        }
        std::fs::write(&e.target, bytes)
            .map_err(|err| format!("写入 {} 失败: {err}", e.target.display()))?;
        restored.push(json!({ "app": e.app, "target": e.target.display().to_string() }));
    }
    Ok(json!({
        "restored": restored,
        "skipped": skipped,
        "snapshot": if snapshot.exists() { snapshot.display().to_string() } else { String::new() },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn env_with_preset(tag: &str) -> (TakeoverEnv, PathBuf) {
        let home = std::env::temp_dir().join(format!("ls-preset-{tag}-{}", now_ms()));
        let dir = home.join("myllm/agent-configs");
        fs::create_dir_all(dir.join("demo")).unwrap();
        fs::write(dir.join("demo/settings.json"), b"{\"v\":2}").unwrap();
        fs::write(
            dir.join(MANIFEST),
            json!({"entries":[{"app":"demo","target":"~/demoapp/settings.json","source":"~/myllm/agent-configs/demo/settings.json"}]})
                .to_string(),
        )
        .unwrap();
        (
            TakeoverEnv::with_roots(home.clone(), home.join("backups")),
            home,
        )
    }

    /// 运行时目录有清单就以它为准，没有才回落开发期存档（两份目录两个身份）。
    #[test]
    fn runtime_dir_wins_over_dev_archive() {
        let home = std::env::temp_dir().join(format!("ls-preset-prio-{}", now_ms()));
        let dev = home.join("myllm/agent-configs");
        let run = home.join(".llm-switch/agent-configs");
        fs::create_dir_all(&dev).unwrap();
        fs::write(dev.join(MANIFEST), b"{\"entries\":[]}").unwrap();
        let env = TakeoverEnv::with_roots(home.clone(), home.join("backups"));
        assert_eq!(preset_dir(&env), dev, "运行时没清单 → 回落开发期存档");
        // 空目录不算数：别把「刚建好还没放东西」当成运行时存档已就位。
        fs::create_dir_all(&run).unwrap();
        assert_eq!(preset_dir(&env), dev, "运行时是空目录 → 仍认开发期");
        fs::write(run.join(MANIFEST), b"{\"entries\":[]}").unwrap();
        assert_eq!(preset_dir(&env), run, "运行时有了清单 → 以运行时为准");
    }

    /// 清单从开发期归档整份拷进运行时目录：source 里那段前缀跟着换，别报「越出预设目录」。
    #[test]
    fn copied_manifest_remaps_source_into_active_dir() {
        let home = std::env::temp_dir().join(format!("ls-preset-copy-{}", now_ms()));
        let run = home.join(".llm-switch/agent-configs");
        fs::create_dir_all(run.join("demo")).unwrap();
        fs::write(run.join("demo/settings.json"), b"{\"v\":9}").unwrap();
        fs::write(
            run.join(MANIFEST),
            json!({"entries":[{"app":"demo","target":"~/demoapp/settings.json","source":"~/myllm/agent-configs/demo/settings.json"}]})
                .to_string(),
        )
        .unwrap();
        let env = TakeoverEnv::with_roots(home.clone(), home.join("backups"));
        let entries = load_entries(&env).unwrap();
        assert_eq!(entries[0].source, run.join("demo/settings.json"));
        fs::create_dir_all(home.join("demoapp")).unwrap();
        let r = restore(&env, None).unwrap();
        assert_eq!(r["restored"].as_array().unwrap().len(), 1);
        assert_eq!(
            fs::read(home.join("demoapp/settings.json")).unwrap(),
            b"{\"v\":9}"
        );
    }

    #[test]
    fn restore_writes_target_and_snapshots_old() {
        let (env, home) = env_with_preset("restore");
        fs::create_dir_all(home.join("demoapp")).unwrap();
        fs::write(home.join("demoapp/settings.json"), b"{\"v\":1}").unwrap();
        let r = restore(&env, None).unwrap();
        assert_eq!(r["restored"].as_array().unwrap().len(), 1);
        assert_eq!(
            fs::read(home.join("demoapp/settings.json")).unwrap(),
            b"{\"v\":2}"
        );
        // 旧内容进了快照
        let snap = r["snapshot"].as_str().unwrap();
        assert!(Path::new(snap).exists());
        assert_eq!(
            fs::read(Path::new(snap).join("demo/settings.json")).unwrap(),
            b"{\"v\":1}"
        );
        fs::remove_dir_all(home).ok();
    }

    #[test]
    fn rejects_source_outside_preset_dir() {
        let (env, home) = env_with_preset("src_out");
        fs::write(
            home.join("myllm/agent-configs/manifest.json"),
            json!({"entries":[{"app":"demo","target":"~/x.json","source":"~/elsewhere.json"}]})
                .to_string(),
        )
        .unwrap();
        assert!(status(&env).is_err());
        fs::remove_dir_all(home).ok();
    }

    #[test]
    fn rejects_target_outside_home() {
        let (env, home) = env_with_preset("tgt_out");
        fs::write(
            home.join("myllm/agent-configs/manifest.json"),
            json!({"entries":[{"app":"demo","target":"/tmp/outside.json","source":"~/myllm/agent-configs/demo/settings.json"}]})
                .to_string(),
        )
        .unwrap();
        assert!(status(&env).is_err());
        fs::remove_dir_all(home).ok();
    }
}
