//! Desktop Native PCM Engine Orchestrator (`crates/core/src/pcm_engine/engine.rs`).
//!
//! Connects [`DualSlotMixer`], [`WsolaTimeStretcher`], [`DspPipeline`], [`SlewResampler`],
//! and a lock-free [`SpscRingBuffer`] so the real-time audio consumer never allocates or locks.

use crate::audio_dsp::{
    DspConfig, DspPipeline, SpscConsumer, SpscProducer, SpscRingBuffer, WsolaTimeStretcher,
};
use crate::pcm_engine::decoder::DecodedPcmTrack;
use crate::pcm_engine::mixer::{DualSlotMixer, MixerTransition};
use crate::pcm_engine::resampler::SlewResampler;

#[derive(Debug, Clone)]
pub struct PcmEngineConfig {
    pub sample_rate: u32,
    pub device_sample_rate: u32,
    pub channels: usize,
    pub ring_capacity_samples: usize,
}

impl Default for PcmEngineConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            device_sample_rate: 48_000,
            channels: 2,
            ring_capacity_samples: 48_000 * 2, // ~1s stereo buffer
        }
    }
}

#[derive(Debug, Clone)]
pub enum PcmEngineCommand {
    UpdateDsp(DspConfig),
    SetSpeed { speed: f32, preserve_pitch: bool },
    SetTrackLoudnessDb(Option<f32>),
    SetVolumeBoost(f32),
    SetSlewRatio(f64),
}

pub struct PcmEngine {
    mixer: DualSlotMixer,
    stretcher: WsolaTimeStretcher,
    dsp: DspPipeline,
    resampler: SlewResampler,
    producer: SpscProducer<f32>,
}

impl PcmEngine {
    pub fn new(config: PcmEngineConfig, dsp_config: &DspConfig) -> (Self, SpscConsumer<f32>) {
        let (producer, consumer) =
            SpscRingBuffer::with_capacity::<f32>(config.ring_capacity_samples);
        let engine = Self {
            mixer: DualSlotMixer::new(config.sample_rate, config.channels, dsp_config.crossfade_ms),
            stretcher: WsolaTimeStretcher::new(config.sample_rate, config.channels),
            dsp: DspPipeline::new(config.sample_rate, config.channels, dsp_config),
            resampler: SlewResampler::new(
                config.sample_rate,
                config.device_sample_rate,
                config.channels,
            ),
            producer,
        };
        (engine, consumer)
    }

    pub fn apply_command(&mut self, cmd: PcmEngineCommand) {
        match cmd {
            PcmEngineCommand::UpdateDsp(cfg) => {
                self.mixer.set_crossfade_ms(cfg.crossfade_ms);
                self.dsp.apply_config(&cfg);
            }
            PcmEngineCommand::SetSpeed {
                speed,
                preserve_pitch,
            } => {
                self.stretcher.set_speed(speed, preserve_pitch);
            }
            PcmEngineCommand::SetTrackLoudnessDb(db) => {
                self.dsp.set_track_loudness_db(db);
            }
            PcmEngineCommand::SetVolumeBoost(boost) => {
                self.dsp.set_volume_boost(boost);
            }
            PcmEngineCommand::SetSlewRatio(ratio) => {
                self.resampler.set_slew_ratio(ratio);
            }
        }
    }

    pub fn load_track_a(&mut self, track: DecodedPcmTrack) {
        self.mixer.load_slot_a(track);
    }

    pub fn stage_track_b(&mut self, track: DecodedPcmTrack) {
        self.mixer.stage_slot_b(track);
    }

    pub fn transition_and_push(&mut self) -> Option<(usize, MixerTransition)> {
        let (mixed, transition) = self.mixer.transition_a_to_b()?;
        let pushed = self.process_and_push(&mixed);
        Some((pushed, transition))
    }

    /// Runs WSOLA -> DspPipeline -> SlewResampler and pushes into the lock-free SPSC ring buffer.
    /// Returns the number of samples written (0 if block was silence-skipped).
    pub fn process_and_push(&mut self, interleaved: &[f32]) -> usize {
        let stretched = self.stretcher.process_interleaved(interleaved);
        let mut processed = stretched;
        if !self.dsp.process_interleaved(&mut processed) {
            return 0;
        }
        let resampled = self.resampler.process_interleaved(&processed);
        self.producer.push_slice(&resampled)
    }
}
