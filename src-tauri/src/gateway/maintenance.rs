//! 后台维护循环（M3）：周期探活（健康灯）+ 亲和表修剪 + 日志保留双淘汰。
//! 与流量驱动的熔断相互独立：探活只管健康灯，不计入熔断计数。

use crate::gateway::State;
use crate::types::now_ms;
use std::sync::Arc;
use std::time::Duration;

/// Spawn the loop; returns the JoinHandle so embedders may cancel it.
pub fn spawn(st: Arc<State>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let interval = st.config.server.maintenance_interval_secs;
        if interval == 0 {
            return;
        }
        loop {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            tick(&st).await;
        }
    })
}

/// M8-C：明细保留天数（settings `usage.retention_days`，默认 90，0=不清理）。
pub fn retention_days(st: &Arc<State>) -> i64 {
    match st.store.get_setting("usage.retention_days") {
        Ok(Some(v)) => v.trim().parse::<i64>().unwrap_or(90),
        _ => 90,
    }
}

/// settings 覆盖值读整数（网关日志保留）：读不到/解析不了 = None（回落配置文件）。
fn setting_i64(st: &Arc<State>, key: &str) -> Option<i64> {
    st.store
        .get_setting(key)
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<i64>().ok())
}

/// 网关日志保留天数（settings `logs.retention_days` 覆盖，未设回落配置文件值；下限 1 天）。
pub fn log_retention_days(st: &Arc<State>) -> i64 {
    setting_i64(st, "logs.retention_days")
        .unwrap_or(st.config.logs.retention_days)
        .max(1)
}

/// 网关日志体积预算 MB（settings `logs.max_mb` 覆盖，未设回落配置文件值；下限 1 MB）。
pub fn log_max_mb(st: &Arc<State>) -> i64 {
    setting_i64(st, "logs.max_mb")
        .unwrap_or(st.config.logs.max_mb as i64)
        .max(1)
}

pub async fn tick(st: &Arc<State>) {
    // 0) 明细保留策略：老于水位的 usage_records 删除（usage_daily 永久）。
    let days = retention_days(st);
    if days > 0 {
        let watermark = crate::types::now_ms() - days * 24 * 3600 * 1000;
        if let Ok(n) = st.store.prune_usage_records(watermark) {
            if n > 0 {
                tracing::debug!(target: "llm_switch::maintain", pruned = n, "usage records pruned by retention");
            }
        }
    }
    // 1) 探活：所有启用后端逐个 ping，结果写健康列（UI 健康灯）。
    let rows = st.store.list_backends().unwrap_or_default();
    let enabled: Vec<(i64, String)> = rows
        .iter()
        .filter(|b| b.enabled)
        .map(|b| (b.id, b.name.clone()))
        .collect();
    // 健康的 (id, engine)：供下面的 KV 遥测采集只跑活着的后端。
    let mut healthy: Vec<(i64, Option<String>)> = Vec::new();
    for (id, name) in &enabled {
        match crate::services::backend::probe(&st.store, name).await {
            Ok(out) => {
                if out.healthy {
                    healthy.push((
                        *id,
                        rows.iter()
                            .find(|r| r.name == *name)
                            .and_then(|r| r.engine.clone()),
                    ));
                    // 能力自动探测（M5-E）：只填空白（engine IS NULL），不覆盖用户指定。
                    if let Some(row) = rows.iter().find(|r| r.name == *name) {
                        if row.engine.is_none() {
                            let client = reqwest::Client::builder()
                                .timeout(std::time::Duration::from_secs(3))
                                .build()
                                .unwrap_or_default();
                            if let Some(eng) =
                                crate::gateway::capability::detect_engine(&client, &row.base_url)
                                    .await
                            {
                                if let Ok(true) =
                                    st.store
                                        .fill_backend_engine(name, eng, &serde_json::json!({}))
                                {
                                    st.log.log(crate::gateway::log::Event {
                                        level: crate::gateway::log::Level::Info,
                                        code: "HM202 capability.detected",
                                        message: "engine auto-detected",
                                        request_id: None,
                                        backend: Some(name.as_str()),
                                        model: None,
                                        extra: serde_json::json!({ "engine": eng }),
                                    });
                                }
                            }
                        }
                    }
                }
                if !out.healthy {
                    st.log.log(crate::gateway::log::Event {
                        level: crate::gateway::log::Level::Warn,
                        code: "HM200 health.down",
                        message: &out.error.clone().unwrap_or_else(|| "probe failed".into()),
                        request_id: None,
                        backend: Some(name.as_str()),
                        model: None,
                        extra: serde_json::json!({ "latency_ms": out.latency_ms }),
                    });
                }
            }
            Err(e) => {
                tracing::warn!(target: "llm_switch::maintain", backend = %name, error = %e, "probe error")
            }
        }
    }

    // 1.5) KV 遥测（M5.5-D）：只拉健康后端；失败即移除旧快照（优雅退化）。
    if !healthy.is_empty() {
        let tc = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap_or_default();
        // 先拉完再落锁：std RwLock 守卫不能跨 await。
        let mut updates: Vec<(i64, Option<crate::gateway::telemetry::KvTelemetry>)> = Vec::new();
        for (id, engine) in &healthy {
            match rows.iter().find(|r| r.id == *id) {
                Some(r) => updates.push((
                    *id,
                    crate::gateway::telemetry::collect(&tc, &r.base_url, engine.as_deref()).await,
                )),
                None => updates.push((*id, None)),
            }
        }
        let mut map = st.telemetry.write().expect("telemetry lock poisoned");
        for (id, t) in updates {
            match t {
                Some(t) => {
                    map.insert(id, t);
                }
                None => {
                    map.remove(&id);
                }
            }
        }
        let alive: std::collections::HashSet<i64> = healthy.iter().map(|(i, _)| *i).collect();
        map.retain(|id, _| alive.contains(id));
    }

    // 2) 亲和表修剪（ttl 过期行）。
    let ttl_ms = st
        .routing
        .read()
        .expect("routing lock poisoned")
        .affinity
        .ttl_secs
        .max(1)
        * 1000;
    if let Ok(n) = st.store.prune_affinity(now_ms() - ttl_ms) {
        if n > 0 {
            tracing::debug!(target: "llm_switch::maintain", pruned = n, "affinity rows pruned");
        }
    }

    // 2.5) usage_daily 增量 rollup（游标推进，代价 O(新增行)）。
    match st.store.rollup_usage_daily() {
        Ok(n) if n > 0 => {
            tracing::debug!(target: "llm_switch::maintain", rows = n, "usage_daily rolled up")
        }
        Err(e) => {
            tracing::warn!(target: "llm_switch::maintain", error = %e, "usage_daily rollup failed")
        }
        _ => {}
    }

    // 3) 日志保留：按天数与体积双淘汰（先到为准）。
    //    两个数走设置覆盖（设置页日志卡），未覆盖回落配置文件——维护 tick 每轮现读，即时生效。
    let days = log_retention_days(st);
    let max_mb = log_max_mb(st);
    let _ = st.store.purge_logs_before(now_ms() - days * 86_400_000);
    if st.store.log_table_bytes().unwrap_or(0) > max_mb * 1024 * 1024 {
        // 超体积：再砍一半保留期，逐步收敛而不是一刀清空。
        let _ = st
            .store
            .purge_logs_before(now_ms() - (days * 86_400_000) / 2);
        st.log.log(crate::gateway::log::Event {
            level: crate::gateway::log::Level::Warn,
            code: "HM201 logs.over_budget",
            message: "log table over size budget, trimmed",
            request_id: None,
            backend: None,
            model: None,
            extra: serde_json::json!({ "max_mb": max_mb }),
        });
    }
}
