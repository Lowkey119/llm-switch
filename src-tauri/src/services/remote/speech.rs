//! 语音路 1：系统转写（Apple Speech 框架，计划 2026-11-19 §六 路 1）。
//!
//! 两种音频源，同一个识别会话骨架（`start_session`）：
//! - [`run`]：Mac 本机麦克风（AVAudioEngine inputNode tap）。给没有设备语音通道的
//!   遥控器当「按住说话」触发器用（P7d：mSBC-over-HID 在 macOS 到不了应用层）。
//! - [`run_piped`]：外部 PCM 管道（16 kHz i16 mono，ATVV 解码后的设备麦音频）。
//!   ATVV 机型（小米 RC001/RC003 等）用户对着遥控器说话，Mac 麦根本听不到，
//!   必须把设备音频直灌 `SFSpeechAudioBufferRecognitionRequest`（用户 2026-11-21 拍板）。
//!
//! 线程模型：objc2 对象 `!Send`，整场会话在调用方给的专用线程里创建、运行、收尾；
//! 识别结果块默认在主队列回调（Tauri 主循环服务它），只回传 `Send + Sync` 的文本。
//!
//! 已知边界：首次麦克风授权前的第一场会话可能收到全静音（系统不报错），重按一次即好。

#![allow(unsafe_code)]

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_avf_audio::{
    AVAudioCommonFormat, AVAudioEngine, AVAudioFormat, AVAudioFrameCount, AVAudioPCMBuffer,
    AVAudioTime,
};
use objc2_foundation::{NSError, NSLocale};
use objc2_speech::{
    SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionResult, SFSpeechRecognitionTask,
    SFSpeechRecognizer, SFSpeechRecognizerAuthorizationStatus,
};

/// 授权弹窗等待上限：没人点就一直挂着会话不体面，60 s 到点报错走人。
const AUTH_WAIT: Duration = Duration::from_secs(60);
/// `endAudio` 后等定稿结果的上限：定稿一般几百 ms 内到，2 s 是兜底。
const FINAL_WAIT: Duration = Duration::from_secs(2);
/// 输入 tap 的缓冲帧数（≈85 ms @ 48 kHz）：再小回调太碎，再大首字延迟变长。
const TAP_FRAMES: u32 = 4096;
/// 会话轮询节拍：松手（stop）与定稿（done）都靠它发现。
const POLL: Duration = Duration::from_millis(50);
/// 管道路的设备音频采样率（ATVV 协商值，caps 缺省口径一致）。
const PIPED_RATE: f64 = 16_000.0;
/// 管道兜底静默窗：发送端已停且超过它没新样本才收尾（比尾包窗 300 ms 宽）。
const PIPE_GUARD: Duration = Duration::from_millis(400);

/// 一场识别会话的公共骨架：请求 + 任务 + 收尾信号 + 识别器错误回传。
struct Session {
    request: Retained<SFSpeechAudioBufferRecognitionRequest>,
    task: Retained<SFSpeechRecognitionTask>,
    done: Arc<AtomicBool>,
    /// 识别器报错（含「没听到说话」）：以前静默吞掉，会话秒断用户只能看到
    /// 「聆听中闪一下就没了」，现在带出文案（用户 2026-11-21 抓 1 s 闪断）。
    err: Arc<Mutex<Option<String>>>,
}

impl Session {
    fn take_err(&self) -> Option<String> {
        self.err.lock().expect("speech err poisoned").take()
    }
}

/// 建识别器 + 流式请求 + 识别任务（两种音频源共用，不各写一份）。
fn start_session(
    on_text: impl Fn(String, bool) + Send + Sync + 'static,
) -> Result<Session, String> {
    let recognizer = unsafe {
        SFSpeechRecognizer::initWithLocale(SFSpeechRecognizer::alloc(), &NSLocale::currentLocale())
    }
    .ok_or("当前系统语言不支持语音识别")?;
    if unsafe { !recognizer.isAvailable() } {
        return Err("系统语音识别当前不可用（检查网络与「听写」设置）".into());
    }

    let request = unsafe { SFSpeechAudioBufferRecognitionRequest::new() };
    unsafe {
        request.setShouldReportPartialResults(true);
        if recognizer.supportsOnDeviceRecognition() {
            // 支持端侧识别就强制端侧：不依赖网络，音频也不出机器。
            request.setRequiresOnDeviceRecognition(true);
        }
    }

    let done = Arc::new(AtomicBool::new(false));
    let err = Arc::new(Mutex::new(None));
    let done2 = done.clone();
    let err2 = err.clone();
    let result_block = RcBlock::new(
        move |result: *mut SFSpeechRecognitionResult, error: *mut NSError| unsafe {
            if let Some(r) = result.as_ref() {
                let text = r.bestTranscription().formattedString().to_string();
                let fin = r.isFinal();
                if !text.trim().is_empty() {
                    on_text(text, fin);
                }
                if fin {
                    done2.store(true, Ordering::SeqCst);
                }
            }
            // 出错（含「没听到说话」）同样收尾，不干等松手；文案带出去给页面看。
            if !error.is_null() {
                if let Some(e) = error.as_ref() {
                    *err2.lock().expect("speech err poisoned") =
                        Some(e.localizedDescription().to_string());
                }
                done2.store(true, Ordering::SeqCst);
            }
        },
    );
    let task =
        unsafe { recognizer.recognitionTaskWithRequest_resultHandler(&request, &result_block) };
    Ok(Session {
        request,
        task,
        done,
        err,
    })
}

/// 收尾公共段：定稿等待 + 撤任务 + 识别器错误带出。
fn finish_session(s: &Session) -> Result<(), String> {
    let deadline = Instant::now() + FINAL_WAIT;
    while !s.done.load(Ordering::SeqCst) && Instant::now() < deadline {
        thread::sleep(POLL);
    }
    unsafe { s.task.cancel() };
    match s.take_err() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// 路 1（Mac 麦克风）：跑一场系统转写会话（阻塞到松手或识别收尾）。
///
/// `on_text(text, is_final)`：partial 实时回显、final 定稿注入，从主队列回调，
/// 实现方自己保证线程安全（voice.rs 走 emit / run_on_main_thread）。
pub fn run(
    stop: Arc<AtomicBool>,
    on_text: impl Fn(String, bool) + Send + Sync + 'static,
) -> Result<(), String> {
    ensure_authorized()?;
    let s = start_session(on_text)?;

    // 采集：inputNode tap 原样喂 buffer。格式取设备原生输出，Speech 自己会重采样。
    let engine = unsafe { AVAudioEngine::new() };
    let input = unsafe { engine.inputNode() };
    let format = unsafe { input.outputFormatForBus(0) };
    let req = s.request.clone();
    let tap = RcBlock::new(
        move |buf: NonNull<AVAudioPCMBuffer>, _when: NonNull<AVAudioTime>| {
            // tap 回调在音频实时线程：只转发 buffer，不干别的。
            unsafe { req.appendAudioPCMBuffer(buf.as_ref()) };
        },
    );
    unsafe {
        input.installTapOnBus_bufferSize_format_block(
            0,
            TAP_FRAMES,
            Some(&format),
            RcBlock::as_ptr(&tap),
        );
    }
    if let Err(e) = unsafe { engine.startAndReturnError() } {
        unsafe { input.removeTapOnBus(0) };
        return Err(format!("音频采集启动失败：{e}"));
    }

    // 撑住会话：松手（stop）或识别定稿/出错（done）为止。
    while !stop.load(Ordering::SeqCst) && !s.done.load(Ordering::SeqCst) {
        thread::sleep(POLL);
    }
    // 收尾：停采集 → 告诉识别器没有更多音频 → 等定稿（≤2 s）→ 撤任务。
    unsafe {
        engine.stop();
        input.removeTapOnBus(0);
        s.request.endAudio();
    }
    finish_session(&s)
}

/// 路 1（设备音频管道）：外部 16 kHz i16 mono PCM 直灌识别器。
///
/// `rx` 收一段段解码后的 PCM（ATVV 会话松手后还会推 ~300 ms 尾包，发送端排空才
/// drop，所以这里以「通道断开」为音频结束信号，不能一见 stop 就收——尾音会丢）。
/// `stop` 只作兜底闸门：发送端已停且无积压时提前收尾。
pub fn run_piped(
    stop: Arc<AtomicBool>,
    rx: mpsc::Receiver<Vec<i16>>,
    on_text: impl Fn(String, bool) + Send + Sync + 'static,
) -> Result<(), String> {
    ensure_authorized()?;
    let s = start_session(on_text)?;
    let format = unsafe {
        AVAudioFormat::initWithCommonFormat_sampleRate_channels_interleaved(
            AVAudioFormat::alloc(),
            AVAudioCommonFormat::PCMFormatInt16,
            PIPED_RATE,
            1,
            true,
        )
    }
    .ok_or("无法建立 16 kHz 单声道 PCM 格式")?;
    let mut last_chunk = Instant::now();
    loop {
        match rx.recv_timeout(POLL) {
            Ok(chunk) => {
                last_chunk = Instant::now();
                append_pcm(&s.request, &format, &chunk);
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                // 兜底闸门：发送端已停且 400 ms 没有新样本（尾包窗 300 ms 早过）
                // 才收尾；正常路径靠通道断开，尾音一个不丢。
                if stop.load(Ordering::SeqCst) && last_chunk.elapsed() > PIPE_GUARD {
                    break;
                }
            }
        }
        if s.done.load(Ordering::SeqCst) {
            break;
        }
    }
    unsafe { s.request.endAudio() };
    finish_session(&s)
}

/// 把 i16 PCM 拷进 `AVAudioPCMBuffer` 喂给识别请求（Speech 内部会按需重采样）。
fn append_pcm(
    request: &SFSpeechAudioBufferRecognitionRequest,
    format: &AVAudioFormat,
    pcm: &[i16],
) {
    if pcm.is_empty() {
        return;
    }
    // 16-bit PCM 一帧一样本，frameCapacity 直接给样本数。
    let Some(buf) = (unsafe {
        AVAudioPCMBuffer::initWithPCMFormat_frameCapacity(
            AVAudioPCMBuffer::alloc(),
            format,
            pcm.len() as AVAudioFrameCount,
        )
    }) else {
        return;
    };
    let chans = unsafe { buf.int16ChannelData() };
    if chans.is_null() {
        return;
    }
    unsafe {
        // 单声道交织：通道指针数组第一个元素即整段样本。
        let dst = chans.read().as_ptr();
        std::ptr::copy_nonoverlapping(pcm.as_ptr(), dst, pcm.len());
        buf.setFrameLength(pcm.len() as AVAudioFrameCount);
        request.appendAudioPCMBuffer(&buf);
    }
}

/// 语音识别授权：未决定时弹系统窗等待；拒绝/受限给可执行的出路文案。
fn ensure_authorized() -> Result<(), String> {
    let status = unsafe { SFSpeechRecognizer::authorizationStatus() };
    if status == SFSpeechRecognizerAuthorizationStatus::Authorized {
        return Ok(());
    }
    if status != SFSpeechRecognizerAuthorizationStatus::NotDetermined {
        return Err("语音识别未授权（系统设置 → 隐私与安全性 → 语音识别）".into());
    }
    let (tx, rx) = mpsc::channel();
    unsafe {
        SFSpeechRecognizer::requestAuthorization(&RcBlock::new(move |s| {
            let _ = tx.send(s);
        }));
    }
    // 回调在主队列；本线程在专用线程上睡等，Tauri 主循环照常服务弹窗。
    let status = rx
        .recv_timeout(AUTH_WAIT)
        .map_err(|_| "语音识别授权等待超时（60 秒）".to_string())?;
    if status == SFSpeechRecognizerAuthorizationStatus::Authorized {
        Ok(())
    } else {
        Err("语音识别未授权（系统设置 → 隐私与安全性 → 语音识别）".into())
    }
}
