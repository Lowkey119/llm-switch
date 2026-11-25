//! M7 · 后端控制台反向代理：`/console/:id/*path` → 该后端自己的 WebUI。
//!
//! 安全模型（SSRF 防线）：代理目标永远取自 `backends` 表内的
//! `webui_url`（缺省回落 `base_url`），请求方只能贡献 path/query，
//! 无法指向我方未登记的任何主机——但「登记」本身是可写配置，所以整条
//! 通道受 `gateway::auth` 的访问令牌保护（令牌未设 = 全网关开放回环）。
//!
//! 免登录：按配置注入凭据——`webui_user`+`webui_password` → Basic；
//! 仅 `webui_password` → Bearer；两者皆空 → 回落 `api_key` Bearer。
//! **`api_key` 的回落只发往与 `base_url` 同主机的入口**：那是这个 key 的
//! 属主机器；换到别的主机就只代理、不带凭据（显式填的账户/密码仍照注入，
//! 那是用户明确指向那个地址的）。
//! 内嵌适配：剥 `X-Frame-Options` 与 CSP `frame-ancestors`，按哈希放行注入的
//! 补丁脚本（`csp_allow_patch`，上游 `script-src 'self'` 会拦内联 `net_patch`），
//! HTML 注入 `<base href>`，`Location`/`Set-Cookie` 的 path 前缀化。
//!
//! **子资源自带令牌**：文档请求带 `?access_token=` 即可过鉴权，但 CSS/JS/字体
//! 是浏览器按文档里的 URL 自己去取的——WKWebView 里 iframe 与 UI 跨站，第三方
//! Cookie 被拦、Referer 也不发，**URL 上的 query 是唯一确定性通道**。所以 HTML
//! 的 `src`/`href` 与 CSS 的 `url()`/`@import` 一律改写成 `/console/:id/…?access_token=`
//! （见 `rewrite_html_assets` / `rewrite_css`）；漏一条就是一条 401，整页退化成
//! 无样式无脚本的裸 HTML（2026-10-21 oMLX 面板现场）。

use crate::gateway::State;
use crate::types::BackendRow;
use axum::body::{to_bytes, Body};
use axum::extract::{Path, State as AxumState};
use axum::http::{header, HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use reqwest::Url;
use std::sync::OnceLock;
use std::time::Duration;

/// 控制台流量小、多为交互式请求：不跟随重定向（交浏览器带着 base 走）。
///
/// 超时只设**连接**与**读空闲**两档：前者挡住打不通的主机把请求吊死，后者
/// 挡住上游假死时请求与连接永不释放（老实现一档都不设 = 一个挂死的 WebUI
/// 能一直占着 socket）。总时长故意不设——控制台有 SSE 与长轮询，掐总时长
/// 会把正常的长连接掐断；流本身的空闲已经由 read_timeout 兜住。
fn client() -> &'static reqwest::Client {
    static C: OnceLock<reqwest::Client> = OnceLock::new();
    C.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client")
    })
}

/// 请求体上限（控制台上传极少超过此量）。
const MAX_BODY: usize = 32 * 1024 * 1024;
/// HTML 改写缓冲上限：超过则放弃 `<base>` 注入直接流式透传。
const MAX_HTML_REWRITE: usize = 2 * 1024 * 1024;
/// CSS 改写缓冲上限：超过则放弃 `url()` 注入直接流式透传。
const MAX_CSS_REWRITE: usize = 4 * 1024 * 1024;

pub async fn root(
    st: AxumState<std::sync::Arc<State>>,
    Path(id): Path<i64>,
    req: Request<Body>,
) -> Response {
    proxy(st.0.clone(), id, "", req).await
}

pub async fn proxied(
    st: AxumState<std::sync::Arc<State>>,
    Path((id, path)): Path<(i64, String)>,
    req: Request<Body>,
) -> Response {
    proxy(st.0.clone(), id, &path, req).await
}

async fn proxy(st: std::sync::Arc<State>, id: i64, path: &str, req: Request<Body>) -> Response {
    let backend = match st.store.backend_by_id(id) {
        Ok(Some(b)) => b,
        Ok(None) => return (StatusCode::NOT_FOUND, "未知后端").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let base = match console_base(&backend) {
        Ok(u) => u,
        // 地址本身填错了：同样给说明页，别在 iframe 里甩一行裸错误串。
        Err(msg) => return hint_page(&backend, None, Some(msg)),
    };
    if path.split('/').any(|seg| seg == "." || seg == "..") {
        return (StatusCode::BAD_REQUEST, "非法路径").into_response();
    }
    // Unsloth Studio 的启动门读 localStorage 令牌、router 也不认识代理前缀，
    // 需要注入专属 shim（见 STUDIO_SHIM）。分叉轴是引擎，不是操作系统。
    let studio = backend.engine.as_deref() == Some("unsloth");

    let (parts, body) = req.into_parts();
    // 剥掉凭据味 query（尤其网关自己的 `?access_token=`，iframe 入口靠它过 auth，
    // 但那是找我们的凭据、不是给上游的）——主路径同款 sanitize，别把令牌外泄给上游。
    let query = crate::gateway::auth::sanitize_forwarded_query(parts.uri.query())
        .strip_prefix('?')
        .map(str::to_string);
    let body_bytes = match to_bytes(body, MAX_BODY).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("读取请求体失败: {e}")).into_response(),
    };

    // 目标 URL：见 target_path（根绝对与文档相对两条路，别一律拼在 base 后面）。
    let mut url = base.clone();
    {
        let mut pw = url.path_segments_mut().expect("valid url");
        pw.clear();
        for s in target_path(&base, path)
            .split('/')
            .filter(|s| !s.is_empty())
        {
            pw.push(s);
        }
    }
    url.set_query(query.as_deref());

    let mut builder = client()
        .request(parts.method.clone(), url.clone())
        .headers(forward_request_headers(&parts.headers, &backend, &base));
    if !body_bytes.is_empty() {
        builder = builder.body(body_bytes);
    }
    let upstream = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            // 日志不记 query：WebUI 常把会话令牌挂在 URL 上。
            tracing::warn!(target: "llm_switch", "console 反代失败 id={id} url={}: {e}", redact_url(&url));
            return hint_page(&backend, Some(redact_url(&url)), Some(e.to_string()));
        }
    };

    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
    // 根文档就 404/405/501：这家多半压根没开 WebUI（或入口路径不对）。
    // 让它自己的错误页渲染进 iframe 只会让人以为是网关坏了。
    if path.is_empty()
        && matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::NOT_IMPLEMENTED
        )
    {
        return hint_page(
            &backend,
            Some(redact_url(&url)),
            Some(format!("上游返回 {status}")),
        );
    }
    let ctype = upstream
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let encoded = upstream.headers().contains_key(header::CONTENT_ENCODING);
    // 令牌先取：CSP 放行要用它算 `net_patch` 的哈希（见 transform_response_headers）。
    let token = st.config.server.token().unwrap_or("");
    let mut out_headers = transform_response_headers(upstream.headers(), &base, id, token, studio);

    // HTML 文档缓冲改写注入 <base>；CSS 缓冲改写 url()/@import（字体图片由浏览器
    // 按 CSS 里的地址去取，同样得带令牌）；其余（含压缩响应）原样流式透传。
    if ctype.contains("text/html") && !encoded {
        match upstream.bytes().await {
            Ok(bytes) => {
                // 超限的 HTML 原样返回（只是不注 base），不再回流——已消费无损。
                let body = if bytes.len() < MAX_HTML_REWRITE {
                    inject_base(&bytes, id, &base, token, studio)
                } else {
                    bytes.to_vec()
                };
                if let Ok(v) = HeaderValue::from_str(&body.len().to_string()) {
                    out_headers.insert(header::CONTENT_LENGTH, v);
                }
                return Response::new(Body::from(body)).into_partial(status, out_headers);
            }
            Err(e) => {
                return hint_page(
                    &backend,
                    Some(redact_url(&url)),
                    Some(format!("读取上游失败：{e}")),
                )
            }
        }
    }
    if ctype.contains("text/css") && !encoded {
        match upstream.bytes().await {
            Ok(bytes) => {
                let body = if bytes.len() < MAX_CSS_REWRITE {
                    match std::str::from_utf8(&bytes) {
                        // `url` = 这份 CSS 自己的上游地址，`url()` 的相对引用按它解析。
                        Ok(text) => rewrite_css(text, &url, id, token).into_bytes(),
                        Err(_) => bytes.to_vec(),
                    }
                } else {
                    bytes.to_vec()
                };
                if let Ok(v) = HeaderValue::from_str(&body.len().to_string()) {
                    out_headers.insert(header::CONTENT_LENGTH, v);
                }
                return Response::new(Body::from(body)).into_partial(status, out_headers);
            }
            Err(e) => {
                return hint_page(
                    &backend,
                    Some(redact_url(&url)),
                    Some(format!("读取上游失败：{e}")),
                )
            }
        }
    }
    // Studio 的 `/api/auth/status` 特判：上游 `requires_password_change:true` 会把
    // 前端强制踢到 /change-password（改真实密码越权、不可取），代理把该字段改写为
    // false。只动这一个 JSON 字段，解析失败原样透传，不猜别的。
    if studio
        && path.trim_end_matches('/') == "api/auth/status"
        && ctype.contains("application/json")
        && !encoded
    {
        match upstream.bytes().await {
            Ok(bytes) => {
                let body = patch_studio_auth_status(&bytes);
                if let Ok(v) = HeaderValue::from_str(&body.len().to_string()) {
                    out_headers.insert(header::CONTENT_LENGTH, v);
                }
                return Response::new(Body::from(body)).into_partial(status, out_headers);
            }
            Err(e) => {
                return hint_page(
                    &backend,
                    Some(redact_url(&url)),
                    Some(format!("读取上游失败：{e}")),
                )
            }
        }
    }
    Response::new(Body::from_stream(upstream.bytes_stream())).into_partial(status, out_headers)
}

trait IntoPartial {
    fn into_partial(self, status: StatusCode, headers: HeaderMap) -> Response;
}
impl IntoPartial for Response {
    fn into_partial(mut self, status: StatusCode, headers: HeaderMap) -> Response {
        *self.status_mut() = status;
        *self.headers_mut() = headers;
        self
    }
}

/// webui_url 的**文档目录**：`/admin/dashboard` → `/admin/`，`/grafana/` → `/grafana/`，根 → `/`。
fn doc_dir(base: &Url) -> String {
    let p = base.path();
    match p.rfind('/') {
        Some(i) if i + 1 >= p.len() => p.to_string(),
        Some(i) => p[..=i].to_string(),
        None => "/".to_string(),
    }
}

/// 子资源的目标路径。
///
/// 为什么不能一律拼在 base 后面：`webui_url` 常常指向一个**深层页面**
/// （oMLX 的 `http://h:8000/admin/dashboard`），而 HTML 里的根绝对路径
/// `/admin/static/app.css` 是 **origin 根**语义。一律拼接会得到
/// `/admin/dashboard/admin/static/app.css` → CSS/JS 全 404 → iframe 里就是一张
/// 没样式的裸 HTML（2026-10-18 现场）。规则：请求路径已经带上文档目录那段
/// → 按 origin 根解析（根绝对）；否则拼在文档目录之后（文档相对，由 `<base>` 带来）。
fn target_path(base: &Url, path: &str) -> String {
    if path.is_empty() {
        return base.path().to_string(); // 根文档就是 webui_url 本身
    }
    let dir = doc_dir(base);
    let req = format!("/{}", path.trim_start_matches('/'));
    if dir != "/" && req.starts_with(&dir) {
        req
    } else {
        format!("{dir}{}", req.trim_start_matches('/'))
    }
}

/// 控制台入口 URL：webui_url 优先，回落 base_url；仅允许 http/https。
fn console_base(b: &BackendRow) -> Result<Url, String> {
    let raw = b
        .webui_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&b.base_url);
    let u = Url::parse(raw).map_err(|e| format!("控制台入口地址无效: {e}"))?;
    if !matches!(u.scheme(), "http" | "https") {
        return Err("控制台入口仅支持 http/https".into());
    }
    Ok(u)
}

/// 免登录凭据注入规则（均可空）：user+pass → Basic；仅 pass → Bearer；否则 api_key。
/// `api_key` 的回落只发给与 `base_url` 同主机的入口（文件头：那是 key 的属主机器）。
fn console_auth(b: &BackendRow, entry: &Url) -> Option<HeaderValue> {
    let user = b
        .webui_user
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let pass = b.webui_password.as_deref().filter(|s| !s.trim().is_empty());
    let value = match (user, pass) {
        (Some(u), Some(p)) => format!("Basic {}", STANDARD.encode(format!("{u}:{p}"))),
        (None, Some(p)) => format!("Bearer {p}"),
        _ => b
            .api_key
            .as_deref()
            .filter(|k| !k.trim().is_empty())
            .filter(|_| same_host_as_base(b, entry))
            .map(|k| format!("Bearer {k}"))?,
    };
    HeaderValue::from_str(&value).ok()
}

/// 控制台入口与 base_url 是否同主机。端口与协议不管——vLLM 在 :8000、它的
/// WebUI 在 :3000 是常态；换到别的主机则这个 api_key 不属于那里，不发。
fn same_host_as_base(b: &BackendRow, entry: &Url) -> bool {
    let Ok(base) = Url::parse(b.base_url.trim()) else {
        return false; // base_url 解析不了就保守处理：不外带凭据
    };
    match (entry.host_str(), base.host_str()) {
        (Some(a), Some(b_)) => a.eq_ignore_ascii_case(b_),
        _ => false,
    }
}

/// 日志用的 URL：只留 origin + path——WebUI 常把会话令牌挂在 query 上。
fn redact_url(u: &Url) -> String {
    let Some(host) = u.host_str() else {
        return "<无主机>".into();
    };
    let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
    format!("{}://{}{}{}", u.scheme(), host, port, u.path())
}

/// HTML 文本转义：入口地址与错误串都是用户可控内容，不能直接拼进页面。
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 反代打不通时的说明页。
///
/// 为什么要有这一页：iframe 里甩一行 `上游不可达: error sending request for url (...)`
/// 对用户毫无指导性，看着像网关坏了。真正要回答的是两件事——入口配置对不对，
/// 以及这家后端到底有没有 WebUI；后者没有时请用户回后端原生控制台。
fn hint_page(backend: &BackendRow, target: Option<String>, detail: Option<String>) -> Response {
    let entry = backend
        .webui_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&backend.base_url)
        .trim_end_matches('/');
    let fallback = backend.webui_url.as_deref().map(str::trim) != Some(entry);
    // 截断按字符不按字节：错误串里可能有中文，按字节切会 panic 在热路径上。
    let detail: String = detail.unwrap_or_default().chars().take(240).collect();
    let html = format!(
        r#"<!DOCTYPE html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{name} · 控制台打不开</title><style>
html{{color-scheme:dark}}body{{margin:0;padding:clamp(24px,4vw,56px) clamp(20px,4vw,64px);
background:#0f172a;color:#e2e8f0;min-height:100vh;box-sizing:border-box;
font:clamp(15px,1.15vw,17px)/1.75 -apple-system,BlinkMacSystemFont,"PingFang SC",sans-serif}}
/* 说明页原先锁在 640px 居中一小块、正文 13px，周围大片空白还看不清（用户 2026-11-16 截图）。
   改成随容器铺满：宽度上限放宽到 1080px 并按视口缩放，字号整体抬到可读区间。 */
.wrap{{max-width:min(1080px,100%);margin:0 auto}}
h1{{font-size:clamp(20px,2vw,26px);margin:0 0 16px;line-height:1.35}}
.box{{background:#1e293b;border:1px solid #334155;border-radius:10px;padding:clamp(16px,2vw,24px);margin:16px 0}}
.box b{{font-size:1.05em}}
code{{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:0.95em;color:#7dd3fc;word-break:break-all}}
ol{{margin:10px 0 0;padding-left:24px}}li{{margin:10px 0}}
.dim{{color:#94a3b8}}.err{{color:#fca5a5;font-size:0.95em;word-break:break-all}}
a{{color:#38bdf8}}button{{margin-top:14px;padding:9px 20px;border-radius:8px;border:1px solid #475569;
background:#334155;color:#e2e8f0;font-size:1em;cursor:pointer}}
button:hover{{border-color:#38bdf8}}
</style></head><body><div class="wrap">
<h1>打不开 <span class="dim">{name}</span> 的控制台</h1>
<div class="box"><div class="dim">目标地址</div><div><code>{target}</code></div>
{entry_line}<div class="dim" style="margin-top:8px">错误</div><div class="err">{detail}</div></div>
<div class="box"><b>请依次确认</b><ol>
<li>后端是否已启动、控制台端口是否开着；</li>
<li>llm-switch 里该后端的<b>入口配置</b>（控制台地址）是否正确——留空时回落后端 base_url，
很多引擎的 WebUI 端口与 API 端口并不相同；</li>
<li>该后端是否提供 WebUI 控制台。<b>不提供属正常</b>：请改用其后端原生控制台
（浏览器直接打开 <a href="{entry}" target="_blank" rel="noreferrer">{entry}</a> 或引擎自带命令行）。</li>
</ol>
<button onclick="location.reload()">重新加载</button></div>
</div></body></html>"#,
        name = html_escape(&backend.name),
        target = html_escape(&target.unwrap_or_else(|| entry.to_string())),
        detail = html_escape(&detail),
        entry = html_escape(entry),
        entry_line = if fallback {
            format!(
                r#"<div class="dim" style="margin-top:6px">未单独配置控制台地址，当前回落 API 地址 <code>{}</code></div>"#,
                html_escape(&backend.base_url)
            )
        } else {
            String::new()
        },
    );
    let mut resp = Response::new(Body::from(html));
    *resp.status_mut() = StatusCode::BAD_GATEWAY;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// 复制请求头并剔除逐跳头；host 交给 reqwest，authorization 由我们注入/保留。
fn forward_request_headers(src: &HeaderMap, backend: &BackendRow, entry: &Url) -> HeaderMap {
    const SKIP: &[&str] = &[
        "connection",
        "keep-alive",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "host",
        "content-length",
    ];
    let mut out = HeaderMap::new();
    for (k, v) in src {
        if SKIP.contains(&k.as_str()) {
            continue;
        }
        out.append(k.clone(), v.clone());
    }
    if let Some(auth) = console_auth(backend, entry) {
        out.insert(header::AUTHORIZATION, auth);
    }
    out
}

/// 响应头加工：剥内嵌拦截头、放行注入的补丁脚本、重写 Location、Set-Cookie path 前缀化。
fn transform_response_headers(
    src: &HeaderMap,
    base: &Url,
    id: i64,
    token: &str,
    studio: bool,
) -> HeaderMap {
    const DROP: &[&str] = &["x-frame-options", "content-security-policy"];
    let prefix = format!("/console/{id}");
    // 补丁正文的哈希：`inject_base` 把同一份正文写进文档，CSP 只认这一段。
    let patch_hash = net_patch_hash(id, base, token, studio);
    let mut out = HeaderMap::new();
    for (k, v) in src {
        match k.as_str() {
            "x-frame-options" => continue,
            "content-security-policy" => {
                // 只摘 frame-ancestors（内嵌必须），其余 CSP 指令保留（上游自护的
                // XSS 防线不废）；另按哈希放行我们注入的 `net_patch`——不加
                // 'unsafe-inline'，见 csp_allow_patch。
                let kept: Vec<&str> = v
                    .to_str()
                    .map(|s| {
                        s.split(';')
                            .filter(|d| !d.trim_start().starts_with("frame-ancestors"))
                            .collect()
                    })
                    .unwrap_or_default();
                let joined = kept.join(";").trim().to_string();
                if !joined.is_empty() {
                    let relaxed = csp_allow_patch(&joined, &patch_hash);
                    if let Ok(hv) = HeaderValue::from_str(&relaxed) {
                        out.append(k.clone(), hv);
                    }
                }
            }
            "content-length" => continue, // 长度可能因改写变化，交由传输层重算
            "location" => {
                let loc = v.to_str().unwrap_or("").to_string();
                let rewritten = rewrite_location(&loc, base, &prefix).unwrap_or(loc);
                if let Ok(hv) = HeaderValue::from_str(&rewritten) {
                    out.append(k.clone(), hv);
                }
            }
            "set-cookie" => {
                let rewritten = rewrite_cookie_path(v.to_str().unwrap_or(""), &prefix);
                if let Ok(hv) = HeaderValue::from_str(&rewritten) {
                    out.append(k.clone(), hv);
                }
            }
            _ => {
                if !DROP.contains(&k.as_str()) {
                    out.append(k.clone(), v.clone());
                }
            }
        }
    }
    // 内嵌通道永不落缓存：WebKit NetworkCache 对无校验器的响应会长期持有旧变体，
    // 曾存下「只有 <base>、缺运行时补丁」的过渡版本 → 子资源不带令牌、数据静止的
    // 症状全从这来（2026-10-14 排查实锤），一刀 no-store 让每次加载都拿新注入的。
    out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    out
}

/// 把注入的 `net_patch` 补丁正文的哈希追加进上游 CSP，只放行这一段内联脚本。
///
/// **为什么是哈希而不是 `'unsafe-inline'`**：上游明确只信自己域名的脚本
/// （Unsloth 实测 `script-src 'self'`），整条放开等于替上游拆掉它的 XSS 防线；
/// 哈希只多认**这一段我们自己生成的**补丁正文，其余源与指令一个字节不动。
///
/// **加在哪条指令**：内联 `<script>` 按 CSP 规范先看 `script-src-elem`，没有再看
/// `script-src`，两条都缺才落到 `default-src`——所以只给实际管脚本的那几条加，
/// `default-src` 仅在压根没有 script 指令时才动（此时它才是管脚本的那条）。
/// 两条都不限脚本时原样返回。幂等：同款哈希已在指令里就不重复加。
fn csp_allow_patch(csp: &str, hash_b64: &str) -> String {
    const SCRIPT_DIRS: &[&str] = &["script-src", "script-src-elem"];
    let digest = format!("'sha256-{hash_b64}'");
    let has_script_dir = csp
        .split(';')
        .any(|d| SCRIPT_DIRS.contains(&directive_name(d).as_str()));
    csp.split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| {
            let name = directive_name(d);
            let target =
                SCRIPT_DIRS.contains(&name.as_str()) || (!has_script_dir && name == "default-src");
            if target && !d.contains(&digest) {
                format!("{d} {digest}")
            } else {
                d.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// 指令名（`;` 前的第一个词，小写）——CSP 指令名大小写不敏感。
fn directive_name(d: &str) -> String {
    d.split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// 上游域内绝对重定向 → 代理前缀路径；其余保持原样。
fn rewrite_location(loc: &str, base: &Url, prefix: &str) -> Option<String> {
    let abs = Url::parse(loc).ok()?;
    if abs.origin() != base.origin() {
        return None;
    }
    let base_path = base.path().trim_end_matches('/').to_string();
    let rest = abs.path().strip_prefix(&base_path).unwrap_or(abs.path());
    let mut out = format!("{prefix}{}", rest);
    if let Some(q) = abs.query() {
        out.push('?');
        out.push_str(q);
    }
    Some(out)
}

/// Set-Cookie 的 Path 属性挂到代理前缀下，保证会话 Cookie 能回到本通道。
fn rewrite_cookie_path(cookie: &str, prefix: &str) -> String {
    let mut parts: Vec<String> = cookie.split(';').map(|s| s.trim().to_string()).collect();
    let mut seen = false;
    for p in parts.iter_mut().skip(1) {
        if p.split('=').next().map(|k| k.eq_ignore_ascii_case("path")) == Some(true) {
            let val = p.split('=').nth(1).unwrap_or("/");
            let normalized = if val.starts_with('/') {
                format!("{prefix}{val}")
            } else {
                format!("{prefix}/{val}")
            };
            *p = format!("Path={normalized}");
            seen = true;
        }
    }
    if !seen {
        parts.push(format!("Path={prefix}"));
    }
    parts.join("; ")
}

/// 把 HTML 中的资源地址（`src`/`href`/`form action`）改写成走网关控制台代理的 URL：
/// **根绝对与文档相对两种路径都改**，解析成上游真实地址后映射为
/// `/console/:id/<path>` 并补上 `?access_token=`。
///
/// 为什么相对路径也要改（旧实现只改根绝对、相对交给 `<base>`）：`<base>` 只管
/// URL 形状，管不了凭据——iframe 里的子资源在 WKWebView（UI 与网关跨站）既带不出
/// 第三方 Cookie、也不带 Referer，**query 上的令牌是唯一确定性通道**
/// （见 `gateway/auth.rs` 的六条通道与 `net_patch` 的同款判断）。少补一个令牌
/// 就是一条 401，而 401 的响应体是合法 JSON：页面照常解析、只是没样式没脚本，
/// 2026-10-21 oMLX 面板「只剩一张裸 HTML」正是这么来的。
///
/// 不碰的形态：纯锚点（`#top`）、协议相对（`//host`）、外域完整 URL
/// （`https://cdn…`，含 `data:` 等非 http scheme）、已带本 id 前缀的（幂等）。
/// 只认带引号的属性值（资源路径几乎总带引号），逐字节扫描、不引整页解析依赖。
/// **已知缺口**：`srcset` / 内联 `style="url(…)"` 不在改写范围（遇到再说，眼下
/// 几家引擎的面板都没用到；真用到时页面表现是那一条资源 401，不会整页失效）。
fn rewrite_html_assets(html: &[u8], id: i64, base: &Url, token: &str) -> Vec<u8> {
    let bytes = html;
    let mut out = Vec::with_capacity(html.len());
    let mut i = 0;

    while i < bytes.len() {
        // 匹配属性名 src / href / action（大小写不敏感，名字本身由 is_attr_name 核）。
        if let Some(attr_len) = attr_name_len(bytes, i) {
            // 跳过属性名后的空白，找 `=`。
            let mut p = i + attr_len;
            while p < bytes.len() && is_ws(bytes[p]) {
                p += 1;
            }
            if p < bytes.len() && bytes[p] == b'=' {
                p += 1;
                while p < bytes.len() && is_ws(bytes[p]) {
                    p += 1;
                }
                if p < bytes.len() {
                    let q = bytes[p];
                    if q == b'"' || q == b'\'' {
                        // p 指向开引号：值 = [p+1, 关引号)，关引号绝对位置 = p+1+end。
                        if let Some(end) = bytes[p + 1..].iter().position(|&c| c == q) {
                            let val = &bytes[p + 1..p + 1 + end];
                            if let Some(new) = std::str::from_utf8(val)
                                .ok()
                                .and_then(|v| proxy_url(base, id, token, v))
                            {
                                out.extend_from_slice(&bytes[i..p + 1]);
                                out.extend_from_slice(new.as_bytes());
                                i = p + end + 1; // 停在关引号上，下一轮原样复制它
                                continue;
                            }
                        }
                    }
                }
            }
        }

        // 未命中：原样复制一个字节，继续扫描。
        out.push(bytes[i]);
        i += 1;
    }

    out
}

/// 一个资源地址 → 代理 URL（`/console/:id/<path>[?query]`，必要时补访问令牌）。
///
/// 相对路径按**文档地址**解析（`webui_url` 就是这份 HTML 的 URL），绝对地址
/// 必须与文档同源；外域资源（CDN）与各种非 http scheme 一律 `None` = 原样保留。
fn proxy_url(base: &Url, id: i64, token: &str, raw: &str) -> Option<String> {
    let v = raw.trim();
    if v.is_empty() || v.starts_with('#') || v.starts_with("//") {
        return None;
    }
    if v.starts_with("/console/") {
        return None; // 已带本通道前缀：幂等，别二次注入
    }
    let resolved = base.join(v).ok()?;
    if resolved.origin() != base.origin() {
        return None; // 外域（含 data:/mailto: 等 scheme）一个字节都不改
    }
    let mut out = format!("/console/{id}{}", resolved.path());
    if let Some(q) = resolved.query() {
        out.push('?');
        out.push_str(q);
    }
    Some(with_access_token(&out, token))
}

/// 给代理 URL 追加 `access_token`（幂等；令牌按 form-urlencoded 转义）。
fn with_access_token(path: &str, token: &str) -> String {
    if token.is_empty() || path.contains("access_token=") {
        return path.to_string();
    }
    let enc: String = url::form_urlencoded::byte_serialize(token.as_bytes()).collect();
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}access_token={enc}")
}

/// 在 `hay` 里从 `from` 起找子串（ASCII 大小写不敏感；CSS 关键字不区分大小写）。
fn find_ci(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() || from + needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len())
        .find(|&i| hay[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

/// CSS 里的 `url(…)` 与 `@import "…"` 改写为代理地址。
///
/// 为什么必须做：字体、背景图是**浏览器解析 CSS 时**自己去取的，页面侧的
/// `net_patch`（覆写 fetch/XHR/属性 setter）一条都管不着——CSS 里的相对地址
/// 经 `<base>` 解析后落在 `/console/:id/…`，没令牌照样 401，整页掉回系统字体。
/// 不碰的形态与 HTML 同口径：`data:`、纯锚点（SVG 引用 `url(#glow)`）、外域。
fn rewrite_css(css: &str, doc: &Url, id: i64, token: &str) -> String {
    let b = css.as_bytes();
    let mut out = String::with_capacity(css.len() + 256);
    let mut i = 0;
    while i < b.len() {
        let at_url = find_ci(b, i, b"url(");
        let at_imp = find_ci(b, i, b"@import");
        let (is_import, at) = match (at_url, at_imp) {
            (None, None) => {
                out.push_str(&css[i..]);
                return out;
            }
            (Some(u), Some(m)) if u < m => (false, u),
            (Some(_), Some(m)) => (true, m),
            (Some(u), None) => (false, u),
            (None, Some(m)) => (true, m),
        };
        out.push_str(&css[i..at]);

        if !is_import {
            // url( <value> )：值可带引号。引号内含 `)`（个别 data: URI）时按第一个
            // `)` 断开会取到半截值——那样的值解析不出同源地址，走「原样保留」分支，
            // 字节一个不掉（下面 None 分支复制的就是原文区间）。
            let s = at + 4;
            let Some(e) = b[s..].iter().position(|&c| c == b')') else {
                out.push_str(&css[at..]);
                return out;
            };
            let inner = css[s..s + e].trim();
            let value = inner.trim_matches(|c| c == '"' || c == '\'');
            match proxy_url(doc, id, token, value) {
                Some(new) => out.push_str(&format!("url(\"{new}\")")),
                None => out.push_str(&css[at..s + e + 1]),
            }
            i = s + e + 1;
        } else {
            // @import "x.css" / @import 'x.css'（不写 url() 的形态；写了的由上一支处理）。
            let s = at + 7;
            let rest = css[s..].trim_start();
            let skipped = css[s..].len() - rest.len();
            let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                out.push_str(&css[at..s]); // 关键字原样，指针推进到关键字之后
                i = s;
                continue;
            };
            let body = &rest[1..];
            let Some(e) = body.find(q) else {
                out.push_str(&css[at..s]);
                i = s;
                continue;
            };
            match proxy_url(doc, id, token, &body[..e]) {
                Some(new) => out.push_str(&format!("@import \"{new}\"")),
                None => out.push_str(&css[at..s + skipped + 1 + e + 1]),
            }
            i = s + skipped + 1 + e + 1;
        }
    }
    out
}

/// 位置 i 处是否是要改写的资源属性（`src` / `href` / `action`），返回属性名字节长度。
/// `formaction` 之类更长的名字会被 `is_attr_name` 的前边界挡掉（前一个字符不是空白）。
fn attr_name_len(bytes: &[u8], i: usize) -> Option<usize> {
    for (name, len) in [
        (b"src".as_slice(), 3usize),
        (b"href".as_slice(), 4),
        (b"action".as_slice(), 6),
    ] {
        if is_attr_name(bytes, i, name) {
            return Some(len);
        }
    }
    None
}

/// 位置 i 处是否出现属性名 `name`（前后为属性边界，避免匹配到 `data-src`、`sources` 等）。
fn is_attr_name(bytes: &[u8], i: usize, name: &[u8]) -> bool {
    if i + name.len() > bytes.len() || !bytes[i..i + name.len()].eq_ignore_ascii_case(name) {
        return false;
    }
    // 前边界：属性名之前必须是标签/空白，避免匹配 `data-src`。
    if i > 0 {
        let prev = bytes[i - 1];
        if !is_ws(prev) && prev != b'<' && prev != b'>' && prev != b'/' {
            return false;
        }
    }
    // 后边界：属性名之后必须是空白/`=`/标签结束，避免匹配 `sources`、`srcfoo`。
    matches!(
        bytes.get(i + name.len()),
        None | Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'=')
    )
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// 在 `<head...>` 之后注入两件套：`<base href="/console/:id/">` 与**运行时路径补丁**
/// （`net_patch`）；若文档无 `<head>` 则整体前缀。
///
/// 曾注入整页缩放脚本（FIT_SCRIPT：按「内容自然宽 vs 视口宽」zoom 收缩适配），
/// 已按用户验收口径**移除**：控制台要「内容自适应」，不做合成缩放——面板给多宽
/// 就按多宽渲染，上游 WebUI 自己的响应式断点/滚动接管适配。旧方案实测两处硬伤：
/// ① SPA 未稳定布局时量到病态自然宽（AU401 留痕 z 压到 0.05 下限，整页缩成 5%）；
/// ② 缩放坐标系里的复量自乘（scrollWidth 已含倍率再除一次），收敛后回不到 1:1。
///
/// 也曾按 `?llms_vw=` 静态定宽 viewport meta（错误理论：以为 WebKit 只是把 iframe 的
/// device-width 按顶层窗口解析），同样**移除**——受控实验证明被 zoom 的文档里 iframe
/// 内视口恒按 zoom 后的视觉宽计算、viewport meta 定宽被引擎覆盖（width=706 也变成
/// 706×1.33）。真·根因修复在 UI 侧：iframe 自身 `zoom: 1/倍率` 反缩放（Console.tsx）。
///
/// `token` = 网关访问令牌：一路嵌进补丁给运行时 URL 追加 `?access_token=`
/// （见 `net_patch`），一路由 `rewrite_html_assets` 直接写死在静态资源地址上
/// （子资源拿不到 Cookie/Referer，见文件头）。
fn inject_base(html: &[u8], id: i64, base: &Url, token: &str, studio: bool) -> Vec<u8> {
    // 先改写资源地址（含补令牌），再注入 <base>：避免刚加的 base 标签被本函数再次命中。
    let rewritten = rewrite_html_assets(html, id, base, token);
    let lower = rewritten.to_ascii_lowercase();
    let tag = format!(
        "<base href=\"/console/{id}/\">{}",
        net_patch(id, base, token, studio)
    );
    if let Some(pos) = lower.windows(5).position(|w| w == b"<head") {
        // 找 head 标签结束的 '>'（兼容 `<head lang=x>`）；残缺文档没 '>' 则退回前置。
        if let Some(off) = rewritten[pos + 5..].iter().position(|&c| c == b'>') {
            let insert_at = pos + 5 + off + 1;
            let mut out = Vec::with_capacity(rewritten.len() + tag.len());
            out.extend_from_slice(&rewritten[..insert_at]);
            out.extend_from_slice(tag.as_bytes());
            out.extend_from_slice(&rewritten[insert_at..]);
            return out;
        }
    }
    // 无 `<head>`（或 head 标签残缺）：整体前缀，相对路径仍按 base 解析。
    let mut out = Vec::with_capacity(rewritten.len() + tag.len());
    out.extend_from_slice(tag.as_bytes());
    out.extend_from_slice(&rewritten);
    out
}

/// 运行时路径补丁（内联注入上游 HTML，紧跟 `<base>`）。
///
/// **为什么需要它**：`rewrite_html_assets` 只能改静态 HTML 属性，管不到页面在
/// JS 里发出的 `fetch("/api/…")` 这类运行时请求。iframe 文档源是网关，绝对路径
/// 会打到网关自己的根路由 → 全局鉴权回 401；**而 401 的响应体是合法 JSON**，
/// 页面的 `await fetch().json()` 不抛异常，只是取不到字段、每个值落默认态
/// （「已停止 / 镜像 ？/ 检测失败」），肉眼就像"数据不刷新"。实测见实施日志。
///
/// 补丁在 `<head>` 里、app 脚本之前执行，把下面几条通道同源绝对路径前缀化为
/// `/console/:id/…`：`fetch` / `XMLHttpRequest.open` / `EventSource` /
/// `sendBeacon` / `setAttribute` 与 `src`/`href`/`action` 的属性 setter（运行时
/// 插入的 `<script src="/chunk.js">`）+ MutationObserver 兜住 innerHTML 解析插入
/// 的节点。所有改写幂等（已带 `/console/` 的一律放行）。
///
/// **不覆盖**：`location.href = "/x"` 跳转、`WebSocket`（网关无 WS 透传，M7-C 未做）。
/// 面板硬编码自己地址（`http://<webui_host>/…`）时按 `B`（webui_url 去尾斜杠）剥前缀。
///
/// **改写后与已代理的 URL 一律补 `?access_token=`**（`T` 非空时，`tok` 函数）：
/// 按 `document.baseURI` 解析后只认 `location.origin`（外域不带，令牌不出网关），
/// 覆盖三种形态——相对路径（`<base>` 解析到 /console/，上游改相对路径后的主通道）、
/// 已带前缀的、改写产生的。子资源过 auth 的三条通道里，Cookie 在跨站 iframe 里会被
/// 浏览器拦（SameSite=Lax），Referer 依赖浏览器的 referrer 策略，只有 query 是确定性的
/// ——iframe 入口 URL 本就明文带令牌，补丁沿用同一口径；`sanitize_forwarded_query`
/// 保证该参数不会被转发给上游。
fn net_patch(id: i64, base: &Url, token: &str, studio: bool) -> String {
    format!(
        "<script data-llms-net>{}</script>",
        net_patch_body(id, base, token, studio)
    )
}

/// 补丁**正文**（`<script>` 标签之间的源文本）。CSP 哈希就是对它算的，
/// 且必须与 `inject_base` 实际写进文档的是同一份字符串——差一个字节哈希就
/// 对不上，补丁照旧被 `script-src` 拦掉，症状回到「401 登录墙」。
/// `studio` = 该后端是 Unsloth Studio：正文尾部追加 `STUDIO_SHIM`（同一份正文、
/// 同一个哈希，不另开 script 块）。
fn net_patch_body(id: i64, base: &Url, token: &str, studio: bool) -> String {
    let mut clean = base.clone();
    clean.set_query(None);
    clean.set_fragment(None);
    // URL 里不该有引号/反斜杠，仍剔掉，别让配置把 JS 字符串截断。
    let b = clean
        .as_str()
        .trim_end_matches('/')
        .replace(['"', '\\'], "");
    let mut body = NET_PATCH_BODY
        .replace("__P__", &format!("/console/{id}/"))
        .replace("__B__", &b)
        .replace("__T__", &js_escape(token));
    if studio {
        body.push('\n');
        body.push_str(&STUDIO_SHIM.replace("__BP__", &format!("/console/{id}")));
    }
    body
}

/// 补丁正文的 CSP 哈希 `base64(SHA-256(正文))`。浏览器按脚本元素标签之间的
/// 源文本算（UTF-8 字节，含中文注释），已实测 Chrome 同口径匹配。
fn net_patch_hash(id: i64, base: &Url, token: &str, studio: bool) -> String {
    use sha2::{Digest, Sha256};
    STANDARD.encode(Sha256::digest(
        net_patch_body(id, base, token, studio).as_bytes(),
    ))
}

/// 令牌嵌进 JS 字符串字面量的转义：防引号截断字符串，`<` 转义防 `</script>`
/// 提前闭合注入的 script 块（令牌是用户配置的任意串，不假设它干净）。
fn js_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '<' => out.push_str("\\x3c"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// 补丁正文模板。`__P__` = `/console/:id/`，`__B__` = 该后端 webui_url（去 query/尾斜杠）。
/// 只放正文，`<script>` 标签由 `net_patch` 包——CSP 哈希对正文算（见 net_patch_hash）。
const NET_PATCH_BODY: &str = r#"(function(){
var P="__P__",B="__B__",T="__T__";
function rew(u){
  if(typeof u!=="string"||u==="")return null;
  if(u.charAt(0)==="/"){                                   // 根绝对路径 → 前缀化
    if(u.charAt(1)==="/"||u.indexOf("/console/")===0)return null;  // 协议相对/已代理
    return P+u.slice(1);
  }
  if(B&&u.indexOf(B)===0&&(u.length===B.length||u.charAt(B.length)==="/")){
    var t=u.slice(B.length);return t.charAt(0)==="/"?P+t.slice(1):P+t;  // 面板硬编码自己
  }
  if(/^[a-z][a-z0-9+.-]*:/i.test(u)){                      // 网关自身的绝对地址
    try{var x=new URL(u,location.href);
      if(x.origin===location.origin&&x.pathname.indexOf("/console/")!==0)
        return P+x.pathname.slice(1)+x.search+x.hash;
    }catch(e){}
  }
  return null;
}
function tok(u){                                      // 网关侧 URL 追加访问令牌（幂等）
  if(!T)return u;
  var k="access_token="+encodeURIComponent(T);
  try{
    var x=new URL(u,document.baseURI);                // 相对路径按 <base> 解析后再判
    if(x.origin!==location.origin)return u;          // 外域一个字节都不带（不泄令牌）
    if(x.search.indexOf(k)>=0)return u;               // 幂等：已有本令牌不重复
    var q=x.search?x.search+"&"+k:"?"+k;
    return x.origin+x.pathname+q+x.hash;              // fragment 归位到末尾
  }catch(e){return u;}
}
function fx(u){var r=rew(u);return tok(r===null?u:r);}
var _f=window.fetch;
if(_f)window.fetch=function(i,init){
  if(typeof i==="string")i=fx(i);
  else if(i&&typeof i.url==="string"){
    var t=tok(rew(i.url)||i.url);
    if(t!==i.url)i=new Request(t,i);
  }
  return _f.call(this,i,init);
};
var _o=window.XMLHttpRequest&&XMLHttpRequest.prototype.open;
if(_o)XMLHttpRequest.prototype.open=function(){
  var a=Array.prototype.slice.call(arguments);a[1]=fx(a[1]);return _o.apply(this,a);};
if(window.EventSource){var _E=window.EventSource;
  window.EventSource=function(u,c){return new _E(fx(u),c);};
  window.EventSource.prototype=_E.prototype;}
var _b=navigator.sendBeacon&&navigator.sendBeacon.bind(navigator);
if(_b)navigator.sendBeacon=function(u,d){return _b(fx(u),d);};
var _sa=window.Element&&Element.prototype.setAttribute;
if(_sa)Element.prototype.setAttribute=function(n,v){
  return _sa.call(this,n,(n==="src"||n==="href"||n==="action")?fx(v):v);};
function defSet(p,prop){                                  // 属性 setter（el.src="/x"）
  if(!p)return;var d=Object.getOwnPropertyDescriptor(p,prop);
  if(!d||!d.set)return;var s=d.set;
  Object.defineProperty(p,prop,{configurable:true,enumerable:d.enumerable,get:d.get,
    set:function(v){s.call(this,fx(v));}});
}
defSet(window.HTMLScriptElement&&HTMLScriptElement.prototype,"src");
defSet(window.HTMLLinkElement&&HTMLLinkElement.prototype,"href");
defSet(window.HTMLImageElement&&HTMLImageElement.prototype,"src");
defSet(window.HTMLAnchorElement&&HTMLAnchorElement.prototype,"href");
defSet(window.HTMLFormElement&&HTMLFormElement.prototype,"action");
function fixEl(n){
  if(!n||n.nodeType!==1||!n.getAttribute)return;
  var a=["src","href","action"];
  for(var i=0;i<a.length;i++){var v=n.getAttribute(a[i]);if(!v)continue;
    var r=rew(v);if(r!==null)n.setAttribute(a[i],r);}
}
function walk(n){fixEl(n);
  if(n.querySelectorAll){var l=n.querySelectorAll("[src],[href],[action]");
    for(var i=0;i<l.length;i++)fixEl(l[i]);}}
function hook(){var d=document.documentElement;if(!d)return;
  if(window.MutationObserver)new MutationObserver(function(ms){   // innerHTML 解析插入的
    for(var i=0;i<ms.length;i++){var add=ms[i].addedNodes;
      for(var j=0;j<add.length;j++)walk(add[j]);}
  }).observe(d,{childList:true,subtree:true});
  walk(d);}
if(document.readyState==="loading")document.addEventListener("DOMContentLoaded",hook);else hook();
})();"#;

/// Studio 专属启动 shim（engine=unsloth 才追加进 `net_patch` 正文，同一个 CSP 哈希）。
/// 实地诊断结论（反编译 Studio chunk + 浏览器验证）：代理下卡在「加载中...」有两道门——
/// ① 启动门只认 `localStorage.unsloth_auth_token`，没有就永远转圈；真实 API 鉴权
///    由代理的凭据注入承担，这里播一个假令牌只为过门（已有令牌不覆盖，尊重真实登录）。
/// ② TanStack Router 不知道自己被挂在 `/console/:id` 下，路由全 not-found；router
///    实例未导出，只能沿 React fiber 摸到 `memoizedProps.router` 再 `update({basepath})`。
///    router 在首帧渲染后才创建，用 setInterval 轮询到找到为止（上限 30s）。
///    `update` 只改 basepath **不重跑当前匹配**（实测初始 not-found 会留着），必须再
///    `navigate` 到剥掉前缀的路径触发重解析；根路径 `/` 的路由自带 redirect 到默认页。
/// `__BP__` = `/console/{id}`（无尾斜杠，与路由匹配口径一致）。
const STUDIO_SHIM: &str = r#"(function(){
try{
  if(!localStorage.getItem("unsloth_auth_token"))localStorage.setItem("unsloth_auth_token","llms-proxy-session");
  if(!localStorage.getItem("unsloth_auth_refresh_token"))localStorage.setItem("unsloth_auth_refresh_token","llms-proxy-session");
}catch(e){}
var tries=0;
var iv=setInterval(function(){
  if(++tries>300){clearInterval(iv);return;}
  try{
    var el=document.getElementById("root")||document.body.firstElementChild;
    if(!el)return;
    var k=null;for(var key in el){if(key.indexOf("__reactContainer$")===0){k=key;break;}}
    if(!k)return;
    var r=null,seen=0;
    (function w(x){
      while(x&&!r){
        if(seen++<20000){
          var p=x.memoizedProps;
          if(p&&p.router&&p.router.basepath!==undefined)r=p.router;
          else if(x.child)w(x.child);
        }
        x=x.sibling;
      }
    })(el[k]);
    if(r){
      clearInterval(iv);
      try{
        if(r.basepath!=="__BP__")r.update({basepath:"__BP__"});
        var sp=location.pathname.slice("__BP__".length)||"/";
        r.navigate({to:sp,replace:true});
      }catch(e){}
    }
  }catch(e){}
},100);
})();"#;

/// 改写 Studio `/api/auth/status` 响应：`requires_password_change` 强制 false。
/// 上游该标志为 true 会把前端踢去 /change-password（改真实密码越权、不可取），
/// 只动这一个字段；解析失败或非预期形态一律原样透传，不猜别的。
fn patch_studio_auth_status(bytes: &[u8]) -> Vec<u8> {
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return bytes.to_vec();
    };
    if v.get("requires_password_change").and_then(|x| x.as_bool()) == Some(true) {
        v["requires_password_change"] = serde_json::json!(false);
        return serde_json::to_vec(&v).unwrap_or_else(|_| bytes.to_vec());
    }
    bytes.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用固定 base（带路径前缀，顺带验 `B` 的剥法）；令牌恒为 `tk`。
    fn ib(html: &[u8], id: i64) -> Vec<u8> {
        inject_base(
            html,
            id,
            &Url::parse("http://h:3000/grafana/").unwrap(),
            "tk",
            false,
        )
    }

    #[test]
    fn deep_webui_url_resolves_root_absolute_assets() {
        // oMLX：入口是深层页面，根绝对资源必须按 origin 根解析，不能再拼一遍前缀。
        let deep = Url::parse("http://h:8000/admin/dashboard").unwrap();
        assert_eq!(doc_dir(&deep), "/admin/");
        assert_eq!(
            target_path(&deep, "admin/static/css/tailwind.css"),
            "/admin/static/css/tailwind.css"
        );
        // 文档相对（<base> 解析出来的）仍落在文档目录里。
        assert_eq!(target_path(&deep, "logo.png"), "/admin/logo.png");
        // 根文档 = 入口本身。
        assert_eq!(target_path(&deep, ""), "/admin/dashboard");

        // 带路径前缀的引擎（Grafana 式）：根绝对自带前缀段，文档相对落在前缀下。
        let graf = Url::parse("http://h:3000/grafana/").unwrap();
        assert_eq!(
            target_path(&graf, "grafana/public/app.js"),
            "/grafana/public/app.js"
        );
        assert_eq!(target_path(&graf, "d/main.json"), "/grafana/d/main.json");

        // 根入口：两条路重合。
        let root = Url::parse("http://h:8000/").unwrap();
        assert_eq!(target_path(&root, "assets/app.js"), "/assets/app.js");
        assert_eq!(target_path(&root, ""), "/");
    }

    #[test]
    fn cookie_path_rewrite() {
        assert_eq!(
            rewrite_cookie_path("sid=1; Path=/; HttpOnly", "/console/3"),
            "sid=1; Path=/console/3/; HttpOnly"
        );
        assert_eq!(
            rewrite_cookie_path("sid=1; Path=sub; Secure", "/console/3"),
            "sid=1; Path=/console/3/sub; Secure"
        );
        // 无 Path 属性 → 追加。
        assert_eq!(
            rewrite_cookie_path("sid=1", "/console/3"),
            "sid=1; Path=/console/3"
        );
    }

    #[test]
    fn location_rewrite_same_origin_only() {
        let base = Url::parse("http://h:80/grafana/").unwrap();
        assert_eq!(
            rewrite_location("http://h:80/grafana/dash?x=1", &base, "/console/1").as_deref(),
            Some("/console/1/dash?x=1")
        );
        // 外部域不改写。
        assert_eq!(rewrite_location("http://evil/x", &base, "/console/1"), None);
        // 相对跳转交给浏览器按 base 解析。
        assert_eq!(rewrite_location("./rel", &base, "/console/1"), None);
    }

    #[test]
    fn base_injection_positions() {
        let id = 7;
        let html = ib(b"<html><head lang=x><title>t</title>", id);
        let s = String::from_utf8_lossy(&html);
        assert!(s.contains("<head lang=x><base href=\"/console/7/\">"));
        assert!(s.contains("data-llms-net"), "运行时补丁必须一并注入");
        assert!(
            !s.contains("data-llms-fit"),
            "整页缩放已按内容自适应口径移除，不得再注入"
        );
        let html = ib(b"plain", id);
        let s = String::from_utf8_lossy(&html);
        assert!(s.starts_with("<base href=\"/console/7/\">"));
        assert!(!s.contains("data-llms-fit"));
        assert!(s.ends_with("plain"));
    }

    #[test]
    fn auth_priority() {
        let mut b = crate::types::backend_row_for_test("x");
        b.base_url = "http://h".into();
        b.api_key = Some("sk-api".into());
        let same = Url::parse("http://h:3000/grafana/").unwrap();
        // 都空 → api_key（同主机不同端口也发：WebUI 常另开一个端口）。
        assert_eq!(console_auth(&b, &same).unwrap(), "Bearer sk-api");
        // 仅密码 → Bearer 密码。
        b.webui_password = Some("p1".into());
        assert_eq!(console_auth(&b, &same).unwrap(), "Bearer p1");
        // 账户+密码 → Basic。
        b.webui_user = Some("u1".into());
        assert!(console_auth(&b, &same)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Basic "));
        // 只有账户没有密码 → 回落 api_key。
        b.webui_password = None;
        assert_eq!(console_auth(&b, &same).unwrap(), "Bearer sk-api");

        // 入口换了主机：api_key 不属于那里，不发（账户/密码是用户明确填的，照发）。
        let elsewhere = Url::parse("http://other.example/").unwrap();
        b.webui_user = None;
        assert_eq!(console_auth(&b, &elsewhere), None);
        b.webui_user = Some("u1".into());
        b.webui_password = Some("p1".into());
        assert!(console_auth(&b, &elsewhere).is_some());
    }

    #[test]
    fn url_logging_drops_query() {
        let u = Url::parse("http://h:9000/ui/session?token=s3cret&page=2").unwrap();
        assert_eq!(redact_url(&u), "http://h:9000/ui/session");
    }

    /// 测试用文档地址（深层页面，顺带验相对路径按文档目录解析）；令牌恒为 `tk`。
    fn deep_doc() -> Url {
        Url::parse("http://h:8000/admin/dashboard").unwrap()
    }

    fn ha(html: &[u8], id: i64) -> String {
        String::from_utf8(rewrite_html_assets(html, id, &deep_doc(), "tk")).unwrap()
    }

    #[test]
    fn html_assets_absolute_paths_carry_token() {
        // oMLX 形态：根绝对资源 + 带版本 query 的静态件，一条都不能漏令牌——
        // 子资源在 WKWebView 里既无 Cookie 也无 Referer，只有 URL 是确定通道。
        let html =
            br#"<html><head><link rel="stylesheet" href="/admin/static/css/app.css?v=9"></head>
<body><img src="/img/logo.png"><script src="/admin/static/js/app.js"></script>
<form action="/admin/login" method="post"></form></body></html>"#;
        let s = ha(html, 123);
        assert!(
            s.contains(r#"href="/console/123/admin/static/css/app.css?v=9&access_token=tk""#),
            "根绝对 + query 的资源要前缀化并补令牌: {s}"
        );
        assert!(
            s.contains(r#"src="/console/123/img/logo.png?access_token=tk""#),
            "根绝对 src 前缀化并补令牌: {s}"
        );
        assert!(
            s.contains(r#"src="/console/123/admin/static/js/app.js?access_token=tk""#),
            "script src 同款: {s}"
        );
        assert!(
            s.contains(r#"action="/console/123/admin/login?access_token=tk""#),
            "表单提交地址同样要带令牌（否则登录/保存类面板一提交就 401）: {s}"
        );
    }

    #[test]
    fn html_assets_relative_paths_resolve_against_document() {
        // 相对路径以前交给 <base>——形状对了但没令牌，照样 401。现在按文档地址解析。
        let html =
            br#"<a href="foo.html"><img src='bar.png'><link href="./x.css"><link href="../up.js">"#;
        let s = ha(html, 123);
        assert!(
            s.contains(r#"href="/console/123/admin/foo.html?access_token=tk""#),
            "文档相对: {s}"
        );
        assert!(
            s.contains("src='/console/123/admin/bar.png?access_token=tk'"),
            "单段相对: {s}"
        );
        assert!(
            s.contains(r#"href="/console/123/admin/x.css?access_token=tk""#),
            "./ 相对: {s}"
        );
        assert!(
            s.contains(r#"href="/console/123/up.js?access_token=tk""#),
            "../ 相对要爬出文档目录: {s}"
        );
    }

    #[test]
    fn html_assets_leave_external_and_idempotent() {
        let html = br##"<a href="https://ext.example/x">ext</a><script src="//cdn.example/j.js"></script>
<a href="#top">top</a><img src="data:image/png;base64,AA"><link href="/console/123/already.css?access_token=tk">
<a href="mailto:a@b.c">mail</a>"##;
        let s = ha(html, 123);
        // 完整 URL / 协议相对 / 纯锚点 / data: / mailto: → 不碰，别把外部资源打到网关上
        for keep in [
            r#"href="https://ext.example/x""#,
            r#"src="//cdn.example/j.js""#,
            r##"href="#top""##,
            r#"src="data:image/png;base64,AA""#,
            r#"href="mailto:a@b.c""#,
        ] {
            assert!(s.contains(keep), "不该改写的值被改了（{keep}）: {s}");
        }
        // 已带本 id 前缀 → 幂等，不二次注入
        assert!(
            s.contains(r#"href="/console/123/already.css?access_token=tk""#),
            "已带前缀不重注入: {s}"
        );
    }

    #[test]
    fn html_assets_handles_uppercase_attrs() {
        let html = br#"<SCRIPT SRC="/a.js"></SCRIPT><A HREF='/b'>x</A>"#;
        let s = ha(html, 9);
        assert!(
            s.contains(r#"SRC="/console/9/a.js?access_token=tk""#),
            "大写 SRC 也要改写: {s}"
        );
        assert!(
            s.contains(r#"HREF='/console/9/b?access_token=tk'"#),
            "大写 HREF 也要改写: {s}"
        );
    }

    #[test]
    fn with_access_token_is_idempotent_and_encodes() {
        // 已有本令牌 → 不重复追加。
        assert_eq!(
            with_access_token("/console/1/x?access_token=tk", "tk"),
            "/console/1/x?access_token=tk"
        );
        // 无 query 用 ?，有 query 用 &；令牌里的空格按 form 转义（网关按同款解析还原）。
        assert_eq!(
            with_access_token("/console/1/x", "tk"),
            "/console/1/x?access_token=tk"
        );
        assert_eq!(
            with_access_token("/console/1/x?v=2", "tk"),
            "/console/1/x?v=2&access_token=tk"
        );
        assert_eq!(
            with_access_token("/console/1/x", "a b&c"),
            "/console/1/x?access_token=a+b%26c"
        );
        // 未设令牌（开放回环口径）→ 原样。
        assert_eq!(with_access_token("/console/1/x", ""), "/console/1/x");
    }

    #[test]
    fn css_url_and_import_rewritten() {
        // 字体/图片由浏览器按 CSS 里的地址去取，页面侧补丁够不着——CSS 必须自己带令牌。
        let css = r##"@font-face{font-family:I;src:url("./inter-latin-400.woff2") format("woff2")}
.hero{background:url(/img/bg.png)}
.raw{background:url(data:image/svg+xml;utf8,<svg>)}
.glow{fill:url("#cluster-fabric-glow")}
@import "base.css";
@import url('z.css');
@import screen;"##;
        let out = rewrite_css(css, &deep_doc(), 7, "tk");
        assert!(
            out.contains(r#"url("/console/7/admin/inter-latin-400.woff2?access_token=tk")"#),
            "文档相对字体要按 CSS 地址解析并带令牌: {out}"
        );
        assert!(
            out.contains(r#"url("/console/7/img/bg.png?access_token=tk")"#),
            "根绝对背景图要前缀化: {out}"
        );
        assert!(
            out.contains("url(data:image/svg+xml;utf8,<svg>)"),
            "data: 原样: {out}"
        );
        assert!(
            out.contains(r##"url("#cluster-fabric-glow")"##),
            "SVG 锚点引用原样: {out}"
        );
        assert!(
            out.contains(r#"@import "/console/7/admin/base.css?access_token=tk""#),
            "带引号的 @import 也要改写: {out}"
        );
        assert!(
            out.contains(r#"@import url("/console/7/admin/z.css?access_token=tk")"#),
            "@import url() 形态由 url() 支路处理: {out}"
        );
        assert!(out.contains("@import screen;"), "非字符串形态原样: {out}");
    }

    #[test]
    fn css_rewrite_preserves_bytes_when_nothing_matches() {
        let css = ".a{color:red}.b{margin:0 10px}.c{content:\")\"}";
        assert_eq!(rewrite_css(css, &deep_doc(), 7, "tk"), css);
    }

    #[test]
    fn inject_base_injects_base_patch_and_rewrite() {
        let html = br#"<html><head><link href="/app.css"></head><body></body></html>"#;
        let out = ib(html, 1);
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains(r#"<head><base href="/console/1/">"#),
            "base 插在 head 之后: {s}"
        );
        assert!(
            !s.contains("data-llms-fit"),
            "整页缩放脚本已移除（内容自适应口径），不得再注入: {s}"
        );
        assert!(
            s.contains(r#"href="/console/1/app.css?access_token=tk""#),
            "绝对路径一并前缀化并带令牌: {s}"
        );
    }

    #[test]
    fn inject_base_without_head_prepends() {
        let html = br#"<html><body><img src="/i.png"></body></html>"#;
        let out = ib(html, 1);
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.starts_with(r#"<base href="/console/1/">"#),
            "无 head 也要前置 base: {s}"
        );
        assert!(
            !s.contains("data-llms-fit"),
            "无 head 也不注入缩放脚本: {s}"
        );
        assert!(
            s.contains(r#"src="/console/1/i.png?access_token=tk""#),
            "无 head 也要改写绝对路径并带令牌: {s}"
        );
    }

    #[test]
    fn inject_base_malformed_head_does_not_panic() {
        // head 标签残缺（没有 '>'）时不能 panic——这跑在请求热路径上。
        let out = ib(b"<html><head", 1);
        assert!(
            String::from_utf8_lossy(&out).starts_with("<base"),
            "head 残缺时退回前置注入"
        );
    }

    #[test]
    fn inject_base_installs_runtime_patch() {
        let out = inject_base(
            br#"<html><head></head><body></body></html>"#,
            5,
            &Url::parse("http://10.0.0.2:8739/grafana/").unwrap(),
            "tk",
            false,
        );
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("data-llms-net"), "运行时补丁必须注入: {s}");
        assert!(s.contains(r#"var P="/console/5/""#), "前缀按 id 拼: {s}");
        assert!(
            s.contains(r#"B="http://10.0.0.2:8739/grafana""#),
            "B 剥 query 与尾斜杠: {s}"
        );
        // 顺序：base → 运行时补丁，都在 head 内、app 脚本（body）之前。
        let b = s.find("<base ").unwrap();
        let n = s.find("data-llms-net").unwrap();
        assert!(b < n, "注入顺序 base<net: {s}");
    }

    #[test]
    fn net_patch_is_idempotent_by_guard() {
        // 幂等靠补丁内的 `/console/` 守卫——这里只验守卫真的在源码里（JS 逻辑
        // 由 node --check/人工验收兜底，Rust 侧不复刻 JS 解释器）。
        let p = net_patch(
            1,
            &Url::parse("http://h:3000/grafana/").unwrap(),
            "tk",
            false,
        );
        assert!(p.contains(r#"u.indexOf("/console/")===0"#), "{p}");
        assert!(p.contains("data-llms-net"));
    }

    #[test]
    fn net_patch_embeds_token_with_js_escaping() {
        let base = Url::parse("http://h:8000/").unwrap();
        // 令牌嵌进 JS 字面量：追加逻辑在，且恶意串不会截断 script。
        let p = net_patch(2, &base, "s3cr et\"x", false);
        assert!(p.contains(r#"T="s3cr et\"x""#), "{p}");
        assert!(p.contains(r#"access_token="+encodeURIComponent(T)"#), "{p}");
        assert!(p.contains("function tok(u)"), "tok 函数必须在: {p}");
        assert!(
            p.contains("new URL(u,document.baseURI)"),
            "相对路径也要按 baseURI 解析进 tok: {p}"
        );
        assert!(
            p.contains("x.origin!==location.origin"),
            "外域必须挡在令牌之外: {p}"
        );
        // `</script` 序列必须被转义，否则注入的 script 提前闭合。
        let evil = net_patch(2, &base, "a</script><b>", false);
        assert!(!evil.contains("</script><b>"), "{evil}");
        assert!(evil.contains(r#"\x3c/script>"#), "{evil}");
        // 未设令牌 → T 为空串，tok 原样放行（全开回环口径不变）。
        let none = net_patch(2, &base, "", false);
        assert!(none.contains(r#"T="""#), "{none}");
    }

    #[test]
    fn console_responses_forbid_caching() {
        // 内嵌通道必须 no-store：WebKit 缓存无校验器的旧变体会让补丁失灵。
        let base = Url::parse("http://h:8000/").unwrap();
        let mut src = HeaderMap::new();
        src.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html"));
        src.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=600"),
        );
        let out = transform_response_headers(&src, &base, 3, "tk", false);
        assert_eq!(
            out.get(header::CACHE_CONTROL).and_then(|v| v.to_str().ok()),
            Some("no-store"),
            "上游的缓存头必须被 no-store 覆盖"
        );
    }

    /// 上游真实 CSP（Unsloth 实测值）：只摘 frame-ancestors + 追加补丁哈希，
    /// 其余指令与源一个不动——尤其 `script-src 'self'` 必须还在（不许退化成放开）。
    #[test]
    fn csp_keeps_upstream_sources_and_only_allows_patch() {
        let src_csp = "default-src 'self'; img-src 'self' data: blob: https:; \
                       script-src 'self'; style-src 'self' 'unsafe-inline'; \
                       frame-ancestors 'none'; base-uri 'self'";
        let mut src = HeaderMap::new();
        src.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html"));
        src.insert(
            "content-security-policy",
            HeaderValue::from_str(src_csp).unwrap(),
        );
        let base = Url::parse("http://127.0.0.1:8888/").unwrap();
        let out_csp = transform_response_headers(&src, &base, 23, "tk", false)
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok())
            .expect("CSP 必须还在")
            .to_string();
        let out = out_csp.as_str();
        assert!(
            !out.contains("frame-ancestors"),
            "内嵌通道必须摘掉 frame-ancestors: {out}"
        );
        assert!(
            out.contains("script-src 'self' 'sha256-"),
            "script-src 保留 'self' 并只多认这一段哈希: {out}"
        );
        assert!(
            out.contains("img-src 'self' data: blob: https:"),
            "无关指令原样保留: {out}"
        );
        assert!(
            out.contains("base-uri 'self'"),
            "base-uri 必须留着——注入的 <base> 靠它才合法: {out}"
        );
        assert!(
            out.matches("'unsafe-inline'").count() == 1,
            "只许 style-src 原有的那一个 unsafe-inline，别给 script-src 加: {out}"
        );
        // 幂等：再跑一次不重复追加。
        assert_eq!(
            csp_allow_patch(out, &net_patch_hash(23, &base, "tk", false)),
            out
        );
    }

    /// 管脚本的指令缺位时回落 default-src；两条都不限脚本时原样返回。
    #[test]
    fn csp_falls_back_to_default_src_only_when_needed() {
        let hash = "QUJDRA==";
        let only_default = csp_allow_patch("default-src 'self'", hash);
        assert_eq!(only_default, "default-src 'self' 'sha256-QUJDRA=='");
        let both = csp_allow_patch("default-src 'self'; script-src 'none'", hash);
        assert!(
            both.contains("script-src 'none' 'sha256-QUJDRA=='"),
            "有 script-src 时哈希只加它: {both}"
        );
        assert!(
            !both.contains("default-src 'self' 'sha256"),
            "有 script-src 时别动 default-src: {both}"
        );
        let unrestricted = csp_allow_patch("img-src 'self'", hash);
        assert_eq!(unrestricted, "img-src 'self'", "不限脚本就不必加");
    }

    /// CSP 里那个哈希必须与**实际注入文档**的补丁正文逐字节对上，否则等于没修：
    /// 补丁仍被拦 → fetch 不打补丁 → 控制台 401 登录墙（2026-11-06 Unsloth 现场）。
    #[test]
    fn csp_hash_matches_the_injected_body() {
        let base = Url::parse("http://127.0.0.1:8888/").unwrap();
        let (id, tok) = (23, "sk-unsloth-x");
        use sha2::Digest;
        let tag = net_patch(id, &base, tok, false);
        let body = tag
            .strip_prefix("<script data-llms-net>")
            .and_then(|s| s.strip_suffix("</script>"))
            .expect("net_patch 仍是 script 标签包正文");
        // 正文与 CSP 哈希同源（都取 net_patch_body），这里钉死「标签里的正文 == 算哈希的正文」。
        assert_eq!(body, net_patch_body(id, &base, tok, false));
        let want = format!(
            "'sha256-{}'",
            STANDARD.encode(sha2::Sha256::digest(body.as_bytes()))
        );
        let csp = csp_allow_patch("script-src 'self'", &net_patch_hash(id, &base, tok, false));
        assert!(csp.contains(&want), "CSP 哈希对不上注入正文: {csp}");
        // 注入到 HTML 里的也是这份正文（rewrite 不碰它）。
        let html = String::from_utf8(inject_base(
            br#"<html><head></head><body></body></html>"#,
            id,
            &base,
            tok,
            false,
        ))
        .unwrap();
        assert!(html.contains(body), "注入的正文与算哈希的不是同一份");
    }

    /// Studio shim 与 CSP 哈希的口径：`studio=true` 时 shim 追加进**同一份正文**，
    /// 哈希必须仍与注入正文逐字节对上（只开一个 script 块、只放行一个哈希）。
    #[test]
    fn studio_shim_shares_the_same_csp_hash() {
        let base = Url::parse("http://127.0.0.1:8888/").unwrap();
        let (id, tok) = (23, "tk");
        let plain = net_patch_body(id, &base, tok, false);
        let studio = net_patch_body(id, &base, tok, true);
        assert!(!plain.contains("unsloth_auth_token"), "非 Studio 不带 shim");
        assert!(
            studio.contains("unsloth_auth_token"),
            "Studio 正文要带令牌播种"
        );
        assert!(
            studio.contains(r#"basepath:"/console/23""#),
            "shim 的 basepath 按 id 拼"
        );
        assert!(
            studio.contains("r.navigate({to:sp,replace:true})"),
            "update 后必须 navigate 重跑匹配（update 不重跑，实测 not-found 会留着）"
        );
        assert!(
            studio.starts_with(&plain),
            "shim 是追加，net_patch 本体不动"
        );
        // 关键不变量：CSP 哈希 == 注入文档正文的哈希（studio 口径）。
        let html = String::from_utf8(inject_base(
            br#"<html><head></head><body></body></html>"#,
            id,
            &base,
            tok,
            true,
        ))
        .unwrap();
        let tag = net_patch(id, &base, tok, true);
        let body = tag
            .strip_prefix("<script data-llms-net>")
            .and_then(|s| s.strip_suffix("</script>"))
            .unwrap();
        assert!(html.contains(body), "注入正文与 net_patch 不一致");
        let csp = csp_allow_patch("script-src 'self'", &net_patch_hash(id, &base, tok, true));
        use sha2::Digest;
        let want = format!(
            "'sha256-{}'",
            STANDARD.encode(sha2::Sha256::digest(body.as_bytes()))
        );
        assert!(csp.contains(&want), "Studio 口径 CSP 哈希对不上: {csp}");
    }

    /// auth/status 改写只动 `requires_password_change`，其余字段原样；
    /// 非 JSON / 无该字段时原样透传（不猜）。
    #[test]
    fn studio_auth_status_patch_only_touches_the_flag() {
        let src = br#"{"initialized":true,"login_mode":"single","requires_password_change":true}"#;
        let out = patch_studio_auth_status(src);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["requires_password_change"], false);
        assert_eq!(v["login_mode"], "single");
        assert_eq!(v["initialized"], true);
        // 已是 false：字节不动。
        let ok = br#"{"requires_password_change":false}"#;
        assert_eq!(patch_studio_auth_status(ok), ok);
        // 垃圾输入原样透传。
        let junk = b"<html>not json</html>";
        assert_eq!(patch_studio_auth_status(junk), junk);
    }
}
