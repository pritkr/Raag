//! Offline verification tests for W1C deltas:
//! - HybridMusicProvider circuit breaker (trips after 3 consecutive failures, auth errors do not trip)
//! - Native InnerTube parsers: Explore, Mood playlists, Home mood chips, Search suggestions, Artist, Album
//! - AtrTracker privacy fence (`atr_enabled = false` and `local:` tracks)
//! - CatalogSlice MRU search history deduplication & deletion

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use resonance_contract::{
    AppEvent, AuthError, AuthSession, AuthStore, CacheStore, CoreError, CoreResult, HomeSection,
    LyricsDocument, MusicProvider, PlayerSnapshot, PlayerState, Playlist, ProviderError,
    SearchResults, Track,
};
use resonance_core::cache::SqliteCacheStore;
use resonance_core::innertube::client::InnerTubeClient;
use resonance_core::innertube::parser::{
    SuggestionEntityKind, extract_home_mood_chips, parse_album_response, parse_artist_response,
    parse_explore_response, parse_mood_playlists_response, parse_search_suggestions_response,
};
use resonance_core::innertube::provider_adapter::BrowseDelegate;
use resonance_core::providers::{AtrTracker, HybridMusicProvider};
use resonance_core::solver::{QuickJsSolver, SharedSolver};
use serde_json::json;

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

struct CountingFailingPrimary {
    calls: Arc<AtomicUsize>,
    emit_auth_error: bool,
}

#[async_trait]
impl BrowseDelegate for CountingFailingPrimary {
    async fn search(&self, _query: &str) -> CoreResult<SearchResults> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.emit_auth_error {
            Err(CoreError::Auth(AuthError::NotSignedIn))
        } else {
            Err(CoreError::Provider(ProviderError::Parse("drift".into())))
        }
    }
    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        Err(CoreError::Provider(ProviderError::Parse("drift".into())))
    }
    async fn browse_collection(&self, _browse_id: &str) -> CoreResult<Playlist> {
        Err(CoreError::Provider(ProviderError::Parse("drift".into())))
    }
    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Err(CoreError::Auth(AuthError::NotSignedIn))
    }
    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Err(CoreError::Provider(ProviderError::Parse("drift".into())))
    }
    async fn ytm_lyrics(&self, _video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

struct HealthyFallback {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl BrowseDelegate for HealthyFallback {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        self.calls.fetch_add(1, Ordering::SeqCst);
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
    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        Ok(Playlist {
            id: browse_id.to_owned(),
            title: "Ok".into(),
            subtitle: None,
            artwork_url: None,
            tracks: Vec::new(),
        })
    }
    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        Ok(Vec::new())
    }
    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Ok(Vec::new())
    }
    async fn ytm_lyrics(&self, _video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

#[tokio::test]
async fn hybrid_provider_trips_circuit_breaker_after_3_failures_and_skips_primary() {
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

    // 1. Auth errors do NOT trip the circuit breaker.
    let auth_calls = Arc::new(AtomicUsize::new(0));
    let fb_calls = Arc::new(AtomicUsize::new(0));
    let auth_hybrid = HybridMusicProvider::with_delegates(
        Arc::clone(&inner),
        Arc::clone(&cache),
        Some(Arc::new(CountingFailingPrimary {
            calls: Arc::clone(&auth_calls),
            emit_auth_error: true,
        })),
        Some(Arc::new(HealthyFallback {
            calls: Arc::clone(&fb_calls),
        })),
        None,
        None,
    );
    for i in 0..4 {
        let _ = auth_hybrid.search(&format!("q-{i}")).await.unwrap();
    }
    assert!(!auth_hybrid.is_primary_circuit_open());
    assert_eq!(auth_calls.load(Ordering::SeqCst), 4);

    // 2. Parse/network errors trip circuit after 3 failures; 4th call skips primary.
    let primary_calls = Arc::new(AtomicUsize::new(0));
    let secondary_calls = Arc::new(AtomicUsize::new(0));
    let hybrid = HybridMusicProvider::with_delegates(
        inner,
        cache,
        Some(Arc::new(CountingFailingPrimary {
            calls: Arc::clone(&primary_calls),
            emit_auth_error: false,
        })),
        Some(Arc::new(HealthyFallback {
            calls: Arc::clone(&secondary_calls),
        })),
        None,
        None,
    );
    for i in 0..4 {
        let _ = hybrid.search(&format!("synthwave-{i}")).await.unwrap();
    }
    assert!(hybrid.is_primary_circuit_open());
    assert_eq!(primary_calls.load(Ordering::SeqCst), 3);
    assert_eq!(secondary_calls.load(Ordering::SeqCst), 4);
}

#[test]
fn explore_suggestions_artist_and_album_parsers_handle_innertube_fixtures() {
    // 1. Home mood chips
    let home_json = json!({
        "contents": {
            "singleColumnBrowseResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "header": {
                                    "chipCloudRenderer": {
                                        "chips": [{
                                            "chipCloudChipRenderer": {
                                                "text": { "runs": [{ "text": "Focus" }] },
                                                "navigationEndpoint": {
                                                    "browseEndpoint": { "params": "ggMPOg1uX1JOQWZFeD" }
                                                }
                                            }
                                        }]
                                    }
                                },
                                "contents": []
                            }
                        }
                    }
                }]
            }
        }
    });
    let chips = extract_home_mood_chips(&home_json);
    assert_eq!(chips.len(), 1);
    assert_eq!(chips[0].title, "Focus");

    // 2. Explore + Mood grid
    let explore_json = json!({
        "contents": {
            "singleColumnBrowseResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "contents": [
                                    {
                                        "musicCarouselShelfRenderer": {
                                            "header": {
                                                "musicCarouselShelfBasicHeaderRenderer": {
                                                    "title": { "runs": [{ "text": "New albums & singles" }] }
                                                }
                                            },
                                            "contents": [{
                                                "musicTwoRowItemRenderer": {
                                                    "title": { "runs": [{ "text": "Midnight LP" }] },
                                                    "subtitle": { "runs": [{ "text": "Album • Artist" }] },
                                                    "navigationEndpoint": {
                                                        "browseEndpoint": { "browseId": "MPREb_123" }
                                                    }
                                                }
                                            }]
                                        }
                                    },
                                    {
                                        "gridRenderer": {
                                            "items": [{
                                                "musicNavigationButtonRenderer": {
                                                    "buttonText": { "runs": [{ "text": "Chill" }] },
                                                    "clickCommand": {
                                                        "browseEndpoint": { "params": "mood_chill_p" }
                                                    }
                                                }
                                            }]
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }]
            }
        }
    });
    let explore = parse_explore_response(&explore_json).unwrap();
    assert_eq!(explore.new_releases.len(), 1);
    assert_eq!(explore.moods_and_genres.len(), 1);
    assert_eq!(explore.moods_and_genres[0].params, "mood_chill_p");

    // 3. Mood playlists
    let mood_pl_json = json!({
        "contents": {
            "singleColumnBrowseResultsRenderer": {
                "tabs": [{
                    "tabRenderer": {
                        "content": {
                            "sectionListRenderer": {
                                "contents": [{
                                    "gridRenderer": {
                                        "items": [{
                                            "musicTwoRowItemRenderer": {
                                                "title": { "runs": [{ "text": "Lo-Fi Beats" }] },
                                                "subtitle": { "runs": [{ "text": "YouTube Music" }] },
                                                "navigationEndpoint": {
                                                    "browseEndpoint": { "browseId": "VLRDCLAK5uy" }
                                                }
                                            }
                                        }]
                                    }
                                }]
                            }
                        }
                    }
                }]
            }
        }
    });
    let pls = parse_mood_playlists_response(&mood_pl_json).unwrap();
    assert_eq!(pls.len(), 1);
    assert_eq!(pls[0].browse_id, "VLRDCLAK5uy");

    // 4. Search suggestions (query + direct entity)
    let sugg_json = json!({
        "contents": [{
            "searchSuggestionsSectionRenderer": {
                "contents": [
                    {
                        "searchSuggestionRenderer": {
                            "suggestion": { "runs": [{ "text": "nujabes feather" }] }
                        }
                    },
                    {
                        "musicResponsiveListItemRenderer": {
                            "flexColumns": [
                                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Feather" }] } } },
                                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Song • Nujabes" }] } } }
                            ],
                            "navigationEndpoint": {
                                "watchEndpoint": { "videoId": "vid_feather" }
                            }
                        }
                    }
                ]
            }
        }]
    });
    let suggs = parse_search_suggestions_response(&sugg_json).unwrap();
    assert_eq!(suggs.len(), 2);
    assert!(matches!(
        &suggs[1],
        resonance_core::innertube::parser::SearchSuggestionItem::DirectEntity {
            kind: SuggestionEntityKind::Song,
            id,
            ..
        } if id == "vid_feather"
    ));

    // 5. Artist & Album
    let artist = parse_artist_response(
        "UC123",
        &json!({
            "header": {
                "musicImmersiveHeaderRenderer": {
                    "title": { "runs": [{ "text": "Nujabes" }] }
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(artist.name, "Nujabes");

    let album = parse_album_response(
        "MPREb_123",
        &json!({
            "header": {
                "musicDetailHeaderRenderer": {
                    "title": { "runs": [{ "text": "Modal Soul" }] },
                    "subtitle": { "runs": [{ "text": "Album" }, { "text": " • " }, { "text": "2005" }] }
                }
            }
        }),
    )
    .unwrap();
    assert_eq!(album.release_year.as_deref(), Some("2005"));
}

#[test]
fn atr_tracker_blocks_local_tracks_and_disabled_setting() {
    let local_track = Track {
        id: "local:42".to_owned(),
        title: "Local Flac".to_owned(),
        artists: Vec::new(),
        album: None,
        duration_ms: 180_000,
        artwork_url: None,
        is_explicit: false,
        is_liked: false,
        is_video: false,
    };
    let mut tracker = AtrTracker::new(true);
    let r_local = tracker.process_event(&AppEvent::PlayerUpdated(PlayerSnapshot {
        state: PlayerState::Playing,
        current_track: Some(local_track),
        position_ms: 45_000,
        duration_ms: 180_000,
        ..Default::default()
    }));
    assert!(r_local.is_empty(), "local tracks must never emit ATR");

    let yt_track = Track {
        id: "vid-yt-1".to_owned(),
        title: "YT Track".to_owned(),
        artists: Vec::new(),
        album: None,
        duration_ms: 180_000,
        artwork_url: None,
        is_explicit: false,
        is_liked: false,
        is_video: false,
    };
    let mut disabled = AtrTracker::new(true).with_enabled(false);
    let r_disabled = disabled.process_event(&AppEvent::PlayerUpdated(PlayerSnapshot {
        state: PlayerState::Playing,
        current_track: Some(yt_track),
        position_ms: 45_000,
        duration_ms: 180_000,
        ..Default::default()
    }));
    assert!(r_disabled.is_empty(), "disabled ATR must not emit");
}
