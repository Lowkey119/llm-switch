//! 凭据脱敏（红线切片 #43）：**所有 API 密钥不得出现在任何日志/事件/错误文本里**。
//!
//! 手写扫描器（仓库不引 regex 依赖），覆盖三种真实泄漏形态：
//! 1. 带前缀的密钥串——`sk-...`、`gsk_...`、`AIza...` 等（上游 401 响应体常回显）；
//! 2. `Bearer <token>` / `Authorization: ...` 头形态；
//! 3. `key=value` 与 `"api_key": "value"` 参数形态（URL query 与 JSON 同口径）。
//!
//! 命中一律换成 `***`。宁可偶尔多掩一个像凭据的词，也不放一个真 key 进日志——
//! 但只在「长得真像凭据」处动手（前缀 ≥8 位密钥字符、参数值 ≥4 字符），
//! 免得把正常诊断文本打成筛子。

/// `key=value` / `"key": "value"` 形态的敏感参数名（ASCII 小写比较）。
const PAIR_KEYS: &[&str] = &[
    "authorization",
    "x-api-key",
    "api_key",
    "apikey",
    "api-key",
    "access_token",
    "refresh_token",
    "password",
    "token",
    "secret",
    "key",
];

/// 带前缀的密钥串（大小写敏感：这些前缀本身即形态的一部分）。
const TOKEN_PREFIXES: &[&str] = &["sk-", "sk_", "gsk_", "hf_", "xai-", "nvapi-", "AIza"];

/// 密钥字符集：前缀之后（或之前）算同一个词。
fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// 把文本里所有凭据味的内容换成 `***`（日志、事件、错误诊断的统一出口）。
pub fn secrets(text: &str) -> String {
    let s = mask_bearer(text);
    let s = mask_pairs(&s);
    mask_token_prefixes(&s)
}

/// `Bearer <token>`（大小写不敏感）→ `Bearer ***`。
fn mask_bearer(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if matches_word(&chars, i, "bearer") {
            let mut j = i + 6;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let start = j;
            while j < chars.len() && (is_key_char(chars[j]) || chars[j] == '/') {
                j += 1;
            }
            if j > start {
                out.extend(&chars[i..start]);
                out.push_str("***");
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `key=value` / `"key": "value"` → 值换 `***`。
fn mask_pairs(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'outer: while i < chars.len() {
        for key in PAIR_KEYS {
            let kc: Vec<char> = key.chars().collect();
            if !matches_word(&chars, i, key) {
                continue;
            }
            // 词首边界：前面是标识符字符就不认（`monkey` 里的 `key` 不算）。
            if i > 0 && is_key_char(chars[i - 1]) {
                continue;
            }
            let mut j = i + kc.len();
            // JSON 形态 key 后可能裹着引号（`"api_key":`），与空白一并跳过。
            while j < chars.len() && (chars[j].is_whitespace() || chars[j] == '"') {
                j += 1;
            }
            if j >= chars.len() || !matches!(chars[j], '=' | ':') {
                continue;
            }
            j += 1;
            while j < chars.len() && (chars[j].is_whitespace() || chars[j] == '"') {
                j += 1;
            }
            let start = j;
            let is_auth = *key == "authorization";
            // 认证头形态：方案词（Bearer/Basic/…）保留可读（读日志的人需要知道
            // 形态），其后凭据本体掩掉；裸凭据（无方案词）整段掩。
            let mask_from = if is_auth {
                auth_scheme_len(&chars, start)
                    .and_then(|n| skip_ws(&chars, start + n))
                    .unwrap_or(start)
            } else {
                start
            };
            let mut e = mask_from;
            // 认证头值可含 base64 的 `+/=`，但仍是单个词（空白即凭据结束）。
            while e < chars.len()
                && !chars[e].is_whitespace()
                && !matches!(chars[e], ',' | '}' | ']' | '"' | '\'' | '&' | '<' | '>')
            {
                e += 1;
            }
            if e > mask_from && (is_auth || e - mask_from >= 4) {
                out.extend(&chars[i..mask_from]);
                out.push_str("***");
                i = e;
                continue 'outer;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `sk-...` / `AIza...` 等前缀密钥 → 整串换 `***`。
fn mask_token_prefixes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    'outer: while i < chars.len() {
        for prefix in TOKEN_PREFIXES {
            let pc: Vec<char> = prefix.chars().collect();
            if i + pc.len() > chars.len() || chars[i..i + pc.len()] != pc[..] {
                continue;
            }
            // `task-xxx` 里的 `sk-` 不算（前缀前是标识符字符）。
            if i > 0 && is_key_char(chars[i - 1]) {
                continue;
            }
            let mut j = i + pc.len();
            while j < chars.len() && is_key_char(chars[j]) {
                j += 1;
            }
            if j - (i + pc.len()) >= 8 {
                out.push_str("***");
                i = j;
                continue 'outer;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 从 `i` 起是否命中单词（ASCII 大小写不敏感），且词后不是标识符字符。
fn matches_word(chars: &[char], i: usize, word: &str) -> bool {
    let w: Vec<char> = word.chars().collect();
    if i + w.len() > chars.len() {
        return false;
    }
    if !chars[i..i + w.len()]
        .iter()
        .zip(&w)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
    {
        return false;
    }
    !chars
        .get(i + w.len())
        .is_some_and(|next| is_key_char(*next))
}

/// 跳过空白后的下标（无空白原样返回；越界返回 None）。
fn skip_ws(chars: &[char], i: usize) -> Option<usize> {
    if i > chars.len() {
        return None;
    }
    let mut j = i;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    Some(j)
}

/// 认证头的方案词长度（`Bearer`/`Basic`/`Digest` 等，大小写不敏感）；不是方案词返回 None。
fn auth_scheme_len(chars: &[char], i: usize) -> Option<usize> {
    ["bearer", "basic", "digest", "token", "apikey"]
        .into_iter()
        .find(|s| matches_word(chars, i, s))
        .map(|s| s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_openai_style_key_anywhere() {
        let s = "Incorrect API key provided: sk-proj-Abc123Def456Ghi789";
        assert!(!secrets(s).contains("sk-proj"));
        assert!(secrets(s).contains("***"));
    }

    #[test]
    fn masks_google_and_groq_prefixes() {
        assert!(!secrets("AIzaSyA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6q").contains("AIzaSy"));
        assert!(!secrets("key gsk_abcdefghijklm12345 done").contains("gsk_abc"));
    }

    #[test]
    fn masks_bearer_token() {
        let out = secrets("Authorization: Bearer abc123def456ghi789 rejected");
        assert!(!out.contains("abc123def456"));
        assert!(out.to_lowercase().contains("bearer ***"));
    }

    #[test]
    fn masks_query_and_json_pairs() {
        assert!(!secrets("post /v1?key=SuperSecretValue&x=1").contains("SuperSecretValue"));
        assert!(!secrets(r#"{"api_key":"SuperSecretValue"}"#).contains("SuperSecretValue"));
        assert!(!secrets(r#""access_token": "ghs_abcdef1234567890""#).contains("ghs_abcdef"));
    }

    #[test]
    fn keeps_normal_diagnostic_text() {
        // 正常诊断不被打成筛子：无凭据味的词、短数字值都原样保留。
        let s = "upstream returned 400: model `qwen3.8` not found, token: 42";
        assert_eq!(secrets(s), s);
        let s2 = "invalid request: missing key field";
        assert_eq!(secrets(s2), s2);
    }

    #[test]
    fn does_not_mask_embedded_substring() {
        // `monkey` / `tokens:` 这类词中词不误伤。
        let s = "the monkey kept its keys safe";
        assert_eq!(secrets(s), s);
    }
}
