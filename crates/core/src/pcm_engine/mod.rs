//! Pure-Rust Native PCM Audio Engine (`crates/core/src/pcm_engine/`).
//!
//! Decouples source reading, dual-slot gapless/crossfade mixing, WSOLA time-stretching,
//! modular `DspPipeline`, slew-capable resampling, and lock-free `SpscRingBuffer` output.

pub mod decoder;
pub mod engine;
pub mod mixer;
pub mod resampler;
pub mod source;

pub use decoder::{DecodedPcmTrack, PcmDecoder, WavDecoder};
pub use engine::{PcmEngine, PcmEngineCommand, PcmEngineConfig};
pub use mixer::{DualSlotMixer, MixerTransition};
pub use resampler::{MAX_SLEW_RATIO, MIN_SLEW_RATIO, SlewResampler};
pub use source::{PcmMediaSource, SeekableReadAheadSource};
