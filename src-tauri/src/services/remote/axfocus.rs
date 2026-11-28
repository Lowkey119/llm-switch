// AX 聚焦前台输入区（C2 `FocusInput`，计划 `2026-11-21-SayAll对标` §三）。
//
// 为什么手写绑定：Accessibility 是 ApplicationServices 的 C API（不是 objc 类），
// objc2 没有它，core-foundation 只给 CFString/CFArray。文件顶 allow(unsafe_code)
// 与 ime.rs / perms.rs / speech.rs 同法，每个 unsafe 块带 SAFETY 注释。
//
// 找输入区的口径：前台 App 自己报的 AXFocusedUIElement 若已是输入区就直接确认（零遍历，
// 多数 App 走这条）；否则从 App 元素广度优先找第一个文本输入角色。遍历有硬预算——
// AX 调用是跨进程同步往返，Electron 那种上万节点的树能把主线程拖死，
// 所以节点数、深度、单元素消息超时三条都封了顶。
#![allow(unsafe_code)]

use core_foundation::array::CFArray;
use core_foundation::base::{CFRelease, CFRetain, TCFType};
use core_foundation::boolean::kCFBooleanTrue;
use core_foundation::string::CFString;
use std::os::raw::{c_float, c_int, c_void};

type AXRef = *mut c_void;
type AXError = c_int;
const AX_SUCCESS: AXError = 0;

/// 单次聚焦请求的硬预算：广度优先最多摸这么多元素、最深这么多层。
const MAX_NODES: usize = 250;
const MAX_DEPTH: usize = 6;
/// 单个元素的 AX 往返超时（秒）：目标 App 卡住时不许把调用方一起拖住。
const MSG_TIMEOUT_SECS: c_float = 0.2;

/// 算「输入区」的 AX 角色（SayAll 的 focusInput 目标就是这几类可写文本元素）。
const INPUT_ROLES: [&str; 4] = ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField"];

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AXRef;
    fn AXUIElementCreateApplication(pid: c_int) -> AXRef;
    fn AXUIElementCopyAttributeValue(
        element: AXRef,
        attribute: *const c_void,
        value: *mut AXRef,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: AXRef,
        attribute: *const c_void,
        value: *const c_void,
    ) -> AXError;
    fn AXUIElementSetMessagingTimeout(element: AXRef, timeout: c_float) -> AXError;
}

/// AX 元素引用：`AXUIElementCopyAttributeValue` 回的是 Create 规则（调用方持有），
/// 靠这个守卫统一释放，避免十处手工 CFRelease 漏一处。
struct El(AXRef);

impl Drop for El {
    fn drop(&mut self) {
        // SAFETY: 构造时已确认非空且由本守卫持有（Create 规则或显式 CFRetain）。
        unsafe { CFRelease(self.0 as *const c_void) };
    }
}

fn attr(name: &str) -> CFString {
    CFString::new(name)
}

/// 读一个元素属性（Create 规则回值）；失败或空回 None。
fn copy_attr(el: AXRef, name: &str) -> Option<El> {
    let key = attr(name);
    let mut out: AXRef = std::ptr::null_mut();
    // SAFETY: el 有效、key 是合法 CFString、out 是可写槽；成功时 out 由本函数接管并交给 El。
    let err = unsafe {
        AXUIElementCopyAttributeValue(el, key.as_concrete_TypeRef() as *const c_void, &mut out)
    };
    // 必须 `then`（惰性）不能 `then_some`（急切）：`then_some(El(out))` 无论条件真假都会
    // 先构造 `El(out)`，条件为假时这个临时 El 立刻走 Drop → `CFRelease(NULL)` → SIGTRAP，
    // 整个应用当场没（2026-11-25 实报：从 VS Code 非编辑器区呼出面板即崩，
    // DiagnosticReports 里 asi = `*** CFRelease() called with NULL ***`）。
    (err == AX_SUCCESS && !out.is_null()).then(|| El(out))
}

/// 读一个字符串属性（AXRole 等）。
fn copy_string(el: AXRef, name: &str) -> Option<String> {
    let raw = copy_attr(el, name)?;
    // SAFETY: copy_attr 刚拿到的引用实际类型是 CFString（角色/标题属性都是）。
    // 用 get_rule 而不是 create_rule：所有权在 `raw`（El 负责释放），这里只是借用视图，
    // 两处都按 Create 规则释放就是双重 CFRelease。
    let s = unsafe { CFString::wrap_under_get_rule(raw.0 as *const _) };
    Some(s.to_string())
}

fn role(el: AXRef) -> Option<String> {
    copy_string(el, "AXRole")
}

fn is_input(el: AXRef) -> bool {
    role(el).is_some_and(|r| INPUT_ROLES.contains(&r.as_str()))
}

/// 子元素：数组本身按 Create 规则接管（drop 即释放），元素是数组借出的，
/// 要带出作用域必须各自 CFRetain（否则数组一释放就是悬空指针）。
fn children(el: AXRef) -> Vec<El> {
    let Some(raw) = copy_attr(el, "AXChildren") else {
        return Vec::new();
    };
    // SAFETY: AXChildren 回的是 CFArrayRef。数组所有权仍归 `raw`（El 释放），
    // 所以这里按 Get 规则借用——两处都释放就是双重 CFRelease。
    let arr = unsafe { CFArray::<*const c_void>::wrap_under_get_rule(raw.0 as *const _) };
    arr.get_all_values()
        .into_iter()
        .map(|p| {
            // SAFETY: 数组持有的合法 CF 对象引用；retain 后本元素独立于数组生命周期。
            unsafe { CFRetain(p) };
            El(p as *mut c_void)
        })
        .collect()
}

/// 把布尔属性设成 true（AXFocused / AXMain 都吃这一形）。
fn set_true(el: AXRef, name: &str) -> Result<(), String> {
    let key = attr(name);
    // SAFETY: el 有效；kCFBooleanTrue 是 CoreFoundation 常量（CFBoolean，这两个属性要的类型）。
    let err = unsafe {
        AXUIElementSetAttributeValue(
            el,
            key.as_concrete_TypeRef() as *const c_void,
            kCFBooleanTrue as *const c_void,
        )
    };
    if err == AX_SUCCESS {
        Ok(())
    } else {
        Err(format!("AX 设置 {name} 失败（错误码 {err}）"))
    }
}

/// 把焦点设到某个元素上。
fn set_focused(el: AXRef) -> Result<(), String> {
    set_true(el, "AXFocused")
}

/// 广度优先找第一个输入区并聚焦；返回找没找到。
fn search(root: AXRef) -> bool {
    let mut frontier: Vec<El> = children(root);
    let mut seen = 0usize;
    for _depth in 0..MAX_DEPTH {
        if frontier.is_empty() || seen >= MAX_NODES {
            return false;
        }
        let mut next = Vec::new();
        for node in frontier {
            if seen >= MAX_NODES {
                return false;
            }
            seen += 1;
            if is_input(node.0) {
                // 找到就抢在预算耗尽前落定；设焦点失败也按「找到了」处理，
                // 由上层把错误码报出去，不再继续翻（再翻也是同一个 App 的树）。
                return set_focused(node.0).is_ok();
            }
            next.extend(children(node.0));
        }
        frontier = next;
    }
    false
}

/// 让前台 App 的输入区拿到键盘焦点。找不到即回 Err，调用方按 no-op 处理（计划 §三）。
pub fn focus_frontmost_input() -> Result<(), String> {
    let sys = El(unsafe { AXUIElementCreateSystemWide() });
    let app =
        copy_attr(sys.0, "AXFocusedApplication").ok_or("取不到前台应用（辅助功能未授权？）")?;
    // 目标 App 可能是无响应的大 App：先给它定个消息超时，别把调用方一起拖住。
    // SAFETY: app.0 是合法元素；超时值是秒（浮点）。返回值不影响后续调用。
    unsafe { AXUIElementSetMessagingTimeout(app.0, MSG_TIMEOUT_SECS) };
    // 1) App 自己报的聚焦元素已是输入区 → 直接确认（零遍历）。
    if let Some(fe) = copy_attr(app.0, "AXFocusedUIElement") {
        if is_input(fe.0) {
            return set_focused(fe.0);
        }
    }
    // 2) 有界遍历找输入区。
    if search(app.0) {
        return Ok(());
    }
    Err("前台 App 里没找到可聚焦的输入框".to_string())
}

/// 前台 App 有没有可写输入区（只探测，不改焦点）。
///
/// 快速面板的注入目标门控（用户 2026-11-25）：唤起瞬间缓存的前台 App 必须
/// 「有输入编辑框」才配当首要注入对象——没有就显示「无注入目标」，
/// 免得用户对着一个没输入框的 App 按 OK 却毫无反应。
/// 口径与 focus_frontmost_input 的找法一致（聚焦元素快路径 + 有界遍历），
/// 只是不 set_focused。AX 未授权 / 超时都按「没有」处理——探测失败不许假装成功。
pub fn front_has_input() -> bool {
    let sys = El(unsafe { AXUIElementCreateSystemWide() });
    let Some(app) = copy_attr(sys.0, "AXFocusedApplication") else {
        return false;
    };
    // SAFETY: app.0 是合法元素；超时值是秒（浮点）。
    unsafe { AXUIElementSetMessagingTimeout(app.0, MSG_TIMEOUT_SECS) };
    if let Some(fe) = copy_attr(app.0, "AXFocusedUIElement") {
        if is_input(fe.0) {
            return true;
        }
    }
    // 有界遍历（只读不聚焦）：search() 会 set_focused，这里不能复用。
    let mut frontier: Vec<El> = children(app.0);
    let mut seen = 0usize;
    for _depth in 0..MAX_DEPTH {
        if frontier.is_empty() || seen >= MAX_NODES {
            return false;
        }
        let mut next = Vec::new();
        for node in frontier {
            if seen >= MAX_NODES {
                return false;
            }
            seen += 1;
            if is_input(node.0) {
                return true;
            }
            next.extend(children(node.0));
        }
        frontier = next;
    }
    false
}

/// 目标 App 当前聚焦窗口的标题（无窗 / 无标题 / 未授权 = None）。
/// 快速面板唤起瞬间记下它，归还焦点时按标题找回同一个窗口（同 App 多窗口场景，
/// 用户 2026-11-27 实报：两个 Orca 窗口，activate 只回到 App，键落错了窗）。
pub fn focused_window_title(pid: i32) -> Option<String> {
    let app = El(unsafe { AXUIElementCreateApplication(pid) });
    // SAFETY: app.0 是合法元素；超时值是秒（浮点）。
    unsafe { AXUIElementSetMessagingTimeout(app.0, MSG_TIMEOUT_SECS) };
    let win = copy_attr(app.0, "AXFocusedWindow")?;
    copy_string(win.0, "AXTitle")
}

/// 把标题匹配的窗口提为当前窗口（AXMain 提窗 + AXFocused 给键盘焦点）。
/// 找到并设置成功回 true；没找到（改名/已关）回 false，调用方维持 activate 整 App 的现状。
pub fn focus_window(pid: i32, title: &str) -> bool {
    let app = El(unsafe { AXUIElementCreateApplication(pid) });
    // SAFETY: app.0 是合法元素；超时值是秒（浮点）。
    unsafe { AXUIElementSetMessagingTimeout(app.0, MSG_TIMEOUT_SECS) };
    let Some(raw) = copy_attr(app.0, "AXWindows") else {
        return false;
    };
    // SAFETY: AXWindows 回的是 CFArrayRef；数组所有权归 `raw`（El 释放），按 Get 规则借用。
    let arr = unsafe { CFArray::<*const c_void>::wrap_under_get_rule(raw.0 as *const _) };
    for w in arr.get_all_values() {
        let el = w as AXRef;
        if copy_string(el, "AXTitle").as_deref() != Some(title) {
            continue;
        }
        // 先提窗再给焦点；AXMain 失败不拦（有的 App 只认 AXFocused）。
        let _ = set_true(el, "AXMain");
        return set_focused(el).is_ok();
    }
    false
}
