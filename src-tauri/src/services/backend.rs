//! Backend liveness probing: `GET {base_url}{health_path|/v1/models}` with the
//! adapter's auth headers, then persist the verdict via `Store::record_health`.

use crate::gateway::backends::{adapter, adapter_for};
use crate::services::discover;
use crate::store::Store;
use http::HeaderMap;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProbeOutcome {
    pub name: String,
    pub healthy: bool,
    pub status: Option<u16>,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// 探活失败的报错口径（纯函数，便于单测）。
///
/// 以前这里是把 4 个候选端点的失败用「；」拼起来，每条再把 reqwest 原文截到
/// 60 字——截断点刚好落在 URL 中间，于是 UI 上看到的是
/// `/v1/models 连不上（error sending request for url (http://localhost:8000/v1/mode`。
/// 现在：连接级失败只报一条（站点根 + 人话原因）；有 HTTP 应答的失败按端点列，
/// 最多 3 条，其余折叠，避免把 `last_error` 撑爆。
fn failure_message(unreachable: Option<String>, attempts: Vec<String>) -> String {
    if let Some(one) = unreachable {
        return one;
    }
    if attempts.is_empty() {
        return "探活端点没有应答".to_string();
    }
    const MAX_LINES: usize = 3;
    if attempts.len() <= MAX_LINES {
        return attempts.join("；");
    }
    let total = attempts.len();
    let mut head: Vec<String> = attempts.into_iter().take(MAX_LINES).collect();
    head.push(format!("…等 {total} 个端点"));
    head.join("；")
}

pub async fn probe(store: &Store, name: &str) -> Result<ProbeOutcome, String> {
    let row = store
        .list_backends()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|b| b.name == name)
        .ok_or_else(|| format!("no backend named `{name}`"))?;

    // base_url 归一（去空白与结尾斜杠）：手敲出来的 ` http://127.0.0.1:8000/v1/ `
    // 以前会原样进 join_path，拼出带空格的 URL，报错还看不懂。
    let base = row.base_url.trim().trim_end_matches('/').to_string();
    let site = discover::origin(&base).unwrap_or_else(|| base.clone());

    // 没显式指定 health_path 就别死磕单一端点：跟「扫描模型」一样挨个试，
    // 否则 base_url 带 /v1、或引擎只认 /api/tags 的后端会被冤枉成 down。
    let paths: Vec<String> = match row.health_path.clone().filter(|p| !p.trim().is_empty()) {
        Some(p) => vec![p],
        None => discover::candidates()
            .iter()
            .map(|(_, p)| p.to_string())
            .collect(),
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let mut headers = HeaderMap::new();
    if let Some(adapter) = adapter_for(&row.protocol) {
        let pairs = adapter
            .auth_headers(row.api_key.as_deref())
            .map_err(|e| e.to_string())?;
        for (k, v) in pairs {
            headers.insert(k, v);
        }
    }

    let started = std::time::Instant::now();
    let mut status: Option<u16> = None;
    let mut attempts: Vec<String> = Vec::new();
    // 连接级失败：四个候选端点打的是同一个 origin，再试等于把同一条错误抄四遍
    // （实报：后端行的 last_error 变成四段半截 URL 的天书）。探到一条就收工。
    let mut unreachable: Option<String> = None;
    let mut healthy = false;
    for path in &paths {
        let url = adapter::join_path(&base, path);
        match client.get(&url).headers(headers.clone()).send().await {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if status.is_none() {
                    status = Some(code);
                }
                if resp.status().is_success() {
                    healthy = true;
                    break;
                }
                attempts.push(format!("{path} → HTTP {code}"));
            }
            Err(e) if e.is_connect() || e.is_timeout() => {
                unreachable = Some(format!("连不上 {site}（{}）", discover::explain(&e)));
                break;
            }
            Err(e) => {
                attempts.push(format!("{path} 打不开（{}）", discover::explain(&e)));
            }
        }
    }
    let latency_ms = started.elapsed().as_millis() as u64;
    let error = if healthy {
        None
    } else {
        Some(failure_message(unreachable, attempts))
    };

    let _ = store.record_health(name, healthy, error.as_deref());
    tracing::info!(
        target: "llm_switch::routing",
        backend = %name,
        healthy,
        ?status,
        latency_ms,
        "backend probed"
    );

    Ok(ProbeOutcome {
        name: name.to_string(),
        healthy,
        status,
        latency_ms,
        error,
    })
}

/// 托盘「快速切后端」的后端选择逻辑（也可被别处复用）：
/// `keep = Some(name)` → 只启用该后端；`keep = None` → 全部启用。
/// 返回本次真正改变启用态的后端名列表。
pub fn apply_backend_selection(store: &Store, keep: Option<&str>) -> Result<Vec<String>, String> {
    let rows = store.list_backends().map_err(|e| e.to_string())?;
    let mut changed = Vec::new();
    for b in rows {
        let want = match keep {
            Some(name) => b.name == name,
            None => true,
        };
        if b.enabled != want {
            store
                .set_backend_enabled(&b.name, want)
                .map_err(|e| e.to_string())?;
            changed.push(b.name);
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BackendUpsert, Protocol};

    fn store_two() -> Store {
        let store = Store::in_memory().unwrap();
        for name in ["a", "b"] {
            store
                .upsert_backend(&BackendUpsert {
                    name: name.into(),
                    base_url: "http://127.0.0.1:9".into(),
                    protocol: Protocol::OpenaiChat,
                    api_key: None,
                    weight: 1,
                    enabled: true,
                    engine: None,
                    webui_url: None,
                    webui_user: None,
                    webui_password: None,
                    health_path: None,
                    models: vec![],
                    aliases: serde_json::json!({}),
                    capabilities: serde_json::json!({}),
                    params: serde_json::json!({}),
                })
                .unwrap();
        }
        store
    }

    #[test]
    fn solo_and_all_selection() {
        let store = store_two();
        // 只留 b：a 被关掉
        let changed = apply_backend_selection(&store, Some("b")).unwrap();
        assert_eq!(changed, vec!["a".to_string()]);
        let flags: Vec<(String, bool)> = store
            .list_backends()
            .unwrap()
            .into_iter()
            .map(|b| (b.name, b.enabled))
            .collect();
        assert_eq!(
            flags,
            vec![("a".to_string(), false), ("b".to_string(), true)]
        );

        // 再点一次 b：没有变化
        assert!(apply_backend_selection(&store, Some("b"))
            .unwrap()
            .is_empty());

        // 全部启用：a 回来
        assert_eq!(
            apply_backend_selection(&store, None).unwrap(),
            vec!["a".to_string()]
        );
        assert!(store.list_backends().unwrap().iter().all(|b| b.enabled));
    }

    /// 端到端：真起一个只答 `/v1/models` 的小服务，探活必须绿。
    /// （`curl` 通、探活不通这种差异以前真发生过，光靠单测纯函数钉不住。）
    #[tokio::test]
    async fn probe_against_a_real_models_endpoint() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let body = r#"{"data":[{"id":"mock-a","max_model_len":8192}]}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&BackendUpsert {
                name: "live".into(),
                base_url: format!("http://127.0.0.1:{port}"),
                protocol: Protocol::OpenaiChat,
                weight: 1,
                enabled: true,
                models: vec![],
                ..Default::default()
            })
            .unwrap();
        let out = probe(&store, "live").await.unwrap();
        assert!(out.healthy, "探活应当成功：{:?}", out.error);
        assert_eq!(out.status, Some(200));
    }

    /// 端口没开（oMLX 停了的现场）：一条能读懂的错误，不再四段半截 URL。
    #[tokio::test]
    async fn probe_on_closed_port_reports_the_site_once() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // 端口随即空出来 → 连接被拒
        let store = Store::in_memory().unwrap();
        store
            .upsert_backend(&BackendUpsert {
                name: "down".into(),
                base_url: format!("http://127.0.0.1:{port}/v1"),
                protocol: Protocol::OpenaiChat,
                weight: 1,
                enabled: true,
                models: vec![],
                ..Default::default()
            })
            .unwrap();
        let out = probe(&store, "down").await.unwrap();
        assert!(!out.healthy);
        let err = out.error.unwrap();
        assert!(
            err.starts_with(&format!("连不上 http://127.0.0.1:{port}")),
            "{err}"
        );
        assert_eq!(
            err.matches(&format!("127.0.0.1:{port}")).count(),
            1,
            "四个候选端点不许各抄一遍：{err}"
        );
        assert!(!err.contains("/v1/models"), "错误里别混端点路径：{err}");
    }

    /// 端口没开时只报一条站点根结论 —— 不许把 4 个候选端点的同一句错误各抄一遍
    /// （实报：omlx 行的 last_error 是四段被截断的半截 URL，读不出「端口没开」）。
    #[test]
    fn connect_failure_collapses_to_one_line() {
        let msg = failure_message(
            Some("连不上 http://localhost:8000（TCP 连不上（端口没在监听 / 服务未启动））".into()),
            vec!["/v1/models → HTTP 502".into()],
        );
        assert!(msg.starts_with("连不上 http://localhost:8000"));
        assert_eq!(msg.matches("连不上 http://localhost:8000").count(), 1);
        assert!(!msg.contains("/v1/models"));
    }

    /// 有 HTTP 应答（401/404）时按端点逐条列，超过 3 条折叠，别撑爆 last_error。
    #[test]
    fn http_failures_are_listed_but_folded() {
        let msg = failure_message(None, vec![]);
        assert_eq!(msg, "探活端点没有应答");

        let msg = failure_message(
            None,
            vec![
                "/v1/models → HTTP 401".into(),
                "/models → HTTP 401".into(),
                "/api/tags → HTTP 404".into(),
                "/v1beta/models → HTTP 404".into(),
            ],
        );
        assert!(msg.contains("/v1/models → HTTP 401"));
        assert!(msg.contains("…等 4 个端点"));
        // 第 4 条被折叠，正文只留前三条。
        assert!(!msg.contains("/v1beta/models"));
    }
}
