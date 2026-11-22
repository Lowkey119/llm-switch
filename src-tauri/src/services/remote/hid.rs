//! 按键通道（计划 S4）：hidapi 独占/共享打开 + 读线程 + 闸门接线。
//!
//! 为什么是 hidapi 而不是直调 IOHIDManager：本仓 `unsafe_code = "forbid"`，
//! io-kit-sys / objc2-io-kit 的报告回调注册都是 unsafe（编不进来）；hidapi 的
//! macOS 后端就是 IOHIDManager，`set_open_exclusive(true)` 即 `SeizeDevice`——
//! 「独占优先/共享回退」（计划 §四·五）在安全 Rust 下唯一可行的入口。
//!
//! 两种模式的语义（计划口径）：
//! - **独占**：系统收不到任何事件 → 全部动作由我们注入（不经 tap）。原生透传 =
//!   注入原生等价事件；连发由 RepeatGate 驱动。
//! - **共享**：系统照常收到事件 → 全原生配置 = 纯直通（零注入零抑制）；
//!   `disabled` 键登记抑制账本由 tap 吞掉原生事件；电源键必须先 hidutil 映射 F20
//!   才允许开（失败关闭，见 powermap）。
//!
//! 占用识别：独占与共享都打不开 = 设备被别的程序独占（Karabiner/SayAll），
//! 状态里说清楚，不静默。

use crate::services::remote::actions::{self, ButtonAction, ButtonConfig, Shortcut, Slot};
use crate::services::remote::buttons::NativeEvent;
use crate::services::remote::buttons::{
    native_event, repeat_interval, EdgeTracker, Gesture, GestureTracker, RapidGate, RemoteButton,
    RepeatGate, DOUBLE_WINDOW, LONG_HOLD,
};
use crate::services::remote::msbc;
use crate::services::remote::platform::{injector, interceptor, Injector, Interceptor};
use crate::services::remote::powermap;
use crate::services::remote::profile::{self, DeviceProfile, SeizePolicy, VoiceSpec, VoiceTrigger};
use crate::store::Store;
use hidapi::{HidApi, HidDevice};
use objc2_io_kit::{IOHIDAccessType, IOHIDCheckAccess, IOHIDRequestType};
use serde::Serialize;
use serde_json::json;
use std::ffi::CStr;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Generic Desktop 页 + Keyboard usage：键盘接口档案优先认领的接口（找不到就全试）。
const USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x01;
const USAGE_KEYBOARD: u16 = 0x06;
/// 设备巡检周期：hidapi 没有热插拔回调，靠 refresh 轮询（设备出现/消失都在这收敛）。
const SCAN_INTERVAL: Duration = Duration::from_secs(5);
/// 读超时：同时兼作节拍——独占模式连发 RepeatGate 与手势判定 GestureTracker 的 tick 都靠它。
const READ_TIMEOUT_MS: i32 = 50;

/// HID 通道对外状态（页面状态卡数据）。
#[derive(Clone, Serialize)]
pub struct HidStatus {
    /// `off` / `exclusive` / `shared` / `blocked`。
    pub mode: String,
    /// 失败原因（blocked 必有；运行中出错也记这里）。
    pub error: Option<String>,
    /// 最近一次解码到的按键 id（真机验收 12 键全通用，无数据 = None 不编）。
    pub last_button: Option<String>,
    /// 输入监控 TCC 是否已授权。
    pub input_monitoring: bool,
}

impl Default for HidStatus {
    fn default() -> Self {
        Self {
            mode: "off".to_string(),
            error: None,
            last_button: None,
            input_monitoring: false,
        }
    }
}

/// 未知输入报告的台账项：（条数、字节数、头 8 字节）。
type RawReport = (u64, usize, [u8; 8]);

/// 按键通道：按档案分仓的键位配置 + 状态 + 抑制账本 + 主线程投递句柄。
pub struct ButtonChannel {
    /// profile.id → 键位配置（21b 分仓：不同设备一张表，互相踩）。
    configs: Mutex<std::collections::HashMap<String, ButtonConfig>>,
    store: Mutex<Option<Arc<Store>>>,
    data_dir: Mutex<std::path::PathBuf>,
    status: Mutex<HidStatus>,
    /// 平台注入/拦截实现（S-X0：macOS = CGEvent + CGEventTap，其余平台见 platform 模块）。
    inj: Arc<dyn Injector>,
    intercept: Arc<dyn Interceptor>,
    /// 键表解不了的输入报告台账：report id →（条数、字节数、头 8 字节）。
    /// 这是「这台设备的语音到底走不走 HID」的唯一真话来源——小米 1 代据上游实测
    /// 把 mSBC 塞在 HID 报告里（不走 ATVV），没有这条探针就只能猜 macOS 收不收得到。
    raw_reports: Mutex<std::collections::BTreeMap<u8, RawReport>>,
    /// mSBC/HID 语音报告的收数（音频帧数、控制报告数）+ 每种 report id 首次落一行日志。
    /// 这是「macOS 到不到得了这台设备的语音」的读数（计划 §八 P7d）：0 就是收不到，
    /// 不用等解码出字才知道，也不用把整段原始字节贴出来对。
    msbc_hits: Mutex<(u64, u64, std::collections::BTreeSet<u8>)>,
    tap_started: AtomicBool,
    inject_warned: AtomicBool,
    shared_open: AtomicUsize,
    app: Mutex<Option<AppHandle>>,
    /// 语音通道与 BLE 会话（mic 键按下时由本通道转交，语音键本身不做按键动作）。
    voice: Mutex<Option<Arc<super::voice::VoiceChannel>>>,
    session: Mutex<Option<Arc<super::identity::RemoteSession>>>,
}

impl Default for ButtonChannel {
    fn default() -> Self {
        Self::new()
    }
}

pub fn input_monitoring_granted() -> bool {
    IOHIDCheckAccess(IOHIDRequestType::ListenEvent) == IOHIDAccessType::Granted
}

impl ButtonChannel {
    pub fn new() -> Self {
        Self {
            configs: Mutex::new(std::collections::HashMap::new()),
            store: Mutex::new(None),
            data_dir: Mutex::new(std::path::PathBuf::new()),
            status: Mutex::new(HidStatus::default()),
            inj: injector(),
            intercept: interceptor(),
            raw_reports: Mutex::new(std::collections::BTreeMap::new()),
            msbc_hits: Mutex::new((0, 0, std::collections::BTreeSet::new())),
            tap_started: AtomicBool::new(false),
            inject_warned: AtomicBool::new(false),
            shared_open: AtomicUsize::new(0),
            app: Mutex::new(None),
            voice: Mutex::new(None),
            session: Mutex::new(None),
        }
    }

    /// 起通道：记住数据目录（键位文件落点）+ 语音通道句柄 + 巡检线程。setup 里调一次。
    pub fn start(
        self: &Arc<Self>,
        app: AppHandle,
        store: Arc<Store>,
        data_dir: std::path::PathBuf,
        voice: Arc<super::voice::VoiceChannel>,
        session: Arc<super::identity::RemoteSession>,
    ) {
        *self.app.lock().expect("remote app handle poisoned") = Some(app);
        *self.voice.lock().expect("remote voice poisoned") = Some(voice);
        *self.session.lock().expect("remote session poisoned") = Some(session);
        *self.store.lock().expect("remote store poisoned") = Some(store);
        *self.data_dir.lock().expect("remote data dir poisoned") = data_dir;
        let me = Arc::clone(self);
        std::thread::spawn(move || me.monitor_loop());
    }

    /// 该档案的键位配置：map 缓存，未命中读文件（文件缺失自动从旧 settings 迁移）。
    pub fn config_for(&self, profile_id: &str) -> ButtonConfig {
        if let Some(c) = self
            .configs
            .lock()
            .expect("remote configs poisoned")
            .get(profile_id)
        {
            return c.clone();
        }
        let store = self.store.lock().expect("remote store poisoned").clone();
        let dir = self
            .data_dir
            .lock()
            .expect("remote data dir poisoned")
            .clone();
        let cfg = store
            .map(|s| actions::load(&dir, &s, profile_id))
            .unwrap_or_default();
        self.configs
            .lock()
            .expect("remote configs poisoned")
            .insert(profile_id.to_string(), cfg.clone());
        cfg
    }

    /// IPC 快照：当前档案的配置 + 通道状态（档案由命令层按绑定设备解析）。
    pub fn snapshot(&self, profile_id: &str) -> serde_json::Value {
        json!({
            "config": self.config_for(profile_id),
            "hid": *self.status.lock().expect("remote status poisoned"),
            "rawReports": self.raw_report_stats(),
            "voiceReports": self.msbc_stats(),
        })
    }

    /// 未知报告的台账（页面/排障直读）：`[{id, count, bytes, head}]`。
    fn raw_report_stats(&self) -> Vec<serde_json::Value> {
        self.raw_reports
            .lock()
            .expect("raw reports poisoned")
            .iter()
            .map(|(id, (count, bytes, head))| {
                json!({
                    "id": format!("{id:#04x}"),
                    "count": count,
                    "bytes": bytes,
                    "head": head.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
                })
            })
            .collect()
    }

    /// mSBC 语音收数（页面/排障直读）：`{audio, control}`。
    fn msbc_stats(&self) -> serde_json::Value {
        let (audio, control, _) = *self.msbc_hits.lock().expect("msbc hits poisoned");
        json!({ "audio": audio, "control": control })
    }

    /// 记一笔 mSBC 语音报告；该 report id 首次出现打一行日志（含头字节，只打一次）。
    fn note_msbc(&self, report_id: u8, payload: &[u8], audio: bool, profile_id: &str) {
        let mut g = self.msbc_hits.lock().expect("msbc hits poisoned");
        if audio {
            g.0 += 1;
        } else {
            g.1 += 1;
        }
        if !g.2.insert(report_id) {
            return;
        }
        tracing::info!(target: "remote::hid",
            profile = profile_id,
            report_id = report_id,
            kind = if audio { "音频帧" } else { "起止控制" },
            bytes = payload.len(),
            head = payload.iter().take(8).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
            "mSBC/HID 语音报告已到达（macOS 收得到这台设备的语音）");
    }

    /// 记一笔解不了的报告；每种 report id 首次出现打一行日志（只打一次，别刷盘）。
    fn note_raw_report(&self, id: u8, payload: &[u8], profile_id: &str) {
        let mut map = self.raw_reports.lock().expect("raw reports poisoned");
        let first = !map.contains_key(&id);
        let e = map.entry(id).or_insert((0, payload.len(), [0u8; 8]));
        e.0 += 1;
        for (i, b) in payload.iter().take(8).enumerate() {
            e.2[i] = *b;
        }
        if first {
            tracing::info!(target: "remote::hid",
                profile = profile_id,
                report_id = id,
                bytes = payload.len(),
                head = payload.iter().take(8).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "),
                "未识别的输入报告（键表解不了，已计入台账）");
        }
    }

    /// 保存该档案的配置（整体替换，写档案分仓文件并即时生效）。
    pub fn set_config(&self, profile_id: &str, config: ButtonConfig) -> Result<(), String> {
        let dir = self
            .data_dir
            .lock()
            .expect("remote data dir poisoned")
            .clone();
        actions::save(&dir, profile_id, &config)?;
        self.configs
            .lock()
            .expect("remote configs poisoned")
            .insert(profile_id.to_string(), config);
        Ok(())
    }

    /// 预存方案名列表（下拉内容，用户 2026-11-20 拍板）。
    pub fn scheme_names(&self, profile_id: &str) -> Vec<String> {
        actions::scheme_names(&self.data_dir(), profile_id)
    }

    /// 把当前生效配置存为命名方案（同名覆盖 = 「存当前为新方案」）。
    pub fn scheme_save(&self, profile_id: &str, name: &str) -> Result<(), String> {
        let cfg = self.config_for(profile_id);
        actions::save_scheme(&self.data_dir(), profile_id, name, &cfg)
    }

    /// 应用方案：整体覆盖当前映射（前端已二次确认）。方案本身带冲突则拒绝，不落盘。
    pub fn scheme_apply(&self, profile_id: &str, name: &str) -> Result<(), String> {
        let cfg = actions::get_scheme(&self.data_dir(), profile_id, name)
            .ok_or_else(|| format!("方案「{name}」不存在"))?;
        self.set_config(profile_id, cfg)
    }

    /// 改名方案（新名占用报错，不静默覆盖）。
    pub fn scheme_rename(&self, profile_id: &str, old: &str, new: &str) -> Result<(), String> {
        actions::rename_scheme(&self.data_dir(), profile_id, old, new)
    }

    /// 导出方案：整套 ButtonConfig 转 pretty JSON 文本（前端 blob 存盘）。
    pub fn scheme_export(&self, profile_id: &str, name: &str) -> Result<String, String> {
        let cfg = actions::get_scheme(&self.data_dir(), profile_id, name)
            .ok_or_else(|| format!("方案「{name}」不存在"))?;
        serde_json::to_string_pretty(&cfg).map_err(|e| format!("方案序列化失败：{e}"))
    }

    /// 导入方案：解析 JSON 存为指定名（同名覆盖，与「存新方案」同口径）。
    pub fn scheme_import(&self, profile_id: &str, name: &str, content: &str) -> Result<(), String> {
        let cfg: actions::ButtonConfig =
            serde_json::from_str(content).map_err(|e| format!("解析失败：{e}"))?;
        actions::save_scheme(&self.data_dir(), profile_id, name, &cfg)
    }

    fn data_dir(&self) -> std::path::PathBuf {
        self.data_dir
            .lock()
            .expect("remote data dir poisoned")
            .clone()
    }

    // ---------------------------------------------------------------- 巡检

    fn monitor_loop(self: &Arc<Self>) {
        let mut api = match HidApi::new() {
            Ok(a) => a,
            Err(e) => {
                self.set_status("blocked", Some(format!("HID 枚举失败：{e}")));
                return;
            }
        };
        let open_paths: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        loop {
            let granted = input_monitoring_granted();
            self.status
                .lock()
                .expect("remote status poisoned")
                .input_monitoring = granted;
            if !granted {
                // 失败关闭：没授权打开也读不到报告，还会被误判成「设备被占用」。
                self.set_status(
                    "blocked",
                    Some(
                        "输入监控未授权：系统设置→隐私与安全性→输入监控 勾选后重启本应用"
                            .to_string(),
                    ),
                );
            } else {
                let _ = api.refresh_devices();
                // 认领走档案（计划 2026-11-19 §四）：先 VID/PID，无 PnP 的退产品名。
                // 档案已登记但未落地（键表空/未真机）的不打开，只如实报状态。
                let mut candidates: Vec<(String, std::ffi::CString, &'static DeviceProfile)> =
                    Vec::new();
                let mut fallback: Vec<(String, std::ffi::CString, &'static DeviceProfile)> =
                    Vec::new();
                let mut unlanded: Vec<&'static str> = Vec::new();
                for info in api.device_list() {
                    let Some(prof) =
                        profile::by_pnp(info.vendor_id(), info.product_id(), info.product_string())
                    else {
                        continue;
                    };
                    if !prof.claimable() {
                        if !unlanded.contains(&prof.id) {
                            unlanded.push(prof.id);
                        }
                        continue;
                    }
                    let path = info.path().to_string_lossy().into_owned();
                    let cpath: std::ffi::CString = info.path().to_owned();
                    // 键盘接口优先只对键盘页档案有意义；其余一律直接候选。
                    // mSBC/HID 档案（小米 1 代）例外：语音帧可能挂在设备另一个
                    // HID 接口上（厂商/Consumer 页），只开键盘接口就永远读不到它，
                    // 所以每个接口都开一条读线程，谁收到音频谁投。
                    let preferred = matches!(prof.voice, VoiceSpec::MsbcHid)
                        || !matches!(prof.report, profile::ReportFormat::KeyboardLe16 { .. })
                        || (info.usage_page() == USAGE_PAGE_GENERIC_DESKTOP
                            && info.usage() == USAGE_KEYBOARD);
                    if preferred {
                        candidates.push((path, cpath, prof));
                    } else {
                        fallback.push((path, cpath, prof));
                    }
                }
                if candidates.is_empty() {
                    candidates = fallback;
                }
                let had_candidates = !candidates.is_empty();
                // 同一路径开多个句柄 = 同一服务的重复投递（IOHID 把每份报告发给所有
                // 打开的客户端），一次按键会分发 2~3 次、绑了快捷键的键连发。
                // BLE 设备的多个集合在 hidapi 里可能塌成同一路径，按路径去重。
                let mut opened: std::collections::HashSet<String> = open_paths
                    .lock()
                    .expect("remote open paths poisoned")
                    .iter()
                    .cloned()
                    .collect();
                for (path, cpath, prof) in candidates {
                    if !opened.insert(path.clone()) {
                        continue;
                    }
                    match self.open_device(&api, &cpath, prof) {
                        Ok((dev, exclusive)) => {
                            if !exclusive {
                                self.shared_open.fetch_add(1, Ordering::SeqCst);
                            }
                            open_paths
                                .lock()
                                .expect("remote open paths poisoned")
                                .push(path.clone());
                            let me = Arc::clone(self);
                            let paths = Arc::clone(&open_paths);
                            std::thread::spawn(move || {
                                me.reader_thread(dev, exclusive, prof, path, paths)
                            });
                        }
                        Err(e) => {
                            // 已有健康设备时不覆盖 mode：只在什么都没开时标 blocked。
                            let idle = open_paths
                                .lock()
                                .expect("remote open paths poisoned")
                                .is_empty();
                            if idle {
                                self.set_status("blocked", Some(e));
                            }
                        }
                    }
                }
                let still_open = open_paths.lock().expect("remote open paths poisoned").len();
                if still_open == 0 && !had_candidates {
                    // 只扫到未落地档案时，别谎报「没设备」——说清是哪款未验证。
                    match unlanded.first() {
                        Some(id) => self.set_status(
                            "off",
                            Some(format!("{id} 档案已登记但未落地（未验证），按键通道不接管")),
                        ),
                        None => self.set_status("off", None),
                    }
                }
            }
            std::thread::sleep(SCAN_INTERVAL);
        }
    }

    /// 独占优先、共享回退（计划 §四·五），但**独占必须先过权限门**（新计划事实 22）：
    /// 输入监控未授权时 seize 成功 = 遥控器变成「App 收不到报告、系统也用不了」的死设备。
    /// 鼠标类档案（`SeizePolicy::SharedOnly`）永不独占——抢走光标就是废了用户的鼠标。
    fn open_device(
        &self,
        api: &HidApi,
        path: &CStr,
        prof: &DeviceProfile,
    ) -> Result<(HidDevice, bool), String> {
        let may_seize =
            matches!(prof.seize, SeizePolicy::ExclusiveFirst) && input_monitoring_granted();
        if may_seize {
            api.set_open_exclusive(true);
            if let Ok(dev) = api.open_path(path) {
                self.set_status("exclusive", None);
                return Ok((dev, true));
            }
        }
        // 共享回退前必须先落设备级按键改写表：没落表就共享 = 一按电源键整机睡眠、
        // 原生键透传干扰映射。失败关闭。只有声明需要该映射的档案才做。
        if prof.power_map {
            powermap::apply_share_key_map()?;
        }
        api.set_open_exclusive(false);
        match api.open_path(path) {
            Ok(dev) => {
                self.set_status("shared", None);
                Ok((dev, false))
            }
            Err(e) => {
                // 只有没有别的共享设备在场才清表：表是设备级共用的，
                // 开一台失败就清，会把在用的表连坐抹掉（真机实锤：表被抹后
                // 电源键长按直通弹关机框）。
                if prof.power_map && self.shared_open.load(Ordering::SeqCst) == 0 {
                    powermap::clear_share_key_map();
                }
                Err(format!(
                    "设备打不开（独占与共享都失败，可能被 Karabiner/SayAll 占用）：{e}"
                ))
            }
        }
    }

    // ---------------------------------------------------------------- 读线程

    fn reader_thread(
        self: Arc<Self>,
        dev: HidDevice,
        exclusive: bool,
        prof: &'static DeviceProfile,
        path: String,
        open_paths: Arc<Mutex<Vec<String>>>,
    ) {
        let mut buf = [0u8; 128];
        let mut edges = EdgeTracker::default();
        let mut rapid = RapidGate::default();
        let mut repeat = RepeatGate::default();
        let mut gestures = GestureTracker::default();
        let mut seen_unknown: std::collections::HashSet<u16> = std::collections::HashSet::new();
        // 自愈看门狗计数：读节拍 50ms，200 拍 ≈ 10s 复核一次改写表。
        let mut map_watch: u32 = 0;
        // mSBC/HID 档案：把「该看什么」先写进日志，收不到时负答案也是答案。
        if matches!(prof.voice, VoiceSpec::MsbcHid) {
            tracing::info!(target: "remote::hid",
                profile = prof.id,
                report_id = prof.key_report_id().unwrap_or(0),
                path = %path,
                "该档案语音走 mSBC/HID：按住语音键后若本行之后没出现「语音报告已到达」，\
                 说明 macOS 没把这些报告交给 hidapi（计划 §八 P7d）");
        }
        loop {
            // 改写表可被外部抹掉（设备重连丢 IORegistry 属性、其他 app 整表覆盖），
            // 共享模式丢表 = 电源键直通弹关机框——发现缺失立刻重新落表。
            if !exclusive && prof.power_map {
                map_watch += 1;
                if map_watch >= 200 {
                    map_watch = 0;
                    if powermap::share_key_map_missing() {
                        tracing::warn!(target: "remote::hid", "按键改写表不在位（被外部清除？），重新落表");
                        if let Err(e) = powermap::apply_share_key_map() {
                            tracing::warn!(target: "remote::hid", "重新落表失败：{e}");
                        }
                    }
                }
            }
            match dev.read_timeout(&mut buf, READ_TIMEOUT_MS) {
                Ok(0) => {
                    // 无报告的间隙也要走节拍：手势判定（长按到点/单击窗到期）两种模式都要，
                    // 连发 RepeatGate 只在独占模式有意义（设备不自动重复）。
                    let now = Instant::now();
                    for (b, g) in gestures.tick(now) {
                        self.gesture_fired(b, g, exclusive, prof.id);
                    }
                    if exclusive {
                        for b in repeat.tick(now) {
                            self.inject_button(b);
                        }
                    }
                }
                Ok(n) => {
                    // mSBC/HID 档案（小米 1 代）：语音帧与起止控制报告先分流给语音通道。
                    // 它们键表本来就解不了，不先接住就会全灌进未知报告台账刷屏。
                    if matches!(prof.voice, VoiceSpec::MsbcHid)
                        && self.route_msbc(buf[0], &buf[1..n], prof)
                    {
                        continue;
                    }
                    // 报告格式与键表都由档案给（不跨档案查 usage）。
                    let parsed = prof.parse_report(buf[0], &buf[1..n]);
                    if parsed.is_none() {
                        self.note_raw_report(buf[0], &buf[1..n], prof.id);
                        continue;
                    }
                    if let Some(usages) = parsed {
                        let now = Instant::now();
                        // 未映射 usage 首次出现各报一次：语音键（mic）不在键表里，它的真实来源
                        // （哪个接口 / 哪个 usage）是三槽映射片 S-F1 的开工前提，没真机就只能猜
                        // ——这行日志就是拿真机回答它。
                        for u in &usages {
                            if prof.key_of(*u).is_none() && seen_unknown.insert(*u) {
                                tracing::info!(target: "remote::hid",
                                    profile = prof.id,
                                    report_id = buf[0],
                                    usage = format!("{u:#04x}"),
                                    "未映射 usage（可能是语音键或新键）");
                            }
                        }
                        let pressed: Vec<RemoteButton> =
                            usages.iter().filter_map(|u| prof.key_of(*u)).collect();
                        let (down, up) = edges.apply(&pressed);
                        let cfg = self.config_for(prof.id);
                        // 报告帧也走一次手势节拍：长按到点/单击窗到期可能正落在别的键的报告上。
                        for (b, g) in gestures.tick(now) {
                            self.gesture_fired(b, g, exclusive, prof.id);
                        }
                        for b in &up {
                            // 语音键不进手势判定：它的「动作」就是语音录入本身，
                            // 按下 = 开始采集、松开 = 停止（HoldToTalk，计划 §六）。
                            if *b == RemoteButton::Mic {
                                self.emit_key(*b, None, "up");
                                self.voice_end(prof);
                                continue;
                            }
                            if exclusive {
                                repeat.release(*b, now);
                            }
                            if let Some(g) = gestures.release(*b, now) {
                                self.gesture_fired(*b, g, exclusive, prof.id);
                            }
                            self.emit_key(*b, None, "up");
                        }
                        for b in &down {
                            if *b == RemoteButton::Mic {
                                self.note_last_button(*b);
                                self.emit_key(*b, None, "down");
                                self.voice_begin(prof);
                                continue;
                            }
                            if let Some(g) = gestures.press(*b, now) {
                                self.gesture_fired(*b, g, exclusive, prof.id);
                            }
                            self.emit_key(*b, None, "down");
                            let accepted =
                                self.dispatch_press(*b, exclusive, &cfg, now, &mut rapid);
                            if exclusive
                                && accepted
                                && cfg.action(*b) == ButtonAction::Native
                                // 手势驱动的键（有槽绑定）不进连发：连发注原生会与手势执行打架。
                                && !cfg.has_shortcuts(*b)
                                && repeat_interval(*b).is_some()
                            {
                                repeat.press(*b, now);
                            }
                        }
                        if exclusive {
                            for b in repeat.tick(now) {
                                self.inject_button(b);
                            }
                        }
                    }
                }
                // 读错 = 断连/设备没了：收尾退出，巡检线程会再认领。
                Err(_) => break,
            }
        }
        open_paths
            .lock()
            .expect("remote open paths poisoned")
            .retain(|p| *p != path);
        // 注意顺序：fetch_sub 必须无条件执行（它才是计数归零的判据），power_map 只看该档案是否需要。
        if !exclusive && self.shared_open.fetch_sub(1, Ordering::SeqCst) <= 1 && prof.power_map {
            tracing::info!(target: "remote::hid", path = %path, "共享通道收尾，清除按键改写表");
            powermap::clear_share_key_map();
        }
    }

    /// 按下分发。返回 false = 被连按闸门丢弃。
    fn dispatch_press(
        self: &Arc<Self>,
        button: RemoteButton,
        exclusive: bool,
        cfg: &ButtonConfig,
        now: Instant,
        rapid: &mut RapidGate,
    ) -> bool {
        self.note_last_button(button);
        // 真机验收逐键核对出口（12 键全通 = 每条都该出现一次）。
        tracing::info!(target: "remote::hid", button = button.id(), exclusive, "按键按下");
        let action = cfg.action(button);
        if exclusive {
            // 系统没收到任何事件：disabled 什么都不做；native 由我们注入等价事件。
            if action != ButtonAction::Native {
                return true;
            }
            if !rapid.allow(button, now, cfg.allows_rapid(button)) {
                return false;
            }
            // 独占替换（S-C，计划 §四）：任一槽有绑定 = 手势驱动——按下不注原生，
            // 等手势执行；未绑槽的手势在 execute_gesture 里注原生等价，透传不丢。
            if !cfg.has_shortcuts(button) {
                self.inject_button(button);
            }
            true
        } else {
            match action {
                // 直通。透传拦截主机制是设备级 UserKeyMapping（powermap 共享开启前
                // 整表落定），账本兜底（用户 2026-11-20：所有按键都要相应拦截）：
                // - 有槽绑定的键 = 手势驱动，原生透传必须吞，否则自定义动作与
                //   原生动作双触发；
                // - 设备级禁用键全部登记兜底——UserKeyMapping 会漏（菜单键 110
                //   实锤：0x65→0 已落表仍生成；电源键产物是改写后的 F20）；
                // - 方向键/音量未绑定时保持设计透传，不登记。音量是 systemDefined，
                //   tap 订不了（core-graphics 限制），绑定后原生音量仍会到系统。
                ButtonAction::Native => {
                    let gesture_driven = cfg.has_shortcuts(button);
                    let ledger_backstop = matches!(
                        button,
                        RemoteButton::Ok
                            | RemoteButton::Tv
                            | RemoteButton::Home
                            | RemoteButton::Menu
                            | RemoteButton::Back
                            | RemoteButton::Power
                    );
                    if gesture_driven || ledger_backstop {
                        self.intercept.arm(button, now);
                        self.ensure_tap();
                    }
                    true
                }
                // 禁用：把该键的原生事件登记进账本，由 tap 在窗口内吞掉。
                ButtonAction::Disabled => {
                    self.intercept.arm(button, now);
                    self.ensure_tap();
                    true
                }
            }
        }
    }

    // ---------------------------------------------------------------- 注入 / tap

    /// 注入按键的原生等价事件。注入必须主线程（CGEventPost 铁律，见 inject 模块头）。
    fn inject_button(self: &Arc<Self>, button: RemoteButton) {
        // 录制态一切注入停摆（含连发 RepeatGate 与将来 S-C 的快捷键注入）：
        // 用户拍板「录制时不要进行其他响应」，透传侧由 tap 统一拦。
        if self.intercept.recording() {
            return;
        }
        let ev = native_event(button);
        if ev == NativeEvent::None {
            return;
        }
        let Some(app) = self.app.lock().expect("remote app handle poisoned").clone() else {
            return;
        };
        let me = Arc::clone(self);
        let inj = self.inj.clone();
        if app
            .run_on_main_thread(move || {
                if let Err(e) = inj.inject_native(ev) {
                    me.warn_once(&format!("注入失败：{e}（辅助功能未授权？）"));
                }
            })
            .is_err()
        {
            self.warn_once("主线程未响应，注入丢弃");
        }
    }

    /// mSBC/HID 语音报告分流：音频帧投给语音通道，控制报告给会话起止。
    /// 返回是否已消费（没消费才继续走键表 / 未知报告台账）。
    fn route_msbc(&self, report_id: u8, payload: &[u8], prof: &'static DeviceProfile) -> bool {
        let Some(voice) = self.voice.lock().expect("remote voice poisoned").clone() else {
            return false;
        };
        // 按键接口的报告一律先走键表：语音帧与控制报告都在另一个 report id 上。
        // 混着认会让空键盘报告（首字节 0、其余补 0）冒充「语音结束」，半句话中间停会话。
        if prof.key_report_id() == Some(report_id) {
            return false;
        }
        if msbc::is_audio(payload) {
            self.note_msbc(report_id, payload, true, prof.id);
            voice.feed(payload);
            return true;
        }
        let start = match msbc::control(payload) {
            Some(start) => start,
            None => return false,
        };
        self.note_msbc(report_id, payload, false, prof.id);
        // 先取句柄再调用：`if let` 的临时量会活到块尾，锁不能跨到 press() 里。
        let session = self
            .session
            .lock()
            .expect("remote session poisoned")
            .clone();
        if let Some(s) = session {
            voice.signal(s, prof, start);
        }
        true
    }

    /// mic 键按下 → 交给语音通道开一次采集会话（未接语音时静默不做事）。
    /// 触发语义按档案：HoldToTalk 按下开、松开停；Toggle 按一下开、再按一下停。
    fn voice_begin(&self, prof: &'static DeviceProfile) {
        let voice = self.voice.lock().expect("remote voice poisoned").clone();
        let session = self
            .session
            .lock()
            .expect("remote session poisoned")
            .clone();
        if let (Some(v), Some(s)) = (voice, session) {
            match prof.voice_trigger {
                VoiceTrigger::Toggle => v.toggle(s, prof),
                _ => v.press(s, prof),
            }
        }
    }

    /// mic 键松开 → 停止采集（HoldToTalk 的止沿）。
    /// Toggle 的止沿是「下一次按下」，松开那一沿必须什么都不做。
    fn voice_end(&self, prof: &'static DeviceProfile) {
        if prof.voice_trigger == VoiceTrigger::Toggle {
            return;
        }
        if let Some(v) = self.voice.lock().expect("remote voice poisoned").clone() {
            v.release();
        }
    }

    /// 录制闸门（前端槽位聚焦/失焦调用）：true 起 tap 拦键盘转交录制器，false 放行。
    /// tap 起不来（无辅助功能授权）时静默降级——前端 keydown 兜底路径照常能录。
    pub fn set_recording(self: &Arc<Self>, active: bool) {
        self.intercept.set_recording(active);
        if active {
            self.ensure_tap();
        }
    }

    /// 共享模式出现非原生绑定时起抑制 tap（进程级一次；失败可重试并显示原因）。
    fn ensure_tap(self: &Arc<Self>) {
        if self.tap_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(app) = self.app.lock().expect("remote app handle poisoned").clone() else {
            self.tap_started.store(false, Ordering::SeqCst);
            return;
        };
        let me = Arc::clone(self);
        // tap 自带专用线程（inject::start_suppress_tap），不必再投主线程。
        match me.intercept.start(app) {
            Err(e) => {
                me.tap_started.store(false, Ordering::SeqCst);
                tracing::warn!(target: "remote::tap", error = %e, "抑制 tap 启动失败");
                // 强制上报（不吃 warn_once 闸门）：录制拦截/禁用键全押在这 tap 上，
                // 起不来必须让用户看见——多半是重编后辅助功能授权没重勾。
                let mut s = me.status.lock().expect("remote status poisoned");
                s.error = Some(format!("抑制器不可用：{e}；禁用键与录制拦截将不生效"));
            }
            Ok(()) => {
                let mut s = me.status.lock().expect("remote status poisoned");
                if s.error
                    .as_deref()
                    .is_some_and(|e| e.starts_with("抑制器不可用"))
                {
                    s.error = None;
                }
            }
        }
    }

    // ---------------------------------------------------------------- 状态

    fn set_status(&self, mode: &str, error: Option<String>) {
        let mut s = self.status.lock().expect("remote status poisoned");
        if s.mode != mode || s.error != error {
            // 状态变化落日志：真机验收（S4 候选→done）靠它核 mode 流转；页面 S6 接入前这也是唯一出口。
            tracing::info!(target: "remote::hid", mode, error = error.as_deref().unwrap_or("—"), "按键通道状态");
        }
        s.mode = mode.to_string();
        s.error = error;
    }

    fn note_last_button(&self, button: RemoteButton) {
        self.status
            .lock()
            .expect("remote status poisoned")
            .last_button = Some(button.id().to_string());
    }

    /// 实时按键事件（用户 2026-11-19 拍板；S-B 扩 payload）：映射页据此同步高亮
    /// 实物图热点与键卡。只读遥测、不带动作语义：`remote://key` =
    /// `{id, slot, phase}`，phase = down/up/gesture，slot = 手势归类的槽名（click/double/long）。
    fn emit_key(&self, button: RemoteButton, slot: Option<&'static str>, phase: &'static str) {
        if let Some(app) = self.app.lock().expect("remote app handle poisoned").clone() {
            let _ = app.emit(
                "remote://key",
                json!({ "id": button.id(), "slot": slot, "phase": phase }),
            );
        }
    }

    fn emit_gesture(&self, button: RemoteButton, gesture: Gesture) {
        self.emit_key(button, Some(gesture.slot_id()), "gesture");
    }

    /// 真键手势出口（S-C）：UI 事件 + 动作执行。演示 simulate 只走 emit_gesture
    /// （只演 UI、不碰真实快捷键），所以不受这里影响。
    fn gesture_fired(
        self: &Arc<Self>,
        button: RemoteButton,
        gesture: Gesture,
        exclusive: bool,
        profile_id: &str,
    ) {
        self.emit_key(button, Some(gesture.slot_id()), "gesture");
        self.execute_gesture(button, gesture, exclusive, profile_id);
    }

    /// 手势执行（S-C，计划 §四）：shared = 叠加注入（原生已照常透传，这里加塞一份组合）；
    /// exclusive = 替换（按下不注原生，手势命中未绑槽时在这里注原生等价）。禁用键不执行。
    fn execute_gesture(
        self: &Arc<Self>,
        button: RemoteButton,
        gesture: Gesture,
        exclusive: bool,
        profile_id: &str,
    ) {
        let cfg = self.config_for(profile_id);
        if cfg.action(button) != ButtonAction::Native {
            return;
        }
        let slot = match gesture {
            Gesture::Click => Slot::Click,
            Gesture::Double => Slot::Double,
            Gesture::Long => Slot::Long,
        };
        match cfg.shortcut(button, slot) {
            Some(sc) => self.inject_shortcut_gated(sc),
            None if exclusive && cfg.has_shortcuts(button) => self.inject_button(button),
            None => {}
        }
    }

    /// 注入快捷键组合。与 inject_button 同纪律：录制态停摆 + 主线程投递。
    fn inject_shortcut_gated(self: &Arc<Self>, sc: Shortcut) {
        if self.intercept.recording() {
            return;
        }
        let Some(app) = self.app.lock().expect("remote app handle poisoned").clone() else {
            return;
        };
        let me = Arc::clone(self);
        let inj = self.inj.clone();
        if app
            .run_on_main_thread(move || {
                if let Err(e) = inj.inject_shortcut(sc) {
                    me.warn_once(&format!("快捷键注入失败：{e}（辅助功能未授权？）"));
                }
            })
            .is_err()
        {
            self.warn_once("主线程未响应，注入丢弃");
        }
    }

    /// 按稳定 id / 手势名喂键（命令与演示驱动口共用这一份口径）。
    /// **仅 debug 构建可用**：release 直接明报错，不让线上被外部脚本驱键。
    /// 手势在后台线程跑完整时序（长按要真按住 500ms+），调用方不等。
    pub fn simulate_named(self: &Arc<Self>, id: &str, gesture: &str) -> Result<(), String> {
        if !cfg!(debug_assertions) {
            return Err("模拟按键只在 debug 构建可用".into());
        }
        let button = RemoteButton::from_id(id).ok_or_else(|| format!("未知键 id：{id}"))?;
        let gesture = match gesture {
            "click" => Gesture::Click,
            "double" => Gesture::Double,
            "long" => Gesture::Long,
            other => return Err(format!("未知手势：{other}（应为 click/double/long）")),
        };
        let ch = self.clone();
        std::thread::spawn(move || ch.simulate(button, gesture));
        Ok(())
    }

    /// 走一段节拍直到 `until`：长按到点、单击窗到期都靠 tick 出判定（与真键循环同口径）。
    fn tick_until(&self, g: &mut GestureTracker, until: Instant) {
        loop {
            let now = Instant::now();
            if now >= until {
                return;
            }
            for (b, gesture) in g.tick(now) {
                self.emit_gesture(b, gesture);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// 演示与自动化专用：把一颗键的整段手势喂进**与真键完全相同的出口**——同一个
    /// `GestureTracker` 判 click/double/long、同一份 `remote://key` payload，所以映射页的
    /// 高亮与闪示和真机一模一样。只演 UI：不做动作注入，免得模拟键踩到真实快捷键。
    /// release 构建由命令层（`cfg!(debug_assertions)`）拦住，这里不重复把关。
    pub fn simulate(&self, button: RemoteButton, gesture: Gesture) {
        let mut g = GestureTracker::default();
        // 一次按下-松开，再给单击窗留足时间让 tick 能出判定。
        let tap = |me: &Self, g: &mut GestureTracker, hold: Duration, after: Duration| {
            let t0 = Instant::now();
            if let Some(x) = g.press(button, t0) {
                me.emit_gesture(button, x);
            }
            me.emit_key(button, None, "down");
            me.tick_until(g, t0 + hold);
            let t1 = Instant::now();
            if let Some(x) = g.release(button, t1) {
                me.emit_gesture(button, x);
            }
            me.emit_key(button, None, "up");
            me.tick_until(g, t1 + after);
        };
        match gesture {
            Gesture::Click => tap(
                self,
                &mut g,
                Duration::from_millis(60),
                DOUBLE_WINDOW + Duration::from_millis(80),
            ),
            Gesture::Long => tap(
                self,
                &mut g,
                LONG_HOLD + Duration::from_millis(250),
                Duration::from_millis(80),
            ),
            // 双击：两次按下必须落在双击窗内，判定才出 double（与真键同口径）。
            Gesture::Double => {
                let t0 = Instant::now();
                if let Some(x) = g.press(button, t0) {
                    self.emit_gesture(button, x);
                }
                self.emit_key(button, None, "down");
                self.tick_until(&mut g, t0 + Duration::from_millis(60));
                let t1 = Instant::now();
                if let Some(x) = g.release(button, t1) {
                    self.emit_gesture(button, x);
                }
                self.emit_key(button, None, "up");
                self.tick_until(&mut g, t0 + Duration::from_millis(140));
                let t2 = Instant::now();
                if let Some(x) = g.press(button, t2) {
                    self.emit_gesture(button, x);
                }
                self.emit_key(button, None, "down");
                self.tick_until(&mut g, t2 + Duration::from_millis(60));
                let t3 = Instant::now();
                if let Some(x) = g.release(button, t3) {
                    self.emit_gesture(button, x);
                }
                self.emit_key(button, None, "up");
                self.tick_until(&mut g, t3 + Duration::from_millis(80));
            }
        }
    }

    fn warn_once(&self, msg: &str) {
        if self.inject_warned.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut s = self.status.lock().expect("remote status poisoned");
        if s.error.is_none() {
            s.error = Some(msg.to_string());
        }
    }
}

/// 演示驱动口（**仅 debug 构建**，调用方把关）：回环上的一个极小 HTTP 口，
/// 让外部脚本能喂键——`curl 'http://127.0.0.1:8898/simulate?id=ok&gesture=click'`。
/// 走的是与 `remote_simulate_key` 完全相同的事件出口，所以录屏里的闪示与真机一致。
/// 只读回环、无鉴权、不做动作注入；端口用 `LLS_SIM_PORT` 改（默认 8898）。
pub fn spawn_sim_server(channel: Arc<ButtonChannel>) {
    let port: u16 = std::env::var("LLS_SIM_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8898);
    let addr = format!("127.0.0.1:{port}");
    std::thread::spawn(move || {
        let Ok(listener) = std::net::TcpListener::bind(&addr) else {
            tracing::warn!(target: "remote::sim", addr, "演示驱动口起不来（端口被占？）");
            return;
        };
        tracing::info!(target: "remote::sim", addr, "演示驱动口已监听（仅 debug）");
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 512];
            let n = stream.read(&mut buf).unwrap_or(0);
            let line = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = match line.split_whitespace().nth(1) {
                Some(path) if path.starts_with("/simulate") => {
                    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
                    let param = |k: &str| {
                        query.split('&').find_map(|kv| {
                            kv.split_once('=')
                                .filter(|(k2, _)| *k2 == k)
                                .map(|(_, v)| v.to_string())
                        })
                    };
                    match (param("id"), param("gesture")) {
                        (Some(id), Some(gesture)) => match channel.simulate_named(&id, &gesture) {
                            Ok(()) => format!("已喂键 {id}·{gesture}\n"),
                            Err(e) => format!("{e}\n"),
                        },
                        _ => "用法 /simulate?id=<键 id>&gesture=<click|double|long>\n".to_string(),
                    }
                }
                _ => "未知路径。用 /simulate?id=<键 id>&gesture=<click|double|long>\n".to_string(),
            };
            let head = format!(
                "HTTP/1.0 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
    });
}
