//! 设备级按键改写表（仅共享模式需要，共享开启前整表落定，失败关闭）。
//!
//! 为什么共享模式必须做在设备级：系统同时收到遥控器的原生 HID 键，键盘 tap 的
//! 账本时序窗有漏键（事件到达顺序与时间窗对不上就透传出去，用户实测返回键等仍干扰）。
//! UserKeyMapping 直接把键在 HID 层改写：destination=0 = 系统确定性收不到，
//! 而原始报告不受影响——我们的读线程照常看到按下。清单按本仓 XIAOMI_KEYS 真机键表
//! 逐键定（SayAll 的清单不含本设备返回键 0xF1，照抄会漏）；方向键（0x4F-0x52）
//! 保持透传（导航刚需），音量 0x80/0x81 保持透传（本设备音量走键盘页，禁了音量就废）。
//! 电源键 0x66 不置 0 而是改写为 F20：不映射则一按整机睡眠（旧仓教训）；
//! 语音键 0x3E 的去向随路 2 开关：禁用（置 0）或改写为硬件 🌐（微信输入法语音只认硬件 Fn）。
//! 本仓 `unsafe_code = "forbid"`，IOHIDServiceClient SPI 够不到——改用 Apple
//! 官方 `hidutil`（同一 UserKeyMapping 机制，`--matching` 按 VID/PID 设备级作用域）。
//!
//! 失败关闭：设置后回读校验全部条目，不通过则调用方不得开共享通道。
//! 崩溃残留提示：映射残留期间遥控器的 Ok/返回/主页/TV/菜单对系统失效（重连即恢复），
//! 正常退出路径会清除。

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

/// HID usage 合成值：page << 32 | usage（Apple hidutil 官方示例与 SayAll 同款编码）。
/// 曾用 page << 16：hidutil 回读在位但驱动不认——2026-11-21 实测 SayAll（page<<32）
/// 在位时语音键唤 HUD 正常、只有我们的表在位时 HUD 不出，定案驱动只认 page<<32。
const KEYBOARD_PAGE: u64 = 0x0007;
const USAGE_POWER: u64 = 0x0066;
const USAGE_F20: u64 = 0x006F;
const SRC_POWER: u64 = (KEYBOARD_PAGE << 32) | USAGE_POWER;
const DST_F20: u64 = (KEYBOARD_PAGE << 32) | USAGE_F20;
/// 语音键（mic，键盘页 0x3E，2026-11-20 真机捕获）改写目标之一：Apple top-case
/// Fn/Globe（page 0xFF usage 3，SayAll 同款）。路 2 把语音键改成硬件 🌐——
/// 微信输入法语音（按住说话式）只认硬件 Fn，合成 CGEvent 不认（2026-11-21 实测：
/// 合成按住与合成点触都唤不出 HUD，SayAll 默认口径正是 UserKeyMapping 硬件改写）。
const USAGE_VOICE: u64 = 0x003E;
const SRC_VOICE: u64 = (KEYBOARD_PAGE << 32) | USAGE_VOICE;
const DST_FN: u64 = 0x00FF_0000_0003;

/// 共享模式设备级禁用清单（page 0x07 usage，小米键表实测值，见 profile.rs XIAOMI_KEYS）：
/// 0x28=Ok(Enter)、0x35=TV、0x4A=主页、0x65=菜单、0xF1=返回（非标准值）。
/// 音量 0x80/0x81 **不禁**：本设备音量走键盘页这两个非标准值，禁了音量键就废
/// （SayAll 禁它们是因为它自己重注入音量，我们没有注入路径，保持系统透传）。
/// 语音键 0x3E 不在这里——它单独一条，去向随路 2 开关在 0（禁用）与 Fn 之间切换。
const DISABLE_USAGES: [u64; 5] = [0x28, 0x35, 0x4A, 0x65, 0xF1];

/// 路 2 在位时语音键是否改写为硬件 🌐（voice 模块随语音偏好置；false = 禁用置 0）。
static VOICE_TO_FN: AtomicBool = AtomicBool::new(false);
/// 改写表当前是否在位（apply 成功置位、clear 撤位）：语音键换向只就地重写已在位的表，
/// 共享模式没开就不碰表（避免不该设备禁键时禁了键）。
static APPLIED: AtomicBool = AtomicBool::new(false);

/// 设定语音键去向；调用方随后 reapply_if_active()，表在位即就地跟上。
pub fn set_voice_to_fn(on: bool) {
    VOICE_TO_FN.store(on, Ordering::SeqCst);
}

/// 改写表在位时按当前口径重写（语音键换向的即时通道）；不在位什么都不做，
/// 共享通道开启或看门狗补表时自然按新口径落定。
pub fn reapply_if_active() {
    if APPLIED.load(Ordering::SeqCst) {
        if let Err(e) = apply_share_key_map() {
            tracing::warn!(target: "remote::powermap", "语音键换向重写改写表失败：{e}");
        }
    }
}

/// 小米语音遥控器匹配串（RC001/RC003 同 VID/PID：0x2717=10007 / 0x32B8=12984，计划 §三）。
const MATCHING: &str = r#"{"VendorID":10007,"ProductID":12984}"#;

fn hidutil(args: &[&str]) -> Result<String, String> {
    let out = Command::new("/usr/bin/hidutil")
        .args(args)
        .output()
        .map_err(|e| format!("hidutil 启动失败：{e}"))?;
    if !out.status.success() {
        return Err(format!(
            "hidutil 失败：{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn entry(src: u64, dst: u64) -> String {
    format!(r#"{{"HIDKeyboardModifierMappingSrc":{src},"HIDKeyboardModifierMappingDst":{dst}}}"#)
}

/// 期望全表：禁用键 → 0，语音键 → Fn（路 2 开）或 0（关），电源键 → F20。
fn desired(voice_to_fn: bool) -> Vec<(u64, u64)> {
    let mut v: Vec<(u64, u64)> = DISABLE_USAGES
        .iter()
        .map(|u| ((KEYBOARD_PAGE << 32) | u, 0))
        .collect();
    v.push((SRC_VOICE, if voice_to_fn { DST_FN } else { 0 }));
    v.push((SRC_POWER, DST_F20));
    v
}

/// 回读解析：hidutil 打印为每条目一对 `MappingSrc = N;` / `MappingDst = N;`
/// （实测十进制；十六进制 `0x…` 一并认）。RegistryID 等杂行按 key 名过滤，不数错对。
fn parse_pairs(out: &str) -> Vec<(u64, u64)> {
    fn val(s: &str) -> Option<u64> {
        let s = s
            .trim()
            .trim_start_matches('=')
            .trim()
            .trim_end_matches(';');
        match s.strip_prefix("0x").or(s.strip_prefix("0X")) {
            Some(h) => u64::from_str_radix(h, 16).ok(),
            None => s.parse().ok(),
        }
    }
    let mut pairs = Vec::new();
    let mut src = None;
    let mut dst = None;
    for line in out.lines() {
        if let Some((_, v)) = line.split_once("HIDKeyboardModifierMappingSrc") {
            src = val(v);
        } else if let Some((_, v)) = line.split_once("HIDKeyboardModifierMappingDst") {
            dst = val(v);
        }
        if let (Some(s), Some(d)) = (src, dst) {
            pairs.push((s, d));
            src = None;
            dst = None;
        }
    }
    pairs
}

/// 应用共享模式按键改写表并回读校验全部条目（设备必须在场，否则 matching 落空 → 校验不过）。
pub fn apply_share_key_map() -> Result<(), String> {
    let want = desired(VOICE_TO_FN.load(Ordering::SeqCst));
    let items: Vec<String> = want.iter().map(|(s, d)| entry(*s, *d)).collect();
    let set = format!(r#"{{"UserKeyMapping":[{}]}}"#, items.join(","));
    hidutil(&["property", "--matching", MATCHING, "--set", &set])?;
    let got = hidutil(&[
        "property",
        "--matching",
        MATCHING,
        "--get",
        "UserKeyMapping",
    ])?;
    let have = parse_pairs(&got);
    if let Some((s, _)) = want.iter().find(|p| !have.contains(p)) {
        let e = format!(
            "按键改写回读校验未通过（{:X} 未生效，遥控器是否已连接？）",
            s & 0xFFFF
        );
        tracing::warn!(target: "remote::powermap", "{e}");
        return Err(e);
    }
    tracing::info!(target: "remote::powermap", n = want.len(), "按键改写表已落定并回读校验通过");
    APPLIED.store(true, Ordering::SeqCst);
    Ok(())
}

/// 改写表是否缺失（共享模式看门狗用）：任一条目不在位即算缺。
/// 表可被外部抹掉（设备重连丢属性、其他 app 整表覆盖），丢了电源键就直通关机。
pub fn share_key_map_missing() -> bool {
    let got = match hidutil(&[
        "property",
        "--matching",
        MATCHING,
        "--get",
        "UserKeyMapping",
    ]) {
        Ok(g) => g,
        Err(_) => return true,
    };
    let have = parse_pairs(&got);
    desired(VOICE_TO_FN.load(Ordering::SeqCst))
        .iter()
        .any(|p| !have.contains(p))
}

/// 清除映射（共享通道收尾尽力而为；进程异常退出时映射会残留——
/// 后果是遥控器被禁键对系统失效，设备重连或重启即消失，本机键盘不受影响）。
pub fn clear_share_key_map() {
    let _ = hidutil(&[
        "property",
        "--matching",
        MATCHING,
        "--set",
        r#"{"UserKeyMapping":[]}"#,
    ]);
    APPLIED.store(false, Ordering::SeqCst);
    tracing::info!(target: "remote::powermap", "按键改写表已清除");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_constants_match_old_repo() {
        // 旧仓 RemoteVoiceFunctionMapper：0x0000_0007_0000_0066 → F20（page<<32 编码逐位一致）。
        assert_eq!(SRC_POWER, 0x0000_0007_0000_0066);
        assert_eq!(DST_F20, 0x0000_0007_0000_006F);
    }

    #[test]
    fn desired_matches_xiaomi_keytable_policy() {
        // 除方向键（0x4F-0x52 导航刚需）与音量（0x80/0x81 本设备走键盘页，禁了音量就废）
        // 外，XIAOMI_KEYS 全部键都要禁用；电源键改写 F20。
        let want = desired(false);
        let usages: Vec<u64> = want.iter().map(|(s, _)| s & 0xFFFF).collect();
        for u in [0x28u64, 0x35, 0x3E, 0x4A, 0x65, 0x66, 0xF1] {
            assert!(usages.contains(&u), "缺 {u:X}");
        }
        assert!(!usages.iter().any(|u| (0x4F..=0x52).contains(u)));
        assert!(!usages.contains(&0x80) && !usages.contains(&0x81));
        // 路 2 关：只有电源键改写为 F20，其余（含语音键）全部置 0。
        assert_eq!(want.iter().filter(|(_, d)| *d != 0).count(), 1);
    }

    #[test]
    fn voice_key_to_fn_maps_to_apple_globe() {
        // 路 2 开：语音键 0x3E → Apple top-case Fn/Globe（SayAll 同款 0xFF00000003），
        // 非 0 条目变成两条（电源 F20 + 语音 Fn）。
        let want = desired(true);
        assert!(want.contains(&(SRC_VOICE, DST_FN)));
        assert_eq!(want.iter().filter(|(_, d)| *d != 0).count(), 2);
        assert!(desired(false).contains(&(SRC_VOICE, 0)));
    }

    #[test]
    fn parse_pairs_reads_hidutil_output() {
        // 2026-11-20 实测回读样本（十进制，Dst 在 Src 前）。
        let sample = "RegistryID  Key                   Value\n100ac0409   UserKeyMapping   (\n        {\n        HIDKeyboardModifierMappingDst = 458863;\n        HIDKeyboardModifierMappingSrc = 458854;\n    }\n)";
        assert_eq!(parse_pairs(sample), vec![(0x0007_0066, 0x0007_006F)]);
        // 空表 / 无输出 → 空集（校验会判缺失，失败关闭）。
        assert!(parse_pairs("UserKeyMapping : ( )").is_empty());
    }
}
