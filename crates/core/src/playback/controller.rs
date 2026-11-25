//! Async runtime bridge (`PlaybackController`) coordinating [`PlaybackFsm`],
//! [`QueueManager`], [`PrefetchCoordinator`], [`LoopbackProxy`], and platform audio backends.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use resonance_contract::{
    AppEvent, AudioBackend, CacheStore, CoreError, CoreResult, EventBus, HistoryEntry,
    LyricsProvider, MediaCommand, MusicProvider, NowPlayingMetadata, OfflineStore, PlaybackError,
    PlayerSnapshot, PlayerState, RepeatMode, Track,
};
use tokio::sync::Mutex;

use crate::innertube::auth::redact_secrets;
use crate::playback::fsm::PlaybackFsm;
use crate::playback::prefetch::PrefetchCoordinator;
use crate::playback::queue::QueueManager;
use crate::proxy::LoopbackProxy;
use crate::unix_secs;

/// Unified telemetry emitted by the native audio executor (`nsh/audio` or platform bridge)
/// back to [`PlaybackController`] for Single Source-of-Truth state updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioTelemetry {
    Loaded {
        generation: u64,
        duration_ms: u64,
    },
    StateChanged {
        generation: u64,
        state: PlayerState,
    },
    Buffering {
        generation: u64,
        buffering: bool,
    },
    Progress {
        generation: u64,
        position_ms: u64,
        duration_ms: Option<u64>,
    },
    Ended {
        generation: u64,
    },
    Failed {
        generation: u64,
        message: String,
    },
}

impl AudioTelemetry {
    pub fn generation(&self) -> u64 {
        match self {
            Self::Loaded { generation, .. }
            | Self::StateChanged { generation, .. }
            | Self::Buffering { generation, .. }
            | Self::Progress { generation, .. }
            | Self::Ended { generation }
            | Self::Failed { generation, .. } => *generation,
        }
    }
}

pub struct PlaybackController {
    provider: Arc<dyn MusicProvider>,
    audio: Arc<dyn AudioBackend>,
    cache: Arc<dyn CacheStore>,
    offline: Option<Arc<dyn OfflineStore>>,
    proxy: Arc<LoopbackProxy>,
    lyrics: Arc<dyn LyricsProvider>,
    sponsorblock: Arc<dyn resonance_contract::SponsorBlockProvider>,
    bus: EventBus,
    play_generation: AtomicU64,
    last_now_playing_sync_ms: AtomicU64,
    fsm: Mutex<PlaybackFsm>,
}

impl PlaybackController {
    pub fn new(
        provider: Arc<dyn MusicProvider>,
        audio: Arc<dyn AudioBackend>,
        cache: Arc<dyn CacheStore>,
        proxy: Arc<LoopbackProxy>,
        lyrics: Arc<dyn LyricsProvider>,
        sponsorblock: Arc<dyn resonance_contract::SponsorBlockProvider>,
        bus: EventBus,
    ) -> Self {
        let restored_queue = cache.load_queue().ok().flatten().unwrap_or_default();
        Self {
            provider,
            audio,
            cache,
            offline: None,
            proxy,
            lyrics,
            sponsorblock,
            bus,
            play_generation: AtomicU64::new(0),
            last_now_playing_sync_ms: AtomicU64::new(0),
            fsm: Mutex::new(PlaybackFsm::from_restored_queue(restored_queue)),
        }
    }

    pub fn current_generation(&self) -> u64 {
        self.play_generation.load(Ordering::SeqCst)
    }

    pub fn with_offline_store(mut self, offline: Arc<dyn OfflineStore>) -> Self {
        self.offline = Some(offline);
        self
    }

    pub async fn snapshot(&self) -> PlayerSnapshot {
        self.fsm.lock().await.snapshot.clone()
    }

    /// Returns the video ID of the next track pre-resolved at the 50% progress threshold.
    pub async fn preresolved_next_video_id(&self) -> Option<String> {
        self.fsm.lock().await.preresolved_video_id.clone()
    }

    /// Returns the video ID of the next track pre-warmed into Proxy v2 at the 75% progress threshold.
    pub async fn prewarmed_next_video_id(&self) -> Option<String> {
        self.fsm.lock().await.prewarmed_video_id.clone()
    }

    pub fn event_bus(&self) -> EventBus {
        self.bus.clone()
    }

    fn publish_and_persist(&self, fsm: &PlaybackFsm) {
        let _ = self.cache.save_queue(&fsm.snapshot.queue);
        self.bus
            .publish(AppEvent::PlayerUpdated(fsm.snapshot.clone()));
    }

    /// Replaces queue with `tracks` and starts playing track at `start_index`.
    pub async fn play_tracks(&self, tracks: Vec<Track>, start_index: usize) -> CoreResult<()> {
        {
            let mut fsm = self.fsm.lock().await;
            QueueManager::replace(&mut fsm.snapshot.queue, tracks, start_index)?;
        }
        self.load_and_play_current(0).await
    }

    /// Plays a single track immediately.
    pub async fn play_track(&self, track: Track) -> CoreResult<()> {
        self.play_tracks(vec![track], 0).await
    }

    /// Inserts a track immediately after the currently playing item.
    pub async fn play_next(&self, track: Track) -> CoreResult<()> {
        let mut fsm = self.fsm.lock().await;
        QueueManager::play_next(&mut fsm.snapshot.queue, track);
        self.publish_and_persist(&fsm);
        Ok(())
    }

    /// Appends a track to the end of the queue.
    pub async fn play_last(&self, track: Track) -> CoreResult<()> {
        let mut fsm = self.fsm.lock().await;
        QueueManager::play_last(&mut fsm.snapshot.queue, track);
        self.publish_and_persist(&fsm);
        Ok(())
    }

    /// Reorders an item in the queue, preserving the currently playing track index.
    pub async fn reorder_queue(&self, from: usize, to: usize) -> CoreResult<()> {
        let mut fsm = self.fsm.lock().await;
        QueueManager::reorder(&mut fsm.snapshot.queue, from, to);
        self.publish_and_persist(&fsm);
        Ok(())
    }

    /// Removes an item at `index` from the queue.
    pub async fn remove_from_queue(&self, index: usize) -> CoreResult<()> {
        let mut fsm = self.fsm.lock().await;
        if let Some(updated_track) = QueueManager::remove(&mut fsm.snapshot.queue, index) {
            fsm.snapshot.current_track = updated_track;
            if fsm.snapshot.current_track.is_none() {
                let _ = fsm.snapshot.state.transition(PlayerState::Idle);
            }
        }
        self.publish_and_persist(&fsm);
        Ok(())
    }

    pub async fn clear_queue(&self) -> CoreResult<()> {
        let _ = self.audio.stop().await;
        let mut fsm = self.fsm.lock().await;
        fsm.clear_to_idle();
        self.publish_and_persist(&fsm);
        Ok(())
    }

    pub async fn toggle_shuffle(&self) {
        let mut fsm = self.fsm.lock().await;
        QueueManager::toggle_shuffle(&mut fsm.snapshot.queue);
        self.publish_and_persist(&fsm);
    }

    pub async fn cycle_repeat(&self) {
        let mut fsm = self.fsm.lock().await;
        QueueManager::cycle_repeat(&mut fsm.snapshot.queue);
        self.publish_and_persist(&fsm);
    }

    pub async fn set_autoplay_radio(&self, enabled: bool) {
        let mut fsm = self.fsm.lock().await;
        fsm.snapshot.queue.autoplay_radio = enabled;
        self.publish_and_persist(&fsm);
    }

    pub async fn set_sponsorblock_enabled(&self, enabled: bool) {
        let mut fsm = self.fsm.lock().await;
        fsm.snapshot.sponsorblock_enabled = enabled;
        self.publish_and_persist(&fsm);
    }

    pub async fn set_repeat_mode(&self, mode: RepeatMode) {
        let mut fsm = self.fsm.lock().await;
        fsm.snapshot.queue.repeat = mode;
        self.publish_and_persist(&fsm);
    }

    pub async fn set_shuffle(&self, enabled: bool) {
        let mut fsm = self.fsm.lock().await;
        if fsm.snapshot.queue.shuffle != enabled {
            QueueManager::toggle_shuffle(&mut fsm.snapshot.queue);
            self.publish_and_persist(&fsm);
        }
    }

    pub async fn set_volume(&self, volume: f32) -> CoreResult<()> {
        let clamped = volume.clamp(0.0, 1.0);
        self.audio.set_volume(clamped).await?;
        let mut fsm = self.fsm.lock().await;
        fsm.snapshot.volume = clamped;
        self.publish_and_persist(&fsm);
        Ok(())
    }

    pub async fn set_muted(&self, muted: bool) -> CoreResult<()> {
        let target = if muted {
            0.0
        } else {
            self.fsm.lock().await.snapshot.volume.clamp(0.0, 1.0)
        };
        self.audio.set_volume(target).await?;
        Ok(())
    }

    pub async fn jump_to_queue_index(&self, index: usize) -> CoreResult<()> {
        {
            let mut fsm = self.fsm.lock().await;
            if index >= fsm.snapshot.queue.items.len() {
                return Err(CoreError::Playback(PlaybackError::EmptyQueue));
            }
            fsm.snapshot.queue.current_index = Some(index);
            fsm.snapshot.queue.resume_position_ms = 0;
        }
        self.load_and_play_current(0).await
    }

    async fn load_and_play_current(&self, resume_ms: u64) -> CoreResult<()> {
        let my_gen = self.play_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.last_now_playing_sync_ms.store(0, Ordering::SeqCst);
        let proxy_gen = self.proxy.advance_generation();

        let track = {
            let mut fsm = self.fsm.lock().await;
            let t = fsm.begin_loading(resume_ms)?;
            self.publish_and_persist(&fsm);
            t
        };

        let offline_hit = self.offline.as_ref().and_then(|o| {
            let rec = o.get_completed_download(&track.id).ok().flatten()?;
            let rel = rec.relative_path.as_deref()?;
            if crate::offline::resolve_confined_path(&o.downloads_root(), rel)
                .is_ok_and(|p| p.is_file())
            {
                Some(rec)
            } else {
                None
            }
        });

        let (proxy_url, duration_ms, offline_used) = if let Some(rec) = offline_hit {
            (
                self.proxy
                    .stream_url_with_gen(&track.id, rec.itag, proxy_gen),
                track.duration_ms,
                true,
            )
        } else {
            let stream_res = self.provider.resolve_stream(&track.id).await;

            // If a rapid track skip happened while resolving, cleanly abort without clobbering state.
            if self.play_generation.load(Ordering::SeqCst) != my_gen {
                return Ok(());
            }

            match stream_res {
                Ok(info) => {
                    let dur = if info.duration_ms > 0 {
                        info.duration_ms
                    } else {
                        track.duration_ms
                    };
                    let itag = info.itag;
                    self.proxy.register_current_stream(info);
                    (
                        self.proxy.stream_url_with_gen(&track.id, itag, proxy_gen),
                        dur,
                        false,
                    )
                }
                Err(e) => {
                    // If offline, attempt direct loopback proxy URL against cached chunks.
                    if matches!(e, CoreError::Provider(_) | CoreError::Offline(_)) {
                        let fallback_itag = if cfg!(any(target_os = "macos", target_os = "ios")) {
                            140
                        } else {
                            251
                        };
                        (
                            self.proxy
                                .stream_url_with_gen(&track.id, fallback_itag, proxy_gen),
                            track.duration_ms,
                            true,
                        )
                    } else {
                        let mut fsm = self.fsm.lock().await;
                        fsm.enter_error(e.to_string());
                        self.publish_and_persist(&fsm);
                        return Err(e);
                    }
                }
            }
        };

        if self.play_generation.load(Ordering::SeqCst) != my_gen {
            return Ok(());
        }

        {
            let mut fsm = self.fsm.lock().await;
            fsm.enter_buffering(proxy_url.clone(), duration_ms, offline_used);
            self.publish_and_persist(&fsm);
        }

        let meta = NowPlayingMetadata {
            track_id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist_display(),
            album: track.album.as_ref().map(|a| a.title.clone()),
            artwork_url: track.artwork_url.clone(),
            duration_ms,
        };

        if let Err(e) = self.audio.load_and_play(&proxy_url, meta.clone()).await {
            let mut fsm = self.fsm.lock().await;
            fsm.enter_error(e.to_string());
            self.publish_and_persist(&fsm);
            return Err(e);
        }

        if resume_ms > 0 {
            let _ = self.audio.seek_ms(resume_ms).await;
        }

        self.audio.update_now_playing(meta, true, resume_ms).await;

        let _ = self.cache.record_history(&HistoryEntry {
            track: track.clone(),
            played_at_unix_secs: unix_secs(),
        });

        {
            let mut fsm = self.fsm.lock().await;
            fsm.enter_playing();
            self.publish_and_persist(&fsm);
        }

        // Fetch lyrics and SponsorBlock segments asynchronously without blocking playback start.
        let lyrics_provider = self.lyrics.clone();
        let sb_provider = self.sponsorblock.clone();
        let bus = self.bus.clone();
        let track_for_bg = track.clone();
        tokio::spawn(async move {
            let lyrics = lyrics_provider
                .fetch_lyrics(&track_for_bg)
                .await
                .ok()
                .flatten();
            bus.publish(AppEvent::LyricsLoaded {
                video_id: track_for_bg.id.clone(),
                lyrics,
            });
        });

        let segs = sb_provider
            .fetch_segments(&track.id)
            .await
            .unwrap_or_default();
        if !segs.is_empty() {
            let mut fsm = self.fsm.lock().await;
            if fsm
                .snapshot
                .current_track
                .as_ref()
                .is_some_and(|t| t.id == track.id)
            {
                fsm.sponsor_segments = segs;
            }
        }

        Ok(())
    }

    pub async fn pause(&self) -> CoreResult<()> {
        self.audio.pause().await?;
        let mut fsm = self.fsm.lock().await;
        fsm.enter_paused();
        if let Some(t) = &fsm.snapshot.current_track {
            self.audio
                .update_now_playing(
                    NowPlayingMetadata {
                        track_id: t.id.clone(),
                        title: t.title.clone(),
                        artist: t.artist_display(),
                        album: t.album.as_ref().map(|a| a.title.clone()),
                        artwork_url: t.artwork_url.clone(),
                        duration_ms: fsm.snapshot.duration_ms,
                    },
                    false,
                    fsm.snapshot.position_ms,
                )
                .await;
        }
        self.publish_and_persist(&fsm);
        Ok(())
    }

    pub async fn resume(&self) -> CoreResult<()> {
        let needs_load = {
            let fsm = self.fsm.lock().await;
            fsm.snapshot.active_proxy_url.is_none() && fsm.snapshot.current_track.is_some()
        };
        if needs_load {
            let resume_pos = self.fsm.lock().await.snapshot.position_ms;
            return self.load_and_play_current(resume_pos).await;
        }
        self.audio.play().await?;
        let mut fsm = self.fsm.lock().await;
        fsm.enter_playing();
        self.publish_and_persist(&fsm);
        Ok(())
    }

    pub async fn toggle_play_pause(&self) -> CoreResult<()> {
        let is_playing = self.fsm.lock().await.snapshot.state.is_playing();
        if is_playing {
            self.pause().await
        } else {
            self.resume().await
        }
    }

    pub async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        let clamped = self.fsm.lock().await.clamp_seek(position_ms);
        self.audio.seek_ms(clamped).await?;
        let mut fsm = self.fsm.lock().await;
        fsm.apply_seek(clamped);
        self.publish_and_persist(&fsm);
        Ok(())
    }

    pub async fn next(&self) -> CoreResult<()> {
        let action = {
            let fsm = self.fsm.lock().await;
            QueueManager::resolve_next_index(&fsm.snapshot.queue)?
        };

        if let Some(next_idx) = action {
            return self.jump_to_queue_index(next_idx).await;
        }

        // End of queue: check Autoplay Radio (`next`).
        let (autoplay, seed_id) = {
            let fsm = self.fsm.lock().await;
            (
                fsm.snapshot.queue.autoplay_radio,
                fsm.snapshot.current_track.as_ref().map(|t| t.id.clone()),
            )
        };
        if autoplay
            && let Some(video_id) = seed_id
            && let Ok(radio) = self.provider.autoplay_next(&video_id).await
        {
            let next_idx = {
                let mut fsm = self.fsm.lock().await;
                QueueManager::merge_radio_tracks(&mut fsm.snapshot.queue, radio)
            };
            if let Some(idx) = next_idx {
                return self.jump_to_queue_index(idx).await;
            }
        }

        self.pause().await
    }

    pub async fn previous(&self) -> CoreResult<()> {
        let pos = self.fsm.lock().await.snapshot.position_ms;
        if pos > 3_000 {
            return self.seek_ms(0).await;
        }
        let prev_idx = {
            let fsm = self.fsm.lock().await;
            QueueManager::resolve_previous_index(&fsm.snapshot.queue)?
        };
        self.jump_to_queue_index(prev_idx).await
    }

    /// Periodic tick called on playback progress; persists resume position, enforces
    /// SponsorBlock (`music_offtopic`) auto-skip, pre-resolves the next queued track at >= 50%,
    /// and pre-warms Proxy v2 `next_track` chunks at >= 75%.
    pub async fn tick_position(&self, position_ms: u64) -> CoreResult<()> {
        let effects = {
            let mut fsm = self.fsm.lock().await;
            fsm.tick_position(position_ms)
        };

        PrefetchCoordinator::spawn_for_decision(
            effects.prefetch,
            self.provider.clone(),
            self.proxy.clone(),
            self.offline.clone(),
        );

        if let Some((video_id, target_ms)) = effects.sponsor_skip {
            self.seek_ms(target_ms).await?;
            {
                let mut fsm = self.fsm.lock().await;
                fsm.snapshot.last_skipped_segment = Some(format!(
                    "Skipped non-music segment ({}s → {}s)",
                    position_ms / 1000,
                    target_ms / 1000
                ));
                self.publish_and_persist(&fsm);
            }
            self.bus.publish(AppEvent::SponsorBlockSkipped {
                video_id,
                from_ms: position_ms,
                to_ms: target_ms,
            });
            return Ok(());
        }

        let fsm = self.fsm.lock().await;
        self.publish_and_persist(&fsm);
        Ok(())
    }

    /// Handles track completion from the native audio driver (`RepeatMode::One`, queue advance,
    /// or `autoplay_next` radio expansion).
    pub async fn on_track_ended(&self) -> CoreResult<()> {
        let repeat = self.fsm.lock().await.snapshot.queue.repeat;
        if repeat == RepeatMode::One {
            return self.load_and_play_current(0).await;
        }
        self.next().await
    }

    /// Processes unified telemetry from the native audio executor (`nsh/audio` or platform bridge).
    ///
    /// Ignores stale telemetry whose `generation` does not match the active `play_generation`
    /// (preventing double-skips when switching tracks).
    pub async fn handle_audio_telemetry(&self, telemetry: AudioTelemetry) -> CoreResult<()> {
        let active_gen = self.play_generation.load(Ordering::SeqCst);
        let msg_gen = telemetry.generation();
        if msg_gen != 0 && active_gen != 0 && msg_gen != active_gen {
            return Ok(());
        }

        match telemetry {
            AudioTelemetry::Loaded { duration_ms, .. } => {
                if duration_ms > 0 {
                    let mut fsm = self.fsm.lock().await;
                    if fsm.snapshot.duration_ms == 0 {
                        fsm.snapshot.duration_ms = duration_ms;
                        self.publish_and_persist(&fsm);
                    }
                }
                Ok(())
            }
            AudioTelemetry::StateChanged { state, .. } => {
                let mut fsm = self.fsm.lock().await;
                if fsm.snapshot.current_track.is_some() {
                    let _ = fsm.snapshot.state.transition(state);
                    self.publish_and_persist(&fsm);
                }
                Ok(())
            }
            AudioTelemetry::Buffering { buffering, .. } => {
                let mut fsm = self.fsm.lock().await;
                if fsm.snapshot.current_track.is_some() {
                    if buffering {
                        let _ = fsm.snapshot.state.transition(PlayerState::Buffering);
                    } else if fsm.snapshot.state == PlayerState::Buffering {
                        let _ = fsm.snapshot.state.transition(PlayerState::Playing);
                    }
                    self.publish_and_persist(&fsm);
                }
                Ok(())
            }
            AudioTelemetry::Progress {
                position_ms,
                duration_ms,
                ..
            } => {
                if let Some(dur) = duration_ms.filter(|d| *d > 0) {
                    let mut fsm = self.fsm.lock().await;
                    if fsm.snapshot.duration_ms == 0 {
                        fsm.snapshot.duration_ms = dur;
                    }
                }
                self.tick_position(position_ms).await?;

                let last_sync = self.last_now_playing_sync_ms.load(Ordering::SeqCst);
                if position_ms.abs_diff(last_sync) >= 1_000 {
                    let sync_payload = {
                        let fsm = self.fsm.lock().await;
                        fsm.snapshot.current_track.as_ref().map(|t| {
                            (
                                NowPlayingMetadata {
                                    track_id: t.id.clone(),
                                    title: t.title.clone(),
                                    artist: t.artist_display(),
                                    album: t.album.as_ref().map(|a| a.title.clone()),
                                    artwork_url: t.artwork_url.clone(),
                                    duration_ms: fsm.snapshot.duration_ms,
                                },
                                fsm.snapshot.state.is_playing(),
                            )
                        })
                    };
                    if let Some((meta, playing)) = sync_payload {
                        self.last_now_playing_sync_ms
                            .store(position_ms, Ordering::SeqCst);
                        self.audio
                            .update_now_playing(meta, playing, position_ms)
                            .await;
                    }
                }
                Ok(())
            }
            AudioTelemetry::Ended { .. } => self.on_track_ended().await,
            AudioTelemetry::Failed { message, .. } => {
                let safe = redact_secrets(&message);
                let mut fsm = self.fsm.lock().await;
                fsm.enter_error(safe.clone());
                self.publish_and_persist(&fsm);
                self.bus.publish(AppEvent::Toast {
                    message: safe,
                    is_error: true,
                });
                Ok(())
            }
        }
    }

    /// Handles external system commands from Android MediaSession / BecomingNoisy / Desktop MPRIS.
    pub async fn handle_media_command(&self, cmd: MediaCommand) -> CoreResult<()> {
        match cmd {
            MediaCommand::Play => self.resume().await,
            MediaCommand::Pause => self.pause().await,
            MediaCommand::TogglePlayPause => self.toggle_play_pause().await,
            MediaCommand::Next => self.next().await,
            MediaCommand::Previous => self.previous().await,
            MediaCommand::SeekMs(ms) => self.seek_ms(ms).await,
            MediaCommand::Stop => self.pause().await,
            MediaCommand::BecomingNoisy => self.pause().await,
            MediaCommand::AudioFocusLostTransient => {
                let was_playing = self.fsm.lock().await.snapshot.state.is_playing();
                if was_playing {
                    self.pause().await?;
                    self.fsm.lock().await.paused_by_focus = true;
                }
                Ok(())
            }
            MediaCommand::AudioFocusGained => {
                let should_resume = {
                    let mut fsm = self.fsm.lock().await;
                    let v = fsm.paused_by_focus;
                    fsm.paused_by_focus = false;
                    v
                };
                if should_resume {
                    self.resume().await?;
                }
                Ok(())
            }
        }
    }
}
