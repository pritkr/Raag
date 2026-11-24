//! Application EventBus and MediaCommand primitives.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{CacheStats, LyricsDocument, PlayerSnapshot, TrackId};

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
