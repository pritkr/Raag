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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PlaylistPrivacy {
    #[default]
    Private,
    Unlisted,
    Public,
}

impl PlaylistPrivacy {
    pub fn as_innertube_str(self) -> &'static str {
        match self {
            Self::Private => "PRIVATE",
            Self::Unlisted => "UNLISTED",
            Self::Public => "PUBLIC",
        }
    }

    pub fn from_label(label: &str) -> Self {
        match label.trim().to_ascii_lowercase().as_str() {
            "public" => Self::Public,
            "unlisted" => Self::Unlisted,
            _ => Self::Private,
        }
    }
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
    Rename {
        title: String,
        #[serde(default)]
        description: Option<String>,
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
    CreatePlaylist {
        local_temp_id: String,
        title: String,
        description: String,
        privacy: PlaylistPrivacy,
        initial_video_ids: Vec<TrackId>,
    },
    DeletePlaylist {
        playlist_id: String,
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

    async fn create_playlist(
        &self,
        _title: &str,
        _description: &str,
        _privacy: PlaylistPrivacy,
        _initial_video_ids: &[TrackId],
    ) -> Result<MutationReceipt, MutationError> {
        Err(MutationError::InvalidInput(
            "create_playlist not implemented by provider".to_owned(),
        ))
    }

    async fn delete_playlist(&self, _playlist_id: &str) -> Result<MutationReceipt, MutationError> {
        Err(MutationError::InvalidInput(
            "delete_playlist not implemented by provider".to_owned(),
        ))
    }
}
