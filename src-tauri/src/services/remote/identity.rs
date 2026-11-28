//! BLE 身份与连接（计划 S1，macOS）。
//!
//! 事实基线（计划 §三，真机核实）：RC001/RC003 同 VID/PID `0x2717/0x32B8`、同 GATT；
//! 型号判定以 DIS `0x2A24` Model Number 为准（2 报 `RC001`、2 Pro 报 `RC003`），
//! 广播名只作连接前筛选不作型号判定。ATVV Service `AB5E0001-…B664`。
//!
//! 同名双设备（坑 1）：两只小米遥控器广播名/VID/PID 全同。本片只把候选收进
//! `candidates`；「首份含已知普通按键的真实报告才绑定」要等 S4 按键通道，
//! 缺真机验收一律标候选。

use crate::services::remote::profile::{self, DeviceProfile};
use crate::store::Store;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// DIS Model Number（0x2A24）：`ModelSource::Dis2A24` 档案的型号判定依据。
const DIS_MODEL: u16 = 0x2A24;
/// 标准设备信息服务（DIS）/ 电池服务 / 电量特征（0x2A19 = uint8 百分比）。
const DIS_SERVICE: u16 = 0x180A;
const BATTERY_SERVICE: u16 = 0x180F;
const BATTERY_LEVEL: u16 = 0x2A19;

/// 遥测探针结果：从「系统已连接」设备直读的真实数据（缺 = None 不编）。
#[derive(Clone, Debug)]
pub struct DeviceTelemetry {
    /// 读到的外设 id（型号兜底只认同台设备）。
    pub id: String,
    pub battery: Option<u8>,
    pub model: Option<String>,
}
/// 绑定持久化键（settings 表）：`"<profile>|<id>|<model>"`，重启后按 id 续接。
/// 旧格式 `"<id>|<model>"`（S1 时代）仍读得懂，按小米档案处理，不强迫用户重绑。
const BIND_KEY: &str = "remote.bind";

/// 解析绑定串 → (档案 id, 外设 id, 型号)。
fn parse_bind(raw: &str) -> Option<(&str, &str, Option<&str>)> {
    let mut it = raw.split('|');
    let a = it.next()?;
    let b = it.next()?;
    match it.next() {
        // 三段：新格式 profile|id|model。
        Some(model) => Some((a, b, Some(model))),
        // 两段：S1 旧格式 id|model，当时只支持小米。
        None => Some(("xiaomi_rc00x", a, Some(b))),
    }
}

/// 绑定记录里的档案 id（仅当绑定 id 与目标 id 是同台北）：候选表为空时的档案回落。
fn bind_profile(store: &Store, id: &str) -> Option<String> {
    let raw = store.get_setting(BIND_KEY).ok().flatten()?;
    let (pid, bid, _) = parse_bind(&raw)?;
    (bid == id).then(|| profile::by_id(pid).id.to_string())
}

/// 一台候选/已绑遥控器的对外形状（前端只读；字段缺失一律 None，不编）。
#[derive(Clone, Serialize)]
pub struct RemoteDevice {
    /// CoreBluetooth 外设 id（UUID 串）；跨重启稳定，续接就认它。
    pub id: String,
    /// 广播名（连接前筛选依据）。
    pub name: Option<String>,
    /// 型号（判定源由档案声明）；未连接读不到 = None。
    pub model: Option<String>,
    /// 是否已连接。
    pub connected: bool,
    /// 命中的设备档案 id（计划 2026-11-19 §四）；未识别 = None。
    pub profile: Option<String>,
    /// 该档案在本仓是否已落地可用（键表 + 真机）。false = 页面显示「未验证」。
    pub supported: bool,
}

/// 页头状态卡的数据（电量走 BLE 电池服务；固件版本随 S2+ 接入，缺 = None 不编）。
#[derive(Serialize)]
pub struct RemoteStatus {
    pub bound: Option<RemoteDevice>,
    /// 扫描到的白名单候选（含已绑那台）。
    pub candidates: Vec<RemoteDevice>,
    /// BLE 通道状态：`idle` / `scanning` / `connected`。
    pub ble: String,
    /// 遥控器电量 0-100；未连接/读不到 = None（不造假数字）。
    pub battery: Option<u8>,
    /// 绑定档案的键表（`RemoteButton::id()` 序）。空 = 按键未落地（未验证档案），
    /// 页面据此走「未验证」状态卡，不拿小米键位表冒充（队列 22）。
    pub profile_keys: Vec<String>,
    /// 绑定档案是否真机验过（AGENTS 口径：未验过一律标候选，页面挂「未验证」徽标）。
    pub profile_verified: bool,
    /// 绑定档案是否有语音通道（voice trigger 非 None）。
    pub profile_voice: bool,
}

type Adapter = btleplug::platform::Adapter;
type Peripheral = btleplug::platform::Peripheral;

/// 进程内会话：Adapter 句柄 + 已连外设表 + 最近一轮候选。
/// CoreBluetooth 回调要求主线程上下文，命令层负责投递（与 rateline 同口径）。
///
/// 多设备并发（21b）：`connected`/`telemetry` 按外设 id 分仓，GATT 互斥**每台一把**——
/// v0.2.90 踩过「同一 CBPeripheral 上并发 discover/read 互踩 CoreBluetooth 回调」，
/// 扩成多外设后若仍共用一把锁，一台探针会把另一台的 connect 整个卡住；
/// 锁按台分开后同台北互斥、跨台并行，互不牵连。
pub struct RemoteSession {
    adapter: Mutex<Option<Adapter>>,
    /// 已连外设（key = 外设 id）。BTreeMap：多设备时取用顺序稳定可复现。
    connected: Mutex<BTreeMap<String, Peripheral>>,
    /// 连接时记下的「外设 id → 档案」：候选表为空（重启未扫描）时据此选台/切页。
    connected_profiles: Mutex<BTreeMap<String, String>>,
    /// 最后按键的档案：即使该台没有 App 内 BLE 会话（只走系统 HID），页面也跟它走。
    active_profile: Mutex<Option<String>>,
    /// 每台外设一把 GATT 锁（首次用到时建，只增不删——设备数是个位数，不值得回收）。
    gatts: Mutex<BTreeMap<String, Arc<tokio::sync::Mutex<()>>>>,
    candidates: Mutex<Vec<RemoteDevice>>,
    /// 各台最近一次遥测探针结果（电量 + DIS 型号，key = 外设 id）：电量/型号不跨台串。
    telemetry: Mutex<BTreeMap<String, DeviceTelemetry>>,
}

impl Default for RemoteSession {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteSession {
    pub fn new() -> Self {
        Self {
            adapter: Mutex::new(None),
            connected: Mutex::new(BTreeMap::new()),
            connected_profiles: Mutex::new(BTreeMap::new()),
            active_profile: Mutex::new(None),
            gatts: Mutex::new(BTreeMap::new()),
            candidates: Mutex::new(Vec::new()),
            telemetry: Mutex::new(BTreeMap::new()),
        }
    }

    /// 取（或建）某台外设的 GATT 锁。
    fn gatt_for(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.gatts
            .lock()
            .expect("remote gatts poisoned")
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn adapter(&self) -> Result<Adapter, String> {
        if let Some(a) = self
            .adapter
            .lock()
            .expect("remote adapter poisoned")
            .clone()
        {
            return Ok(a);
        }
        use btleplug::api::Manager as _;
        let manager = btleplug::platform::Manager::new()
            .await
            .map_err(|e| format!("蓝牙管理器不可用：{e}"))?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| format!("蓝牙适配器枚举失败：{e}"))?;
        let a = adapters
            .into_iter()
            .next()
            .ok_or_else(|| "没有可用的蓝牙适配器".to_string())?;
        *self.adapter.lock().expect("remote adapter poisoned") = Some(a.clone());
        Ok(a)
    }

    /// 广播名是否命中任一档案白名单（大小写不敏感；广播名带尾随空格也认）。
    fn name_hit(name: Option<&str>) -> Option<&'static DeviceProfile> {
        profile::by_name(name)
    }

    /// 短扫描一轮：start_scan（档案声明的服务并集）→ 等首条 DeviceDiscovered 或超时 →
    /// stop_scan，再按「档案广播名 ∪ 档案服务」筛候选。
    pub async fn scan(&self) -> Result<Vec<RemoteDevice>, String> {
        use btleplug::api::{Central, CentralEvent, Peripheral as _, ScanFilter};
        use futures_util::StreamExt;
        let adapter = self.adapter().await?;
        let filter: Vec<Uuid> = profile::scan_services()
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect();
        adapter
            .start_scan(ScanFilter { services: filter })
            .await
            // 扫描失败不致命：macOS 蓝牙未授权时 CoreBluetooth 静默无回调，这里先报错。
            .map_err(|e| format!("扫描启动失败（检查蓝牙权限）：{e}"))?;
        // 给广播一点时间；事件流只用来等「扫到了」，设备清单从 peripherals() 拿。
        let mut events = adapter
            .events()
            .await
            .map_err(|e| format!("事件流订阅失败：{e}"))?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(2500);
        loop {
            match tokio::time::timeout_at(deadline, events.next()).await {
                Ok(Some(CentralEvent::DeviceDiscovered(_))) | Ok(None) | Err(_) => break,
                Ok(_) => continue,
            }
        }
        let _ = adapter.stop_scan().await;
        let mut found = Vec::new();
        for p in adapter
            .peripherals()
            .await
            .map_err(|e| format!("外设清单读取失败：{e}"))?
        {
            let Some(props) = p
                .properties()
                .await
                .map_err(|e| format!("外设属性读取失败：{e}"))?
            else {
                continue;
            };
            let hit = Self::name_hit(props.local_name.as_deref())
                .or_else(|| Self::name_hit(props.advertisement_name.as_deref()))
                .or_else(|| {
                    // 服务命中：按服务归属档案（多档案共用 ATVV 时取首个声明者）。
                    profile::PROFILES.iter().copied().find(|p| {
                        p.services.iter().any(|s| {
                            props
                                .services
                                .iter()
                                .any(|x| x.to_string().eq_ignore_ascii_case(s))
                        })
                    })
                });
            let Some(hit) = hit else { continue };
            let connected = p.is_connected().await.unwrap_or(false);
            found.push(RemoteDevice {
                id: p.id().to_string(),
                name: props
                    .advertisement_name
                    .or(props.local_name)
                    .or_else(|| Some("未知名称".into())),
                model: None,
                connected,
                profile: Some(hit.id.to_string()),
                supported: hit.supported(),
            });
        }
        *self.candidates.lock().expect("remote candidates poisoned") = found.clone();
        Ok(found)
    }

    /// 连接 + 按档案声明的判定源读型号 + 落绑定。
    pub async fn connect(&self, store: &Store, id: &str) -> Result<RemoteDevice, String> {
        use btleplug::api::{Central, Peripheral as _};
        let adapter = self.adapter().await?;
        let target = adapter
            .peripherals()
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|p| p.id().to_string() == id)
            .ok_or_else(|| "外设不在扫描结果里，先扫描".to_string())?;
        // 档案从候选里取（扫描时已按广播名/服务归属）；不在候选 = 小米（S1 旧行为）。
        let prof = self
            .candidates
            .lock()
            .expect("remote candidates poisoned")
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.profile.as_deref())
            .map(profile::by_id)
            .unwrap_or_else(|| profile::by_id("xiaomi_rc00x"));
        // 与同台北极探针互斥：同一 CBPeripheral 上并发 discover/read 会互踩回调，
        // 这是「已连接但型号未识别」的头号嫌疑（v0.2.90 真机复现）。别的台不受牵连。
        let gatt = self.gatt_for(id);
        let _g = gatt.lock().await;
        // 系统已连（HID 链路在）就不再重复 connect：对已连接外设重复 connect
        // 的 CoreBluetooth 回调行为不可靠，可能挂死或假失败。
        if !target.is_connected().await.unwrap_or(false) {
            target
                .connect()
                .await
                .map_err(|e| format!("连接失败：{e}"))?;
        }
        if let Err(e) = target.discover_services().await {
            tracing::warn!(target: "remote::identity", error = %e, "服务发现失败");
            return Err(format!("服务发现失败：{e}"));
        }
        let model = Self::read_model(&target, prof).await;
        tracing::info!(target: "remote::identity", profile = prof.id,
            model = model.as_deref().unwrap_or("—"), "连接完成");
        let name = target
            .properties()
            .await
            .ok()
            .flatten()
            .and_then(|p| p.advertisement_name.or(p.local_name));
        let dev = RemoteDevice {
            id: id.to_string(),
            name: name.clone(),
            model: model.clone(),
            connected: true,
            profile: Some(prof.id.to_string()),
            supported: prof.supported(),
        };
        self.connected
            .lock()
            .expect("remote connected poisoned")
            .insert(id.to_string(), target);
        self.connected_profiles
            .lock()
            .expect("remote connected profiles poisoned")
            .insert(id.to_string(), prof.id.to_string());
        // 绑定即持久化（profile|id|model），重启后 status 显示「已绑定未连接」，S2+ 再自动续连。
        let _ = store.set_setting(
            BIND_KEY,
            &format!("{}|{}|{}", prof.id, id, model.unwrap_or_default()),
        );
        Ok(dev)
    }

    /// 已连接外设（语音会话要在同一只外设上订阅 ATVV 特征）。未连接 = None。
    /// shortcut: 语音链目前是单台语义（mic 键不分台），多设备时取 id 最小的一台；
    /// 按绑定/按键来源选台归 S4 双设备绑定策略，届时改这里。
    pub fn peripheral(&self) -> Option<Peripheral> {
        self.connected
            .lock()
            .expect("remote connected poisoned")
            .values()
            .next()
            .cloned()
    }

    /// 档案 → 已连接外设 id：优先连接时记下的映射，候选表为空也能认。
    fn id_for_profile(&self, profile_id: &str) -> Option<String> {
        if let Some((id, _)) = self
            .connected_profiles
            .lock()
            .expect("remote connected profiles poisoned")
            .iter()
            .find(|(_, p)| p.as_str() == profile_id)
        {
            return Some(id.clone());
        }
        self.candidates
            .lock()
            .expect("remote candidates poisoned")
            .iter()
            .find(|c| c.profile.as_deref() == Some(profile_id) && c.connected)
            .map(|c| c.id.clone())
    }

    /// 已连接外设列表（语音选台校验 ATVV 特征用，按 id 稳定排序）。
    pub fn connected_peripherals(&self) -> Vec<Peripheral> {
        self.connected
            .lock()
            .expect("remote connected poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// 按档案选已连接的外设：两只小米同时在线时，语音跟最后按下的那只走。
    /// 认不到就退回 id 最小的一台（与 `peripheral()` 同兜底）。
    pub fn peripheral_for(&self, profile_id: &str) -> Option<Peripheral> {
        if let Some(id) = self.id_for_profile(profile_id) {
            if let Some(p) = self
                .connected
                .lock()
                .expect("remote connected poisoned")
                .get(&id)
                .cloned()
            {
                return Some(p);
            }
        }
        self.peripheral()
    }

    /// 把某档案标为"当前使用"：写回绑定记录，页面的图/映射/语音都随最后按下的设备走。
    /// 双设备同连时不再需要手动切换开关。
    pub fn activate_profile(&self, store: &crate::store::Store, profile_id: &str) {
        *self
            .active_profile
            .lock()
            .expect("remote active profile poisoned") = Some(profile_id.to_string());
        let Some(id) = self.id_for_profile(profile_id) else {
            return;
        };
        let model = self
            .candidates
            .lock()
            .expect("remote candidates poisoned")
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.model.clone())
            .or_else(|| {
                self.telemetry
                    .lock()
                    .expect("remote telemetry poisoned")
                    .get(&id)
                    .and_then(|t| t.model.clone())
            })
            .unwrap_or_default();
        let _ = store.set_setting(BIND_KEY, &format!("{profile_id}|{id}|{model}"));
    }

    /// 按档案声明的判定源读型号；读不到返回 None（型号未知 ≠ 某个具体型号，不猜）。
    async fn read_model(p: &Peripheral, prof: &DeviceProfile) -> Option<String> {
        use btleplug::api::Peripheral as _;
        if prof.model != profile::ModelSource::Dis2A24 {
            // 档案声明没有 0x2A24（DIY 飞鼠一类）——读不到才是诚实的。
            return None;
        }
        let model_uuid = btleplug::api::bleuuid::uuid_from_u16(DIS_MODEL);
        let svc = p
            .services()
            .into_iter()
            .find(|s| s.characteristics.iter().any(|c| c.uuid == model_uuid))?;
        let ch = svc.characteristics.iter().find(|c| c.uuid == model_uuid)?;
        let bytes = p.read(ch).await.ok()?;
        let s: String = bytes.iter().map(|b| *b as char).collect();
        let s = s.trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    /// 断开：会话 GATT 连接与系统级 HID 共享链路（探针可见的那条）都断。
    ///
    /// 用户点「断开」的语义是「遥控器离开本机」；只放掉本进程句柄是假断开——
    /// 设备实际还连着系统，状态卡会继续显示已连接。断开后清空遥测，
    /// 状态立刻落回「已绑定 · 未连接」，不等下一轮探针（60s）滞后说谎。
    /// 遥控器按电源键即可重连（与页面引导文案同口径）。
    pub async fn disconnect(&self) -> Result<(), String> {
        use btleplug::api::{bleuuid::uuid_from_u16, Central, Peripheral as _};
        let all: Vec<(String, Peripheral)> = self
            .connected
            .lock()
            .expect("remote connected poisoned")
            .iter()
            .map(|(id, p)| (id.clone(), p.clone()))
            .collect();
        for (id, p) in all {
            let gatt = self.gatt_for(&id);
            let _g = gatt.lock().await;
            let _ = p.disconnect().await;
        }
        if let Ok(adapter) = self.adapter().await {
            if let Ok(ps) = adapter
                .retrieve_peripherals(btleplug::api::RetrievePeripheralsOptions {
                    identifiers: None,
                    services: Some(vec![uuid_from_u16(BATTERY_SERVICE)]),
                })
                .await
            {
                for p in ps {
                    let name = p
                        .properties()
                        .await
                        .ok()
                        .flatten()
                        .and_then(|pr| pr.advertisement_name.or(pr.local_name));
                    if Self::name_hit(name.as_deref()).is_none() {
                        continue;
                    }
                    if p.is_connected().await.unwrap_or(false) {
                        let _ = p.disconnect().await;
                    }
                }
            }
        }
        self.connected
            .lock()
            .expect("remote connected poisoned")
            .clear();
        self.connected_profiles
            .lock()
            .expect("remote connected profiles poisoned")
            .clear();
        self.telemetry
            .lock()
            .expect("remote telemetry poisoned")
            .clear();
        Ok(())
    }

    /// 读一个标准 16-bit UUID 特征（服务 + 特征都精确匹配；失败 = None 不编）。
    async fn read_char(p: &Peripheral, service: u16, characteristic: u16) -> Option<Vec<u8>> {
        use btleplug::api::{bleuuid::uuid_from_u16, Peripheral as _};
        let suuid = uuid_from_u16(service);
        let cuuid = uuid_from_u16(characteristic);
        let ch = p
            .services()
            .into_iter()
            .find(|s| s.uuid == suuid)?
            .characteristics
            .into_iter()
            .find(|c| c.uuid == cuuid)?;
        p.read(&ch).await.ok()
    }

    /// 从「系统已连接设备」探一轮遥测：每台白名单设备各读电量（0x180F/0x2A19）+ DIS 型号（0x180A/0x2A24），
    /// 结果按外设 id 分仓**整表替换**（battery.rs 的循环调用）——本轮没探到的台如实消失，
    /// 不把旧值留着谎报「还连着」（断开后状态立刻落回，不等下一轮滞后说谎）。
    ///
    /// 为什么走 retrieve_connected 而不是扫描：遥控器连着主机时**不再广播**，
    /// 扫描永远看不见它（页面「扫描不到」的根因）；CoreBluetooth 的
    /// `retrieveConnectedPeripherals` 能拿到系统已连外设，GATT 连接与 HID 共享链路。
    /// 只认白名单广播名——鼠标/键盘的电池不串给遥控器；读不到 = None 不编。
    /// 顺带读型号：会话 connect 读 DIS 失败时（并发竞态、重复 connect 不可靠），
    /// status 用同 id 的探针型号兜底。每台只与自己那把 GATT 锁较劲，跨台并行。
    /// 返回本轮第一台的结果（页面状态卡单台语义）；None = 一台都没探到。
    pub async fn probe_telemetry(&self) -> Option<DeviceTelemetry> {
        use btleplug::api::{bleuuid::uuid_from_u16, Central, Peripheral as _};
        let adapter = self.adapter().await.ok()?;
        let peripherals = adapter
            .retrieve_peripherals(btleplug::api::RetrievePeripheralsOptions {
                identifiers: None,
                services: Some(vec![uuid_from_u16(BATTERY_SERVICE)]),
            })
            .await
            .ok()?;
        let mut round: BTreeMap<String, DeviceTelemetry> = BTreeMap::new();
        for p in peripherals {
            let name = p
                .properties()
                .await
                .ok()
                .flatten()
                .and_then(|pr| pr.advertisement_name.or(pr.local_name));
            let Some(hit) = Self::name_hit(name.as_deref()) else {
                continue;
            };
            let id = p.id().to_string();
            let gatt = self.gatt_for(&id);
            let _g = gatt.lock().await;
            // 已连接就不再 connect：对已连接外设重复 connect 的回调行为不可靠。
            if !p.is_connected().await.unwrap_or(false) && p.connect().await.is_err() {
                continue;
            }
            if p.discover_services().await.is_err() {
                continue;
            }
            let battery = Self::read_char(&p, BATTERY_SERVICE, BATTERY_LEVEL)
                .await
                .and_then(|v| v.first().copied())
                // 0x2A19 = uint8 百分比；>100 是坏值，宁缺毋滥。
                .filter(|v| *v <= 100);
            let model = match hit.model {
                profile::ModelSource::Dis2A24 => Self::read_char(&p, DIS_SERVICE, DIS_MODEL)
                    .await
                    .map(|b| -> String {
                        b.iter()
                            .map(|c| *c as char)
                            .collect::<String>()
                            .trim()
                            .to_string()
                    })
                    .filter(|s| !s.is_empty()),
                profile::ModelSource::AdvertiseName => None,
            };
            // 采纳系统已连外设作为本会话对该台的 GATT 句柄（同 id 已有则不覆盖，
            // 保住用户在本会话里显式 connect 的那份）。
            // 为什么必须采纳：遥控器连着主机时 HID 链路是**系统**建的，本进程重启后
            // 从没走 connect()，`connected` 是空的 → 语音会话拿不到外设，
            // 明明按键好用却报「遥控器未连接」（用户 2026-11-21 实机撞到）。
            self.connected
                .lock()
                .expect("remote connected poisoned")
                .entry(id.clone())
                .or_insert_with(|| p.clone());
            let tele = DeviceTelemetry { id, battery, model };
            tracing::info!(target: "remote::battery",
                battery = tele.battery.map(|v| v.to_string()).as_deref().unwrap_or("—"),
                model = tele.model.as_deref().unwrap_or("—"), "遥测轮询");
            round.insert(tele.id.clone(), tele);
        }
        let first = round.values().next().cloned();
        *self.telemetry.lock().expect("remote telemetry poisoned") = round;
        first
    }

    pub fn status(&self, store: &Store) -> RemoteStatus {
        use btleplug::api::Peripheral as _;
        let candidates = self
            .candidates
            .lock()
            .expect("remote candidates poisoned")
            .clone();
        let conn = self
            .connected
            .lock()
            .expect("remote connected poisoned")
            .clone();
        let ble = if !conn.is_empty() {
            "connected"
        } else if !candidates.is_empty() {
            "scanning"
        } else {
            "idle"
        };
        // 遥测探针每 60s 从「系统已连外设」里逐台探：探得到 = 遥控器此刻连着主机
        // （HID 共享链路在工作，按键与电量都是真的）。用户视角的「连接」是遥控器连着
        // 电脑，不是本进程占着 GATT 会话——所以探针探到同 id 也算已连接，
        // 不再出现「电量 73% 却显示未连接」的自相矛盾（用户 2026-11-19 指出）。
        let tele = self
            .telemetry
            .lock()
            .expect("remote telemetry poisoned")
            .clone();
        // 状态卡单台语义（用户 2026-11-21 组合框拍板）：「这台遥控器」优先认绑定记录
        // 那台——卡头组合框选谁就连谁、绑谁，页面跟着选择走；绑定那台没连着才退回
        // id 最小的一台（BTreeMap 序稳定，旧口径兜底）。逐台状态卡归 S6（队列 22）。
        // 绑定记录是"当前使用哪只"的唯一真相：即使 App 内没建该台 BLE 会话
        // （按键走系统 HID 即可用），也不能退回另一台的会话，否则页面切不过去。
        let bind_raw = store.get_setting(BIND_KEY).ok().flatten();
        let first = bind_raw
            .as_deref()
            .and_then(|raw| parse_bind(raw).map(|(_, id, _)| id.to_string()))
            .and_then(|id| conn.get(&id))
            .or_else(|| {
                if bind_raw.is_none() {
                    conn.values().next()
                } else {
                    None
                }
            });
        let bound = first.map(|p| {
            let id = p.id().to_string();
            let known = candidates.iter().find(|c| c.id == id);
            // 候选为空（重启后尚未扫描）时档案回落绑定记录（同「已绑未连」分支口径），
            // 否则档案字段全空，页面误显示「未验证」并藏起键位映射。
            let prof = known
                .and_then(|c| c.profile.clone())
                .or_else(|| bind_profile(store, &id));
            let supported = known.map(|c| c.supported).unwrap_or_else(|| {
                prof.as_deref()
                    .map(|p| profile::by_id(p).supported())
                    .unwrap_or(false)
            });
            RemoteDevice {
                name: known.and_then(|c| c.name.clone()),
                model: known.and_then(|c| c.model.clone()),
                id,
                connected: true,
                profile: prof,
                supported,
            }
        });
        // 会话没连但绑过：用持久化 id/model 显示；连接态看探针是否正探到（同 id）。
        let bound = bound.or_else(|| {
            let raw = store.get_setting(BIND_KEY).ok().flatten()?;
            let (pid, id, model) = parse_bind(&raw)?;
            let prof = profile::by_id(pid);
            Some(RemoteDevice {
                id: id.to_string(),
                name: None,
                model: model.filter(|m| !m.is_empty()).map(str::to_string),
                connected: tele.contains_key(id),
                profile: Some(prof.id.to_string()),
                supported: prof.supported(),
            })
        });
        // 型号兜底：会话 connect 读 DIS 失败时（并发竞态），用该台自己的遥测型号——
        // 按 id 取，不同设备不串型号。
        let bound = bound.map(|mut d| {
            if d.model.is_none() {
                d.model = tele.get(&d.id).and_then(|t| t.model.clone());
            }
            d
        });
        // 最后按键的档案优先：双设备同连时页面按它切图/键表/映射。
        // 必须连 id/型号一起切——只换档案会留下「型号是 1 代、图是 2 代」的打架画面。
        // 认不到那台外设时不改（宁可停在上一台，也不显示互相矛盾的读数）。
        let active = self
            .active_profile
            .lock()
            .expect("remote active profile poisoned")
            .clone();
        let bound = match (active, bound) {
            (Some(pid), Some(mut d)) => match self.id_for_profile(&pid) {
                Some(id) => {
                    let known = candidates.iter().find(|c| c.id == id);
                    d.id = id.clone();
                    d.name = known.and_then(|c| c.name.clone());
                    d.model = known
                        .and_then(|c| c.model.clone())
                        .or_else(|| tele.get(&id).and_then(|t| t.model.clone()));
                    d.connected = true;
                    d.profile = Some(profile::by_id(&pid).id.to_string());
                    Some(d)
                }
                None => Some(d),
            },
            (_, b) => b,
        };
        // 电量：绑定那台的遥测；状态卡还没绑定时退回第一台（同「单台语义」口径）。
        let battery = bound
            .as_ref()
            .and_then(|d| tele.get(&d.id))
            .or_else(|| tele.values().next())
            .and_then(|t| t.battery);
        // 档案只读字段（队列 22）：键表/验证态/语音能力由页面渲染键位形态与能力灯，
        // 档案事实唯一真相在 profile.rs，前端不抄表。未绑定 = 全空，页面不猜。
        let prof = bound
            .as_ref()
            .and_then(|b| b.profile.as_deref())
            .map(profile::by_id);
        let (profile_keys, profile_verified, profile_voice) = match prof {
            Some(p) => (
                p.keys.iter().map(|(_, b)| b.id().to_string()).collect(),
                p.verified,
                !matches!(p.voice_trigger, profile::VoiceTrigger::None),
            ),
            None => (Vec::new(), false, false),
        };
        RemoteStatus {
            bound,
            candidates,
            ble: ble.to_string(),
            battery,
            profile_keys,
            profile_verified,
            profile_voice,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertise_names_lowercase_match() {
        assert!(RemoteSession::name_hit(Some("Mi RC")).is_some());
        assert!(RemoteSession::name_hit(Some("小米蓝牙遥控器2 Pro")).is_some());
        assert!(RemoteSession::name_hit(Some(" arn9 ")).is_some());
        assert!(RemoteSession::name_hit(Some("AirPods")).is_none());
        assert!(RemoteSession::name_hit(None).is_none());
    }

    #[test]
    fn bind_string_accepts_new_and_legacy_format() {
        // 新格式 profile|id|model。
        assert_eq!(
            parse_bind("g9n9n|UUID-1|A0"),
            Some(("g9n9n", "UUID-1", Some("A0")))
        );
        // S1 旧格式 id|model → 归小米档案，不强迫老用户重绑。
        assert_eq!(
            parse_bind("UUID-2|RC003"),
            Some(("xiaomi_rc00x", "UUID-2", Some("RC003")))
        );
        // 型号可为空串（当时没读到）。
        assert_eq!(
            parse_bind("xiaomi_rc00x|UUID-3|"),
            Some(("xiaomi_rc00x", "UUID-3", Some("")))
        );
        assert_eq!(parse_bind("garbage"), None);
    }

    #[test]
    fn atvv_uuid_parses_and_is_declared_by_xiaomi_profile() {
        let svc = profile::by_id("xiaomi_rc00x").services[0];
        let u = Uuid::parse_str(svc).expect("ATVV service uuid parses");
        assert_eq!(u.to_string().to_uppercase(), svc);
    }

    #[test]
    fn gatt_lock_is_per_device_and_stable() {
        // 21b：每台一把 GATT 锁——同 id 拿到同一把（同台北互斥），异 id 各拿各的（跨台不牵连）。
        let s = RemoteSession::new();
        let a1 = s.gatt_for("dev-a");
        let a2 = s.gatt_for("dev-a");
        let b = s.gatt_for("dev-b");
        assert!(Arc::ptr_eq(&a1, &a2), "同台北必须共用一把锁");
        assert!(!Arc::ptr_eq(&a1, &b), "异台必须各一把，一台探针不卡另一台");
    }

    #[test]
    fn empty_session_status_is_honest() {
        // 没扫没连没绑：idle + 无绑定 + 无电量，一个假数字都不许有。
        let store = Store::in_memory().unwrap();
        let st = RemoteSession::new().status(&store);
        assert_eq!(st.ble, "idle");
        assert!(st.bound.is_none());
        assert!(st.battery.is_none());
        // 未绑定：档案字段全空，页面不拿任何档案冒充（队列 22）。
        assert!(st.profile_keys.is_empty() && !st.profile_verified && !st.profile_voice);
    }

    #[test]
    fn bind_profile_falls_back_only_for_same_device() {
        // 回归（队列 22）：重启后会话已连但候选表为空时，档案必须回落绑定记录，
        // 否则页面误显示「未验证」藏起键位映射。异台北的 id 不许串档案。
        let store = Store::in_memory().unwrap();
        assert_eq!(bind_profile(&store, "UUID-9"), None, "没绑过不编档案");
        store
            .set_setting(BIND_KEY, "xiaomi_rc00x|UUID-9|RC001")
            .unwrap();
        assert_eq!(
            bind_profile(&store, "UUID-9").as_deref(),
            Some("xiaomi_rc00x")
        );
        assert_eq!(bind_profile(&store, "UUID-other"), None, "异台不串档案");
    }

    #[test]
    fn bound_from_setting_reads_back_per_id_telemetry() {
        // 绑过但会话没连：连接态与型号都只认同 id 的遥测（多设备不串台）。
        let store = Store::in_memory().unwrap();
        store
            .set_setting(BIND_KEY, "xiaomi_rc00x|UUID-9|RC001")
            .unwrap();
        let s = RemoteSession::new();
        s.telemetry.lock().unwrap().insert(
            "UUID-9".to_string(),
            DeviceTelemetry {
                id: "UUID-9".to_string(),
                battery: Some(73),
                model: Some("RC001".to_string()),
            },
        );
        let st = s.status(&store);
        let bound = st.bound.expect("绑定应回读");
        assert_eq!(bound.id, "UUID-9");
        assert!(bound.connected, "探针正探到同 id = 已连接");
        assert_eq!(st.battery, Some(73));
        // 档案字段按绑定档案（xiaomi_rc00x 已验证）：有键、验过、有语音。
        assert!(st.profile_keys.contains(&"power".to_string()));
        assert!(!st.profile_keys.is_empty());
        assert!(st.profile_verified && st.profile_voice);
        // 遥测换到别的 id：绑定这台立刻落回未连接，电量不跨台串。
        let s2 = RemoteSession::new();
        s2.telemetry.lock().unwrap().insert(
            "UUID-other".to_string(),
            DeviceTelemetry {
                id: "UUID-other".to_string(),
                battery: Some(50),
                model: None,
            },
        );
        let st2 = s2.status(&store);
        assert!(!st2.bound.expect("绑定仍在").connected);
        assert_eq!(st2.battery, Some(50), "未绑定时退回第一台（单台语义）");
    }
}
