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
pub mod time_stretch;

pub use eq::{EQ_BAND_FREQS_HZ, EqPreset, TenBandEq};
pub use loudness::{DEFAULT_TARGET_OFFSET_DB, LoudnessNormalizer, db_to_linear};
pub use ring_buffer::{SpscConsumer, SpscProducer, SpscRingBuffer};
pub use silence::SilenceSkipper;
pub use splice::GaplessSplicer;
pub use time_stretch::WsolaTimeStretcher;

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
    volume_boost: f32,
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
            volume_boost: 1.0,
        }
    }

    /// Sets linear output gain boost (`1.0..=1.6`) for >100% volume amplification.
    pub fn set_volume_boost(&mut self, volume: f32) {
        self.volume_boost = volume.clamp(1.0, 1.6);
    }

    pub fn volume_boost(&self) -> f32 {
        self.volume_boost
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
        if !self.silence.process_interleaved(interleaved) {
            return false;
        }
        self.loudness.process_in_place(interleaved);
        let preamp = self.eq.preamp_linear();
        if preamp < 0.999 {
            for s in interleaved.iter_mut() {
                *s *= preamp;
            }
        }
        self.eq.process_interleaved(interleaved);
        if self.volume_boost > 1.0 + 1e-6 || self.eq.enabled {
            let boost = self.volume_boost;
            for s in interleaved.iter_mut() {
                *s = loudness::soft_limit(*s * boost);
            }
        }
        true
    }
}

/// Creates a lock-free cache-line-aligned SPSC PCM ring buffer for decoupling audio decoding
/// and DSP normalization from UI/network stalls.
pub fn create_pcm_ring_buffer(capacity_samples: usize) -> (SpscProducer<f32>, SpscConsumer<f32>) {
    SpscRingBuffer::with_capacity(capacity_samples)
}

/// Lock-free DSP parameter update sent from control/UI threads to [`LockFreeAudioConsumer`].
#[derive(Debug, Clone)]
pub enum DspControlUpdate {
    SetConfig(DspConfig),
    SetTrackLoudnessDb(Option<f32>),
    SetVolumeBoost(f32),
}

/// Real-time audio callback consumer reading decoded PCM frames from a lock-free SPSC ring
/// buffer and running a thread-owned [`DspPipeline`] with zero mutex locks or heap allocations.
pub struct LockFreeAudioConsumer {
    consumer: SpscConsumer<f32>,
    dsp: DspPipeline,
    updates: crossbeam_channel::Receiver<DspControlUpdate>,
}

impl LockFreeAudioConsumer {
    pub fn new(
        consumer: SpscConsumer<f32>,
        dsp: DspPipeline,
        updates: crossbeam_channel::Receiver<DspControlUpdate>,
    ) -> Self {
        Self {
            consumer,
            dsp,
            updates,
        }
    }

    /// Drains pending DSP updates without blocking, reads up to `out.len()` samples from the
    /// SPSC ring buffer, zero-fills any underrun tail, and processes the DSP chain in-place.
    /// Returns `(samples_popped, keep_block)` where `keep_block == false` indicates silence skip.
    pub fn render_callback(&mut self, out: &mut [f32]) -> (usize, bool) {
        while let Ok(upd) = self.updates.try_recv() {
            match upd {
                DspControlUpdate::SetConfig(cfg) => self.dsp.apply_config(&cfg),
                DspControlUpdate::SetTrackLoudnessDb(db) => self.dsp.set_track_loudness_db(db),
                DspControlUpdate::SetVolumeBoost(vol) => self.dsp.set_volume_boost(vol),
            }
        }
        let popped = self.consumer.pop_slice(out);
        if popped < out.len() {
            out[popped..].fill(0.0);
        }
        let keep = self.dsp.process_interleaved(out);
        (popped, keep)
    }
}

/// Creates a lock-free PCM producer, real-time [`LockFreeAudioConsumer`], and non-blocking DSP sender.
pub fn create_lock_free_audio_pipeline(
    capacity_samples: usize,
    sample_rate: u32,
    channels: usize,
) -> (
    SpscProducer<f32>,
    LockFreeAudioConsumer,
    crossbeam_channel::Sender<DspControlUpdate>,
) {
    let (prod, cons) = SpscRingBuffer::with_capacity::<f32>(capacity_samples);
    let (tx, rx) = crossbeam_channel::unbounded();
    let consumer = LockFreeAudioConsumer::new(
        cons,
        DspPipeline::new(sample_rate, channels, &DspConfig::default()),
        rx,
    );
    (prod, consumer, tx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_boost_above_100_percent_amplifies_and_soft_limits() {
        let mut dsp = DspPipeline::new(48_000, 2, &DspConfig::default());
        dsp.set_volume_boost(1.6);
        let mut samples = [0.25_f32, -0.25_f32, 0.8_f32, -0.8_f32];
        assert!(dsp.process_interleaved(&mut samples));
        assert!((samples[0] - 0.4).abs() < 1e-5);
        assert!((samples[1] + 0.4).abs() < 1e-5);
        assert!(samples[2] > 0.8 && samples[2] < 1.0);
        assert!(samples[3] < -0.8 && samples[3] > -1.0);
    }
}
