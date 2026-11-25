//! True Gapless & Modular PCM DSP Pipeline (`crates/core/src/audio_dsp/`).
//!
//! Components:
//! - [`ring_buffer`]: Lock-free cache-line-aligned SPSC ring buffer (`SpscRingBuffer`)
//! - [`loudness`]: ReplayGain / EBU R128 loudness normalization from YouTube `loudnessDb`
//! - [`eq`]: 10-band parametric biquad EQ (ISO 31 Hz – 16 kHz)
//! - [`silence`]: RMS dBFS silence skipper
//! - [`splice`]: Sample-accurate gapless splicing & equal-power crossfade

pub mod eq;
pub mod loudness;
pub mod ring_buffer;
pub mod silence;
pub mod splice;

pub use eq::{EQ_BAND_FREQS_HZ, EqPreset, TenBandEq};
pub use loudness::{DEFAULT_TARGET_OFFSET_DB, LoudnessNormalizer, db_to_linear};
pub use ring_buffer::{SpscConsumer, SpscProducer, SpscRingBuffer};
pub use silence::SilenceSkipper;
pub use splice::GaplessSplicer;

/// Serializable user-facing DSP settings controlled from Settings and synced to `platform/nsh_audio.rs`.
#[derive(Debug, Clone, PartialEq)]
pub struct DspConfig {
    pub loudness_normalization: bool,
    pub eq_preset: EqPreset,
    pub eq_gains_db: [f32; 10],
    pub silence_skip_enabled: bool,
    pub crossfade_ms: u32,
}

impl Default for DspConfig {
    fn default() -> Self {
        Self {
            loudness_normalization: true,
            eq_preset: EqPreset::Flat,
            eq_gains_db: [0.0; 10],
            silence_skip_enabled: false,
            crossfade_ms: 0,
        }
    }
}

/// Composed zero-allocation real-time PCM DSP stage.
pub struct DspPipeline {
    pub loudness: LoudnessNormalizer,
    pub eq: TenBandEq,
    pub silence: SilenceSkipper,
    pub splicer: GaplessSplicer,
}

impl DspPipeline {
    pub fn new(sample_rate: u32, channels: usize, config: &DspConfig) -> Self {
        let mut eq = TenBandEq::new(sample_rate, channels);
        eq.enabled = config.eq_preset != EqPreset::Flat
            || config.eq_gains_db.iter().any(|g| g.abs() >= 0.01);
        eq.set_gains_db(config.eq_gains_db);

        let mut silence = SilenceSkipper::new(sample_rate, channels, -45.0, 350);
        silence.enabled = config.silence_skip_enabled;

        Self {
            loudness: LoudnessNormalizer::new(
                config.loudness_normalization,
                DEFAULT_TARGET_OFFSET_DB,
                0.0,
            ),
            eq,
            silence,
            splicer: GaplessSplicer::new(sample_rate, channels, config.crossfade_ms),
        }
    }

    pub fn apply_config(&mut self, config: &DspConfig) {
        self.loudness.enabled = config.loudness_normalization;
        self.eq.enabled = config.eq_preset != EqPreset::Flat
            || config.eq_gains_db.iter().any(|g| g.abs() >= 0.01);
        self.eq.set_gains_db(config.eq_gains_db);
        self.silence.enabled = config.silence_skip_enabled;
        self.splicer.crossfade_ms = config.crossfade_ms;
    }

    pub fn set_track_loudness_db(&mut self, loudness_db: Option<f32>) {
        self.loudness.set_track_loudness_db(loudness_db);
    }

    /// Processes an interleaved PCM block in-place.
    /// Returns `true` if the block should be played, or `false` if skipped by `SilenceSkipper`.
    pub fn process_interleaved(&mut self, interleaved: &mut [f32]) -> bool {
        if self.silence.should_skip_block(interleaved) {
            return false;
        }
        self.loudness.process_in_place(interleaved);
        self.eq.process_interleaved(interleaved);
        true
    }
}
