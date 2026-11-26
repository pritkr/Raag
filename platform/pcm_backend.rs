//! Desktop Native PCM `AudioBackend` (`platform/pcm_backend.rs`).
//!
//! Real-Time Audio Thread Contract (`RtAudioRenderCallback::render`):
//! - ZERO heap allocations, ZERO mutex locks, ZERO syscalls, ZERO logging.
//! - Communicates exclusively via lock-free atomics and `SpscConsumer<f32>`.
//! - Applies a 5 ms exponential de-click ramp on play/pause/seek transitions and
//!   64-frame fade-to-zero on ring-buffer underruns.

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use resonance_contract::{AudioBackend, CoreResult, MediaCommand, NowPlayingMetadata};
use resonance_core::audio_dsp::{DspConfig, SpscConsumer};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::platform::desktop::SystemMediaControlsBridge;

/// Lock-free real-time render callback state for OS audio output (`cpal` / CoreAudio / WASAPI / PipeWire).
pub struct RtAudioRenderCallback {
    consumer: SpscConsumer<f32>,
    channels: usize,
    sample_rate: u32,
    playing: Arc<AtomicBool>,
    target_gain_bits: Arc<AtomicU64>,
    frames_played: Arc<AtomicU64>,
    current_gain: f32,
    ramp_coeff: f32,
}

impl RtAudioRenderCallback {
    pub fn new(
        consumer: SpscConsumer<f32>,
        sample_rate: u32,
        channels: usize,
        playing: Arc<AtomicBool>,
        target_gain_bits: Arc<AtomicU64>,
        frames_played: Arc<AtomicU64>,
    ) -> Self {
        let sr = sample_rate.max(8_000) as f32;
        // 5 ms exponential de-click time constant
        let tau_samples = (0.005 * sr).max(1.0);
        let ramp_coeff = 1.0 - (-1.0 / tau_samples).exp();
        Self {
            consumer,
            channels: channels.max(1),
            sample_rate: sample_rate.max(8_000),
            playing,
            target_gain_bits,
            frames_played,
            current_gain: 0.0,
            ramp_coeff,
        }
    }

    /// Real-time audio callback entry point.
    ///
    /// Strictly lock-free, allocation-free, syscall-free, and log-free.
    #[inline]
    pub fn render(&mut self, out: &mut [f32]) {
        let is_playing = self.playing.load(Ordering::Relaxed);
        let target_gain = if is_playing {
            f32::from_bits(self.target_gain_bits.load(Ordering::Relaxed) as u32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        if !is_playing && self.current_gain < 1e-5 {
            out.fill(0.0);
            self.current_gain = 0.0;
            return;
        }

        let popped = self.consumer.pop_with_underrun_fade(out, self.channels);
        let ch = self.channels;
        for frame in out.chunks_exact_mut(ch) {
            self.current_gain += (target_gain - self.current_gain) * self.ramp_coeff;
            let g = self.current_gain;
            for s in frame.iter_mut() {
                *s *= g;
            }
        }

        let played = (popped / ch) as u64;
        if played > 0 {
            self.frames_played.fetch_add(played, Ordering::Relaxed);
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Desktop Native PCM `AudioBackend` integrating `SystemMediaControlsBridge` and lock-free RT telemetry.
pub struct PcmAudioBackend {
    playing: Arc<AtomicBool>,
    target_gain_bits: Arc<AtomicU64>,
    frames_played: Arc<AtomicU64>,
    seek_base_ms: AtomicU64,
    sample_rate: u32,
    active_url: Mutex<Option<String>>,
    last_meta: Mutex<Option<NowPlayingMetadata>>,
    dsp_config: Mutex<DspConfig>,
    media_controls: SystemMediaControlsBridge,
}

impl PcmAudioBackend {
    pub fn new(sample_rate: u32) -> Self {
        Self::with_hwnd(sample_rate, None)
    }

    pub fn with_hwnd(sample_rate: u32, hwnd: Option<*mut c_void>) -> Self {
        Self {
            playing: Arc::new(AtomicBool::new(false)),
            target_gain_bits: Arc::new(AtomicU64::new(1.0_f32.to_bits() as u64)),
            frames_played: Arc::new(AtomicU64::new(0)),
            seek_base_ms: AtomicU64::new(0),
            sample_rate: sample_rate.max(8_000),
            active_url: Mutex::new(None),
            last_meta: Mutex::new(None),
            dsp_config: Mutex::new(DspConfig::default()),
            media_controls: SystemMediaControlsBridge::new(hwnd),
        }
    }

    pub fn create_rt_callback(
        &self,
        consumer: SpscConsumer<f32>,
        channels: usize,
    ) -> RtAudioRenderCallback {
        RtAudioRenderCallback::new(
            consumer,
            self.sample_rate,
            channels,
            self.playing.clone(),
            self.target_gain_bits.clone(),
            self.frames_played.clone(),
        )
    }

    pub fn set_dsp_config(&self, config: DspConfig) {
        *self.dsp_config.lock().unwrap() = config;
    }

    pub fn dsp_config(&self) -> DspConfig {
        self.dsp_config.lock().unwrap().clone()
    }

    pub fn media_controls(&self) -> &SystemMediaControlsBridge {
        &self.media_controls
    }

    fn computed_position_ms(&self) -> u64 {
        let frames = self.frames_played.load(Ordering::Relaxed);
        let elapsed_ms = (frames * 1000) / (self.sample_rate as u64);
        self.seek_base_ms.load(Ordering::Relaxed) + elapsed_ms
    }
}

#[async_trait]
impl AudioBackend for PcmAudioBackend {
    async fn load_and_play(&self, proxy_url: &str, meta: NowPlayingMetadata) -> CoreResult<()> {
        *self.active_url.lock().unwrap() = Some(proxy_url.to_owned());
        *self.last_meta.lock().unwrap() = Some(meta.clone());
        self.frames_played.store(0, Ordering::SeqCst);
        self.seek_base_ms.store(0, Ordering::SeqCst);
        self.playing.store(true, Ordering::SeqCst);
        self.media_controls.sync_metadata(&meta);
        self.media_controls.sync_playback(true, 0, true);
        Ok(())
    }

    async fn play(&self) -> CoreResult<()> {
        self.playing.store(true, Ordering::SeqCst);
        let has_track =
            self.active_url.lock().unwrap().is_some() || self.last_meta.lock().unwrap().is_some();
        self.media_controls
            .sync_playback(true, self.computed_position_ms(), has_track);
        Ok(())
    }

    async fn pause(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        let has_track =
            self.active_url.lock().unwrap().is_some() || self.last_meta.lock().unwrap().is_some();
        self.media_controls
            .sync_playback(false, self.computed_position_ms(), has_track);
        Ok(())
    }

    async fn stop(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        self.frames_played.store(0, Ordering::SeqCst);
        self.seek_base_ms.store(0, Ordering::SeqCst);
        *self.active_url.lock().unwrap() = None;
        *self.last_meta.lock().unwrap() = None;
        self.media_controls.sync_playback(false, 0, false);
        Ok(())
    }

    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        self.frames_played.store(0, Ordering::SeqCst);
        self.seek_base_ms.store(position_ms, Ordering::SeqCst);
        let has_track =
            self.active_url.lock().unwrap().is_some() || self.last_meta.lock().unwrap().is_some();
        self.media_controls.sync_playback(
            self.playing.load(Ordering::SeqCst),
            position_ms,
            has_track,
        );
        Ok(())
    }

    async fn set_volume(&self, volume: f32) -> CoreResult<()> {
        let clamped = volume.clamp(0.0, 1.0);
        self.target_gain_bits
            .store(clamped.to_bits() as u64, Ordering::SeqCst);
        Ok(())
    }

    async fn position_ms(&self) -> u64 {
        self.computed_position_ms()
    }

    async fn update_now_playing(&self, meta: NowPlayingMetadata, playing: bool, position_ms: u64) {
        *self.last_meta.lock().unwrap() = Some(meta.clone());
        self.playing.store(playing, Ordering::SeqCst);
        self.media_controls.sync_metadata(&meta);
        self.media_controls
            .sync_playback(playing, position_ms, true);
    }

    fn take_command_receiver(&self) -> Option<UnboundedReceiver<MediaCommand>> {
        self.media_controls.take_command_receiver()
    }
}
