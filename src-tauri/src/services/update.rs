//! 应用更新检测与安装（fix:update-check-and-install，用户 2026-11-17 拍板 A 档）。
//!
//! 为什么要有这个文件：顶栏那个「有更新」原先是**占位常显**——没有任何检测，
//! 无论有没有新版都一直挂着，等于谎报。本仓铁律「不造假数字、不加假指标、不留占位文案」，
//! 占位必须换成真检测。
//!
//! 三条口径（都写在代码里，别只留在文档）：
//! 1. **双公仓并行检测**：GitHub Releases 与 Gitee releases 同时打，谁报的版本新
//!    用谁（同版本优先 GitHub——资产自带 digest）。撤旧「主源挂了才退备源」：
//!    主源抽风时备源不该是死备胎。
//! 2. **检测失败 ≠ 无更新**：断网/限流时静默保留上次结果、不报错糊脸；
//!    只有用户手动点「检查更新」才把失败原因说清楚。
//! 3. **A 档自动安装**：下载 → **SHA256 校验** → 拉起系统安装器
//!    （macOS `open *.dmg`、Windows 跑安装程序、Linux `xdg-open`）。
//!    不做无损替换（CI 未做代码签名，替换 /Applications 会被 Gatekeeper 拦），
//!    也不引 tauri-plugin-updater（要改发版链路与密钥托管，另案）。
//!    校验不过就删包报错，**绝不拉起没校验过的包**。

use crate::services::halctl::semver_key;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// 发布源集中定义（铁律：URL 不散落字面量）。公开仓地址见 docs/多仓发版规范.md。
const GH_OWNER: &str = "Lowkey119";
const GH_REPO: &str = "llm-switch";
const GITEE_OWNER: &str = "lowkey110";
const GITEE_REPO: &str = "llm-switch";

/// 一个可下载的安装包。`gitee_url` 可能没有：Gitee 的 Release 附件接口不开放给 API，
/// 直链只能从 Release 正文里解析（发版铁律 11），取不到就留 None，**不给假链接**。
#[derive(Clone, Serialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// CI 随包出的 SHA256（GitHub 资产自带 digest）。缺校验和的包不许自动安装。
    pub sha256: Option<String>,
    /// 是否匹配当前平台/架构（面板据此把该装的那个排在最前并标出来）。
    pub platform_match: bool,
    pub gitee_url: Option<String>,
}

/// 一次检测的结果。`error` 只在两个源都失败时有值，且只该在手动检查时展示。
#[derive(Serialize)]
pub struct Latest {
    pub current: String,
    pub tag: String,
    pub version: String,
    pub notes: String,
    pub source: String,
    pub published: Option<String>,
    pub assets: Vec<Asset>,
    pub has_update: bool,
    pub page_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 当前平台该挑哪种扩展名（顺序 = 优先级）。
fn wanted_exts() -> Vec<&'static str> {
    if cfg!(target_os = "macos") {
        vec!["dmg"]
    } else if cfg!(target_os = "windows") {
        vec!["msi", "exe"]
    } else {
        vec!["AppImage", "deb", "rpm"]
    }
}

/// 当前机器架构在文件名里的写法（mac 的 dmg 分 aarch64 / x64，装错架构装不上）。
fn arch_token() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some(if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "x64"
        })
    } else {
        None
    }
}

fn matches_platform(name: &str) -> bool {
    let lower = name.to_lowercase();
    if !wanted_exts()
        .iter()
        .any(|e| lower.ends_with(&e.to_lowercase()))
    {
        return false;
    }
    match arch_token() {
        // mac 必须对上架构；文件名没写架构的（单一通用包）不算匹配也不排除，交给用户挑。
        Some(tok) => lower.contains(tok),
        None => true,
    }
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(8))
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!("llm-switch/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// GitHub Releases latest（公开仓，匿名可读）。
async fn from_github() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{GH_OWNER}/{GH_REPO}/releases/latest");
    let j: serde_json::Value = http()?
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub 请求失败：{e}"))?
        .json()
        .await
        .map_err(|e| format!("GitHub 解析失败：{e}"))?;
    let tag = j["tag_name"]
        .as_str()
        .ok_or("GitHub 未返回 tag_name")?
        .to_string();
    let notes = j["body"].as_str().unwrap_or("").to_string();
    let published = j["published_at"].as_str().map(|s| s.to_string());
    let page = j["html_url"].as_str().unwrap_or("").to_string();
    let assets = j["assets"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|a| {
            let name = a["name"].as_str()?.to_string();
            if name.ends_with(".txt") {
                return None; // SHA256SUMS-*.txt 不是安装包
            }
            Some(Asset {
                sha256: a["digest"]
                    .as_str()
                    .map(|s| s.trim_start_matches("sha256:").to_string())
                    .filter(|s| !s.is_empty()),
                name,
                url: a["browser_download_url"].as_str().unwrap_or("").to_string(),
                size: a["size"].as_u64().unwrap_or(0),
                platform_match: false,
                gitee_url: None,
            })
        })
        .collect();
    Ok(Release {
        tag,
        notes,
        published,
        page,
        assets,
        source: "GitHub".into(),
    })
}

/// 两个源统一成这一个形状，谁报的版本新用谁。
#[derive(Clone)]
struct Release {
    tag: String,
    notes: String,
    published: Option<String>,
    page: String,
    assets: Vec<Asset>,
    source: String,
}

/// 从 Gitee Release 正文解析出可下载的包。
///
/// 为什么解析正文：Gitee 的 Release **附件接口不开放给 API**（发版铁律 11），
/// 所以 `scripts/gitee-release-sync.sh` 写进去的正文里带了「**文件名**…／下载：<url>／SHA256：`hex`」，
/// 这里按行配回来。正文不是这个格式（老的手写 Release）就返回空，让调用方退回 GitHub，
/// **绝不编一个链接出来**。
fn assets_from_gitee_body(body: &str) -> Vec<Asset> {
    let mut out: Vec<Asset> = Vec::new();
    let mut name = String::new();
    let mut url = String::new();
    for line in body.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("**") {
            // **llm-switch_0.2.55_aarch64.dmg**（macOS…，5.1 MB）
            if let Some(end) = rest.find("**") {
                name = rest[..end].to_string();
            }
        } else if let Some(rest) = l.strip_prefix("下载：<") {
            url = rest.trim_end_matches('>').to_string();
        } else if let Some(rest) = l.strip_prefix("SHA256：`") {
            let sha = rest.trim_end_matches('`').to_string();
            if !name.is_empty() && url.starts_with("https://") {
                out.push(Asset {
                    name: std::mem::take(&mut name),
                    gitee_url: Some(std::mem::take(&mut url)),
                    sha256: (!sha.is_empty()).then_some(sha),
                    url: String::new(),
                    size: 0,
                    platform_match: false,
                });
            }
        }
    }
    out
}

/// Gitee releases（取列表里 semver 最新的一条）。国内直连比 GitHub 快，作为备源。
async fn from_gitee() -> Result<Release, String> {
    let url =
        format!("https://gitee.com/api/v5/repos/{GITEE_OWNER}/{GITEE_REPO}/releases?per_page=20");
    let j: serde_json::Value = http()?
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Gitee 请求失败：{e}"))?
        .json()
        .await
        .map_err(|e| format!("Gitee 解析失败：{e}"))?;
    let rel = j
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r["tag_name"].as_str().is_some_and(|t| !t.is_empty()))
        .max_by_key(|r| semver_key(r["tag_name"].as_str().unwrap_or("")));
    let rel = rel.ok_or("Gitee 未返回任何 Release")?;
    let body = rel["body"].as_str().unwrap_or("").to_string();
    Ok(Release {
        tag: rel["tag_name"].as_str().unwrap_or("").to_string(),
        notes: body.clone(),
        published: rel["publish_at"].as_str().map(str::to_string),
        page: rel["html_url"].as_str().unwrap_or("").to_string(),
        assets: assets_from_gitee_body(&body),
        source: "Gitee".into(),
    })
}

/// 双源结果择一（纯函数，单测锁口径）：谁报的版本新用谁；同版本优先 GitHub
/// （资产自带 SHA256 digest）；单源失败用成功那份并记进 `errors`；全失败返回 None。
fn pick_release(
    gh: Result<Release, String>,
    ge: Result<Release, String>,
) -> (Option<Release>, Vec<String>) {
    match (gh, ge) {
        (Ok(a), Ok(b)) => {
            if semver_key(&b.tag) > semver_key(&a.tag) {
                (Some(b), Vec::new())
            } else {
                (Some(a), Vec::new())
            }
        }
        (Ok(a), Err(e)) => {
            eprintln!("[update] Gitee 源失败，GitHub 结果可用：{e}");
            (Some(a), vec![format!("Gitee：{e}")])
        }
        (Err(e), Ok(b)) => {
            eprintln!("[update] GitHub 源失败，Gitee 结果可用：{e}");
            (Some(b), vec![format!("GitHub：{e}")])
        }
        (Err(e1), Err(e2)) => (None, vec![format!("GitHub：{e1}"), format!("Gitee：{e2}")]),
    }
}

/// 检测新版本。**双公仓并行查询**（用户 2026-11-17 拍板，撤「GitHub 主、Gitee 备」），
/// 择一规则见 `pick_release`。两源都失败时仍返回 `Ok`，但 `error` 有值、`has_update`
/// 为 false——前端据此**保持原状**（不显示标签、不报错糊脸），只有手动检查才把
/// `error` 说出来。
pub async fn check() -> Result<Latest, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let (gh, ge) = tokio::join!(from_github(), from_gitee());
    let (rel, errors) = pick_release(gh, ge);
    let Some(rel) = rel else {
        return Ok(Latest {
            current,
            tag: String::new(),
            version: String::new(),
            notes: String::new(),
            source: String::new(),
            published: None,
            assets: Vec::new(),
            has_update: false,
            page_url: String::new(),
            error: Some(errors.join("；")),
        });
    };
    let version = rel.tag.trim_start_matches('v').to_string();
    let mut assets = rel.assets;
    for a in assets.iter_mut() {
        a.platform_match = matches_platform(&a.name);
    }
    // 该装的那个排最前，其余按名字稳定排序，面板上一眼能找到。
    assets.sort_by(|a, b| {
        b.platform_match
            .cmp(&a.platform_match)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(Latest {
        has_update: semver_key(&version) > semver_key(&current),
        current,
        tag: rel.tag,
        version,
        notes: rel.notes,
        source: rel.source,
        published: rel.published,
        assets,
        page_url: rel.page,
        error: (!errors.is_empty()).then(|| errors.join("；")),
    })
}

/// 下载到临时目录并校验 SHA256。`progress` 由调用方给（0.0..=1.0 回报）。
/// 校验不过：删包、报错，**绝不返回路径**（也就绝不会被拉起）。
pub async fn download_and_verify(
    url: &str,
    sha256: Option<&str>,
    name: &str,
    mut progress: impl FnMut(f64, &str),
) -> Result<PathBuf, String> {
    if url.is_empty() || !url.starts_with("https://") {
        return Err("下载地址不合法，拒绝下载".into());
    }
    let dir = std::env::temp_dir().join("llm-switch-update");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("建临时目录失败：{e}"))?;
    let path = dir.join(name);
    // 大文件不能套总超时（30s 下不完 80MB 的包），这里单独建一个不限总超时的客户端。
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!("llm-switch/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败：HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::io::BufWriter::new(
        std::fs::File::create(&path).map_err(|e| format!("写文件失败：{e}"))?,
    );
    use futures_util::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut got: u64 = 0;
    let mut hasher = Sha256::new();
    let mut last = std::time::Instant::now();
    while let Some(chunk) = stream.next().await {
        let b = chunk.map_err(|e| format!("下载中断：{e}"))?;
        hasher.update(&b);
        std::io::Write::write_all(&mut file, &b).map_err(|e| format!("写文件失败：{e}"))?;
        got += b.len() as u64;
        // 进度别每块都推（每秒几十次会把前端刷爆），200ms 一次。
        if last.elapsed().as_millis() > 200 {
            last = std::time::Instant::now();
            let p = if total > 0 {
                got as f64 / total as f64
            } else {
                0.0
            };
            progress(p, &format!("已下载 {} / {}", human(got), human(total)));
        }
    }
    std::io::Write::flush(&mut file).map_err(|e| e.to_string())?;
    drop(file);
    progress(1.0, "校验 SHA256…");
    let hex = format!("{:x}", hasher.finalize());
    match sha256 {
        Some(want) if !want.is_empty() => {
            if !hex.eq_ignore_ascii_case(want.trim()) {
                let _ = std::fs::remove_file(&path);
                return Err(format!(
                    "SHA256 校验不符，已删除下载文件（期望 {}…，实际 {}…）",
                    &want[..12.min(want.len())],
                    &hex[..12]
                ));
            }
        }
        // 没有校验和就不许自动装：这是 A 档的底线，宁可让用户自己去发布页下。
        _ => {
            let _ = std::fs::remove_file(&path);
            return Err("该包没有 SHA256 校验和，拒绝自动安装；请到发布页自行下载".into());
        }
    }
    Ok(path)
}

/// 拉起系统安装器（A 档最后一步）。macOS 只是打开 dmg，**仍需用户把 App 拖进 Applications**，
/// 面板文案必须说清，不许写得像已经装好了。
pub fn launch_installer(path: &std::path::Path) -> Result<String, String> {
    let p = path.to_string_lossy().to_string();
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(&p);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", &p]);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&p);
        c
    };
    cmd.spawn().map_err(|e| format!("拉起安装器失败：{e}"))?;
    Ok(if cfg!(target_os = "macos") {
        "已打开安装包：把 llm-switch 拖进 Applications 覆盖即完成更新".to_string()
    } else {
        "已拉起安装程序，按提示完成更新".to_string()
    })
}

fn human(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1}MB", n as f64 / 1024.0 / 1024.0)
    } else if n >= 1024 {
        format!("{:.0}KB", n as f64 / 1024.0)
    } else {
        format!("{n}B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, source: &str) -> Release {
        Release {
            tag: tag.to_string(),
            notes: String::new(),
            published: None,
            page: String::new(),
            assets: Vec::new(),
            source: source.to_string(),
        }
    }

    #[test]
    fn dual_sources_pick_the_newer_tag() {
        let (picked, errors) =
            pick_release(Ok(rel("v0.2.61", "GitHub")), Ok(rel("v0.2.62", "Gitee")));
        assert_eq!(picked.unwrap().tag, "v0.2.62");
        assert!(errors.is_empty());
        // 反向同样取较新者。
        let (picked, errors) =
            pick_release(Ok(rel("v0.2.63", "GitHub")), Ok(rel("v0.2.62", "Gitee")));
        assert_eq!(picked.unwrap().tag, "v0.2.63");
        assert!(errors.is_empty());
    }

    #[test]
    fn same_version_prefers_github_for_digests() {
        let (picked, errors) =
            pick_release(Ok(rel("v0.2.62", "GitHub")), Ok(rel("v0.2.62", "Gitee")));
        let picked = picked.unwrap();
        assert_eq!(picked.tag, "v0.2.62");
        assert_eq!(picked.source, "GitHub");
        assert!(errors.is_empty());
    }

    #[test]
    fn one_failed_source_yields_the_other_and_records_why() {
        let (picked, errors) = pick_release(
            Err("GitHub 请求失败：超时".to_string()),
            Ok(rel("v0.2.62", "Gitee")),
        );
        assert_eq!(picked.unwrap().source, "Gitee");
        assert_eq!(errors, vec!["GitHub：GitHub 请求失败：超时".to_string()]);
        // 反向也这样。
        let (picked, errors) = pick_release(
            Ok(rel("v0.2.62", "GitHub")),
            Err("Gitee 解析失败：".to_string()),
        );
        assert_eq!(picked.unwrap().source, "GitHub");
        assert!(errors[0].starts_with("Gitee："));
    }

    #[test]
    fn both_failed_means_no_release_with_joined_error() {
        let (picked, errors) = pick_release(Err("a".to_string()), Err("b".to_string()));
        assert!(picked.is_none());
        assert_eq!(
            errors,
            vec!["GitHub：a".to_string(), "Gitee：b".to_string()]
        );
    }
}
