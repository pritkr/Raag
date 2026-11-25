//! Google Sign-In cookie authentication (`SAPISIDHASH`), rotation, Android WebView UA
//! sanitization (`; wv` stripping), and OS Keystore / atomic file AuthStore.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use resonance_contract::{AuthError, AuthSession, AuthStore};
use sha1::{Digest, Sha1};

use crate::innertube::clients::ClientConfig;

/// Standard mobile Chrome User-Agent fallback for Android WebView Google Sign-In.
pub const MOBILE_CHROME_UA: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36";

/// Strips the `; wv` WebView marker and `Version/X.Y` token from an Android WebView
/// User-Agent to prevent Google's `403: disallowed_useragent` rejection during Sign-In.
pub fn sanitize_android_webview_ua(raw_ua: &str) -> String {
    if raw_ua.trim().is_empty() {
        return MOBILE_CHROME_UA.to_owned();
    }
    let cleaned = raw_ua
        .replace("; wv)", ")")
        .replace("; wv;", ";")
        .replace("; wv", "");
    // Also remove Android WebView's `Version/4.0 ` marker if present.
    let mut parts = Vec::new();
    for token in cleaned.split_whitespace() {
        if token.starts_with("Version/") {
            continue;
        }
        parts.push(token);
    }
    let out = parts.join(" ");
    if out.contains("Chrome/") {
        out
    } else {
        MOBILE_CHROME_UA.to_owned()
    }
}

/// Extracts a cookie value by name from a `Cookie` header string.
pub fn extract_cookie<'a>(cookie_header: &'a str, name: &str) -> Option<&'a str> {
    cookie_header
        .split(';')
        .find_map(|kv| kv.trim().strip_prefix(name)?.strip_prefix('='))
        .filter(|v| !v.is_empty())
}

/// Computes `SAPISIDHASH` (+ `SAPISID1PHASH` / `SAPISID3PHASH` when present)
/// for `origin` at `timestamp_secs`.
pub fn compute_sapisidhash(
    cookie_header: &str,
    origin: &str,
    timestamp_secs: u64,
) -> Result<String, AuthError> {
    let schemes = [
        ("SAPISIDHASH", "SAPISID"),
        ("SAPISID1PHASH", "__Secure-1PAPISID"),
        ("SAPISID3PHASH", "__Secure-3PAPISID"),
    ];
    let mut parts = Vec::new();
    for (scheme, cookie_name) in schemes {
        if let Some(sid) = extract_cookie(cookie_header, cookie_name) {
            let input = format!("{timestamp_secs} {sid} {origin}");
            let digest = Sha1::digest(input.as_bytes());
            parts.push(format!("{scheme} {timestamp_secs}_{}", hex::encode(digest)));
        }
    }
    if parts.is_empty() {
        return Err(AuthError::MissingCookie(
            "SAPISID or __Secure-3PAPISID".to_owned(),
        ));
    }
    Ok(parts.join(" "))
}

/// Builds authenticated headers ONLY if `client.allows_browser_cookies()` is true.
/// Enforces Protocol Rule #1: strict per-client auth isolation.
pub fn build_auth_headers(
    client: &ClientConfig,
    session: &AuthSession,
    timestamp_secs: u64,
) -> Result<Vec<(&'static str, String)>, AuthError> {
    if !client.allows_browser_cookies() {
        return Err(AuthError::IsolationViolation(format!(
            "refused to attach browser cookies/SAPISIDHASH to client {} ({:?})",
            client.client_name, client.auth_mode
        )));
    }
    let auth_header = compute_sapisidhash(&session.cookie_header, client.origin, timestamp_secs)?;
    let mut headers = vec![
        ("Cookie", session.cookie_header.clone()),
        ("Authorization", auth_header),
        ("X-Goog-AuthUser", session.auth_user.to_string()),
    ];
    if let Some(page_id) = &session.page_id {
        headers.push(("X-Goog-PageId", page_id.clone()));
    }
    Ok(headers)
}

/// Redacts cookies, SAPISIDHASH authorization headers, and signed googlevideo URLs
/// before writing diagnostics, logs, or QA artifacts.
pub fn redact_secrets(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.split_inclusive('\n') {
        let lower = line.to_ascii_lowercase();
        if lower.contains("sapisid")
            || lower.contains("authorization:")
            || lower.contains("cookie:")
            || lower.contains("googlevideo.com/videoplayback")
            || lower.contains("pot=")
            || lower.contains("potoken")
        {
            let has_nl = line.ends_with('\n');
            out.push_str("[REDACTED_SENSITIVE_LINE]");
            if has_nl {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Merges `Set-Cookie` rotation headers returned by Google into an existing `AuthSession`.
/// Returns `Some(updated_session)` if any cookie value changed.
pub fn apply_cookie_rotation<'a>(
    session: &AuthSession,
    set_cookies: impl IntoIterator<Item = &'a str>,
) -> Option<AuthSession> {
    let mut jar: BTreeMap<String, String> = session
        .cookie_header
        .split(';')
        .filter_map(|kv| {
            let (k, v) = kv.trim().split_once('=')?;
            Some((k.trim().to_owned(), v.trim().to_owned()))
        })
        .collect();

    let mut changed = false;
    for raw in set_cookies {
        let Some(first) = raw.split(';').next() else {
            continue;
        };
        let Some((name, val)) = first.trim().split_once('=') else {
            continue;
        };
        let name = name.trim();
        let val = val.trim();
        // Ignore deletion cookies (empty value / deleted marker); 401 handles real revocation.
        if name.is_empty() || val.is_empty() || val.eq_ignore_ascii_case("deleted") {
            continue;
        }
        let lower = raw.to_ascii_lowercase();
        if lower.contains("max-age=0") {
            continue;
        }
        if jar.get(name).map(String::as_str) != Some(val) {
            jar.insert(name.to_owned(), val.to_owned());
            changed = true;
        }
    }

    if !changed {
        return None;
    }

    let cookie_header = jar
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    Some(AuthSession {
        cookie_header,
        ..session.clone()
    })
}

/// File-backed atomic `AuthStore` (0600 permissions on Unix; wrapped by OS Keystore
/// on Android via JNI and Keychain/SecretService on Desktop).
pub struct FileAuthStore {
    path: PathBuf,
}

impl FileAuthStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("keystore_session.json"),
        }
    }
}

impl AuthStore for FileAuthStore {
    fn load_session(&self) -> Result<Option<AuthSession>, AuthError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&self.path).map_err(|e| AuthError::Keystore(e.to_string()))?;
        let session = serde_json::from_slice::<AuthSession>(&bytes)
            .map_err(|e| AuthError::Keystore(format!("corrupt session store: {e}")))?;
        Ok(Some(session))
    }

    fn save_session(&self, session: &AuthSession) -> Result<(), AuthError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| AuthError::Keystore(e.to_string()))?;
        }
        let payload =
            serde_json::to_vec(session).map_err(|e| AuthError::Keystore(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp).map_err(|e| AuthError::Keystore(e.to_string()))?;
        file.write_all(&payload)
            .map_err(|e| AuthError::Keystore(e.to_string()))?;
        file.sync_all()
            .map_err(|e| AuthError::Keystore(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
        fs::rename(&tmp, &self.path).map_err(|e| AuthError::Keystore(e.to_string()))?;
        Ok(())
    }

    fn clear_session(&self) -> Result<(), AuthError> {
        if self.path.exists() {
            fs::remove_file(&self.path).map_err(|e| AuthError::Keystore(e.to_string()))?;
        }
        Ok(())
    }
}
