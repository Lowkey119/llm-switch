//! 接管页的配置文件浏览与编辑（M12-D）。
//!
//! 接管不管的字段（claude 的 tier 映射、pi 的模型元信息、opencode 的
//! `setCacheKey`——都被 takeover 整块重写抹过至少一次）。
//!
//! 三道安全闸，缺一不可：
//! 1. **路径白名单**：文件一律来自 `evaluate_one` 给出的接管已知路径，
//!    接口不接受任意路径入参，杜绝把本工具变成任意文件读写器。
//! 2. **写前格式校验**：JSON/TOML 必须先能被解析，坏内容不落盘。
//! 3. **写前快照 + 并发拦截**：改动前先备份（与接管同一套备份目录），备份失败
//!    就拒绝写入（没有回滚网就不做覆盖动作），并按 `base_sha` 比对磁盘现状，
//!    别人（或另一个窗口）改过就拒绝覆盖；写入本身走原子替换，不会留半截文件。
//!
//! 密钥默认打码：打码只发生在读取展示时；要打码状态下保存会被拦下来，
//! 免得把 `***` 当真值写回配置。

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::Selection;
use crate::gateway::State;

use super::{evaluate_one, write_backup, ClientApp, Endpoint, TakeoverEnv};

/// 打码占位符。出现在提交内容里就说明用户保存的是打码视图。
pub const MASK: &str = "***";

/// 文件摘要（列表用）。
#[derive(Debug, serde::Serialize)]
pub struct FileBrief {
    /// 编辑时引用它，而不是路径——路径永远不由前端提供。
    pub index: usize,
    pub path: String,
    pub home: String,
    pub format: &'static str,
    pub exists: bool,
    pub size: u64,
    pub mtime_ms: u64,
    /// 内容指纹（不存在的文件按空串计），写回时用作并发基线。
    pub sha: String,
}

/// 文件正文。
#[derive(Debug, serde::Serialize)]
pub struct FileDoc {
    pub index: usize,
    pub path: String,
    pub format: &'static str,
    pub exists: bool,
    pub content: String,
    pub sha: String,
    /// 本次返回是否打了码（打码态不允许保存）。
    pub masked: bool,
}

fn fingerprint(bytes: &[u8]) -> String {
    let mut h = Sha256::digest(bytes);
    // 十六进制取前 16 位足够做并发基线，也方便人眼比对
    let mut out = String::new();
    for b in h.iter_mut().take(8) {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// 接管认识的该客户端配置文件（白名单的唯一来源）。
fn owned_paths(
    env: &TakeoverEnv,
    ep: &Endpoint,
    app: ClientApp,
) -> Result<Vec<(PathBuf, &'static str, bool)>, String> {
    let eval = evaluate_one(env, ep, app, &Selection::default())?;
    Ok(eval
        .files
        .into_iter()
        .map(|f| {
            let exists = f.exists;
            let fmt = match f.format {
                "json" => "json",
                "toml" => "toml",
                _ => "text",
            };
            (PathBuf::from(&f.path), fmt, exists)
        })
        .collect())
}

fn env_of(st: &State) -> TakeoverEnv {
    TakeoverEnv::from_config(&st.config, &st.store)
}

fn endpoint_of(st: &State) -> Endpoint {
    Endpoint::build(&st.config, &st.store)
}

fn stat_brief(index: usize, path: &PathBuf, format: &'static str) -> FileBrief {
    let meta = std::fs::metadata(path).ok();
    let bytes = std::fs::read(path).unwrap_or_default();
    let mtime_ms = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    FileBrief {
        index,
        home: dirs_home_relative(path),
        path: path.display().to_string(),
        format,
        exists: meta.is_some(),
        size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
        mtime_ms,
        sha: fingerprint(&bytes),
    }
}

/// 把家目录折成 `~`，界面上少一串绝对路径。
fn dirs_home_relative(path: &Path) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let s = path.display().to_string();
    if !home.is_empty() && s.starts_with(&home) {
        format!("~{}", &s[home.len()..])
    } else {
        s
    }
}

pub fn list(st: &State, app: ClientApp) -> Result<Vec<FileBrief>, String> {
    let env = env_of(st);
    let ep = endpoint_of(st);
    let paths = owned_paths(&env, &ep, app)?;
    Ok(paths
        .into_iter()
        .enumerate()
        .map(|(i, (p, fmt, _))| stat_brief(i, &p, fmt))
        .collect())
}

/// 看起来像密钥的键名（打码用，宽松一点没关系，多打总比漏打好）。
fn looks_secret(key: &str) -> bool {
    let k = key.to_lowercase();
    // 有些键名带 token/key 却根本不是密钥（`CLAUDE_CODE_MAX_CONTEXT_TOKENS`
    // 就是上下文长度）。误打码会让人怀疑整页的可信度，宁可放过这类。
    if [
        "max_",
        "count",
        "limit",
        "ttl",
        "timeout",
        "version",
        "tokenizer",
        "threshold",
    ]
    .iter()
    .any(|noise| k.contains(noise))
    {
        return false;
    }
    k.contains("token")
        || k.contains("secret")
        || k.contains("password")
        || k.contains("passwd")
        || k.contains("apikey")
        || k.contains("api_key")
        || k.contains("access_key")
        || k.ends_with("_key")
        || k.starts_with("key_")
}

fn mask_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if looks_secret(k) {
                    // 只打码字符串：数字/布尔一律不是秘密（数字型的 *_TOKENS 常被误伤）
                    if let serde_json::Value::String(text) = v {
                        if !text.is_empty() {
                            *v = serde_json::Value::String(MASK.to_string());
                        }
                    }
                } else {
                    mask_json(v);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(mask_json),
        _ => {}
    }
}

/// TOML 打码：解析成文档后遍历，命中的键换成 `***`。
///
/// 与 `mask_json` 同一条口径，三条铁律：
/// 1. **只打码字符串**——数字/布尔不是秘密（数字型的 `*_TOKENS` 常被误伤），
///    而且把数字改成字符串会改掉原文的类型；
/// 2. **只换值不动引号**——`Formatted::new("\"***\"")` 会把引号当内容写进去，
///    序列化出来是 `"\"***\""`（曾经的 bug）；
/// 3. **密钥键向下传导**——`[secrets]` 或 `auth = { … }` 这种整体敏感的子树，
///    里面每层都按敏感处理；inline 表与数组一律递归（老实现不递归，嵌在
///    inline 表里的 `api_key` 是明文显示的）。
fn mask_toml(content: &str) -> String {
    let Ok(doc) = content.parse::<toml_edit::DocumentMut>() else {
        return content.to_string();
    };
    let mut doc = doc;
    walk_table(doc.as_table_mut(), false);
    doc.to_string()
}

fn walk_table(table: &mut toml_edit::Table, parent_secret: bool) {
    for (key, item) in table.iter_mut() {
        let secret = parent_secret || looks_secret(key.get());
        visit_item(item, secret);
    }
}

fn visit_item(item: &mut toml_edit::Item, secret: bool) {
    match item {
        toml_edit::Item::Table(t) => walk_table(t, secret),
        toml_edit::Item::ArrayOfTables(tabs) => {
            for t in tabs.iter_mut() {
                walk_table(t, secret);
            }
        }
        toml_edit::Item::Value(v) => visit_value(v, secret),
        toml_edit::Item::None => {}
    }
}

fn visit_value(value: &mut toml_edit::Value, secret: bool) {
    // `Formatted` 没有公开的 `value_mut`，所以命中时整个换掉值对象——
    // 这也正是引号 bug 的根因所在：换 value 才不会把引号写进内容里。
    let mask_it = matches!(
        value,
        toml_edit::Value::String(s) if secret && !s.value().is_empty()
    );
    if mask_it {
        *value = toml_edit::Value::String(toml_edit::Formatted::new(MASK.to_string()));
        return;
    }
    match value {
        toml_edit::Value::InlineTable(table) => {
            // 先把键名取出来：下面要按键可变访问，边迭代边借会撞可变借用。
            let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
            for k in keys {
                let child_secret = secret || looks_secret(&k);
                if let Some(sub) = table.get_mut(k.as_str()) {
                    visit_value(sub, child_secret);
                }
            }
        }
        toml_edit::Value::Array(items) => {
            for item in items.iter_mut() {
                visit_value(item, secret);
            }
        }
        _ => {}
    }
}

pub fn read(st: &State, app: ClientApp, index: usize, reveal: bool) -> Result<FileDoc, String> {
    let env = env_of(st);
    let ep = endpoint_of(st);
    let paths = owned_paths(&env, &ep, app)?;
    let (path, format, _) = paths
        .into_iter()
        .nth(index)
        .ok_or_else(|| format!("第 {index} 个文件不在 {app:?} 的接管清单里", app = app))?;
    let raw = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(format!("读取 {} 失败: {e}", path.display())),
    };
    let sha = fingerprint(&raw);
    let text = String::from_utf8_lossy(&raw).to_string();
    let (content, masked) = if reveal {
        (text.clone(), false)
    } else {
        let shown = match format {
            "json" => match serde_json::from_str::<serde_json::Value>(&text) {
                Ok(mut v) => {
                    mask_json(&mut v);
                    serde_json::to_string_pretty(&v).unwrap_or_else(|_| text.clone())
                }
                // 解析不了的 JSON（半成品文件）原样给出，不硬造一份“合法”的
                Err(_) => text.clone(),
            },
            "toml" => mask_toml(&text),
            _ => text.clone(),
        };
        let changed = shown != text;
        (shown, changed)
    };
    Ok(FileDoc {
        index,
        path: path.display().to_string(),
        format,
        exists: !raw.is_empty(),
        content,
        sha,
        masked,
    })
}

/// 格式校验：坏内容一律不落盘。返回人话错误。
fn validate(format: &str, content: &str) -> Result<(), String> {
    match format {
        "json" => serde_json::from_str::<serde_json::Value>(content)
            .map(|_| ())
            .map_err(|e| format!("JSON 解析失败：{e}")),
        "toml" => content
            .parse::<toml_edit::DocumentMut>()
            .map(|_| ())
            .map_err(|e| format!("TOML 解析失败：{e}")),
        _ => Ok(()),
    }
}

/// 写回：校验 → 并发拦截 → 快照 → 落盘。
pub fn write(
    st: &State,
    app: ClientApp,
    index: usize,
    content: &str,
    base_sha: &str,
) -> Result<serde_json::Value, String> {
    let env = env_of(st);
    let ep = endpoint_of(st);
    let paths = owned_paths(&env, &ep, app)?;
    let (path, format, _) = paths
        .into_iter()
        .nth(index)
        .ok_or_else(|| "该文件不在接管清单里".to_string())?;

    if content.contains(MASK) {
        return Err("内容里还有打码占位符 ***：请点「显示密钥」拿到真实内容后再保存".to_string());
    }
    validate(format, content)?;

    let disk = std::fs::read(&path).unwrap_or_default();
    if fingerprint(&disk) != base_sha {
        return Err(
            "文件在你打开之后被改过（可能是接管、客户端或另一个窗口）。已拒绝覆盖，请重新读取。"
                .to_string(),
        );
    }

    // 与接管同一套备份：手改崩了也能从备份列表里还原。备份失败就**不写**——
    // 没有回滚网的覆盖动作不该发生（这里原本 `.ok()` 吞掉错误照写不误）。
    let backup_id = write_backup(&env, app, std::slice::from_ref(&path), "manual-edit")
        .map_err(|e| format!("写前备份失败，已放弃改写（文件保持原样）：{e}"))?;
    // 原子写（临时文件 + rename，权限收到 0600）：进程被杀或磁盘写一半时
    // 不会留半截配置文件——裸 `fs::write` 是先截断再写，中途失败就毁了原文件。
    super::atomic_write_private(&path, content)?;

    let after = std::fs::read(&path).unwrap_or_default();
    st.log.log(crate::gateway::log::Event {
        level: crate::gateway::log::Level::Info,
        code: "TK003 takeover.file_edited",
        message: "配置文件由用户在接管页手工改写",
        request_id: None,
        backend: None,
        model: None,
        extra: serde_json::json!({
            "app": app.as_str(),
            "path": path.display().to_string(),
            "format": format,
            "bytes": after.len(),
            "backup_id": backup_id,
        }),
    });
    Ok(serde_json::json!({
        "sha": fingerprint(&after),
        "bytes": after.len(),
        "backup_id": backup_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_rejects_broken_formats() {
        assert!(validate("json", "{\"a\":1}").is_ok());
        assert!(validate("json", "{\"a\":}").is_err());
        assert!(validate("toml", "a = 1\n").is_ok());
        assert!(validate("toml", "a = \n").is_err());
        // 文本格式不设校验，接管之外的杂项文件不该被格式规则挡住
        assert!(validate("text", "anything at all").is_ok());
    }

    #[test]
    fn json_masking_hits_secret_looking_keys_only() {
        let mut v: serde_json::Value = serde_json::from_str(
            r#"{"ANTHROPIC_AUTH_TOKEN":"sk-real","model":"x","nested":{"api_key":"k","name":"n"}}"#,
        )
        .unwrap();
        mask_json(&mut v);
        assert_eq!(v["ANTHROPIC_AUTH_TOKEN"], MASK);
        assert_eq!(v["model"], "x");
        assert_eq!(v["nested"]["api_key"], MASK);
        assert_eq!(v["nested"]["name"], "n");
    }

    #[test]
    fn toml_masking_keeps_structure() {
        let src = "[model_providers.llm-switch]\napi_key = \"real\"\nbase_url = \"http://x\"\n";
        let out = mask_toml(src);
        // 逐字节断言：换值不换结构、注释与其余键原样，且打出来的是 `"***"`
        // 而不是把引号当内容的 `"\"***\""`（老写法
        // `Formatted::new(format!("\"{MASK}\""))` 的坑）。
        assert_eq!(
            out,
            "[model_providers.llm-switch]\napi_key = \"***\"\nbase_url = \"http://x\"\n"
        );
        assert!(!out.contains("real"), "{out}");
        assert!(!out.contains('\\'), "不该出现转义引号: {out}");
    }

    /// 老实现对 inline 表既不递归（嵌在里面的 api_key 是明文），命中时还会把
    /// 键名往表里插一个重复字段。这两条一起钉住。
    #[test]
    fn toml_masking_reaches_inline_tables_and_arrays() {
        let src = concat!(
            "provider = { name = \"p\", auth = { api_key = \"sk-real\", user = \"u\" } }\n",
            "list = [{ token = \"tk-real\", role = \"r\" }]\n"
        );
        let out = mask_toml(src);
        assert!(!out.contains("sk-real"), "inline 表里的密钥要被打码: {out}");
        assert!(!out.contains("tk-real"), "数组元素里的密钥要被打码: {out}");
        assert!(out.contains("name = \"p\""), "{out}");
        assert!(out.contains("user = \"u\""), "{out}");
        assert!(out.contains("role = \"r\""), "{out}");
        assert!(!out.contains("auth = { auth"), "不该往表里插重复键: {out}");
    }

    /// 整体敏感的子树：键名命中后向下传导，里面每层字符串都打码。
    #[test]
    fn toml_masking_propagates_secret_key_into_subtree() {
        let out = mask_toml("[secrets]\nuser = \"alice\"\nnote = \"keep\"\n");
        assert!(out.contains("user = \"***\""), "{out}");
        assert!(out.contains("note = \"***\""), "{out}");
    }

    /// 数字与布尔不是秘密，改成字符串还会动到原文类型（与 mask_json 同口径）。
    #[test]
    fn toml_masking_leaves_numbers_and_booleans_alone() {
        let out = mask_toml("secret_num = 42\nflag_secret = true\n");
        assert!(out.contains("secret_num = 42"), "{out}");
        assert!(out.contains("flag_secret = true"), "{out}");
    }

    #[test]
    fn numeric_token_sized_keys_are_not_masked() {
        // `CLAUDE_CODE_MAX_CONTEXT_TOKENS` 之类是容量不是密钥，误打码会让人怀疑整页
        let mut v: serde_json::Value = serde_json::from_str(
            r#"{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":1000000,"AUTH_TOKEN":"sk-1"}"#,
        )
        .unwrap();
        mask_json(&mut v);
        assert_eq!(v["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], 1000000);
        assert_eq!(v["AUTH_TOKEN"], MASK);
    }

    #[test]
    fn fingerprint_is_stable_and_short() {
        assert_eq!(fingerprint(b"abc"), fingerprint(b"abc"));
        assert_ne!(fingerprint(b"abc"), fingerprint(b"abd"));
        assert_eq!(fingerprint(b"abc").len(), 16);
    }
}
