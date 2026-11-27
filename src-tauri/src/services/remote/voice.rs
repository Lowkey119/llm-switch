//! 语音录入通道（计划 2026-11-19 §六）：mic 键 → ATVV 采集 → ADPCM 解码 → 出口。
//!
//! 出口四路（用户 2026-11-20 拍板：**用户具有最高选择权**，产品只指引不代选）：
//! - `System`（路 1，缺省）：ATVV 机型把**设备麦音频**直灌 Speech 识别器（用户对着
//!   遥控器说话，Mac 麦听不到，用户 2026-11-21 拍板）；没有 ATVV 通道的设备退回
//!   **Mac 本机麦克风**（语音键只当「按住说话」触发器，设备麦在 macOS 拿不到，
//!   队列 47 真机读数＝负）。Speech 框架流式转写，partial 实时回显、定稿注入焦点框。
//!   实现见 `speech` 模块。
//! - `Soundcard`（路 2）：解码后的 PCM 灌进用户指定的输出设备（BlackHole 一类虚拟声卡），
//!   用户自己的语音输入法当麦克风收流转写。本 app 不转写、不上传。
//! - `Gateway`（路 3）：整段 PCM 封 WAV 发给本机网关 `/v1/audio/transcriptions`
//!   （asr 工种类后端，复用现有媒体透传，不另造 STT 通道），回文按「语句定稿」注入焦点框。
//! - `Off`：语音键不做事。
//!
//! 时序口径（计划 §三 已知坑 5/6，都是别人交过学费的常量）：松手后留 300 ms 迟到音频窗，
//! 排空上限 750 ms；正常松手不 flush 在途缓存，必须等尾包投递完再发配对的 MIC_CLOSE。
//!
//! 会话生命周期由 HID 语音键（HoldToTalk）驱动。档案声明 `AtvvStatus` 信号源的设备
//! （G9N9N）没有 HID 语音键，其会话起停要改由 ab5e0004 的 0x04/0x00 驱动——
//! 该档案本身缺真机（队列 23/24 blocked），拿到设备前不猜实现。

use crate::gateway::{self, State as GatewayState};
use crate::services::remote::atvv::{
    self, AdpcmDecoder, Capabilities, Control, FrameAccumulator, Integrity, Keepalive,
};
use crate::services::remote::identity::RemoteSession;
use crate::services::remote::msbc;
use crate::services::remote::profile::{DeviceProfile, VoiceSpec};
use crate::store::Store;
use btleplug::api::{Characteristic, Peripheral as _, WriteType};
use btleplug::platform::Peripheral;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

/// 偏好持久化键（settings 表，整份 JSON）。
pub const SETTINGS_KEY: &str = "remote.voice.config";

/// 语音键（0x3E）是否应改写为硬件 🌐：路 2 且配了输入法。
/// 微信输入法语音是按住说话式、只认硬件 Fn（合成 CGEvent 不认，2026-11-21 实测），
/// 所以触发交给 powermap 的 HID 层改写，会话里不再注入任何按键。
fn voice_key_to_fn(s: &VoiceSettings) -> bool {
    s.route == VoiceRoute::Soundcard && s.ime_source.as_deref().is_some_and(|x| !x.is_empty())
}
/// 会话保活节拍：设备侧 Audio Transfer Timeout 实测区间 15–60 s，取 5 s 留足余量。
const KEEPALIVE: Duration = Duration::from_secs(5);
/// 松手后的迟到音频窗（迟到音频不得丢，尾字完整性靠它）。
const LATE_AUDIO: Duration = Duration::from_millis(300);
/// 输出排空上限：超过就判定设备不再收，收尾退出。
const DRAIN: Duration = Duration::from_millis(750);
/// 电平上报节流：一帧一报（30 ms），前端波形图每帧一个采样点——再稀曲线就
/// 成折线了（用户 2026-11-21 要求调密）；IPC 节拍 33 条/秒，淹不掉。
const LEVEL_EMIT: Duration = Duration::from_millis(30);
/// GET_CAPS 响应等待上限，超时退回档案兜底参数。
const CAPS_WAIT: Duration = Duration::from_millis(1500);
/// 声卡环形缓冲上限（帧数）：约 2 s，卡死时丢最旧的，绝不无界增长。
const RING_FRAMES: usize = 2000;
/// mSBC/HID 帧队列上限（约 3 s）：读线程投得比会话收得快时丢最旧的。
const MSBC_QUEUE: usize = 400;
/// mSBC 开场等首帧上限：控制报告到了音频却没来，直接报错，不干等。
const MSBC_FIRST: Duration = Duration::from_secs(3);
/// mSBC 帧流停摆兜底：设备没发「结束」控制报告时靠它收尾，否则会话闸门永久卡住。
const MSBC_IDLE: Duration = Duration::from_millis(1500);

/// 语音出口。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceRoute {
    /// 路 1：系统语音转写。音频源是 Mac 本机麦克风，设备语音通道不参与。
    System,
    /// 路 2：虚拟声卡透传（用户自有输入法）。缺省（用户 2026-11-21 拍板）。
    #[default]
    Soundcard,
    /// 路 3：网关 ASR 转写 + 文本注入。未调通前只作手动备选，不自动选。
    Gateway,
    Off,
}

/// 语音偏好（页面「语音模式切换」常驻设置的数据）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoiceSettings {
    #[serde(default)]
    pub route: VoiceRoute,
    /// 路 2：输出设备名；None = 未选（页面据此高亮安装引导）。
    #[serde(default)]
    pub device: Option<String>,
    /// 输出音量 0…1000%（1000% ≈ +20 dB）。
    #[serde(default = "default_gain")]
    pub gain: u16,
    /// 路 3：ASR 模型名（网关 asr 工种类暴露的模型）。
    #[serde(default)]
    pub stt_model: Option<String>,
    /// 路 2：接听的输入法 id（Carbon TIS 输入源）；会话开始切到它、收尾还原。
    #[serde(default)]
    pub ime_source: Option<String>,
}

fn default_gain() -> u16 {
    100
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            route: VoiceRoute::default(),
            device: None,
            gain: 100,
            stt_model: None,
            ime_source: None,
        }
    }
}

/// 会话状态（页面三态：聆听中 / 转写中 / 已注入）。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceState {
    Idle,
    Listening,
    Transcribing,
    Injected,
}

/// 上一次会话的实测指标（取不到一律 None，不编数字）。
#[derive(Clone, Debug, Default, Serialize)]
pub struct VoiceMetrics {
    pub frames: u64,
    pub samples: u64,
    pub ms: u64,
    pub rate: u32,
    pub coverage: Option<f64>,
    /// 首个有效 PCM 样本到达时刻（计划 §三 坑 6 要求实测的耗时之一）。
    pub first_pcm_ms: Option<u64>,
}

/// 语音通道：偏好 + 状态 + 在途会话闸门。
pub struct VoiceChannel {
    app: Mutex<Option<AppHandle>>,
    gateway: Mutex<Option<Arc<GatewayState>>>,
    settings: Mutex<VoiceSettings>,
    state: Mutex<VoiceState>,
    metrics: Mutex<Option<VoiceMetrics>>,
    /// 最近一次失败原因（页面状态卡直读；None = 无）。
    note: Mutex<Option<String>>,
    stop: Mutex<Arc<AtomicBool>>,
    busy: AtomicBool,
    /// mSBC/HID 通道（小米 1 代）的在途帧队列：HID 读线程投、语音会话取。
    msbc: Mutex<VecDeque<Box<[u8]>>>,
}

impl Default for VoiceChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceChannel {
    pub fn new() -> Self {
        Self {
            app: Mutex::new(None),
            gateway: Mutex::new(None),
            settings: Mutex::new(VoiceSettings::default()),
            state: Mutex::new(VoiceState::Idle),
            metrics: Mutex::new(None),
            note: Mutex::new(None),
            stop: Mutex::new(Arc::new(AtomicBool::new(false))),
            busy: AtomicBool::new(false),
            msbc: Mutex::new(VecDeque::new()),
        }
    }

    pub fn start(self: &Arc<Self>, app: AppHandle, gateway: Arc<GatewayState>, store: &Store) {
        *self.app.lock().expect("voice app poisoned") = Some(app);
        *self.gateway.lock().expect("voice gateway poisoned") = Some(gateway);
        if let Ok(Some(raw)) = store.get_setting(SETTINGS_KEY) {
            if let Ok(s) = serde_json::from_str::<VoiceSettings>(&raw) {
                super::powermap::set_voice_to_fn(voice_key_to_fn(&s));
                *self.settings.lock().expect("voice settings poisoned") = s;
            }
        }
    }

    /// 页面一次拿齐：状态 + 偏好 + 上次指标 + 可选输出设备。
    pub fn snapshot(&self) -> Value {
        json!({
            "state": *self.state.lock().expect("voice state poisoned"),
            "settings": *self.settings.lock().expect("voice settings poisoned"),
            "metrics": *self.metrics.lock().expect("voice metrics poisoned"),
            "note": *self.note.lock().expect("voice note poisoned"),
            "devices": output_devices(),
        })
    }

    /// 保存偏好（前端整份 JSON 进来，形状与 `VoiceSettings` 对齐）。
    pub fn set_settings(&self, store: &Store, raw: Value) -> Result<(), String> {
        let s: VoiceSettings =
            serde_json::from_value(raw).map_err(|e| format!("语音偏好解析失败：{e}"))?;
        if s.gain > 1000 {
            return Err("音量范围是 0…1000%".into());
        }
        if s.route == VoiceRoute::Gateway && s.stt_model.as_deref().unwrap_or("").is_empty() {
            return Err("路 3 需要选一个语音转写模型".into());
        }
        let raw = serde_json::to_string(&s).map_err(|e| e.to_string())?;
        store
            .set_setting(SETTINGS_KEY, &raw)
            .map_err(|e| e.to_string())?;
        // 语音键去向跟着路 2 走：开着且配了输入法 → 硬件 🌐；否则禁用。表在位就地重写。
        super::powermap::set_voice_to_fn(voice_key_to_fn(&s));
        super::powermap::reapply_if_active();
        *self.settings.lock().expect("voice settings poisoned") = s;
        Ok(())
    }

    /// 语音键按下（HID mic 键 down 沿）。
    pub fn press(self: &Arc<Self>, session: Arc<RemoteSession>, prof: &'static DeviceProfile) {
        let settings = self
            .settings
            .lock()
            .expect("voice settings poisoned")
            .clone();
        if settings.route == VoiceRoute::Off {
            return;
        }
        // 路 1：系统转写。ATVV 机型把设备麦音频直灌识别器（用户对着遥控器说话，
        // Mac 麦听不到）——走下方设备采集路径，出口换成识别管道；没有 ATVV 通道的
        // 设备退回 Mac 本机麦克风（语音键只是触发器），通道支持检查也跳过——
        // 报「这台遥控器没有语音通道」是假话。
        if settings.route == VoiceRoute::System && !matches!(prof.voice, VoiceSpec::Atvv(_)) {
            if self.busy.swap(true, Ordering::SeqCst) {
                return;
            }
            let stop = Arc::new(AtomicBool::new(false));
            *self.stop.lock().expect("voice stop poisoned") = stop.clone();
            *self.note.lock().expect("voice note poisoned") = None;
            self.set_state(VoiceState::Listening);
            tracing::info!(target: "remote::voice", profile = prof.id, route = ?settings.route, source = "mac-mic", "语音会话开始");
            let me = Arc::clone(self);
            tauri::async_runtime::spawn(async move {
                let r = me.clone().run_system(stop).await;
                me.busy.store(false, Ordering::SeqCst);
                match r {
                    Ok(()) => {
                        // 系统路没有设备音频链路可测：指标一律 None，不拿别路数字冒充。
                        *me.metrics.lock().expect("voice metrics poisoned") = None;
                        if me.state() != VoiceState::Injected {
                            me.set_state(VoiceState::Idle);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(target: "remote::voice", error = %e, "系统转写会话结束（错误）");
                        *me.note.lock().expect("voice note poisoned") = Some(e.clone());
                        me.set_state(VoiceState::Idle);
                        me.emit(json!({ "state": VoiceState::Idle, "error": e }));
                    }
                }
            });
            return;
        }
        // 说法按通道给准：把还没实现的 CustomGatt 报成「没有 ATVV」，用户会以为是
        // 连接问题，而真相是这条通道本仓没实现。
        let unsupported = match prof.voice {
            VoiceSpec::Atvv(_) | VoiceSpec::MsbcHid => None,
            VoiceSpec::CustomGatt => Some("这台遥控器的自定义 GATT 语音通道未实现（按固件适配）"),
            VoiceSpec::None => Some("这台遥控器没有语音通道"),
        };
        if let Some(msg) = unsupported {
            *self.note.lock().expect("voice note poisoned") = Some(msg.to_string());
            self.emit(json!({ "state": self.state(), "error": msg }));
            return;
        }
        if self.busy.swap(true, Ordering::SeqCst) {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        *self.stop.lock().expect("voice stop poisoned") = stop.clone();
        *self.note.lock().expect("voice note poisoned") = None;
        // 上一场残留的帧不得串进这一场（迟到帧混入会让开头多出半句话）。
        self.clear_msbc();
        self.set_state(VoiceState::Listening);
        // 路 1 + ATVV：先起识别线程（objc2 对象 !Send，专用 std::thread），
        // ATVV 会话把解码 PCM 灌进管道；发送端 drop 后识别端 endAudio 等定稿。
        let mut speech_tx = None;
        if settings.route == VoiceRoute::System {
            let (tx, rx) = mpsc::channel();
            speech_tx = Some(tx);
            let sp_stop = stop.clone();
            let on_text = self.text_sink();
            std::thread::spawn(move || {
                if let Err(e) = super::speech::run_piped(sp_stop, rx, on_text) {
                    tracing::warn!(target: "remote::voice", error = %e, "系统转写（设备音频）结束（错误）");
                }
            });
        }
        tracing::info!(target: "remote::voice", profile = prof.id, route = ?settings.route, "语音会话开始");
        // 路 2：会话开始切到配置的输入法（SayAll 同款），收尾还原；
        // 真切换了才记原输入法，没配/切失败都不动用户当前状态。
        let ime = settings
            .ime_source
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        // 路 2 且配了输入法：会话开始切到微信输入法。语音触发不用我们做——
        // 路 2 在位时语音键已被 powermap 在 HID 层改写为硬件 🌐，物理按下即触发
        // 输入法语音（按住说话式，松手定稿）。shortcut: 切换比物理 Fn 按下晚几毫秒，
        // 若当时不在微信输入法，第一次按下可能唤不出 HUD，第二次起正常；真遇到再前置。
        let prev_ime = match (&ime, settings.route) {
            (Some(id), VoiceRoute::Soundcard) => {
                let id2 = id.clone();
                let (before, prev, after) = self
                    .on_main(move || {
                        let before = super::ime::current_id();
                        let r = super::ime::switch_to(&id2);
                        (before, r, super::ime::current_id())
                    })
                    .unwrap_or((None, None, None));
                tracing::info!(target: "remote::voice", before = ?before, after = ?after, "路 2 输入法切换（after≠目标=切换没成）");
                prev
            }
            _ => None,
        };
        let me = Arc::clone(self);
        let msbc_session = matches!(prof.voice, VoiceSpec::MsbcHid);
        tauri::async_runtime::spawn(async move {
            let r = if msbc_session {
                me.run_msbc(settings, stop.clone()).await
            } else {
                me.run(session, prof, settings, stop.clone(), speech_tx)
                    .await
            };
            stop.store(true, Ordering::SeqCst);
            me.busy.store(false, Ordering::SeqCst);
            match r {
                Ok(m) => {
                    *me.metrics.lock().expect("voice metrics poisoned") = Some(m);
                    // 收尾必须回空闲：路 2 全程停在 Listening、路 3 没出文本时停在
                    // Transcribing，不回一下状态灯就会一直亮着「聆听中」。
                    // Injected 保留——页面据此显示这次定稿注入的文本。
                    if me.state() != VoiceState::Injected {
                        me.set_state(VoiceState::Idle);
                    }
                }
                Err(e) => {
                    tracing::warn!(target: "remote::voice", error = %e, "语音会话结束（错误）");
                    *me.note.lock().expect("voice note poisoned") = Some(e.clone());
                    me.set_state(VoiceState::Idle);
                    me.emit(json!({ "state": VoiceState::Idle, "error": e }));
                }
            }
            // 松手时物理 🌐 已随按键抬起，输入法自己定稿上屏；给输入法留出上屏时间
            // 再还原会话前输入法——切太早会把还没落定的转写掐掉。切了才还原，没切不动。
            if let Some(p) = prev_ime {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                me.on_main(move || super::ime::select(&p));
            }
        });
    }

    /// Toggle 触发（档案 `VoiceTrigger::Toggle`）：按一下开、再按一下停。
    /// 判据就是会话闸门本身——`busy` 为真说明这一按是「停」，否则是「开」。
    pub fn toggle(self: &Arc<Self>, session: Arc<RemoteSession>, prof: &'static DeviceProfile) {
        if self.busy.load(Ordering::SeqCst) {
            self.release();
        } else {
            self.press(session, prof);
        }
    }

    /// 语音键松开（HoldToTalk 的止沿）。
    pub fn release(&self) {
        self.stop
            .lock()
            .expect("voice stop poisoned")
            .store(true, Ordering::SeqCst);
    }

    /// mSBC/HID 通道的起止信号（20 字节控制报告的 `01`/`00`）。
    ///
    /// 与 HID 语音键并行驱动同一条会话：`press` 有 `busy` 闸门，两条来源同时到
    /// 也开不出第二场，所以不必判重。
    pub fn signal(
        self: &Arc<Self>,
        session: Arc<RemoteSession>,
        prof: &'static DeviceProfile,
        start: bool,
    ) {
        if start {
            if !self.busy.load(Ordering::SeqCst) {
                self.press(session, prof);
            }
        } else if self.busy.load(Ordering::SeqCst) {
            // 只在真有会话在跑时才认「结束」：形状撞上的杂报告不该去动别人的 stop 位。
            self.release();
        }
    }

    /// HID 读线程投递一条 mSBC 音频报告；没有会话在收就直接丢。
    pub fn feed(&self, payload: &[u8]) {
        if !self.busy.load(Ordering::SeqCst) {
            return;
        }
        let mut q = self.msbc.lock().expect("voice msbc poisoned");
        while q.len() >= MSBC_QUEUE {
            q.pop_front();
        }
        q.push_back(payload.to_vec().into_boxed_slice());
    }

    fn drain_msbc(&self) -> Vec<Box<[u8]>> {
        self.msbc
            .lock()
            .expect("voice msbc poisoned")
            .drain(..)
            .collect()
    }

    fn clear_msbc(&self) {
        self.msbc.lock().expect("voice msbc poisoned").clear();
    }

    fn state(&self) -> VoiceState {
        *self.state.lock().expect("voice state poisoned")
    }

    fn set_state(&self, s: VoiceState) {
        *self.state.lock().expect("voice state poisoned") = s;
        self.emit(json!({ "state": s }));
    }

    fn emit(&self, extra: Value) {
        if let Some(app) = self.app.lock().expect("voice app poisoned").clone() {
            let _ = app.emit("remote://voice", extra);
        }
    }

    /// Carbon TIS 必须在主线程调用（2026-11-21 崩溃实锤：islGetInputSourceListWithAdditions
    /// 在非主队列直接 SIGTRAP，与 CGEventPost 同一条 libdispatch 铁律）。投递主线程并
    /// 限时等结果——等不到返回 None 降级，绝不挂死事件线程。
    fn on_main<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let (tx, rx) = std::sync::mpsc::channel();
        let app = self.app.lock().expect("voice app poisoned").clone()?;
        app.run_on_main_thread(move || {
            let _ = tx.send(f());
        })
        .ok()?;
        rx.recv_timeout(std::time::Duration::from_secs(2)).ok()
    }

    /// 路 3 落点：本机网关的 OpenAI 兼容转写端点（端口走网关最终口径，改过端口也跟着走）。
    fn gateway_target(&self, model: &str) -> Result<(String, Option<String>, String), String> {
        let gw = self
            .gateway
            .lock()
            .expect("voice gateway poisoned")
            .clone()
            .ok_or("网关未就绪")?;
        let host = match gw.config.server.host.as_str() {
            "" | "0.0.0.0" | "::" => "127.0.0.1".to_string(),
            h => h.to_string(),
        };
        let port = gateway::effective_listen_port(&gw.store, &gw.config);
        Ok((
            format!("http://{host}:{port}/v1/audio/transcriptions"),
            gw.config.server.token().map(|t| t.to_string()),
            model.to_string(),
        ))
    }

    // ---------------------------------------------------------------- 会话

    /// 按偏好开出口（ATVV 与 mSBC 两条通道共用，不各写一份）。
    /// `speech_tx`：路 1 设备音频管道（press 起识别线程时给）；其余路为 None。
    fn open_sink(
        &self,
        settings: &VoiceSettings,
        rate: u32,
        speech_tx: Option<mpsc::Sender<Vec<i16>>>,
    ) -> Result<Sink, String> {
        Ok(match settings.route {
            VoiceRoute::Soundcard => {
                Sink::Soundcard(SoundSink::open(settings.device.clone(), rate)?)
            }
            VoiceRoute::Gateway => {
                let model = settings
                    .stt_model
                    .clone()
                    .filter(|m| !m.trim().is_empty())
                    .ok_or("路 3 需要选一个语音转写模型")?;
                let (url, token, model) = self.gateway_target(&model)?;
                Sink::Gateway {
                    url,
                    token,
                    model,
                    rate,
                    pcm: Vec::new(),
                }
            }
            VoiceRoute::Off => return Err("语音已关闭".into()),
            // 路 1 设备音频：识别管道由 press 建好递进来；没拿到说明状态不对。
            VoiceRoute::System => {
                Sink::Speech(speech_tx.ok_or("系统转写缺少音频管道（内部状态错误）")?)
            }
        })
    }

    /// 识别文本出口（路 1/路 3 同一上屏口径）：partial 回显页面，final 注入焦点框。
    fn text_sink(self: &Arc<Self>) -> impl Fn(String, bool) + Send + Sync + 'static {
        let me = Arc::clone(self);
        move |text: String, is_final: bool| {
            if is_final {
                me.inject_text(&text);
                me.set_state(VoiceState::Injected);
                me.emit(json!({ "state": VoiceState::Injected, "text": text }));
            } else {
                me.emit(json!({ "state": VoiceState::Listening, "partial": text }));
            }
        }
    }

    /// 路 1（Mac 麦克风）：系统转写会话。objc2 对象 !Send，整场会话在专用 std::thread
    /// 创建、运行、收尾；async 侧只等结果。
    async fn run_system(self: Arc<Self>, stop: Arc<AtomicBool>) -> Result<(), String> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let me = Arc::clone(&self);
        std::thread::spawn(move || {
            let on_text = me.text_sink();
            let _ = tx.send(super::speech::run(stop, on_text));
        });
        // 线程意外退出（panic）时 recv 断链，同样按失败报，不静默。
        rx.recv().await.ok_or("系统转写线程意外结束")?
    }

    async fn run(
        &self,
        session: Arc<RemoteSession>,
        prof: &'static DeviceProfile,
        settings: VoiceSettings,
        stop: Arc<AtomicBool>,
        speech_tx: Option<mpsc::Sender<Vec<i16>>>,
    ) -> Result<VoiceMetrics, String> {
        let spec = match prof.voice {
            VoiceSpec::Atvv(s) => s,
            _ => return Err("该遥控器没有语音通道".into()),
        };
        let p = session.peripheral().ok_or("遥控器未连接，先扫描并连接")?;
        let send = find_char(&p, atvv::CH_SEND)?;
        let audio = find_char(&p, atvv::CH_AUDIO)?;
        let status = find_char(&p, atvv::CH_STATUS)?;
        p.subscribe(&audio)
            .await
            .map_err(|e| format!("订阅音频特征失败：{e}"))?;
        p.subscribe(&status)
            .await
            .map_err(|e| format!("订阅状态特征失败：{e}"))?;
        let mut stream = p
            .notifications()
            .await
            .map_err(|e| format!("通知流订阅失败：{e}"))?;

        // 能力协商：拿不到 0B 响应就用档案兜底参数（不因此中止会话）。
        let mut caps: Option<Capabilities> = None;
        let _ = p
            .write(&send, spec.caps_request, WriteType::WithResponse)
            .await;
        let deadline = tokio::time::Instant::now() + CAPS_WAIT;
        while tokio::time::Instant::now() < deadline {
            let left = deadline - tokio::time::Instant::now();
            match tokio::time::timeout(left, stream.next()).await {
                Ok(Some(n)) => {
                    if let Some(c) = Capabilities::parse(&n.value) {
                        caps = Some(c);
                        break;
                    }
                }
                _ => break,
            }
        }
        let rate = caps.map(|c| c.sample_rate()).unwrap_or(16_000);
        let frame_bytes = caps
            .and_then(|c| (c.frame_bytes > 0).then_some(c.frame_bytes as usize))
            .unwrap_or(spec.frame_bytes as usize);
        // v1.0 起 MIC_OPEN 可省 codec；v0.4 必须带（发错会被 0C 0F 01 拒）。
        let codec = match caps {
            Some(c) if !c.supports_extend() => Some(if rate == 8_000 { 0x0001 } else { 0x0002 }),
            _ => None,
        };

        let mut sink = self.open_sink(&settings, rate, speech_tx)?;

        p.write(&send, &atvv::mic_open(codec), WriteType::WithResponse)
            .await
            .map_err(|e| format!("开麦失败：{e}（若报权限不足，请删除设备后重新配对）"))?;

        let mut acc = FrameAccumulator::new(frame_bytes);
        let mut dec = AdpcmDecoder::new();
        let mut integ = Integrity::new(rate);
        let started = Instant::now();
        let mut first_pcm_ms: Option<u64> = None;
        let mut sid = 0u8;
        let mut last_keep = Instant::now();
        let mut last_level = Instant::now();

        // 采集主循环：松手（stop）后进迟到窗。
        loop {
            if let Ok(Some(n)) =
                tokio::time::timeout(Duration::from_millis(50), stream.next()).await
            {
                if n.uuid == audio.uuid {
                    if first_pcm_ms.is_none() {
                        first_pcm_ms = Some(started.elapsed().as_millis() as u64);
                    }
                    for frame in acc.push(&n.value) {
                        integ.push(frame.len() * 2);
                        let pcm = dec.decode(&frame, spec.frame_header, spec.low_nibble_first);
                        // 页面那条电平曲线的数据源（增益前的原始电平，反映的是
                        // 「收到多少声音」，不是灌出去多大）。
                        let lv = atvv::level(&pcm);
                        if last_level.elapsed() >= LEVEL_EMIT {
                            last_level = Instant::now();
                            self.emit(json!({ "level": lv }));
                        }
                        sink.push(atvv::smooth(&pcm), settings.gain);
                    }
                } else if n.uuid == status.uuid {
                    match atvv::parse_control(&n.value) {
                        Some(Control::StreamStart { session: s }) => sid = s,
                        Some(Control::Resync {
                            predictor,
                            step_index,
                        }) => dec.resync(predictor, step_index),
                        _ => {}
                    }
                }
            }
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if last_keep.elapsed() >= KEEPALIVE {
                last_keep = Instant::now();
                let cmd = match spec.keepalive {
                    Keepalive::MicExtend => atvv::mic_extend(sid),
                    Keepalive::ReopenMicOpen => atvv::mic_open(codec),
                };
                let _ = p.write(&send, &cmd, WriteType::WithoutResponse).await;
            }
        }

        // 迟到音频窗：松手后设备还会推 300 ms 内的尾包，这段必须收（坑 5）。
        let late = Instant::now() + LATE_AUDIO;
        while Instant::now() < late {
            let left = late - Instant::now();
            if let Ok(Some(n)) = tokio::time::timeout(left, stream.next()).await {
                if n.uuid == audio.uuid {
                    for frame in acc.push(&n.value) {
                        integ.push(frame.len() * 2);
                        let pcm = dec.decode(&frame, spec.frame_header, spec.low_nibble_first);
                        sink.push(atvv::smooth(&pcm), settings.gain);
                    }
                }
            }
        }
        let _ = p
            .write(&send, &atvv::mic_close(sid), WriteType::WithResponse)
            .await;
        let _ = p.unsubscribe(&audio).await;
        let _ = p.unsubscribe(&status).await;
        // 不足一帧的尾巴也要投递（排空语义，尾音不得静默丢）。
        let tail = acc.take_partial();
        if !tail.is_empty() {
            integ.push(tail.len() * 2);
            let pcm = dec.decode(&tail, spec.frame_header, spec.low_nibble_first);
            sink.push(atvv::smooth(&pcm), settings.gain);
        }
        integ.set_duration(started.elapsed().as_millis() as u64);

        let metrics = VoiceMetrics {
            frames: integ.frames,
            samples: integ.samples,
            ms: integ.ms,
            rate,
            coverage: integ.coverage(),
            first_pcm_ms,
        };
        // 出口收尾：路 3 在这里转写并注入（定稿注入，partial 不注）。
        if matches!(sink, Sink::Gateway { .. }) {
            self.set_state(VoiceState::Transcribing);
        }
        if let Some(text) = sink.finish().await? {
            self.inject_text(&text);
            self.set_state(VoiceState::Injected);
            self.emit(json!({ "state": VoiceState::Injected, "text": text }));
        }
        sink.wait_drained();
        Ok(metrics)
    }

    /// mSBC/HID 会话（小米「普通 1 代」，计划 §八 P7）：没有 GATT 可订，
    /// 帧由 HID 读线程投进 `msbc` 队列，这里取帧解码后走同一个出口。
    ///
    /// 收尾三保险：控制报告的 `00`（`release` 置 stop）、帧流停摆 1.5 s
    /// （设备压根不发结束报告）、开场 3 s 等不到首帧（控制报告到了音频没来）。
    async fn run_msbc(
        &self,
        settings: VoiceSettings,
        stop: Arc<AtomicBool>,
    ) -> Result<VoiceMetrics, String> {
        let rate = msbc::SAMPLE_RATE;
        // mSBC 机型的路 1 走 Mac 麦（press 已分流），这里永远收不到识别管道。
        let mut sink = self.open_sink(&settings, rate, None)?;
        let mut dec = msbc::MsbcHid::new();
        let mut integ = atvv::Integrity::new(rate);
        let started = Instant::now();
        let mut first_pcm_ms: Option<u64> = None;
        let mut last_frame: Option<Instant> = None;
        let mut last_level = started;
        let mut late: Option<Instant> = None;

        loop {
            let frames = self.drain_msbc();
            if frames.is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            } else {
                last_frame = Some(Instant::now());
                for f in frames {
                    let pcm = dec.push(&f);
                    if pcm.is_empty() {
                        continue;
                    }
                    if first_pcm_ms.is_none() {
                        first_pcm_ms = Some(started.elapsed().as_millis() as u64);
                    }
                    integ.push(pcm.len());
                    let lv = atvv::level(&pcm);
                    if last_level.elapsed() >= LEVEL_EMIT {
                        last_level = Instant::now();
                        self.emit(json!({ "level": lv }));
                    }
                    sink.push(atvv::smooth(&pcm), settings.gain);
                }
            }
            if stop.load(Ordering::SeqCst) {
                // 松手后仍收 LATE_AUDIO 的尾帧（与 ATVV 同一口径，尾字不得丢）。
                late = Some(late.unwrap_or_else(|| Instant::now() + LATE_AUDIO));
                if Instant::now() >= late.unwrap() {
                    break;
                }
            } else if match last_frame {
                // 首帧前给到 MSBC_FIRST（设备起播本就有延迟），首帧后 1.5 s 没帧就是真停了。
                Some(t) => t.elapsed() >= MSBC_IDLE,
                None => started.elapsed() >= MSBC_FIRST,
            } {
                break;
            }
        }

        let first = first_pcm_ms.is_some();
        integ.set_duration(started.elapsed().as_millis() as u64);
        let metrics = VoiceMetrics {
            frames: integ.frames,
            samples: integ.samples,
            ms: integ.ms,
            rate,
            coverage: integ.coverage(),
            first_pcm_ms,
        };
        if matches!(sink, Sink::Gateway { .. }) {
            self.set_state(VoiceState::Transcribing);
        }
        let text = sink.finish().await;
        sink.wait_drained();
        if !first {
            return Err(
                "没收到音频：已收到开始指令但设备没推 mSBC 帧（把日志页的 HID 报告台账发给开发者）"
                    .into(),
            );
        }
        if let Some(text) = text? {
            self.inject_text(&text);
            self.set_state(VoiceState::Injected);
            self.emit(json!({ "state": VoiceState::Injected, "text": text }));
        }
        Ok(metrics)
    }

    /// 快速面板是否可见——「这条语音该由谁落字」的唯一判据。
    fn panel_open(&self) -> bool {
        let Some(app) = self.app.lock().expect("voice app poisoned").clone() else {
            return false;
        };
        app.get_webview_window(crate::gui::promptfloat::LABEL)
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false)
    }

    /// 定稿文本注入当前焦点输入框（等价输入法上屏，不碰剪贴板）。
    ///
    /// **面板可见时不注入**（计划 2026-11-26「双重注入必须解决」）：浮窗里的语音由前端
    /// 逐字写进回显编辑框，后端再 inject_text 就是把同一段话打两遍。非浮窗场景
    /// （普通遥控操作）后端注入照旧。门控只加在这一处，三个调用点共用同一个口径。
    fn inject_text(&self, text: &str) {
        if self.panel_open() {
            tracing::debug!(
                target: "llm_switch::voice",
                "面板可见：语音定稿交前端回显编辑框，后端不注入（避免双重注入）"
            );
            return;
        }
        let Some(app) = self.app.lock().expect("voice app poisoned").clone() else {
            return;
        };
        let text = text.to_string();
        // 注入主线程铁律（CGEventPost 非主线程会带走进程）。
        let _ = app.run_on_main_thread(move || {
            if let Err(e) = super::platform::injector().inject_text(&text) {
                tracing::warn!(target: "remote::voice", error = %e, "文本注入失败");
            }
        });
    }
}

/// 在已发现的服务里按 UUID 找特征；找不到给可执行的出路（ATVV 写特征要已配对加密链路）。
fn find_char(p: &Peripheral, uuid: &str) -> Result<Characteristic, String> {
    let want = uuid::Uuid::parse_str(uuid).map_err(|e| e.to_string())?;
    p.services()
        .into_iter()
        .flat_map(|s| s.characteristics)
        .find(|c| c.uuid == want)
        .ok_or_else(|| format!("遥控器没有 ATVV 特征 {uuid}（请删除设备后重新配对）"))
}

// ---------------------------------------------------------------- 出口

/// 采集到的 PCM 的去向。
enum Sink {
    Soundcard(SoundSink),
    /// 路 1（ATVV 设备音频）：解码 PCM 直灌系统转写线程，出口不出文本
    /// （文本从识别回调走，与路 1 Mac 麦同一口径）。
    Speech(mpsc::Sender<Vec<i16>>),
    /// 路 3：整段收集，松手后一次性转写。
    Gateway {
        url: String,
        token: Option<String>,
        model: String,
        rate: u32,
        pcm: Vec<i16>,
    },
}

impl Sink {
    fn push(&mut self, pcm: Vec<i16>, gain: u16) {
        let scaled = atvv::apply_gain(&pcm, gain);
        match self {
            Sink::Soundcard(s) => s.push(&scaled),
            Sink::Speech(tx) => {
                let _ = tx.send(scaled);
            }
            Sink::Gateway { pcm: buf, .. } => buf.extend_from_slice(&scaled),
        }
    }

    /// 返回需要注入焦点框的定稿文本（路 2 没有文本，返回 None）。
    async fn finish(&mut self) -> Result<Option<String>, String> {
        match self {
            Sink::Gateway {
                url,
                token,
                model,
                rate,
                pcm,
            } => {
                if pcm.is_empty() {
                    return Err("没收到任何音频（检查遥控器是否连着、语音键是否生效）".into());
                }
                let wav = atvv::to_wav(pcm, *rate);
                let text = transcribe(url, token.as_deref(), model, wav).await?;
                Ok(Some(text))
            }
            Sink::Soundcard(_) | Sink::Speech(_) => Ok(None),
        }
    }

    fn wait_drained(&mut self) {
        if let Sink::Soundcard(s) = self {
            s.finish();
        }
    }
}

/// 把 WAV 发给网关 ASR 后端，取回文本。
async fn transcribe(
    url: &str,
    token: Option<&str>,
    model: &str,
    wav: Vec<u8>,
) -> Result<String, String> {
    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("voice.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .part("file", part);
    let mut req = reqwest::Client::new().post(url).multipart(form);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.map_err(|e| format!("转写请求失败：{e}"))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("转写响应解析失败：{e}"))?;
    if !status.is_success() {
        let msg = body
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .unwrap_or("网关返回错误");
        return Err(format!("转写失败：{msg}"));
    }
    let text = body
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("转写结果为空（没听清或模型不对）".into());
    }
    Ok(text)
}

// ---------------------------------------------------------------- 声卡出口（路 2）

/// 路 2：把 PCM 灌进指定输出设备。cpal 的 `Stream` 不是 `Send`，故整条流活在本线程里，
/// 采集侧只往共享环形缓冲写；松手后排空上限 750 ms 再释放设备。
struct SoundSink {
    buf: Arc<Mutex<VecDeque<i16>>>,
    src_rate: u32,
    dev_rate: u32,
    stop: Arc<AtomicBool>,
}

impl Drop for SoundSink {
    /// 提前退出（开麦失败、没收到音频、转写报错）也必须停掉播放线程：
    /// 不设 stop 那个线程就永远守着缓冲，线程泄漏且输出设备一直被占着。
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl SoundSink {
    fn open(name: Option<String>, src_rate: u32) -> Result<Self, String> {
        let buf = Arc::new(Mutex::new(VecDeque::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel::<Result<u32, String>>();
        let (buf2, stop2) = (Arc::clone(&buf), Arc::clone(&stop));
        std::thread::spawn(move || {
            // run_soundcard 播起来就立刻把设备采样率回传，再守着 stop 直到收尾。
            // 失败必须把原因回传：否则调用方只能报「打开超时」，把「没装虚拟声卡」
            // 这条可执行出路洗成一句看不懂的话。
            if let Err(e) = run_soundcard(name.as_deref(), &buf2, &stop2, &tx) {
                let _ = tx.send(Err(e));
            }
        });
        let dev_rate = rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "输出设备打开超时".to_string())??;
        Ok(Self {
            buf,
            src_rate,
            dev_rate,
            stop,
        })
    }

    fn push(&mut self, pcm: &[i16]) {
        let resampled = atvv::resample(pcm, self.src_rate, self.dev_rate);
        if let Ok(mut b) = self.buf.lock() {
            let keep = RING_FRAMES.saturating_sub(resampled.len());
            while b.len() > keep {
                b.pop_front();
            }
            b.extend(resampled);
        }
    }

    /// 松手收尾：置停 → 等排空（上限 DRAIN）→ 工作线程随后 drop 流释放设备。
    fn finish(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + DRAIN;
        while Instant::now() < deadline {
            let empty = self.buf.lock().map(|b| b.is_empty()).unwrap_or(true);
            if empty {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// cpal 0.18 把设备名挪进了 `description()`。
fn device_name(d: &cpal::Device) -> Option<String> {
    d.description().ok().map(|d| d.name().to_string())
}

fn run_soundcard(
    name: Option<&str>,
    buf: &Arc<Mutex<VecDeque<i16>>>,
    stop: &Arc<AtomicBool>,
    tx: &std::sync::mpsc::Sender<Result<u32, String>>,
) -> Result<(), String> {
    let host = cpal::default_host();
    let device = match name {
        Some(n) => host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| device_name(d).as_deref() == Some(n)),
        None => host.default_output_device(),
    }
    .ok_or("没找到输出设备：路 2 需要一个虚拟声卡（BlackHole 2ch 等），请先安装并在设置里选择")?;
    let cfg = device.default_output_config().map_err(|e| e.to_string())?;
    let rate = cfg.sample_rate();
    let channels = cfg.channels().max(1);
    let stream_cfg = cpal::StreamConfig {
        channels,
        ..cfg.config()
    };
    let err = |e: cpal::Error| {
        tracing::warn!(target: "remote::voice", error = %e, "声卡流错误");
    };
    let cb_buf = Arc::clone(buf);
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_output_stream(
                stream_cfg,
                move |data: &mut [f32], _| fill(data, &cb_buf, channels),
                err,
                None,
            )
            .map_err(|e| format!("输出流创建失败：{e}"))?,
        cpal::SampleFormat::I16 => device
            .build_output_stream(
                stream_cfg,
                move |data: &mut [i16], _| fill(data, &cb_buf, channels),
                err,
                None,
            )
            .map_err(|e| format!("输出流创建失败：{e}"))?,
        other => return Err(format!("输出设备采样格式不支持：{other:?}")),
    };
    stream.play().map_err(|e| format!("输出流启动失败：{e}"))?;
    let _ = tx.send(Ok(rate));
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }
    // 尾音排空后再释放设备（正常松手不等满上限，缓冲空即走）。
    let deadline = Instant::now() + DRAIN;
    while Instant::now() < deadline {
        if buf.lock().map(|b| b.is_empty()).unwrap_or(true) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(stream);
    Ok(())
}

/// 环形缓冲 → 交织声道（单声道源复制到每个声道）。回调里不许阻塞，锁忙就丢这一帧。
fn fill<T>(data: &mut [T], buf: &Arc<Mutex<VecDeque<i16>>>, channels: u16)
where
    T: cpal::Sample + cpal::FromSample<i16>,
{
    let Ok(mut b) = buf.try_lock() else { return };
    for frame in data.chunks_mut(channels.max(1) as usize) {
        let s = b.pop_front().unwrap_or(0);
        for cell in frame {
            *cell = T::from_sample(s);
        }
    }
}

/// 输出设备清单（页面选择器）：名字 + 是否默认。
pub fn output_devices() -> Value {
    let host = cpal::default_host();
    let default = host
        .default_output_device()
        .and_then(|d| device_name(&d))
        .unwrap_or_default();
    let list: Vec<String> = host
        .output_devices()
        .map(|ds| ds.filter_map(|d| device_name(&d)).collect())
        .unwrap_or_default();
    json!({ "devices": list, "default": default })
}
