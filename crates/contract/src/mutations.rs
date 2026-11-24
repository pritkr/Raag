use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::TrackId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LikeAction {
    Like,
    Dislike,
    RemoveLike,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistEditOp {
    AddVideo {
        video_id: TrackId,
    },
    RemoveVideo {
        video_id: TrackId,
        set_video_id: String,
    },
    MoveVideoBefore {
        set_video_id: String,
        predecessor_set_video_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchtimeReport {
    pub video_id: TrackId,
    /// Client Playback Nonce (16-char base64url) stable per playback session.
    pub cpn: String,
    pub start_secs: u64,
    pub end_secs: u64,
    pub duration_secs: u64,
    pub is_final: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationCommand {
    ReportAtr(WatchtimeReport),
    SetLike {
        video_id: TrackId,
        action: LikeAction,
    },
    EditPlaylist {
        playlist_id: String,
        op: PlaylistEditOp,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationReceipt {
    pub idempotency_key: String,
    pub endpoint: String,
    pub server_status: Option<String>,
    pub executed_at_unix_secs: u64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MutationError {
    #[error("authentication required for write mutation")]
    NotAuthenticated,
    #[error("invalid mutation payload: {0}")]
    InvalidInput(String),
    #[error("rate limited (429), retry after {retry_after_secs:?}s")]
    RateLimited { retry_after_secs: Option<u64> },
    #[error("mutation queue full (capacity {capacity})")]
    QueueFull { capacity: usize },
    #[error("http error {status}: {message}")]
    Http { status: u16, message: String },
    #[error("network error: {0}")]
    Network(String),
}

#[async_trait]
pub trait InnerTubeMutations: Send + Sync {
    async fn report_atr(&self, report: &WatchtimeReport) -> Result<MutationReceipt, MutationError>;
    async fn set_like(
        &self,
        video_id: &str,
        action: LikeAction,
    ) -> Result<MutationReceipt, MutationError>;
    async fn edit_playlist(
        &self,
        playlist_id: &str,
        op: &PlaylistEditOp,
    ) -> Result<MutationReceipt, MutationError>;
}
