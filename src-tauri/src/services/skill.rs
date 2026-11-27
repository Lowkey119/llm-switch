//! Copyright (c) 2025 Jason Young，见 docs/NOTICE-upstream.md）。
//!
//! 适配为本仓库形态的裁剪版：
//! - SSOT（单一事实源）：`<data_dir>/skills/`（默认 `~/.llm-switch/skills/`）；
//!   卸载备份：`<data_dir>/skill-backups/`。
//! - 八客户端目录映射（计划 §M6 + 后来补的端）：Claude Code / Codex / OpenCode /
//!   DevEco / Gemini / Pi / CodeBuddy / Copilot。
//! - 分发方式：软链（默认，失败自动回退复制）/ 纯复制，settings 表全局配置。
//!   一切破坏性操作（覆盖同步 / 停用 / 卸载）先对目标做「身份鉴定」——
//!   指向 SSOT 的软链、内容与 SSOT 一致的复制目录才算「我方部署」；
//!   同名但内容不同的真实目录（用户自有）一律拒绝触碰。
//!   在参与任何 join/删除前必须过 `require_valid_directory`（单段、无穿越）。
//!
//! 本切片（A）不含：git/ZIP 安装（B）、仓库发现与更新（C）。

use crate::config::Config;
use crate::store::Store;
use crate::types::{now_ms, SkillAppRow, SkillRecord};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{OnceLock, RwLock, RwLockWriteGuard};
use std::time::{SystemTime, UNIX_EPOCH};

/// 卸载备份保留个数（超出按 mtime 逐旧）。
const BACKUP_RETAIN_COUNT: usize = 20;

/// settings 表键：全局分发方式。
pub const SYNC_METHOD_SETTING: &str = "skills.sync_method";

// ========== 并发协调 ==========

/// DB 状态与文件系统 SSOT 的一致性变更不允许交错。
fn skill_lock() -> &'static RwLock<()> {
    static LOCK: OnceLock<RwLock<()>> = OnceLock::new();
    LOCK.get_or_init(|| RwLock::new(()))
}

fn write_guard() -> RwLockWriteGuard<'static, ()> {
    skill_lock().write().unwrap_or_else(|p| p.into_inner())
}

// ========== 客户端与分发方式 ==========

/// 受管客户端（计划 §M6 五端 + 后来补的三端）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkillApp {
    Claude,
    Codex,
    OpenCode,
    DevEco,
    Gemini,
    Pi,
    CodeBuddy,
    Copilot,
}

impl SkillApp {
    pub const ALL: [SkillApp; 8] = [
        SkillApp::Claude,
        SkillApp::Codex,
        SkillApp::OpenCode,
        SkillApp::DevEco,
        SkillApp::Gemini,
        SkillApp::Pi,
        SkillApp::CodeBuddy,
        SkillApp::Copilot,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            SkillApp::Claude => "claude",
            SkillApp::Codex => "codex",
            SkillApp::OpenCode => "opencode",
            SkillApp::DevEco => "deveco",
            SkillApp::Gemini => "gemini",
            SkillApp::Pi => "pi",
            SkillApp::CodeBuddy => "codebuddy",
            SkillApp::Copilot => "copilot",
        }
    }

    pub fn parse(s: &str) -> Result<SkillApp, String> {
        SkillApp::ALL
            .into_iter()
            .find(|a| a.as_str() == s)
            .ok_or_else(|| format!("unknown skill app: {s}"))
    }

    /// 各客户端 skills 目录（相对 home，计划 §M6 列表）。
    pub fn dir(self, home: &Path) -> PathBuf {
        match self {
            SkillApp::Claude => home.join(".claude").join("skills"),
            SkillApp::Codex => home.join(".codex").join("skills"),
            SkillApp::OpenCode => home.join(".config").join("opencode").join("skills"),
            // DevEco Code 的内核是 opencode，技能目录也跟着同一形状
            SkillApp::DevEco => home.join(".config").join("deveco").join("skills"),
            SkillApp::Gemini => home.join(".gemini").join("skills"),
            SkillApp::Pi => home.join(".pi").join("agent").join("skills"),
            SkillApp::CodeBuddy => home.join(".codebuddy").join("skills"),
            // Copilot CLI 的 Personal 技能目录（`copilot skill --help` 官方口径：
            // Personal = ~/.copilot/skills/ 或 ~/.agents/skills/）。门面端没有接管配置，
            // 但技能这一层它是真能吃的，所以技能矩阵必须有它一列。
            SkillApp::Copilot => home.join(".copilot").join("skills"),
        }
    }
}

/// 分发方式（全局配置，默认 Auto：优先软链，失败回退复制）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncMethod {
    #[default]
    Auto,
    Symlink,
    Copy,
}

impl SyncMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncMethod::Auto => "auto",
            SyncMethod::Symlink => "symlink",
            SyncMethod::Copy => "copy",
        }
    }

    pub fn parse(s: &str) -> Result<SyncMethod, String> {
        match s {
            "auto" => Ok(SyncMethod::Auto),
            "symlink" => Ok(SyncMethod::Symlink),
            "copy" => Ok(SyncMethod::Copy),
            other => Err(format!("invalid sync_method: {other}")),
        }
    }

    pub fn from_store(store: &Store) -> SyncMethod {
        store
            .get_setting(SYNC_METHOD_SETTING)
            .ok()
            .flatten()
            .and_then(|v| SyncMethod::parse(&v).ok())
            .unwrap_or_default()
    }
}

// ========== 环境 ==========

/// 文件系统布局。生产从 Config 派生；测试用 [`SkillEnv::with_roots`] 指到临时目录。
#[derive(Debug, Clone)]
pub struct SkillEnv {
    pub home: PathBuf,
    pub ssot: PathBuf,
    pub backups: PathBuf,
}

impl SkillEnv {
    pub fn from_config(config: &Config) -> SkillEnv {
        let data = config.data_dir();
        SkillEnv {
            home: crate::config::dirs_home(),
            ssot: data.join("skills"),
            backups: data.join("skill-backups"),
        }
    }

    pub fn with_roots(home: PathBuf, ssot: PathBuf, backups: PathBuf) -> SkillEnv {
        SkillEnv {
            home,
            ssot,
            backups,
        }
    }

    pub fn ensure_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(&self.ssot).map_err(|e| format!("创建 SSOT 目录失败: {e}"))?;
        fs::create_dir_all(&self.backups).map_err(|e| format!("创建备份目录失败: {e}"))?;
        Ok(())
    }

    pub fn app_dir(&self, app: SkillApp) -> PathBuf {
        app.dir(&self.home)
    }
}

// ========== 对外视图 ==========

/// 列表页一行：DB 记录 + 每客户端期望/实际状态。
#[derive(Debug, Clone, Serialize)]
pub struct SkillView {
    pub dir: String,
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub version: Option<String>,
    pub latest: Option<String>,
    pub size: i64,
    pub managed: bool,
    pub updated_at: i64,
    pub apps: Vec<SkillAppState>,
}

/// 某技能的某客户端状态格（矩阵视图数据源）。
#[derive(Debug, Clone, Serialize)]
pub struct SkillAppState {
    pub app: String,
    /// 期望状态（skill_apps 行）。
    pub enabled: bool,
    /// 实际状态：linked | copied | missing | foreign | absent。
    /// enabled 且 absent 记 missing（断链/被外部删除，UI 应提示可重同步）。
    pub actual: String,
    pub sync_mode: String,
}

/// 卸载备份条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupEntry {
    pub backup_id: String,
    pub backup_path: String,
    pub created_at: i64,
    pub dir: String,
    pub name: String,
    pub source_path: String,
}

/// 备份 meta.json：技能行快照 + 当时的启用开关快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupMeta {
    skill: SkillRecord,
    apps: Vec<SkillAppRow>,
    created_at: i64,
    source_path: String,
}

/// 目标目录的身份鉴定结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    /// 不存在。
    Absent,
    /// 指向 SSOT 的软链。
    Linked,
    /// 内容与 SSOT 一致的真实目录（复制分发产物）。
    Copied,
    /// 他人资产（异向软链 / 内容不同的真实目录）——禁止触碰。
    Foreign,
    /// 真实目录但 SSOT 源已丢（无法比对内容）：停用拒绝，
    /// 卸载时先备份再删（内容不丢）。
    OrphanCopy,
}

/// 校验并规范化安装目录名（最终落盘目录名，仅单段）。
fn sanitize_install_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    // 显式拒绝两种分隔符，不能依赖 components() 的平台语义：
    // `\` 在 Linux/macOS 上不是分隔符，会被当成合法单段名放行，
    // 但同一个值同步/还原到 Windows 上就变成了嵌套路径。
    if trimmed.contains('/') || trimmed.contains('\\') {
        return None;
    }
    let path = Path::new(trimmed);
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) => {
            let normalized = name.to_string_lossy().trim().to_string();
            if normalized.is_empty()
                || normalized == "."
                || normalized == ".."
                || normalized.starts_with('.')
            {
                None
            } else {
                Some(normalized)
            }
        }
        _ => None,
    }
}

/// 校验来自 DB 行 / 备份 meta.json 等外部来源的 directory 字段。
/// 只校验、不归一化：normalize 后的值可能与磁盘真实目录名（带空格）不同，
/// 静默替换反而让合法目录永远 join 不中，因此要求与原值逐字相同。
fn require_valid_directory(directory: &str) -> Result<String, String> {
    match sanitize_install_name(directory) {
        Some(clean) if clean == directory => Ok(directory.to_string()),
        _ => Err(format!("非法的技能目录名: {directory:?}")),
    }
}

fn sanitize_backup_segment(segment: &str) -> String {
    let sanitized: String = segment
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '-',
        })
        .collect();
    let trimmed = sanitized.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "skill".to_string()
    } else {
        trimmed
    }
}

fn backup_path_for_id(env: &SkillEnv, backup_id: &str) -> Result<PathBuf, String> {
    if backup_id.contains("..")
        || backup_id.contains('/')
        || backup_id.contains('\\')
        || backup_id.trim().is_empty()
    {
        return Err(format!("非法的备份 id: {backup_id}"));
    }
    Ok(env.backups.join(backup_id))
}

fn is_symlink(path: &Path) -> bool {
    path.symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(unix)]
fn create_symlink(src: &Path, dest: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(src, dest).map_err(|e| {
        format!(
            "创建符号链接失败: {} -> {}: {e}",
            src.display(),
            dest.display()
        )
    })
}

#[cfg(windows)]
fn create_symlink(src: &Path, dest: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_dir(src, dest).map_err(|e| {
        format!(
            "创建符号链接失败: {} -> {}: {e}",
            src.display(),
            dest.display()
        )
    })
}

/// 删除路径（软链只删链接本身，不动源）。
fn remove_path(path: &Path) -> Result<(), String> {
    if is_symlink(path) {
        #[cfg(unix)]
        fs::remove_file(path).map_err(|e| format!("删除软链失败 {}: {e}", path.display()))?;
        #[cfg(windows)]
        fs::remove_dir(path).map_err(|e| format!("删除软链失败 {}: {e}", path.display()))?;
    } else if path.is_dir() {
        fs::remove_dir_all(path).map_err(|e| format!("删除目录失败 {}: {e}", path.display()))?;
    } else if path.exists() {
        fs::remove_file(path).map_err(|e| format!("删除文件失败 {}: {e}", path.display()))?;
    }
    Ok(())
}

/// 判断路径是否为指向 SSOT 目录内的符号链接。
fn is_symlink_to_ssot(path: &Path, ssot_dir: &Path) -> bool {
    if !is_symlink(path) {
        return false;
    }
    let Ok(target) = fs::read_link(path) else {
        return false;
    };
    if target.is_absolute() && target.starts_with(ssot_dir) {
        return true;
    }
    let resolved = path
        .parent()
        .map(|parent| parent.join(&target))
        .unwrap_or(target.clone());
    let canonical_ssot = ssot_dir
        .canonicalize()
        .unwrap_or_else(|_| ssot_dir.to_path_buf());
    let canonical_target = resolved.canonicalize().unwrap_or(resolved);
    canonical_target.starts_with(&canonical_ssot)
}

fn paths_alias(left: &Path, right: &Path) -> Result<(), String> {
    let same = left == right
        || matches!(
            (left.canonicalize(), right.canonicalize()),
            (Ok(l), Ok(r)) if l == r
        );
    if same {
        return Err(format!(
            "SSOT 目录不能与客户端 skills 目录相同: {}",
            left.display()
        ));
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("创建目录失败 {}: {e}", dest.display()))?;
    for entry in fs::read_dir(src).map_err(|e| format!("读目录失败 {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("读目录项失败: {e}"))?;
        let path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dest_path)?;
        } else {
            fs::copy(&path, &dest_path).map_err(|e| {
                format!(
                    "复制失败 {} -> {}: {e}",
                    path.display(),
                    dest_path.display()
                )
            })?;
        }
    }
    Ok(())
}

/// 目录树指纹：条目类型 + 相对路径 + 内容（软链记目标）。
/// Pi deployment hash 思路并推广到全部客户端）。
fn tree_hash(dir: &Path) -> Option<String> {
    let mut entries: Vec<PathBuf> = Vec::new();
    collect_tree(dir, &mut entries).ok()?;
    entries.sort();
    let mut hasher = Sha256::new();
    for path in entries {
        let relative = path.strip_prefix(dir).unwrap_or(&path);
        hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update(b"\0");
        let md = fs::symlink_metadata(&path).ok()?;
        let ft = md.file_type();
        if ft.is_symlink() {
            hasher.update(b"link\0");
            hasher.update(fs::read_link(path).ok()?.to_string_lossy().as_bytes());
        } else if ft.is_dir() {
            hasher.update(b"dir\0");
        } else if ft.is_file() {
            hasher.update(b"file\0");
            hasher.update(fs::read(path).ok()?);
        } else {
            hasher.update(b"other\0");
        }
        hasher.update(b"\0");
    }
    Some(format!("{:x}", hasher.finalize()))
}

fn collect_tree(current: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|e| format!("读目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读目录项失败: {e}"))?;
        let path = entry.path();
        out.push(path.clone());
        if path.is_dir() && !is_symlink(&path) {
            collect_tree(&path, out)?;
        }
    }
    Ok(())
}

/// 目录体积（字节，粗粒度即可，仅展示用）。
fn dir_size(dir: &Path) -> i64 {
    let mut total: i64 = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let Ok(entries) = fs::read_dir(&cur) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !is_symlink(&path) {
                stack.push(path);
            } else if let Ok(md) = entry.metadata() {
                total += md.len() as i64;
            }
        }
    }
    total
}

#[derive(Debug, Default, Deserialize)]
struct SkillMetadata {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

fn parse_skill_metadata(path: &Path) -> SkillMetadata {
    let Ok(content) = fs::read_to_string(path) else {
        return SkillMetadata::default();
    };
    let content = content.trim_start_matches('\u{feff}');
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return SkillMetadata::default();
    }
    serde_yaml::from_str(parts[1].trim()).unwrap_or_default()
}

fn read_skill_name_desc(skill_md: &Path, fallback: &str) -> (String, Option<String>) {
    if !skill_md.exists() {
        return (fallback.to_string(), None);
    }
    let meta = parse_skill_metadata(skill_md);
    (
        meta.name.unwrap_or_else(|| fallback.to_string()),
        meta.description,
    )
}

// ========== 目标身份鉴定 ==========

/// 判定 `app` 目录里 `dir` 这一条相对 SSOT 源的身份。
fn classify(env: &SkillEnv, dir: &str, app: SkillApp) -> EntryKind {
    let dest = env.app_dir(app).join(dir);
    if is_symlink(&dest) {
        if is_symlink_to_ssot(&dest, &env.ssot) {
            // 指向 SSOT 但目标已被删（断链）：按不存在处理，enabled 时显 missing。
            if fs::metadata(&dest).is_err() {
                return EntryKind::Absent;
            }
            EntryKind::Linked
        } else {
            EntryKind::Foreign
        }
    } else if dest.is_dir() {
        let source = env.ssot.join(dir);
        if !source.is_dir() {
            EntryKind::OrphanCopy
        } else if tree_hash(&dest) == tree_hash(&source) {
            EntryKind::Copied
        } else {
            EntryKind::Foreign
        }
    } else {
        EntryKind::Absent
    }
}

fn actual_label(kind: EntryKind, enabled: bool) -> &'static str {
    match kind {
        EntryKind::Absent => {
            if enabled {
                "missing"
            } else {
                "absent"
            }
        }
        EntryKind::Linked => "linked",
        EntryKind::Copied => "copied",
        EntryKind::Foreign => "foreign",
        EntryKind::OrphanCopy => "orphan",
    }
}

// ========== 同步 / 移除 ==========

fn validate_sync_source(env: &SkillEnv, dir: &str) -> Result<PathBuf, String> {
    let source = env.ssot.join(dir);
    if !source.is_dir() {
        return Err(format!("技能不存在于 SSOT: {dir}"));
    }
    if !source.join("SKILL.md").is_file() {
        return Err(format!(
            "技能源目录缺少 SKILL.md，拒绝同步: {}",
            source.display()
        ));
    }
    Ok(source)
}

/// 原子替换：先复制到同级临时目录再 rename，避免半途失败留下半份目录。
fn replace_dest_with_copy(source: &Path, dest: &Path, dir: &str) -> Result<(), String> {
    let parent = dest
        .parent()
        .ok_or_else(|| format!("非法的目标路径: {}", dest.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = parent.join(format!(
        ".{}.tmp-{}-{nonce}",
        sanitize_backup_segment(dir),
        std::process::id()
    ));
    if tmp.exists() || is_symlink(&tmp) {
        remove_path(&tmp)?;
    }
    if let Err(e) = copy_dir_recursive(source, &tmp) {
        let _ = remove_path(&tmp);
        return Err(e);
    }
    if dest.exists() || is_symlink(dest) {
        remove_path(dest)?;
    }
    if let Err(e) = fs::rename(&tmp, dest) {
        let _ = remove_path(&tmp);
        return Err(format!(
            "替换技能目录失败 {} -> {}: {e}",
            tmp.display(),
            dest.display()
        ));
    }
    Ok(())
}

/// 把一个技能投影到某客户端目录。返回实际采用的方式（"symlink"|"copy"）。
/// 目标是「他人资产」时拒绝覆盖。
fn sync_one(
    env: &SkillEnv,
    dir: &str,
    app: SkillApp,
    method: SyncMethod,
) -> Result<&'static str, String> {
    let dir = require_valid_directory(dir)?;
    let source = validate_sync_source(env, &dir)?;
    let app_dir = env.app_dir(app);
    paths_alias(&env.ssot, &app_dir)?;
    fs::create_dir_all(&app_dir).map_err(|e| format!("创建客户端 skills 目录失败: {e}"))?;
    let dest = app_dir.join(&dir);

    match classify(env, &dir, app) {
        EntryKind::Foreign => {
            return Err(format!(
                "{:?} 中已存在同名但非本工具部署的技能，拒绝覆盖: {dir}",
                app
            ))
        }
        EntryKind::OrphanCopy => {
            return Err(format!(
                "{:?} 中存在同名目录且 SSOT 源缺失，无法鉴定归属，拒绝覆盖: {dir}",
                app
            ))
        }
        EntryKind::Linked | EntryKind::Copied => {
            remove_path(&dest)?;
        }
        EntryKind::Absent => {}
    }

    match method {
        SyncMethod::Symlink => {
            create_symlink(&source, &dest)?;
            Ok("symlink")
        }
        SyncMethod::Copy => {
            replace_dest_with_copy(&source, &dest, &dir)?;
            Ok("copy")
        }
        SyncMethod::Auto => match create_symlink(&source, &dest) {
            Ok(()) => Ok("symlink"),
            Err(e) => {
                tracing::warn!("软链创建失败，回退复制: {e}");
                replace_dest_with_copy(&source, &dest, &dir)?;
                Ok("copy")
            }
        },
    }
}

/// 从客户端目录移除（仅移除可鉴定为我方部署的条目；他人资产报错）。
fn remove_one(env: &SkillEnv, dir: &str, app: SkillApp) -> Result<(), String> {
    let dir = require_valid_directory(dir)?;
    let dest = env.app_dir(app).join(&dir);
    match classify(env, &dir, app) {
        EntryKind::Linked | EntryKind::Copied => remove_path(&dest),
        EntryKind::Absent => Ok(()),
        EntryKind::OrphanCopy => Err(format!(
            "{:?} 中的 {dir} 是无 SSOT 源的同名目录，无法鉴定归属，拒绝删除（可在卸载时自动备份后清理）",
            app
        )),
        EntryKind::Foreign => Err(format!(
            "{:?} 中的 {dir} 不是本工具部署的副本，拒绝删除",
            app
        )),
    }
}

/// 逐文件 feed "相对路径\0内容\0"。更新检测的本地/远端可比对基准。
fn compute_dir_hash(dir: &Path) -> Result<String, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_files_for_hash(dir, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for file_path in &files {
        let relative = file_path.strip_prefix(dir).unwrap_or(file_path);
        hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update(b"\0");
        hasher.update(
            fs::read(file_path).map_err(|e| format!("读取失败 {}: {e}", file_path.display()))?,
        );
        hasher.update(b"\0");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files_for_hash(current: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|e| format!("读目录失败: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_files_for_hash(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

// ========== 对外 API ==========

/// SSOT ↔ DB 对账扫描：新目录入库（managed=0），消失目录清行。
/// 返回 `{"added":N,"updated":N,"removed":N}`。
pub fn scan(env: &SkillEnv, store: &Store) -> Result<serde_json::Value, String> {
    let _guard = write_guard();
    env.ensure_dirs()?;

    let disk: HashSet<String> = fs::read_dir(&env.ssot)
        .map_err(|e| format!("读 SSOT 目录失败: {e}"))?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            (path.is_dir()
                && !name.starts_with('.')
                && sanitize_install_name(&name).as_deref() == Some(name.as_str())
                && path.join("SKILL.md").is_file())
            .then_some(name)
        })
        .collect();

    let existing: HashMap<String, SkillRecord> = store
        .list_skills()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| (r.dir.clone(), r))
        .collect();

    let mut added = 0usize;
    let mut updated = 0usize;
    for dir in disk.iter() {
        let skill_dir = env.ssot.join(dir);
        let (name, description) = read_skill_name_desc(&skill_dir.join("SKILL.md"), dir);
        let size = dir_size(&skill_dir);
        let prev = existing.get(dir);
        let rec = match prev {
            Some(old) => SkillRecord {
                dir: dir.clone(),
                name,
                description,
                repo: old.repo.clone(),
                branch: old.branch.clone(),
                // 早期版本未存哈希的行在此自愈补写。
                version: old
                    .version
                    .clone()
                    .or_else(|| compute_dir_hash(&skill_dir).ok()),
                latest: old.latest.clone(),
                size,
                managed: old.managed,
                updated_at: old.updated_at,
            },
            None => SkillRecord {
                dir: dir.clone(),
                name,
                description,
                repo: None,
                branch: None,
                version: None,
                latest: None,
                size,
                managed: false,
                updated_at: now_ms(),
            },
        };
        let changed = match prev {
            None => true,
            Some(old) => {
                old.name != rec.name || old.description != rec.description || old.size != rec.size
            }
        };
        store.upsert_skill(&rec).map_err(|e| e.to_string())?;
        if prev.is_none() {
            added += 1;
        } else if changed {
            updated += 1;
        }
    }

    let mut removed = 0usize;
    for dir in existing.keys() {
        if !disk.contains(dir) {
            store.delete_skill(dir).map_err(|e| e.to_string())?;
            removed += 1;
        }
    }

    Ok(json!({ "added": added, "updated": updated, "removed": removed }))
}

/// 列表（含每客户端期望/实际状态，矩阵视图直接消费）。
pub fn list(env: &SkillEnv, store: &Store) -> Result<Vec<SkillView>, String> {
    let rows = store.list_skills().map_err(|e| e.to_string())?;
    let app_rows = store.list_skill_apps(None).map_err(|e| e.to_string())?;
    let mut by_dir: HashMap<String, Vec<SkillAppRow>> = HashMap::new();
    for r in app_rows {
        by_dir.entry(r.skill_dir.clone()).or_default().push(r);
    }

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let switches = by_dir.remove(&row.dir).unwrap_or_default();
        let apps = SkillApp::ALL
            .iter()
            .map(|app| {
                let sw = switches.iter().find(|s| s.app_id == app.as_str());
                let enabled = sw.is_some_and(|s| s.enabled);
                let sync_mode = sw.map(|s| s.sync_mode.clone()).unwrap_or_default();
                let kind = classify(env, &row.dir, *app);
                SkillAppState {
                    app: app.as_str().to_string(),
                    enabled,
                    actual: actual_label(kind, enabled).to_string(),
                    sync_mode,
                }
            })
            .collect();
        out.push(SkillView {
            dir: row.dir,
            name: row.name,
            description: row.description,
            repo: row.repo,
            branch: row.branch,
            version: row.version,
            latest: row.latest,
            size: row.size,
            managed: row.managed,
            updated_at: row.updated_at,
            apps,
        });
    }
    Ok(out)
}

/// 启用/停用某技能的某客户端：改开关 + 落盘/清理。返回实际状态格。
pub fn set_enabled(
    env: &SkillEnv,
    store: &Store,
    dir: &str,
    app: SkillApp,
    enabled: bool,
) -> Result<SkillAppState, String> {
    let _guard = write_guard();
    let dir = require_valid_directory(dir)?;
    if store.get_skill(&dir).map_err(|e| e.to_string())?.is_none() {
        return Err(format!("技能不存在: {dir}"));
    }
    env.ensure_dirs()?;

    let method = SyncMethod::from_store(store);
    if enabled {
        let mode = sync_one(env, &dir, app, method)?;
        store
            .set_skill_app(&dir, app.as_str(), true, mode)
            .map_err(|e| e.to_string())?;
        Ok(SkillAppState {
            app: app.as_str().to_string(),
            enabled: true,
            actual: actual_label(classify(env, &dir, app), true).to_string(),
            sync_mode: mode.to_string(),
        })
    } else {
        remove_one(env, &dir, app)?;
        store
            .set_skill_app(&dir, app.as_str(), false, "")
            .map_err(|e| e.to_string())?;
        Ok(SkillAppState {
            app: app.as_str().to_string(),
            enabled: false,
            actual: "absent".to_string(),
            sync_mode: String::new(),
        })
    }
}

/// 立即重新同步：按 DB 开关把全部技能重新投影到各客户端目录，
/// 顺带清理孤儿软链（指向 SSOT 但 DB 已无记录）。逐条容错。
pub fn resync(env: &SkillEnv, store: &Store) -> Result<Vec<serde_json::Value>, String> {
    let _guard = write_guard();
    env.ensure_dirs()?;
    let method = SyncMethod::from_store(store);
    let rows = store.list_skills().map_err(|e| e.to_string())?;
    let switches = store.list_skill_apps(None).map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for app in SkillApp::ALL {
        let mut failed: Vec<serde_json::Value> = Vec::new();
        let mut synced = 0usize;

        // 1) 清场：目录里挂着但期望不启用的条目。
        let app_dir = env.app_dir(app);
        if app_dir.exists() {
            let wanted: HashSet<String> = rows
                .iter()
                .filter(|row| {
                    switches
                        .iter()
                        .any(|s| s.skill_dir == row.dir && s.app_id == app.as_str() && s.enabled)
                })
                .map(|row| row.dir.clone())
                .collect();
            for entry in fs::read_dir(&app_dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || wanted.contains(&name) {
                    continue;
                }
                let path = entry.path();
                let known = rows.iter().any(|r| r.dir == name);
                let managed_like = is_symlink_to_ssot(&path, &env.ssot)
                    || (known && classify(env, &name, app) != EntryKind::Foreign);
                if managed_like {
                    if let Err(e) = remove_one(env, &name, app) {
                        failed.push(json!({ "dir": name, "error": e }));
                    }
                }
            }
        }

        // 2) 投影：期望启用的逐条同步。
        for row in &rows {
            let on = switches
                .iter()
                .any(|s| s.skill_dir == row.dir && s.app_id == app.as_str() && s.enabled);
            if !on {
                continue;
            }
            match sync_one(env, &row.dir, app, method) {
                Ok(mode) => {
                    let _ = store.set_skill_app(&row.dir, app.as_str(), true, mode);
                    synced += 1;
                }
                Err(e) => {
                    tracing::warn!("重新同步 {}/{} 失败: {e}", row.dir, app.as_str());
                    failed.push(json!({ "dir": row.dir, "error": e }));
                }
            }
        }

        out.push(json!({
            "app": app.as_str(),
            "ok": failed.is_empty(),
            "synced": synced,
            "failed": failed,
        }));
    }
    Ok(out)
}

/// 卸载：备份 → 清各客户端副本 → 删 SSOT → 删 DB 行。
/// 任一客户端存在「他人资产」同名条目时整体中止（宁停勿错删）。
/// 返回备份路径（无可备份内容时为 None）。
pub fn uninstall(env: &SkillEnv, store: &Store, dir: &str) -> Result<Option<String>, String> {
    let _guard = write_guard();
    let dir = require_valid_directory(dir)?;
    let rec = store
        .get_skill(&dir)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("技能不存在: {dir}"))?;

    // 预检：有任何一份「他人资产」就先整体拒绝，不做半程删除。
    // OrphanCopy（SSOT 源已丢的副本）不阻断：上面的备份步骤会以它为源留底，
    // 留底后才删。
    let mut orphan_copies: Vec<(SkillApp, PathBuf)> = Vec::new();
    for app in SkillApp::ALL {
        match classify(env, &dir, app) {
            EntryKind::Foreign => {
                return Err(format!(
                    "{:?} 中存在同名且内容不同的技能，请先人工处置后再卸载: {dir}",
                    app
                ))
            }
            EntryKind::OrphanCopy => {
                orphan_copies.push((app, env.app_dir(app).join(&dir)));
            }
            _ => {}
        }
    }

    let backup = create_uninstall_backup(env, store, &rec)?;

    for app in SkillApp::ALL {
        if orphan_copies.iter().any(|(a, _)| *a == app) {
            continue; // 下面单独清理（已备份留底）
        }
        remove_one(env, &dir, app)?;
    }
    // 无源副本已在备份里留底，直接清理。
    for (app, path) in &orphan_copies {
        remove_path(path).map_err(|e| format!("清理 {:?} 残留副本失败: {e}", app))?;
    }

    let ssot_path = env.ssot.join(&dir);
    if ssot_path.is_dir() && !is_symlink(&ssot_path) {
        fs::remove_dir_all(&ssot_path).map_err(|e| format!("删除 SSOT 失败: {e}"))?;
    }
    store.delete_skill(&dir).map_err(|e| e.to_string())?;
    Ok(backup.map(|p| p.to_string_lossy().to_string()))
}

fn create_uninstall_backup(
    env: &SkillEnv,
    store: &Store,
    rec: &SkillRecord,
) -> Result<Option<PathBuf>, String> {
    // 备份源：优先 SSOT；SSOT 缺失时退回第一个存在的客户端副本。
    let source = if env.ssot.join(&rec.dir).is_dir() {
        env.ssot.join(&rec.dir)
    } else {
        SkillApp::ALL
            .iter()
            .map(|a| env.app_dir(*a).join(&rec.dir))
            .find(|p| p.is_dir())
            .ok_or_else(|| format!("技能 {rec_dir} 无任何可备份的目录", rec_dir = rec.dir))?
    };

    fs::create_dir_all(&env.backups).map_err(|e| e.to_string())?;
    let stamp = timestamp_slug();
    let slug = sanitize_backup_segment(&rec.dir);
    let mut backup_path = env.backups.join(format!("{stamp}_{slug}"));
    let mut counter = 1;
    while backup_path.exists() {
        backup_path = env.backups.join(format!("{stamp}_{slug}_{counter}"));
        counter += 1;
    }

    let write = || -> Result<(), String> {
        copy_dir_recursive(&source, &backup_path.join("skill"))?;
        Ok(())
    };
    if let Err(e) = write() {
        let _ = fs::remove_dir_all(&backup_path);
        return Err(e);
    }

    let meta = BackupMeta {
        skill: rec.clone(),
        apps: store
            .list_skill_apps(Some(&rec.dir))
            .map_err(|e| e.to_string())?,
        created_at: now_ms(),
        source_path: source.to_string_lossy().to_string(),
    };
    let meta_json = serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?;
    if let Err(e) = fs::write(backup_path.join("meta.json"), meta_json) {
        let _ = fs::remove_dir_all(&backup_path);
        return Err(format!("写备份元数据失败: {e}"));
    }

    if let Err(e) = cleanup_old_backups(&env.backups) {
        tracing::warn!("清理旧技能备份失败: {e}");
    }
    Ok(Some(backup_path))
}

fn timestamp_slug() -> String {
    // UTC 秒 → yyyyMMdd_HHmmss（备份目录排序用，无需 chrono）。
    let secs = now_ms() / 1000;
    let (y, mo, d, h, mi, s) = civil_from_unix(secs);
    format!("{y:04}{mo:02}{d:02}_{h:02}{mi:02}{s:02}")
}

/// Howard Hinnant civil_from_days 算法（days → y-m-d h:m:s，UTC）。
fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 1 } else { mp - 11 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        (rem % 3600 / 60) as u32,
        (rem % 60) as u32,
    )
}

fn cleanup_old_backups(dir: &Path) -> Result<(), String> {
    let mut entries: Vec<(PathBuf, SystemTime)> = fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let md = entry.metadata().ok()?;
            md.is_dir().then_some((path, md.modified().ok()?))
        })
        .collect();
    if entries.len() <= BACKUP_RETAIN_COUNT {
        return Ok(());
    }
    entries.sort_by_key(|(_, m)| *m);
    let excess = entries.len() - BACKUP_RETAIN_COUNT;
    for (path, _) in entries.into_iter().take(excess) {
        fs::remove_dir_all(&path).map_err(|e| format!("删除旧备份失败: {e}"))?;
    }
    Ok(())
}

/// 立即备份当前技能库（BUG-6，用户 2026-11-16）：对每个已纳管技能做一份快照，
/// 落点与 meta.json 形状和卸载备份完全一致，现有列表/还原直接可用。
/// 返回成功备份的技能数；单个技能备份失败不中断其余（汇总进日志）。
pub fn backup_now(env: &SkillEnv, store: &Store) -> Result<usize, String> {
    env.ensure_dirs()?;
    let recs = store.list_skills().map_err(|e| e.to_string())?;
    let mut done = 0usize;
    for rec in &recs {
        match create_uninstall_backup(env, store, rec) {
            Ok(Some(_)) => done += 1,
            Ok(None) => {}
            Err(e) => tracing::warn!("技能 {} 手动备份失败: {e}", rec.dir),
        }
    }
    Ok(done)
}

/// 备份列表（按时间倒序）。
pub fn list_backups(env: &SkillEnv) -> Result<Vec<BackupEntry>, String> {
    env.ensure_dirs()?;
    let mut out = Vec::new();
    for entry in fs::read_dir(&env.backups).map_err(|e| e.to_string())? {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("读备份目录项失败: {e}");
                continue;
            }
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match read_backup_meta(&path) {
            Ok(meta) => out.push(BackupEntry {
                backup_id: entry.file_name().to_string_lossy().to_string(),
                created_at: meta.created_at,
                dir: meta.skill.dir.clone(),
                name: meta.skill.name.clone(),
                source_path: meta.source_path,
                backup_path: path.to_string_lossy().to_string(),
            }),
            Err(e) => tracing::warn!("解析技能备份失败 {}: {e}", path.display()),
        }
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    Ok(out)
}

fn read_backup_meta(backup_path: &Path) -> Result<BackupMeta, String> {
    let meta_path = backup_path.join("meta.json");
    let content = fs::read_to_string(&meta_path)
        .map_err(|e| format!("读 {} 失败: {e}", meta_path.display()))?;
    serde_json::from_str(&content).map_err(|e| format!("解析 {} 失败: {e}", meta_path.display()))
}

pub fn delete_backup(env: &SkillEnv, backup_id: &str) -> Result<(), String> {
    let path = backup_path_for_id(env, backup_id)?;
    if !path.is_dir() {
        return Err(format!("备份不存在: {backup_id}"));
    }
    fs::remove_dir_all(&path).map_err(|e| format!("删除备份失败: {e}"))
}

/// 从备份还原：回到 SSOT + DB，并按备份时的开关重新投影到各客户端。
/// 同名技能仍存在时拒绝（先卸再还）。
pub fn restore_backup(env: &SkillEnv, store: &Store, backup_id: &str) -> Result<SkillView, String> {
    let _guard = write_guard();
    env.ensure_dirs()?;
    let backup_path = backup_path_for_id(env, backup_id)?;
    let meta = read_backup_meta(&backup_path)?;
    let skill_src = backup_path.join("skill");
    if !skill_src.join("SKILL.md").is_file() {
        return Err(format!("备份不完整（缺 SKILL.md）: {backup_id}"));
    }
    // meta.json 可能是手工放置/篡改的，directory 必须先过校验再 join。
    let dir = require_valid_directory(&meta.skill.dir)?;
    if store.get_skill(&dir).map_err(|e| e.to_string())?.is_some() || env.ssot.join(&dir).exists() {
        return Err(format!("技能已存在，请先卸载当前的: {dir}"));
    }

    let restore_path = env.ssot.join(&dir);
    copy_dir_recursive(&skill_src, &restore_path)?;

    let rec = SkillRecord {
        dir: dir.clone(),
        updated_at: now_ms(),
        // 重算内容哈希（备份内旧哈希可能来自更早的内容）。
        version: compute_dir_hash(&restore_path).ok(),
        ..meta.skill.clone()
    };
    if let Err(e) = store.upsert_skill(&rec) {
        let _ = fs::remove_dir_all(&restore_path);
        return Err(e.to_string());
    }

    // 按备份时的开关恢复投影；单端失败不回滚整体（记录警告，UI 可重同步）。
    for sw in &meta.apps {
        if !sw.enabled {
            continue;
        }
        let Ok(app) = SkillApp::parse(&sw.app_id) else {
            continue;
        };
        match sync_one(env, &dir, app, SyncMethod::from_store(store)) {
            Ok(mode) => {
                let _ = store.set_skill_app(&dir, app.as_str(), true, mode);
            }
            Err(e) => tracing::warn!("还原 {dir} 到 {:?} 失败: {e}", app),
        }
    }

    drop(_guard);
    list(env, store)?
        .into_iter()
        .find(|v| v.dir == dir)
        .ok_or_else(|| format!("还原后未能找到技能: {dir}"))
}

/// 全局分发方式（GET）。
pub fn get_sync_method(store: &Store) -> String {
    SyncMethod::from_store(store).as_str().to_string()
}

/// 全局分发方式（PUT）。
pub fn set_sync_method(store: &Store, method: &str) -> Result<String, String> {
    let m = SyncMethod::parse(method)?;
    store
        .set_setting(SYNC_METHOD_SETTING, m.as_str())
        .map_err(|e| e.to_string())?;
    Ok(m.as_str().to_string())
}

// ======================= 切片 B：归档安装 / ZIP 导入 / 未纳管 / 仓库 =======================

use std::io::{Cursor, Read, Seek, Write};
use std::time::Duration;

const MAX_ARCHIVE_ENTRIES: usize = 30_000;
/// 解压后总字节预算（含目录/inode 折算，兜压缩炸弹）。
const MAX_ARCHIVE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
/// 压缩体下载上限：解压预算在 ZipArchive 建立时才生效，下载这步要自己的闸。
const MAX_DOWNLOAD_BYTES: u64 = 128 * 1024 * 1024;
/// symlink 目标就是一条路径，几十字节足够；4 KiB 是宽松上限（防 symlink 内存炸弹）。
const MAX_SYMLINK_TARGET_BYTES: u64 = 4 * 1024;
/// 每个新建目录/文件至少按一个磁盘块计费（空文件也吃 inode）。
const ENTRY_BLOCK_COST: u64 = 4096;

/// settings 键：GitHub 下载加速前缀（如 `https://gh-proxy.com/`；空 = 直连）。
pub const DOWNLOAD_PROXY_SETTING: &str = "skills.download_proxy";

fn charge_archive_budget(total: &mut u64, amount: u64) -> Result<(), String> {
    if total.saturating_add(amount) > MAX_ARCHIVE_TOTAL_BYTES {
        return Err(format!(
            "归档解压超出总量上限（{} MB），拒绝解压可疑压缩包",
            MAX_ARCHIVE_TOTAL_BYTES / 1024 / 1024
        ));
    }
    *total += amount;
    Ok(())
}

fn copy_entry_within_budget<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    total: &mut u64,
) -> Result<(), String> {
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| format!("解压读取失败: {e}"))?;
        if read == 0 {
            return Ok(());
        }
        charge_archive_budget(total, read as u64)?;
        writer
            .write_all(&buffer[..read])
            .map_err(|e| format!("解压写入失败: {e}"))?;
    }
}

/// 把单个文件条目写到 dest 并计入预算；不足一个磁盘块按一块补齐。
fn write_file_within_budget<R: Read>(
    reader: &mut R,
    dest: &Path,
    total: &mut u64,
) -> Result<(), String> {
    let mut writer = fs::File::create(dest).map_err(|e| format!("创建文件失败: {e}"))?;
    let before = *total;
    copy_entry_within_budget(reader, &mut writer, total)?;
    let written = total.saturating_sub(before);
    if written < ENTRY_BLOCK_COST {
        charge_archive_budget(total, ENTRY_BLOCK_COST - written)?;
    }
    Ok(())
}

/// 读取 symlink 条目声明的目标路径；超长/非 UTF-8 一律 None 跳过——
/// 合法 symlink 目标是一条路径，别的形状都可能是投毒。
fn read_symlink_target<R: Read>(reader: &mut R, total: &mut u64) -> Result<Option<String>, String> {
    let mut raw = Vec::new();
    // 多读一个字节，用来区分「正好到上限」和「被截断」。
    let mut limited = std::io::Read::take(reader, MAX_SYMLINK_TARGET_BYTES + 1);
    std::io::Read::read_to_end(&mut limited, &mut raw).map_err(|e| e.to_string())?;
    if raw.len() as u64 > MAX_SYMLINK_TARGET_BYTES {
        return Ok(None);
    }
    charge_archive_budget(total, raw.len() as u64)?;
    Ok(String::from_utf8(raw).ok().map(|t| t.trim().to_string()))
}

/// 建目录并按实际新建层数计费（`a/a/…/f` 能隐式造几百层）。
fn create_dir_all_within_budget(path: &Path, total: &mut u64) -> Result<(), String> {
    let missing = path.ancestors().take_while(|p| !p.exists()).count() as u64;
    if missing > 0 {
        charge_archive_budget(total, missing * ENTRY_BLOCK_COST)?;
    }
    fs::create_dir_all(path).map_err(|e| format!("创建目录失败: {e}"))
}

fn copy_dir_within_budget(src: &Path, dest: &Path, total: &mut u64) -> Result<(), String> {
    create_dir_all_within_budget(dest, total)?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let dest_path = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir_within_budget(&path, &dest_path, total)?;
        } else {
            let mut reader = fs::File::open(&path).map_err(|e| e.to_string())?;
            write_file_within_budget(&mut reader, &dest_path, total)?;
        }
    }
    Ok(())
}

/// 解压归档到 dest。`strip_root=true` 用于 GitHub codeload 归档（统一带一层
/// `<repo>-<branch>/` 根目录）；本地 ZIP 不剥。两道穿越防护原样移植：
/// `enclosed_name()` 挡绝对路径/净深度为负；剥根会花掉一级深度预算，
/// 所以对实际使用的相对路径再验一次 ParentDir。
fn extract_zip_archive<R: Read + Seek>(
    mut archive: zip::ZipArchive<R>,
    dest: &Path,
    strip_root: bool,
) -> Result<(), String> {
    if archive.is_empty() {
        return Err("空归档".to_string());
    }
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "归档条目数超限: {} > {MAX_ARCHIVE_ENTRIES}",
            archive.len()
        ));
    }
    let root_name = if strip_root {
        let first = archive.by_index(0).map_err(|e| e.to_string())?;
        first.name().split('/').next().unwrap_or("").to_string()
    } else {
        String::new()
    };

    let mut total: u64 = 0;
    let mut symlinks: Vec<(PathBuf, String)> = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(safe_path) = file.enclosed_name() else {
            tracing::warn!("跳过不安全压缩包条目: {}", file.name());
            continue;
        };
        let relative: &Path = if strip_root {
            match safe_path.strip_prefix(&root_name) {
                Ok(r) => r,
                Err(_) => continue,
            }
        } else {
            safe_path.as_ref()
        };
        if relative
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            tracing::warn!("跳过越界压缩包条目: {}", file.name());
            continue;
        }
        if relative.as_os_str().is_empty() {
            continue;
        }
        let outpath = dest.join(relative);
        if file.is_symlink() {
            let Some(target) = read_symlink_target(&mut file, &mut total)? else {
                tracing::warn!("跳过非法 symlink 条目: {}", file.name());
                continue;
            };
            symlinks.push((outpath, target));
        } else if file.is_dir() {
            create_dir_all_within_budget(&outpath, &mut total)?;
        } else {
            if let Some(parent) = outpath.parent() {
                create_dir_all_within_budget(parent, &mut total)?;
            }
            write_file_within_budget(&mut file, &outpath, &mut total)?;
        }
    }

    resolve_symlinks_in_dir(dest, &symlinks, &mut total)?;
    Ok(())
}

/// 把 symlink 条目物化为真实内容（自包含、跨平台），共用解压字节预算。
/// 防护：目标必须落在解压根内；目标不得包含链接自身（`link -> ..` 解析后
/// 恰是根本身，递归复制会逐层自我膨胀）。
fn resolve_symlinks_in_dir(
    base: &Path,
    symlinks: &[(PathBuf, String)],
    total: &mut u64,
) -> Result<(), String> {
    if symlinks.is_empty() {
        return Ok(());
    }
    let canonical_base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    for (link_path, target) in symlinks {
        let parent = link_path.parent().unwrap_or(base);
        let resolved = match parent.join(target).canonicalize() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!(
                    "symlink 目标不存在，跳过: {} -> {}",
                    link_path.display(),
                    target
                );
                continue;
            }
        };
        if !resolved.starts_with(&canonical_base) {
            tracing::warn!(
                "symlink 目标越出解压根，跳过: {} -> {}",
                link_path.display(),
                target
            );
            continue;
        }
        // 自包含检查必须在规范形式上做（link_path 可能带未消解的 `..`）。
        let canonical_link = match parent.canonicalize() {
            Ok(p) => match link_path.file_name() {
                Some(n) => p.join(n),
                None => p,
            },
            Err(_) => link_path.clone(),
        };
        if canonical_link.starts_with(&resolved) {
            tracing::warn!(
                "symlink 目标包含链接自身，跳过: {} -> {}",
                link_path.display(),
                target
            );
            continue;
        }
        if resolved.is_dir() {
            copy_dir_within_budget(&resolved, link_path, total)?;
        } else if resolved.is_file() {
            if let Some(p) = link_path.parent() {
                create_dir_all_within_budget(p, total)?;
            }
            let mut reader = fs::File::open(&resolved).map_err(|e| e.to_string())?;
            write_file_within_budget(&mut reader, link_path, total)?;
        }
    }
    Ok(())
}

/// 递归找出所有含 SKILL.md 的目录（命中即止，不下钻技能内部）。
fn scan_skills_in_dir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    scan_skills_recursive(dir, &mut out)?;
    Ok(out)
}

fn scan_skills_recursive(current: &Path, results: &mut Vec<PathBuf>) -> Result<(), String> {
    if current.join("SKILL.md").is_file() {
        results.push(current.to_path_buf());
        return Ok(());
    }
    if let Ok(entries) = fs::read_dir(current) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() && !name.starts_with('.') {
                scan_skills_recursive(&path, results)?;
            }
        }
    }
    Ok(())
}

/// 在整仓里找名为 `leaf` 且含 `SKILL.md` 的目录，取最浅的一个。
///
/// 为什么需要：skills.sh 搜到的条目只给技能名，仓库里却常把它嵌在子目录里
/// （实测 nextlevelbuilder/ui-ux-pro-max-skill 住在 `.claude/skills/ui-ux-pro-max`）。
/// 与 `scan_skills_recursive` 的差别：那边特意跳过点目录，这里必须看得见
/// `.claude/skills/`，只排掉 `.git`。
fn find_skill_dir_by_name(root: &Path, leaf: &str) -> Option<PathBuf> {
    let mut best: Option<PathBuf> = None;
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 6 {
            continue;
        }
        if dir.file_name().is_some_and(|n| n == leaf) && dir.join("SKILL.md").is_file() {
            if best
                .as_ref()
                .is_none_or(|b| b.components().count() > dir.components().count())
            {
                best = Some(dir.clone());
            }
            continue;
        }
        if depth >= 6 {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if p.is_dir() && name != ".git" {
                    stack.push((p, depth + 1));
                }
            }
        }
    }
    best
}

/// 校验并规范化技能源路径（允许多级目录），拒绝路径穿越和绝对路径。
fn sanitize_skill_source_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut normalized = PathBuf::new();
    let mut has_component = false;
    for component in Path::new(trimmed).components() {
        match component {
            Component::Normal(name) => {
                let segment = name.to_string_lossy().trim().to_string();
                if segment.is_empty() || segment == "." || segment == ".." {
                    return None;
                }
                normalized.push(segment);
                has_component = true;
            }
            _ => return None,
        }
    }
    has_component.then_some(normalized)
}

fn is_valid_github_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= 39
        && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn is_valid_github_repo_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// git 分支名：逐段白名单；额外禁 `#`（URL fragment）与 `%`（百分号编码绕字符检查）。
fn is_valid_git_branch(branch: &str) -> bool {
    // 空串/HEAD 是「用默认分支」哨兵，永远不会拼进 URL。
    if branch.is_empty() || branch.eq_ignore_ascii_case("HEAD") {
        return true;
    }
    if branch.len() > 255 {
        return false;
    }
    if branch.starts_with('/') || branch.ends_with('/') || branch.contains("//") {
        return false;
    }
    if branch.contains("@{") {
        return false;
    }
    if branch
        .chars()
        .any(|c| c.is_ascii_control() || " ~^:?*[\\#%".contains(c))
    {
        return false;
    }
    branch.split('/').all(|segment| {
        !segment.is_empty()
            && !segment.starts_with('.')
            && !segment.ends_with('.')
            && !segment.ends_with(".lock")
    })
}

fn validate_repo_ref(owner: &str, name: &str, branch: &str) -> Result<(), String> {
    if !is_valid_github_owner(owner) || !is_valid_github_repo_name(name) {
        return Err(format!("非法仓库坐标: {owner}/{name}"));
    }
    if !is_valid_git_branch(branch) {
        return Err(format!("非法分支名: {branch}"));
    }
    Ok(())
}

pub fn get_download_proxy(store: &Store) -> String {
    store
        .get_setting(DOWNLOAD_PROXY_SETTING)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// 下载加速前缀：空 = 直连；否则必须是 http(s) URL 前缀（如 `https://gh-proxy.com/`）。
pub fn set_download_proxy(store: &Store, proxy: &str) -> Result<String, String> {
    let p = proxy.trim().trim_end_matches('/').to_string();
    if !p.is_empty()
        && (!(p.starts_with("https://") || p.starts_with("http://"))
            || p.chars().any(|c| c.is_whitespace() || c.is_ascii_control()))
    {
        return Err(format!("非法下载前缀: {proxy}"));
    }
    store
        .set_setting(DOWNLOAD_PROXY_SETTING, &p)
        .map_err(|e| e.to_string())?;
    Ok(p)
}

/// 拼 GitHub 归档 URL（可选镜像前缀）并做出口断言：无论前缀怎么拼，
/// 最终 URL 必须仍然指向 github.com/{owner}/{name}/archive/refs/heads/*.zip。
fn build_archive_url(proxy: &str, owner: &str, name: &str, branch: &str) -> Result<String, String> {
    let github = format!("https://github.com/{owner}/{name}/archive/refs/heads/{branch}.zip");
    let url = if proxy.is_empty() {
        github
    } else {
        format!("{}/{}", proxy.trim_end_matches('/'), github)
    };
    let expect = format!("github.com/{owner}/{name}/archive/refs/heads/");
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || !url.contains(&expect)
        || !url.ends_with(".zip")
    {
        return Err(format!("归档 URL 校验失败: {owner}/{name}@{branch}"));
    }
    Ok(url)
}

/// 逐块下载并按实际收到的字节限流（Content-Length 可以撒谎，不能只信它）。
async fn download_bytes(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("下载失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载失败: HTTP {}", resp.status().as_u16()));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("下载中断: {e}"))? {
        if body.len().saturating_add(chunk.len()) as u64 > MAX_DOWNLOAD_BYTES {
            return Err(format!(
                "归档下载超出上限（{} MB）",
                MAX_DOWNLOAD_BYTES / 1024 / 1024
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// 下载仓库归档（分支候选：指定分支 → main → master），解压到临时目录。
async fn download_repo_zip(
    store: &Store,
    owner: &str,
    name: &str,
    branch: &str,
) -> Result<(tempfile::TempDir, String), String> {
    let proxy = get_download_proxy(store);
    let mut branches: Vec<String> = Vec::new();
    if !branch.is_empty() && !branch.eq_ignore_ascii_case("HEAD") {
        branches.push(branch.to_string());
    }
    for b in ["main", "master"] {
        if !branches.iter().any(|x| x == b) {
            branches.push(b.to_string());
        }
    }

    let temp = tempfile::tempdir().map_err(|e| format!("创建临时目录失败: {e}"))?;
    let mut last_err = String::from("所有分支下载失败");
    for b in &branches {
        let url = build_archive_url(&proxy, owner, name, b)?;
        let bytes = match download_bytes(&url).await {
            Ok(v) => v,
            Err(e) => {
                last_err = e;
                continue;
            }
        };
        let archive = match zip::ZipArchive::new(Cursor::new(bytes)) {
            Ok(a) => a,
            Err(e) => {
                last_err = format!("归档读取失败: {e}");
                continue;
            }
        };
        // 每个分支各自重算预算：失败后清掉残留，避免 N 个候选分支叠出 N 倍落盘量。
        let path = temp.path().to_path_buf();
        if let Err(e) = extract_zip_archive(archive, &path, true) {
            last_err = e;
            let _ = fs::remove_dir_all(&path);
            let _ = fs::create_dir_all(&path);
            continue;
        }
        return Ok((temp, b.clone()));
    }
    Err(last_err)
}

/// 从 git 仓库安装一个技能。`path` 为仓库内技能子目录（省略时：仓库根含
/// SKILL.md 即整仓安装；恰好只有一个技能目录则自动选中；多个则报错列候选）。
pub async fn install_from_repo(
    env: &SkillEnv,
    store: &Store,
    owner: &str,
    name: &str,
    branch: &str,
    path: Option<&str>,
) -> Result<SkillView, String> {
    env.ensure_dirs()?;
    validate_repo_ref(owner, name, branch)?;
    if let Some(p) = path {
        if sanitize_skill_source_path(p).is_none() {
            return Err(format!("非法技能子路径: {p}"));
        }
    }

    // 下载在网络侧完成，不持全局写锁（std 守卫不能跨 await）。
    let (temp, used_branch) = download_repo_zip(store, owner, name, branch).await?;
    let temp_root = temp.path().to_path_buf();

    let source: PathBuf = if let Some(p) = path {
        let rel = sanitize_skill_source_path(p).unwrap();
        let cand = temp_root.join(&rel);
        if cand.is_dir() && cand.join("SKILL.md").is_file() {
            cand
        } else {
            // 直接路径没命中：按目录名在整仓里找同名技能目录（skills.sh 只给名字，
            // 仓库常把技能嵌在 `.claude/skills/`、`skills/` 这类子目录下）。
            let leaf = rel
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            find_skill_dir_by_name(&temp_root, &leaf).ok_or_else(|| {
                let names: Vec<String> = scan_skills_in_dir(&temp_root)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|d| d.file_name().map(|n| n.to_string_lossy().to_string()))
                    .collect();
                format!(
                    "仓库内找不到含 SKILL.md 的目录: {p}{}",
                    if names.is_empty() {
                        String::new()
                    } else {
                        format!("（仓库内可用：{}）", names.join(", "))
                    }
                )
            })?
        }
    } else if temp_root.join("SKILL.md").is_file() {
        temp_root.clone()
    } else {
        let found = scan_skills_in_dir(&temp_root)?;
        match found.len() {
            0 => return Err("仓库中没有含 SKILL.md 的技能目录".to_string()),
            1 => found[0].clone(),
            n => {
                let names: Vec<String> = found
                    .iter()
                    .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .collect();
                return Err(format!(
                    "仓库含 {n} 个技能，请指定 path 之一: {}",
                    names.join(", ")
                ));
            }
        }
    };

    let canonical_temp = temp_root.canonicalize().unwrap_or(temp_root.clone());
    let canon_src = source
        .canonicalize()
        .map_err(|_| "技能源目录不存在".to_string())?;
    if !canon_src.starts_with(&canonical_temp) || !canon_src.is_dir() {
        return Err("技能源目录越出下载根".to_string());
    }

    // 安装名：子目录取其名；整仓根技能用元数据 name 或仓库名兜底。
    let install_name = if canon_src == canonical_temp {
        let meta = parse_skill_metadata(&canon_src.join("SKILL.md"));
        meta.name
            .as_deref()
            .and_then(sanitize_install_name)
            .or_else(|| sanitize_install_name(name))
            .ok_or("无法确定安装目录名")?
    } else {
        let dir_name = canon_src
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        sanitize_install_name(&dir_name).ok_or(format!("非法技能目录名: {dir_name}"))?
    };

    // 落库 + 复制：从这里起不再有 await。
    let _guard = write_guard();
    let dest = env.ssot.join(&install_name);
    if store
        .get_skill(&install_name)
        .map_err(|e| e.to_string())?
        .is_some()
        || dest.exists()
    {
        return Err(format!("技能已存在: {install_name}（如需重装请先卸载）"));
    }
    copy_dir_recursive(&canon_src, &dest)?;
    let (skill_name, description) = read_skill_name_desc(&dest.join("SKILL.md"), &install_name);
    let rec = SkillRecord {
        dir: install_name.clone(),
        name: skill_name,
        description,
        repo: Some(format!("{owner}/{name}")),
        branch: Some(used_branch),
        version: compute_dir_hash(&dest).ok(),
        latest: None,
        size: dir_size(&dest),
        managed: true,
        updated_at: now_ms(),
    };
    if let Err(e) = store.upsert_skill(&rec) {
        let _ = fs::remove_dir_all(&dest);
        return Err(e.to_string());
    }
    drop(temp); // 显式清理下载临时目录。
    list(env, store)?
        .into_iter()
        .find(|v| v.dir == install_name)
        .ok_or_else(|| format!("安装后未找到技能: {install_name}"))
}

/// 从本地 ZIP 导入技能（可含多个）。同名冲突逐个跳过并回报，不整体失败。
pub fn import_zip(
    env: &SkillEnv,
    store: &Store,
    zip_path: &Path,
) -> Result<serde_json::Value, String> {
    let _guard = write_guard();
    env.ensure_dirs()?;
    let file = fs::File::open(zip_path).map_err(|e| format!("打开 ZIP 失败: {e}"))?;
    let archive = zip::ZipArchive::new(file).map_err(|e| format!("读取 ZIP 失败: {e}"))?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    extract_zip_archive(archive, temp.path(), false)?;
    let skill_dirs = scan_skills_in_dir(temp.path())?;
    if skill_dirs.is_empty() {
        return Err("ZIP 中没有含 SKILL.md 的技能目录".to_string());
    }
    let stem = zip_path
        .file_stem()
        .and_then(|s| s.to_str())
        .and_then(sanitize_install_name);
    let existing = store.list_skills().map_err(|e| e.to_string())?;

    let mut installed: Vec<serde_json::Value> = Vec::new();
    let mut skipped: Vec<serde_json::Value> = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();

    for sd in skill_dirs {
        let dir_name = sd
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let meta = parse_skill_metadata(&sd.join("SKILL.md"));
        // SKILL.md 在 ZIP 根时 dir_name 是临时目录名（.tmpXXX），回退元数据名/ZIP 文件名。
        let install_name = if sd == temp.path() || dir_name.starts_with('.') {
            meta.name
                .as_deref()
                .and_then(sanitize_install_name)
                .or(stem.clone())
        } else {
            sanitize_install_name(&dir_name)
                .or_else(|| meta.name.as_deref().and_then(sanitize_install_name))
                .or(stem.clone())
        };
        let install_name = match install_name {
            Some(n) => n,
            None => {
                skipped.push(json!({ "dir": dir_name, "reason": "无法确定合法安装名" }));
                continue;
            }
        };
        let conflict = existing
            .iter()
            .find(|s| s.dir.eq_ignore_ascii_case(&install_name))
            .map(|s| s.name.clone())
            .or_else(|| {
                if env.ssot.join(&install_name).exists() || taken.contains(&install_name) {
                    Some("(磁盘已存在)".to_string())
                } else {
                    None
                }
            });
        if let Some(conflict_name) = conflict {
            skipped.push(json!({ "dir": install_name, "reason": format!("同名技能已存在: {conflict_name}") }));
            continue;
        }
        let dest = env.ssot.join(&install_name);
        copy_dir_recursive(&sd, &dest)?;
        let (skill_name, description) = read_skill_name_desc(&dest.join("SKILL.md"), &install_name);
        let rec = SkillRecord {
            dir: install_name.clone(),
            name: skill_name.clone(),
            description,
            repo: None,
            branch: None,
            version: compute_dir_hash(&dest).ok(),
            latest: None,
            size: dir_size(&dest),
            managed: true,
            updated_at: now_ms(),
        };
        if let Err(e) = store.upsert_skill(&rec) {
            let _ = fs::remove_dir_all(&dest);
            return Err(e.to_string());
        }
        taken.insert(install_name.clone());
        installed.push(json!({ "dir": install_name, "name": skill_name }));
    }
    Ok(json!({ "installed": installed, "skipped": skipped }))
}

/// 未纳管技能：五端目录/SSOT 里有 SKILL.md 但 DB 无记录的目录。
pub fn scan_unmanaged(env: &SkillEnv, store: &Store) -> Result<Vec<serde_json::Value>, String> {
    let managed: HashSet<String> = store
        .list_skills()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| r.dir)
        .collect();
    let mut sources: Vec<(PathBuf, &str)> = SkillApp::ALL
        .iter()
        .map(|a| (env.app_dir(*a), a.as_str()))
        .chain(std::iter::once((env.ssot.clone(), "ssot")))
        .filter(|(p, _)| p.exists())
        .collect();
    // 稳定顺序：按标签排。
    sources.sort_by_key(|(_, label)| label.to_string());

    let mut found: HashMap<String, (String, Option<String>, Vec<String>, String)> = HashMap::new();
    for (base, label) in &sources {
        let Ok(entries) = fs::read_dir(base) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || managed.contains(&name) || !path.is_dir() {
                continue;
            }
            let skill_md = path.join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }
            let (display, desc) = read_skill_name_desc(&skill_md, &name);
            found
                .entry(name.clone())
                .and_modify(|f| f.2.push(label.to_string()))
                .or_insert((
                    display,
                    desc,
                    vec![label.to_string()],
                    path.display().to_string(),
                ));
        }
    }

    let mut out: Vec<serde_json::Value> = found
        .into_iter()
        .map(|(dir, (name, description, found_in, path))| {
            json!({ "dir": dir, "name": name, "description": description, "found_in": found_in, "path": path })
        })
        .collect();
    out.sort_by_key(|v| v["dir"].as_str().unwrap_or_default().to_string());
    Ok(out)
}

/// 认领未纳管技能：移入 SSOT、登记 managed=1，并按 apps 列表建立投影。
pub fn claim(
    env: &SkillEnv,
    store: &Store,
    dir: &str,
    apps: &[String],
) -> Result<SkillView, String> {
    let _guard = write_guard();
    let dir = require_valid_directory(dir)?;
    if store.get_skill(&dir).map_err(|e| e.to_string())?.is_some() {
        return Err(format!("技能已在管理中: {dir}"));
    }
    let ssot_target = env.ssot.join(&dir);
    let source = std::iter::once(ssot_target.clone())
        .chain(SkillApp::ALL.iter().map(|a| env.app_dir(*a).join(&dir)))
        .find(|p| p.is_dir() && p.join("SKILL.md").is_file())
        .ok_or_else(|| format!("未找到可认领的目录: {dir}"))?;
    if source != ssot_target {
        copy_dir_recursive(&source, &ssot_target)?;
    }
    let (name, description) = read_skill_name_desc(&ssot_target.join("SKILL.md"), &dir);
    let rec = SkillRecord {
        dir: dir.clone(),
        name,
        description,
        repo: None,
        branch: None,
        version: compute_dir_hash(&ssot_target).ok(),
        latest: None,
        size: dir_size(&ssot_target),
        managed: true,
        updated_at: now_ms(),
    };
    store.upsert_skill(&rec).map_err(|e| e.to_string())?;
    let method = SyncMethod::from_store(store);
    for app_str in apps {
        let Ok(app) = SkillApp::parse(app_str) else {
            continue;
        };
        match sync_one(env, &dir, app, method) {
            Ok(mode) => {
                let _ = store.set_skill_app(&dir, app.as_str(), true, mode);
            }
            Err(e) => tracing::warn!("认领 {dir} 到 {:?} 失败: {e}", app),
        }
    }
    list(env, store)?
        .into_iter()
        .find(|v| v.dir == dir)
        .ok_or_else(|| format!("认领后未找到技能: {dir}"))
}

// ---------- 仓库管理 ----------

pub fn list_repos(store: &Store) -> Result<Vec<serde_json::Value>, String> {
    let rows = store.list_skill_repos().map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id, "owner": r.owner, "name": r.name,
                "branch": r.branch, "enabled": r.enabled,
            })
        })
        .collect())
}

pub fn add_repo(store: &Store, owner: &str, name: &str, branch: &str) -> Result<(), String> {
    let branch = if branch.is_empty() { "main" } else { branch };
    validate_repo_ref(owner, name, branch)?;
    store
        .add_skill_repo(owner, name, branch)
        .map_err(|e| e.to_string())
}

pub fn delete_repo(store: &Store, id: i64) -> Result<bool, String> {
    store.delete_skill_repo(id).map_err(|e| e.to_string())
}

pub fn set_repo_enabled(store: &Store, id: i64, enabled: bool) -> Result<bool, String> {
    store
        .set_skill_repo_enabled(id, enabled)
        .map_err(|e| e.to_string())
}

/// SKILL.md 原文预览（256KB 截断，UI 展示用）。
pub fn read_doc(env: &SkillEnv, dir: &str) -> Result<String, String> {
    let dir = require_valid_directory(dir)?;
    let path = env.ssot.join(&dir).join("SKILL.md");
    let bytes = fs::read(&path).map_err(|e| format!("读取 SKILL.md 失败: {e}"))?;
    const CAP: usize = 256 * 1024;
    let take = bytes.len().min(CAP);
    let mut out = String::from_utf8_lossy(&bytes[..take]).to_string();
    if bytes.len() > CAP {
        out.push_str("\n…（已截断）");
    }
    Ok(out)
}

// ======================= 切片 C：发现 / 搜索 / 更新 =======================

/// 仓库索引缓存 TTL（毫秒）。到期后首次 discover 重新拉全部启用仓库归档。
const DISCOVER_TTL_MS: i64 = 10 * 60 * 1000;
/// settings 键：发现结果缓存（JSON {ts, skills, failures}）。
const DISCOVER_CACHE_SETTING: &str = "skills.discover.cache";

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 扫描临时目录里的技能并转成可发现条目（相对路径作 directory，支持嵌套）。
fn discoverable_from_temp(
    temp_root: &Path,
    owner: &str,
    name: &str,
    branch: &str,
) -> Result<Vec<serde_json::Value>, String> {
    let mut found = scan_skills_in_dir(temp_root)?;
    found.sort();
    let mut out = Vec::new();
    for dir in found {
        let rel = dir
            .strip_prefix(temp_root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        if rel.is_empty() {
            continue;
        }
        let (skill_name, desc) = read_skill_name_desc(&dir.join("SKILL.md"), &rel);
        out.push(json!({
            "key": format!("{owner}/{name}:{rel}"),
            "name": skill_name,
            "description": desc,
            "directory": rel,
            "repo_owner": owner,
            "repo_name": name,
            "repo_branch": branch,
        }));
    }
    Ok(out)
}

/// 从全部启用仓库拉取归档并编制可发现技能索引（带 TTL 缓存，force 强刷）。
pub async fn discover(store: &Store, force: bool) -> Result<serde_json::Value, String> {
    if !force {
        if let Some(raw) = store
            .get_setting(DISCOVER_CACHE_SETTING)
            .map_err(|e| e.to_string())?
        {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                let ts = v["ts"].as_i64().unwrap_or(0);
                if now_ms() - ts < DISCOVER_TTL_MS {
                    let mut out = v.clone();
                    out["cached"] = json!(true);
                    return Ok(out);
                }
            }
        }
    }

    let repos = store.list_skill_repos().map_err(|e| e.to_string())?;
    let mut skills: Vec<serde_json::Value> = Vec::new();
    let mut failures: Vec<serde_json::Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for repo in repos.iter().filter(|r| r.enabled) {
        match download_repo_zip(store, &repo.owner, &repo.name, &repo.branch).await {
            Ok((temp, used_branch)) => {
                match discoverable_from_temp(temp.path(), &repo.owner, &repo.name, &used_branch) {
                    Ok(items) => {
                        for it in items {
                            let key = it["key"].as_str().unwrap_or_default().to_string();
                            if seen.insert(key) {
                                skills.push(it);
                            }
                        }
                    }
                    Err(e) => failures.push(json!({
                        "owner": repo.owner, "name": repo.name, "error": e,
                    })),
                }
            }
            Err(e) => failures.push(json!({
                "owner": repo.owner, "name": repo.name, "error": e,
            })),
        }
    }

    let out = json!({ "ts": now_ms(), "skills": skills, "failures": failures, "cached": false });
    store
        .set_setting(DISCOVER_CACHE_SETTING, &out.to_string())
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// 搜索 skills.sh 公共目录（来源过滤复用仓库坐标校验，堵非法 owner/repo）。
pub async fn search_skills_sh(
    query: &str,
    limit: usize,
    offset: usize,
) -> Result<serde_json::Value, String> {
    let url = format!(
        "https://skills.sh/api/search?q={}&limit={}&offset={}",
        percent_encode(query),
        limit.clamp(1, 50),
        offset
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("skills.sh 搜索失败: {e}"))?
        .error_for_status()
        .map_err(|e| format!("skills.sh 返回错误: {e}"))?;

    #[derive(Deserialize)]
    struct ShSkill {
        id: String,
        #[serde(rename = "skillId")]
        skill_id: String,
        name: String,
        installs: u64,
        source: String,
    }
    #[derive(Deserialize)]
    struct ShResp {
        query: String,
        skills: Vec<ShSkill>,
        count: usize,
    }

    let body: ShResp = resp
        .json()
        .await
        .map_err(|e| format!("skills.sh 响应解析失败: {e}"))?;
    let skills: Vec<serde_json::Value> = body
        .skills
        .into_iter()
        .filter_map(|s| {
            let parts: Vec<&str> = s.source.splitn(2, '/').collect();
            if parts.len() != 2 {
                return None;
            }
            let (owner, repo) = (parts[0], parts[1]);
            if validate_repo_ref(owner, repo, "main").is_err() {
                return None;
            }
            Some(json!({
                "key": s.id,
                "name": s.name,
                "directory": s.skill_id,
                "repo_owner": owner,
                "repo_name": repo,
                "repo_branch": "main",
                "installs": s.installs,
                "readme_url": format!("https://github.com/{owner}/{repo}"),
            }))
        })
        .collect();
    Ok(json!({ "skills": skills, "total_count": body.count, "query": body.query }))
}

/// 按仓库分组下载远端并比对内容哈希；差异写入行的 latest 字段（UI 显「可更新」）。
/// version 缺失的行现场补算（自愈早期数据）。
pub async fn check_updates(env: &SkillEnv, store: &Store) -> Result<serde_json::Value, String> {
    let rows = store.list_skills().map_err(|e| e.to_string())?;
    let mut groups: std::collections::HashMap<(String, String, String), Vec<SkillRecord>> =
        std::collections::HashMap::new();
    for row in rows {
        let Some(spec) = row.repo.as_deref() else {
            continue;
        };
        let Some((owner, name)) = spec.split_once('/') else {
            continue;
        };
        let branch = row.branch.clone().unwrap_or_else(|| "main".to_string());
        groups
            .entry((owner.to_string(), name.to_string(), branch))
            .or_default()
            .push(row);
    }

    let mut updates: Vec<serde_json::Value> = Vec::new();
    let mut failures: Vec<serde_json::Value> = Vec::new();

    let mut keys: Vec<_> = groups.keys().cloned().collect();
    keys.sort();
    for key in &keys {
        let (owner, name, branch) = (&key.0, &key.1, &key.2);
        let temp = match download_repo_zip(store, owner, name, branch).await {
            Ok((t, _)) => t,
            Err(e) => {
                failures.push(json!({ "owner": owner, "name": name, "error": e }));
                continue;
            }
        };
        let temp_root = temp.path().to_path_buf();
        let remote_dirs = match scan_skills_in_dir(&temp_root) {
            Ok(mut v) => {
                v.sort();
                v
            }
            Err(e) => {
                failures.push(json!({ "owner": owner, "name": name, "error": e }));
                continue;
            }
        };

        for row in groups.get(key).unwrap() {
            let remote = remote_dirs.iter().find(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().as_ref() == row.dir)
                    .unwrap_or(false)
            });
            let Some(remote) = remote else { continue };
            let Ok(remote_hash) = compute_dir_hash(remote) else {
                continue;
            };
            let local_hash = match row.version.as_deref() {
                Some(h) => Some(h.to_string()),
                None => {
                    let h = compute_dir_hash(&env.ssot.join(&row.dir)).ok();
                    if let Some(h) = &h {
                        let mut patched = row.clone();
                        patched.version = Some(h.clone());
                        let _ = store.upsert_skill(&patched);
                    }
                    h
                }
            };
            if local_hash.as_deref() != Some(remote_hash.as_str()) {
                let mut patched = row.clone();
                patched.latest = Some(remote_hash.clone());
                let _ = store.upsert_skill(&patched);
                updates.push(json!({
                    "dir": row.dir, "name": row.name, "repo": format!("{owner}/{name}"),
                    "version": local_hash, "latest": remote_hash,
                }));
            } else if row.latest.is_some() {
                let mut patched = row.clone();
                patched.latest = None;
                let _ = store.upsert_skill(&patched);
            }
        }
    }
    Ok(json!({ "updates": updates, "failures": failures }))
}

/// 更新单个技能：备份旧版 → 远端目录整体替换 SSOT → 刷新哈希 → 重新投影已启用端。
pub async fn update_skill(env: &SkillEnv, store: &Store, dir: &str) -> Result<SkillView, String> {
    env.ensure_dirs()?;
    let dir = require_valid_directory(dir)?;
    let rec = store
        .get_skill(&dir)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("技能不存在: {dir}"))?;
    let spec = rec
        .repo
        .as_deref()
        .ok_or_else(|| format!("{dir} 非仓库安装，无更新通道"))?
        .to_string();
    let (owner, name) = spec
        .split_once('/')
        .ok_or_else(|| format!("非法仓库坐标: {spec}"))?;
    let branch = rec.branch.clone().unwrap_or_else(|| "main".to_string());

    let (temp, _used) = download_repo_zip(store, owner, name, &branch).await?;
    let temp_root = temp.path().to_path_buf();
    let found = scan_skills_in_dir(&temp_root)?;
    let remote = found
        .iter()
        .find(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().as_ref() == dir)
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("仓库中已找不到该技能目录: {dir}"))?;
    let remote_hash = compute_dir_hash(remote)?;

    // 以下无 await，可安全持全局写锁。
    let _guard = write_guard();
    let ssot_dir = env.ssot.join(&dir);
    if rec.version.as_deref() == Some(remote_hash.as_str()) {
        // 已最新：顺手清残留 latest 标记。
        if rec.latest.is_some() {
            let mut patched = rec.clone();
            patched.latest = None;
            store.upsert_skill(&patched).map_err(|e| e.to_string())?;
        }
    } else {
        create_uninstall_backup(env, store, &rec)?;
        if ssot_dir.is_dir() {
            fs::remove_dir_all(&ssot_dir).map_err(|e| format!("清理旧版失败: {e}"))?;
        }
        copy_dir_recursive(remote, &ssot_dir)?;
        let (skill_name, description) = read_skill_name_desc(&ssot_dir.join("SKILL.md"), &dir);
        let patched = SkillRecord {
            name: skill_name,
            description,
            version: Some(remote_hash),
            latest: None,
            size: dir_size(&ssot_dir),
            updated_at: now_ms(),
            ..rec.clone()
        };
        store.upsert_skill(&patched).map_err(|e| e.to_string())?;
        // 复制模式的投影需要重刷（软链天然跟随 SSOT）。
        let method = SyncMethod::from_store(store);
        for sw in store
            .list_skill_apps(Some(&dir))
            .map_err(|e| e.to_string())?
        {
            if !sw.enabled {
                continue;
            }
            if let Ok(app) = SkillApp::parse(&sw.app_id) {
                match sync_one(env, &dir, app, method) {
                    Ok(mode) => {
                        let _ = store.set_skill_app(&dir, app.as_str(), true, mode);
                    }
                    Err(e) => tracing::warn!("更新后重投影 {dir}→{} 失败: {e}", app.as_str()),
                }
            }
        }
    }
    list(env, store)?
        .into_iter()
        .find(|v| v.dir == dir)
        .ok_or_else(|| format!("更新后未找到技能: {dir}"))
}

/// 全部更新：逐个更新并汇总（单个失败不影响其余）。
pub async fn update_all(env: &SkillEnv, store: &Store) -> Result<serde_json::Value, String> {
    let report = check_updates(env, store).await?;
    let mut results = Vec::new();
    for item in report["updates"].as_array().cloned().unwrap_or_default() {
        let dir = item["dir"].as_str().unwrap_or_default().to_string();
        match update_skill(env, store, &dir).await {
            Ok(_) => results.push(json!({ "dir": dir, "ok": true })),
            Err(e) => results.push(json!({ "dir": dir, "ok": false, "error": e })),
        }
    }
    Ok(json!({ "results": results, "checked_failures": report["failures"] }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::sync::Arc;
    use zip::ZipArchive;

    #[test]
    fn finds_nested_skill_dir_by_name() {
        // skills.sh 只给技能名，仓库把技能嵌在 `.claude/skills/` 下（实测
        // nextlevelbuilder/ui-ux-pro-max-skill）：点目录要能找到，浅的优先。
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let deep = root.join(".claude/skills/ui-ux-pro-max");
        let shallow = root.join("skills/ui-ux-pro-max");
        let other = root.join(".claude/skills/other");
        for d in [&deep, &shallow, &other] {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join("SKILL.md"), "# x").unwrap();
        }
        fs::create_dir_all(root.join("skills/nameless")).unwrap();
        assert_eq!(find_skill_dir_by_name(root, "ui-ux-pro-max"), Some(shallow));
        assert_eq!(find_skill_dir_by_name(root, "other"), Some(other));
        // 名字对不上 / 只有目录没有 SKILL.md，都不算命中。
        assert_eq!(find_skill_dir_by_name(root, "nameless"), None);
        assert_eq!(find_skill_dir_by_name(root, "nope"), None);
    }

    #[test]
    fn deveco_shares_the_opencode_shape() {
        // DevEco Code：内核 opencode，技能目录同一形状住在 ~/.config/deveco 下
        assert_eq!(
            SkillApp::DevEco.dir(Path::new("/home/u")),
            PathBuf::from("/home/u/.config/deveco/skills")
        );
        assert_eq!(SkillApp::parse("deveco").unwrap(), SkillApp::DevEco);
        assert!(SkillApp::ALL.contains(&SkillApp::DevEco));
        // Copilot 是网关门面（没有接管配置文件），但技能它真吃：Personal 技能目录
        // ~/.copilot/skills。门面条在技能矩阵里也必须有一列，否则装了 CLI 却纳管不了。
        assert_eq!(
            SkillApp::Copilot.dir(Path::new("/home/u")),
            PathBuf::from("/home/u/.copilot/skills")
        );
        assert_eq!(SkillApp::parse("copilot").unwrap(), SkillApp::Copilot);
        assert!(SkillApp::ALL.contains(&SkillApp::Copilot));
    }

    struct Ctx {
        _tmp: tempfile::TempDir,
        env: SkillEnv,
        store: Arc<Store>,
    }

    fn ctx() -> Ctx {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let data = tmp.path().join("data");
        let env = SkillEnv::with_roots(
            home.clone(),
            data.join("skills"),
            data.join("skill-backups"),
        );
        env.ensure_dirs().unwrap();
        fs::create_dir_all(&home).unwrap();
        let store = Arc::new(Store::in_memory().unwrap());
        Ctx {
            _tmp: tmp,
            env,
            store,
        }
    }

    /// 在 SSOT 放一个技能目录（模拟手工摆放）。
    fn seed_ssot(env: &SkillEnv, dir: &str, body_extra: Option<&str>) -> PathBuf {
        let d = env.ssot.join(dir);
        fs::create_dir_all(&d).unwrap();
        let md = format!("---\nname: {dir} 显示名\ndescription: 测试技能\n---\n正文\n");
        if let Some(extra) = body_extra {
            fs::write(d.join("extra.txt"), extra).unwrap();
        }
        fs::write(d.join("SKILL.md"), md).unwrap();
        d
    }

    fn app_state<'a>(view: &'a SkillView, app: &str) -> &'a SkillAppState {
        view.apps.iter().find(|a| a.app == app).unwrap()
    }

    #[test]
    fn scan_registers_and_self_heals() {
        let c = ctx();
        seed_ssot(&c.env, "demo", None);
        let report = scan(&c.env, &c.store).unwrap();
        assert_eq!(report["added"], 1);
        let views = list(&c.env, &c.store).unwrap();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name, "demo 显示名");
        assert_eq!(views[0].description.as_deref(), Some("测试技能"));
        assert!(!views[0].managed, "手工摆放应为未纳管");

        // 目录被外部删除 → 再扫描自愈清行。
        fs::remove_dir_all(c.env.ssot.join("demo")).unwrap();
        let report = scan(&c.env, &c.store).unwrap();
        assert_eq!(report["removed"], 1);
        assert!(list(&c.env, &c.store).unwrap().is_empty());
    }

    #[test]
    fn enable_creates_symlink_disable_removes() {
        let c = ctx();
        seed_ssot(&c.env, "demo", None);
        scan(&c.env, &c.store).unwrap();

        let st = set_enabled(&c.env, &c.store, "demo", SkillApp::Claude, true).unwrap();
        assert_eq!(st.actual, "linked");
        assert_eq!(st.sync_mode, "symlink");
        let link = c.env.app_dir(SkillApp::Claude).join("demo");
        assert!(is_symlink(&link));

        let views = list(&c.env, &c.store).unwrap();
        assert!(app_state(&views[0], "claude").enabled);

        set_enabled(&c.env, &c.store, "demo", SkillApp::Claude, false).unwrap();
        assert!(!link.exists() && !is_symlink(&link));
        let views = list(&c.env, &c.store).unwrap();
        assert!(!app_state(&views[0], "claude").enabled);
    }

    #[test]
    fn copy_mode_tampered_copy_is_foreign() {
        let c = ctx();
        set_sync_method(&c.store, "copy").unwrap();
        seed_ssot(&c.env, "demo", Some("orig"));
        scan(&c.env, &c.store).unwrap();

        let st = set_enabled(&c.env, &c.store, "demo", SkillApp::Codex, true).unwrap();
        assert_eq!(st.actual, "copied");
        let dest = c.env.app_dir(SkillApp::Codex).join("demo");
        assert!(dest.is_dir() && !is_symlink(&dest));

        // 用户改动了副本内容 → 停用必须拒绝（无法与我方部署区分前科）。
        fs::write(dest.join("extra.txt"), "tampered").unwrap();
        assert!(set_enabled(&c.env, &c.store, "demo", SkillApp::Codex, false).is_err());
        assert!(dest.is_dir());
    }

    #[test]
    fn foreign_same_name_blocks_enable() {
        let c = ctx();
        seed_ssot(&c.env, "demo", None);
        scan(&c.env, &c.store).unwrap();
        // 客户端目录已有用户自有的同名目录（内容不同）。
        let mine = c.env.app_dir(SkillApp::Gemini).join("demo");
        fs::create_dir_all(&mine).unwrap();
        fs::write(mine.join("SKILL.md"), "---\nname: 用户自有\n---\n").unwrap();
        assert!(set_enabled(&c.env, &c.store, "demo", SkillApp::Gemini, true).is_err());
        assert!(mine.is_dir());
    }

    #[test]
    fn uninstall_backs_up_and_restore_roundtrip() {
        let c = ctx();
        seed_ssot(&c.env, "demo", Some("payload"));
        scan(&c.env, &c.store).unwrap();
        set_enabled(&c.env, &c.store, "demo", SkillApp::Claude, true).unwrap();
        set_enabled(&c.env, &c.store, "demo", SkillApp::Pi, true).unwrap();

        let backup = uninstall(&c.env, &c.store, "demo").unwrap();
        assert!(backup.is_some());
        assert!(!c.env.ssot.join("demo").exists());
        assert!(!c.env.app_dir(SkillApp::Claude).join("demo").exists());
        assert!(!c.env.app_dir(SkillApp::Pi).join("demo").exists());
        assert!(c.store.get_skill("demo").unwrap().is_none());

        // 还原：SSOT + DB + 之前的启用端都回来。
        let entries = list_backups(&c.env).unwrap();
        let id = entries[0].backup_id.clone();
        let view = restore_backup(&c.env, &c.store, &id).unwrap();
        assert_eq!(view.dir, "demo");
        assert!(c.env.ssot.join("demo").join("SKILL.md").is_file());
        assert!(c.env.app_dir(SkillApp::Claude).join("demo").exists());
        assert!(app_state(&view, "claude").enabled);
        assert!(app_state(&view, "pi").enabled);

        // 重复还原被拒。
        assert!(restore_backup(&c.env, &c.store, &id).is_err());
    }

    #[test]
    fn invalid_names_are_rejected() {
        let c = ctx();
        seed_ssot(&c.env, "demo", None);
        scan(&c.env, &c.store).unwrap();
        for evil in ["../evil", "a/b", ".hidden", "", " "] {
            assert!(
                set_enabled(&c.env, &c.store, evil, SkillApp::Claude, true).is_err(),
                "should reject {evil:?}"
            );
            assert!(uninstall(&c.env, &c.store, evil).is_err());
        }
        assert!(delete_backup(&c.env, "../x").is_err());
        assert!(restore_backup(&c.env, &c.store, "..%2f").is_err());
    }

    #[test]
    fn broken_symlink_shows_missing() {
        let c = ctx();
        let d = seed_ssot(&c.env, "demo", None);
        scan(&c.env, &c.store).unwrap();
        set_enabled(&c.env, &c.store, "demo", SkillApp::Claude, true).unwrap();
        // SSOT 目录被外部删除 → 软链断掉，列表应显 missing。
        fs::remove_dir_all(&d).unwrap();
        let views = list(&c.env, &c.store).unwrap();
        assert_eq!(app_state(&views[0], "claude").actual, "missing");
    }

    #[test]
    fn resync_prunes_orphan_symlinks_and_keeps_foreign() {
        let c = ctx();
        seed_ssot(&c.env, "ghost", None);
        let claude_dir = c.env.app_dir(SkillApp::Claude);
        fs::create_dir_all(&claude_dir).unwrap();
        // 孤儿软链：指向 SSOT 但 DB 无记录（比如上次崩溃残留）。
        #[cfg(unix)]
        std::os::unix::fs::symlink(c.env.ssot.join("ghost"), claude_dir.join("ghost")).unwrap();
        // 用户自有真实目录：resync 不得触碰。
        fs::create_dir_all(claude_dir.join("mine")).unwrap();

        let outcomes = resync(&c.env, &c.store).unwrap();
        let claude = outcomes.iter().find(|o| o["app"] == "claude").unwrap();
        assert_eq!(claude["ok"], json!(true));
        assert!(!claude_dir.join("ghost").exists());
        assert!(claude_dir.join("mine").is_dir());
    }

    #[test]
    fn orphan_copy_uninstall_backs_up_before_removal() {
        let c = ctx();
        let d = seed_ssot(&c.env, "demo", Some("payload"));
        scan(&c.env, &c.store).unwrap();
        set_sync_method(&c.store, "copy").unwrap();
        set_enabled(&c.env, &c.store, "demo", SkillApp::Claude, true).unwrap();
        // SSOT 源被外部删除 → 客户端副本成为 orphan copy，停用被拒但卸载可备份后清理。
        fs::remove_dir_all(&d).unwrap();
        assert!(set_enabled(&c.env, &c.store, "demo", SkillApp::Claude, false).is_err());
        let backup = uninstall(&c.env, &c.store, "demo").unwrap();
        assert!(backup.is_some());
        assert!(!c.env.app_dir(SkillApp::Claude).join("demo").exists());
        assert!(c.store.get_skill("demo").unwrap().is_none());
    }

    #[test]
    fn sync_method_validation() {
        let c = ctx();
        assert_eq!(get_sync_method(&c.store), "auto");
        assert_eq!(set_sync_method(&c.store, "copy").unwrap(), "copy");
        assert_eq!(get_sync_method(&c.store), "copy");
        assert!(set_sync_method(&c.store, "teleport").is_err());
    }

    // ---------- 切片 B：安装 / 导入 / 未纳管 / 仓库 ----------

    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    /// 内存构造 zip：entries = [(路径, 内容)]。
    fn make_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let opts: SimpleFileOptions =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (path, content) in entries {
            zip.start_file(path.to_string(), opts).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn skill_md(name: &str) -> String {
        format!("---\nname: {name}\ndescription: d\n---\nbody\n")
    }

    #[test]
    fn extract_zip_strips_root_and_blocks_traversal() {
        let bytes = make_zip(&[
            ("repo-main/ok-skill/SKILL.md", &skill_md("ok")),
            ("repo-main/../escape.txt", "evil"),
            ("/abs-evil.txt", "evil"),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        extract_zip_archive(
            ZipArchive::new(Cursor::new(bytes)).unwrap(),
            tmp.path(),
            true,
        )
        .unwrap();
        assert!(tmp.path().join("ok-skill/SKILL.md").is_file());
        assert!(!tmp.path().join("../escape.txt").exists());
        assert!(!tmp.path().join("escape.txt").exists());
        assert!(!tmp.path().join("abs-evil.txt").exists());
    }

    #[tokio::test]
    async fn install_from_repo_via_download_proxy() {
        let c = ctx();
        // mock 镜像：任意 GET 都回同一份归档（前缀拼接模式下 URL 含完整 github 地址）。
        let bytes = make_zip(&[
            ("repo-main/my-skill/SKILL.md", &skill_md("我的技能")),
            ("repo-main/my-skill/body.txt", "hello"),
        ]);
        let app = axum::Router::new().fallback(axum::routing::get(move || {
            let b = bytes.clone();
            async move { b }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        set_download_proxy(&c.store, &format!("http://{addr}/")).unwrap();
        let view = install_from_repo(&c.env, &c.store, "some-owner", "some-repo", "main", None)
            .await
            .unwrap();
        assert_eq!(view.dir, "my-skill");
        assert_eq!(view.name, "我的技能");
        assert!(view.managed);
        assert_eq!(view.repo.as_deref(), Some("some-owner/some-repo"));
        assert!(c.env.ssot.join("my-skill/SKILL.md").is_file());
        assert!(!c.env.app_dir(SkillApp::Claude).join("my-skill").exists());

        // 重复安装被拒（先卸再装）。
        assert!(
            install_from_repo(&c.env, &c.store, "some-owner", "some-repo", "main", None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn install_multi_skill_repo_requires_path() {
        let c = ctx();
        let bytes = make_zip(&[
            ("repo-main/a/SKILL.md", &skill_md("a")),
            ("repo-main/b/SKILL.md", &skill_md("b")),
        ]);
        let app = axum::Router::new().fallback(axum::routing::get(move || {
            let b = bytes.clone();
            async move { b }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        set_download_proxy(&c.store, &format!("http://{addr}/")).unwrap();

        // 多技能仓库不带 path → 报错并列出候选。
        let err = install_from_repo(&c.env, &c.store, "o", "r", "main", None)
            .await
            .unwrap_err();
        assert!(err.contains("a") && err.contains("b"));
        // 带 path 精确安装。
        let view = install_from_repo(&c.env, &c.store, "o", "r", "main", Some("b"))
            .await
            .unwrap();
        assert_eq!(view.dir, "b");
    }

    #[test]
    fn import_zip_installs_and_skips_conflicts() {
        let c = ctx();
        let zip_path = c._tmp.path().join("pack.zip");
        let bytes = make_zip(&[
            ("alpha/SKILL.md", &skill_md("alpha")),
            ("nested/beta/SKILL.md", &skill_md("beta")),
            ("readme.md", "not a skill"),
        ]);
        fs::write(&zip_path, bytes).unwrap();

        let report = import_zip(&c.env, &c.store, &zip_path).unwrap();
        assert_eq!(report["installed"].as_array().unwrap().len(), 2);
        assert!(report["skipped"].as_array().unwrap().is_empty());
        assert!(!c.env.ssot.join("alpha").is_file());
        assert!(c.env.ssot.join("beta/SKILL.md").is_file());

        // 再导一次：全部同名冲突跳过。
        let report = import_zip(&c.env, &c.store, &zip_path).unwrap();
        assert!(report["installed"].as_array().unwrap().is_empty());
        assert_eq!(report["skipped"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn scan_unmanaged_and_claim_flow() {
        let c = ctx();
        // 用户在 Claude 目录自己放了一个技能。
        let mine = c.env.app_dir(SkillApp::Claude).join("solo");
        fs::create_dir_all(&mine).unwrap();
        fs::write(mine.join("SKILL.md"), skill_md("solo")).unwrap();

        let items = scan_unmanaged(&c.env, &c.store).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["dir"], "solo");
        assert!(items[0]["found_in"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "claude"));

        let view = claim(&c.env, &c.store, "solo", &["claude".to_string()]).unwrap();
        assert!(view.managed);
        assert!(c.env.ssot.join("solo/SKILL.md").is_file());
        assert!(app_state(&view, "claude").enabled);
        // 已纳管后不再出现在未纳管列表。
        assert!(scan_unmanaged(&c.env, &c.store).unwrap().is_empty());
    }

    #[test]
    fn repo_ref_and_proxy_validation() {
        assert!(validate_repo_ref("some-owner", "some-repo", "main").is_ok());
        assert!(validate_repo_ref("some-owner", "some-repo", "").is_ok());
        assert!(validate_repo_ref("own/er", "repo", "main").is_err());
        assert!(validate_repo_ref("owner", "repo", "../../../x").is_err());
        assert!(validate_repo_ref("owner", "repo", "feat%2Fx").is_err());

        // URL 出口断言：镜像前缀拼完仍须指向 github.com 预期路径。
        assert!(build_archive_url("", "o", "r", "main")
            .unwrap()
            .starts_with("https://github.com/o/r/archive/refs/heads/main.zip"));
        assert!(build_archive_url("https://gh-proxy.com/", "o", "r", "main")
            .unwrap()
            .contains("gh-proxy.com/https://github.com/o/r/"));

        let c = ctx();
        assert!(set_download_proxy(&c.store, "ftp://nope").is_err());
        assert!(set_download_proxy(&c.store, "https://good.example/").is_ok());
        assert_eq!(get_download_proxy(&c.store), "https://good.example");
        assert!(set_download_proxy(&c.store, "").is_ok());
    }

    #[test]
    fn repo_crud() {
        let c = ctx();
        add_repo(&c.store, "acme", "skills", "main").unwrap();
        add_repo(&c.store, "acme", "more", "").unwrap(); // 空分支回落 main
        let repos = list_repos(&c.store).unwrap();
        assert_eq!(repos.len(), 2);
        let id = repos[0]["id"].as_i64().unwrap();
        assert!(set_repo_enabled(&c.store, id, false).unwrap());
        assert_eq!(list_repos(&c.store).unwrap()[0]["enabled"], json!(false));
        assert!(delete_repo(&c.store, id).unwrap());
        assert!(!delete_repo(&c.store, id).unwrap()); // 二次删除 false
        assert_eq!(list_repos(&c.store).unwrap().len(), 1);
    }
    // ---------- 切片 C：发现 / 更新 ----------

    /// 可换体 mock 归档服务器：任意 GET 返回 shared 里当前的字节。
    async fn mock_archive_server() -> (
        std::net::SocketAddr,
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    ) {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s2 = shared.clone();
        let app = axum::Router::new().fallback(axum::routing::get(move || {
            let s = s2.clone();
            async move { s.lock().unwrap().clone() }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (addr, shared)
    }

    #[tokio::test]
    async fn discover_indexes_enabled_repos_with_cache() {
        let c = ctx();
        let (addr, shared) = mock_archive_server().await;
        *shared.lock().unwrap() = make_zip(&[
            ("repo-main/sk-a/SKILL.md", &skill_md("A")),
            ("repo-main/sub/sk-b/SKILL.md", &skill_md("B")),
        ]);
        set_download_proxy(&c.store, &format!("http://{addr}/")).unwrap();
        add_repo(&c.store, "acme", "skills", "main").unwrap();

        let r = discover(&c.store, false).await.unwrap();
        assert_eq!(r["skills"].as_array().unwrap().len(), 2);
        assert_eq!(r["cached"], json!(false));
        // 嵌套目录以相对路径呈现
        assert!(r["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["directory"] == json!("sub/sk-b")));
        // TTL 内二次调用走缓存；force 强刷。
        assert_eq!(
            discover(&c.store, false).await.unwrap()["cached"],
            json!(true)
        );
        assert_eq!(
            discover(&c.store, true).await.unwrap()["cached"],
            json!(false)
        );
        // 停用仓库不进索引。
        let id = list_repos(&c.store).unwrap()[0]["id"].as_i64().unwrap();
        set_repo_enabled(&c.store, id, false).unwrap();
        assert!(discover(&c.store, true).await.unwrap()["skills"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn check_updates_and_update_skill_flow() {
        let c = ctx();
        let (addr, shared) = mock_archive_server().await;
        set_download_proxy(&c.store, &format!("http://{addr}/")).unwrap();
        *shared.lock().unwrap() = make_zip(&[("repo-main/flavor/SKILL.md", &skill_md("flavor"))]);
        let view = install_from_repo(&c.env, &c.store, "acme", "pack", "main", None)
            .await
            .unwrap();
        assert_eq!(view.dir, "flavor");
        assert!(view.version.is_some(), "安装应写入内容哈希");
        set_enabled(&c.env, &c.store, "flavor", SkillApp::Claude, true).unwrap();

        // 内容未变 → 无更新。
        let r = check_updates(&c.env, &c.store).await.unwrap();
        assert!(r["updates"].as_array().unwrap().is_empty());

        // 远端改版 → 检出可更新。
        *shared.lock().unwrap() =
            make_zip(&[("repo-main/flavor/SKILL.md", &skill_md("flavor-v2"))]);
        let r = check_updates(&c.env, &c.store).await.unwrap();
        assert_eq!(r["updates"].as_array().unwrap().len(), 1);

        // 更新：SSOT 内容/哈希刷新，latest 清空，投影重刷。
        let after = update_skill(&c.env, &c.store, "flavor").await.unwrap();
        assert_ne!(after.version, view.version);
        assert!(after.latest.is_none());
        let md = fs::read_to_string(c.env.ssot.join("flavor/SKILL.md")).unwrap();
        assert!(md.contains("flavor-v2"));

        // 更新后复查无更新。
        let r = check_updates(&c.env, &c.store).await.unwrap();
        assert!(r["updates"].as_array().unwrap().is_empty());
    }

    #[test]
    fn compute_dir_hash_is_order_independent() {
        let t1 = tempfile::tempdir().unwrap();
        fs::create_dir_all(t1.path().join("d")).unwrap();
        fs::write(t1.path().join("d/b.txt"), "b").unwrap();
        fs::write(t1.path().join("d/a.txt"), "a").unwrap();
        fs::write(t1.path().join("d/.hidden"), "x1").unwrap();
        let t2 = tempfile::tempdir().unwrap();
        fs::create_dir_all(t2.path().join("d")).unwrap();
        fs::write(t2.path().join("d/a.txt"), "a").unwrap();
        fs::write(t2.path().join("d/b.txt"), "b").unwrap();
        fs::write(t2.path().join("d/.hidden"), "x2").unwrap();
        assert_eq!(
            compute_dir_hash(&t1.path().join("d")).unwrap(),
            compute_dir_hash(&t2.path().join("d")).unwrap(),
            "创建顺序无关、隐藏文件不参与"
        );
    }

    #[test]
    fn percent_encoding_for_search() {
        assert_eq!(percent_encode("a b&c=d"), "a%20b%26c%3Dd");
        assert_eq!(percent_encode("ok-1.2~x"), "ok-1.2~x");
    }
}
