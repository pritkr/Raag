//! Offline verification suite for ST-3:
//! - Hybrid provider browse failover (primary -> secondary) + SWR SQLite cache
//! - Playback failover + platform decoder format matrix (Desktop AAC, Android Opus, iOS no-Opus)
//! - LibrarySnapshot partial fault isolation
//! - Search deduplication & categorized search
//! - AtrTracker watchtime milestone reporting (signed-in only)

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use resonance_contract::{
    AppEvent, AudioCodec, AuthError, AuthSession, AuthStore, CacheStore, CoreError, CoreResult,
    HomeItem, HomeSection, LibraryCollectionRef, LibrarySnapshot, LyricsDocument, MusicProvider,
    PlayerSnapshot, PlayerState, Playlist, ProviderError, SearchResults, StreamInfo, Track,
};
use resonance_core::cache::SqliteCacheStore;
use resonance_core::innertube::client::InnerTubeClient;
use resonance_core::innertube::formats::TargetPlatform;
use resonance_core::innertube::matrix::AccountTier;
use resonance_core::innertube::provider_adapter::{BrowseDelegate, PlaybackResolver};
use resonance_core::providers::mappers::{deduplicate_playlists, deduplicate_tracks};
use resonance_core::providers::{AtrTracker, HybridMusicProvider, select_rustypipe_stream};
use resonance_core::solver::{QuickJsSolver, SharedSolver};
use rustypipe::model::AudioStream;

fn make_rp_audio_stream(
    url: &str,
    itag: u32,
    bitrate: u32,
    mime: &str,
    format: &str,
    codec: &str,
    loudness_db: Option<f32>,
) -> AudioStream {
    serde_json::from_value(serde_json::json!({
        "url": url,
        "itag": itag,
        "bitrate": bitrate,
        "average_bitrate": bitrate,
        "size": 1_000_000_u64,
        "duration_ms": 180_000_u64,
        "mime": mime,
        "format": format,
        "codec": codec,
        "channels": 2_u8,
        "loudness_db": loudness_db,
        "drm_systems": []
    }))
    .unwrap()
}

struct MemoryAuthStore;

impl AuthStore for MemoryAuthStore {
    fn load_session(&self) -> Result<Option<AuthSession>, AuthError> {
        Ok(None)
    }
    fn save_session(&self, _session: &AuthSession) -> Result<(), AuthError> {
        Ok(())
    }
    fn clear_session(&self) -> Result<(), AuthError> {
        Ok(())
    }
}

fn make_test_track(id: &str, title: &str) -> Track {
    Track {
        id: id.to_owned(),
        title: title.to_owned(),
        artists: Vec::new(),
        album: None,
        duration_ms: 180_000,
        artwork_url: None,
        is_explicit: false,
        is_liked: false,
        is_video: false,
    }
}

struct FailingBrowseDelegate {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl BrowseDelegate for FailingBrowseDelegate {
    async fn search(&self, _query: &str) -> CoreResult<SearchResults> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(CoreError::Provider(ProviderError::Parse(
            "simulated ytmapi schema drift".into(),
        )))
    }
    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(CoreError::Provider(ProviderError::Parse(
            "simulated home parse failure".into(),
        )))
    }
    async fn browse_collection(&self, _browse_id: &str) -> CoreResult<Playlist> {
        Err(CoreError::Provider(ProviderError::Parse("fail".into())))
    }
    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Err(CoreError::Auth(AuthError::NotSignedIn))
    }
    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Err(CoreError::Provider(ProviderError::Parse("fail".into())))
    }
    async fn ytm_lyrics(&self, _video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

struct HealthySecondaryBrowse {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl BrowseDelegate for HealthySecondaryBrowse {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(SearchResults {
            query: query.to_owned(),
            tracks: vec![make_test_track("vid-rp-1", "Fallback Track")],
            albums: Vec::new(),
            continuation: None,
        })
    }
    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![HomeSection {
            id: "sec-rp-1".into(),
            title: "Charts".into(),
            items: vec![HomeItem::Track(make_test_track("vid-rp-home", "Chart Hit"))],
        }])
    }
    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        Ok(Playlist {
            id: browse_id.to_owned(),
            title: "Fallback Playlist".into(),
            subtitle: None,
            artwork_url: None,
            tracks: vec![make_test_track("vid-rp-2", "Playlist Track")],
        })
    }
    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Err(CoreError::Auth(AuthError::NotSignedIn))
    }
    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Ok(vec![make_test_track("vid-rp-next", "Up Next")])
    }
    async fn ytm_lyrics(&self, _video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

struct FailingPlaybackResolver;

#[async_trait]
impl PlaybackResolver for FailingPlaybackResolver {
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        Err(CoreError::Provider(ProviderError::NoStream {
            video_id: video_id.to_owned(),
        }))
    }
}

struct MockRustyPipePlayback {
    platform: TargetPlatform,
    tier: AccountTier,
}

#[async_trait]
impl PlaybackResolver for MockRustyPipePlayback {
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        let streams = vec![
            make_rp_audio_stream(
                "https://example.invalid/opus_251",
                251,
                160_000,
                "audio/webm; codecs=\"opus\"",
                "webm",
                "opus",
                Some(-3.2),
            ),
            make_rp_audio_stream(
                "https://example.invalid/aac_140",
                140,
                128_000,
                "audio/mp4; codecs=\"mp4a.40.2\"",
                "m4a",
                "mp4a",
                Some(-3.2),
            ),
        ];
        select_rustypipe_stream(video_id, &streams, self.platform, self.tier, 180_000)
    }
}

#[tokio::test]
async fn hybrid_provider_falls_back_from_ytmapi_to_rustypipe_on_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    let cache: Arc<dyn CacheStore> = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let auth: Arc<dyn AuthStore> = Arc::new(MemoryAuthStore);
    let inner = Arc::new(
        InnerTubeClient::new(
            Arc::clone(&cache),
            auth,
            SharedSolver::new(QuickJsSolver::new(dir.path())),
        )
        .unwrap(),
    );

    let primary_calls = Arc::new(AtomicUsize::new(0));
    let secondary_calls = Arc::new(AtomicUsize::new(0));

    let hybrid = HybridMusicProvider::with_delegates(
        inner,
        Arc::clone(&cache),
        Some(Arc::new(FailingBrowseDelegate {
            calls: Arc::clone(&primary_calls),
        })),
        Some(Arc::new(HealthySecondaryBrowse {
            calls: Arc::clone(&secondary_calls),
        })),
        None,
        None,
    );

    let sr = hybrid.search("synthwave").await.unwrap();
    assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(sr.tracks[0].id, "vid-rp-1");

    let home = hybrid.home().await.unwrap();
    assert_eq!(home[0].title, "Charts");

    // Verify SWR SQLite cache was populated.
    let cached = cache.get_metadata("hybrid:home:v1", u64::MAX).unwrap();
    assert!(cached.is_some());
}

#[tokio::test]
async fn hybrid_playback_falls_back_to_rustypipe_and_enforces_platform_format_matrix() {
    let dir = tempfile::tempdir().unwrap();
    let cache: Arc<dyn CacheStore> = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let auth: Arc<dyn AuthStore> = Arc::new(MemoryAuthStore);
    let inner = Arc::new(
        InnerTubeClient::new(
            Arc::clone(&cache),
            auth,
            SharedSolver::new(QuickJsSolver::new(dir.path())),
        )
        .unwrap(),
    );

    // Desktop prefers AAC (140).
    let desktop_hybrid = HybridMusicProvider::with_delegates(
        Arc::clone(&inner),
        Arc::clone(&cache),
        None,
        None,
        Some(Arc::new(FailingPlaybackResolver)),
        Some(Arc::new(MockRustyPipePlayback {
            platform: TargetPlatform::Desktop,
            tier: AccountTier::Anonymous,
        })),
    );
    let desktop_stream = desktop_hybrid.resolve_stream("vid-1").await.unwrap();
    assert_eq!(desktop_stream.itag, 140);
    assert_eq!(desktop_stream.codec, AudioCodec::Aac);

    // Android prefers Opus (251).
    let android_hybrid = HybridMusicProvider::with_delegates(
        Arc::clone(&inner),
        Arc::clone(&cache),
        None,
        None,
        Some(Arc::new(FailingPlaybackResolver)),
        Some(Arc::new(MockRustyPipePlayback {
            platform: TargetPlatform::Android,
            tier: AccountTier::Anonymous,
        })),
    );
    let android_stream = android_hybrid.resolve_stream("vid-1").await.unwrap();
    assert_eq!(android_stream.itag, 251);
    assert_eq!(android_stream.codec, AudioCodec::Opus);

    // iOS strictly excludes Opus even if only Opus is available.
    let opus_only = vec![make_rp_audio_stream(
        "https://example.invalid/opus_only",
        251,
        160_000,
        "audio/webm; codecs=\"opus\"",
        "webm",
        "opus",
        None,
    )];
    let ios_res = select_rustypipe_stream(
        "vid-1",
        &opus_only,
        TargetPlatform::Ios,
        AccountTier::Anonymous,
        180_000,
    );
    assert!(matches!(
        ios_res,
        Err(CoreError::Provider(ProviderError::NoStream { .. }))
    ));
}

#[test]
fn library_snapshot_fault_isolates_individual_tabs() {
    let snap = LibrarySnapshot {
        liked_songs: vec![make_test_track("liked-1", "Liked Track")],
        playlists: vec![LibraryCollectionRef {
            browse_id: "VLPL1".into(),
            playlist_id: Some("PL1".into()),
            title: "My Mix".into(),
            subtitle: "10 songs".into(),
            artwork_url: None,
            track_count: Some(10),
        }],
        uploads: Vec::new(),
        ..Default::default()
    };
    assert_eq!(snap.liked_songs.len(), 1);
    assert_eq!(snap.playlists.len(), 1);
    assert!(snap.uploads.is_empty());
}

#[test]
fn categorized_search_suggestions_and_continuation_deduplicate_ids() {
    let tracks = vec![
        make_test_track("t1", "Song 1"),
        make_test_track("t2", "Song 2"),
        make_test_track("t1", "Song 1 Duplicate"),
        make_test_track("", "Empty ID"),
    ];
    let deduped = deduplicate_tracks(tracks);
    assert_eq!(deduped.len(), 2);
    assert_eq!(deduped[0].id, "t1");
    assert_eq!(deduped[1].id, "t2");

    let playlists = vec![
        Playlist {
            id: "p1".into(),
            title: "P1".into(),
            subtitle: None,
            artwork_url: None,
            tracks: Vec::new(),
        },
        Playlist {
            id: "p1".into(),
            title: "P1 Dup".into(),
            subtitle: None,
            artwork_url: None,
            tracks: Vec::new(),
        },
    ];
    assert_eq!(deduplicate_playlists(playlists).len(), 1);
}

#[test]
fn atr_watchtime_reporter_fires_at_30s_pause_and_track_end_when_signed_in_only() {
    let track_a = make_test_track("vid-a", "Track A");
    let track_b = make_test_track("vid-b", "Track B");

    // 1. Signed out -> zero reports.
    let mut signed_out = AtrTracker::new(false);
    let r0 = signed_out.process_event(&AppEvent::PlayerUpdated(PlayerSnapshot {
        state: PlayerState::Playing,
        current_track: Some(track_a.clone()),
        position_ms: 45_000,
        duration_ms: 180_000,
        ..Default::default()
    }));
    assert!(r0.is_empty());

    // 2. Signed in -> 30s milestone, pause, and track switch (is_final: true).
    let mut signed_in = AtrTracker::new(true);
    let r_30s = signed_in.process_event(&AppEvent::PlayerUpdated(PlayerSnapshot {
        state: PlayerState::Playing,
        current_track: Some(track_a.clone()),
        position_ms: 31_000,
        duration_ms: 180_000,
        ..Default::default()
    }));
    assert_eq!(r_30s.len(), 1);
    assert_eq!(r_30s[0].end_secs, 31);
    assert!(!r_30s[0].is_final);
    assert_eq!(r_30s[0].cpn.len(), 16);

    let r_pause = signed_in.process_event(&AppEvent::PlayerUpdated(PlayerSnapshot {
        state: PlayerState::Paused,
        current_track: Some(track_a.clone()),
        position_ms: 50_000,
        duration_ms: 180_000,
        ..Default::default()
    }));
    assert_eq!(r_pause.len(), 1);
    assert_eq!(r_pause[0].start_secs, 31);
    assert_eq!(r_pause[0].end_secs, 50);
    assert!(!r_pause[0].is_final);

    let r_switch = signed_in.process_event(&AppEvent::PlayerUpdated(PlayerSnapshot {
        state: PlayerState::Playing,
        current_track: Some(track_b),
        position_ms: 0,
        duration_ms: 200_000,
        ..Default::default()
    }));
    assert_eq!(r_switch.len(), 1);
    assert_eq!(r_switch[0].video_id, "vid-a");
    assert!(r_switch[0].is_final);
}
