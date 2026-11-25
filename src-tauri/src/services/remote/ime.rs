// 输入法（Text Input Source）枚举与切换：路 2 虚拟声卡的「接哪个输入法」配置。
//
// Carbon TIS 没有安全封装 crate，本文件按仓例（speech.rs / inject_win.rs 同口径）
// 文件顶 allow(unsafe_code)，unsafe 块逐条带 SAFETY 注释。
// 口径对齐 SayAll：语音会话开始切到用户指定的输入法，收尾还原。
#![allow(unsafe_code)]

use serde::Serialize;
use std::ffi::{c_void, CStr};

/// 一个可选择的键盘输入法（已在本机启用）。
#[derive(Clone, Serialize)]
pub struct ImeSource {
    pub id: String,
    pub name: String,
}

type CFStringRef = *const c_void;
type CFArrayRef = *const c_void;
/// kCFStringEncodingUTF8
const UTF8: u32 = 0x0800_0100;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    /// +1 返回调用者持有的 CFArray<TISInputSourceRef>；filter=null → 本机已启用的全部源。
    fn TISCreateInputSourceList(filter: *const c_void, include_all_installed: bool) -> CFArrayRef;
    /// +1 返回当前键盘输入源。
    fn TISCopyCurrentKeyboardInputSource() -> *const c_void;
    fn TISSelectInputSource(src: *const c_void) -> i32;
    ///  borrowed 属性值（不拥有，不释放）。
    fn TISGetInputSourceProperty(src: *const c_void, key: CFStringRef) -> *const c_void;
    static kTISPropertyInputSourceID: CFStringRef;
    static kTISPropertyLocalizedName: CFStringRef;
    static kTISPropertyInputSourceCategory: CFStringRef;
    static kTISPropertyInputSourceIsSelectCapable: CFStringRef;
    static kTISCategoryKeyboardInputSource: CFStringRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(arr: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: isize) -> *const c_void;
    fn CFStringGetCStringPtr(s: CFStringRef, enc: u32) -> *const i8;
    fn CFStringGetLength(s: CFStringRef) -> isize;
    fn CFStringGetCString(s: CFStringRef, buf: *mut i8, size: isize, enc: u32) -> bool;
    fn CFBooleanGetValue(b: *const c_void) -> bool;
    fn CFRelease(p: *const c_void);
}

/// CFString → String。先试缓存指针，再走 UTF8 缓冲兜底。
///
/// # Safety
/// `s` 必须是有效 CFString 或 null。
unsafe fn cf_str(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    // SAFETY: 非空 CFString；GetCStringPtr 返回的指针由 CF 持有，只读拷贝。
    let p = unsafe { CFStringGetCStringPtr(s, UTF8) };
    if !p.is_null() {
        // SAFETY: UTF8 编码的 NUL 结尾 C 串。
        return Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned());
    }
    let len = unsafe { CFStringGetLength(s) }.max(0) as usize;
    let mut buf = vec![0i8; len * 4 + 1];
    // SAFETY: buf 长度按 UTF8 最坏 4 字节/字符预留。
    if unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) } {
        // SAFETY: 成功写入后 buf 是 NUL 结尾 UTF8。
        Some(
            unsafe { CStr::from_ptr(buf.as_ptr()) }
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    }
}

/// 本机已启用、可选择的键盘输入法清单（虚拟键盘/布局也在其中——按 selectCapable 收）。
pub fn list() -> Vec<ImeSource> {
    // SAFETY: TIS/CF 契约——null filter 回已启用源数组；属性指针 borrowed 不释放。
    unsafe {
        let arr = TISCreateInputSourceList(std::ptr::null(), false);
        if arr.is_null() {
            return Vec::new();
        }
        let kb = cf_str(kTISCategoryKeyboardInputSource);
        let mut out = Vec::new();
        for i in 0..CFArrayGetCount(arr) {
            let src = CFArrayGetValueAtIndex(arr, i);
            let cat = cf_str(TISGetInputSourceProperty(
                src,
                kTISPropertyInputSourceCategory,
            ));
            if cat.is_none() || cat != kb {
                continue;
            }
            let sel = TISGetInputSourceProperty(src, kTISPropertyInputSourceIsSelectCapable);
            if sel.is_null() || !CFBooleanGetValue(sel) {
                continue;
            }
            if let Some(id) = cf_str(TISGetInputSourceProperty(src, kTISPropertyInputSourceID)) {
                let name = cf_str(TISGetInputSourceProperty(src, kTISPropertyLocalizedName))
                    .unwrap_or_else(|| id.clone());
                out.push(ImeSource { id, name });
            }
        }
        CFRelease(arr);
        out
    }
}

/// 当前键盘输入源 id。
pub fn current_id() -> Option<String> {
    // SAFETY: Copy 语义 +1，读完即释放。
    unsafe {
        let src = TISCopyCurrentKeyboardInputSource();
        if src.is_null() {
            return None;
        }
        let id = cf_str(TISGetInputSourceProperty(src, kTISPropertyInputSourceID));
        CFRelease(src);
        id
    }
}

/// 切到指定输入法；成功返回 true。
pub fn select(id: &str) -> bool {
    // SAFETY: 同 list()；选中的 borrowed 项不释放，数组整体释放。
    unsafe {
        let arr = TISCreateInputSourceList(std::ptr::null(), false);
        if arr.is_null() {
            return false;
        }
        let mut ok = false;
        for i in 0..CFArrayGetCount(arr) {
            let src = CFArrayGetValueAtIndex(arr, i);
            if cf_str(TISGetInputSourceProperty(src, kTISPropertyInputSourceID)).as_deref()
                == Some(id)
            {
                ok = TISSelectInputSource(src) == 0;
                break;
            }
        }
        CFRelease(arr);
        ok
    }
}

/// 会话开始切到目标输入法；真的切换了才返回原输入法 id（供收尾还原）。
pub fn switch_to(id: &str) -> Option<String> {
    let prev = current_id()?;
    if prev == id || !select(id) {
        return None;
    }
    Some(prev)
}

/// 打开系统设置「键盘 → 输入法」面板（输入法增删与启用入口）。
pub fn open_settings() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.Keyboard-Settings.extension?InputSources")
        .spawn();
}
