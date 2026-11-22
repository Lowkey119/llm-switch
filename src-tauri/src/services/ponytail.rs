//! Ponytail 插件的源码检出托管、上游更新检测，以及逐端安装清单。
//!
//! ## 为什么源码要由 llm-switch 托管检出
//!
//! Ponytail 是一个 git 仓（不是单文件扩展），八家代理各按自己的官方插件机制装它，
//! 而机制的入参几乎都是「一个本地目录」或「一个 git 地址」。llm-switch 已经接管这八家
//! 的配置，装/卸/升级该由它统一做，因此源码也归它管一份：
//! `<数据目录>/ponytail`（默认 `~/.llm-switch/ponytail`），首次用时浅克隆，
//! 之后按**上游 tag** 更新——只认 tag，不追 `main`，与 ponytail 自己的发布身份一致。
//!
//! ## 上游检测口径（与 llm-switch 自查更新 `services/update.rs` 对齐，不另立一套）
//!
//! 1. **双源并行，谁新用谁**：GitHub Releases 与 npm registry 同时查，semver 新者胜；
//!    同版本优先 GitHub（带 release 说明可展示）。
//! 2. **检测失败 ≠ 无更新**：断网/限流时静默保留上次结果（store 缓存），不报错糊脸；
//!    只有用户手点「检查更新」才把失败原因说清楚。
//! 3. **三层版本分开呈现，不混为一谈**：上游最新 / 本地检出 / 各端已装。取不到就标
//!    未知，绝不拿上游版本冒充本地检出，也不拿「检出已最新」冒充「各端已装最新」。
//!
//! ## 逐端机制（2026-11-19 逐家 `--help` 实测，不是文档抄的）
//!
//! | 端 | 装 | 卸 | 探测 |
//! |---|---|---|---|
//! | pi | `pi install "<dir>"` | `pi uninstall "<dir>"` | `pi list` |
//! | claude | `plugin marketplace add` → `plugin install ponytail@ponytail` | `plugin uninstall ponytail` | `plugin list` |
//! | codebuddy | 与 claude 同一套子命令 | 同 claude | `plugin list` |
//! | codex | `plugin marketplace add` → `plugin add ponytail@ponytail` | `plugin remove ponytail` | `plugin list` |
//! | copilot | `copilot plugin install "<dir>"` | `copilot plugin uninstall ponytail` | `plugin list` |
//! | gemini | `extensions install "<dir>" --consent --skip-settings` | `extensions uninstall ponytail` | `extensions list` |
//! | opencode | `plugin add <npm名>`（只收 npm/git，不收本地目录） | `plugin remove <npm名>` | 看 opencode.json 登记 |
//! | deveco | 与 opencode 同内核，同一份 npm 包 | 同 opencode | `plugin list` |
//!
//! 探测只回答「装没装」，**不回答装的哪版**——各家 `plugin list` 的输出格式不统一，
//! 硬解析会猜。所以 ponytail 项的状态只有 `installed` / `missing` 两态，版本差一律在
//! 本节上方的「上游 / 本地检出」两行上说清，不伪装成逐端版本。

use super::ext_install::{run_login_shell, Item, Kind};
use crate::services::halctl::semver_key;
use crate::takeover::ClientApp;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

// ── 上游与包名（铁律：URL 不散落字面量）────────────────────────────────────

const UPSTREAM_GIT: &str = "https://github.com/DietrichGebert/ponytail.git";
const GH_RELEASE_LATEST: &str =
    "https://api.github.com/repos/DietrichGebert/ponytail/releases/latest";
const NPM_PACKAGE: &str = "https://registry.npmjs.org/@dietrichgebert/ponytail";
const PROJECT_PAGE: &str = "https://github.com/DietrichGebert/ponytail";

/// 插件在代理侧的包名：探测命中与卸载都用它。
pub const PKG: &str = "ponytail";
/// 托管检出目录名（相对 llm-switch 数据目录）。
const CHECKOUT: &str = "ponytail";
/// store 缓存键：上游检测结果（JSON）与检测时间（epoch 秒）。
const CACHE_JSON: &str = "ponytail.latest.json";
const CACHE_AT: &str = "ponytail.latest.at";

/// git 动作的超时：浅克隆整个仓（含 assets/benchmarks）比跑一条插件 CLI 慢得多。
const GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
/// 上游 HTTP 检测的超时（比 git 快得多，卡住多半是被墙或被限流）。
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

// ── 检出目录 ────────────────────────────────────────────────────────────────

/// 托管检出目录：`<数据目录>/ponytail`（默认 `~/.llm-switch/ponytail`）。
pub fn checkout_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(CHECKOUT)
}

/// 检出不等于「目录存在」：必须拿到带 `plugin.json` 的源码才算，半截克隆不算装好。
pub fn checked_out(dir: &Path) -> bool {
    dir.join("plugin.json").is_file()
}

/// 本地检出的版本号（读 `package.json` 的 version）。读不到返回 None，不猜也不留空串。
pub fn local_version(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("package.json")).ok()?;
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()?
        .get("version")?
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 确保有检出：没有就浅克隆默认分支。已有检出原样返回，不擅自更新。
pub fn ensure_checkout(dir: &Path) -> Result<(), String> {
    if checked_out(dir) {
        return Ok(());
    }
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建数据目录失败 {}: {e}", parent.display()))?;
    }
    // 目录存在但不是检出（半截克隆 / 用户放了别的东西）：不覆盖、不清空，直接报缺口。
    if dir.exists()
        && fs::read_dir(dir)
            .map(|mut r| r.next().is_some())
            .unwrap_or(false)
    {
        return Err(format!(
            "{} 已存在但不是 ponytail 检出（缺 plugin.json），请清空该目录或换数据目录",
            dir.display()
        ));
    }
    run_login_shell(
        &format!("git clone --depth 1 {UPSTREAM_GIT} \"{}\"", dir.display()),
        GIT_TIMEOUT,
    )?;
    if !checked_out(dir) {
        return Err("克隆完成但没找到 plugin.json，看运行日志".to_string());
    }
    Ok(())
}

/// 把检出更新到指定 tag：先取该 tag（浅克隆只有默认分支），再硬重置过去。
pub fn checkout_tag(dir: &Path, tag: &str) -> Result<(), String> {
    // 校验必须在任何磁盘/网络动作之前，且不先 trim：带前后空白的 tag 直接拒
    // （trim 后再验会把 " v1" 放过去，触发一次网络克隆才被拒——门禁实测卡死在这）。
    let t = tag.trim();
    if t.is_empty() || t != tag || t.starts_with('-') || t.contains(char::is_whitespace) {
        return Err(format!("非法的 tag 名：{tag}"));
    }
    if !checked_out(dir) {
        ensure_checkout(dir)?;
        // 浅克隆拿的就是默认分支，若上游已发布该 tag，仍需取一次 tag。
    }
    let dirq = format!("\"{}\"", dir.display());
    // tag 可能已存在（本地检出就是该 tag）：取失败不致命，后面 checkout 能过就算成功。
    let _ = run_login_shell(
        &format!("git -C {dirq} fetch --depth 1 origin tag {t}"),
        GIT_TIMEOUT,
    );
    run_login_shell(&format!("git -C {dirq} checkout --force {t}"), GIT_TIMEOUT)?;
    if !checked_out(dir) {
        return Err(format!("切到 {t} 后检出缺 plugin.json，看运行日志"));
    }
    Ok(())
}

// ── 上游检测 ────────────────────────────────────────────────────────────────

/// 一次上游检测的结果。`source` 记的是**采信源**，UI 要显示，别让用户以为只有一个源。
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Upstream {
    pub version: String,
    /// 带不带 `v` 前缀都按上游原样存（GitHub tag 常带 v，npm 不带）。
    pub tag: String,
    pub source: String,
    pub notes: String,
    pub url: String,
    pub published: Option<String>,
    /// 本次检测完成时间（epoch 秒）。缓存命中时是上次的时间，UI 必须一起显示。
    pub checked_at: u64,
}

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn http() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(8))
        .timeout(HTTP_TIMEOUT)
        .user_agent(concat!("llm-switch/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())
}

/// GitHub Releases 最新一条（公开仓，匿名可读）。
async fn from_github() -> Result<Upstream, String> {
    let j: serde_json::Value = http()?
        .get(GH_RELEASE_LATEST)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GitHub 请求失败：{e}"))?
        .json()
        .await
        .map_err(|e| format!("GitHub 解析失败：{e}"))?;
    let tag = j["tag_name"].as_str().unwrap_or("").trim().to_string();
    if tag.is_empty() {
        return Err("GitHub 没报 tag_name（可能限流或无 release）".to_string());
    }
    Ok(Upstream {
        version: tag.trim_start_matches('v').to_string(),
        tag,
        source: "GitHub".to_string(),
        notes: j["body"].as_str().unwrap_or("").to_string(),
        url: j["html_url"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| PROJECT_PAGE.to_string()),
        published: j["published_at"].as_str().map(|s| s.to_string()),
        checked_at: epoch_secs(),
    })
}

/// npm registry 的 `dist-tags.latest`。ponytail 同时发 npm，国内直连通常比 GitHub 快。
async fn from_npm() -> Result<Upstream, String> {
    let j: serde_json::Value = http()?
        .get(NPM_PACKAGE)
        .send()
        .await
        .map_err(|e| format!("npm 请求失败：{e}"))?
        .json()
        .await
        .map_err(|e| format!("npm 解析失败：{e}"))?;
    let version = j["dist-tags"]["latest"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if version.is_empty() {
        return Err("npm 没报 dist-tags.latest".to_string());
    }
    Ok(Upstream {
        version: version.clone(),
        tag: format!("v{version}"),
        source: "npm".to_string(),
        notes: String::new(),
        url: format!("{PROJECT_PAGE}/releases/tag/v{version}"),
        published: None,
        checked_at: epoch_secs(),
    })
}

/// 双源并行，semver 新者胜；同版本优先 GitHub（有 release 说明）。
/// 两源都挂才返回 Err，调用方据此区分「检测失败」与「无更新」。
pub async fn check_upstream() -> Result<Upstream, String> {
    let (gh, npm) = tokio::join!(from_github(), from_npm());
    match (gh, npm) {
        (Ok(g), Ok(n)) => Ok(if semver_key(&n.version) > semver_key(&g.version) {
            n
        } else {
            g
        }),
        (Ok(g), Err(_)) => Ok(g),
        (Err(_), Ok(n)) => Ok(n),
        (Err(ge), Err(ne)) => Err(format!("{ge}；{ne}")),
    }
}

/// 缓存进 store：整份 JSON + 检测时间。失败不覆盖旧缓存（宁缺勿假）。
pub fn cache_store(store: &crate::store::Store, up: &Upstream) {
    if let Ok(text) = serde_json::to_string(up) {
        let _ = store.set_setting(CACHE_JSON, &text);
        let _ = store.set_setting(CACHE_AT, &up.checked_at.to_string());
    }
}

/// 读上次缓存的上游结果（离线或检测失败时用它，UI 必须同时显示检测时间）。
pub fn cached_upstream(store: &crate::store::Store) -> Option<Upstream> {
    let text = store.get_setting(CACHE_JSON).ok().flatten()?;
    serde_json::from_str::<Upstream>(&text).ok()
}

/// 代理页顶部那一行要的东西：检出路径 / 本地版本 / 上游 / 是否有更新。
#[derive(Serialize)]
pub struct Overview {
    /// 托管检出目录（绝对路径，UI 直接显示，用户能一眼看出源码在哪）。
    pub checkout: String,
    pub checked_out: bool,
    /// 本地检出版本；未检出或读不到为 `None`（UI 标 `—`，不写「未知」糊过去）。
    pub local_version: Option<String>,
    pub upstream: Option<Upstream>,
    /// 上游比本地检出新。**未检出时为 false**——没装谈不上有更新。
    pub has_update: bool,
    /// 只在手动检查且两源都失败时有值；自动检查失败一律留空（口径 2）。
    pub error: Option<String>,
    /// 缓存命中的检测时间（epoch 秒）；本次现查成功时为 None。
    pub cached_at: Option<u64>,
}

fn overview(
    dir: &Path,
    upstream: Option<Upstream>,
    error: Option<String>,
    cached_at: Option<u64>,
) -> Overview {
    let out = checked_out(dir);
    let local = local_version(dir);
    let has_update = match (&upstream, &local) {
        (Some(u), Some(l)) => semver_key(&u.version) > semver_key(l),
        _ => false,
    };
    Overview {
        checkout: dir.display().to_string(),
        checked_out: out,
        local_version: local,
        upstream,
        has_update,
        error,
        cached_at,
    }
}

/// 读状态（不联网）：用 store 缓存的上游结果，检测时间一并带回。
/// 上次缓存检测的时间（epoch 秒）。读不到就是没测过，返回 None。
fn cached_at(store: &crate::store::Store) -> Option<u64> {
    store
        .get_setting(CACHE_AT)
        .ok()
        .flatten()
        .and_then(|s| s.parse::<u64>().ok())
}

pub fn overview_cached(data_dir: &Path, store: &crate::store::Store) -> Overview {
    overview(
        &checkout_dir(data_dir),
        cached_upstream(store),
        None,
        cached_at(store),
    )
}

/// 手动检查上游：成功就刷新缓存；失败保留旧缓存并把原因交给 UI 说清楚。
pub async fn overview_fresh(data_dir: &Path, store: &crate::store::Store) -> Overview {
    let dir = checkout_dir(data_dir);
    match check_upstream().await {
        Ok(up) => {
            cache_store(store, &up);
            overview(&dir, Some(up), None, None)
        }
        Err(e) => overview(&dir, cached_upstream(store), Some(e), cached_at(store)),
    }
}

// ── 逐端安装清单 ────────────────────────────────────────────────────────────

/// 一个 ponytail 安装项：装法/卸法/探测都是该端官方 CLI，`{dir}` 由安装器换成检出目录。
/// `const fn`：清单是 `static`，构造必须能在编译期跑。
const fn item(
    install: &'static str,
    uninstall: &'static str,
    probe: &'static str,
    desc: &'static str,
) -> Item {
    Item {
        name: "ponytail",
        desc,
        kind: Kind::Repo {
            install,
            uninstall,
            probe,
        },
    }
}

/// claude 与 codebuddy 同一套 marketplace 子命令：marketplace 已加过会报错，
/// 但装插件本身必须成——所以 marketplace 那条用 `;` 放行，退出码取最后一条。
const MARKETPLACE_INSTALL: &str =
    "plugin marketplace add \"{dir}\" >/dev/null 2>&1; plugin install ponytail@ponytail";

/// 该端的 ponytail 清单（没有 ponytail 机制的端返回空数组，由 `items()` 决定）。
pub fn items(app: ClientApp) -> &'static [Item] {
    static PI: [Item; 1] = [item(
        "pi install \"{dir}\"",
        "pi uninstall \"{dir}\"",
        "pi list",
        "pi 扩展：规则每轮注入 + /ponytail 六条命令",
    )];
    static CLAUDE: [Item; 1] = [item(
        MARKETPLACE_INSTALL,
        "claude plugin uninstall ponytail",
        "claude plugin list",
        "Claude 插件：marketplace 指向本地检出后装 ponytail@ponytail",
    )];
    static CODEBUDDY: [Item; 1] = [item(
        "codebuddy plugin marketplace add \"{dir}\" >/dev/null 2>&1; codebuddy plugin install ponytail@ponytail",
        "codebuddy plugin uninstall ponytail",
        "codebuddy plugin list",
        "CodeBuddy 插件：与 Claude Code 同一套 marketplace 子命令",
    )];
    static CODEX: [Item; 1] = [item(
        "codex plugin marketplace add \"{dir}\" >/dev/null 2>&1; codex plugin add ponytail@ponytail",
        "codex plugin remove ponytail",
        "codex plugin list",
        "Codex 插件：装完需在 /hooks 里信任它的两条生命周期钩子",
    )];
    static COPILOT: [Item; 1] = [item(
        "copilot plugin install \"{dir}\"",
        "copilot plugin uninstall ponytail",
        "copilot plugin list",
        "Copilot CLI 插件：直接从本地检出目录装",
    )];
    static GEMINI: [Item; 1] = [item(
        "printf 'y\\n' | GEMINI_CLI_TRUST_WORKSPACE=1 gemini extensions install \"{dir}\" --consent --skip-settings",
        "gemini extensions uninstall ponytail",
        "gemini extensions list",
        "Gemini 扩展：免交互旗标与速率扩展同一套",
    )];
    // opencode 的 `plugin add` 只收 npm 包名或 git  specifier，**不收本地目录**（实测报
    // “Plugin target must be an npm registry package or Git package specifier”），
    // 上游 INSTALL.md 给的也是 npm 名。`plugin list` 不列 npm 插件，探测只能看登记。
    static OPENCODE: [Item; 1] = [item(
        "opencode plugin add @dietrichgebert/ponytail",
        "opencode plugin remove @dietrichgebert/ponytail",
        "grep -q @dietrichgebert/ponytail \"$HOME/.config/opencode/opencode.json\" && echo ponytail",
        "OpenCode 插件：规则每轮注入 + /ponytail 命令（装 npm 包，登记进 opencode.json）",
    )];
    // DevEco Code：与 opencode 同内核，但它的 `plugin` 子命令**没有 add**，
    // 形式是 `deveco plugin <npm模块名>`（实测 `plugin add <目录>` 直接当非法参数报 usage）。
    // 它也没有插件卸载命令（`deveco uninstall` 是卸整个 deveco，绝对不能碰），
    // 所以卸法＝npm 卸包 + 从 deveco.jsonc 的 plugin 数组剔掉那条登记。
    static DEVECO: [Item; 1] = [item(
        "deveco plugin @dietrichgebert/ponytail -g",
        r#"npm --prefix "$HOME/.config/deveco" uninstall @dietrichgebert/ponytail >/dev/null 2>&1; node -e 'const fs=require("fs"),p=process.env.HOME+"/.config/deveco/deveco.jsonc",e=JSON.stringify("@dietrichgebert/ponytail");let s=fs.readFileSync(p,"utf8");const i=s.indexOf(e);if(i>=0){let a=i,b=i+e.length;while(a>0&&/\s/.test(s[a-1]))a--;if(s[a-1]===",")a--;else{while(b<s.length&&/\s/.test(s[b]))b++;if(s[b]===",")b++}fs.writeFileSync(p,s.slice(0,a)+s.slice(b))}'"#,
        "grep -q @dietrichgebert/ponytail \"$HOME/.config/deveco/deveco.jsonc\" && echo ponytail",
        "DevEco Code：与 opencode 同内核，同一份 npm 插件（上游未官方声明，装失败如实报）",
    )];

    match app {
        ClientApp::Pi => &PI,
        ClientApp::Claude => &CLAUDE,
        ClientApp::CodeBuddy => &CODEBUDDY,
        ClientApp::Codex => &CODEX,
        ClientApp::Copilot => &COPILOT,
        ClientApp::Gemini => &GEMINI,
        ClientApp::OpenCode => &OPENCODE,
        ClientApp::DevEco => &DEVECO,
    }
}

/// 装完/卸完的人话口径（UI 直接显示，别让用户猜「装完要干什么」）。
pub fn hint(app: ClientApp) -> &'static str {
    match app {
        ClientApp::Pi => {
            "重启 pi 生效：状态栏出现 ponytail 标记，/ponytail lite|full|ultra|off 切档"
        }
        ClientApp::Claude | ClientApp::CodeBuddy => {
            "新开会话生效：规则每轮注入，/ponytail 切档；首次装要在 /plugin 里确认一次"
        }
        ClientApp::Codex => "新线程生效：装完在 codex 里 /hooks 信任它的两条钩子，否则规则不注入",
        ClientApp::Copilot => "新会话生效：命令带插件名前缀，形如 /ponytail:ponytail-review",
        ClientApp::Gemini => "在 gemini 里 /extensions reload 后生效，/ponytail 切档",
        ClientApp::OpenCode | ClientApp::DevEco => "重启生效：规则每轮注入，/ponytail 命令切档",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 八个消费端都得有 ponytail 清单——漏一家就是「代理页少一节」的成因。
    #[test]
    fn every_client_has_a_ponytail_item() {
        for app in ClientApp::ALL {
            let its = items(app);
            assert_eq!(its.len(), 1, "{:?} 应恰好一条 ponytail 项", app);
            assert_eq!(its[0].name, PKG);
            match its[0].kind {
                Kind::Repo { install, probe, .. } => {
                    assert!(
                        probe.contains(app.as_str()),
                        "{:?} 探测命令应走自家 CLI",
                        app
                    );
                    assert!(!install.is_empty());
                }
                _ => panic!("{:?} 的 ponytail 项必须是 Kind::Repo", app),
            }
        }
    }

    /// 未检出时不得谎报「有更新」：没装谈不上升级。
    #[test]
    fn missing_checkout_never_claims_update() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = checkout_dir(tmp.path());
        let up = Some(Upstream {
            version: "9.9.9".into(),
            tag: "v9.9.9".into(),
            source: "GitHub".into(),
            notes: String::new(),
            url: String::new(),
            published: None,
            checked_at: 1,
        });
        let ov = overview(&dir, up, None, None);
        assert!(!ov.checked_out);
        assert!(!ov.has_update, "未检出不得报有更新");
        assert!(
            ov.local_version.is_none(),
            "未检出的版本必须是 None，UI 标 —"
        );
    }

    /// 检出版本比上游新（本地指到更新的 tag）时也不得报有更新。
    #[test]
    fn newer_checkout_is_not_an_update() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = checkout_dir(tmp.path());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("plugin.json"), "{}").unwrap();
        fs::write(dir.join("package.json"), r#"{"version":"5.4.0"}"#).unwrap();
        assert_eq!(local_version(&dir).as_deref(), Some("5.4.0"));
        let up = Some(Upstream {
            version: "5.3.3".into(),
            tag: "v5.3.3".into(),
            source: "npm".into(),
            notes: String::new(),
            url: String::new(),
            published: None,
            checked_at: 1,
        });
        assert!(!overview(&dir, up, None, None).has_update);
    }

    /// 非法 tag 必须挡住：命令串里拼进去就是命令注入。
    #[test]
    fn rejects_unsafe_tag_names() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = checkout_dir(tmp.path());
        for bad in ["", " v1", "-rf", "a b", "x;rm -rf /"] {
            assert!(checkout_tag(&dir, bad).is_err(), "tag {bad:?} 必须被拒");
        }
    }
}
