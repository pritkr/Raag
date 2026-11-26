//! Dual-Slot Gapless & Equal-Power Crossfade Mixer (`crates/core/src/pcm_engine/mixer.rs`).

use crate::audio_dsp::GaplessSplicer;
use crate::pcm_engine::decoder::DecodedPcmTrack;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerTransition {
    GaplessSplice,
    EqualPowerCrossfade { crossfade_ms: u32 },
}

/// Dual-slot (Active Slot A + Pre-staged Slot B) mixer for sample-accurate gapless
/// concatenation (`crossfade_ms == 0`) and constant-loudness equal-power crossfades.
pub struct DualSlotMixer {
    sample_rate: u32,
    channels: usize,
    crossfade_ms: u32,
    slot_a: Option<DecodedPcmTrack>,
    slot_b: Option<DecodedPcmTrack>,
}

impl DualSlotMixer {
    pub fn new(sample_rate: u32, channels: usize, crossfade_ms: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(8_000),
            channels: channels.max(1),
            crossfade_ms,
            slot_a: None,
            slot_b: None,
        }
    }

    pub fn set_crossfade_ms(&mut self, crossfade_ms: u32) {
        self.crossfade_ms = crossfade_ms;
    }

    pub fn load_slot_a(&mut self, track: DecodedPcmTrack) {
        self.slot_a = Some(track);
    }

    pub fn stage_slot_b(&mut self, track: DecodedPcmTrack) {
        self.slot_b = Some(track);
    }

    pub fn has_slot_b(&self) -> bool {
        self.slot_b.is_some()
    }

    /// Transitions Slot A -> Slot B using `GaplessSplicer` (0-sample splice or equal-power
    /// `cos/sin` crossfade) and promotes Slot B to Slot A.
    pub fn transition_a_to_b(&mut self) -> Option<(Vec<f32>, MixerTransition)> {
        let a = self.slot_a.take()?;
        let b = self.slot_b.take()?;
        let effective_xfade = if self.crossfade_ms == 0 {
            0
        } else {
            let max_ms = (a.duration_ms() / 2).min(b.duration_ms() / 2) as u32;
            self.crossfade_ms.min(max_ms)
        };
        let splicer = GaplessSplicer::new(self.sample_rate, self.channels, effective_xfade);
        let mixed = splicer.splice_interleaved(a.trimmed_samples(), b.trimmed_samples());
        let kind = if effective_xfade == 0 {
            MixerTransition::GaplessSplice
        } else {
            MixerTransition::EqualPowerCrossfade {
                crossfade_ms: effective_xfade,
            }
        };
        self.slot_a = Some(b);
        Some((mixed, kind))
    }
}
