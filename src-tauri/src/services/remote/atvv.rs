//! ATVV（Google Voice over BLE）协议层——纯逻辑，无 cfg、无 FFI，默认 `cargo test` 全测。
//!
//! 事实来源（clean-room：只取协议事实，不搬任何 GPL 代码）：Google 公开
//! 《Voice over BLE spec v1.0》+ MIT 同类实现（ATVVoice / vibe-mote）+ 本仓真机记录。
//!
//! 三特征分工：
//! - `ab5e0002` 命令（写 GET_CAPS、MIC_OPEN/CLOSE/EXTEND）
//! - `ab5e0003` 音频（ADPCM 帧通知）
//! - `ab5e0004` 状态（语音键请求开麦、流起止、解码器重同步）
//!
//! **逐设备差异一律走档案**（计划 2026-11-19 §一·五）：GET_CAPS 请求模板、帧长、
//! 帧头形状、nibble 序、保活方式、会话信号源都是 `profile::AtvvSpec` 的字段，
//! 本文件里出现 `if 某型号` 就是缺陷。

/// ATVV 主服务（小米与 Google 系共用同一只）。
pub const SERVICE: &str = "AB5E0001-5A21-4F05-BC7D-AF01F617B664";
/// 命令特征（写）。
pub const CH_SEND: &str = "AB5E0002-5A21-4F05-BC7D-AF01F617B664";
/// 音频数据特征（通知）。
pub const CH_AUDIO: &str = "AB5E0003-5A21-4F05-BC7D-AF01F617B664";
/// 状态/控制特征（通知）。
pub const CH_STATUS: &str = "AB5E0004-5A21-4F05-BC7D-AF01F617B664";

// —— 主机 → 设备命令 ——
const CMD_MIC_OPEN: u8 = 0x0C;
const CMD_MIC_CLOSE: u8 = 0x0D;
const CMD_MIC_EXTEND: u8 = 0x0E;
// —— 设备 → 主机控制消息（ab5e0004）——
const EVT_REQUEST_OPEN: u8 = 0x08;
const EVT_STREAM_START: u8 = 0x04;
const EVT_STREAM_STOP: u8 = 0x00;
const EVT_ADPCM_RESYNC: u8 = 0x0A;
const RSP_CAPABILITIES: u8 = 0x0B;

/// IMA/DVI ADPCM 步长表（标准 89 项，全行业同一张）。
static STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66,
    73, 80, 88, 97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449,
    494, 544, 598, 658, 724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272,
    2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493,
    10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794, 32767,
];

/// 保活方式（计划事实 8）：v1.0+ 用 MIC_EXTEND（静默、不打断流）；
/// v0.4 不支持 EXTEND，只能重发 MIC_OPEN（设备回 AUDIO_START 并重置序号）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Keepalive {
    MicExtend,
    ReopenMicOpen,
}

/// 会话信号源（谁说了算）：小米看 HID 语音键（usage 0x3E）按下/松开；
/// G9N9N 看 ab5e0004 的 0x04/0x00。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionSignal {
    HidUsage,
    AtvvStatus,
}

/// 帧头形状：无头（解码器状态跳帧连续）/ 逐帧自带 DVI 预测器与步长。
/// 后者必须逐帧重置——沿用上一帧状态第 3 帧就发散爆音（ATVVoice 实测）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameHeader {
    None,
    /// 前 4 字节 = predictor(i16 小端) + step_index(u8) + 保留，其余是 ADPCM。
    DviState,
}

/// GET_CAPS 响应（`0B` 打头，多字节字段大端）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capabilities {
    /// 大端版本号，v1.0 = `0x0100`。
    pub version: u16,
    /// 设备支持的 codec 位图；`& 0x02` = 16 kHz，否则 8 kHz。
    pub codecs: u16,
    /// 设备声明的帧长（字节）；0 = 未声明，调用方回退档案值。
    pub frame_bytes: u16,
}

impl Capabilities {
    /// 解析 GET_CAPS 响应；魔数不对或长度不足返回 None（不猜）。
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.first() != Some(&RSP_CAPABILITIES) || b.len() < 7 {
            return None;
        }
        Some(Self {
            version: u16::from_be_bytes([b[1], b[2]]),
            codecs: u16::from_be_bytes([b[3], b[4]]),
            frame_bytes: u16::from_be_bytes([b[5], b[6]]),
        })
    }

    /// 协商出的采样率（codec 位图决定；不做「只收 16k」硬门禁，档案说了算）。
    pub fn sample_rate(&self) -> u32 {
        if self.codecs & 0x02 != 0 {
            16_000
        } else {
            8_000
        }
    }

    /// v≥0x0100 才支持 MIC_EXTEND。
    pub fn supports_extend(&self) -> bool {
        self.version >= 0x0100
    }
}

/// MIC_OPEN：v1.0 起 codec 可省，v0.4 必须带上（字节序发错会被 `0C 0F 01` 拒）。
pub fn mic_open(codec: Option<u16>) -> Vec<u8> {
    let mut v = vec![CMD_MIC_OPEN, 0x00];
    if let Some(c) = codec {
        v.extend_from_slice(&c.to_be_bytes());
    }
    v
}

/// MIC_CLOSE <sessionID>。
pub fn mic_close(session: u8) -> Vec<u8> {
    vec![CMD_MIC_CLOSE, session]
}

/// MIC_EXTEND <sessionID>（仅 v≥0x0100）。
pub fn mic_extend(session: u8) -> Vec<u8> {
    vec![CMD_MIC_EXTEND, session]
}

/// 设备侧 ab5e0004 控制消息。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    /// 设备主动请求开麦（语音键按下）。
    RequestOpen,
    /// 音频流开始，带会话号。
    StreamStart { session: u8 },
    /// 音频流停止（松开或设备侧超时）。
    StreamStop,
    /// ADPCM 解码器重同步（丢帧后设备把当前状态推过来）。
    Resync { predictor: i16, step_index: usize },
}

/// 解析 ab5e0004 通知；认不出的一律 None（固件杂音不得当成事件）。
pub fn parse_control(b: &[u8]) -> Option<Control> {
    match *b.first()? {
        EVT_REQUEST_OPEN => Some(Control::RequestOpen),
        EVT_STREAM_START => Some(Control::StreamStart {
            session: *b.get(3).unwrap_or(&0),
        }),
        EVT_STREAM_STOP => Some(Control::StreamStop),
        EVT_ADPCM_RESYNC if b.len() >= 7 => Some(Control::Resync {
            predictor: i16::from_le_bytes([b[4], b[5]]),
            step_index: usize::from(b[6] % 89),
        }),
        _ => None,
    }
}

/// IMA/DVI ADPCM 解码器（4:1，单声道）。
///
/// nibble 序按型号翻转（RC001/RC003 高半字节在前，ARN9 老固件低半字节在前），
/// 由档案一次性给定，不能省——解反了整段是噪声。
#[derive(Clone, Debug)]
pub struct AdpcmDecoder {
    predictor: i16,
    step_index: usize,
}

impl Default for AdpcmDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl AdpcmDecoder {
    pub fn new() -> Self {
        Self {
            predictor: 0,
            step_index: 0,
        }
    }

    /// 设备侧重同步（0x0A）：把解码器状态钉回设备那一侧的真值。
    pub fn resync(&mut self, predictor: i16, step_index: usize) {
        self.predictor = predictor;
        self.step_index = step_index.min(88);
    }

    pub fn state(&self) -> (i16, usize) {
        (self.predictor, self.step_index)
    }

    /// 解一帧 ADPCM → PCM 样本。`header` 决定帧头是否自带解码器状态。
    pub fn decode(
        &mut self,
        frame: &[u8],
        header: FrameHeader,
        low_nibble_first: bool,
    ) -> Vec<i16> {
        let body = match header {
            FrameHeader::None => frame,
            FrameHeader::DviState => {
                if frame.len() < 4 {
                    return Vec::new();
                }
                self.resync(
                    i16::from_le_bytes([frame[0], frame[1]]),
                    usize::from(frame[2] % 89),
                );
                &frame[4..]
            }
        };
        let mut out = Vec::with_capacity(body.len() * 2);
        for byte in body {
            let (first, second) = if low_nibble_first {
                (byte & 0x0F, byte >> 4)
            } else {
                (byte >> 4, byte & 0x0F)
            };
            out.push(self.step(first));
            out.push(self.step(second));
        }
        out
    }

    /// 单个 4 bit 码 → 一个样本（标准 IMA 递推）。
    fn step(&mut self, code: u8) -> i16 {
        let step = STEP_TABLE[self.step_index.min(88)];
        let mut diff = step >> 3;
        if code & 4 != 0 {
            diff += step;
        }
        if code & 2 != 0 {
            diff += step >> 1;
        }
        if code & 1 != 0 {
            diff += step >> 2;
        }
        if code & 8 != 0 {
            diff = -diff;
        }
        let next = (i32::from(self.predictor) + diff).clamp(i16::MIN as i32, i16::MAX as i32);
        self.predictor = next as i16;
        // 步长索引走标准 indexTable：幅度位（低 3 位）<4 降一档，≥4 按幅度升档。
        let mag = code & 7;
        let delta = if mag < 4 { -1 } else { 2 * i32::from(mag - 3) };
        self.step_index = (self.step_index as i32 + delta).clamp(0, 88) as usize;
        self.predictor
    }
}

/// BLE 通知可能把一帧拆成多包（MTU 没谈大时 20 字节一包），按档案帧长切帧。
#[derive(Default)]
pub struct FrameAccumulator {
    size: usize,
    buf: Vec<u8>,
}

impl FrameAccumulator {
    pub fn new(size: usize) -> Self {
        Self {
            size: size.max(1),
            buf: Vec::new(),
        }
    }

    /// 协商出真实帧长后改档（GET_CAPS 声明优先于档案兜底值）。
    pub fn set_size(&mut self, size: usize) {
        self.size = size.max(1);
    }

    /// 喂一包通知，吐出凑齐的整帧；不足一帧的留在缓冲里。
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while self.buf.len() >= self.size {
            out.push(self.buf.drain(..self.size).collect());
        }
        out
    }

    /// 会话收尾：取走不足一帧的尾巴（排空语义，尾音不得静默丢）。
    pub fn take_partial(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buf)
    }
}

/// 三点平滑 `(a + 2b + c) >> 2`（首尾沿用原值，不引入边界毛刺）。
pub fn smooth(pcm: &[i16]) -> Vec<i16> {
    if pcm.len() < 3 {
        return pcm.to_vec();
    }
    let mut out = Vec::with_capacity(pcm.len());
    out.push(pcm[0]);
    for w in pcm.windows(3) {
        out.push(((i32::from(w[0]) + 2 * i32::from(w[1]) + i32::from(w[2])) >> 2) as i16);
    }
    out.push(pcm[pcm.len() - 1]);
    out
}

/// 用户音量 `0…1000%`（1000% ≈ +20 dB），在输出出口统一缩放并削顶。
/// 不作用在解码层——那是完整性度量的基准，动了指标就假了。
pub fn apply_gain(pcm: &[i16], percent: u16) -> Vec<i16> {
    if percent == 100 {
        return pcm.to_vec();
    }
    pcm.iter()
        .map(|s| {
            (i32::from(*s) * i32::from(percent) / 100).clamp(i16::MIN as i32, i16::MAX as i32)
                as i16
        })
        .collect()
}

/// 完整性度量：实收样本 ÷ 应收样本（按协商采样率算，不写死 16k）。
#[derive(Clone, Debug)]
pub struct Integrity {
    rate: u32,
    pub frames: u64,
    pub samples: u64,
    pub ms: u64,
}

impl Integrity {
    pub fn new(rate: u32) -> Self {
        Self {
            rate: rate.max(1),
            frames: 0,
            samples: 0,
            ms: 0,
        }
    }

    pub fn push(&mut self, frame_samples: usize) {
        self.frames += 1;
        self.samples += frame_samples as u64;
    }

    /// 会话时长（松手时刻定稿），coverage 的分母。
    pub fn set_duration(&mut self, ms: u64) {
        self.ms = ms;
    }

    /// 覆盖率百分比；时长未知返回 None（不编数字）。
    pub fn coverage(&self) -> Option<f64> {
        if self.ms == 0 {
            return None;
        }
        let expected = self.ms * u64::from(self.rate) / 1000;
        if expected == 0 {
            return None;
        }
        Some(self.samples as f64 / expected as f64 * 100.0)
    }
}

/// 输入电平（0..1）：RMS 再开一次平方根，静音 0、满幅 1。
/// 直接给线性 RMS 的话语音在表上几乎不动（语音 RMS 天然偏低），
/// 平方根既单调又让小声看得出来在动。
pub fn level(pcm: &[i16]) -> f64 {
    if pcm.is_empty() {
        return 0.0;
    }
    let mean_sq = pcm
        .iter()
        .map(|s| {
            let s = *s as f64 / 32768.0;
            s * s
        })
        .sum::<f64>()
        / pcm.len() as f64;
    mean_sq.sqrt().sqrt()
}

/// 线性重采样：路 2 灌进声卡时把协商采样率变到设备默认采样率。
pub fn resample(pcm: &[i16], from: u32, to: u32) -> Vec<i16> {
    if from == 0 || to == 0 || from == to || pcm.is_empty() {
        return pcm.to_vec();
    }
    let ratio = f64::from(from) / f64::from(to);
    let out_len = ((pcm.len() as f64) / ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let i0 = pos.floor() as usize;
        let frac = pos - i0 as f64;
        let a = f64::from(pcm[i0.min(pcm.len() - 1)]);
        let b = f64::from(pcm[(i0 + 1).min(pcm.len() - 1)]);
        out.push((a + (b - a) * frac).round().clamp(-32768.0, 32767.0) as i16);
    }
    out
}

/// PCM → WAV（路 3 发给 OpenAI 兼容 `/v1/audio/transcriptions` 的 multipart 体）。
pub fn to_wav(pcm: &[i16], rate: u32) -> Vec<u8> {
    let data_len = pcm.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    let put = |out: &mut Vec<u8>, bytes: &[u8]| out.extend_from_slice(bytes);
    put(&mut out, b"RIFF");
    put(&mut out, &(36 + data_len as u32).to_le_bytes());
    put(&mut out, b"WAVEfmt ");
    put(&mut out, &16u32.to_le_bytes());
    put(&mut out, &1u16.to_le_bytes()); // PCM
    put(&mut out, &1u16.to_le_bytes()); // 单声道
    put(&mut out, &rate.to_le_bytes());
    put(&mut out, &(rate * 2).to_le_bytes());
    put(&mut out, &2u16.to_le_bytes());
    put(&mut out, &16u16.to_le_bytes());
    put(&mut out, b"data");
    put(&mut out, &(data_len as u32).to_le_bytes());
    for s in pcm {
        put(&mut out, &s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn level_maps_silence_to_zero_and_full_scale_to_one() {
        assert_eq!(level(&[]), 0.0);
        assert_eq!(level(&[0, 0, 0, 0]), 0.0);
        let full = [-32768i16, 32767, -32768, 32767];
        assert!((level(&full) - 1.0).abs() < 0.01, "{}", level(&full));
        let half = [-16384i16, 16383, -16384, 16383];
        let l = level(&half);
        assert!(l > 0.5 && l < 1.0, "{l}");
    }

    use super::*;

    #[test]
    fn caps_parses_big_endian_and_rejects_garbage() {
        // 0B | 0100(v1.0) | 0003(codecs) | 0078(120 字节帧)
        let c = Capabilities::parse(&[0x0B, 0x01, 0x00, 0x00, 0x03, 0x00, 0x78]).unwrap();
        assert_eq!(c.version, 0x0100);
        assert_eq!(c.codecs, 3);
        assert_eq!(c.frame_bytes, 120);
        assert_eq!(c.sample_rate(), 16_000);
        assert!(c.supports_extend());
        // codec 只给 0x0001 → 8k（收不收由档案决定，不在这里硬门禁）。
        let v04 = Capabilities::parse(&[0x0B, 0x00, 0x04, 0x00, 0x01, 0x00, 0x86]).unwrap();
        assert_eq!(v04.sample_rate(), 8_000);
        assert!(!v04.supports_extend());
        // 魔数不对 / 长度不足一律 None。
        assert!(Capabilities::parse(&[0x0C, 1, 2, 3, 4, 5, 6]).is_none());
        assert!(Capabilities::parse(&[0x0B, 1, 2]).is_none());
        assert!(Capabilities::parse(&[]).is_none());
    }

    #[test]
    fn session_commands_match_spec() {
        assert_eq!(mic_open(None), vec![0x0C, 0x00]);
        assert_eq!(mic_open(Some(0x0002)), vec![0x0C, 0x00, 0x00, 0x02]);
        assert_eq!(mic_close(0x07), vec![0x0D, 0x07]);
        assert_eq!(mic_extend(0x07), vec![0x0E, 0x07]);
    }

    #[test]
    fn control_messages_decode() {
        assert_eq!(parse_control(&[0x08]), Some(Control::RequestOpen));
        assert_eq!(
            parse_control(&[0x04, 0x01, 0x02, 0x09]),
            Some(Control::StreamStart { session: 0x09 })
        );
        // 短包也认流开始，会话号缺省 0（G9N9N 只发单字节 0x04 表「按住」）。
        assert_eq!(
            parse_control(&[0x04]),
            Some(Control::StreamStart { session: 0 })
        );
        assert_eq!(parse_control(&[0x00]), Some(Control::StreamStop));
        // 重同步：bytes[4..5] = predictor，[6] = stepIndex。
        assert_eq!(
            parse_control(&[0x0A, 0, 0, 0, 0x34, 0x12, 0x05]),
            Some(Control::Resync {
                predictor: 0x1234,
                step_index: 5
            })
        );
        // 认不出的不得当成事件。
        assert_eq!(parse_control(&[0x0A, 0, 0]), None);
        assert_eq!(parse_control(&[0x77]), None);
        assert_eq!(parse_control(&[]), None);
    }

    #[test]
    fn adpcm_silence_stays_silent() {
        // 全 0 码 + 零起点：递推必须停在 0，否则静音段会自己长出噪声。
        let mut d = AdpcmDecoder::new();
        let pcm = d.decode(&[0u8; 16], FrameHeader::None, false);
        assert_eq!(pcm.len(), 32);
        assert!(pcm.iter().all(|s| *s == 0));
    }

    #[test]
    fn adpcm_nibble_order_flips_sample_order() {
        // 0xE0：高 nibble E 解出负样本，低 nibble 0 解出 0。翻转必须换第一个样本。
        let mut hi = AdpcmDecoder::new();
        assert_eq!(hi.decode(&[0xE0], FrameHeader::None, false)[0], -10);
        let mut lo = AdpcmDecoder::new();
        assert_eq!(lo.decode(&[0xE0], FrameHeader::None, true)[0], 0);
    }

    #[test]
    fn adpcm_tracks_state_and_resync() {
        let mut d = AdpcmDecoder::new();
        // 连续正码必须把预测器推上去（发散方向正确）。
        let pcm = d.decode(&[0x77; 8], FrameHeader::None, false);
        assert!(pcm[pcm.len() - 1] > pcm[0]);
        let (p, i) = d.state();
        assert!(p > 0 && i > 0);
        d.resync(-500, 3);
        assert_eq!(d.state(), (-500, 3));
        // 步长索引越界钳到表内，不许 panic。
        d.resync(0, 999);
        assert_eq!(d.state(), (0, 88));
    }

    #[test]
    fn dvi_frame_header_resets_state_each_frame() {
        // 逐帧自带状态：两帧喂同一解码器，第二帧结果不得受第一帧影响。
        let mut d = AdpcmDecoder::new();
        let mut frame = vec![0u8; 4 + 8];
        frame[..4].copy_from_slice(&[0x10, 0x27, 0x04, 0x00]); // predictor 10000、index 4
        let a = d.decode(&frame, FrameHeader::DviState, false);
        let b = d.decode(&frame, FrameHeader::DviState, false);
        assert_eq!(a, b, "帧头状态没重置，第二帧会发散");
        // 帧比头还短 = 坏帧，返回空而不是拿半截当样本。
        assert!(d.decode(&[0, 1], FrameHeader::DviState, false).is_empty());
    }

    #[test]
    fn frame_accumulator_splits_across_packets() {
        let mut acc = FrameAccumulator::new(4);
        assert!(acc.push(&[1, 2]).is_empty());
        let frames = acc.push(&[3, 4, 5, 6, 7]);
        assert_eq!(frames, vec![vec![1, 2, 3, 4]]);
        assert_eq!(acc.take_partial(), vec![5, 6, 7]);
        assert!(acc.take_partial().is_empty());
        // 协商后改帧长立刻生效。
        acc.set_size(2);
        assert_eq!(acc.push(&[9, 8, 7]), vec![vec![9, 8]]);
    }

    #[test]
    fn pcm_post_processing() {
        assert_eq!(smooth(&[5, 5, 5, 5]), vec![5, 5, 5, 5]);
        // 单点脉冲被摊平：(0+2*100+0)>>2 = 50。
        assert_eq!(smooth(&[0, 100, 0]), vec![0, 50, 0]);
        assert_eq!(smooth(&[1, 2]), vec![1, 2]);
        assert_eq!(apply_gain(&[100, -100], 100), vec![100, -100]);
        assert_eq!(apply_gain(&[100, -100], 200), vec![200, -200]);
        // 1000% 削顶不绕回。
        assert_eq!(apply_gain(&[i16::MAX], 1000), vec![i16::MAX]);
    }

    #[test]
    fn integrity_coverage_is_honest() {
        let mut m = Integrity::new(16_000);
        // 时长未知不编指标。
        assert!(m.coverage().is_none());
        // 1000 ms @16k 应收 16000 样本；实收 120 样本/帧 × 100 帧 = 12000 → 75%。
        for _ in 0..100 {
            m.push(120);
        }
        m.set_duration(1000);
        assert_eq!(m.frames, 100);
        assert!((m.coverage().unwrap() - 75.0).abs() < 0.001);
    }

    #[test]
    fn resample_changes_length_and_keeps_shape() {
        let pcm: Vec<i16> = (0..1600).map(|i| (i % 64) as i16).collect();
        let up = resample(&pcm, 16_000, 48_000);
        assert_eq!(up.len(), 4800);
        // 同率直通。
        assert_eq!(resample(&pcm, 16_000, 16_000), pcm);
        assert!(resample(&[], 16_000, 48_000).is_empty());
        // 下采样不越界、不绕回。
        let down = resample(&[i16::MAX; 100], 48_000, 16_000);
        assert!(down.iter().all(|s| *s > 0));
    }

    #[test]
    fn wav_header_is_canonical() {
        let wav = to_wav(&[1, -1], 16_000);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]), 36 + 4);
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 4);
        assert_eq!(wav.len(), 44 + 4);
        assert_eq!(i16::from_le_bytes([wav[44], wav[45]]), 1);
    }
}
