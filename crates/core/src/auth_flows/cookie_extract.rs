//! WebView & manual cookie-jar extraction + `HttpOnly` session validation (`H3.1`).

use resonance_contract::{AuthError, AuthSession};

use crate::innertube::auth::extract_cookie;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedCookies {
    pub has_sapisid: bool,
    pub has_3papisid: bool,
    pub has_session_cookie: bool,
    pub cookie_header: String,
}

/// Validates that a captured or pasted `Cookie` header contains BOTH:
/// 1. `SAPISID` or `__Secure-3PAPISID` / `__Secure-1PAPISID` (for `SAPISIDHASH`), AND
/// 2. An `HttpOnly` session cookie (`SID`, `__Secure-1PSID`, or `__Secure-3PSID`).
///
/// Prevents silent 401s when JS `document.cookie` only captures non-`HttpOnly` cookies.
pub fn validate_cookie_jar(cookie_header: &str) -> Result<ExtractedCookies, AuthError> {
    let trimmed = cookie_header.trim();
    if trimmed.is_empty() {
        return Err(AuthError::MissingCookie(
            "SAPISID or __Secure-3PAPISID".to_owned(),
        ));
    }

    let has_sapisid = extract_cookie(trimmed, "SAPISID").is_some();
    let has_1papisid = extract_cookie(trimmed, "__Secure-1PAPISID").is_some();
    let has_3papisid = extract_cookie(trimmed, "__Secure-3PAPISID").is_some();

    if !has_sapisid && !has_1papisid && !has_3papisid {
        return Err(AuthError::MissingCookie(
            "SAPISID or __Secure-3PAPISID".to_owned(),
        ));
    }

    let has_session_cookie = extract_cookie(trimmed, "SID").is_some()
        || extract_cookie(trimmed, "__Secure-1PSID").is_some()
        || extract_cookie(trimmed, "__Secure-3PSID").is_some();

    if !has_session_cookie {
        return Err(AuthError::MissingCookie(
            "HttpOnly session cookie (SID or __Secure-3PSID) missing — use TV OAuth or paste full Cookie header".to_owned(),
        ));
    }

    Ok(ExtractedCookies {
        has_sapisid,
        has_3papisid,
        has_session_cookie,
        cookie_header: trimmed.to_owned(),
    })
}

/// Builds a validated [`AuthSession`] from a complete browser `Cookie` header.
pub fn build_auth_session_from_cookies(
    cookie_header: &str,
    auth_user: u32,
) -> Result<AuthSession, AuthError> {
    let validated = validate_cookie_jar(cookie_header)?;
    Ok(AuthSession {
        cookie_header: validated.cookie_header,
        auth_user,
        page_id: None,
        visitor_data: None,
        account_name: None,
        is_premium: false,
    })
}
