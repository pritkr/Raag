use std::fmt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PoTokenScope {
    /// GVS (Google Video Server) token bound to `visitorData` (anonymous) or session `dataSyncId` (free signed-in).
    Gvs,
    /// Player / playback-minted token bound to `video_id`.
    Player,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoTokenRequest {
    pub client_name: String,
    pub scope: PoTokenScope,
    pub video_id: String,
    pub visitor_data: Option<String>,
    pub signed_in: bool,
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
}
