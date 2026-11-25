//! ST-2 Offline Verification Suite:
//! - Unified `PlaybackController` <-> `NshAudioBackend` command & telemetry roundtrip
//!   (SponsorBlock single-shot auto-skip, 50% next-track prefetch, track-end advance)
//! - Stale-generation `AudioTelemetry` rejection (prevents double-skip on rapid switch)
//! - `SystemMediaControlsBridge` metadata/playback projection & `MediaCommand` dispatch
//! - Truthful `AudioDspCapability` gating (`ExternalUrlStreamOnly`)
//! - `--features desktop-native` headless initialization & `0o600` keyring fallback

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
#[cfg(all(
    feature = "desktop-keyring",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use resonance::platform::desktop::OsKeyringVault;
use resonance::platform::desktop::{DesktopAuthStore, SecretVault, SystemMediaControlsBridge};
use resonance::platform::nsh_audio::{NshAudioBackend, NshAudioControl};
use resonance::store::settings_slice::{AudioDspCapability, SettingsSlice};
use resonance_contract::{
    AudioBackend, AudioCodec, AuthSession, AuthStore, CoreResult, CredentialBackendKind, EventBus,
    HomeSection, LyricsDocument, LyricsProvider, MediaCommand, MusicProvider, PlayerState,
    Playlist, SearchResults, SponsorBlockProvider, SponsorSegment, StreamInfo, Track,
};
use resonance_core::audio_dsp::EqPreset;
use resonance_core::cache::{ChunkCache, SqliteCacheStore};
use resonance_core::fixture::fixture_tracks;
use resonance_core::innertube::FileAuthStore;
use resonance_core::playback::{AudioTelemetry, PlaybackController};
use resonance_core::proxy::LoopbackProxy;
use tempfile::tempdir;
use tokio::sync::mpsc::UnboundedReceiver;

struct CountingProvider {
    resolve_calls: AtomicUsize,
    resolved_ids: std::sync::Mutex<Vec<String>>,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            resolve_calls: AtomicUsize::new(0),
            resolved_ids: std::sync::Mutex::new(Vec::new()),
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
            title: "Test".into(),
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
            content_length: 128_000,
            duration_ms: 100_000,
            loudness_db: None,
            expires_at_unix_secs: u64::MAX,
            resolved_by_client: "TEST_STUB".into(),
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

struct UnavailableVault;

impl SecretVault for UnavailableVault {
    fn get_secret(&self) -> Result<Option<String>, String> {
        Err("org.freedesktop.secrets unavailable in headless CI".into())
    }

    fn set_secret(&self, _val: &str) -> Result<(), String> {
        Err("org.freedesktop.secrets unavailable in headless CI".into())
    }

    fn delete_secret(&self) -> Result<(), String> {
        Err("org.freedesktop.secrets unavailable in headless CI".into())
    }
}

fn drain_controls(rx: &mut UnboundedReceiver<NshAudioControl>) -> Vec<NshAudioControl> {
    let mut out = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        out.push(cmd);
    }
    out
}

async fn make_test_controller(
    audio: Arc<NshAudioBackend>,
    segments: Vec<SponsorSegment>,
) -> (tempfile::TempDir, Arc<CountingProvider>, PlaybackController) {
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(&dir.path().join("st2.db")).unwrap());
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
        cache,
        proxy,
        Arc::new(NullLyrics),
        Arc::new(FixedSponsorBlock(segments)),
        EventBus::new(64),
    );
    (dir, provider, controller)
}

#[tokio::test]
async fn unified_audio_command_and_telemetry_roundtrip_drives_prefetch_and_sponsorblock() {
    let backend = Arc::new(NshAudioBackend::new());
    let mut control_rx = backend
        .take_control_receiver()
        .expect("control receiver available once");

    let (_dir, provider, controller) = make_test_controller(
        backend.clone(),
        vec![SponsorSegment {
            start_secs: 10.0,
            end_secs: 25.0,
            video_duration_secs: 100.0,
        }],
    )
    .await;

    let mut tracks = fixture_tracks();
    tracks.truncate(2);
    tracks[0].duration_ms = 100_000;
    tracks[1].duration_ms = 100_000;
    let t1_id = tracks[0].id.clone();
    let t2_id = tracks[1].id.clone();

    controller.play_tracks(tracks, 0).await.unwrap();
    assert_eq!(controller.current_generation(), 1);

    let initial = drain_controls(&mut control_rx);
    assert!(
        initial.iter().any(|c| matches!(
            c,
            NshAudioControl::LoadAndPlay { proxy_url, .. } if proxy_url.contains(&t1_id)
        )),
        "expected LoadAndPlay for first track, got {initial:?}"
    );

    // (a) Progress inside SponsorBlock segment [10s, 25s] emits SeekMs(25_000) exactly once.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 12_000,
            duration_ms: Some(100_000),
        })
        .await
        .unwrap();

    let after_sb = drain_controls(&mut control_rx);
    let seek_count = after_sb
        .iter()
        .filter(|c| matches!(c, NshAudioControl::SeekMs(25_000)))
        .count();
    assert_eq!(
        seek_count, 1,
        "expected single SeekMs(25_000), got {after_sb:?}"
    );

    // Stale Progress still inside [10s, 25s] before seek settles must NOT re-trigger SeekMs.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 13_000,
            duration_ms: Some(100_000),
        })
        .await
        .unwrap();
    let after_stale_sb = drain_controls(&mut control_rx);
    assert!(
        !after_stale_sb
            .iter()
            .any(|c| matches!(c, NshAudioControl::SeekMs(_))),
        "SponsorBlock must not re-trigger on stale Progress: {after_stale_sb:?}"
    );

    // (b) Progress crossing 50% triggers LoopbackProxy / provider pre-resolution for track #2.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 52_000,
            duration_ms: Some(100_000),
        })
        .await
        .unwrap();
    assert_eq!(
        controller.preresolved_next_video_id().await.as_deref(),
        Some(t2_id.as_str())
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(provider.resolve_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        provider.resolved_ids.lock().unwrap().as_slice(),
        &[t1_id.as_str(), t2_id.as_str()]
    );

    // (c) Ended advances queue to track #2 and emits LoadAndPlay with incremented generation.
    drain_controls(&mut control_rx);
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
    let after_end = drain_controls(&mut control_rx);
    assert!(
        after_end.iter().any(|c| matches!(
            c,
            NshAudioControl::LoadAndPlay { proxy_url, .. } if proxy_url.contains(&t2_id)
        )),
        "expected LoadAndPlay for track #2, got {after_end:?}"
    );
}

#[tokio::test]
async fn stale_generation_ended_event_does_not_double_skip() {
    let backend = Arc::new(NshAudioBackend::new());
    let (_dir, _provider, controller) = make_test_controller(backend, Vec::new()).await;

    let mut tracks = fixture_tracks();
    tracks.truncate(3);
    let t2_id = tracks[1].id.clone();

    controller.play_tracks(tracks, 0).await.unwrap();
    assert_eq!(controller.current_generation(), 1);

    // Switch to track #2 (generation 2).
    controller.next().await.unwrap();
    assert_eq!(controller.current_generation(), 2);
    assert_eq!(
        controller
            .snapshot()
            .await
            .current_track
            .as_ref()
            .map(|t| t.id.as_str()),
        Some(t2_id.as_str())
    );

    // Inject stale telemetry from generation 1; active track must stay on generation 2.
    controller
        .handle_audio_telemetry(AudioTelemetry::Progress {
            generation: 1,
            position_ms: 95_000,
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
async fn system_media_controls_updates_metadata_and_handles_media_commands() {
    let backend = Arc::new(NshAudioBackend::new());
    let mut cmd_rx = backend
        .take_command_receiver()
        .expect("command receiver available once");

    let (_dir, _provider, controller) = make_test_controller(backend.clone(), Vec::new()).await;

    let mut tracks = fixture_tracks();
    tracks.truncate(2);
    let t1_title = tracks[0].title.clone();
    let t2_id = tracks[1].id.clone();
    let t2_title = tracks[1].title.clone();

    controller.play_tracks(tracks, 0).await.unwrap();
    let bridge = backend.media_controls();
    assert!(bridge.playing());
    assert_eq!(bridge.last_metadata().map(|m| m.title), Some(t1_title));

    controller.pause().await.unwrap();
    assert!(!bridge.playing());

    controller.seek_ms(15_000).await.unwrap();
    assert_eq!(bridge.position_ms(), 15_000);

    // Inject media commands and dispatch them through PlaybackController.
    backend.inject_command(MediaCommand::TogglePlayPause);
    backend.inject_command(MediaCommand::SeekMs(22_000));
    backend.inject_command(MediaCommand::Next);

    for _ in 0..3 {
        let cmd = cmd_rx.recv().await.expect("injected media command");
        controller.handle_media_command(cmd).await.unwrap();
    }

    let snap = controller.snapshot().await;
    assert_eq!(snap.state, PlayerState::Playing);
    assert_eq!(
        snap.current_track.as_ref().map(|t| t.id.as_str()),
        Some(t2_id.as_str())
    );
    assert_eq!(bridge.last_metadata().map(|m| m.title), Some(t2_title));
    assert!(bridge.playing());

    controller.clear_queue().await.unwrap();
    assert!(!bridge.playing());
    assert!(bridge.last_metadata().is_none());
}

#[test]
fn dsp_capability_gate_reports_external_url_stream_honestly() {
    let mut settings = SettingsSlice::new(true);
    let cap = settings.dsp_capability();

    assert_eq!(cap, AudioDspCapability::ExternalUrlStreamOnly);
    assert!(!cap.is_active());
    assert_eq!(cap.badge_label(), "Planned • Native PCM Engine");
    assert!(
        cap.detail_text()
            .contains("nsh/audio streams directly to the OS decoder")
    );

    // Persisted DSP preferences remain intact while gated.
    settings.set_loudness_normalization(true);
    settings.set_silence_skip(true);
    assert_eq!(settings.cycle_eq_preset(), EqPreset::BassBoost.label());
    assert_eq!(settings.cycle_crossfade_ms(), 2_000);
    assert!(settings.dsp_config.loudness_normalization);
    assert!(settings.dsp_config.silence_skip_enabled);
}

#[test]
fn desktop_native_feature_compiles_and_falls_back_cleanly_headless() {
    let bridge = SystemMediaControlsBridge::new(None);
    assert!(!bridge.playing());
    assert!(bridge.last_metadata().is_none());

    let backend = NshAudioBackend::new();
    assert!(backend.active_stream_url().is_none());

    #[cfg(all(
        feature = "desktop-keyring",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    {
        let _vault = OsKeyringVault::new();
    }

    let dir = tempdir().unwrap();
    let store = DesktopAuthStore::new(Arc::new(UnavailableVault), FileAuthStore::new(dir.path()));
    let session = AuthSession {
        cookie_header: "SAPISID=headless_sapisid; __Secure-3PAPISID=headless_3pa".into(),
        auth_user: 0,
        page_id: None,
        visitor_data: None,
        account_name: Some("Headless User".into()),
        is_premium: false,
    };

    store.save_session(&session).unwrap();
    assert_eq!(
        store.backend_kind(),
        CredentialBackendKind::EncryptedFallbackFile
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("keystore_session.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    assert_eq!(store.load_session().unwrap(), Some(session));
    store.clear_session().unwrap();
    assert_eq!(store.load_session().unwrap(), None);
}
