//! 遥控器桥接（计划 2026-11-19-遥控器桥接重写）。
//!
//! S1 只落 BLE 身份与连接（macOS）。平台差异全部关在 `identity`（btleplug 依赖
//! 也只在 macOS 段），跨平台 trait 随 S9/S10 落地时再抽——现在没有第二平台
//! 消费者，提前抽 trait 只会让 clippy -D dead_code 挂门禁。
//!
//! S4 的纯逻辑（`buttons` 按键表/报告/闸门、`actions` 动作表/持久化）不带 cfg，
//! 默认 `cargo test` 就编到并全测；FFI 层（hidapi 通道、注入、抑制）随 S4b 挂
//! `gui` + macOS 段。
//!
//! 非 macOS 走同名 stub：命令门面在三个平台都注册（generate_handler 是静态列表），
//! 但每个动作直接报「平台不支持」——页面在非 macOS 整页降级，本来也不会调。

#[cfg(feature = "gui")]
pub mod platform;

#[cfg(all(feature = "gui", target_os = "macos"))]
pub mod identity;

#[cfg(all(feature = "gui", target_os = "macos"))]
pub mod battery;
#[cfg(all(feature = "gui", target_os = "macos"))]
pub mod hid;
#[cfg(all(feature = "gui", target_os = "macos"))]
pub mod ime; // 输入法枚举/切换（Carbon TIS）：路 2 虚拟声卡接输入法。
#[cfg(all(feature = "gui", target_os = "macos"))]
pub mod inject;
#[cfg(all(feature = "gui", target_os = "macos"))]
pub mod perms; // 系统授权批量检测：遥控器页统一授权检查入口。
#[cfg(all(feature = "gui", target_os = "macos"))]
pub mod powermap;
#[cfg(all(feature = "gui", target_os = "macos"))]
pub mod speech; // 语音路 1（系统转写）：Speech 框架 + AVAudioEngine，macOS 专属。
#[cfg(all(feature = "gui", target_os = "macos"))]
pub mod voice;

#[cfg(all(feature = "gui", not(target_os = "macos")))]
pub mod hid {
    //! 非 macOS stub：按键通道 macOS 专属（计划 §五：非 macOS 整页降级，不发 IPC）。
    //! 形状与 macOS 版一致，保证命令门面三平台静态注册编得过。
    use serde_json::{json, Value};

    #[derive(Default)]
    pub struct ButtonChannel;

    impl ButtonChannel {
        pub fn new() -> Self {
            Self
        }
        #[allow(clippy::too_many_arguments)]
        pub fn start(
            &self,
            _app: tauri::AppHandle,
            _store: std::sync::Arc<crate::store::Store>,
            _data_dir: std::path::PathBuf,
            _voice: std::sync::Arc<super::voice::VoiceChannel>,
            _session: std::sync::Arc<super::identity::RemoteSession>,
        ) {
        }
        pub fn snapshot(&self, _profile_id: &str) -> Value {
            json!({ "config": null, "hid": { "mode": "unsupported" } })
        }
        pub fn set_config(
            &self,
            _profile_id: &str,
            _config: super::actions::ButtonConfig,
        ) -> Result<(), String> {
            Err("平台不支持（当前仅 macOS）".to_string())
        }
        pub fn set_recording(&self, _active: bool) {}
        pub fn inner(&self) -> &Self {
            self
        }
        pub fn simulate_named(&self, _id: &str, _gesture: &str) -> Result<(), String> {
            Err("平台不支持（当前仅 macOS）".to_string())
        }
        pub fn scheme_names(&self, _profile_id: &str) -> Vec<String> {
            Vec::new()
        }
        pub fn scheme_save(&self, _profile_id: &str, _name: &str) -> Result<(), String> {
            Err("平台不支持（当前仅 macOS）".to_string())
        }
        pub fn scheme_apply(&self, _profile_id: &str, _name: &str) -> Result<(), String> {
            Err("平台不支持（当前仅 macOS）".to_string())
        }
        pub fn scheme_rename(&self, _p: &str, _o: &str, _n: &str) -> Result<(), String> {
            Err("平台不支持（当前仅 macOS）".to_string())
        }
        pub fn scheme_export(&self, _profile_id: &str, _name: &str) -> Result<String, String> {
            Err("平台不支持（当前仅 macOS）".to_string())
        }
        pub fn scheme_import(
            &self,
            _profile_id: &str,
            _name: &str,
            _content: &str,
        ) -> Result<(), String> {
            Err("平台不支持（当前仅 macOS）".to_string())
        }
    }
}

#[cfg(all(feature = "gui", not(target_os = "macos")))]
pub mod voice {
    //! 非 macOS stub：语音通道 macOS 专属（ATVV 走 btleplug，当前只进 macOS 段）。
    //! 形状与 macOS 版一致，保证命令门面三平台静态注册编得过。
    use crate::gateway::State;
    use crate::services::remote::identity::RemoteSession;
    use crate::services::remote::profile::DeviceProfile;
    use crate::store::Store;
    use serde_json::{json, Value};
    use std::sync::Arc;

    #[derive(Default)]
    pub struct VoiceChannel;

    impl VoiceChannel {
        pub fn new() -> Self {
            Self
        }
        pub fn start(
            self: &Arc<Self>,
            _app: tauri::AppHandle,
            _gateway: Arc<State>,
            _store: &Store,
        ) {
        }
        pub fn snapshot(&self) -> Value {
            json!({ "state": "idle", "settings": null, "metrics": null, "note": "平台不支持", "devices": [] })
        }
        pub fn set_settings(&self, _store: &Store, _s: Value) -> Result<(), String> {
            Err("平台不支持（当前仅 macOS）".to_string())
        }
        pub fn press(&self, _session: Arc<RemoteSession>, _prof: &'static DeviceProfile) {}
        pub fn release(&self) {}
    }

    pub fn output_devices() -> Value {
        json!({ "devices": [], "default": "" })
    }
}

#[cfg(all(feature = "gui", not(target_os = "macos")))]
pub mod battery {
    //! 非 macOS stub：电量轮询 macOS 专属（电池走 CoreBluetooth 已连接检索）。
    use crate::services::remote::identity::RemoteSession;
    use std::sync::Arc;

    pub async fn poll_loop(_session: Arc<RemoteSession>) {}
}

pub mod actions;
pub mod atvv;
pub mod buttons;
// mSBC-over-HID 语音帧（小米 1 代，计划 §八 P7）：纯逻辑，无 cfg，默认 cargo test 编到。
pub mod msbc; // 设备档案：纯数据 + 纯逻辑，无 cfg（计划 2026-11-19 §四）。
pub mod profile;

#[cfg(all(feature = "gui", not(target_os = "macos")))]
pub mod identity {
    //! 非 macOS stub：形状与 macOS 版一致，动作一律「平台不支持」（计划 §五）。
    use crate::store::Store;
    use serde::Serialize;

    #[derive(Clone, Serialize)]
    pub struct RemoteDevice {
        pub id: String,
        pub name: Option<String>,
        pub model: Option<String>,
        pub connected: bool,
        // 与 macOS 版同形（命令门面三平台静态注册，页面拿到的形状必须一致）。
        pub profile: Option<String>,
        pub supported: bool,
    }

    #[derive(Serialize)]
    pub struct RemoteStatus {
        pub bound: Option<RemoteDevice>,
        pub candidates: Vec<RemoteDevice>,
        pub ble: String,
        pub battery: Option<u8>,
    }

    pub struct RemoteSession;

    impl Default for RemoteSession {
        fn default() -> Self {
            Self
        }
    }

    impl RemoteSession {
        pub fn new() -> Self {
            Self
        }
        pub async fn scan(&self) -> Result<Vec<RemoteDevice>, String> {
            Err("遥控器当前仅支持 macOS".into())
        }
        pub async fn connect(&self, _store: &Store, _id: &str) -> Result<RemoteDevice, String> {
            Err("遥控器当前仅支持 macOS".into())
        }
        pub async fn disconnect(&self) -> Result<(), String> {
            Err("遥控器当前仅支持 macOS".into())
        }
        pub fn status(&self, _store: &Store) -> RemoteStatus {
            RemoteStatus {
                bound: None,
                candidates: Vec::new(),
                ble: "unsupported".into(),
                battery: None,
            }
        }
    }
}
