//! Deterministic `PlayerState` state machine frozen in Phase 0.
//!
//! Allowed lifecycle:
//! `Idle -> Loading -> Buffering -> Playing <-> Paused -> Recovering -> Error`

use serde::{Deserialize, Serialize};

use crate::{CoreError, CoreResult, QueueState, Track};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PlayerState {
    #[default]
    Idle,
    Loading,
    Buffering,
    Playing,
    Paused,
    Recovering,
    Error(String),
}

impl PlayerState {
    pub fn name(&self) -> &'static str {
        match self {
            PlayerState::Idle => "Idle",
            PlayerState::Loading => "Loading",
            PlayerState::Buffering => "Buffering",
            PlayerState::Playing => "Playing",
            PlayerState::Paused => "Paused",
            PlayerState::Recovering => "Recovering",
            PlayerState::Error(_) => "Error",
        }
    }

    pub fn is_playing(&self) -> bool {
        matches!(self, PlayerState::Playing)
    }

    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            PlayerState::Loading | PlayerState::Buffering | PlayerState::Recovering
        )
    }

    /// Strict deterministic state transition validation.
    pub fn can_transition_to(&self, next: &PlayerState) -> bool {
        use PlayerState::*;
        if std::mem::discriminant(self) == std::mem::discriminant(next) {
            return true;
        }
        matches!(
            (self, next),
            // Any state can reset to Idle, start Loading a new track, or enter Error.
            (_, Idle)
                | (_, Loading)
                | (_, Error(_))
                // Loading -> Buffering | Playing | Paused
                | (Loading, Buffering)
                | (Loading, Playing)
                | (Loading, Paused)
                // Buffering -> Playing | Paused | Recovering
                | (Buffering, Playing)
                | (Buffering, Paused)
                | (Buffering, Recovering)
                // Playing <-> Paused | Buffering | Recovering
                | (Playing, Paused)
                | (Playing, Buffering)
                | (Playing, Recovering)
                // Paused -> Playing | Buffering | Recovering
                | (Paused, Playing)
                | (Paused, Buffering)
                | (Paused, Recovering)
                // Recovering -> Buffering | Playing | Paused
                | (Recovering, Buffering)
                | (Recovering, Playing)
                | (Recovering, Paused)
        )
    }

    pub fn transition(&mut self, next: PlayerState) -> CoreResult<()> {
        if !self.can_transition_to(&next) {
            return Err(CoreError::InvalidTransition {
                from: self.name().to_owned(),
                to: next.name().to_owned(),
            });
        }
        *self = next;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub state: PlayerState,
    pub current_track: Option<Track>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub buffered_ms: u64,
    pub volume: f32,
    pub queue: QueueState,
    pub active_proxy_url: Option<String>,
    pub sponsorblock_enabled: bool,
    pub last_skipped_segment: Option<String>,
    pub offline_mode: bool,
}

impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            state: PlayerState::Idle,
            current_track: None,
            position_ms: 0,
            duration_ms: 0,
            buffered_ms: 0,
            volume: 1.0,
            queue: QueueState::default(),
            active_proxy_url: None,
            sponsorblock_enabled: true,
            last_skipped_segment: None,
            offline_mode: false,
        }
    }
}
