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
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use nsh_core::audio::{AudioError, AudioEvent, PlayerSettings};
use nsh_core::prelude::{AudioPlayer, audio_available, audio_player, cleanup, effect, spawn};
use resonance_contract::{AudioBackend, CoreResult, MediaCommand, NowPlayingMetadata, PlayerState};
use resonance_core::audio_dsp::{
    DspConfig, DspPipeline, SpscConsumer, SpscProducer, SpscRingBuffer,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::platform::desktop::SystemMediaControlsBridge;
use crate::store;

pub type NshAudioTelemetry = resonance_core::playback::AudioTelemetry;

/// Global lock-free bus connecting `NshAudioBackend` (Tokio runtime) with
/// `mount_nsh_audio_driver()` (NSH engine thread).
pub struct NshAudioBus {
    pub control_tx: crossbeam_channel::Sender<NshAudioControl>,
    pub control_rx: crossbeam_channel::Receiver<NshAudioControl>,
    pub telemetry_tx: crossbeam_channel::Sender<NshAudioTelemetry>,
    pub telemetry_rx: crossbeam_channel::Receiver<NshAudioTelemetry>,
    control_waker: Mutex<Option<Waker>>,
}

impl NshAudioBus {
    pub fn send_control(&self, cmd: NshAudioControl) {
        let _ = self.control_tx.send(cmd);
        if let Some(w) = self.control_waker.lock().unwrap().take() {
            w.wake();
        }
    }

    pub fn wake_control(&self) {
        if let Some(w) = self.control_waker.lock().unwrap().take() {
            w.wake();
        }
    }
}

static NSH_AUDIO_BUS: OnceLock<NshAudioBus> = OnceLock::new();

pub fn nsh_audio_bus() -> &'static NshAudioBus {
    NSH_AUDIO_BUS.get_or_init(|| {
        let (control_tx, control_rx) = crossbeam_channel::unbounded();
        let (telemetry_tx, telemetry_rx) = crossbeam_channel::unbounded();
        NshAudioBus {
            control_tx,
            control_rx,
            telemetry_tx,
            telemetry_rx,
            control_waker: Mutex::new(None),
        }
    })
}

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

    let fixture_mode = store::is_fixture_mode();
    let driver: Rc<RefCell<DriverState>> = Rc::new(RefCell::new(DriverState::default()));

    // Component-scoped native AudioEvent pump (survives all reactive effect re-runs).
    let driver_for_pump = driver.clone();
    spawn(async move {
        let mut last_progress_emit: Option<(u64, Instant)> = None;
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

            if fixture_mode {
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
            } else {
                let bus = nsh_audio_bus();
                match event {
                    AudioEvent::Load {
                        duration: Some(dur),
                    } => {
                        last_progress_emit = None;
                        let _ = bus.telemetry_tx.send(NshAudioTelemetry::Loaded {
                            generation,
                            duration_ms: dur.as_millis() as u64,
                        });
                    }
                    AudioEvent::Load { duration: None } => {
                        last_progress_emit = None;
                    }
                    AudioEvent::Playing => {
                        if let Some(entry) = driver_for_pump.borrow_mut().active.as_mut()
                            && entry.generation == generation
                        {
                            entry.is_playing = true;
                        }
                        let _ = bus.telemetry_tx.send(NshAudioTelemetry::StateChanged {
                            generation,
                            state: PlayerState::Playing,
                        });
                    }
                    AudioEvent::Paused => {
                        if let Some(entry) = driver_for_pump.borrow_mut().active.as_mut()
                            && entry.generation == generation
                        {
                            entry.is_playing = false;
                        }
                        let _ = bus.telemetry_tx.send(NshAudioTelemetry::StateChanged {
                            generation,
                            state: PlayerState::Paused,
                        });
                    }
                    AudioEvent::Buffer { buffering } => {
                        let _ = bus.telemetry_tx.send(NshAudioTelemetry::Buffering {
                            generation,
                            buffering,
                        });
                    }
                    AudioEvent::Progress { position, duration } => {
                        let pos_ms = position.as_millis() as u64;
                        let now = Instant::now();
                        let should_emit = match last_progress_emit {
                            None => true,
                            Some((prev_ms, prev_at)) => {
                                pos_ms.abs_diff(prev_ms) >= 250
                                    || now.duration_since(prev_at) >= Duration::from_millis(250)
                            }
                        };
                        if should_emit {
                            last_progress_emit = Some((pos_ms, now));
                            let _ = bus.telemetry_tx.send(NshAudioTelemetry::Progress {
                                generation,
                                position_ms: pos_ms,
                                duration_ms: duration.map(|d| d.as_millis() as u64),
                            });
                        }
                    }
                    AudioEvent::Ended => {
                        if let Some(entry) = driver_for_pump.borrow_mut().active.as_mut()
                            && entry.generation == generation
                        {
                            entry.is_playing = false;
                        }
                        let _ = bus
                            .telemetry_tx
                            .send(NshAudioTelemetry::Ended { generation });
                    }
                    AudioEvent::Failed { error, message } if error != AudioError::Unavailable => {
                        let _ = bus.telemetry_tx.send(NshAudioTelemetry::Failed {
                            generation,
                            message: format!(
                                "Native audio error ({}): {}",
                                error.as_str(),
                                message
                            ),
                        });
                    }
                    _ => {}
                }
            }
        }
    });

    if fixture_mode {
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
    } else {
        // Live mode: pure executor draining NSH_AUDIO_BUS.control_rx (zero store signal echo).
        let driver_for_ctrl = driver.clone();
        spawn(async move {
            let bus = nsh_audio_bus();
            let mut volume = 1.0_f32;
            let mut playback_speed = 1.0_f32;
            loop {
                let cmd = std::future::poll_fn(|cx| {
                    if driver_for_ctrl.borrow().shutdown {
                        return Poll::Ready(None);
                    }
                    if let Ok(cmd) = bus.control_rx.try_recv() {
                        return Poll::Ready(Some(cmd));
                    }
                    *bus.control_waker.lock().unwrap() = Some(cx.waker().clone());
                    // Re-check after registering waker to avoid missed wakeups.
                    if let Ok(cmd) = bus.control_rx.try_recv() {
                        *bus.control_waker.lock().unwrap() = None;
                        return Poll::Ready(Some(cmd));
                    }
                    Poll::Pending
                })
                .await;

                let Some(cmd) = cmd else {
                    break;
                };

                match cmd {
                    NshAudioControl::LoadAndPlay { proxy_url, .. } => {
                        {
                            let mut state = driver_for_ctrl.borrow_mut();
                            state.active.take();
                        }
                        let mut settings = PlayerSettings::default();
                        settings.volume = volume.clamp(0.0, 1.0);
                        settings.rate = playback_speed.clamp(0.25, 3.0);
                        let player = audio_player(&proxy_url, settings);
                        player.play();
                        let parsed_gen = parse_generation_from_url(&proxy_url);
                        let waker = {
                            let mut state = driver_for_ctrl.borrow_mut();
                            let generation = if let Some(g) = parsed_gen {
                                state.next_generation = state.next_generation.max(g);
                                g
                            } else {
                                state.next_generation += 1;
                                state.next_generation
                            };
                            state.active = Some(ActiveNativePlayer {
                                generation,
                                url: proxy_url,
                                player,
                                is_playing: true,
                                last_seek_generation: 0,
                                last_settings: settings,
                                drained: false,
                            });
                            state.pump_waker.take()
                        };
                        if let Some(w) = waker {
                            w.wake();
                        }
                    }
                    NshAudioControl::Play => {
                        if let Some(entry) = driver_for_ctrl.borrow_mut().active.as_mut() {
                            entry.player.play();
                            entry.is_playing = true;
                        }
                    }
                    NshAudioControl::Pause => {
                        if let Some(entry) = driver_for_ctrl.borrow_mut().active.as_mut() {
                            entry.player.pause();
                            entry.is_playing = false;
                        }
                    }
                    NshAudioControl::Stop => {
                        let waker = {
                            let mut state = driver_for_ctrl.borrow_mut();
                            state.active.take();
                            state.pump_waker.take()
                        };
                        if let Some(w) = waker {
                            w.wake();
                        }
                    }
                    NshAudioControl::SeekMs(ms) => {
                        if let Some(entry) = driver_for_ctrl.borrow_mut().active.as_mut() {
                            entry.player.seek(Duration::from_millis(ms));
                        }
                    }
                    NshAudioControl::SetVolume(vol) => {
                        volume = vol.clamp(0.0, 1.0);
                        if let Some(entry) = driver_for_ctrl.borrow_mut().active.as_mut() {
                            entry.last_settings.volume = volume;
                            entry.player.set_settings(entry.last_settings);
                        }
                    }
                    NshAudioControl::SetPlaybackSpeed(speed) => {
                        playback_speed = speed.clamp(0.25, 3.0);
                        if let Some(entry) = driver_for_ctrl.borrow_mut().active.as_mut() {
                            entry.last_settings.rate = playback_speed;
                            entry.player.set_settings(entry.last_settings);
                        }
                    }
                    NshAudioControl::UpdateDsp(_) | NshAudioControl::SetTrackLoudnessDb(_) => {}
                }
            }
        });
    }

    cleanup(move || {
        let (waker, generation) = {
            let mut state = driver.borrow_mut();
            state.shutdown = true;
            let generation = state.active.as_ref().map_or(0, |a| a.generation);
            state.active.take();
            (state.pump_waker.take(), generation)
        };
        if let Some(w) = waker {
            w.wake();
        }
        if !fixture_mode {
            let bus = nsh_audio_bus();
            bus.wake_control();
            if generation > 0 {
                let _ = bus.telemetry_tx.send(NshAudioTelemetry::StateChanged {
                    generation,
                    state: PlayerState::Idle,
                });
            }
        }
    });
}

/// Extracts the monotonic `gen=<u64>` query parameter from a loopback proxy URL
/// so `mount_nsh_audio_driver()` stays generation-fenced with `PlaybackController`.
pub fn parse_generation_from_url(url: &str) -> Option<u64> {
    let query = url.split('?').nth(1)?;
    for pair in query.split('&') {
        if let Some(val) = pair.strip_prefix("gen=")
            && let Ok(parsed) = val.parse::<u64>()
            && parsed > 0
        {
            return Some(parsed);
        }
    }
    None
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
    SetPlaybackSpeed(f32),
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
/// for Desktop system media keys (`souvlaki` MPRIS/SMTC when enabled via [`SystemMediaControlsBridge`]).
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
    media_controls: SystemMediaControlsBridge,
}

impl NshAudioBackend {
    pub fn new() -> Self {
        Self::with_hwnd(None)
    }

    pub fn with_hwnd(hwnd: Option<*mut c_void>) -> Self {
        let (control_tx, control_rx) = unbounded_channel();
        let default_dsp = DspConfig::default();
        let dsp_pipeline = DspPipeline::new(48_000, 2, &default_dsp);
        Self {
            playing: AtomicBool::new(false),
            position_ms: AtomicU64::new(0),
            volume_bits: AtomicU64::new(1.0_f32.to_bits() as u64),
            active_url: Mutex::new(None),
            last_meta: Mutex::new(None),
            dsp_config: Mutex::new(default_dsp),
            dsp_pipeline: Mutex::new(dsp_pipeline),
            control_tx,
            control_rx: Mutex::new(Some(control_rx)),
            media_controls: SystemMediaControlsBridge::new(hwnd),
        }
    }

    pub fn attach_window_hwnd(&self, hwnd: *mut c_void) {
        self.media_controls.attach_window_hwnd(hwnd);
    }

    pub fn media_controls(&self) -> &SystemMediaControlsBridge {
        &self.media_controls
    }

    pub fn take_control_receiver(&self) -> Option<UnboundedReceiver<NshAudioControl>> {
        self.control_rx.lock().unwrap().take()
    }

    pub fn inject_command(&self, cmd: MediaCommand) {
        self.media_controls.inject_command(cmd);
    }

    pub fn active_stream_url(&self) -> Option<String> {
        self.active_url.lock().unwrap().clone()
    }

    pub async fn set_playback_speed(&self, speed: f32) -> CoreResult<()> {
        let clamped = speed.clamp(0.25, 3.0);
        self.emit_control(NshAudioControl::SetPlaybackSpeed(clamped));
        Ok(())
    }

    fn emit_control(&self, cmd: NshAudioControl) {
        let _ = self.control_tx.send(cmd.clone());
        nsh_audio_bus().send_control(cmd);
    }

    /// Updates user DSP configuration (ReplayGain, 10-band EQ, silence skip, crossfade)
    /// without blocking the NSH engine thread.
    pub fn set_dsp_config(&self, config: DspConfig) {
        *self.dsp_config.lock().unwrap() = config.clone();
        self.dsp_pipeline.lock().unwrap().apply_config(&config);
        self.emit_control(NshAudioControl::UpdateDsp(config));
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
        self.emit_control(NshAudioControl::SetTrackLoudnessDb(loudness_db));
    }

    /// Runs the configured modular PCM DSP pipeline on an interleaved stereo audio frame block.
    /// Returns `false` if the block is silence-skipped.
    pub fn process_dsp_block(&self, interleaved: &mut [f32]) -> bool {
        self.dsp_pipeline
            .lock()
            .unwrap()
            .process_interleaved(interleaved)
    }
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
        self.media_controls.sync_metadata(&meta);
        self.media_controls.sync_playback(true, 0, true);
        self.emit_control(NshAudioControl::LoadAndPlay {
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
        let has_track =
            self.active_url.lock().unwrap().is_some() || self.last_meta.lock().unwrap().is_some();
        self.media_controls
            .sync_playback(true, self.position_ms.load(Ordering::SeqCst), has_track);
        self.emit_control(NshAudioControl::Play);
        Ok(())
    }

    async fn pause(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        let has_track =
            self.active_url.lock().unwrap().is_some() || self.last_meta.lock().unwrap().is_some();
        self.media_controls.sync_playback(
            false,
            self.position_ms.load(Ordering::SeqCst),
            has_track,
        );
        self.emit_control(NshAudioControl::Pause);
        Ok(())
    }

    async fn stop(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        self.position_ms.store(0, Ordering::SeqCst);
        *self.active_url.lock().unwrap() = None;
        *self.last_meta.lock().unwrap() = None;
        self.media_controls.sync_playback(false, 0, false);
        self.emit_control(NshAudioControl::Stop);
        Ok(())
    }

    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        self.position_ms.store(position_ms, Ordering::SeqCst);
        let has_track =
            self.active_url.lock().unwrap().is_some() || self.last_meta.lock().unwrap().is_some();
        self.media_controls.sync_playback(
            self.playing.load(Ordering::SeqCst),
            position_ms,
            has_track,
        );
        self.emit_control(NshAudioControl::SeekMs(position_ms));
        Ok(())
    }

    async fn set_volume(&self, volume: f32) -> CoreResult<()> {
        let clamped = volume.clamp(0.0, 1.6);
        self.volume_bits
            .store(clamped.to_bits() as u64, Ordering::SeqCst);
        self.emit_control(NshAudioControl::SetVolume(clamped));
        Ok(())
    }

    async fn position_ms(&self) -> u64 {
        self.position_ms.load(Ordering::SeqCst)
    }

    async fn update_now_playing(&self, meta: NowPlayingMetadata, playing: bool, position_ms: u64) {
        *self.last_meta.lock().unwrap() = Some(meta.clone());
        self.playing.store(playing, Ordering::SeqCst);
        self.position_ms.store(position_ms, Ordering::SeqCst);
        self.media_controls.sync_metadata(&meta);
        self.media_controls
            .sync_playback(playing, position_ms, true);
    }

    fn take_command_receiver(&self) -> Option<UnboundedReceiver<MediaCommand>> {
        self.media_controls.take_command_receiver()
    }
}
