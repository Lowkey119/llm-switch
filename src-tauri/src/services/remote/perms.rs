// 系统授权批量检测（遥控器页「授权检查」统一入口）：安全（辅助功能/输入监控）、
// 隐私（语音识别/麦克风/蓝牙）一次读齐，页面据此提醒并给每项快速入口。
//
// 全部是只读查询——请求弹窗仍由各功能首次使用时触发（speech.rs 的 requestAuthorization 等），
// 这里不抢弹、不越权。
#![allow(unsafe_code)]

use serde::Serialize;

/// 一项授权的状态（id 同时是前端文案与 remote_perm_open 的键）。
#[derive(Clone, Serialize)]
pub struct Perm {
    pub id: &'static str,
    pub granted: bool,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    /// 辅助功能（按键注入所需）。只查询，不弹提示窗。
    fn AXIsProcessTrusted() -> bool;
}

/// 语音识别授权（objc2-speech，与 speech.rs 同一入口）。
fn speech_granted() -> bool {
    use objc2_speech::{SFSpeechRecognizer, SFSpeechRecognizerAuthorizationStatus};
    // SAFETY: 类方法纯查询。
    matches!(
        unsafe { SFSpeechRecognizer::authorizationStatus() },
        SFSpeechRecognizerAuthorizationStatus::Authorized
    )
}

/// 麦克风授权（AVAudioApplication.recordPermission，macOS 12+ 口径）。
fn mic_granted() -> bool {
    use objc2_avf_audio::{AVAudioApplication, AVAudioApplicationRecordPermission};
    // SAFETY: 单例获取 + 纯查询。
    unsafe {
        AVAudioApplication::sharedInstance().recordPermission()
            == AVAudioApplicationRecordPermission::Granted
    }
}

/// 蓝牙授权（CoreBluetooth CBCentralManager.authorization，3 = allowedAlways）。
fn bluetooth_granted() -> bool {
    // SAFETY: CBCentralManager 类恒在（CoreBluetooth 已链接）；authorization 是类方法回枚举。
    let auth: isize = unsafe { objc2::msg_send![objc2::class!(CBCentralManager), authorization] };
    auth >= 3
}

/// 全部授权项（顺序即页面展示序）。
pub fn check() -> Vec<Perm> {
    vec![
        Perm {
            id: "a11y",
            // SAFETY: 纯查询。
            granted: unsafe { AXIsProcessTrusted() },
        },
        Perm {
            id: "input_monitor",
            granted: super::hid::input_monitoring_granted(),
        },
        Perm {
            id: "speech",
            granted: speech_granted(),
        },
        Perm {
            id: "mic",
            granted: mic_granted(),
        },
        Perm {
            id: "bluetooth",
            granted: bluetooth_granted(),
        },
    ]
}

/// 打开某项对应的系统设置面板；未知 id 报错。
pub fn open(id: &str) -> Result<(), String> {
    if id == "ime" {
        super::ime::open_settings();
        return Ok(());
    }
    let pane = match id {
        "a11y" => "Privacy_Accessibility",
        "input_monitor" => "Privacy_ListenEvent",
        "speech" => "Privacy_SpeechRecognition",
        "mic" => "Privacy_Microphone",
        "bluetooth" => "Privacy_Bluetooth",
        _ => return Err(format!("未知授权项：{id}")),
    };
    let _ = std::process::Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{pane}"
        ))
        .spawn();
    Ok(())
}
