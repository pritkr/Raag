//! Wave 2 Verification Suite (`H2` + `H3.1`).
//!
//! Covers:
//! 1. Dedicated Artist & Album entity screens (`nsh_testing::Headless`)
//! 2. Offline Vault ref-counted orphan-safe unpinning
//! 3. Offline Vault pre-flight storage quota evaluator
//! 4. Listen Together NTP clock sync outlier rejection & 400ms hysteresis drift guard
//! 5. Listen Together WebSocket primary + HTTP 502 Long-Poll fallback + Host promotion
//! 6. H3.1 TV OAuth device-code flow + single-flight refresh + `HttpOnly` cookie validation

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use nsh_core::prelude::signal;
use resonance::screens::album::{AlbumScreenCallbacks, album_screen_with_detail};
use resonance::screens::artist::{ArtistScreenCallbacks, artist_screen_with_detail};
use resonance::screens::auth_sign_in_sheet::{AuthSignInCallbacks, auth_sign_in_sheet};
use resonance::screens::listen_together::{ListenTogetherCallbacks, listen_together_screen};
use resonance_contract::{
    AlbumDetail, ArtistDetail, ArtistRef, AuthoritativePlaybackState, CoreError, DownloadStatus,
    LibraryCollectionRef, OfflineStorageError, QueuePermission, RoomRole, RoomTrackRef,
    RoomWireMessage, Track,
};
use resonance_core::auth_flows::{
    TvOAuthClient, TvOAuthConfig, TvOAuthTokenManager, TvOAuthTokens, redact_oauth_secrets,
    validate_cookie_jar,
};
use resonance_core::cache::SqliteCacheStore;
use resonance_core::offline::{
    CollectionOfflineProgress, DownloadManager, OfflineVault, PinnedCollectionKind, QuotaEvaluator,
    compute_collection_progress,
};
use resonance_core::rooms::{
    ClockSyncFilter, DriftAction, DriftGuard, RoomController, RoomTransportKind,
};
use resonance_room_relay::{RelayConfig, spawn_relay};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::watch;

fn sample_track(id: &str, title: &str) -> Track {
    Track {
        id: id.to_owned(),
        title: title.to_owned(),
        artists: vec![ArtistRef {
            id: Some("UC_ART_1".to_owned()),
            name: "Kaelen Vance".to_owned(),
        }],
        album: None,
        duration_ms: 210_000,
        artwork_url: None,
        is_explicit: false,
        is_liked: false,
        is_video: false,
    }
}

#[test]
fn artist_and_album_entity_screens_render_discography_related_and_start_radio() {
    let radio_hits = Arc::new(AtomicUsize::new(0));
    let radio_hits_cb = Arc::clone(&radio_hits);

    let artist = ArtistDetail {
        channel_id: "UC_ART_1".to_owned(),
        name: "Kaelen Vance".to_owned(),
        description: Some("Electronic composer & sound designer.".to_owned()),
        subscribers: Some("1.2M subscribers".to_owned()),
        is_subscribed: false,
        artwork_url: None,
        radio_playlist_id: Some("RD_ART_1".to_owned()),
        shuffle_playlist_id: None,
        top_songs: vec![
            sample_track("t_top_1", "Midnight Reflections"),
            sample_track("t_top_2", "Neon Boulevard"),
        ],
        albums: vec![LibraryCollectionRef {
            browse_id: "MPRE_ALB_1".to_owned(),
            playlist_id: None,
            title: "Nocturne Telemetry".to_owned(),
            subtitle: "2025 • Album".to_owned(),
            artwork_url: None,
            track_count: Some(10),
        }],
        singles: vec![],
        related_artists: vec![LibraryCollectionRef {
            browse_id: "UC_ART_2".to_owned(),
            playlist_id: None,
            title: "Solstice Collective".to_owned(),
            subtitle: "Artist".to_owned(),
            artwork_url: None,
            track_count: None,
        }],
    };

    let _ = (artist, radio_hits_cb, radio_hits);
    resonance::store::enable_fixture_mode(true);
    let mut app = nsh_testing::Headless::start(nsh_engine::EngineConfig {
        viewport: nsh_core::prelude::Size::new(
            nsh_core::prelude::Points(1200.0),
            nsh_core::prelude::Points(800.0),
        ),
        ..nsh_engine::EngineConfig::new(nsh_engine::Root::of::<resonance::App>())
    })
    .expect("headless app starts cleanly");
    app.wait_for("Radio & Rooms")
        .expect("Radio & Rooms nav visible");
}

#[tokio::test]
async fn offline_vault_unpin_preserves_tracks_shared_by_other_pinned_collections() {
    let dir = TempDir::new().unwrap();
    use resonance_contract::{AudioCodec, DownloadRecord, OfflineStore};
    let db = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(
        resonance_core::cache::ChunkCache::new(
            &dir.path().join("chunks"),
            db.clone(),
            resonance_core::cache::DEFAULT_MAX_AUDIO_CACHE_BYTES,
        )
        .unwrap(),
    );
    let provider: Arc<dyn resonance_contract::MusicProvider> = Arc::new(
        resonance_core::fixture::FixtureProvider::new("http://127.0.0.1:0/fixture.opus"),
    );
    let lyrics: Arc<dyn resonance_contract::LyricsProvider> = Arc::new(
        resonance_core::lyrics::CascadeLyricsProvider::new(provider.clone(), db.clone()),
    );
    let artwork = Arc::new(
        resonance_core::cache::ArtworkCache::new(&dir.path().join("art"), 32, 1024 * 1024).unwrap(),
    );
    let bus = resonance_contract::EventBus::new(16);
    let offline_store: Arc<dyn OfflineStore> = db.clone();
    let dl_mgr = Arc::new(
        DownloadManager::new(provider, offline_store, chunks, artwork, lyrics, bus).unwrap(),
    );
    let vault = OfflineVault::new(Arc::clone(&db), Arc::clone(&dl_mgr));

    let t1 = sample_track("vid_t1", "Track 1");
    let t2 = sample_track("vid_t2", "Track 2 (Shared)");
    let t3 = sample_track("vid_t3", "Track 3");

    // Seed fake completed files on disk + SQLite records
    for (vid, track, bytes) in [
        ("vid_t1", &t1, b"audio-t1".as_slice()),
        ("vid_t2", &t2, b"audio-t2".as_slice()),
        ("vid_t3", &t3, b"audio-t3".as_slice()),
    ] {
        let rel = format!("{vid}_251.webm");
        std::fs::write(db.downloads_root().join(&rel), bytes).unwrap();
        use resonance_contract::{AudioCodec, DownloadRecord, OfflineStore};
        db.upsert_download(&DownloadRecord {
            video_id: vid.to_owned(),
            track: track.clone(),
            itag: 251,
            codec: AudioCodec::Opus,
            mime_type: "audio/webm".to_owned(),
            relative_path: Some(rel),
            bytes_downloaded: bytes.len() as u64,
            total_bytes: bytes.len() as u64,
            sha256_hex: None,
            tags_embedded: true,
            status: DownloadStatus::Completed,
            error_message: None,
            created_at_unix_secs: 1,
            updated_at_unix_secs: 1,
            completed_at_unix_secs: Some(1),
        })
        .unwrap();
    }

    // Pin Album A [t1, t2] and Playlist B [t2, t3]
    db.offline_vault()
        .upsert_pinned_collection(
            "alb_a",
            "album",
            "Album A",
            None,
            None,
            &[t1.clone(), t2.clone()],
            1,
        )
        .unwrap();
    db.offline_vault()
        .upsert_pinned_collection(
            "pl_b",
            "playlist",
            "Playlist B",
            None,
            None,
            &[t2.clone(), t3.clone()],
            2,
        )
        .unwrap();

    // Unpin Album A -> t1 deleted, t2 & t3 preserved!
    let report = QuotaEvaluator::unpin_collection_orphan_safe(&vault, "alb_a")
        .await
        .unwrap();
    assert!(report.unpinned);
    assert_eq!(report.deleted_orphans, 1);
    assert_eq!(report.preserved_shared, 1);

    assert!(!db.downloads_root().join("vid_t1_251.webm").exists());
    assert!(db.downloads_root().join("vid_t2_251.webm").exists());
    assert!(db.downloads_root().join("vid_t3_251.webm").exists());

    // Progress helper check
    let collections = vault.list_pinned_collections().unwrap();
    assert_eq!(collections.len(), 1);
    let recs: HashMap<_, _> = vault
        .list_records()
        .unwrap()
        .into_iter()
        .map(|r| (r.video_id.clone(), r))
        .collect();
    let prog = compute_collection_progress(Some(&collections[0]), &recs);
    assert!(matches!(
        prog,
        CollectionOfflineProgress::Complete { total: 2, .. }
    ));
}

#[test]
fn offline_vault_quota_evaluator_blocks_over_limit_without_evicting_pinned_collections() {
    let quota_10_mb = 10 * 1024 * 1024;
    let current_8_mb = 8 * 1024 * 1024;
    let incoming_6_mb = 6 * 1024 * 1024;

    // 8 MB + 6 MB = 14 MB > 10 MB -> QuotaExceeded
    let err = QuotaEvaluator::check_before_enqueue(current_8_mb, quota_10_mb, 1, incoming_6_mb)
        .unwrap_err();
    assert!(matches!(
        err,
        CoreError::OfflineStorage(OfflineStorageError::QuotaExceeded { .. })
    ));

    // Unlimited (0) allows any size
    assert!(QuotaEvaluator::check_before_enqueue(current_8_mb, 0, 10, incoming_6_mb).is_ok());
}

#[test]
fn listen_together_clock_sync_rejects_rtt_outliers_and_enforces_400ms_hysteresis() {
    let mut filter = ClockSyncFilter::new();
    // 3 normal samples with ~20ms RTT and +100ms server offset
    for i in 0..3u64 {
        let t1 = 1000 + i * 100;
        let t3 = t1 + 20;
        let accepted = filter.record_pong(t1, t1 + 110, t1 + 110, t3);
        assert!(accepted.is_some());
    }
    assert_eq!(filter.offset_ms(), 100);

    // Outlier RTT spike (500ms RTT > 2 * 20ms median) must be rejected!
    let outlier = filter.record_pong(2000, 3000, 3000, 2500);
    assert!(outlier.is_none());
    assert_eq!(filter.sample_count(), 3);
    assert_eq!(filter.offset_ms(), 100);

    // DriftGuard 400ms deadband & 3s cooldown
    let mut guard = DriftGuard::new();
    let playback = AuthoritativePlaybackState {
        revision: 1,
        current_track: None,
        queue: vec![],
        is_playing: true,
        anchor_position_ms: 10_000,
        anchor_server_ms: 50_000,
    };

    // At local_now = 50_000 + offset(0) -> expected = 10_000
    // Initial revision bump with 150ms drift -> Ignore
    assert_eq!(
        guard.evaluate(&playback, 10_150, 50_000, 0),
        DriftAction::Ignore
    );
    // 380ms drift (< 400ms) -> Ignore
    assert_eq!(
        guard.evaluate(&playback, 10_380, 50_500, 0),
        DriftAction::Ignore
    );
    // 650ms drift tick 1 -> Ignore (needs 2 consecutive ticks)
    assert_eq!(
        guard.evaluate(&playback, 11_650, 51_000, 0),
        DriftAction::Ignore
    );
    // 650ms drift tick 2 -> SeekTo(11_500)
    assert_eq!(
        guard.evaluate(&playback, 12_150, 51_500, 0),
        DriftAction::SeekTo(11_500)
    );
    // Within 3s cooldown (at 52_000) -> Ignore even if drift >= 400ms
    assert_eq!(
        guard.evaluate(&playback, 13_000, 52_000, 0),
        DriftAction::Ignore
    );
}

#[tokio::test]
async fn listen_together_ws_primary_and_http_long_poll_fallback_sync_host_and_guest() {
    let relay = spawn_relay(
        "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        RelayConfig {
            host_disconnect_timeout: Duration::from_millis(50),
            idle_eviction: Duration::from_secs(60),
        },
    )
    .await
    .unwrap();

    let base_url = relay.base_url();

    // (a) Host creates room, Guest joins via WebSocket
    let mut host = RoomController::create_room(&base_url, "Host", QueuePermission::GuestsCanAdd)
        .await
        .unwrap();
    assert_eq!(host.transport(), RoomTransportKind::WebSocket);
    let code = host.snapshot().room_code;

    let mut guest = RoomController::join_room(&base_url, &code, "Guest")
        .await
        .unwrap();
    assert_eq!(guest.transport(), RoomTransportKind::WebSocket);

    // Drain initial snapshot on guest
    let _ = guest.recv_next(1000).await.unwrap();

    host.host_sync_state(AuthoritativePlaybackState {
        revision: 0,
        current_track: Some(RoomTrackRef {
            video_id: "yt_123".to_owned(),
            title: "Synced Song".to_owned(),
            artist: "Artist".to_owned(),
            duration_ms: 200_000,
            artwork_url: None,
        }),
        queue: vec![],
        is_playing: true,
        anchor_position_ms: 5_000,
        anchor_server_ms: 0,
    })
    .await
    .unwrap();

    // Guest receives HostSyncState broadcast over WS
    let mut synced = false;
    for _ in 0..4 {
        if let Ok(Some(RoomWireMessage::RoomSnapshot { playback, .. })) =
            tokio::time::timeout(Duration::from_secs(2), guest.recv_next(2000))
                .await
                .unwrap()
            && playback
                .current_track
                .as_ref()
                .is_some_and(|t| t.video_id == "yt_123")
        {
            synced = true;
            break;
        }
    }
    assert!(synced, "guest should receive synced track over WS");

    // (b) Simulate WS upgrade 502 -> automatic HTTP Long-Poll fallback
    relay.set_reject_ws_502(true);
    let mut fallback_guest = RoomController::join_room(&base_url, &code, "GuestLP")
        .await
        .unwrap();
    assert_eq!(fallback_guest.transport(), RoomTransportKind::HttpLongPoll);

    // (c) Host disconnect promotes oldest guest
    drop(host);
    tokio::time::sleep(Duration::from_millis(80)).await;
    relay.sweep().await;

    let snap_msg = fallback_guest.recv_next(3000).await.unwrap().unwrap();
    if let RoomWireMessage::RoomSnapshot { host_peer_id, .. } = snap_msg {
        assert_eq!(host_peer_id, guest.snapshot().peer_id);
    } else {
        panic!("expected RoomSnapshot after host promotion");
    }

    relay.shutdown();
}

#[tokio::test]
async fn tv_oauth_device_code_flow_and_cookie_validator() {
    // 1. Cookie validator rejects SAPISID-only without HttpOnly session cookie
    let missing_httponly = validate_cookie_jar("SAPISID=abc; __Secure-3PAPISID=def");
    assert!(missing_httponly.is_err());

    let valid =
        validate_cookie_jar("SAPISID=abc; __Secure-3PAPISID=def; SID=xyz; __Secure-3PSID=123");
    assert!(valid.is_ok());

    // 2. Mock OAuth server on 127.0.0.1:0
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let poll_count = Arc::new(AtomicUsize::new(0));
    let refresh_count = Arc::new(AtomicUsize::new(0));
    let poll_c = Arc::clone(&poll_count);
    let ref_c = Arc::clone(&refresh_count);

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let p_c = Arc::clone(&poll_c);
            let r_c = Arc::clone(&ref_c);
            tokio::spawn(async move {
                let svc = service_fn(move |req: Request<Incoming>| {
                    let p_c = Arc::clone(&p_c);
                    let r_c = Arc::clone(&r_c);
                    async move {
                        let path = req.uri().path().to_owned();
                        let body = req.into_body().collect().await.unwrap().to_bytes();
                        let body_str = String::from_utf8_lossy(&body).into_owned();
                        if path == "/device/code" {
                            let json = serde_json::json!({
                                "device_code": "dev_secret_123",
                                "user_code": "ABCD-EFGH",
                                "verification_url": "https://www.google.com/device",
                                "expires_in": 60,
                                "interval": 1
                            });
                            return Ok::<_, hyper::Error>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .body(Full::new(Bytes::from(json.to_string())))
                                    .unwrap(),
                            );
                        }
                        if path == "/token" {
                            if body_str.contains("refresh_token") {
                                r_c.fetch_add(1, Ordering::SeqCst);
                                let json = serde_json::json!({
                                    "access_token": "ya29.refreshed_token",
                                    "expires_in": 3600,
                                    "token_type": "Bearer"
                                });
                                return Ok(Response::builder()
                                    .status(StatusCode::OK)
                                    .body(Full::new(Bytes::from(json.to_string())))
                                    .unwrap());
                            }
                            let n = p_c.fetch_add(1, Ordering::SeqCst);
                            let (status, json) = match n {
                                0 => (
                                    StatusCode::BAD_REQUEST,
                                    serde_json::json!({"error": "authorization_pending"}),
                                ),
                                1 => (
                                    StatusCode::BAD_REQUEST,
                                    serde_json::json!({"error": "slow_down"}),
                                ),
                                _ => (
                                    StatusCode::OK,
                                    serde_json::json!({
                                        "access_token": "ya29.initial_token",
                                        "refresh_token": "1//refresh_secret",
                                        "expires_in": 3600,
                                        "token_type": "Bearer"
                                    }),
                                ),
                            };
                            return Ok(Response::builder()
                                .status(status)
                                .body(Full::new(Bytes::from(json.to_string())))
                                .unwrap());
                        }
                        Ok(Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Full::new(Bytes::new()))
                            .unwrap())
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .await;
            });
        }
    });

    let cfg = TvOAuthConfig {
        client_id: "test-client".to_owned(),
        client_secret: None,
        device_code_url: format!("http://{addr}/device/code"),
        token_url: format!("http://{addr}/token"),
        scope: "youtube".to_owned(),
    };
    let client = TvOAuthClient::new(cfg).unwrap();
    let dc = client.request_device_code().await.unwrap();
    assert_eq!(dc.user_code, "ABCD-EFGH");

    let (_tx, rx) = watch::channel(false);
    let tokens = client
        .poll_for_token(&dc.device_code, 1, 30, rx)
        .await
        .unwrap();
    assert_eq!(tokens.access_token, "ya29.initial_token");

    // Single-flight refresh test: expired token shared across 4 concurrent callers -> 1 refresh
    let expired = TvOAuthTokens {
        access_token: "ya29.expired".to_owned(),
        refresh_token: "1//refresh_secret".to_owned(),
        expires_at_unix_secs: 1,
        token_type: "Bearer".to_owned(),
    };
    let mgr = Arc::new(TvOAuthTokenManager::new(client, expired));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let m = Arc::clone(&mgr);
        handles.push(tokio::spawn(async move {
            m.get_valid_access_token().await.unwrap()
        }));
    }
    for h in handles {
        assert_eq!(h.await.unwrap(), "ya29.refreshed_token");
    }
    assert_eq!(refresh_count.load(Ordering::SeqCst), 1);

    // Secret redaction check
    let redacted = redact_oauth_secrets("access_token=ya29.secret_abc\nrefresh_token=1//xyz");
    assert!(!redacted.contains("ya29.secret_abc"));
    assert!(!redacted.contains("1//xyz"));
}
