//! mSBC-over-HID 语音通道（计划 §八 P7）：小米「普通 1 代」把 mSBC 塞在 HID 输入报告里传。
//!
//! 上游实机事实（`ToStardust/xiaomi-remote-voice`，Windows USBPcap 抓 HCI/ATT）：
//! - 音频报告 60 字节 = `[状态 0x01][H2 序号 08/38/C8/F8][57 字节 mSBC 帧，同步 0xAD][填充]`，
//!   一帧 = 120 样本 @16k = 7.5 ms。
//! - 起止由 20 字节控制报告给：首字节 `01` = 开始、`00` = 结束，其余为 0。
//! - 厂商服务 D0FF / D1FF / 6287 直订拿不到音频 → 这条通道没有 GATT 可订，只能读 HID 报告。
//!
//! 纯逻辑（无 cfg、无 FFI），默认 `cargo test` 就编到并全测。mSBC 解码用 `nanoaudio-sbc`
//! （MIT，no_std 无分配，自带 CRC 校验与合成滤波器组），不手搓 DSP。
//!
//! 帧起始偏移做了 0..3 字节容错：上游抓的是 ATT 通知原始字节，hidapi 在 macOS 上
//! 是否额外补 report id 尚未真机确认（P7a 台账就是去拿这个读数），差一字节就整段解不出来。

/// mSBC 单帧字节数（blocks 15 × subbands 8 / 8 × bitpool 26 的定长帧）。
pub const FRAME_BYTES: usize = 57;
/// mSBC 恒为 16 kHz 单声道（HFP 写死，不是设备差）。
pub const SAMPLE_RATE: u32 = 16_000;
/// mSBC 帧同步字节。
const SYNC: u8 = 0xAD;
/// 控制报告长度（上游实测 20 字节，首字节 01/00，其余补 0）。
const CONTROL_LEN: usize = 20;
/// 帧头（状态 + H2 序号）最多占几字节：同步字节搜索窗的上界。
const HEADER_MAX: usize = 3;
/// 单次解码输出缓冲（样本数）：一帧 120 样本，留一倍余量。
const OUT_SAMPLES: usize = 256;

/// 找到音频报告里 mSBC 帧的起始偏移；找不到 = 这不是音频报告。
pub fn frame_offset(payload: &[u8]) -> Option<usize> {
    let tail = payload.len().checked_sub(FRAME_BYTES)?;
    (0..=HEADER_MAX.min(tail)).find(|&i| payload[i] == SYNC)
}

/// 这条输入报告是不是音频报告。
pub fn is_audio(payload: &[u8]) -> bool {
    frame_offset(payload).is_some()
}

/// 控制报告 → 会话起止：`Some(true)` 开始、`Some(false)` 结束，其余形状一律 None。
///
/// 卡死形状是必须的：键盘报告的空槽也是全 0，不卡长度和补零就会把「松开了别的键」
/// 当成「语音说完」。
pub fn control(payload: &[u8]) -> Option<bool> {
    if payload.len() != CONTROL_LEN || payload[1..].iter().any(|b| *b != 0) {
        return None;
    }
    match payload[0] {
        0x01 => Some(true),
        0x00 => Some(false),
        _ => None,
    }
}

/// mSBC 流解码器：按序喂音频报告，吐出 PCM。
pub struct MsbcHid {
    decoder: nanoaudio_sbc::SbcDecoder,
    out: Vec<i16>,
    /// 成功解出的帧数。
    pub frames: u64,
    /// 丢掉的帧数（同步字节没找到 / CRC 不过）。取不到不编，但必须看得见。
    pub bad: u64,
}

impl Default for MsbcHid {
    fn default() -> Self {
        Self::new()
    }
}

impl MsbcHid {
    pub fn new() -> Self {
        Self {
            decoder: nanoaudio_sbc::SbcDecoder::new(),
            out: vec![0; OUT_SAMPLES],
            frames: 0,
            bad: 0,
        }
    }

    /// 喂一条音频报告，返回解出的 PCM（解不动返回空，绝不中断整场会话）。
    pub fn push(&mut self, payload: &[u8]) -> Vec<i16> {
        let Some(off) = frame_offset(payload) else {
            self.bad += 1;
            return Vec::new();
        };
        let frame = &payload[off..off + FRAME_BYTES];
        // decode_into 是 inherent 方法（不必引 nanoaudio-core 的 trait）；
        // 它自带 CRC 校验，坏帧报错而不是吐出噪声，这里复位滤波器接着收下一帧。
        match self.decoder.decode_into(frame, &mut self.out) {
            Ok(p) if p.produced > 0 => {
                self.frames += 1;
                self.out[..p.produced].to_vec()
            }
            Ok(_) => {
                self.bad += 1;
                Vec::new()
            }
            Err(_) => {
                self.bad += 1;
                self.decoder.reset();
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nanoaudio_core::codec::Encoder;
    use nanoaudio_sbc::{SbcEncoder, SbcParameters};

    /// 用编码器造一条真 mSBC 帧，包成上游那个 60 字节 HID 报告。
    fn report(seq: u8) -> Vec<u8> {
        let mut enc = SbcEncoder::new(SbcParameters::MSBC).unwrap();
        // 400 Hz 正弦，一帧 120 样本 @16k。
        let pcm: Vec<i16> = (0..120)
            .map(|i| {
                (8000.0 * (2.0 * std::f32::consts::PI * 400.0 * i as f32 / 16000.0).sin()) as i16
            })
            .collect();
        let mut frame = [0u8; FRAME_BYTES];
        let p = enc.encode(&pcm, &mut frame).unwrap();
        assert_eq!(p.produced, FRAME_BYTES);
        let mut r = vec![0x01, seq];
        r.extend_from_slice(&frame);
        r.resize(60, 0);
        r
    }

    #[test]
    fn audio_report_is_found_by_its_sync_byte() {
        let r = report(0x08);
        assert_eq!(frame_offset(&r), Some(2));
        assert!(is_audio(&r));
        // hidapi 若额外补了 report id，帧整体后移一字节也必须认得。
        let mut shifted = vec![0u8];
        shifted.extend_from_slice(&r);
        assert_eq!(frame_offset(&shifted), Some(3));
        // 太短 / 没有同步字节 = 不是音频报告。
        assert!(!is_audio(&[0x01, 0x08]));
        assert!(!is_audio(&[0u8; 60]));
    }

    #[test]
    fn control_accepts_only_the_vendor_shape() {
        let mut start = [0u8; CONTROL_LEN];
        start[0] = 0x01;
        assert_eq!(control(&start), Some(true));
        assert_eq!(control(&[0u8; CONTROL_LEN]), Some(false));
        // 键盘空报告也是全 0，靠长度把它挡在外面。
        assert_eq!(control(&[0u8; 8]), None);
        // 尾部非零 = 不是控制报告。
        let mut dirty = [0u8; CONTROL_LEN];
        dirty[3] = 0x28;
        assert_eq!(control(&dirty), None);
        // 首字节不是 01/00 不猜语义。
        let mut other = [0u8; CONTROL_LEN];
        other[0] = 0x05;
        assert_eq!(control(&other), None);
    }

    #[test]
    fn frames_decode_to_pcm() {
        let mut d = MsbcHid::new();
        let mut total = 0usize;
        let mut peak = 0i16;
        for seq in [0x08u8, 0x38, 0xC8, 0xF8, 0x08, 0x38, 0xC8, 0xF8] {
            let pcm = d.push(&report(seq));
            total += pcm.len();
            peak = peak.max(pcm.iter().map(|s| s.abs()).max().unwrap_or(0));
        }
        assert_eq!(d.frames, 8);
        assert_eq!(d.bad, 0);
        assert_eq!(total, 8 * 120);
        // 400 Hz / 幅值 8000 的正弦回来必须还是听得见的电平（SBC 有损，不卡死具体值）。
        assert!((1_000..=16_000).contains(&peak), "解出来的电平不对: {peak}");
    }

    #[test]
    fn a_bad_frame_is_counted_and_the_stream_recovers() {
        let mut d = MsbcHid::new();
        assert!(d.push(&[0x01, 0x08, 0x00, 0xAD]).is_empty());
        assert_eq!(d.bad, 1);
        assert_eq!(d.frames, 0);
        // 坏帧不得把后面的好帧一起带走。
        assert!(!d.push(&report(0x08)).is_empty());
        assert_eq!(d.frames, 1);
    }
}
