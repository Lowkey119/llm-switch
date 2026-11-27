//! 访问令牌中间件（P0）。
//!
//! 背景：`server.access_token` 过去只在「客户端接管」时被写进各家客户端配置，
//! **网关侧从未校验**——FAQ 里「非回环监听必须设 token」的承诺形同虚设，
//! `/admin/*`（后端密钥、接管文件读写）、`/console/*`（注入凭据的反代）、
//! `/v1/*` 全部裸奔；配合 `CorsLayer::permissive()`，任意网页都能跨源调用。
//!
//! 口径（2026-10-12 拍板）：
//! * **未设 token**：全开，= 文档里的「open loopback proxy」；但此时必须监听
//!   回环地址，否则 `Config::validate()` 直接拒绝启动。
//! * **设了 token**：除 `GET /health` 与 CORS 预检（浏览器预检不带
//!   Authorization，见 `routes.rs`）外，全路由校验。
//!
//! 令牌的携带方式——前三种是各客户端 SDK 的习惯，后三种是浏览器侧的通道
//! （`<iframe>` 发不出自定义请求头）：
//! 1. `Authorization: Bearer <token>`（OpenAI / Anthropic / Codex / OpenCode）
//! 2. `x-api-key: <token>`（Anthropic SDK 用 `ANTHROPIC_API_KEY` 时）
//! 3. `x-goog-api-key: <token>`（Gemini 系客户端）
//! 4. `?access_token=`（仅 `/console/*`：控制台 iframe / 新窗口的入口 URL）
//! 5. `Referer` 里的同一份 query（仅 `/console/*`：iframe 内的同源子资源带不出
//!    自定义头，浏览器会把文档 URL 当 Referer 发回来）
//! 6. `llm_switch_console` Cookie（仅 `/console/*`：根页面响应时下发的 HttpOnly
//!    会话，兜底覆盖「SPA pushState 之后 Referer 丢令牌」；第三方 Cookie 被
//!    浏览器拦掉时自动退化到 5）
//! 7. **静态资源免令牌**（仅 `/console/*` 的 GET/HEAD + 静态后缀）：跨站 iframe
//!    里三条浏览器通道全断——第三方 Cookie 被 ITP 拦、WKWebView 对 ES module
//!    取源不发 Referer、子资源又发不出自定义头；而 module 图内部的
//!    `import "./x.js"` 相对说明符带不进 query 令牌（JS 正文改写太危险）。
//!    内容哈希 chunk / 字体 / 图片是惰性 UI 代码（Unsloth Studio 本身 AGPL
//!    开源），放行不泄漏任何凭据与数据；所有数据接口（/api、/admin、协议入口）
//!    照旧严格校验，且请求若带了令牌但不对，依旧拒。

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use super::State as GatewayState;

/// 控制台会话 Cookie 名（值就是令牌本身，HttpOnly 挡住页面 JS 读取）。
pub const CONSOLE_COOKIE: &str = "llm_switch_console";
/// Cookie 头里的前缀（`concat!` 只吃字面量，所以这里单独一份）。
const CONSOLE_COOKIE_PREFIX: &str = "llm_switch_console=";

/// 凭据味的 query 键：一律不转发给上游（客户端的 key 只该拿来找我们，
/// 不能被我们带给别人，见 `sanitize_forwarded_query`）。
const CREDENTIAL_QUERY_KEYS: &[&str] = &[
    "key",
    "apikey",
    "api_key",
    "access_token",
    "token",
    "auth",
    "authorization",
    "google_api_key",
    "signature",
    "sig",
];

pub async fn require_token(
    State(st): State<Arc<GatewayState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(expected) = st.config.server.token() else {
        return next.run(req).await; // 未设 = 开放回环代理（文档口径）
    };
    // CORS 预检（OPTIONS）不会携带 Authorization，拦了就把浏览器调用全废掉；
    // 预检本身不进业务，真正的请求还会再来一次并被校验。
    if req.method() == Method::OPTIONS {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    // /health 是探活端点，只回版本号与 uptime（load balancer、容器健康检查
    // 都靠它，且它不碰任何后端数据）。
    if path == "/health" {
        return next.run(req).await;
    }

    let presented = presented_tokens(&req);
    if !presented.iter().any(|t| ct_eq(t, expected)) {
        // 通道 7：跨站 iframe 的 module 图取源三条凭据通道全断（见模块头），
        // 惰性静态资源在无令牌时放行；带了令牌但不对的不享受此豁免。
        if presented.is_empty() && is_static_console_asset(&req, &path) {
            return next.run(req).await;
        }
        // 401 必须留痕：此前只回 JSON、日志无痕，内嵌页排障时无从分辨
        // 「补丁没跑（打到根路由）」还是「鉴权没过（补丁跑了但没带够凭据）」。
        // 只记方法与路径（不含 query）——令牌一个字节都不落。
        let msg = format!("401 {} {}（令牌缺失或不匹配）", req.method(), path);
        st.log.log(crate::gateway::log::Event {
            level: crate::gateway::log::Level::Warn,
            code: "AU401 auth.reject",
            message: &msg,
            request_id: None,
            backend: None,
            model: None,
            extra: json!({ "method": req.method().as_str(), "path": path }),
        });
        return unauthorized();
    }

    let mut resp = next.run(req).await;
    if is_console_document(&path) {
        // 控制台根文档请求：下发会话 Cookie，子资源后续自动带上（浏览器
        // 允许第三方 Cookie 时生效；被拦则靠 Referer 兜底）。
        append_console_cookie(&mut resp, expected);
    }
    resp
}

/// 本次请求上出现过的所有令牌候选（都按不可信处理，逐个做常量时间比对）。
fn presented_tokens(req: &Request) -> Vec<String> {
    let headers = req.headers();
    let mut out: Vec<String> = Vec::new();
    for name in ["authorization", "x-api-key", "x-goog-api-key"] {
        if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
            out.push(strip_bearer(v));
        }
    }
    let path = req.uri().path();
    if let Some(q) = req.uri().query() {
        for (k, v) in url::form_urlencoded::parse(q.as_bytes()) {
            let hit = match &*k {
                // 控制台入口（iframe / 新窗口）：URL 直达。
                "access_token" if is_console_path(path) => true,
                // Gemini 原生协议自己的凭据位置。
                "key" if path.starts_with("/v1beta/") => true,
                _ => false,
            };
            if hit {
                out.push(v.into_owned());
            }
        }
    }
    if is_console_path(path) {
        if let Some(t) = console_token_from_referer(req) {
            out.push(t);
        }
        if let Some(t) = console_token_from_cookie(headers) {
            out.push(t);
        }
    }
    out
}

/// `Bearer xxx` / `xxx` 两种形态（Anthropic 的 `x-api-key` 不带前缀）。
fn strip_bearer(v: &str) -> String {
    let v = v.trim();
    v.strip_prefix("Bearer ")
        .or_else(|| v.strip_prefix("bearer "))
        .unwrap_or(v)
        .trim()
        .to_string()
}

/// iframe 内的同源子资源：浏览器把文档 URL（带着 `?access_token=`）当
/// Referer 发回来。只认与本次请求同源的 Referer——跨源页面的 query 不算凭据。
fn console_token_from_referer(req: &Request) -> Option<String> {
    let referer = req.headers().get(header::REFERER)?.to_str().ok()?;
    let url = reqwest::Url::parse(referer).ok()?;
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    if format!("{host}:{port}") != self_authority(req)? {
        return None;
    }
    url.query_pairs()
        .find(|(k, _)| k == "access_token")
        .map(|(_, v)| v.into_owned())
}

/// 请求自己的 authority：HTTP/1.1 在 `Host` 头，HTTP/2 在 `:authority`（即 URI）。
fn self_authority(req: &Request) -> Option<String> {
    if let Some(a) = req.uri().authority() {
        return Some(a.as_str().to_string());
    }
    req.headers()
        .get(header::HOST)?
        .to_str()
        .ok()
        .map(str::to_string)
}

fn console_token_from_cookie(headers: &HeaderMap) -> Option<String> {
    for value in headers.get_all(header::COOKIE) {
        let text = value.to_str().ok()?;
        for pair in text.split(';') {
            if let Some(v) = pair.trim().strip_prefix(CONSOLE_COOKIE_PREFIX) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// `/console/…` 路径族（含根页面与全部子资源）。
fn is_console_path(path: &str) -> bool {
    path == "/console" || path.starts_with("/console/")
}

/// 惰性静态资源后缀（通道 7 豁免范围）：只收 UI 代码与媒体，**不含** `.json`
/// /`.html`——那两类可能承载运行时数据，必须走令牌。
const STATIC_CONSOLE_EXTS: &[&str] = &[
    ".js",
    ".mjs",
    ".css",
    ".map",
    ".woff",
    ".woff2",
    ".ttf",
    ".otf",
    ".eot",
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".svg",
    ".webp",
    ".ico",
    ".avif",
    ".webmanifest",
];

/// 通道 7 判定：GET/HEAD + `/console/<id>/<子路径>` + 静态后缀。
/// 根文档（`/console/<id>`）与数据接口不在豁免之列。
fn is_static_console_asset(req: &Request, path: &str) -> bool {
    if !matches!(req.method(), &Method::GET | &Method::HEAD) {
        return false;
    }
    let Some(rest) = path.strip_prefix("/console/") else {
        return false;
    };
    // 必须带子路径段（`<id>/assets/x.js`），`/console/<id>` 根文档不算。
    let Some(sub) = rest.split_once('/').map(|(_, s)| s) else {
        return false;
    };
    let lower = sub.rsplit('/').next().unwrap_or(sub).to_ascii_lowercase();
    STATIC_CONSOLE_EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// `/console/<id>` 根文档（没有后续斜杠段）——只有这里需要下发会话 Cookie。
/// 顺带说明：尾斜杠版本 `matchit` 不匹配（见 `routes.rs` M7 注释），故无须考虑。
fn is_console_document(path: &str) -> bool {
    let rest = match path.strip_prefix("/console/") {
        Some(r) => r,
        None => return false,
    };
    !rest.is_empty() && !rest.contains('/')
}

fn append_console_cookie(resp: &mut Response, token: &str) {
    let Ok(value) = HeaderValue::from_str(&format!(
        "{CONSOLE_COOKIE}={token}; Path=/console; HttpOnly; SameSite=Lax"
    )) else {
        return;
    };
    let already = resp
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|s| s.starts_with(CONSOLE_COOKIE_PREFIX));
    if !already {
        resp.headers_mut().append(header::SET_COOKIE, value);
    }
}

/// 常量时间比较：令牌是可枚举的短字符串，按位短路比较会把正确前缀泄漏给
/// 攻击者（时序侧信道）。
fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer realm=\"llm-switch\"")],
        Json(json!({
            "type": "error",
            "error": {
                "type": "authentication_error",
                "message": "缺少或错误的 server.access_token：请带 \
                            `Authorization: Bearer <token>`（Anthropic 也可用 `x-api-key`，\
                            Gemini 可用 `x-goog-api-key`）。刚启用令牌的话，\
                            请在设置页重新执行一次客户端接管。"
            }
        })),
    )
        .into_response()
}

/// 剥掉凭据味的 query 再转发给上游：客户端带来的 Gemini `?key=` 是它来找我们
/// 用的凭据，不是给上游的（上游一律用我们登记的 api_key，见 `backends/gemini.rs`）。
/// 返回值带前导 `?` 或为空串，跟原实现的拼接口径一致。
pub fn sanitize_forwarded_query(query: Option<&str>) -> String {
    let Some(q) = query.filter(|q| !q.is_empty()) else {
        return String::new();
    };
    let kept: Vec<(String, String)> = url::form_urlencoded::parse(q.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .filter(|(k, _)| !is_credential_query_key(k))
        .collect();
    if kept.is_empty() {
        return String::new();
    }
    let mut ser = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in kept {
        ser.append_pair(&k, &v);
    }
    format!("?{}", ser.finish())
}

fn is_credential_query_key(k: &str) -> bool {
    CREDENTIAL_QUERY_KEYS
        .iter()
        .any(|c| c.eq_ignore_ascii_case(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_token(token: Option<&str>) -> crate::config::Config {
        let mut cfg = crate::config::Config::default();
        cfg.server.access_token = token.map(str::to_string);
        cfg
    }

    fn request(method: &str, uri: &str, headers: &[(&str, &str)]) -> Request {
        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        builder.body(axum::body::Body::empty()).unwrap()
    }

    #[test]
    fn constant_time_compare_works() {
        assert!(ct_eq("abc", "abc"));
        assert!(!ct_eq("abc", "abd"));
        assert!(!ct_eq("abc", "abcd"));
        assert!(!ct_eq("", "a"));
    }

    #[test]
    fn bearer_prefix_is_optional_and_case_insensitive() {
        assert_eq!(strip_bearer("Bearer  t1 "), "t1");
        assert_eq!(strip_bearer("bearer t2"), "t2");
        assert_eq!(strip_bearer("t3"), "t3");
    }

    #[test]
    fn token_helper_treats_blank_as_unset() {
        let mut cfg = cfg_with_token(Some("  "));
        assert_eq!(cfg.server.token(), None);
        cfg.server.access_token = Some("s3cret".into());
        assert_eq!(cfg.server.token(), Some("s3cret"));
    }

    #[test]
    fn presented_tokens_covers_the_three_headers() {
        let req = request(
            "POST",
            "/v1/messages",
            &[("authorization", "Bearer t1"), ("x-goog-api-key", "t2")],
        );
        assert_eq!(
            presented_tokens(&req),
            vec!["t1".to_string(), "t2".to_string()]
        );
        // x-api-key 不带 Bearer 前缀。
        let req = request("POST", "/v1/messages", &[("x-api-key", "t3")]);
        assert_eq!(presented_tokens(&req), vec!["t3".to_string()]);
    }

    #[test]
    fn query_token_only_counts_on_its_own_routes() {
        // 控制台：认。
        let req = request("GET", "/console/1?access_token=t1", &[]);
        assert_eq!(presented_tokens(&req), vec!["t1".to_string()]);
        // 其它路由：不认（否则任何一处 URL 转发都会把令牌洗进日志）。
        let req = request("GET", "/status?access_token=t1", &[]);
        assert!(presented_tokens(&req).is_empty());
        // Gemini 原生入口的 ?key= 是该协议自己的凭据位置，认。
        let req = request("POST", "/v1beta/models/m:generateContent?key=k1", &[]);
        assert_eq!(presented_tokens(&req), vec!["k1".to_string()]);
    }

    #[test]
    fn static_console_assets_are_exempt_without_token() {
        // 通道 7：跨站 iframe 的 module 图取源（无 Cookie/Referer/头）——惰性静态件放行。
        let req = request("GET", "/console/23/assets/react-WmG6g5jw.js", &[]);
        assert!(is_static_console_asset(
            &req,
            "/console/23/assets/react-WmG6g5jw.js"
        ));
        let req = request("GET", "/console/23/assets/x.woff2", &[]);
        assert!(is_static_console_asset(&req, "/console/23/assets/x.woff2"));
        // 数据面不豁免：API、JSON、根文档、非 GET。
        let req = request("GET", "/console/23/api/status", &[]);
        assert!(!is_static_console_asset(&req, "/console/23/api/status"));
        let req = request("GET", "/console/23/config.json", &[]);
        assert!(!is_static_console_asset(&req, "/console/23/config.json"));
        let req = request("GET", "/console/23", &[]);
        assert!(!is_static_console_asset(&req, "/console/23"));
        let req = request("POST", "/console/23/assets/x.js", &[]);
        assert!(!is_static_console_asset(&req, "/console/23/assets/x.js"));
        // 非控制台路由永不豁免。
        let req = request("GET", "/v1/assets/x.js", &[]);
        assert!(!is_static_console_asset(&req, "/v1/assets/x.js"));
    }

    #[test]
    fn console_subresources_may_authenticate_via_referer() {
        // 同源 Referer：iframe 内的子资源请求（文档 URL 带令牌）。真实请求里
        // HTTP/1.1 必带 Host、HTTP/2 走 :authority，这里按前者摆。
        let req = request(
            "GET",
            "/console/1/assets/app.js",
            &[
                ("referer", "http://127.0.0.1:8787/console/1?access_token=t1"),
                ("host", "127.0.0.1:8787"),
            ],
        );
        assert_eq!(presented_tokens(&req), vec!["t1".to_string()]);
        // 跨源 Referer：不认。
        let req = request(
            "GET",
            "/console/1/assets/app.js",
            &[
                ("referer", "http://evil.example/console/1?access_token=t1"),
                ("host", "127.0.0.1:8787"),
            ],
        );
        assert!(presented_tokens(&req).is_empty());
    }

    #[test]
    fn console_cookie_channel() {
        let req = request(
            "GET",
            "/console/1/assets/app.js",
            &[("cookie", "foo=1; llm_switch_console=t1; bar=2")],
        );
        assert_eq!(presented_tokens(&req), vec!["t1".to_string()]);
    }

    #[test]
    fn console_document_detection() {
        assert!(is_console_document("/console/1"));
        assert!(!is_console_document("/console/1/"));
        assert!(!is_console_document("/console/1/assets/app.js"));
        assert!(!is_console_document("/console"));
        assert!(!is_console_document("/status"));
    }

    #[test]
    fn credential_query_params_are_stripped_before_forwarding() {
        // 客户端带来的 key 被剥掉，正常参数按原序保留。
        assert_eq!(
            sanitize_forwarded_query(Some("alt=sse&key=SECRET&prettyPrint=false")),
            "?alt=sse&prettyPrint=false"
        );
        // 全是凭据 → 空串（不带问号）。
        assert_eq!(sanitize_forwarded_query(Some("key=SECRET")), "");
        assert_eq!(sanitize_forwarded_query(None), "");
        assert_eq!(sanitize_forwarded_query(Some("")), "");
        // 大小写与百分号编码都拦得住（%6bey 解码回来就是 key）。
        assert_eq!(sanitize_forwarded_query(Some("KeY=1&%6bey=2")), "");
    }

    #[test]
    fn listen_policy_requires_token_off_loopback() {
        let mut cfg = cfg_with_token(None);
        cfg.server.host = "127.0.0.1".into();
        assert!(cfg.validate().is_ok());

        cfg.server.host = "0.0.0.0".into();
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains("access_token"), "报错要指明缺什么: {err}");

        cfg.server.access_token = Some("s3cret".into());
        assert!(cfg.validate().is_ok());
    }
}
