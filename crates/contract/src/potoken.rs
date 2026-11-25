use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PoTokenScope {
    /// GVS (Google Video Server) token bound to `visitorData` (anonymous) or session hash (signed-in).
    Gvs,
    /// Player / playback-minted token bound to `video_id`.
    Player,
    /// Subtitles / captions endpoint PO token bound to `video_id` or session context.
    Subs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoTokenRequest {
    pub client_name: String,
    pub scope: PoTokenScope,
    pub video_id: String,
    pub visitor_data: Option<String>,
    /// Optional hashed session identity (e.g. SHA-256 of `dataSyncId` / SAPISID)
    /// used to key TTL cache entries separately per signed-in session.
    pub session_hash: Option<String>,
    pub signed_in: bool,
}

impl PoTokenRequest {
    pub fn new(
        client_name: impl Into<String>,
        scope: PoTokenScope,
        video_id: impl Into<String>,
        visitor_data: Option<String>,
        signed_in: bool,
    ) -> Self {
        Self {
            client_name: client_name.into(),
            scope,
            video_id: video_id.into(),
            visitor_data,
            session_hash: None,
            signed_in,
        }
    }

    pub fn with_session_hash(mut self, session_hash: Option<String>) -> Self {
        self.session_hash = session_hash;
        self
    }

    /// Canonical session/visitor binding identity for TTL cache keying:
    /// prefers `session_hash` when signed in, falling back to `visitor_data`.
    pub fn identity_key(&self) -> Option<&str> {
        if self.signed_in {
            self.session_hash
                .as_deref()
                .filter(|s| !s.is_empty())
                .or_else(|| self.visitor_data.as_deref().filter(|s| !s.is_empty()))
        } else {
            self.visitor_data
                .as_deref()
                .filter(|s| !s.is_empty())
                .or_else(|| self.session_hash.as_deref().filter(|s| !s.is_empty()))
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoToken {
    pub value: String,
    pub scope: PoTokenScope,
    pub expires_at_unix_secs: u64,
}

impl fmt::Debug for PoToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PoToken")
            .field("value", &"[REDACTED_POT]")
            .field("scope", &self.scope)
            .field("expires_at_unix_secs", &self.expires_at_unix_secs)
            .finish()
    }
}

/// Non-blocking operational state surfaced in Settings diagnostics badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PoTokenHealthStatus {
    /// Active provider chain is minting valid PO tokens normally.
    Healthy,
    /// All PO token providers failed or timed out; running on lower-enforcement
    /// fallback clients in `client_matrix.json`.
    DegradedColdStart,
    /// No external PO token provider configured yet (anonymous / fallback clients active).
    #[default]
    Unconfigured,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PoTokenDiagnostics {
    pub status: PoTokenHealthStatus,
    pub active_provider: String,
    pub detail: String,
    pub degraded_fallback_active: bool,
    pub last_checked_unix_secs: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PoTokenError {
    #[error("po token provider unavailable: {0}")]
    Unavailable(String),
    #[error("po token mint timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("po token mint failed: {0}")]
    MintFailed(String),
    #[error("po token forbidden for client {client_name} ({reason})")]
    PolicyForbidden { client_name: String, reason: String },
}

#[async_trait]
pub trait PoTokenProvider: Send + Sync {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError>;
    fn invalidate(&self, scope: PoTokenScope);
    fn provider_name(&self) -> &'static str;

    fn diagnostics(&self) -> PoTokenDiagnostics {
        PoTokenDiagnostics {
            status: PoTokenHealthStatus::Healthy,
            active_provider: self.provider_name().to_owned(),
            detail: format!("Active provider: {}", self.provider_name()),
            degraded_fallback_active: false,
            last_checked_unix_secs: 0,
        }
    }
}
