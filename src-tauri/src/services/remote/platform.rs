//! 平台能力接口（跨平台移植 S-X0）。
//!
//! 注入与拦截是遥控器桥接里唯一真正平台相关的两层；其余（按键表、报告解析、
//! 手势/连发闸门、动作与方案存储、设备档案、ATVV 协议、语音出口）三平台共用一份。
//! 这里只定义形状与工厂：macOS 实现在 `inject`，Windows/Linux 实现随 S-X2/S-X4 落地。
//!
//! 线程口径由调用方负责——macOS 的 `CGEventPost` 必须主线程，`hid.rs` 统一经
//! `run_on_main_thread` 投递；enigo 在 Win/Linux 不挑线程，投主线程也无害。

use crate::services::remote::actions::Shortcut;
use crate::services::remote::buttons::{NativeEvent, RemoteButton};
use std::sync::Arc;
use std::time::Instant;
use tauri::AppHandle;

/// 注入侧：把语义事件变成系统输入。
pub trait Injector: Send + Sync {
    fn inject_shortcut(&self, sc: Shortcut) -> Result<(), String>;
    fn inject_native(&self, ev: NativeEvent) -> Result<(), String>;
    /// 以输入法上屏的方式写入文本（语音转写落点，不碰剪贴板）。
    fn inject_text(&self, text: &str) -> Result<(), String>;
}

/// 拦截侧：吞掉遥控器自己触发的原生事件，以及录制态的全量键盘。
pub trait Interceptor: Send + Sync {
    /// 登记某键的原生事件：随后窗口内到达的同码事件一律吞掉。
    fn arm(&self, button: RemoteButton, now: Instant);
    /// 录制闸门：true = 拦下所有键盘转交前端录制器。
    fn set_recording(&self, active: bool);
    fn recording(&self) -> bool;
    /// 启动拦截器（进程级一次）。失败 = 平台权限不足，错误必须冒到页面，不许静默。
    fn start(&self, app: AppHandle) -> Result<(), String>;
    /// 该平台是否真的具备拦截能力。false = 禁用键抑制与录制拦截不生效，
    /// 页面必须据此说破（Wayland / Linux 抑制降级），不许装成生效。
    fn available(&self) -> bool {
        true
    }
}

#[cfg(target_os = "macos")]
pub fn injector() -> Arc<dyn Injector> {
    Arc::new(crate::services::remote::inject::MacInjector)
}

#[cfg(target_os = "macos")]
pub fn interceptor() -> Arc<dyn Interceptor> {
    Arc::new(crate::services::remote::inject::MacInterceptor::default())
}

/// 其余平台：接口就位、实现待 S-X2/S-X4，动作一律明报不支持。
#[cfg(not(target_os = "macos"))]
pub fn injector() -> Arc<dyn Injector> {
    Arc::new(Unsupported)
}

#[cfg(not(target_os = "macos"))]
pub fn interceptor() -> Arc<dyn Interceptor> {
    Arc::new(Unsupported)
}

#[cfg(not(target_os = "macos"))]
struct Unsupported;

#[cfg(not(target_os = "macos"))]
const ERR: &str = "平台不支持";

#[cfg(not(target_os = "macos"))]
impl Injector for Unsupported {
    fn inject_shortcut(&self, _sc: Shortcut) -> Result<(), String> {
        Err(ERR.to_string())
    }
    fn inject_native(&self, _ev: NativeEvent) -> Result<(), String> {
        Err(ERR.to_string())
    }
    fn inject_text(&self, _text: &str) -> Result<(), String> {
        Err(ERR.to_string())
    }
}

#[cfg(not(target_os = "macos"))]
impl Interceptor for Unsupported {
    fn arm(&self, _button: RemoteButton, _now: Instant) {}
    fn set_recording(&self, _active: bool) {}
    fn recording(&self) -> bool {
        false
    }
    fn start(&self, _app: AppHandle) -> Result<(), String> {
        Err(ERR.to_string())
    }
    fn available(&self) -> bool {
        false
    }
}
