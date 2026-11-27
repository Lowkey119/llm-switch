// 已安装应用清单（C7，计划 `2026-11-21-SayAll对标-组合序列与App绑定` §三）。
//
// 三件事：扫常见目录读 Info.plist（进程内缓存 + 手动刷新）、按需出图标 PNG、
// 「浏览…」走原生 NSOpenPanel（装在别处的 .app 也能绑）。
//
// 为什么自己读 plist 而不是 `plutil`/`mdfind`：一次列表要读几百份 plist，
// 每份起一个进程在慢盘上就是十几秒；core-foundation 的 propertylist 是同一套解析，
// 纯函数调用。文件顶 allow(unsafe_code) 与 axfocus.rs / perms.rs 同法。
#![allow(unsafe_code)]

use base64::Engine;
use core_foundation::base::{CFGetTypeID, CFRelease, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::propertylist::{create_with_data, kCFPropertyListImmutable};
use core_foundation::string::CFString;
use objc2::rc::Retained;
use serde::Serialize;
use std::collections::HashSet;
use std::os::raw::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 一条应用条目（前端列表的行）。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppEntry {
    pub name: String,
    pub bundle_id: String,
    pub path: String,
}

/// 进程内缓存：扫描一次几百毫秒，映射页每次开面板不该重扫。`refresh` 手动作废。
static CACHE: OnceLock<Mutex<Option<Vec<AppEntry>>>> = OnceLock::new();

/// 扫描目录（顺序即优先级：同名 App 用户装的排在系统装的前面）。
fn scan_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let mut v: Vec<PathBuf> = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    if let Some(h) = home {
        v.push(h.join("Applications"));
    }
    v
}

/// CoreFoundation 引用的释放守卫（Create 规则的回值统一在这里还）。
struct CfRef(*const c_void);

impl Drop for CfRef {
    fn drop(&mut self) {
        // SAFETY: 构造时已确认非空且由本守卫持有。
        unsafe { CFRelease(self.0) };
    }
}

/// 读 .app 的 Info.plist 里一个字符串键（CFBundleIdentifier / CFBundleDisplayName 等）。
fn plist_str(app: &Path, key: &str) -> Option<String> {
    let raw = std::fs::read(app.join("Contents/Info.plist")).ok()?;
    let data = CFData::from_buffer(&raw);
    let (plist, _fmt) = create_with_data(data, kCFPropertyListImmutable).ok()?;
    // 所有权交给守卫：函数返回时释放，中途的借用（下面的字典）全程有效。
    let _guard = CfRef(plist);
    if plist.is_null()
        || unsafe { CFGetTypeID(plist) } != CFDictionary::<CFString, *const c_void>::type_id()
    {
        return None;
    }
    // SAFETY: 上面核对过类型；字典所有权归 guard，这里按 Get 规则借用。
    let dict =
        unsafe { CFDictionary::<CFString, *const c_void>::wrap_under_get_rule(plist as *const _) };
    let val = dict.find(CFString::new(key))?;
    let vp: *const c_void = *val;
    if vp.is_null() || unsafe { CFGetTypeID(vp) } != CFString::type_id() {
        return None;
    }
    // SAFETY: 类型核对过是 CFString；所有权仍在字典（guard）手上，借用不释放。
    Some(unsafe { CFString::wrap_under_get_rule(vp as *const _) }.to_string())
}

/// 一个 .app 的条目：显示名优先 CFBundleDisplayName，退回 CFBundleName，再退回目录名。
fn entry_for(app: &Path) -> Option<AppEntry> {
    let bundle_id = plist_str(app, "CFBundleIdentifier")?;
    let name = plist_str(app, "CFBundleDisplayName")
        .or_else(|| plist_str(app, "CFBundleName"))
        .unwrap_or_else(|| {
            app.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| bundle_id.clone())
        });
    Some(AppEntry {
        name,
        bundle_id,
        path: app.display().to_string(),
    })
}

/// 扫一轮：顶层 .app，按 bundle id 去重（先扫的目录赢），按显示名排序。
fn scan() -> Vec<AppEntry> {
    scan_in(&scan_dirs())
}

/// 扫描主体（目录参数化，单测才喂得进临时目录）。
fn scan_in(dirs: &[PathBuf]) -> Vec<AppEntry> {
    let mut out: Vec<AppEntry> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("app") {
                continue;
            }
            let Some(item) = entry_for(&p) else { continue };
            if item.bundle_id.trim().is_empty() || !seen.insert(item.bundle_id.clone()) {
                continue;
            }
            out.push(item);
        }
    }
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.bundle_id.cmp(&b.bundle_id))
    });
    out
}

/// 应用清单（缓存优先）。
pub fn apps(refresh: bool) -> Vec<AppEntry> {
    let lock = CACHE.get_or_init(|| Mutex::new(None));
    let mut guard = lock.lock().expect("应用清单缓存锁损坏");
    if refresh || guard.is_none() {
        let scanned = scan();
        tracing::info!(target: "llm_switch::remote", count = scanned.len(), "已安装应用清单已刷新");
        *guard = Some(scanned);
    }
    guard.clone().unwrap_or_default()
}

/// 启动预热（用户 2026-11-23 拍板「启动就把应用清单缓存好，用时无感」）：
/// 在后台阻塞线程扫一轮填好缓存，选择器打开时只读缓存。
/// 已有缓存时直接返回，重复调用无害。**不要在主线程调**（扫盘会卡界面）。
pub fn warm() {
    apps(false);
}

/// 图标：系统按 LaunchServices 出图（与访达同一个），转 PNG 后 base64 给前端。
/// 按需调用（列表滚动到才取），整表预取会把 IPC 撑爆。
pub fn icon_png(path: &str) -> Result<String, String> {
    use objc2::{msg_send, AnyThread};
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSString};

    let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
    let tiff = image.TIFFRepresentation().ok_or("图标读取失败")?;
    let rep =
        NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff).ok_or("图标转位图失败")?;
    let props = NSDictionary::new();
    // SAFETY: 空属性字典是该 API 的合法入参；类型常量 PNG 是受支持的存储类型。
    let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }
        .ok_or("图标转 PNG 失败")?;
    // objc2 没给 NSData 的 bytes 访问器（返回裸指针不安全），直接发选择器取指针。
    // SAFETY: bytes 与 length 来自同一个 NSData，指针有效且长度匹配。
    let ptr: *const u8 = unsafe { msg_send![&png, bytes] };
    let len: usize = png.length();
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 「浏览…」：原生打开面板挑一个 .app（列表外的也能绑）。**必须在主线程调用**
/// （NSOpenPanel 要 MainThreadMarker；Tauri 的同步命令就跑在主线程）。
pub fn pick_app() -> Result<Option<AppEntry>, String> {
    use objc2::{msg_send, MainThreadMarker};
    use objc2_app_kit::NSOpenPanel;
    use objc2_foundation::{NSString, NSURL};

    let Some(mtm) = MainThreadMarker::new() else {
        return Err("应用选择面板只能在主线程打开".to_string());
    };
    let panel = NSOpenPanel::openPanel(mtm);
    // .app 本质是目录包，所以文件与目录都放开，选完按路径核对（不核对就会挑到随便一个文件夹）。
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(true);
    panel.setAllowsMultipleSelection(false);
    // 默认定位 /Applications（用户 2026-11-24 拍板：打开应用直接走系统选择框，不再页内扫列表）。
    panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(
        "/Applications",
    ))));
    // SAFETY: runModal 是 NSPanel 的既有选择器（objc2 未生成绑定），回 NSInteger 模态码。
    let resp: isize = unsafe { msg_send![&panel, runModal] };
    const NS_MODAL_RESPONSE_OK: isize = 1;
    if resp != NS_MODAL_RESPONSE_OK {
        return Ok(None);
    }
    let url: Option<Retained<NSURL>> = panel.URL();
    let Some(url) = url else {
        return Ok(None);
    };
    let path = url.path().map(|p| p.to_string()).unwrap_or_default();
    let app = Path::new(&path);
    if app.extension().and_then(|x| x.to_str()) != Some("app") {
        return Err("只能选 .app 应用".to_string());
    }
    entry_for(app)
        .map(Some)
        .ok_or_else(|| "选中的 .app 里没有可用的 bundle id".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个最小 .app：只有 Contents/Info.plist（XML 形态，人写得动）。
    fn make_app(root: &Path, stem: &str, plist_body: &str) -> PathBuf {
        let app = root.join(format!("{stem}.app"));
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
{plist_body}
</dict>
</plist>"#
        );
        std::fs::write(app.join("Contents/Info.plist"), body).unwrap();
        app
    }

    #[test]
    fn entry_prefers_display_name_then_bundle_name_then_dir_stem() {
        let dir = tempfile::tempdir().unwrap();
        let both = make_app(
            dir.path(),
            "Junk",
            r#"<key>CFBundleIdentifier</key><string>com.ex.both</string>
               <key>CFBundleName</key><string>名字</string>
               <key>CFBundleDisplayName</key><string>显示名</string>"#,
        );
        assert_eq!(entry_for(&both).unwrap().name, "显示名");
        let named = make_app(
            dir.path(),
            "Junk2",
            r#"<key>CFBundleIdentifier</key><string>com.ex.named</string>
               <key>CFBundleName</key><string>只有名字</string>"#,
        );
        assert_eq!(entry_for(&named).unwrap().name, "只有名字");
        let bare = make_app(
            dir.path(),
            "OnlyStem",
            r#"<key>CFBundleIdentifier</key><string>com.ex.bare</string>"#,
        );
        assert_eq!(entry_for(&bare).unwrap().name, "OnlyStem");
    }

    #[test]
    fn plist_without_bundle_id_is_not_an_entry() {
        let dir = tempfile::tempdir().unwrap();
        let app = make_app(
            dir.path(),
            "NoId",
            r#"<key>CFBundleName</key><string>没身份</string>"#,
        );
        assert!(entry_for(&app).is_none());
    }

    #[test]
    fn scan_dedupes_by_bundle_id_and_sorts_by_name() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        // 同名 bundle id 两处都有：先扫的目录赢（用户装的排在系统装的前面）。
        make_app(
            a.path(),
            "zeta",
            r#"<key>CFBundleIdentifier</key><string>com.ex.dup</string>
               <key>CFBundleDisplayName</key><string>Zeta</string>"#,
        );
        make_app(
            b.path(),
            "alpha",
            r#"<key>CFBundleIdentifier</key><string>com.ex.dup</string>
               <key>CFBundleDisplayName</key><string>Alpha</string>"#,
        );
        make_app(
            a.path(),
            "Beta",
            r#"<key>CFBundleIdentifier</key><string>com.ex.beta</string>
               <key>CFBundleDisplayName</key><string>beta</string>"#,
        );
        // 非 .app 目录与没 plist 的 .app 都不进清单。
        std::fs::create_dir_all(a.path().join("NotAnApp")).unwrap();
        let out = scan_in(&[a.path().to_path_buf(), b.path().to_path_buf()]);
        let ids: Vec<&str> = out.iter().map(|e| e.bundle_id.as_str()).collect();
        assert_eq!(ids, vec!["com.ex.beta", "com.ex.dup"]);
        assert_eq!(out[1].name, "Zeta", "先扫的目录赢");
    }
}
