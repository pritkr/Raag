//! Application EventBus and MediaCommand primitives.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    CacheStats, DownloadRecord, LyricsDocument, MutationCommand, MutationReceipt, PlayerSnapshot,
    PlayerState, TrackId,
};

/// Unified telemetry emitted by any platform [`AudioBackend`] back to `PlaybackController`
/// for Single Source-of-Truth playback state updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioTelemetry {
    Loaded {
        generation: u64,
        duration_ms: u64,
    },
    StateChanged {
        generation: u64,
        state: PlayerState,
    },
    Buffering {
        generation: u64,
        buffering: bool,
    },
    Progress {
        generation: u64,
        position_ms: u64,
        duration_ms: Option<u64>,
    },
    Ended {
        generation: u64,
    },
    Failed {
        generation: u64,
        message: String,
    },
}

impl AudioTelemetry {
    pub fn generation(&self) -> u64 {
        match self {
            Self::Loaded { generation, .. }
            | Self::StateChanged { generation, .. }
            | Self::Buffering { generation, .. }
            | Self::Progress { generation, .. }
            | Self::Ended { generation }
            | Self::Failed { generation, .. } => *generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaCommand {
    Play,
    Pause,
    TogglePlayPause,
    Next,
    Previous,
    SeekMs(u64),
    Stop,
    AudioFocusLostTransient,
    AudioFocusGained,
    BecomingNoisy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AppEvent {
    PlayerUpdated(PlayerSnapshot),
    LyricsLoaded {
        video_id: TrackId,
        lyrics: Option<LyricsDocument>,
    },
    AuthStateChanged {
        signed_in: bool,
        account_name: Option<String>,
    },
    CacheStatsUpdated(CacheStats),
    SponsorBlockSkipped {
        video_id: TrackId,
        from_ms: u64,
        to_ms: u64,
    },
    Toast {
        message: String,
        is_error: bool,
    },
    DownloadUpdated(DownloadRecord),
    DownloadRemoved {
        video_id: TrackId,
    },
    MutationCompleted(MutationReceipt),
    MutationFailed {
        command: MutationCommand,
        reason: String,
    },
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(32));
        Self { tx }
    }

    pub fn publish(&self, event: AppEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(256)
    }
}
