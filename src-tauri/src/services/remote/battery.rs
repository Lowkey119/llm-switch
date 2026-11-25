//! 电量轮询（gui + macOS）：从系统已连接设备读电池服务，60s 一轮。
//!
//! 为什么轮询而不是订阅通知：电池服务 0x2A19 的 notify 各家实现参差（小米遥控器
//! 未证实支持），且电量变化以小时计；60s 轮询一次 GATT 读，成本可忽略，
//! 换来「随时能看到电量」的确定性（SayAll 同款口径）。

use crate::services::remote::identity::RemoteSession;
use std::sync::Arc;
use std::time::Duration;

/// 轮询周期：电量变化慢，60s 足够新；也更礼貌（不给 BLE 链路加常驻负载）。
const POLL: Duration = Duration::from_secs(60);

/// 首轮延迟：等蓝牙适配器上电稳定再探。
const FIRST_DELAY: Duration = Duration::from_secs(5);

pub async fn poll_loop(session: Arc<RemoteSession>) {
    tokio::time::sleep(FIRST_DELAY).await;
    loop {
        // 探针内部按台分仓整表替换（21b），这里只管节奏。
        session.probe_telemetry().await;
        tokio::time::sleep(POLL).await;
    }
}
