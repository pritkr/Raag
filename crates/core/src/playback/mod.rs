//! Single Source-of-Truth `PlaybackController` state machine.
//!
//! Owns:
//! - Deterministic `PlayerState` (`Idle -> Loading -> Buffering -> Playing <-> Paused -> Recovering -> Error`)
//! - Persistent Queue (Play Next, Play Last, Reorder, Remove, Shuffle, Repeat None/One/All, Autoplay Radio)
//! - Local Listening History & Resume Position restoration across restarts
//! - SponsorBlock `music_offtopic` auto-skip
//! - MediaCommand dispatch (MediaSession / MPRIS / AudioFocus / BecomingNoisy)

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rand::seq::SliceRandom;
use resonance_contract::{
    AppEvent, AudioBackend, CacheStore, CoreError, CoreResult, EventBus, HistoryEntry,
    LyricsProvider, MediaCommand, MusicProvider, NowPlayingMetadata, OfflineStore, PlaybackError,
    PlayerSnapshot, PlayerState, QueueState, RepeatMode, SponsorBlockProvider, SponsorSegment,
    Track,
};
use tokio::sync::Mutex;

use crate::proxy::LoopbackProxy;
use crate::unix_secs;

pub struct PlaybackController {
    provider: Arc<dyn MusicProvider>,
    audio: Arc<dyn AudioBackend>,
    cache: Arc<dyn CacheStore>,
    offline: Option<Arc<dyn OfflineStore>>,
    proxy: Arc<LoopbackProxy>,
    lyrics: Arc<dyn LyricsProvider>,
    sponsorblock: Arc<dyn SponsorBlockProvider>,
    bus: EventBus,
    play_generation: AtomicU64,
    inner: Mutex<ControllerInner>,
}

struct ControllerInner {
    snapshot: PlayerSnapshot,
    sponsor_segments: Vec<SponsorSegment>,
    paused_by_focus: bool,
    /// Video ID of the upcoming queue track already pre-resolved at >= 50% progress.
    preresolved_video_id: Option<String>,
}

impl PlaybackController {
    pub fn new(
        provider: Arc<dyn MusicProvider>,
        audio: Arc<dyn AudioBackend>,
        cache: Arc<dyn CacheStore>,
        proxy: Arc<LoopbackProxy>,
        lyrics: Arc<dyn LyricsProvider>,
        sponsorblock: Arc<dyn SponsorBlockProvider>,
        bus: EventBus,
    ) -> Self {
        let restored_queue = cache.load_queue().ok().flatten().unwrap_or_default();
        let current_track = restored_queue.current_track().cloned();
        let resume_pos = restored_queue.resume_position_ms;
        let duration_ms = current_track.as_ref().map_or(0, |t| t.duration_ms);
        let initial_state = if current_track.is_some() {
            PlayerState::Paused
        } else {
            PlayerState::Idle
        };

        let snapshot = PlayerSnapshot {
            state: initial_state,
            current_track,
            position_ms: resume_pos,
            duration_ms,
            buffered_ms: 0,
            volume: 1.0,
            queue: restored_queue,
            active_proxy_url: None,
            sponsorblock_enabled: true,
            last_skipped_segment: None,
            offline_mode: false,
        };

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
            inner: Mutex::new(ControllerInner {
                snapshot,
                sponsor_segments: Vec::new(),
                paused_by_focus: false,
                preresolved_video_id: None,
            }),
        }
    }

    pub fn with_offline_store(mut self, offline: Arc<dyn OfflineStore>) -> Self {
        self.offline = Some(offline);
        self
    }

    pub async fn snapshot(&self) -> PlayerSnapshot {
        self.inner.lock().await.snapshot.clone()
    }

    /// Returns the video ID of the next track pre-resolved at the 50% progress threshold.
    pub async fn preresolved_next_video_id(&self) -> Option<String> {
        self.inner.lock().await.preresolved_video_id.clone()
    }

    pub fn event_bus(&self) -> EventBus {
        self.bus.clone()
    }

    fn publish_and_persist(&self, inner: &ControllerInner) {
        let _ = self.cache.save_queue(&inner.snapshot.queue);
        self.bus
            .publish(AppEvent::PlayerUpdated(inner.snapshot.clone()));
    }

    /// Replaces queue with `tracks` and starts playing track at `start_index`.
    pub async fn play_tracks(&self, tracks: Vec<Track>, start_index: usize) -> CoreResult<()> {
        if tracks.is_empty() {
            return Err(CoreError::Playback(PlaybackError::EmptyQueue));
        }
        let idx = start_index.min(tracks.len() - 1);
        {
            let mut inner = self.inner.lock().await;
            inner.snapshot.queue.items = tracks;
            inner.snapshot.queue.current_index = Some(idx);
            inner.snapshot.queue.resume_position_ms = 0;
        }
        self.load_and_play_current(0).await
    }

    /// Plays a single track immediately (appending or replacing if queue empty).
    pub async fn play_track(&self, track: Track) -> CoreResult<()> {
        self.play_tracks(vec![track], 0).await
    }

    /// Inserts a track immediately after the currently playing item.
    pub async fn play_next(&self, track: Track) -> CoreResult<()> {
        let mut inner = self.inner.lock().await;
        let insert_at = inner
            .snapshot
            .queue
            .current_index
            .map(|i| (i + 1).min(inner.snapshot.queue.items.len()))
            .unwrap_or(0);
        inner.snapshot.queue.items.insert(insert_at, track);
        if inner.snapshot.queue.current_index.is_none() {
            inner.snapshot.queue.current_index = Some(0);
        }
        self.publish_and_persist(&inner);
        Ok(())
    }

    /// Appends a track to the end of the queue.
    pub async fn play_last(&self, track: Track) -> CoreResult<()> {
        let mut inner = self.inner.lock().await;
        inner.snapshot.queue.items.push(track);
        if inner.snapshot.queue.current_index.is_none() {
            inner.snapshot.queue.current_index = Some(0);
        }
        self.publish_and_persist(&inner);
        Ok(())
    }

    /// Reorders an item in the queue, preserving the currently playing track index.
    pub async fn reorder_queue(&self, from: usize, to: usize) -> CoreResult<()> {
        let mut inner = self.inner.lock().await;
        let len = inner.snapshot.queue.items.len();
        if from >= len || to >= len || from == to {
            return Ok(());
        }
        let item = inner.snapshot.queue.items.remove(from);
        inner.snapshot.queue.items.insert(to, item);

        if let Some(cur) = inner.snapshot.queue.current_index {
            inner.snapshot.queue.current_index = Some(if cur == from {
                to
            } else if from < cur && cur <= to {
                cur - 1
            } else if to <= cur && cur < from {
                cur + 1
            } else {
                cur
            });
        }
        self.publish_and_persist(&inner);
        Ok(())
    }

    /// Removes an item at `index` from the queue.
    pub async fn remove_from_queue(&self, index: usize) -> CoreResult<()> {
        let mut inner = self.inner.lock().await;
        if index >= inner.snapshot.queue.items.len() {
            return Ok(());
        }
        inner.snapshot.queue.items.remove(index);
        if inner.snapshot.queue.items.is_empty() {
            inner.snapshot.queue.current_index = None;
            inner.snapshot.current_track = None;
            let _ = inner.snapshot.state.transition(PlayerState::Idle);
        } else if let Some(cur) = inner.snapshot.queue.current_index {
            if index < cur {
                inner.snapshot.queue.current_index = Some(cur - 1);
            } else if index == cur {
                let next_idx = cur.min(inner.snapshot.queue.items.len() - 1);
                inner.snapshot.queue.current_index = Some(next_idx);
                inner.snapshot.current_track = inner.snapshot.queue.items.get(next_idx).cloned();
            }
        }
        self.publish_and_persist(&inner);
        Ok(())
    }

    pub async fn clear_queue(&self) -> CoreResult<()> {
        let _ = self.audio.stop().await;
        let mut inner = self.inner.lock().await;
        inner.snapshot.queue = QueueState {
            shuffle: inner.snapshot.queue.shuffle,
            repeat: inner.snapshot.queue.repeat,
            autoplay_radio: inner.snapshot.queue.autoplay_radio,
            ..QueueState::default()
        };
        inner.snapshot.current_track = None;
        inner.snapshot.position_ms = 0;
        inner.snapshot.duration_ms = 0;
        inner.snapshot.active_proxy_url = None;
        let _ = inner.snapshot.state.transition(PlayerState::Idle);
        self.publish_and_persist(&inner);
        Ok(())
    }

    pub async fn toggle_shuffle(&self) {
        let mut inner = self.inner.lock().await;
        inner.snapshot.queue.shuffle = !inner.snapshot.queue.shuffle;
        if inner.snapshot.queue.shuffle && inner.snapshot.queue.items.len() > 2 {
            let cur_idx = inner.snapshot.queue.current_index.unwrap_or(0);
            let tail_start = (cur_idx + 1).min(inner.snapshot.queue.items.len());
            let mut rng = rand::rng();
            inner.snapshot.queue.items[tail_start..].shuffle(&mut rng);
        }
        self.publish_and_persist(&inner);
    }

    pub async fn cycle_repeat(&self) {
        let mut inner = self.inner.lock().await;
        inner.snapshot.queue.repeat = inner.snapshot.queue.repeat.cycle();
        self.publish_and_persist(&inner);
    }

    pub async fn set_autoplay_radio(&self, enabled: bool) {
        let mut inner = self.inner.lock().await;
        inner.snapshot.queue.autoplay_radio = enabled;
        self.publish_and_persist(&inner);
    }

    pub async fn set_sponsorblock_enabled(&self, enabled: bool) {
        let mut inner = self.inner.lock().await;
        inner.snapshot.sponsorblock_enabled = enabled;
        self.publish_and_persist(&inner);
    }

    pub async fn jump_to_queue_index(&self, index: usize) -> CoreResult<()> {
        {
            let mut inner = self.inner.lock().await;
            if index >= inner.snapshot.queue.items.len() {
                return Err(CoreError::Playback(PlaybackError::EmptyQueue));
            }
            inner.snapshot.queue.current_index = Some(index);
            inner.snapshot.queue.resume_position_ms = 0;
        }
        self.load_and_play_current(0).await
    }

    async fn load_and_play_current(&self, resume_ms: u64) -> CoreResult<()> {
        let my_gen = self.play_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let proxy_gen = self.proxy.advance_generation();

        let track = {
            let mut inner = self.inner.lock().await;
            let Some(t) = inner.snapshot.queue.current_track().cloned() else {
                return Err(CoreError::Playback(PlaybackError::EmptyQueue));
            };
            inner.snapshot.current_track = Some(t.clone());
            inner.snapshot.position_ms = resume_ms;
            inner.snapshot.duration_ms = t.duration_ms;
            inner.snapshot.last_skipped_segment = None;
            inner.sponsor_segments.clear();
            inner.preresolved_video_id = None;
            let _ = inner.snapshot.state.transition(PlayerState::Loading);
            self.publish_and_persist(&inner);
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
                    self.proxy.register_stream(info);
                    (
                        self.proxy.stream_url_with_gen(&track.id, itag, proxy_gen),
                        dur,
                        false,
                    )
                }
                Err(e) => {
                    // If offline, attempt direct loopback proxy URL against cached chunks (itag 251).
                    if matches!(e, CoreError::Provider(_) | CoreError::Offline(_)) {
                        (
                            self.proxy.stream_url_with_gen(&track.id, 251, proxy_gen),
                            track.duration_ms,
                            true,
                        )
                    } else {
                        let mut inner = self.inner.lock().await;
                        let _ = inner
                            .snapshot
                            .state
                            .transition(PlayerState::Error(e.to_string()));
                        self.publish_and_persist(&inner);
                        return Err(e);
                    }
                }
            }
        };

        if self.play_generation.load(Ordering::SeqCst) != my_gen {
            return Ok(());
        }

        {
            let mut inner = self.inner.lock().await;
            inner.snapshot.active_proxy_url = Some(proxy_url.clone());
            inner.snapshot.duration_ms = duration_ms;
            inner.snapshot.offline_mode = offline_used;
            let _ = inner.snapshot.state.transition(PlayerState::Buffering);
            self.publish_and_persist(&inner);
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
            let mut inner = self.inner.lock().await;
            let _ = inner
                .snapshot
                .state
                .transition(PlayerState::Error(e.to_string()));
            self.publish_and_persist(&inner);
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
            let mut inner = self.inner.lock().await;
            let _ = inner.snapshot.state.transition(PlayerState::Playing);
            self.publish_and_persist(&inner);
        }

        // Fetch lyrics and SponsorBlock segments asynchronously without blocking playback start.
        let lyrics_provider = self.lyrics.clone();
        let sb_provider = self.sponsorblock.clone();
        let bus = self.bus.clone();
        let track_for_bg = track.clone();
        tokio::spawn(async move {
            let lyrics = lyrics_provider.fetch_lyrics(&track_for_bg).await.ok().flatten();
            bus.publish(AppEvent::LyricsLoaded {
                video_id: track_for_bg.id.clone(),
                lyrics,
            });
        });

        let segs = sb_provider.fetch_segments(&track.id).await.unwrap_or_default();
        if !segs.is_empty() {
            let mut inner = self.inner.lock().await;
            if inner
                .snapshot
                .current_track
                .as_ref()
                .is_some_and(|t| t.id == track.id)
            {
                inner.sponsor_segments = segs;
            }
        }

        Ok(())
    }

    pub async fn pause(&self) -> CoreResult<()> {
        self.audio.pause().await?;
        let mut inner = self.inner.lock().await;
        inner.paused_by_focus = false;
        let _ = inner.snapshot.state.transition(PlayerState::Paused);
        inner.snapshot.queue.resume_position_ms = inner.snapshot.position_ms;
        if let Some(t) = &inner.snapshot.current_track {
            self.audio
                .update_now_playing(
                    NowPlayingMetadata {
                        track_id: t.id.clone(),
                        title: t.title.clone(),
                        artist: t.artist_display(),
                        album: t.album.as_ref().map(|a| a.title.clone()),
                        artwork_url: t.artwork_url.clone(),
                        duration_ms: inner.snapshot.duration_ms,
                    },
                    false,
                    inner.snapshot.position_ms,
                )
                .await;
        }
        self.publish_and_persist(&inner);
        Ok(())
    }

    pub async fn resume(&self) -> CoreResult<()> {
        let needs_load = {
            let inner = self.inner.lock().await;
            inner.snapshot.active_proxy_url.is_none() && inner.snapshot.current_track.is_some()
        };
        if needs_load {
            let resume_pos = self.inner.lock().await.snapshot.position_ms;
            return self.load_and_play_current(resume_pos).await;
        }
        self.audio.play().await?;
        let mut inner = self.inner.lock().await;
        inner.paused_by_focus = false;
        let _ = inner.snapshot.state.transition(PlayerState::Playing);
        self.publish_and_persist(&inner);
        Ok(())
    }

    pub async fn toggle_play_pause(&self) -> CoreResult<()> {
        let is_playing = self.inner.lock().await.snapshot.state.is_playing();
        if is_playing {
            self.pause().await
        } else {
            self.resume().await
        }
    }

    pub async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        let clamped = {
            let inner = self.inner.lock().await;
            if inner.snapshot.duration_ms > 0 {
                position_ms.min(inner.snapshot.duration_ms)
            } else {
                position_ms
            }
        };
        self.audio.seek_ms(clamped).await?;
        let mut inner = self.inner.lock().await;
        inner.snapshot.position_ms = clamped;
        inner.snapshot.queue.resume_position_ms = clamped;
        self.publish_and_persist(&inner);
        Ok(())
    }

    pub async fn next(&self) -> CoreResult<()> {
        let action = {
            let inner = self.inner.lock().await;
            let len = inner.snapshot.queue.items.len();
            if len == 0 {
                return Err(CoreError::Playback(PlaybackError::EmptyQueue));
            }
            let cur = inner.snapshot.queue.current_index.unwrap_or(0);
            match inner.snapshot.queue.repeat {
                RepeatMode::One => Some(cur),
                RepeatMode::All => Some((cur + 1) % len),
                RepeatMode::None => {
                    if cur + 1 < len {
                        Some(cur + 1)
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(next_idx) = action {
            return self.jump_to_queue_index(next_idx).await;
        }

        // End of queue: check Autoplay Radio (`next`).
        let (autoplay, seed_id) = {
            let inner = self.inner.lock().await;
            (
                inner.snapshot.queue.autoplay_radio,
                inner.snapshot.current_track.as_ref().map(|t| t.id.clone()),
            )
        };
        if autoplay
            && let Some(video_id) = seed_id
            && let Ok(radio) = self.provider.autoplay_next(&video_id).await
        {
            let next_idx = {
                let mut inner = self.inner.lock().await;
                let existing_ids: std::collections::HashSet<String> = inner
                    .snapshot
                    .queue
                    .items
                    .iter()
                    .map(|t| t.id.clone())
                    .collect();
                let start = inner.snapshot.queue.items.len();
                for t in radio {
                    if !existing_ids.contains(&t.id) {
                        inner.snapshot.queue.items.push(t);
                    }
                }
                (inner.snapshot.queue.items.len() > start).then_some(start)
            };
            if let Some(idx) = next_idx {
                return self.jump_to_queue_index(idx).await;
            }
        }

        self.pause().await
    }

    pub async fn previous(&self) -> CoreResult<()> {
        let pos = self.inner.lock().await.snapshot.position_ms;
        if pos > 3_000 {
            return self.seek_ms(0).await;
        }
        let prev_idx = {
            let inner = self.inner.lock().await;
            let len = inner.snapshot.queue.items.len();
            if len == 0 {
                return Err(CoreError::Playback(PlaybackError::EmptyQueue));
            }
            let cur = inner.snapshot.queue.current_index.unwrap_or(0);
            if cur > 0 {
                cur - 1
            } else if inner.snapshot.queue.repeat == RepeatMode::All {
                len - 1
            } else {
                0
            }
        };
        self.jump_to_queue_index(prev_idx).await
    }

    /// Periodic tick called on playback progress; persists resume position, enforces
    /// SponsorBlock (`music_offtopic`) auto-skip when enabled, and pre-resolves the next
    /// queued track once playback reaches >= 50% of the current track's duration.
    pub async fn tick_position(&self, position_ms: u64) -> CoreResult<()> {
        let (skip_target, preresolve_candidate) = {
            let mut inner = self.inner.lock().await;
            inner.snapshot.position_ms = position_ms;
            inner.snapshot.queue.resume_position_ms = position_ms;

            let dur_ms = inner.snapshot.duration_ms;
            let dur_secs = dur_ms as f64 / 1000.0;
            let skip = if inner.snapshot.sponsorblock_enabled {
                inner
                    .sponsor_segments
                    .iter()
                    .find(|seg| seg.fits_duration(dur_secs) && seg.contains_ms(position_ms))
                    .map(|seg| {
                        (
                            inner
                                .snapshot
                                .current_track
                                .as_ref()
                                .map(|t| t.id.clone())
                                .unwrap_or_default(),
                            (seg.end_secs * 1000.0) as u64,
                        )
                    })
            } else {
                None
            };

            // Trigger 50% next-track pre-resolution once per track for near-instant gapless switches.
            let mut preresolve = None;
            if dur_ms > 0 && position_ms.saturating_mul(2) >= dur_ms {
                let len = inner.snapshot.queue.items.len();
                let next_idx = inner.snapshot.queue.current_index.and_then(|cur| {
                    match inner.snapshot.queue.repeat {
                        RepeatMode::One => None,
                        RepeatMode::All if len > 1 => Some((cur + 1) % len),
                        RepeatMode::None if cur + 1 < len => Some(cur + 1),
                        _ => None,
                    }
                });
                let next_id = next_idx
                    .and_then(|idx| inner.snapshot.queue.items.get(idx))
                    .map(|t| t.id.clone());
                if let Some(id) = next_id
                    && inner.preresolved_video_id.as_deref() != Some(id.as_str())
                {
                    inner.preresolved_video_id = Some(id.clone());
                    preresolve = Some(id);
                }
            }

            (skip, preresolve)
        };

        if let Some(next_video_id) = preresolve_candidate {
            let already_offline = self
                .offline
                .as_ref()
                .and_then(|o| o.get_completed_download(&next_video_id).ok().flatten())
                .is_some();
            if !already_offline {
                let provider = self.provider.clone();
                let proxy = self.proxy.clone();
                tokio::spawn(async move {
                    if let Ok(stream_info) = provider.resolve_stream(&next_video_id).await {
                        proxy.register_stream(stream_info);
                    }
                });
            }
        }

        if let Some((video_id, target_ms)) = skip_target {
            self.seek_ms(target_ms).await?;
            {
                let mut inner = self.inner.lock().await;
                inner.snapshot.last_skipped_segment = Some(format!(
                    "Skipped non-music segment ({}s → {}s)",
                    position_ms / 1000,
                    target_ms / 1000
                ));
                self.publish_and_persist(&inner);
            }
            self.bus.publish(AppEvent::SponsorBlockSkipped {
                video_id,
                from_ms: position_ms,
                to_ms: target_ms,
            });
            return Ok(());
        }

        let inner = self.inner.lock().await;
        self.publish_and_persist(&inner);
        Ok(())
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
            MediaCommand::BecomingNoisy => {
                // Headphone unplug: immediately pause playback.
                self.pause().await
            }
            MediaCommand::AudioFocusLostTransient => {
                let was_playing = self.inner.lock().await.snapshot.state.is_playing();
                if was_playing {
                    self.pause().await?;
                    self.inner.lock().await.paused_by_focus = true;
                }
                Ok(())
            }
            MediaCommand::AudioFocusGained => {
                let should_resume = {
                    let mut inner = self.inner.lock().await;
                    let v = inner.paused_by_focus;
                    inner.paused_by_focus = false;
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
