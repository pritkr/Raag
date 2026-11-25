//! Loopback-only HTTP PO Token provider compatible with `bgutil-ytdlp-pot-provider`
//! (`POST /get_pot` and `GET /ping`).
//!
//! Enforces strict SSRF loopback host validation (`127.0.0.1`, `::1`, `localhost`),
//! bounded 4s timeout, and <= 4 KB response cap.

use std::time::Duration;

use async_trait::async_trait;
use resonance_contract::{
    PoToken, PoTokenDiagnostics, PoTokenError, PoTokenHealthStatus, PoTokenProvider,
    PoTokenRequest, PoTokenScope,
};
use serde::Deserialize;
use serde_json::json;

use crate::unix_secs;

const SIDECAR_TIMEOUT_MS: u64 = 4_000;
const SIDECAR_MAX_BODY_BYTES: usize = 4 * 1024;
const DEFAULT_TOKEN_TTL_SECS: u64 = 6 * 3600;

pub(crate) fn is_loopback_host(host: &str) -> bool {
    let trimmed = host.trim_matches(|c| c == '[' || c == ']');
    trimmed == "127.0.0.1" || trimmed == "::1" || trimmed.eq_ignore_ascii_case("localhost")
}

pub(crate) fn validate_loopback_endpoint(endpoint: &str) -> Result<url::Url, PoTokenError> {
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
struct BgutilResponse {
    #[serde(alias = "po_token", alias = "token", alias = "value")]
    #[serde(rename = "poToken")]
    po_token: Option<String>,
    #[serde(alias = "expires_at_unix_secs")]
    #[serde(rename = "expiresAt")]
    expires_at: Option<u64>,
}

/// Loopback-only HTTP PO provider compatible with `bgutil-ytdlp-pot-provider`.
pub struct SidecarPoTokenProvider {
    endpoint: String,
    http: reqwest::Client,
    timeout_ms: u64,
}

/// Semantic alias matching `bgutil-ytdlp-pot-provider` naming.
pub type BgutilHttpPoTokenProvider = SidecarPoTokenProvider;

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

    /// Probes the `bgutil-ytdlp-pot-provider` `/ping` health endpoint on the loopback server.
    pub async fn ping(&self) -> bool {
        let Ok(mut url) = validate_loopback_endpoint(&self.endpoint) else {
            return false;
        };
        url.set_path("/ping");
        url.set_query(None);
        self.http
            .get(url)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
    }
}

#[async_trait]
impl PoTokenProvider for SidecarPoTokenProvider {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let mut post_url = validate_loopback_endpoint(&self.endpoint)?;
        if post_url.path().is_empty() || post_url.path() == "/" {
            post_url.set_path("/get_pot");
        }

        let (content_binding, context_str) = match req.scope {
            PoTokenScope::Player => (req.video_id.clone(), "player"),
            PoTokenScope::Subs => (req.video_id.clone(), "subs"),
            PoTokenScope::Gvs => (
                req.identity_key().unwrap_or(&req.video_id).to_owned(),
                "gvs",
            ),
        };

        let resp = self
            .http
            .post(post_url)
            .json(&json!({
                "content_binding": content_binding,
                "context": context_str,
                "client": req.client_name,
            }))
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

        let parsed: BgutilResponse = serde_json::from_slice(&body)
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
        "bgutil-http"
    }

    fn diagnostics(&self) -> PoTokenDiagnostics {
        PoTokenDiagnostics {
            status: PoTokenHealthStatus::Healthy,
            active_provider: self.provider_name().to_owned(),
            detail: format!("Local bgutil HTTP sidecar ({})", self.endpoint),
            degraded_fallback_active: false,
            last_checked_unix_secs: unix_secs(),
        }
    }
}
