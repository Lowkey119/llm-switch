//! 设备档案（计划 `2026-11-19-遥控器设备档案与多设备扩展` §四）。
//!
//! **设备差异只许收容在这里**：`identity`（扫描/型号）、`hid`（认领/独占/报告）、
//! `buttons`（键表）一律不许出现 `if 某型号` 的分支——新增一款设备 = 加一条档案 +
//! 一份键表 + 一份单测。硬验收：`rg g9n9n src-tauri/src` 只许命中本文件与测试。
//!
//! 纯数据 + 纯逻辑，不带 cfg、不带 FFI，默认 `cargo test` 就编到并全测。
//!
//! `verified` = 本仓**真机**跑过（AGENTS 铁律：缺真机验收一律标候选，不得宣称支持）。
//! 档案里 `keys` 为空 = 键表未落地（P2/P6 再填），页面据此显示「未验证」而非支持。

use super::atvv::{FrameHeader, Keepalive, SessionSignal};
use super::buttons::RemoteButton;

/// 型号判定源（计划 §四）。读不到声明的源就显示「未识别」，**不许跨档案猜**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModelSource {
    /// DIS Model Number `0x2A24`（小米 RC001/RC003、G9N9N 实测有此特征）。
    Dis2A24,
    /// 没有 `0x2A24`（NimBLE HID 库只提供 manufacturer/PnP）→ 认广播名。
    AdvertiseName,
}

/// 原始报告格式（决定怎么从字节里取出 usage 集）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReportFormat {
    /// 键盘接口：`reportID` + 小端 u16 usage 对，0 = 空槽（小米 RC001/RC003）。
    KeyboardLe16 { report_id: u8 },
    /// Consumer Control 页：同样是小端 u16 usage 对（G9N9N 的 3 字节报告）。
    ConsumerLe16 { report_id: u8 },
    /// 鼠标报告（飞鼠类）：位移 + 按键位。P6 才接，P0 只占位。
    Mouse,
}

/// 独占策略。**鼠标类必须 `SharedOnly`**：`SeizeDevice` 会把系统光标/键盘抢走，
/// 用户设备当场失效（计划 §五 硬约束 1）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeizePolicy {
    ExclusiveFirst,
    SharedOnly,
}

/// ATVV 逐设备参数（计划 P3：这些**不是版本档位而是设备差**，写死任何一条
/// 都让第二款设备的接入要重做协议/解码/保活/度量四层验收）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AtvvSpec {
    /// GET_CAPS 请求模板（小米与 G9N9N 的字节串不一致，各自发各自的）。
    pub caps_request: &'static [u8],
    /// 帧长兜底值（GET_CAPS 响应声明优先）：小米 120 / G9N9N 128 / G20S 134。
    pub frame_bytes: u16,
    /// 帧头形状：无头（状态跳帧连续）/ 逐帧自带 DVI 状态（必须逐帧重置）。
    pub frame_header: FrameHeader,
    /// nibble 序：RC001/RC003 与 G9N9N 高半字节在前，ARN9 老固件低半字节在前。
    pub low_nibble_first: bool,
    /// 会话信号源：小米看 HID 语音键，G9N9N 看 ab5e0004。
    pub session_signal: SessionSignal,
    /// 保活方式：v1.0+ MIC_EXTEND，v0.4 只能重发 MIC_OPEN。
    pub keepalive: Keepalive,
    /// 松键时是否主动发 MIC_CLOSE。1 代实机首轮后不推流，暂按「等设备 AUDIO_STOP」处理。
    pub close_on_stop: bool,
}

/// 语音通道类型。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceSpec {
    /// Google Voice over BLE（ATVV）私有服务，参数由档案给。
    Atvv(&'static AtvvSpec),
    /// mSBC 塞在 HID 输入报告里传（小米「普通 1 代」，计划 §八）：
    /// 60 字节一包 = 状态字节 + H2 序号 + 57 字节 mSBC 帧（同步 `0xAD`），
    /// 起止由 20 字节报告的 01/00 给。**不走 ATVV，也没有 GATT 可订**。
    MsbcHid,
    /// 第三方固件的自定义 GATT（**按固件适配，不等于支持某品牌**，本期不实现）。
    CustomGatt,
    None,
}

/// 语音触发语义（页面文案与 S5 触发实现都按它走）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceTrigger {
    /// 按住说话、松开停止（小米、G9N9N）。
    HoldToTalk,
    /// 按一下开始、再按一下结束（T1 飞鼠实测）。
    Toggle,
    None,
}

/// 一款遥控器。字段全是编译期常量，**不做运行时插件、不做用户自定义协议**。
#[derive(Debug, PartialEq)]
pub struct DeviceProfile {
    /// 稳定 id，进持久化与 IPC。
    pub id: &'static str,
    /// i18n key（页面直读，不硬编码中文）。
    pub display: &'static str,
    /// 广播名白名单（大小写不敏感，连接前筛选 + 无 PnP 时的 HID 认领兜底）。
    pub names: &'static [&'static str],
    /// 扫描筛选服务（比名字可靠）。
    pub services: &'static [&'static str],
    /// HID 认领用 VID/PID；`None` = 设备不设 PnP，只能按产品名匹配。
    pub pnp: Option<(u16, u16)>,
    pub model: ModelSource,
    pub report: ReportFormat,
    /// 该档案的键表（`usage → 键`）。空 = 键表未落地（标未验证）。
    pub keys: &'static [(u16, RemoteButton)],
    pub seize: SeizePolicy,
    /// 持续位移报告（飞鼠）：只取按键、丢 X/Y（计划 §五 硬约束 3）。
    pub motion: bool,
    pub voice: VoiceSpec,
    pub voice_trigger: VoiceTrigger,
    /// 键位配置沿用哪条档案的仓（档案拆分/改名时用，避免用户已配的键位凭空消失）。
    pub config_from: Option<&'static str>,
    /// 该档案需要「Power 键设备级映射成 F20」才敢共享打开（小米专属坑，见 `powermap`）。
    pub power_map: bool,
    /// 本仓真机验过。false = 档案已登记但功能未验证，页面必须显示「未验证」。
    pub verified: bool,
}

/// 小米 RC001/RC003 键表（usage 事实来自本仓真机，既有计划 §三）。
/// 音量/返回是设备塞进键盘接口的非标准值，照实记。
pub(crate) static XIAOMI_KEYS: &[(u16, RemoteButton)] = &[
    (0x66, RemoteButton::Power),
    (0x52, RemoteButton::Up),
    (0x50, RemoteButton::Left),
    (0x28, RemoteButton::Ok),
    (0x4F, RemoteButton::Right),
    (0x51, RemoteButton::Down),
    (0xF1, RemoteButton::Back),
    (0x80, RemoteButton::VolumeUp),
    (0x4A, RemoteButton::Home),
    (0x81, RemoteButton::VolumeDown),
    (0x65, RemoteButton::Menu),
    (0x35, RemoteButton::Tv),
    // 语音键：2026-11-20 真机探针实录（键盘页 usage 0x3E，report_id 1）。
    // 设备级禁用表（powermap）已把它中和掉，系统侧不出事件，只进本仓事件管线。
    (0x3E, RemoteButton::Mic),
];

/// 小米 1 代键表：真机无 TV 键，其余与 RC001/RC003 一致。
pub(crate) static XIAOMI_KEYS_GEN1: &[(u16, RemoteButton)] = &[
    (0x66, RemoteButton::Power),
    (0x52, RemoteButton::Up),
    (0x50, RemoteButton::Left),
    (0x28, RemoteButton::Ok),
    (0x4F, RemoteButton::Right),
    (0x51, RemoteButton::Down),
    (0xF1, RemoteButton::Back),
    (0x80, RemoteButton::VolumeUp),
    (0x4A, RemoteButton::Home),
    (0x81, RemoteButton::VolumeDown),
    (0x65, RemoteButton::Menu),
    (0x3E, RemoteButton::Mic),
];

/// G9N9N 系 ATVV 参数（MIT 来源 vibe-mote 实测：16k / 128B 帧 / 无帧头 / 高 nibble 先）。
/// GY3LE 与 T1 无真机，暂借这份参数登记，档案 `verified = false` 兜住口径。
static ATVV_G9N9N: &AtvvSpec = &AtvvSpec {
    caps_request: &[0x0A, 0x00, 0x04, 0x00, 0x01],
    frame_bytes: 128,
    frame_header: FrameHeader::None,
    low_nibble_first: false,
    session_signal: SessionSignal::AtvvStatus,
    keepalive: Keepalive::MicExtend,
    close_on_stop: true,
};

/// ATVV 主服务（小米与 Google 系共用同一只，计划事实 5）。
const ATVV_SERVICE: &str = "AB5E0001-5A21-4F05-BC7D-AF01F617B664";

/// 全部已知档案（顺序即页面候选的稳定排序依据）。
pub static PROFILES: &[&DeviceProfile] = &[
    &DeviceProfile {
        id: "xiaomi_rc00x",
        display: "remote.profile.xiaomi",
        names: &[
            "mi rc",
            "xiaomi bluetooth remote 2",
            "xiaomi bluetooth remote 2 pro",
            // SayAll 真机实测（remote-mic-app VoiceRemoteCatalog 注释）：广播名「小米蓝牙语音遥控器」
            // 出厂就挂在 RC001/RC003（ATVV 机型）上，型号判定以 DIS 为准、名字不作型号证据。
            // 此前把它划给 gen1 导致 ATVV 设备被领进 mSBC 死路（macOS P7d 已证走死）。
            "小米蓝牙语音遥控器",
            "小米蓝牙遥控器2",
            "小米蓝牙遥控器2 pro",
            "arn9",
        ],
        services: &[ATVV_SERVICE],
        pnp: Some((0x2717, 0x32B8)),
        model: ModelSource::Dis2A24,
        report: ReportFormat::KeyboardLe16 { report_id: 1 },
        keys: XIAOMI_KEYS,
        seize: SeizePolicy::ExclusiveFirst,
        motion: false,
        voice: VoiceSpec::Atvv(&AtvvSpec {
            caps_request: &[0x0A, 0x01, 0x00, 0x00, 0x03, 0x03],
            frame_bytes: 120,
            frame_header: FrameHeader::None,
            low_nibble_first: false,
            session_signal: SessionSignal::HidUsage,
            keepalive: Keepalive::MicExtend,
            close_on_stop: true,
        }),
        voice_trigger: VoiceTrigger::HoldToTalk,
        config_from: None,
        power_map: true,
        verified: true,
    },
    &DeviceProfile {
        // 小米「普通 1 代」语音遥控器（计划 §八）。VID 同 rc00x 但 PID 不同：
        // 真机 1 代 0x32B0（固件 1.0.2.4），2 代 0x32B8（固件 2671）——写错 PID
        // 会让 HID 按 VID/PID 认领时永远落到 rc00x，按键热点全不亮（2026-11-27 实测）。
        // 语音走与 2 代相同的 ATVV：无头帧、高半字节优先。真机已验收按键、
        // 连续语音会话、音频帧与文字上屏；松键不主动 MIC_CLOSE（等设备停流），
        // 否则首轮之后不再推流（open-voice-bridge 同款做法）。
        id: "xiaomi_gen1",
        display: "remote.profile.gen1",
        // 真机无 TV 键：用独立键表，UI 与映射都不显示 TV。
        // 「小米蓝牙语音遥控器」归 rc00x（真机实测为 ATVV 机型）；1 代真机
        // 广播名是「小米语音遥控器」，加入白名单才能被扫描与 HID 认领（P7a 取证）。
        names: &["小米语音遥控器"],
        // ATVV 服务与 2 代相同；扫描以广播名认领，服务作为辅助命中。
        services: &[ATVV_SERVICE],
        pnp: Some((0x2717, 0x32B0)),
        model: ModelSource::Dis2A24,
        report: ReportFormat::KeyboardLe16 { report_id: 1 },
        keys: XIAOMI_KEYS_GEN1,
        seize: SeizePolicy::ExclusiveFirst,
        motion: false,
        voice: VoiceSpec::Atvv(&AtvvSpec {
            caps_request: &[0x0A, 0x01, 0x00, 0x00, 0x03, 0x03],
            frame_bytes: 120,
            frame_header: FrameHeader::None,
            low_nibble_first: false,
            session_signal: SessionSignal::HidUsage,
            keepalive: Keepalive::MicExtend,
            close_on_stop: false,
        }),
        voice_trigger: VoiceTrigger::HoldToTalk,
        config_from: Some("xiaomi_rc00x"),
        power_map: true,
        verified: true,
    },
    &DeviceProfile {
        // 峰米蓝牙语音遥控器（小米生态链，VID 同小米 0x2717，PID 0x32BE，用户 2026-11-21 真机接入请求）。
        // 语音通道未实测：按同门 ATVV 猜测登记（mic_open 写不中特征只会报错，无害）；
        // 真机日志见音频帧到达再转 verified，键表 usage 若对不上会进「未映射 usage」台账再校。
        id: "fengmi_voice",
        display: "remote.profile.xiaomi",
        names: &["峰米蓝牙语音遥控器"],
        services: &[ATVV_SERVICE],
        pnp: Some((0x2717, 0x32BE)),
        model: ModelSource::Dis2A24,
        report: ReportFormat::KeyboardLe16 { report_id: 1 },
        keys: XIAOMI_KEYS,
        seize: SeizePolicy::ExclusiveFirst,
        motion: false,
        voice: VoiceSpec::Atvv(&AtvvSpec {
            caps_request: &[0x0A, 0x01, 0x00, 0x00, 0x03, 0x03],
            frame_bytes: 120,
            frame_header: FrameHeader::None,
            low_nibble_first: false,
            session_signal: SessionSignal::HidUsage,
            keepalive: Keepalive::MicExtend,
            close_on_stop: true,
        }),
        voice_trigger: VoiceTrigger::HoldToTalk,
        config_from: Some("xiaomi_rc00x"),
        power_map: true,
        verified: false,
    },
    &DeviceProfile {
        // Chromecast with Google TV Voice Remote（2020）。兼容款批次差到硬件层，
        // 键表与批次判定归 P2，这里先把认领与能力登记清楚。
        id: "g9n9n",
        display: "remote.profile.g9n9n",
        names: &["chromecast remote", "google tv remote"],
        services: &[ATVV_SERVICE],
        pnp: Some((0x18D1, 0x9450)),
        model: ModelSource::Dis2A24,
        report: ReportFormat::ConsumerLe16 { report_id: 2 },
        keys: &[],
        seize: SeizePolicy::ExclusiveFirst,
        motion: false,
        voice: VoiceSpec::Atvv(ATVV_G9N9N),
        voice_trigger: VoiceTrigger::HoldToTalk,
        config_from: None,
        power_map: false,
        verified: false,
    },
    &DeviceProfile {
        // Google TV Streamer Voice Remote。同类项目至今未验证 → 只登记不宣称支持。
        id: "gy3le",
        display: "remote.profile.gy3le",
        names: &["google tv remote"],
        services: &[ATVV_SERVICE],
        pnp: None,
        model: ModelSource::AdvertiseName,
        report: ReportFormat::ConsumerLe16 { report_id: 2 },
        keys: &[],
        seize: SeizePolicy::ExclusiveFirst,
        motion: false,
        voice: VoiceSpec::Atvv(ATVV_G9N9N),
        voice_trigger: VoiceTrigger::HoldToTalk,
        config_from: None,
        power_map: false,
        verified: false,
    },
    &DeviceProfile {
        // T1 飞鼠键盘语音遥控器：语音是 toggle 语义（不必按住），且带持续位移报告。
        id: "t1_airmouse",
        display: "remote.profile.t1",
        names: &["t1-remote", "t1 remote"],
        services: &[ATVV_SERVICE],
        pnp: None,
        model: ModelSource::AdvertiseName,
        report: ReportFormat::Mouse,
        keys: &[],
        // 飞鼠带鼠标接口 → 禁独占（抢走光标就把用户设备废了）。
        seize: SeizePolicy::SharedOnly,
        motion: true,
        voice: VoiceSpec::Atvv(ATVV_G9N9N),
        voice_trigger: VoiceTrigger::Toggle,
        config_from: None,
        power_map: false,
        verified: false,
    },
    &DeviceProfile {
        // M5Stack 一类 DIY 飞鼠固件（AirMouseS3 / spacecdr-AirMouse / spellstick）。
        // 标准 HOGP，无 ATVV；语音是各家自定义 GATT，只能按固件适配。
        id: "diy_airmouse",
        display: "remote.profile.diy_airmouse",
        names: &["airstick", "airmouse", "stickmouse", "air mouse"],
        services: &[],
        pnp: Some((0x303A, 0x4001)),
        model: ModelSource::AdvertiseName,
        report: ReportFormat::Mouse,
        keys: &[],
        seize: SeizePolicy::SharedOnly,
        motion: true,
        voice: VoiceSpec::CustomGatt,
        voice_trigger: VoiceTrigger::None,
        config_from: None,
        power_map: false,
        verified: false,
    },
];

/// 未知档案 id 的兜底（持久化里出现已删除档案时不炸）。
pub const FALLBACK: &DeviceProfile = PROFILES[0];

pub fn by_id(id: &str) -> &'static DeviceProfile {
    PROFILES
        .iter()
        .copied()
        .find(|p| p.id == id)
        .unwrap_or(FALLBACK)
}

/// 广播名命中哪条档案（大小写不敏感、容忍尾随空格）。
pub fn by_name(name: Option<&str>) -> Option<&'static DeviceProfile> {
    let n = name?.trim().to_lowercase();
    if n.is_empty() {
        return None;
    }
    PROFILES
        .iter()
        .copied()
        .find(|p| p.names.iter().any(|a| *a == n))
}

/// HID 枚举里的设备认领（计划事实 21 + §八 1 代教训）。
///
/// 同一对 VID/PID 下可以挂着**多款语音通道完全不同的设备**——小米 1 代与
/// RC001/RC003 都是 `0x2717:0x32B8`，前者语音是 mSBC 塞 HID 报告、后者是 ATVV。
/// 原先一律「VID/PID 命中即返回」，同 PID 的第二款永远拿不到认领权，只能落到
/// `PROFILES` 里排前面的那条，于是 1 代被当 rc00x 认领、发 ATVV `MIC_OPEN` 收不到东西。
/// 现在同 PID 的候选里**产品名能对上就按名字**，名字对不上才退回 PID 命中的第一条。
pub fn by_pnp(
    vendor: u16,
    product: u16,
    product_name: Option<&str>,
) -> Option<&'static DeviceProfile> {
    let pool: Vec<&'static DeviceProfile> = PROFILES
        .iter()
        .copied()
        .filter(|p| p.pnp == Some((vendor, product)))
        .collect();
    if let Some(p) = by_name(product_name).filter(|p| pool.contains(p)) {
        return Some(p);
    }
    pool.into_iter()
        .next()
        // VID/PID 全 0 或未登记（DIY 固件常不设 PnP）→ 退产品名。
        .or_else(|| {
            if vendor == 0 || product == 0 {
                by_name(product_name)
            } else {
                None
            }
        })
        .or_else(|| by_name(product_name).filter(|p| p.pnp.is_none()))
}

/// 扫描筛选服务的并集（去重后交给 BLE 扫描）。
pub fn scan_services() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for p in PROFILES {
        for s in p.services {
            if !out.contains(s) {
                out.push(*s);
            }
        }
    }
    out
}

impl DeviceProfile {
    /// 该档案是否已落地可用（键表非空 + 真机验过）。
    pub fn supported(&self) -> bool {
        self.verified && !self.keys.is_empty()
    }

    /// hid 通道认不认领这台设备：**键表落地就认领**。
    /// 不能直接用 `supported()`——那会把「语音没验过」连带成「按键也不管了」，
    /// 而键表与语音通道是两件独立的事（计划 §八 影响 3）。
    pub fn claimable(&self) -> bool {
        !self.keys.is_empty()
    }

    /// 本档案键盘/Consumer 接口的 report id（鼠标类为 None）。
    ///
    /// mSBC/HID 通道靠它把语音报告和按键报告隔开：语音帧一定走另一个 report id，
    /// 同 id 的报告一律先走键表。空键盘报告也是「首字节 0 + 其余补 0」，
    /// 万一该接口报告长度正好撞上控制报告的形状，不隔开就会在半句话中间停掉会话。
    pub fn key_report_id(&self) -> Option<u8> {
        match self.report {
            ReportFormat::KeyboardLe16 { report_id } | ReportFormat::ConsumerLe16 { report_id } => {
                Some(report_id)
            }
            ReportFormat::Mouse => None,
        }
    }

    /// usage → 键（按本档案键表，**不跨档案查**）。
    pub fn key_of(&self, usage: u16) -> Option<RemoteButton> {
        self.keys.iter().find(|(u, _)| *u == usage).map(|(_, b)| *b)
    }

    /// 原始报告 → usage 集。鼠标类报告 P6 才解析，这里一律 None。
    pub fn parse_report(&self, report_id: u8, payload: &[u8]) -> Option<Vec<u16>> {
        let want = match self.report {
            ReportFormat::KeyboardLe16 { report_id: r }
            | ReportFormat::ConsumerLe16 { report_id: r } => r,
            ReportFormat::Mouse => return None,
        };
        super::buttons::parse_usages(want, report_id, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<&str> = PROFILES.iter().map(|p| p.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n);
    }

    #[test]
    fn xiaomi_table_covers_every_key() {
        assert_eq!(XIAOMI_KEYS.len(), RemoteButton::ALL.len());
        for b in RemoteButton::ALL {
            assert!(XIAOMI_KEYS.iter().any(|(_, k)| *k == b), "{b:?}");
        }
        let p = by_id("xiaomi_rc00x");
        assert_eq!(p.key_of(0x28), Some(RemoteButton::Ok));
        assert_eq!(p.key_of(0xF1), Some(RemoteButton::Back));
        assert_eq!(p.key_of(0x99), None);
        // 1 代真机无 TV 键，不得映射 usage 0x35。
        assert_eq!(by_id("xiaomi_gen1").key_of(0x35), None);
        assert!(p.supported());
    }

    #[test]
    fn same_pnp_profiles_split_by_product_name() {
        // 同 VID 下 1 代 PID 0x32B0、RC001/RC003 PID 0x32B8；名字能对上就按名字，
        // 不许一律落到 PROFILES 里排前面的那条（那等于把两款混成一款）。
        assert_eq!(
            by_pnp(0x2717, 0x32B8, Some("MI RC")).unwrap().id,
            "xiaomi_rc00x"
        );
        // 「小米蓝牙语音遥控器」经 SayAll 真机实测是 RC001/RC003（ATVV）出厂广播名；
        // gen1 的广播名是「小米语音遥控器」，且 PID 不同，不会撞到这里。
        assert_eq!(
            by_pnp(0x2717, 0x32B8, Some("小米蓝牙语音遥控器"))
                .unwrap()
                .id,
            "xiaomi_rc00x"
        );
        // 名字对不上仍退回 PID 命中的第一条，认领不因名字千奇百怪而失守。
        assert_eq!(
            by_pnp(0x2717, 0x32B8, Some("MI-RC-MISSION")).unwrap().id,
            "xiaomi_rc00x"
        );
        // 没登记过的 VID/PID 不得乱认领。
        assert!(by_pnp(0xFFFF, 0xFFFF, Some("不存在的遥控器")).is_none());
    }

    #[test]
    fn unverified_profiles_are_not_supported() {
        // 没键表或没真机 → 一律「未验证」，不得显示为支持。
        // rc00x/gen1 均已完成真机验收（2026-11-27 含语音与文字上屏）。
        for p in PROFILES {
            if p.id == "xiaomi_rc00x" || p.id == "xiaomi_gen1" {
                continue;
            }
            assert!(!p.supported(), "{} 未落地却报支持", p.id);
        }
    }

    #[test]
    fn mouse_profiles_must_never_seize() {
        // 计划 §五 硬约束 1：带位移报告的设备禁独占。
        for p in PROFILES {
            if p.motion {
                assert!(
                    matches!(p.seize, SeizePolicy::SharedOnly),
                    "{} 是鼠标类却允许独占",
                    p.id
                );
            }
        }
    }

    #[test]
    fn name_matching_is_case_and_space_tolerant() {
        assert_eq!(by_name(Some("Mi RC")).map(|p| p.id), Some("xiaomi_rc00x"));
        assert_eq!(
            by_name(Some(" AirMouse ")).map(|p| p.id),
            Some("diy_airmouse")
        );
        assert_eq!(by_name(Some("AirPods")).map(|p| p.id), None);
        assert_eq!(by_name(None), None);
    }

    #[test]
    fn pnp_matching_prefers_ids_and_falls_back_to_name() {
        assert_eq!(
            by_pnp(0x2717, 0x32B8, None).map(|p| p.id),
            Some("xiaomi_rc00x")
        );
        assert_eq!(by_pnp(0x18D1, 0x9450, None).map(|p| p.id), Some("g9n9n"));
        // DIY 固件不设 PnP（VID/PID 全 0）→ 只能按产品名。
        assert_eq!(
            by_pnp(0, 0, Some("AirStick")).map(|p| p.id),
            Some("diy_airmouse")
        );
        // 陌生 VID/PID 不许靠名字硬认（防把普通鼠标认成遥控器）。
        assert_eq!(
            by_pnp(0x046D, 0xC52B, Some("MX Master")).map(|p| p.id),
            None
        );
    }

    #[test]
    fn scan_services_union_dedups() {
        let s = scan_services();
        assert_eq!(s.len(), 1, "目前只有 ATVV 一只服务，重复说明并集写错了");
        assert_eq!(s[0], ATVV_SERVICE);
    }

    #[test]
    fn report_parse_follows_profile_format() {
        let xiaomi = by_id("xiaomi_rc00x");
        // 键盘接口 reportID=1：OK + 上。
        assert_eq!(
            xiaomi.parse_report(1, &[0x28, 0x00, 0x52, 0x00]),
            Some(vec![0x28, 0x52])
        );
        // reportID 不对 = 不是本档案的报告。
        assert_eq!(xiaomi.parse_report(2, &[0x28, 0x00]), None);
        // 鼠标类档案不解析（P6 才接）。
        assert_eq!(by_id("diy_airmouse").parse_report(1, &[0x28, 0x00]), None);
    }

    #[test]
    fn key_report_id_separates_keys_from_voice_reports() {
        assert_eq!(by_id("xiaomi_rc00x").key_report_id(), Some(1));
        assert_eq!(by_id("xiaomi_gen1").key_report_id(), Some(1));
        assert_eq!(by_id("g9n9n").key_report_id(), Some(2));
        // 鼠标类没有固定的按键 report id。
        assert_eq!(by_id("t1_airmouse").key_report_id(), None);
    }

    #[test]
    fn unknown_id_falls_back_without_panicking() {
        assert_eq!(by_id("no_such_profile").id, FALLBACK.id);
    }
}
