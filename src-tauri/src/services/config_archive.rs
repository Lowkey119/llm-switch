//! 预设配置归档的一键还原与一键备份：随包内嵌的 zip
//! （`assets/config-archive/llm-switch-config.zip`）打包了 config.yaml /
//! agent-configs / scripts / skills / config_snapshots，还原目标为数据目录
//! （默认 `~/.llm-switch`）。
//!
//! 只有「还原」等于预设是只读的、用户调好的配置没地方存（用户口径：应该有个
//! 「备份当前配置」的入口）。现在 `backup` 把同一份白名单从数据目录打成
//! `<data_dir>/config-archive/llm-switch-config.zip`，**还原优先用它**，没有才回落
//! 内置档；每次备份把上一份滚进 `config-archive/history/<毫秒>.zip` 留 5 份，
//! 免得「备份」本身把好档覆盖了。
//!
//! 安全约定（对齐 takeover::preset 的还原语义）：
//! - 只还原白名单内的顶层条目，zip-slip（`enclosed_name`）与目录外路径一律拒绝；
//! - 覆盖前把现行文件快照到 `<data_dir>/config-restore-backups/<毫秒>/`，误还原可人工找回；
//! - 逐字节相同的文件跳过写入（计入 unchanged），脚本保留可执行位。

use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// 随包内嵌的归档（构建期打进二进制，运行时不依赖仓库路径）。
const ARCHIVE: &[u8] = include_bytes!("../../../assets/config-archive/llm-switch-config.zip");

/// 允许还原的顶层条目（其余一律跳过并如实上报）。
const ALLOWED_TOP: [&str; 5] = [
    "config.yaml",
    "config_snapshots",
    "agent-configs",
    "scripts",
    "skills",
];

/// 测试用：从内存 zip 还原（与 `restore` 同逻辑，归档来源可换）。
#[cfg(test)]
fn restore_from(archive_bytes: Vec<u8>, data_dir: &Path) -> Result<Value, String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(archive_bytes)).map_err(|e| e.to_string())?;
    restore_archive(&mut archive, data_dir)
}

fn restore_archive<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    data_dir: &Path,
) -> Result<Value, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let backup_dir = data_dir
        .join("config-restore-backups")
        .join(stamp.to_string());

    let mut restored = 0usize;
    let mut backed_up = 0usize;
    let mut unchanged = 0usize;
    let mut skipped: Vec<Value> = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("读取归档条目失败: {e}"))?;
        let Some(rel) = file.enclosed_name() else {
            skipped.push(json!({ "path": file.name(), "reason": "不安全路径" }));
            continue;
        };
        // 白名单闸：只认约定好的顶层条目，其余（含隐藏目录）不落地。
        let top_ok = matches!(
            rel.components().next(),
            Some(Component::Normal(t)) if ALLOWED_TOP.iter().any(|a| *a == t.to_string_lossy())
        );
        if !top_ok {
            skipped.push(json!({ "path": rel.display().to_string(), "reason": "不在还原白名单" }));
            continue;
        }
        let target = data_dir.join(&rel);
        if file.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|e| format!("创建目录失败 {}: {e}", target.display()))?;
            continue;
        }
        if !file.is_file() {
            skipped.push(
                json!({ "path": rel.display().to_string(), "reason": "非常规文件（符号链接等）" }),
            );
            continue;
        }

        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| format!("解压失败 {}: {e}", rel.display()))?;

        // 逐字节相同 → 不动不备份，省得备份目录被同一份内容反复灌满。
        if fs::read(&target).map(|cur| cur == bytes).unwrap_or(false) {
            unchanged += 1;
            continue;
        }
        // 覆盖前快照现行文件（存在才有得快照，对齐 preset::restore）。
        if let Ok(cur) = fs::read(&target) {
            let slot = backup_dir.join(&rel);
            if let Some(p) = slot.parent() {
                fs::create_dir_all(p).map_err(|e| format!("创建快照目录失败: {e}"))?;
            }
            fs::write(&slot, cur).map_err(|e| format!("写入快照失败: {e}"))?;
            backed_up += 1;
        }
        if let Some(p) = target.parent() {
            fs::create_dir_all(p).map_err(|e| format!("创建目标目录失败 {}: {e}", p.display()))?;
        }
        let mut out =
            fs::File::create(&target).map_err(|e| format!("写入失败 {}: {e}", target.display()))?;
        out.write_all(&bytes)
            .map_err(|e| format!("写入失败 {}: {e}", target.display()))?;
        // 保留归档里的权限位（scripts/*.sh 的可执行位必须跟着回来）。
        // Unix 权限位在 Windows 上不存在，非 Unix 平台整段为空操作。
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            set_unix_mode(&target, mode);
        }
        restored += 1;
    }

    Ok(json!({
        "restored": restored,
        "backed_up": backed_up,
        "unchanged": unchanged,
        "skipped": skipped,
        "backup_dir": if backed_up > 0 { backup_dir.display().to_string() } else { String::new() },
    }))
}

/// 用户自己备份的归档（存在即优先于内置归档）。
pub fn user_archive_path(data_dir: &Path) -> PathBuf {
    data_dir
        .join("config-archive")
        .join("llm-switch-config.zip")
}

/// 备份历史保留份数（每次备份把上一份滚进 history/，防「备份把好档覆盖了」）。
const HISTORY_KEEP: usize = 5;

fn epoch_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 归档概览：用户备份优先，没有才报内置档。
///
/// 用户档存在但打不开时**不静默回落**内置档——那会让「还原」悄悄还原成随包那份，
/// 与用户预期正好相反；失败必须摆出来（见 `restore`）。
pub fn info(data_dir: &Path) -> Value {
    let p = user_archive_path(data_dir);
    let (bytes, entries, source, updated_ms, path) = match fs::File::open(&p) {
        Ok(f) => match zip::ZipArchive::new(f) {
            Ok(a) => {
                let updated = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .map(|t| {
                        t.duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0)
                    })
                    .unwrap_or(0);
                (
                    fs::metadata(&p).map(|m| m.len()).unwrap_or(0) as usize,
                    a.len(),
                    "user",
                    updated,
                    p.display().to_string(),
                )
            }
            Err(_) => (
                ARCHIVE.len(),
                embedded_entries(),
                "broken",
                0,
                p.display().to_string(),
            ),
        },
        Err(_) => (
            ARCHIVE.len(),
            embedded_entries(),
            "embedded",
            0,
            String::new(),
        ),
    };
    json!({
        "bytes": bytes,
        "entries": entries,
        "top": ALLOWED_TOP,
        "source": source,
        "updated_ms": updated_ms,
        "path": path,
        "history": history_count(data_dir),
    })
}

fn embedded_entries() -> usize {
    zip::ZipArchive::new(std::io::Cursor::new(ARCHIVE))
        .map(|a| a.len())
        .unwrap_or(0)
}

fn history_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("config-archive").join("history")
}

fn history_count(data_dir: &Path) -> usize {
    list_history(data_dir).len()
}

/// 历史档按文件名（毫秒戳）从新到旧。
fn list_history(data_dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(history_dir(data_dir))
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().ends_with(".zip"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    v
}

/// 一键备份：把数据目录里的白名单条目打成用户归档。
///
/// 返回 `{ files, bytes, target, skipped: [{path, reason}], history }`。
pub fn backup(data_dir: &Path) -> Result<Value, String> {
    let buf = std::io::Cursor::new(Vec::<u8>::new());
    let mut w = zip::ZipWriter::new(buf);
    let mut files = 0usize;
    let mut skipped: Vec<Value> = Vec::new();

    for top in ALLOWED_TOP {
        let root = data_dir.join(top);
        match fs::symlink_metadata(&root) {
            // 这个条目本机没有：正常，跳过（备份只如实反映现状）。
            Err(_) => continue,
            Ok(md) if md.file_type().is_symlink() => {
                skipped.push(json!({ "path": top, "reason": "顶层条目是符号链接" }));
                continue;
            }
            Ok(_) => {}
        }
        pack(data_dir, &root, &mut w, &mut files, &mut skipped)?;
    }
    if files == 0 {
        return Err(format!(
            "数据目录 {} 里没有可备份的配置条目（{} 都不存在）",
            data_dir.display(),
            ALLOWED_TOP.join(" / ")
        ));
    }
    let bytes = w
        .finish()
        .map_err(|e| format!("打包失败: {e}"))?
        .into_inner();

    let target = user_archive_path(data_dir);
    if let Some(p) = target.parent() {
        fs::create_dir_all(p).map_err(|e| format!("创建归档目录失败: {e}"))?;
    }
    // 先滚旧档再写新档：写坏也不丢上一份。
    if target.is_file() {
        let hist = history_dir(data_dir);
        fs::create_dir_all(&hist).map_err(|e| format!("创建历史目录失败: {e}"))?;
        let stamp = epoch_ms();
        fs::rename(&target, hist.join(format!("llm-switch-config-{stamp}.zip")))
            .map_err(|e| format!("滚动上一份归档失败: {e}"))?;
        for old in list_history(data_dir).into_iter().skip(HISTORY_KEEP) {
            let _ = fs::remove_file(old);
        }
    }
    fs::write(&target, &bytes).map_err(|e| format!("写入归档失败 {}: {e}", target.display()))?;

    Ok(json!({
        "files": files,
        "bytes": bytes.len(),
        "target": target.display().to_string(),
        "skipped": skipped,
        "history": history_count(data_dir),
    }))
}

/// 递归打包：目录建条目后下钻，符号链接与非常规文件跳过（还原侧同样不认它们）。
/// Unix 权限位读写助手：把 `std::os::unix` 的专有 API 关在 cfg 里，
/// 这样同一份代码在 Windows 上也能编译（那边没有权限位，落为空操作/默认值）。
#[cfg(unix)]
fn set_unix_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_unix_mode(_path: &Path, _mode: u32) {}

#[cfg(unix)]
fn unix_mode_of(md: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    md.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn unix_mode_of(_md: &fs::Metadata) -> u32 {
    0o644
}

fn pack(
    base: &Path,
    path: &Path,
    w: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>,
    files: &mut usize,
    skipped: &mut Vec<Value>,
) -> Result<(), String> {
    let md =
        fs::symlink_metadata(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let rel = path
        .strip_prefix(base)
        .map_err(|e| format!("相对路径失败: {e}"))?
        .to_string_lossy()
        .replace('\\', "/");
    let mode = unix_mode_of(&md);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(mode);
    if md.is_dir() {
        if !rel.is_empty() {
            w.add_directory(format!("{rel}/"), opts)
                .map_err(|e| format!("写入目录条目失败 {rel}: {e}"))?;
        }
        for entry in fs::read_dir(path).map_err(|e| format!("读取目录失败 {rel}: {e}"))? {
            let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
            pack(base, &entry.path(), w, files, skipped)?;
        }
        return Ok(());
    }
    if !md.is_file() {
        skipped.push(json!({ "path": rel.as_str(), "reason": "非常规文件（符号链接等）" }));
        return Ok(());
    }
    let bytes = fs::read(path).map_err(|e| format!("读取失败 {rel}: {e}"))?;
    w.start_file(&rel, opts)
        .map_err(|e| format!("写入条目失败 {rel}: {e}"))?;
    w.write_all(&bytes)
        .map_err(|e| format!("写入失败 {rel}: {e}"))?;
    *files += 1;
    Ok(())
}

/// 一键还原：用户备份优先，没有才用内置归档；覆盖前逐文件快照现行版本。
/// 返回 `{ restored, backed_up, unchanged, skipped: [{path, reason}], source }`。
pub fn restore(data_dir: &Path) -> Result<Value, String> {
    let p = user_archive_path(data_dir);
    let mut report = match fs::File::open(&p) {
        Ok(f) => {
            let mut a = zip::ZipArchive::new(f).map_err(|e| {
                format!(
                    "读取自己的备份归档失败（{p}）：{e}——删掉它可回落到内置归档，或重新备份",
                    p = p.display()
                )
            })?;
            let mut r = restore_archive(&mut a, data_dir)?;
            r["source"] = json!("user");
            r
        }
        Err(_) => {
            let mut a = zip::ZipArchive::new(std::io::Cursor::new(ARCHIVE))
                .map_err(|e| format!("读取内置归档失败: {e}"))?;
            let mut r = restore_archive(&mut a, data_dir)?;
            r["source"] = json!("embedded");
            r
        }
    };
    report["archive_path"] = json!(if report["source"] == "user" {
        p.display().to_string()
    } else {
        String::new()
    });
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_zip(files: &[(&str, &str)]) -> Vec<u8> {
        let buf = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, content) in files {
            w.start_file(name, opts).unwrap();
            w.write_all(content.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn restores_whitelist_and_backs_up_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        // 现行 config.yaml：还原时会被覆盖 → 应进快照。
        fs::write(data.join("config.yaml"), "old: true\n").unwrap();
        let zip = make_zip(&[
            ("config.yaml", "new: true\n"),
            ("scripts/a.sh", "#!/bin/sh\necho hi\n"),
            ("evil/../escape.txt", "nope"),
            ("other/x.txt", "not whitelisted"),
        ]);
        let report = restore_from(zip, data).unwrap();
        assert_eq!(report["restored"].as_u64().unwrap(), 2);
        assert_eq!(report["backed_up"].as_u64().unwrap(), 1);
        assert_eq!(
            fs::read_to_string(data.join("config.yaml")).unwrap(),
            "new: true\n"
        );
        assert!(data.join("scripts/a.sh").is_file());
        // 白名单外的条目不落盘。
        assert!(!data.join("other/x.txt").exists());
        assert!(!data.join("escape.txt").exists());
        assert_eq!(report["skipped"].as_array().unwrap().len(), 2);
        // 快照里能找到旧内容。
        let backup = Path::new(report["backup_dir"].as_str().unwrap());
        assert_eq!(
            fs::read(backup.join("config.yaml")).unwrap(),
            b"old: true\n"
        );
    }

    #[test]
    #[cfg(unix)] // 这条测的就是 Unix 可执行位，非 Unix 平台无此语义
    fn backup_roundtrips_whitelist_and_restore_prefers_user_copy() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        fs::write(data.join("config.yaml"), "mine: true\n").unwrap();
        fs::create_dir_all(data.join("scripts")).unwrap();
        let script = data.join("scripts/a.sh");
        fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        // 白名单外的文件不该被卷进归档
        fs::write(data.join("notes.txt"), "private").unwrap();

        let r = backup(data).unwrap();
        assert_eq!(r["files"].as_u64().unwrap(), 2, "只打包白名单：{r}");
        assert!(user_archive_path(data).is_file());

        // 现状改坏后还原，必须回到自己备份的那份，而不是随包预设。
        fs::write(data.join("config.yaml"), "broken\n").unwrap();
        let rep = restore(data).unwrap();
        assert_eq!(rep["source"], "user", "有用户档就不许回落内置：{rep}");
        assert_eq!(
            fs::read_to_string(data.join("config.yaml")).unwrap(),
            "mine: true\n"
        );
        // 可执行位要活着回来，脚本还原后还得能跑。
        assert!(
            fs::metadata(&script).unwrap().permissions().mode() & 0o111 != 0,
            "还原丢了可执行位"
        );
    }

    #[test]
    fn backup_rotates_previous_archive_into_history() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        fs::write(data.join("config.yaml"), "v1\n").unwrap();
        backup(data).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        fs::write(data.join("config.yaml"), "v2\n").unwrap();
        let r = backup(data).unwrap();
        assert_eq!(
            r["history"].as_u64().unwrap(),
            1,
            "上一份该滚进 history：{r}"
        );
        assert_eq!(
            fs::read_to_string(data.join("config.yaml")).unwrap(),
            "v2\n",
            "备份本身不许动现状"
        );

        // 历史只留 HISTORY_KEEP 份，免得「备份」把盘撑爆。
        let hist = history_dir(data);
        for i in 0..(HISTORY_KEEP as u64 + 3) {
            fs::write(hist.join(format!("llm-switch-config-{i}.zip")), b"x").unwrap();
        }
        fs::write(data.join("config.yaml"), "v3\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let r = backup(data).unwrap();
        assert_eq!(
            r["history"].as_u64().unwrap() as usize,
            HISTORY_KEEP,
            "历史该裁到 {HISTORY_KEEP} 份"
        );
    }

    #[test]
    fn info_reports_where_the_archive_came_from() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        // 没备份过：报内置档，路径留空。
        let i0 = info(data);
        assert_eq!(i0["source"], "embedded");
        assert_eq!(i0["path"].as_str().unwrap(), "");
        assert!(i0["entries"].as_u64().unwrap() > 0);

        fs::write(data.join("config.yaml"), "mine\n").unwrap();
        backup(data).unwrap();
        let i1 = info(data);
        assert_eq!(i1["source"], "user");
        assert!(i1["path"]
            .as_str()
            .unwrap()
            .ends_with("llm-switch-config.zip"));
        assert!(i1["updated_ms"].as_i64().unwrap() > 0);

        // 用户档坏了不能静默回落内置——那会让「还原」还原成用户没选的那份。
        fs::write(user_archive_path(data), b"not a zip").unwrap();
        assert_eq!(info(data)["source"], "broken");
        let err = restore(data).unwrap_err();
        assert!(err.contains("备份归档"), "坏档必须摆出错误：{err}");
    }

    #[test]
    fn identical_files_count_as_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        fs::write(data.join("config.yaml"), "same\n").unwrap();
        let zip = make_zip(&[("config.yaml", "same\n")]);
        let report = restore_from(zip, data).unwrap();
        assert_eq!(report["restored"].as_u64().unwrap(), 0);
        assert_eq!(report["unchanged"].as_u64().unwrap(), 1);
        assert_eq!(report["backed_up"].as_u64().unwrap(), 0);
    }

    #[test]
    fn embedded_archive_is_a_valid_zip() {
        // 内嵌归档必须能打开且含白名单条目（防止 assets 文件被误清空/损坏）。
        let mut a = zip::ZipArchive::new(std::io::Cursor::new(ARCHIVE)).unwrap();
        assert!(!a.is_empty());
        let mut has_config = false;
        for i in 0..a.len() {
            if let Ok(f) = a.by_index(i) {
                if f.name() == "config.yaml" {
                    has_config = true;
                }
            }
        }
        assert!(has_config, "内嵌归档缺少 config.yaml");
    }
}
