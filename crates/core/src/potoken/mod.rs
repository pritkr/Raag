//! Layered PO Token (`PoTokenProvider`) acquisition pipeline and `&pot=` URL attachment.
//!
//! Modules:
//! - [`bgutil`] — External/Local HTTP PO provider (`bgutil-ytdlp-pot-provider` `/get_pot` + `/ping`)
//! - [`attestation`] — Embedded WebView / Platform Attestation Bridge + QuickJS-NG VM fallback
//! - [`chain`] — Layered [`ChainedPoTokenProvider`] with automatic failover & degraded diagnostics
//! - [`cache`] — TTL-aware [`CachingPoTokenProvider`] keyed by `(visitor_data | session_hash, video_id, scope)`

pub mod attestation;
pub mod bgutil;
pub mod cache;
pub mod chain;
pub mod rustypipe_bg;
pub mod webview_attestation;

use std::sync::RwLock;

use async_trait::async_trait;
use resonance_contract::{
    CoreError, CoreResult, PoToken, PoTokenDiagnostics, PoTokenError, PoTokenHealthStatus,
    PoTokenProvider, PoTokenRequest, PoTokenScope,
};

use crate::unix_secs;

pub use attestation::{
    PlatformAttestationBridge, PlatformAttestationPoTokenProvider, QuickJsVmAttestationBridge,
};
pub use bgutil::{BgutilHttpPoTokenProvider, SidecarPoTokenProvider};
pub type BgutilHttpProvider = BgutilHttpPoTokenProvider;
pub use cache::CachingPoTokenProvider;
pub use chain::ChainedPoTokenProvider;
pub use rustypipe_bg::{RustyPipeBotguardProvider, validate_botguard_binding};
pub use webview_attestation::{
    AttestationOrchestrator, BotguardChallenge, BotguardExecutionOutput, CachedIntegrityToken,
    EmbeddedWebViewAttestationProvider, WebViewBotguardRunner, validate_interpreter_url,
};

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

    fn diagnostics(&self) -> PoTokenDiagnostics {
        PoTokenDiagnostics {
            status: PoTokenHealthStatus::Unconfigured,
            active_provider: "noop".to_owned(),
            detail: "No PO token provider configured".to_owned(),
            degraded_fallback_active: true,
            last_checked_unix_secs: unix_secs(),
        }
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
    let is_loopback_http =
        parsed.scheme() == "http" && parsed.host_str().is_some_and(bgutil::is_loopback_host);
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
