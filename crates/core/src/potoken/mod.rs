//! PO Token (`PoTokenProvider`) implementations and `&pot=` stream URL attachment.
//!
//! Provides:
//! - [`NoopPoTokenProvider`] — default when no provider is configured
//! - [`StaticPoTokenProvider`] — in-memory `RwLock<Option<PoToken>>` provider
//! - [`SidecarPoTokenProvider`] — loopback-only HTTP sidecar (`127.0.0.1` / `::1` / `localhost`)
//! - [`CachingPoTokenProvider`] — single-flight deduplication + 60s expiry skew cache
//! - [`attach_pot_param`] — safe HTTPS/loopback `&pot=` query parameter attachment

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use resonance_contract::{
    CoreError, CoreResult, PoToken, PoTokenError, PoTokenProvider, PoTokenRequest, PoTokenScope,
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

use crate::unix_secs;

const SIDECAR_TIMEOUT_MS: u64 = 4_000;
const SIDECAR_MAX_BODY_BYTES: usize = 4 * 1024;
const CACHE_EXPIRY_SKEW_SECS: u64 = 60;
const DEFAULT_TOKEN_TTL_SECS: u64 = 6 * 3600;

/// Default provider when no PO token backend is configured.
#[derive(Debug, Default)]
pub struct NoopPoTokenProvider;

#[async_trait]
impl PoTokenProvider for NoopPoTokenProvider {
    async fn mint_token(&self, _req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        Err(PoTokenError::Unavailable("not configured".to_owned()))
    }

    fn invalidate(&self, _scope: PoTokenScope) {}

    fn provider_name(&self) -> &'static str {
        "noop"
    }
}

/// Static in-memory PO token provider backed by `RwLock<Option<PoToken>>`.
#[derive(Debug, Default)]
pub struct StaticPoTokenProvider {
    token: RwLock<Option<PoToken>>,
}

impl StaticPoTokenProvider {
    pub fn new(token: Option<PoToken>) -> Self {
        Self {
            token: RwLock::new(token),
        }
    }

    pub fn set(&self, token: Option<PoToken>) {
        *self.token.write().unwrap() = token;
    }
}

#[async_trait]
impl PoTokenProvider for StaticPoTokenProvider {
    async fn mint_token(&self, _req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let guard = self.token.read().unwrap();
        let Some(tok) = guard.as_ref() else {
            return Err(PoTokenError::Unavailable(
                "static po token not configured".to_owned(),
            ));
        };
        if tok.expires_at_unix_secs > 0 && tok.expires_at_unix_secs <= unix_secs() {
            return Err(PoTokenError::Unavailable(
                "static po token expired".to_owned(),
            ));
        }
        Ok(tok.clone())
    }

    fn invalidate(&self, scope: PoTokenScope) {
        let mut guard = self.token.write().unwrap();
        if guard.as_ref().is_some_and(|t| t.scope == scope) {
            *guard = None;
        }
    }

    fn provider_name(&self) -> &'static str {
        "static"
    }
}

fn is_loopback_host(host: &str) -> bool {
    let trimmed = host.trim_matches(|c| c == '[' || c == ']');
    trimmed == "127.0.0.1" || trimmed == "::1" || trimmed.eq_ignore_ascii_case("localhost")
}

fn validate_loopback_endpoint(endpoint: &str) -> Result<url::Url, PoTokenError> {
    let parsed = url::Url::parse(endpoint)
        .map_err(|e| PoTokenError::Unavailable(format!("invalid sidecar url: {e}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(PoTokenError::Unavailable(format!(
                "unsupported sidecar scheme: {other}"
            )));
        }
    }
    let Some(host) = parsed.host_str() else {
        return Err(PoTokenError::Unavailable("sidecar url missing host".into()));
    };
    if !is_loopback_host(host) {
        return Err(PoTokenError::Unavailable(format!(
            "non-loopback potoken sidecar host rejected: {host}"
        )));
    }
    Ok(parsed)
}

#[derive(Debug, Deserialize)]
struct SidecarResponse {
    #[serde(alias = "po_token", alias = "token", alias = "value")]
    #[serde(rename = "poToken")]
    po_token: Option<String>,
    #[serde(alias = "expires_at_unix_secs")]
    #[serde(rename = "expiresAt")]
    expires_at: Option<u64>,
}

/// Loopback-only HTTP sidecar PO token provider (`127.0.0.1` / `::1` / `localhost`).
///
/// Enforces strict SSRF host validation, 4s timeout, and <= 4 KB response body bound.
pub struct SidecarPoTokenProvider {
    endpoint: String,
    http: reqwest::Client,
    timeout_ms: u64,
}

impl SidecarPoTokenProvider {
    pub fn new(endpoint: impl Into<String>) -> Result<Self, PoTokenError> {
        let endpoint = endpoint.into();
        let _ = validate_loopback_endpoint(&endpoint)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(SIDECAR_TIMEOUT_MS))
            .build()
            .map_err(|e| PoTokenError::Unavailable(format!("http client init failed: {e}")))?;
        Ok(Self {
            endpoint,
            http,
            timeout_ms: SIDECAR_TIMEOUT_MS,
        })
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Result<Self, PoTokenError> {
        let clamped = timeout_ms.max(50);
        self.http = reqwest::Client::builder()
            .timeout(Duration::from_millis(clamped))
            .build()
            .map_err(|e| PoTokenError::Unavailable(format!("http client init failed: {e}")))?;
        self.timeout_ms = clamped;
        Ok(self)
    }
}

#[async_trait]
impl PoTokenProvider for SidecarPoTokenProvider {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        // Re-validate endpoint at call time for defense-in-depth against SSRF.
        let _ = validate_loopback_endpoint(&self.endpoint)?;

        let content_binding = match req.scope {
            PoTokenScope::Player => req.video_id.clone(),
            PoTokenScope::Gvs => req
                .visitor_data
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(&req.video_id)
                .to_owned(),
        };

        let resp = self
            .http
            .post(&self.endpoint)
            .json(&json!({ "content_binding": content_binding }))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    PoTokenError::Timeout {
                        timeout_ms: self.timeout_ms,
                    }
                } else {
                    PoTokenError::Unavailable(format!("sidecar request failed: {e}"))
                }
            })?;

        if !resp.status().is_success() {
            return Err(PoTokenError::MintFailed(format!(
                "sidecar returned HTTP {}",
                resp.status().as_u16()
            )));
        }

        if let Some(len) = resp.content_length()
            && len as usize > SIDECAR_MAX_BODY_BYTES
        {
            return Err(PoTokenError::MintFailed(
                "sidecar response exceeded 4 KB limit".to_owned(),
            ));
        }

        let body = resp
            .bytes()
            .await
            .map_err(|e| PoTokenError::MintFailed(format!("failed reading sidecar body: {e}")))?;
        if body.len() > SIDECAR_MAX_BODY_BYTES {
            return Err(PoTokenError::MintFailed(
                "sidecar response exceeded 4 KB limit".to_owned(),
            ));
        }

        let parsed: SidecarResponse = serde_json::from_slice(&body)
            .map_err(|e| PoTokenError::MintFailed(format!("invalid sidecar json: {e}")))?;
        let value = parsed
            .po_token
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| PoTokenError::MintFailed("sidecar response missing poToken".into()))?;
        let expires_at_unix_secs = parsed
            .expires_at
            .unwrap_or_else(|| unix_secs() + DEFAULT_TOKEN_TTL_SECS);

        Ok(PoToken {
            value,
            scope: req.scope,
            expires_at_unix_secs,
        })
    }

    fn invalidate(&self, _scope: PoTokenScope) {}

    fn provider_name(&self) -> &'static str {
        "sidecar"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    scope: PoTokenScope,
    client_name: String,
    visitor_data: Option<String>,
    signed_in: bool,
    player_video_id: Option<String>,
}

impl CacheKey {
    fn from_request(req: &PoTokenRequest) -> Self {
        Self {
            scope: req.scope,
            client_name: req.client_name.clone(),
            visitor_data: req.visitor_data.clone(),
            signed_in: req.signed_in,
            player_video_id: match req.scope {
                PoTokenScope::Player => Some(req.video_id.clone()),
                PoTokenScope::Gvs => None,
            },
        }
    }
}

/// Caching + single-flight deduplicating wrapper around any [`PoTokenProvider`].
///
/// Caches `Gvs` tokens by `(client_name, visitor_data, signed_in)` with a 60s expiry
/// skew margin and serializes concurrent cold mints via an async mutex.
pub struct CachingPoTokenProvider<P: PoTokenProvider> {
    inner: P,
    cache: RwLock<HashMap<CacheKey, PoToken>>,
    mint_lock: Mutex<()>,
}

impl<P: PoTokenProvider> CachingPoTokenProvider<P> {
    pub fn new(inner: P) -> Self {
        Self {
            inner,
            cache: RwLock::new(HashMap::new()),
            mint_lock: Mutex::new(()),
        }
    }

    pub fn into_arc(self) -> Arc<Self> {
        Arc::new(self)
    }

    fn lookup_fresh(&self, key: &CacheKey) -> Option<PoToken> {
        let now = unix_secs();
        let guard = self.cache.read().unwrap();
        guard
            .get(key)
            .filter(|tok| tok.expires_at_unix_secs > now.saturating_add(CACHE_EXPIRY_SKEW_SECS))
            .cloned()
    }
}

#[async_trait]
impl<P: PoTokenProvider> PoTokenProvider for CachingPoTokenProvider<P> {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let key = CacheKey::from_request(req);
        if let Some(cached) = self.lookup_fresh(&key) {
            return Ok(cached);
        }

        // Single-flight deduplication: serialize cold mints so concurrent callers
        // reuse the newly minted token instead of spawning duplicate mints.
        let _guard = self.mint_lock.lock().await;
        if let Some(cached) = self.lookup_fresh(&key) {
            return Ok(cached);
        }

        let minted = self.inner.mint_token(req).await?;
        self.cache.write().unwrap().insert(key, minted.clone());
        Ok(minted)
    }

    fn invalidate(&self, scope: PoTokenScope) {
        self.cache
            .write()
            .unwrap()
            .retain(|k, _| k.scope != scope);
        self.inner.invalidate(scope);
    }

    fn provider_name(&self) -> &'static str {
        self.inner.provider_name()
    }
}

/// Attaches or replaces `&pot=<token>` on a resolved media stream URL.
///
/// Rejects non-HTTPS URLs (except loopback `http://127.0.0.1` / `localhost` / `[::1]`
/// for offline mock tests) and preserves all existing query parameters (`sig`, `n`, `expire`).
pub fn attach_pot_param(stream_url: &str, po_token: &str) -> CoreResult<String> {
    if po_token.trim().is_empty() {
        return Err(CoreError::PoToken(PoTokenError::MintFailed(
            "cannot attach empty poToken".to_owned(),
        )));
    }
    let mut parsed = url::Url::parse(stream_url).map_err(|e| {
        CoreError::PoToken(PoTokenError::MintFailed(format!(
            "invalid stream url for pot attachment: {e}"
        )))
    })?;

    let is_https = parsed.scheme() == "https";
    let is_loopback_http = parsed.scheme() == "http"
        && parsed.host_str().is_some_and(is_loopback_host);
    if !is_https && !is_loopback_http {
        return Err(CoreError::PoToken(PoTokenError::MintFailed(format!(
            "refused to attach pot param to non-HTTPS non-loopback URL scheme {}",
            parsed.scheme()
        ))));
    }

    let existing_pot = parsed
        .query_pairs()
        .find(|(k, _)| k == "pot")
        .map(|(_, v)| v.into_owned());
    if existing_pot.as_deref() == Some(po_token) {
        return Ok(parsed.to_string());
    }

    let preserved: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| k != "pot")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    {
        let mut qp = parsed.query_pairs_mut();
        qp.clear();
        for (k, v) in preserved {
            qp.append_pair(&k, &v);
        }
        qp.append_pair("pot", po_token);
    }
    Ok(parsed.to_string())
}
