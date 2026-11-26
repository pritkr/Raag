//! Frozen audio extension contracts for Wave 1D, Wave 2A, and Wave 3B.

use serde::{Deserialize, Serialize};

use crate::domain::{AudioCodec, TrackId};
use crate::state::PlayerState;

/// Truthful capability reporting for the audio DSP & PCM pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioDspCapability {
    #[default]
    ExternalUrlStreamOnly,
    NativePcmPipeline,
    AndroidAudioProcessorJni,
}

/// Commands dispatched from `PlaybackController` / UI to the native audio backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AudioControlCommand {
    LoadAndPlay {
        video_id: TrackId,
        proxy_url: String,
        start_ms: u64,
    },
    Pause,
    Resume,
    SeekMs(u64),
    SetVolume(f32),
    SetSpeed {
        speed: f32,
        preserve_pitch: bool,
    },
    Stop,
}

/// Telemetry events emitted from the native audio backend back to `PlaybackController`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AudioTelemetryEvent {
    StateChanged(PlayerState),
    Buffering(bool),
    Progress {
        position_ms: u64,
        duration_ms: Option<u64>,
    },
    DurationDiscovered(u64),
    Ended,
    Error(String),
}

/// Sleep timer state for end-of-track or countdown auto-pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SleepTimerState {
    #[default]
    Off,
    Countdown {
        remaining_secs: u64,
    },
    EndOfTrack,
}

/// Playback speed and pitch-preservation settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlaybackSpeedConfig {
    pub speed: f32,
    pub preserve_pitch: bool,
}

impl Default for PlaybackSpeedConfig {
    fn default() -> Self {
        Self {
            speed: 1.0,
            preserve_pitch: true,
        }
    }
}

/// Truthful codec & bitrate badge info for the Now Playing surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecBadgeInfo {
    pub codec: AudioCodec,
    pub bitrate_kbps: u32,
    pub sample_rate_hz: Option<u32>,
    pub source_label: String,
}
