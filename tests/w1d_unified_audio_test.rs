//! W1D Offline Verification Suite:
//! - Generation fencing blocks stale `Ended` and `Progress` telemetry events
//! - Progress telemetry triggers SponsorBlock once, 50% pre-resolution, 75% 4-chunk (1 MB) warm-up, and 5s SQLite resume persistence
//! - Seek-echo never occurs from `Progress` ticks
//! - Sleep timer countdown attenuates volume over last 3s, pauses at 0, and `EndOfTrack` pauses on track completion
//! - `SystemMediaControlsBridge` metadata/playback projection & `MediaCommand` dispatch
//! - `parse_generation_from_url` & `SetPlaybackSpeed` on `NshAudioBackend`

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use resonance::platform::desktop::SystemMediaControlsBridge;
use resonance::platform::nsh_audio::{NshAudioBackend, NshAudioControl, parse_generation_from_url};
use resonance::store::settings_slice::{AudioDspCapability, SettingsSlice};
use resonance_contract::{
    AudioBackend, AudioCodec, CacheStore, CoreResult, EventBus, HomeSection, LyricsDocument,
    LyricsProvider, MediaCommand, MusicProvider, NowPlayingMetadata, PlayerState, Playlist,
    SearchResults, SleepTimerState, SponsorBlockProvider, SponsorSegment, StreamInfo, Track,
};
use resonance_core::cache::{ChunkCache, SqliteCacheStore};
use resonance_core::fixture::fixture_tracks;
use resonance_core::playback::{AudioTelemetry, NEXT_TRACK_PREWARM_CHUNKS, PlaybackController};
use resonance_core::proxy::LoopbackProxy;
use tempfile::tempdir;
use tokio::sync::mpsc::UnboundedReceiver;

struct CountingProvider {
    resolve_calls: AtomicUsize,
    resolved_ids: Mutex<Vec<String>>,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            resolve_calls: AtomicUsize::new(0),
            resolved_ids: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl MusicProvider for CountingProvider {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        Ok(SearchResults {
            query: query.to_owned(),
            tracks: Vec::new(),
            albums: Vec::new(),
            continuation: None,
        })
    }

    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        Ok(Vec::new())
    }

    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Ok(Vec::new())
    }

    async fn browse_collection(&self, playlist_id: &str) -> CoreResult<Playlist> {
        Ok(Playlist {
            id: playlist_id.into(),
            title: "W1D Test".into(),
            subtitle: None,
            artwork_url: None,
            tracks: Vec::new(),
        })
    }

    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Ok(Vec::new())
    }

    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        self.resolve_calls.fetch_add(1, Ordering::SeqCst);
        self.resolved_ids.lock().unwrap().push(video_id.to_owned());
        Ok(StreamInfo {
            video_id: video_id.to_owned(),
            url: format!("http://127.0.0.1:1/upstream/{video_id}"),
            mime_type: "audio/webm".into(),
            itag: 251,
            codec: AudioCodec::Opus,
            bitrate_kbps: 160,
            content_length: 1_048_576,
            duration_ms: 100_000,
            loudness_db: None,
            expires_at_unix_secs: u64::MAX,
            resolved_by_client: "W1D_STUB".into(),
        })
    }

    async fn ytm_lyrics(&self, _video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

struct NullLyrics;

#[async_trait]
impl LyricsProvider for NullLyrics {
    async fn fetch_lyrics(&self, _track: &Track) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

struct FixedSponsorBlock(Vec<SponsorSegment>);

#[async_trait]
impl SponsorBlockProvider for FixedSponsorBlock {
    async fn fetch_segments(&self, _video_id: &str) -> CoreResult<Vec<SponsorSegment>> {
        Ok(self.0.clone())
    }
}

fn drain_controls(rx: &mut UnboundedReceiver<NshAudioControl>) -> Vec<NshAudioControl> {
    let mut out = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        out.push(cmd);
    }
    out
}

async fn make_controller(
    audio: Arc<NshAudioBackend>,
    segments: Vec<SponsorSegment>,
) -> (
    tempfile::TempDir,
    Arc<SqliteCacheStore>,
    Arc<CountingProvider>,
    PlaybackController,
) {
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(&dir.path().join("w1d.db")).unwrap());
    let chunks = Arc::new(
        ChunkCache::new(&dir.path().join("chunks"), cache.clone(), 16 * 1024 * 1024).unwrap(),
    );
    let provider = Arc::new(CountingProvider::new());
    let proxy = Arc::new(
        LoopbackProxy::start(provider.clone(), chunks)
            .await
            .unwrap(),
    );
    let controller = PlaybackController::new(
        provider.clone(),
        audio,
        cache.clone(),
        proxy,
        Arc::new(NullLyrics),
        Arc::new(FixedSponsorBlock(segments)),
        EventBus::new(64),
    );
    (dir, cache, provider, controller)
}

#[tokio::test]
async fn generation_fence_blocks_stale_ended_and_progress_events() {
    let backend = Arc::new(NshAudioBackend::new());
    let (_dir, _cache, _provider, controller) = make_controller(backend, Vec::new()).await;

    let mut tracks = fixture_tracks();
    tracks.truncate(3);
    let t2_id = tracks[1].id.clone();

    controller.play_tracks(tracks, 0).await.unwrap();
    assert_eq!(controller.current_generation(), 1);

    controller.next().await.unwrap();
    assert_eq!(controller.current_generation(), 2);

    // Stale progress + ended from gen 1 must be ignored.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 99_000,
            duration_ms: Some(100_000),
        })
        .await
        .unwrap();
    controller
        .handle_audio_telemetry(AudioTelemetry::Ended { generation: 1 })
        .await
        .unwrap();

    assert_eq!(controller.current_generation(), 2);
    let snap = controller.snapshot().await;
    assert_eq!(
        snap.current_track.as_ref().map(|t| t.id.as_str()),
        Some(t2_id.as_str())
    );
    assert_eq!(snap.position_ms, 0);
}

#[tokio::test]
async fn progress_telemetry_triggers_sponsorblock_50pct_75pct_and_5s_resume_persist() {
    assert_eq!(NEXT_TRACK_PREWARM_CHUNKS, 4);

    let backend = Arc::new(NshAudioBackend::new());
    let mut rx = backend.take_control_receiver().unwrap();
    let (_dir, cache, provider, controller) = make_controller(
        backend,
        vec![SponsorSegment {
            start_secs: 10.0,
            end_secs: 20.0,
            video_duration_secs: 100.0,
        }],
    )
    .await;

    let mut tracks = fixture_tracks();
    tracks.truncate(2);
    tracks[0].duration_ms = 100_000;
    tracks[1].duration_ms = 100_000;
    let t2_id = tracks[1].id.clone();

    controller.play_tracks(tracks, 0).await.unwrap();
    let badge = controller.codec_badge().await.expect("codec badge set");
    assert_eq!(badge.codec, AudioCodec::Opus);
    assert_eq!(badge.bitrate_kbps, 160);
    assert_eq!(badge.source_label, "W1D_STUB");
    drain_controls(&mut rx);

    // SponsorBlock triggers once.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 12_000,
            duration_ms: Some(100_000),
        })
        .await
        .unwrap();
    let sb_cmds = drain_controls(&mut rx);
    assert_eq!(
        sb_cmds
            .iter()
            .filter(|c| matches!(c, NshAudioControl::SeekMs(20_000)))
            .count(),
        1
    );

    // Progress at 22_000 ms (< 5s delta after SponsorBlock seek to 20_000) should not persist yet.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 22_000,
            duration_ms: Some(100_000),
        })
        .await
        .unwrap();
    let q_before = cache.load_queue().unwrap().unwrap();
    assert_eq!(q_before.resume_position_ms, 20_000);

    // Progress at 25_500 ms (>= 5s delta) persists resume_position_ms to SQLite.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 25_500,
            duration_ms: Some(100_000),
        })
        .await
        .unwrap();
    let q_after = cache.load_queue().unwrap().unwrap();
    assert_eq!(q_after.resume_position_ms, 25_500);

    // 50% pre-resolution.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 50_000,
            duration_ms: Some(100_000),
        })
        .await
        .unwrap();
    assert_eq!(
        controller.preresolved_next_video_id().await.as_deref(),
        Some(t2_id.as_str())
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(provider.resolve_calls.load(Ordering::SeqCst), 2);

    // 75% pre-warm.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 75_000,
            duration_ms: Some(100_000),
        })
        .await
        .unwrap();
    assert_eq!(
        controller.prewarmed_next_video_id().await.as_deref(),
        Some(t2_id.as_str())
    );
}

#[tokio::test]
async fn seek_echo_never_occurs_from_progress_ticks() {
    let backend = Arc::new(NshAudioBackend::new());
    let mut rx = backend.take_control_receiver().unwrap();
    let (_dir, _cache, _provider, controller) = make_controller(backend, Vec::new()).await;

    let mut tracks = fixture_tracks();
    tracks.truncate(1);
    tracks[0].duration_ms = 100_000;
    controller.play_tracks(tracks, 0).await.unwrap();
    drain_controls(&mut rx);

    for i in 1..=20u64 {
        controller
            .handle_audio_telemetry(AudioTelemetry::Progress {
                generation: 1,
                position_ms: i * 1_000,
                duration_ms: Some(100_000),
            })
            .await
            .unwrap();
    }

    let emitted = drain_controls(&mut rx);
    assert!(
        !emitted
            .iter()
            .any(|c| matches!(c, NshAudioControl::SeekMs(_))),
        "Progress ticks must never emit SeekMs: {emitted:?}"
    );
}

#[tokio::test]
async fn sleep_timer_countdown_fades_volume_pauses_and_end_of_track_stops() {
    let backend = Arc::new(NshAudioBackend::new());
    let mut rx = backend.take_control_receiver().unwrap();
    let (_dir, _cache, _provider, controller) = make_controller(backend, Vec::new()).await;

    let mut tracks = fixture_tracks();
    tracks.truncate(2);
    controller.play_tracks(tracks, 0).await.unwrap();
    controller.set_volume(0.9).await.unwrap();
    drain_controls(&mut rx);

    controller
        .set_sleep_timer(SleepTimerState::Countdown { remaining_secs: 4 })
        .await;
    // 4 -> 3s enters fade window (volume attenuated to 0.9 * 1.0 = 0.9, then 2s -> 0.6).
    controller.tick_sleep_timer(1).await.unwrap();
    controller.tick_sleep_timer(1).await.unwrap();
    let fade_cmds = drain_controls(&mut rx);
    assert!(
        fade_cmds
            .iter()
            .any(|c| matches!(c, NshAudioControl::SetVolume(v) if (*v - 0.6).abs() < 0.01)),
        "expected fade volume ~0.6, got {fade_cmds:?}"
    );

    // 2 -> 0s pauses playback and restores original 0.9 volume.
    controller.tick_sleep_timer(2).await.unwrap();
    assert_eq!(controller.snapshot().await.state, PlayerState::Paused);
    assert_eq!(controller.sleep_timer().await, SleepTimerState::Off);
    let expire_cmds = drain_controls(&mut rx);
    assert!(
        expire_cmds
            .iter()
            .any(|c| matches!(c, NshAudioControl::Pause)),
        "expected Pause on sleep timer expiry: {expire_cmds:?}"
    );
    assert!(
        expire_cmds
            .iter()
            .any(|c| matches!(c, NshAudioControl::SetVolume(v) if (*v - 0.9).abs() < 0.01)),
        "expected volume restore to 0.9: {expire_cmds:?}"
    );

    // EndOfTrack pauses instead of advancing to track #2.
    controller.resume().await.unwrap();
    controller
        .set_sleep_timer(SleepTimerState::EndOfTrack)
        .await;
    controller
        .handle_audio_telemetry(AudioTelemetry::Ended { generation: 1 })
        .await
        .unwrap();
    assert_eq!(controller.snapshot().await.state, PlayerState::Paused);
    assert_eq!(controller.current_generation(), 1);
    assert_eq!(controller.sleep_timer().await, SleepTimerState::Off);
}

#[tokio::test]
async fn system_media_controls_projects_metadata_and_handles_injected_commands() {
    let bridge = SystemMediaControlsBridge::new(None);
    let mut rx = bridge.take_command_receiver().unwrap();

    bridge.sync_metadata(&NowPlayingMetadata {
        track_id: "vid-1".into(),
        title: "Test Song".into(),
        artist: "Test Artist".into(),
        album: Some("Test Album".into()),
        artwork_url: None,
        duration_ms: 180_000,
    });
    bridge.sync_playback(true, 12_345, true);
    assert!(bridge.playing());
    assert_eq!(bridge.position_ms(), 12_345);

    bridge.inject_command(MediaCommand::TogglePlayPause);
    bridge.inject_command(MediaCommand::SeekMs(42_000));
    bridge.inject_command(MediaCommand::Next);

    assert_eq!(rx.recv().await, Some(MediaCommand::TogglePlayPause));
    assert_eq!(rx.recv().await, Some(MediaCommand::SeekMs(42_000)));
    assert_eq!(rx.recv().await, Some(MediaCommand::Next));
}

#[tokio::test]
async fn nsh_audio_bus_parses_generation_and_forwards_playback_speed() {
    assert_eq!(
        parse_generation_from_url("http://127.0.0.1:9000/stream/abc?itag=251&gen=42"),
        Some(42)
    );
    assert_eq!(
        parse_generation_from_url("http://127.0.0.1:9000/stream/abc?gen=7"),
        Some(7)
    );
    assert_eq!(
        parse_generation_from_url("http://127.0.0.1:9000/stream/abc?itag=251"),
        None
    );

    let backend = NshAudioBackend::new();
    let mut rx = backend.take_control_receiver().unwrap();
    backend.set_playback_speed(1.25).await.unwrap();
    assert_eq!(
        rx.recv().await,
        Some(NshAudioControl::SetPlaybackSpeed(1.25))
    );
}

#[test]
fn settings_dsp_capability_maps_truthfully_to_contract() {
    let settings = SettingsSlice::new(true);
    let cap = settings.dsp_capability();
    assert_eq!(cap, AudioDspCapability::ExternalUrlStreamOnly);
    assert_eq!(
        cap,
        resonance_contract::AudioDspCapability::ExternalUrlStreamOnly
    );
    assert!(!cap.is_active());
    assert!(AudioDspCapability::NativePcmPipeline.is_active());
    assert!(AudioDspCapability::AndroidAudioProcessorJni.is_active());
}
