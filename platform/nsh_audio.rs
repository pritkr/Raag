//! Native `nsh/audio` Integration (`nsh_core::prelude::audio_player`).
//!
//! Uses NSH's built-in Pure Rust `AudioPlayer` streaming API as the primary audio
//! playback engine on Desktop and foreground NSH surfaces, streaming directly from
//! Resonance's local Rust Loopback HTTP Proxy (`http://127.0.0.1:<port>/stream/...`).
//!
//! Platform capabilities outside `nsh/audio`'s scope (Android background playback,
//! lock-screen `MediaSession`, Bluetooth AVRCP, `ACTION_AUDIO_BECOMING_NOISY`, and
//! Desktop MPRIS/SMTC system media keys) remain cleanly delegated to thin platform
//! bridges (`platform/android/PlaybackForegroundService.kt` and `souvlaki`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Poll, Waker};
use std::time::Duration;

use async_trait::async_trait;
use nsh_core::audio::{AudioError, AudioEvent, PlayerSettings};
use nsh_core::prelude::{AudioPlayer, audio_available, audio_player, cleanup, effect, spawn};
use resonance_contract::{
    AudioBackend, CoreResult, MediaCommand, NowPlayingMetadata, PlayerState,
};
use resonance_core::audio_dsp::{DspConfig, DspPipeline, SpscConsumer, SpscProducer, SpscRingBuffer};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::store;

/// Active native NSH `AudioPlayer` handle owned strictly on the NSH engine thread.
struct ActiveNativePlayer {
    generation: u64,
    url: String,
    player: AudioPlayer,
    is_playing: bool,
    last_seek_generation: u64,
    last_settings: PlayerSettings,
    drained: bool,
}

#[derive(Default)]
struct DriverState {
    active: Option<ActiveNativePlayer>,
    next_generation: u64,
    pump_waker: Option<Waker>,
    shutdown: bool,
}

/// Mounts the reactive native `nsh/audio` driver inside the root `App()` component
/// on the NSH engine thread.
///
/// Observes `store::audio_driver_snapshot()` and drives `nsh_core::prelude::audio_player`:
/// - `Open` + `Resume` / `Preload` on track change
/// - `Pause` / `Resume` on play/pause toggle
/// - `Seek` on explicit user/lyrics seek (never echoed by `AudioEvent::Progress`)
/// - `Set` on volume / mute / repeat-one changes
/// - `Close` on track switch, queue clear, or component unmount
///
/// The `AudioEvent` pump runs as a single component-scoped task (outside `effect`) so
/// reactive effect re-runs on Pause/Seek/Volume never cancel event delivery.
pub fn mount_nsh_audio_driver() {
    spawn(async {
        let available = audio_available().await;
        store::set_nsh_audio_available(available);
    });

    let driver: Rc<RefCell<DriverState>> = Rc::new(RefCell::new(DriverState::default()));

    // Component-scoped native AudioEvent pump (survives all reactive effect re-runs).
    let driver_for_pump = driver.clone();
    spawn(async move {
        loop {
            let next_ev = std::future::poll_fn(|cx| {
                let mut state = driver_for_pump.borrow_mut();
                if state.shutdown {
                    return Poll::Ready(None);
                }
                if let Some(entry) = state.active.as_mut()
                    && !entry.drained
                {
                    match entry.player.poll_next(cx) {
                        Poll::Ready(Some(ev)) => {
                            return Poll::Ready(Some((entry.generation, ev)));
                        }
                        Poll::Ready(None) => {
                            entry.drained = true;
                        }
                        Poll::Pending => {
                            state.pump_waker = Some(cx.waker().clone());
                            return Poll::Pending;
                        }
                    }
                }
                state.pump_waker = Some(cx.waker().clone());
                Poll::Pending
            })
            .await;

            let Some((generation, event)) = next_ev else {
                break;
            };

            // Ignore stale events if a newer track replaced this generation.
            let is_current = driver_for_pump
                .borrow()
                .active
                .as_ref()
                .is_some_and(|entry| entry.generation == generation);
            if !is_current {
                continue;
            }

            match event {
                AudioEvent::Load {
                    duration: Some(dur),
                } => {
                    store::on_native_audio_duration(dur.as_millis() as u64);
                }
                AudioEvent::Load { duration: None } => {}
                AudioEvent::Playing => {
                    if let Some(entry) = driver_for_pump.borrow_mut().active.as_mut()
                        && entry.generation == generation
                    {
                        entry.is_playing = true;
                    }
                    store::on_native_audio_state(PlayerState::Playing);
                }
                AudioEvent::Paused => {
                    if let Some(entry) = driver_for_pump.borrow_mut().active.as_mut()
                        && entry.generation == generation
                    {
                        entry.is_playing = false;
                    }
                    store::on_native_audio_state(PlayerState::Paused);
                }
                AudioEvent::Buffer { buffering } => {
                    store::on_native_audio_buffering(buffering);
                }
                AudioEvent::Progress { position, duration } => {
                    store::on_native_audio_progress(
                        position.as_millis() as u64,
                        duration.map(|d| d.as_millis() as u64),
                    );
                }
                AudioEvent::Ended => {
                    store::on_native_audio_ended();
                }
                AudioEvent::Failed { error, message }
                    // Headless test harnesses without an audio backend emit Unavailable;
                    // keep fixture UI state clean unless a real audio backend failed.
                    if error != AudioError::Unavailable =>
                {
                    store::on_native_audio_error(format!(
                        "Native audio error ({}): {}",
                        error.as_str(),
                        message
                    ));
                }
                _ => {}
            }
        }
    });

    let driver_for_effect = driver.clone();
    effect(move || {
        let snap = store::audio_driver_snapshot();
        let mut desired_settings = PlayerSettings::default();
        desired_settings.volume = snap.volume.clamp(0.0, 1.0);
        desired_settings.rate = 1.0;
        desired_settings.looped = snap.looped;

        let Some(target_url) = snap.active_proxy_url else {
            // Queue cleared / idle: dropping ActiveNativePlayer emits AudioCommand::Close.
            let waker = {
                let mut state = driver_for_effect.borrow_mut();
                state.active.take();
                state.pump_waker.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
            return;
        };

        let same_url = driver_for_effect
            .borrow()
            .active
            .as_ref()
            .is_some_and(|cur| cur.url == target_url);

        if !same_url {
            // Drop any previous player first so AudioCommand::Close precedes the new Open.
            {
                let mut state = driver_for_effect.borrow_mut();
                state.active.take();
            }

            let player = audio_player(&target_url, desired_settings);
            if snap.desired_playing {
                player.play();
            } else {
                player.preload();
            }
            if snap.position_ms > 0 && snap.seek_generation > 0 {
                player.seek(Duration::from_millis(snap.position_ms));
            }

            let waker = {
                let mut state = driver_for_effect.borrow_mut();
                state.next_generation += 1;
                let generation = state.next_generation;
                state.active = Some(ActiveNativePlayer {
                    generation,
                    url: target_url,
                    player,
                    is_playing: snap.desired_playing,
                    last_seek_generation: snap.seek_generation,
                    last_settings: desired_settings,
                    drained: false,
                });
                state.pump_waker.take()
            };
            if let Some(w) = waker {
                w.wake();
            }
            return;
        }

        // Same stream URL: apply incremental settings, seek, and play/pause transitions.
        if let Some(entry) = driver_for_effect.borrow_mut().active.as_mut() {
            if desired_settings != entry.last_settings {
                entry.player.set_settings(desired_settings);
                entry.last_settings = desired_settings;
            }
            if snap.seek_generation != entry.last_seek_generation {
                entry.player.seek(Duration::from_millis(snap.position_ms));
                entry.last_seek_generation = snap.seek_generation;
            }
            if snap.desired_playing != entry.is_playing {
                if snap.desired_playing {
                    entry.player.play();
                } else {
                    entry.player.pause();
                }
                entry.is_playing = snap.desired_playing;
            }
        }
    });

    cleanup(move || {
        let waker = {
            let mut state = driver.borrow_mut();
            state.shutdown = true;
            state.active.take();
            state.pump_waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    });
}

/// Control messages sent from `PlaybackController` (Tokio) to the NSH engine-thread
/// native `AudioPlayer` driver.
#[derive(Debug, Clone, PartialEq)]
pub enum NshAudioControl {
    LoadAndPlay {
        proxy_url: String,
        meta: NowPlayingMetadata,
    },
    Play,
    Pause,
    Stop,
    SeekMs(u64),
    SetVolume(f32),
    UpdateDsp(DspConfig),
    SetTrackLoudnessDb(Option<f32>),
}

/// Creates a lock-free cache-line-aligned SPSC PCM ring buffer for decoupling audio decoding
/// and DSP normalization from UI/network stalls.
pub fn create_pcm_ring_buffer(capacity_samples: usize) -> (SpscProducer<f32>, SpscConsumer<f32>) {
    SpscRingBuffer::with_capacity(capacity_samples)
}

/// `Send + Sync` `resonance_contract::AudioBackend` implementation that routes
/// `PlaybackController` operations to NSH's native `nsh/audio` player and listens
/// for Desktop system media keys (`souvlaki` MPRIS/SMTC when enabled).
pub struct NshAudioBackend {
    playing: AtomicBool,
    position_ms: AtomicU64,
    volume_bits: AtomicU64,
    active_url: Mutex<Option<String>>,
    last_meta: Mutex<Option<NowPlayingMetadata>>,
    dsp_config: Mutex<DspConfig>,
    dsp_pipeline: Mutex<DspPipeline>,
    control_tx: UnboundedSender<NshAudioControl>,
    control_rx: Mutex<Option<UnboundedReceiver<NshAudioControl>>>,
    cmd_tx: UnboundedSender<MediaCommand>,
    cmd_rx: Mutex<Option<UnboundedReceiver<MediaCommand>>>,
}

impl NshAudioBackend {
    pub fn new() -> Self {
        let (control_tx, control_rx) = unbounded_channel();
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let default_dsp = DspConfig::default();
        let dsp_pipeline = DspPipeline::new(48_000, 2, &default_dsp);
        let backend = Self {
            playing: AtomicBool::new(false),
            position_ms: AtomicU64::new(0),
            volume_bits: AtomicU64::new(1.0_f32.to_bits() as u64),
            active_url: Mutex::new(None),
            last_meta: Mutex::new(None),
            dsp_config: Mutex::new(default_dsp),
            dsp_pipeline: Mutex::new(dsp_pipeline),
            control_tx,
            control_rx: Mutex::new(Some(control_rx)),
            cmd_tx,
            cmd_rx: Mutex::new(Some(cmd_rx)),
        };
        backend.init_system_media_controls();
        backend
    }

    pub fn take_control_receiver(&self) -> Option<UnboundedReceiver<NshAudioControl>> {
        self.control_rx.lock().unwrap().take()
    }

    pub fn inject_command(&self, cmd: MediaCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn active_stream_url(&self) -> Option<String> {
        self.active_url.lock().unwrap().clone()
    }

    /// Updates user DSP configuration (ReplayGain, 10-band EQ, silence skip, crossfade)
    /// without blocking the NSH engine thread.
    pub fn set_dsp_config(&self, config: DspConfig) {
        *self.dsp_config.lock().unwrap() = config.clone();
        self.dsp_pipeline.lock().unwrap().apply_config(&config);
        let _ = self.control_tx.send(NshAudioControl::UpdateDsp(config));
    }

    pub fn dsp_config(&self) -> DspConfig {
        self.dsp_config.lock().unwrap().clone()
    }

    /// Applies YouTube's `loudnessDb` metadata for the active track to ReplayGain / EBU R128 normalization.
    pub fn set_track_loudness_db(&self, loudness_db: Option<f32>) {
        self.dsp_pipeline
            .lock()
            .unwrap()
            .set_track_loudness_db(loudness_db);
        let _ = self
            .control_tx
            .send(NshAudioControl::SetTrackLoudnessDb(loudness_db));
    }

    /// Runs the configured modular PCM DSP pipeline on an interleaved stereo audio frame block.
    /// Returns `false` if the block is silence-skipped.
    pub fn process_dsp_block(&self, interleaved: &mut [f32]) -> bool {
        self.dsp_pipeline
            .lock()
            .unwrap()
            .process_interleaved(interleaved)
    }

    #[cfg(all(
        not(target_os = "android"),
        feature = "desktop-mpris",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    fn init_system_media_controls(&self) {
        use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig};
        let tx = self.cmd_tx.clone();
        let config = PlatformConfig {
            dbus_name: "resonance",
            display_name: "Resonance",
            hwnd: None,
        };
        if let Ok(mut controls) = MediaControls::new(config) {
            let _ = controls.attach(move |ev| {
                let mapped = match ev {
                    MediaControlEvent::Play => Some(MediaCommand::Play),
                    MediaControlEvent::Pause | MediaControlEvent::Stop => Some(MediaCommand::Pause),
                    MediaControlEvent::Toggle => Some(MediaCommand::TogglePlayPause),
                    MediaControlEvent::Next => Some(MediaCommand::Next),
                    MediaControlEvent::Previous => Some(MediaCommand::Previous),
                    MediaControlEvent::SetPosition(p) => {
                        Some(MediaCommand::SeekMs(p.0.as_millis() as u64))
                    }
                    _ => None,
                };
                if let Some(cmd) = mapped {
                    let _ = tx.send(cmd);
                }
            });
            std::mem::forget(controls);
        }
    }

    #[cfg(not(all(
        not(target_os = "android"),
        feature = "desktop-mpris",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )))]
    fn init_system_media_controls(&self) {
        log::debug!("NshAudioBackend ready (native nsh/audio + in-process media command bus)");
    }
}

/// Lock-free DSP parameter update sent from control/UI threads to [`LockFreeAudioConsumer`].
#[derive(Debug, Clone)]
pub enum DspControlUpdate {
    SetConfig(DspConfig),
    SetTrackLoudnessDb(Option<f32>),
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

impl Default for NshAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AudioBackend for NshAudioBackend {
    async fn load_and_play(&self, proxy_url: &str, meta: NowPlayingMetadata) -> CoreResult<()> {
        *self.active_url.lock().unwrap() = Some(proxy_url.to_owned());
        *self.last_meta.lock().unwrap() = Some(meta.clone());
        self.position_ms.store(0, Ordering::SeqCst);
        self.playing.store(true, Ordering::SeqCst);
        let _ = self.control_tx.send(NshAudioControl::LoadAndPlay {
            proxy_url: proxy_url.to_owned(),
            meta,
        });
        log::info!(
            "NshAudioBackend: routing {} to native nsh/audio AudioPlayer",
            resonance_core::innertube::auth::redact_secrets(proxy_url)
        );
        Ok(())
    }

    async fn play(&self) -> CoreResult<()> {
        self.playing.store(true, Ordering::SeqCst);
        let _ = self.control_tx.send(NshAudioControl::Play);
        Ok(())
    }

    async fn pause(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        let _ = self.control_tx.send(NshAudioControl::Pause);
        Ok(())
    }

    async fn stop(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        self.position_ms.store(0, Ordering::SeqCst);
        *self.active_url.lock().unwrap() = None;
        let _ = self.control_tx.send(NshAudioControl::Stop);
        Ok(())
    }

    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        self.position_ms.store(position_ms, Ordering::SeqCst);
        let _ = self.control_tx.send(NshAudioControl::SeekMs(position_ms));
        Ok(())
    }

    async fn set_volume(&self, volume: f32) -> CoreResult<()> {
        let clamped = volume.clamp(0.0, 1.0);
        self.volume_bits
            .store(clamped.to_bits() as u64, Ordering::SeqCst);
        let _ = self.control_tx.send(NshAudioControl::SetVolume(clamped));
        Ok(())
    }

    async fn position_ms(&self) -> u64 {
        self.position_ms.load(Ordering::SeqCst)
    }

    async fn update_now_playing(&self, meta: NowPlayingMetadata, playing: bool, position_ms: u64) {
        *self.last_meta.lock().unwrap() = Some(meta);
        self.playing.store(playing, Ordering::SeqCst);
        self.position_ms.store(position_ms, Ordering::SeqCst);
    }

    fn take_command_receiver(&self) -> Option<UnboundedReceiver<MediaCommand>> {
        self.cmd_rx.lock().unwrap().take()
    }
}
