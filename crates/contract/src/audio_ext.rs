//! Frozen audio extension contracts for Wave 1D, Wave 2A, and Wave 3B.

use serde::{Deserialize, Serialize};

use crate::domain::AudioCodec;

/// Truthful capability reporting for the audio DSP & PCM pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioDspCapability {
    #[default]
    ExternalUrlStreamOnly,
    NativePcmPipeline,
    AndroidAudioProcessorJni,
}

impl AudioDspCapability {
    pub fn is_active(self) -> bool {
        matches!(self, Self::NativePcmPipeline | Self::AndroidAudioProcessorJni)
    }

    pub fn badge_label(self) -> &'static str {
        match self {
            Self::NativePcmPipeline => "Active • Rust PCM DSP",
            Self::AndroidAudioProcessorJni => "Active • Android AudioProcessor",
            Self::ExternalUrlStreamOnly => "Planned • Native PCM Engine",
        }
    }

    pub fn detail_text(self) -> &'static str {
        match self {
            Self::NativePcmPipeline | Self::AndroidAudioProcessorJni => {
                "32-bit float PCM pipeline active for ReplayGain, 10-band EQ, silence skip, and crossfade."
            }
            Self::ExternalUrlStreamOnly => {
                "nsh/audio streams directly to the OS decoder. Full PCM DSP (EQ, ReplayGain, silence skip, crossfade) is gated until the custom Rust PCM sink lands."
            }
        }
    }
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
