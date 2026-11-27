//! Async runtime bridge (`PlaybackController`) coordinating [`PlaybackFsm`],
//! [`QueueManager`], [`PrefetchCoordinator`], [`LoopbackProxy`], and platform audio backends.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use resonance_contract::{
    AppEvent, AudioBackend, AudioCodec, CacheStore, CodecBadgeInfo, CoreError, CoreResult,
    EventBus, HistoryEntry, LyricsProvider, MediaCommand, MusicProvider, NowPlayingMetadata,
    OfflineStore, PlaybackError, PlayerSnapshot, PlayerState, RepeatMode, SleepTimerState, Track,
};
use tokio::sync::Mutex;

use crate::innertube::auth::redact_secrets;
use crate::playback::fsm::{PlaybackFsm, SleepTimerTickEffect};
use crate::playback::prefetch::PrefetchCoordinator;
use crate::playback::queue::QueueManager;
use crate::proxy::LoopbackProxy;
use crate::unix_secs;

pub use resonance_contract::AudioTelemetry;

pub struct PlaybackController {
    provider: Arc<dyn MusicProvider>,
    audio: Arc<dyn AudioBackend>,
    cache: Arc<dyn CacheStore>,
    offline: Option<Arc<dyn OfflineStore>>,
    local_media: Option<Arc<crate::local_media::LocalMediaStore>>,
    proxy: Arc<LoopbackProxy>,
    lyrics: Arc<dyn LyricsProvider>,
    sponsorblock: Arc<dyn resonance_contract::SponsorBlockProvider>,
    bus: EventBus,
    play_generation: AtomicU64,
    pause_epoch: AtomicU64,
    last_now_playing_sync_ms: AtomicU64,
    last_resume_persist_ms: AtomicU64,
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
            local_media: None,
            proxy,
            lyrics,
            sponsorblock,
            bus,
            play_generation: AtomicU64::new(0),
            pause_epoch: AtomicU64::new(0),
            last_now_playing_sync_ms: AtomicU64::new(0),
            last_resume_persist_ms: AtomicU64::new(0),
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

    pub fn with_local_media(
        mut self,
        local_media: Arc<crate::local_media::LocalMediaStore>,
    ) -> Self {
        self.local_media = Some(local_media);
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

    pub async fn codec_badge(&self) -> Option<CodecBadgeInfo> {
        self.fsm.lock().await.codec_badge.clone()
    }

    pub async fn set_sleep_timer(&self, state: SleepTimerState) {
        let mut fsm = self.fsm.lock().await;
        fsm.set_sleep_timer(state);
    }

    pub async fn sleep_timer(&self) -> SleepTimerState {
        self.fsm.lock().await.sleep_timer
    }

    pub async fn tick_sleep_timer(&self, elapsed_secs: u64) -> CoreResult<()> {
        let effect = {
            let mut fsm = self.fsm.lock().await;
            fsm.tick_sleep_timer(elapsed_secs)
        };
        match effect {
            SleepTimerTickEffect::None => Ok(()),
            SleepTimerTickEffect::AttenuateVolume(vol) => self.audio.set_volume(vol).await,
            SleepTimerTickEffect::ExpireAndPause { restore_volume } => {
                self.pause().await?;
                self.audio.set_volume(restore_volume).await
            }
        }
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
        let mut should_stop_audio = false;
        let mut should_reload_active = false;
        {
            let mut fsm = self.fsm.lock().await;
            let was_playing = fsm.snapshot.state.is_playing();
            match QueueManager::remove(&mut fsm.snapshot.queue, index) {
                Some(None) => {
                    self.play_generation.fetch_add(1, Ordering::SeqCst);
                    self.proxy.advance_generation();
                    fsm.clear_to_idle();
                    should_stop_audio = true;
                }
                Some(Some(next_track)) => {
                    fsm.snapshot.current_track = Some(next_track);
                    fsm.snapshot.position_ms = 0;
                    fsm.snapshot.queue.resume_position_ms = 0;
                    fsm.snapshot.active_proxy_url = None;
                    if was_playing {
                        should_reload_active = true;
                    } else {
                        self.play_generation.fetch_add(1, Ordering::SeqCst);
                        self.proxy.advance_generation();
                        should_stop_audio = true;
                    }
                }
                None => {}
            }
            self.publish_and_persist(&fsm);
        }
        if should_reload_active {
            return self.load_and_play_current(0).await;
        }
        if should_stop_audio {
            let _ = self.audio.stop().await;
        }
        Ok(())
    }

    pub async fn clear_queue(&self) -> CoreResult<()> {
        self.play_generation.fetch_add(1, Ordering::SeqCst);
        self.proxy.advance_generation();
        let _ = self.audio.stop().await;
        let mut fsm = self.fsm.lock().await;
        fsm.clear_to_idle();
        self.publish_and_persist(&fsm);
        Ok(())
    }

    /// Clears upcoming tracks after the currently playing track without interrupting audio playback.
    pub async fn clear_up_next(&self) -> CoreResult<usize> {
        let mut fsm = self.fsm.lock().await;
        let removed = QueueManager::clear_up_next(&mut fsm.snapshot.queue);
        self.publish_and_persist(&fsm);
        Ok(removed)
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
        let start_pause_epoch = self.pause_epoch.load(Ordering::SeqCst);
        self.last_now_playing_sync_ms.store(0, Ordering::SeqCst);
        self.last_resume_persist_ms
            .store(resume_ms, Ordering::SeqCst);
        let proxy_gen = self.proxy.advance_generation();

        let track = {
            let mut fsm = self.fsm.lock().await;
            let t = fsm.begin_loading(resume_ms)?;
            self.publish_and_persist(&fsm);
            t
        };

        let is_local_track = track.id.starts_with("local:");
        let local_record = if is_local_track {
            self.local_media
                .as_ref()
                .and_then(|lm| lm.get_track_by_id(&track.id).ok().flatten())
        } else {
            None
        };

        let offline_hit = if is_local_track {
            None
        } else {
            self.offline.as_ref().and_then(|o| {
                let rec = o.get_completed_download(&track.id).ok().flatten()?;
                let rel = rec.relative_path.as_deref()?;
                if crate::offline::resolve_confined_path(&o.downloads_root(), rel)
                    .is_ok_and(|p| p.is_file())
                {
                    Some(rec)
                } else {
                    None
                }
            })
        };

        let (proxy_url, duration_ms, offline_used, codec_badge) = if is_local_track {
            let (dur, bitrate, sample_rate, codec) = match local_record.as_ref() {
                Some(rec) => {
                    let codec = match rec.mime_type.as_str() {
                        "audio/mp4" => AudioCodec::Aac,
                        "audio/ogg" => AudioCodec::Opus,
                        _ => AudioCodec::Unknown,
                    };
                    (
                        rec.track.duration_ms.max(track.duration_ms),
                        rec.bitrate_kbps.unwrap_or(0),
                        rec.sample_rate_hz,
                        codec,
                    )
                }
                None => (track.duration_ms, 0, None, AudioCodec::Unknown),
            };
            (
                format!(
                    "{}/local/{}?gen={proxy_gen}",
                    self.proxy.base_url(),
                    track.id
                ),
                dur,
                true,
                Some(CodecBadgeInfo {
                    codec,
                    bitrate_kbps: bitrate,
                    sample_rate_hz: sample_rate,
                    source_label: "Local File".to_owned(),
                }),
            )
        } else if let Some(rec) = offline_hit {
            let codec = if matches!(rec.itag, 140 | 141) {
                AudioCodec::Aac
            } else {
                AudioCodec::Opus
            };
            (
                self.proxy
                    .stream_url_with_gen(&track.id, rec.itag, proxy_gen),
                track.duration_ms,
                true,
                Some(CodecBadgeInfo {
                    codec,
                    bitrate_kbps: if rec.itag == 140 { 128 } else { 160 },
                    sample_rate_hz: Some(48_000),
                    source_label: "Offline Vault".to_owned(),
                }),
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
                    let badge = CodecBadgeInfo {
                        codec: info.codec,
                        bitrate_kbps: info.bitrate_kbps,
                        sample_rate_hz: Some(48_000),
                        source_label: info.resolved_by_client.clone(),
                    };
                    self.proxy.register_current_stream(info);
                    (
                        self.proxy.stream_url_with_gen(&track.id, itag, proxy_gen),
                        dur,
                        false,
                        Some(badge),
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
                            Some(CodecBadgeInfo {
                                codec: if fallback_itag == 140 {
                                    AudioCodec::Aac
                                } else {
                                    AudioCodec::Opus
                                },
                                bitrate_kbps: if fallback_itag == 140 { 128 } else { 160 },
                                sample_rate_hz: Some(48_000),
                                source_label: "ChunkCache".to_owned(),
                            }),
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

        if self.pause_epoch.load(Ordering::SeqCst) != start_pause_epoch {
            let mut fsm = self.fsm.lock().await;
            if self.play_generation.load(Ordering::SeqCst) == my_gen {
                fsm.codec_badge = codec_badge;
                fsm.snapshot.duration_ms = duration_ms;
                fsm.snapshot.active_proxy_url = None;
                fsm.enter_paused();
                self.publish_and_persist(&fsm);
            }
            return Ok(());
        }

        {
            let mut fsm = self.fsm.lock().await;
            fsm.codec_badge = codec_badge;
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
            if self.play_generation.load(Ordering::SeqCst) != my_gen {
                return Ok(());
            }
            let mut fsm = self.fsm.lock().await;
            fsm.enter_error(e.to_string());
            self.publish_and_persist(&fsm);
            return Err(e);
        }

        if self.play_generation.load(Ordering::SeqCst) != my_gen {
            let queue_empty = self.fsm.lock().await.snapshot.current_track.is_none();
            if queue_empty {
                let _ = self.audio.stop().await;
            }
            return Ok(());
        }

        if self.pause_epoch.load(Ordering::SeqCst) != start_pause_epoch {
            let _ = self.audio.pause().await;
            let mut fsm = self.fsm.lock().await;
            if self.play_generation.load(Ordering::SeqCst) == my_gen {
                fsm.enter_paused();
                self.publish_and_persist(&fsm);
            }
            return Ok(());
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
            if self.play_generation.load(Ordering::SeqCst) != my_gen {
                return Ok(());
            }
            if self.pause_epoch.load(Ordering::SeqCst) != start_pause_epoch {
                fsm.enter_paused();
                self.publish_and_persist(&fsm);
                return Ok(());
            }
            fsm.enter_playing();
            self.publish_and_persist(&fsm);
        }

        // Fetch lyrics and SponsorBlock segments asynchronously without blocking playback start.
        let lyrics_provider = self.lyrics.clone();
        let sb_provider = self.sponsorblock.clone();
        let bus = self.bus.clone();
        let track_for_bg = track.clone();
        let embedded_local_lyrics = local_record.and_then(|r| r.embedded_lyrics);
        tokio::spawn(async move {
            let lyrics = if let Some(raw_lrc) = embedded_local_lyrics {
                let parsed = crate::lyrics::parse_lrc(&raw_lrc);
                if parsed.is_empty() {
                    let plain_lines: Vec<resonance_contract::LyricLine> = raw_lrc
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.is_empty())
                        .map(|l| resonance_contract::LyricLine {
                            start_ms: 0,
                            end_ms: 0,
                            text: l.to_owned(),
                            words: Vec::new(),
                            is_instrumental: false,
                        })
                        .collect();
                    if plain_lines.is_empty() {
                        None
                    } else {
                        Some(resonance_contract::LyricsDocument {
                            video_id: track_for_bg.id.clone(),
                            provider: "Embedded / Local".to_owned(),
                            sync_kind: resonance_contract::LyricSyncKind::Plain,
                            lines: plain_lines,
                        })
                    }
                } else {
                    Some(resonance_contract::LyricsDocument {
                        video_id: track_for_bg.id.clone(),
                        provider: "Embedded / Sidecar LRC".to_owned(),
                        sync_kind: resonance_contract::LyricSyncKind::Line,
                        lines: parsed,
                    })
                }
            } else if is_local_track {
                None
            } else {
                lyrics_provider
                    .fetch_lyrics(&track_for_bg)
                    .await
                    .ok()
                    .flatten()
            };
            bus.publish(AppEvent::LyricsLoaded {
                video_id: track_for_bg.id.clone(),
                lyrics,
            });
        });

        if !is_local_track {
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
        }

        Ok(())
    }

    pub async fn pause(&self) -> CoreResult<()> {
        self.pause_epoch.fetch_add(1, Ordering::SeqCst);
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
        self.last_resume_persist_ms.store(clamped, Ordering::SeqCst);
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
        let before_radio_gen = self.play_generation.load(Ordering::SeqCst);
        let (autoplay, seed_id) = {
            let fsm = self.fsm.lock().await;
            (
                fsm.snapshot.queue.autoplay_radio,
                fsm.snapshot.current_track.as_ref().map(|t| t.id.clone()),
            )
        };
        if autoplay
            && let Some(video_id) = seed_id
            && let Ok(radio) = self.fetch_diverse_radio(&video_id).await
        {
            if self.play_generation.load(Ordering::SeqCst) != before_radio_gen {
                return Ok(());
            }
            let next_idx = {
                let mut fsm = self.fsm.lock().await;
                QueueManager::merge_radio_tracks(&mut fsm.snapshot.queue, radio)
            };
            if let Some(idx) = next_idx {
                return self.jump_to_queue_index(idx).await;
            }
        }

        if self.play_generation.load(Ordering::SeqCst) != before_radio_gen {
            return Ok(());
        }
        self.pause().await
    }

    /// Fetches autoplay radio candidates for `seed_video_id`, expanding with a secondary
    /// artist seed when the initial response has low artist diversity.
    async fn fetch_diverse_radio(&self, seed_video_id: &str) -> CoreResult<Vec<Track>> {
        let mut tracks = self.provider.autoplay_next(seed_video_id).await?;
        let distinct_artists: std::collections::HashSet<String> = tracks
            .iter()
            .filter(|t| t.id != seed_video_id)
            .map(|t| t.artist_display().trim().to_lowercase())
            .filter(|a| !a.is_empty() && a != "unknown artist")
            .collect();

        if distinct_artists.len() < 4 || tracks.len() < 10 {
            if let Some(secondary_seed) = tracks
                .iter()
                .find(|t| t.id != seed_video_id)
                .map(|t| t.id.clone())
                && let Ok(more) = self.provider.autoplay_next(&secondary_seed).await
            {
                tracks.extend(more);
            }
        }
        Ok(tracks)
    }

    /// Automatically populates or tops up the Up Next queue when `autoplay_radio` is enabled
    /// and fewer than 5 upcoming tracks remain after the current track.
    pub async fn expand_autoplay_queue(&self, seed_video_id: &str) -> CoreResult<usize> {
        let should_expand = {
            let fsm = self.fsm.lock().await;
            let q = &fsm.snapshot.queue;
            if !q.autoplay_radio || q.repeat == RepeatMode::One {
                false
            } else {
                let remaining = q
                    .current_index
                    .map(|cur| q.items.len().saturating_sub(cur + 1))
                    .unwrap_or(0);
                remaining < 5
            }
        };
        if !should_expand {
            return Ok(0);
        }

        let radio = self.fetch_diverse_radio(seed_video_id).await?;
        if radio.is_empty() {
            return Ok(0);
        }

        let mut fsm = self.fsm.lock().await;
        let still_relevant = fsm
            .snapshot
            .current_track
            .as_ref()
            .is_some_and(|t| t.id == seed_video_id)
            || fsm
                .snapshot
                .queue
                .items
                .iter()
                .any(|t| t.id == seed_video_id);
        if !still_relevant {
            return Ok(0);
        }

        let before = fsm.snapshot.queue.items.len();
        QueueManager::merge_radio_tracks(&mut fsm.snapshot.queue, radio);
        let added = fsm.snapshot.queue.items.len().saturating_sub(before);
        if added > 0 {
            self.publish_and_persist(&fsm);
        }
        Ok(added)
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

        if effects.reached_end {
            return self.on_track_ended().await;
        }

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
        // Throttle SQLite queue resume persistence to every 5s of playback delta.
        let last_persist = self.last_resume_persist_ms.load(Ordering::SeqCst);
        if position_ms.abs_diff(last_persist) >= 5_000 {
            self.last_resume_persist_ms
                .store(position_ms, Ordering::SeqCst);
            let _ = self.cache.save_queue(&fsm.snapshot.queue);
        }
        self.bus
            .publish(AppEvent::PlayerUpdated(fsm.snapshot.clone()));
        Ok(())
    }

    /// Handles track completion from the native audio driver (`SleepTimerState::EndOfTrack`,
    /// `RepeatMode::One`, queue advance, or `autoplay_next` radio expansion).
    pub async fn on_track_ended(&self) -> CoreResult<()> {
        let (end_of_track_sleep, repeat) = {
            let mut fsm = self.fsm.lock().await;
            let eot = matches!(fsm.sleep_timer, SleepTimerState::EndOfTrack);
            if eot {
                fsm.set_sleep_timer(SleepTimerState::Off);
            }
            (eot, fsm.snapshot.queue.repeat)
        };
        if end_of_track_sleep {
            return self.pause().await;
        }
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
                    if fsm.snapshot.duration_ms != duration_ms {
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
                    if fsm.snapshot.duration_ms != dur {
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
