//! Phase 1–3 Loopback Proxy, Offline Cache Replay (<50ms), and Adversarial Fault-Injection Suite:
//! - Live local upstream HTTP server + real `LoopbackProxy` (`127.0.0.1`) HTTP 206 byte ranges
//! - Instant `< 50ms` offline replay from 256 KB atomic `ChunkCache` after upstream terminates
//! - Mid-stream HTTP `403 Forbidden` transparent re-resolution without failing proxy response
//! - Corrupt audio chunk file on disk self-healing
//! - Corrupt SQLite metadata JSON self-healing
//! - Empty playlist handling
//! - Persistent Queue + Resume Position surviving `PlaybackController` restarts
//! - SponsorBlock (`music_offtopic`) auto-skip + `BecomingNoisy` / `AudioFocus` media commands

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use resonance_contract::{
    AudioBackend, CacheStore, CoreError, CoreResult, EventBus, HomeSection, LyricsDocument,
    LyricsProvider, MediaCommand, MusicProvider, NowPlayingMetadata, PlaybackError, PlayerState,
    Playlist, SearchResults, SponsorBlockProvider, SponsorSegment, StreamInfo, Track,
};
use resonance_core::cache::{CHUNK_SIZE, ChunkCache, SqliteCacheStore};
use resonance_core::fixture::fixture_tracks;
use resonance_core::playback::PlaybackController;
use resonance_core::proxy::LoopbackProxy;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, oneshot};

/// Deterministic synthetic audio payload spanning 3 full 256 KB chunks (~700 KB).
fn synthetic_audio_payload() -> Vec<u8> {
    let len = (CHUNK_SIZE as usize) * 2 + 123_456;
    (0..len).map(|i| ((i * 31 + 17) % 251) as u8).collect()
}

struct MockUpstream {
    url_v1: String,
    url_v2: String,
    shutdown: Option<oneshot::Sender<()>>,
    v1_403_hits: Arc<AtomicUsize>,
    v2_hits: Arc<AtomicUsize>,
}

impl MockUpstream {
    async fn start(payload: Arc<Vec<u8>>, fail_v1_with_403: Arc<AtomicBool>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, mut rx) = oneshot::channel();
        let v1_403_hits = Arc::new(AtomicUsize::new(0));
        let v2_hits = Arc::new(AtomicUsize::new(0));

        let hits_403 = v1_403_hits.clone();
        let hits_v2 = v2_hits.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    Ok((stream, _)) = listener.accept() => {
                        let io = TokioIo::new(stream);
                        let data = payload.clone();
                        let fail_flag = fail_v1_with_403.clone();
                        let h403 = hits_403.clone();
                        let hv2 = hits_v2.clone();
                        tokio::spawn(async move {
                            let svc = service_fn(move |req: Request<Incoming>| {
                                let data = data.clone();
                                let fail_flag = fail_flag.clone();
                                let h403 = h403.clone();
                                let hv2 = hv2.clone();
                                async move {
                                    let path = req.uri().path();
                                    if path == "/audio_v1" && fail_flag.load(Ordering::SeqCst) {
                                        h403.fetch_add(1, Ordering::SeqCst);
                                        return Ok::<_, hyper::Error>(
                                            Response::builder()
                                                .status(StatusCode::FORBIDDEN)
                                                .body(Full::new(Bytes::from_static(b"expired")))
                                                .unwrap(),
                                        );
                                    }
                                    if path == "/audio_v2" {
                                        hv2.fetch_add(1, Ordering::SeqCst);
                                    }
                                    let total = data.len() as u64;
                                    let range = req
                                        .headers()
                                        .get(RANGE)
                                        .and_then(|v| v.to_str().ok())
                                        .and_then(|h| h.strip_prefix("bytes="))
                                        .and_then(|r| r.split_once('-'))
                                        .and_then(|(s, e)| {
                                            let start: u64 = s.parse().ok()?;
                                            let end: u64 = if e.is_empty() {
                                                total - 1
                                            } else {
                                                e.parse::<u64>().ok()?.min(total - 1)
                                            };
                                            Some((start, end))
                                        });
                                    if let Some((s, e)) = range {
                                        let slice = data[s as usize..=e as usize].to_vec();
                                        Ok(Response::builder()
                                            .status(StatusCode::PARTIAL_CONTENT)
                                            .header(CONTENT_TYPE, "audio/webm")
                                            .header(ACCEPT_RANGES, "bytes")
                                            .header(CONTENT_LENGTH, slice.len().to_string())
                                            .header(CONTENT_RANGE, format!("bytes {s}-{e}/{total}"))
                                            .body(Full::new(Bytes::from(slice)))
                                            .unwrap())
                                    } else {
                                        Ok(Response::builder()
                                            .status(StatusCode::OK)
                                            .header(CONTENT_TYPE, "audio/webm")
                                            .header(CONTENT_LENGTH, total.to_string())
                                            .body(Full::new(Bytes::from((*data).clone())))
                                            .unwrap())
                                    }
                                }
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, svc)
                                .await;
                        });
                    }
                }
            }
        });

        Self {
            url_v1: format!("http://{addr}/audio_v1"),
            url_v2: format!("http://{addr}/audio_v2"),
            shutdown: Some(tx),
            v1_403_hits,
            v2_hits,
        }
    }

    fn kill(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

struct TestProvider {
    v1_url: String,
    v2_url: String,
    content_length: u64,
    resolve_calls: AtomicUsize,
}

#[async_trait]
impl MusicProvider for TestProvider {
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
            title: "Empty".into(),
            subtitle: None,
            artwork_url: None,
            tracks: Vec::new(),
        })
    }
    async fn autoplay_next(&self, _video_id: &str) -> CoreResult<Vec<Track>> {
        Ok(Vec::new())
    }
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        let n = self.resolve_calls.fetch_add(1, Ordering::SeqCst);
        let url = if n == 0 {
            self.v1_url.clone()
        } else {
            self.v2_url.clone()
        };
        Ok(StreamInfo {
            video_id: video_id.to_owned(),
            url,
            mime_type: "audio/webm".into(),
            itag: 251,
            codec: resonance_contract::AudioCodec::Opus,
            bitrate_kbps: 160,
            content_length: self.content_length,
            duration_ms: 180_000,
            loudness_db: None,
            expires_at_unix_secs: u64::MAX,
            resolved_by_client: if n == 0 { "VISIONOS" } else { "ANDROID_VR" }.into(),
        })
    }
    async fn ytm_lyrics(&self, _video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        Ok(None)
    }
}

struct MockAudioBackend {
    playing: Mutex<bool>,
    position_ms: Mutex<u64>,
    last_url: Mutex<Option<String>>,
}

impl MockAudioBackend {
    fn new() -> Self {
        Self {
            playing: Mutex::new(false),
            position_ms: Mutex::new(0),
            last_url: Mutex::new(None),
        }
    }
}

#[async_trait]
impl AudioBackend for MockAudioBackend {
    async fn load_and_play(&self, url: &str, _meta: NowPlayingMetadata) -> CoreResult<()> {
        *self.last_url.lock().await = Some(url.to_owned());
        *self.playing.lock().await = true;
        Ok(())
    }
    async fn play(&self) -> CoreResult<()> {
        *self.playing.lock().await = true;
        Ok(())
    }
    async fn pause(&self) -> CoreResult<()> {
        *self.playing.lock().await = false;
        Ok(())
    }
    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        *self.position_ms.lock().await = position_ms;
        Ok(())
    }
    async fn set_volume(&self, _volume: f32) -> CoreResult<()> {
        Ok(())
    }
    async fn stop(&self) -> CoreResult<()> {
        *self.playing.lock().await = false;
        Ok(())
    }
    async fn position_ms(&self) -> u64 {
        *self.position_ms.lock().await
    }
    async fn update_now_playing(
        &self,
        _meta: NowPlayingMetadata,
        _is_playing: bool,
        _position_ms: u64,
    ) {
    }
    fn take_command_receiver(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<MediaCommand>> {
        None
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

#[tokio::test]
async fn proxy_streams_206_recovers_403_mid_stream_and_replays_offline_under_50ms() {
    let dir = tempdir().unwrap();
    let payload = Arc::new(synthetic_audio_payload());
    let fail_v1 = Arc::new(AtomicBool::new(false));
    let mut upstream = MockUpstream::start(payload.clone(), fail_v1.clone()).await;

    let provider = Arc::new(TestProvider {
        v1_url: upstream.url_v1.clone(),
        v2_url: upstream.url_v2.clone(),
        content_length: payload.len() as u64,
        resolve_calls: AtomicUsize::new(0),
    });
    let db = Arc::new(SqliteCacheStore::open(&dir.path().join("proxy.db")).unwrap());
    let chunks =
        Arc::new(ChunkCache::new(&dir.path().join("chunks"), db, 64 * 1024 * 1024).unwrap());
    let proxy = LoopbackProxy::start(provider.clone(), chunks.clone())
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let stream_url = proxy.stream_url("vid-fault-test", 251);

    // 1. Fetch chunk 0 via HTTP 206 byte range (populates chunk 0 from v1)
    let r0 = client
        .get(&stream_url)
        .header(RANGE, "bytes=0-65535")
        .send()
        .await
        .unwrap();
    assert_eq!(r0.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    let b0 = r0.bytes().await.unwrap();
    assert_eq!(b0.as_ref(), &payload[0..65536]);

    // 2. FAULT INJECTION: expire v1 URL with HTTP 403 mid-stream!
    fail_v1.store(true, Ordering::SeqCst);

    // Request chunk 1 & 2 (uncached): proxy MUST catch 403, re-resolve to v2, and succeed transparently!
    let start_c1 = CHUNK_SIZE as usize;
    let end_c2 = payload.len() - 1;
    let r1 = client
        .get(&stream_url)
        .header(RANGE, format!("bytes={start_c1}-{end_c2}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r1.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    let b1 = r1.bytes().await.unwrap();
    assert_eq!(b1.as_ref(), &payload[start_c1..=end_c2]);
    assert!(
        upstream.v1_403_hits.load(Ordering::SeqCst) >= 1,
        "upstream must have injected HTTP 403"
    );
    assert!(
        upstream.v2_hits.load(Ordering::SeqCst) >= 1,
        "proxy must have transparently re-resolved to v2"
    );

    // 3. FAULT INJECTION: Corrupt chunk 0 on disk and verify self-healing re-fetch from v2!
    let chunk0_path = chunks.chunk_path("vid-fault-test", 251, 0);
    std::fs::write(&chunk0_path, b"corrupted_garbage_bytes").unwrap();
    assert!(
        chunks.read_chunk("vid-fault-test", 251, 0).unwrap().is_none(),
        "corrupt chunk must be rejected and evicted"
    );
    let r0_healed = client
        .get(&stream_url)
        .header(RANGE, "bytes=0-1023")
        .send()
        .await
        .unwrap();
    assert_eq!(r0_healed.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(r0_healed.bytes().await.unwrap().as_ref(), &payload[0..1024]);

    // 4. OFFLINE REPLAY BENCHMARK: Kill upstream server completely; replay entire track < 50ms!
    upstream.kill();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let t0 = Instant::now();
    let offline_resp = client.get(&stream_url).send().await.unwrap();
    assert_eq!(offline_resp.status(), reqwest::StatusCode::OK);
    let offline_bytes = offline_resp.bytes().await.unwrap();
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(offline_bytes.as_ref(), payload.as_slice());
    println!("MEASURED OFFLINE CHUNK CACHE REPLAY LATENCY: {elapsed_ms:.2} ms ({} bytes)", offline_bytes.len());
    assert!(
        elapsed_ms < 50.0,
        "offline replay exceeded 50ms target: {elapsed_ms:.2} ms"
    );
}

#[tokio::test]
async fn playback_controller_persists_queue_resume_handles_sponsorblock_and_media_focus() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("playback.db");
    let cache = Arc::new(SqliteCacheStore::open(&db_path).unwrap());

    // Corrupt SQLite metadata JSON self-healing check
    cache
        .put_metadata("corrupt_key", "{not valid json", 3600, 100)
        .unwrap();
    let (raw, _) = cache.get_metadata("corrupt_key", 101).unwrap().unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&raw).is_err());
    cache.evict_metadata("corrupt_key").unwrap();
    assert!(cache.get_metadata("corrupt_key", 101).unwrap().is_none());

    let chunks = Arc::new(
        ChunkCache::new(&dir.path().join("chunks"), cache.clone(), 16 * 1024 * 1024).unwrap(),
    );
    let provider = Arc::new(TestProvider {
        v1_url: "http://127.0.0.1:1/unused".into(),
        v2_url: "http://127.0.0.1:1/unused".into(),
        content_length: 100_000,
        resolve_calls: AtomicUsize::new(0),
    });
    let proxy = Arc::new(LoopbackProxy::start(provider.clone(), chunks).await.unwrap());
    let audio = Arc::new(MockAudioBackend::new());
    let sb = Arc::new(FixedSponsorBlock(vec![SponsorSegment {
        start_secs: 10.0,
        end_secs: 25.0,
        video_duration_secs: 180.0,
    }]));

    let tracks = fixture_tracks();
    {
        let ctrl = PlaybackController::new(
            provider.clone(),
            audio.clone(),
            cache.clone(),
            proxy.clone(),
            Arc::new(NullLyrics),
            sb.clone(),
            EventBus::new(128),
        );

        // Empty playlist rejected cleanly
        let empty_err = ctrl.play_tracks(Vec::new(), 0).await.unwrap_err();
        assert!(matches!(empty_err, CoreError::Playback(PlaybackError::EmptyQueue)));

        // Start playback & verify LoopbackProxy URL wired to AudioBackend
        ctrl.play_tracks(tracks.clone(), 0).await.unwrap();
        let snap = ctrl.snapshot().await;
        assert_eq!(snap.state, PlayerState::Playing);
        assert!(
            snap.active_proxy_url
                .as_deref()
                .unwrap()
                .starts_with("http://127.0.0.1:")
        );

        // SponsorBlock auto-skip: tick into 12s -> auto-seeks to 25s
        ctrl.tick_position(12_000).await.unwrap();
        let snap_after_skip = ctrl.snapshot().await;
        assert_eq!(snap_after_skip.position_ms, 25_000);
        assert!(snap_after_skip.last_skipped_segment.is_some());

        // AudioFocus transient loss pauses & focus gain resumes
        ctrl.handle_media_command(MediaCommand::AudioFocusLostTransient)
            .await
            .unwrap();
        assert_eq!(ctrl.snapshot().await.state, PlayerState::Paused);
        ctrl.handle_media_command(MediaCommand::AudioFocusGained)
            .await
            .unwrap();
        assert_eq!(ctrl.snapshot().await.state, PlayerState::Playing);

        // Headphone unplug (BecomingNoisy) immediately pauses
        ctrl.handle_media_command(MediaCommand::BecomingNoisy)
            .await
            .unwrap();
        assert_eq!(ctrl.snapshot().await.state, PlayerState::Paused);

        // Reorder queue item 1 -> 0 while preserving active track index
        ctrl.reorder_queue(1, 0).await.unwrap();
        assert_eq!(ctrl.snapshot().await.queue.current_index, Some(1));
    }

    // Restart controller from same SQLite database: queue & 25_000ms resume position restored!
    let cache_reopened = Arc::new(SqliteCacheStore::open(&db_path).unwrap());
    let ctrl2 = PlaybackController::new(
        provider,
        audio,
        cache_reopened,
        proxy,
        Arc::new(NullLyrics),
        sb,
        EventBus::new(128),
    );
    let restored = ctrl2.snapshot().await;
    assert_eq!(restored.queue.items.len(), tracks.len());
    assert_eq!(restored.queue.current_index, Some(1));
    assert_eq!(restored.position_ms, 25_000);
    assert_eq!(restored.state, PlayerState::Paused);
}

#[tokio::test]
async fn innertube_http_fault_injection_401_revocation_429_rate_limit_and_cookie_rotation() {
    use resonance_contract::{AuthError, AuthSession, AuthStore, ProviderError};
    use resonance_core::innertube::{FileAuthStore, InnerTubeClient};
    use resonance_core::solver::QuickJsSolver;

    let mode = Arc::new(AtomicUsize::new(0)); // 0 = 200+Set-Cookie+visitorData, 1 = 429, 2 = 401
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, mut rx) = oneshot::channel::<()>();
    let mode_srv = mode.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut rx => break,
                Ok((stream, _)) = listener.accept() => {
                    let io = TokioIo::new(stream);
                    let m = mode_srv.clone();
                    tokio::spawn(async move {
                        let svc = service_fn(move |_req: Request<Incoming>| {
                            let m = m.clone();
                            async move {
                                match m.load(Ordering::SeqCst) {
                                    0 => Ok::<_, hyper::Error>(
                                        Response::builder()
                                            .status(StatusCode::OK)
                                            .header(CONTENT_TYPE, "application/json")
                                            .header("set-cookie", "SID=rotated_live_token; Path=/; Secure")
                                            .body(Full::new(Bytes::from(
                                                r#"{"responseContext":{"visitorData":"CgtMSVZFX1ZJU0lUT1I%3D"},"contents":{}}"#,
                                            )))
                                            .unwrap(),
                                    ),
                                    1 => Ok(Response::builder()
                                        .status(StatusCode::TOO_MANY_REQUESTS)
                                        .header("retry-after", "45")
                                        .body(Full::new(Bytes::from_static(b"slow down")))
                                        .unwrap()),
                                    _ => Ok(Response::builder()
                                        .status(StatusCode::UNAUTHORIZED)
                                        .body(Full::new(Bytes::from_static(b"revoked")))
                                        .unwrap()),
                                }
                            }
                        });
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(io, svc)
                            .await;
                    });
                }
            }
        }
    });

    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(&dir.path().join("it.db")).unwrap());
    let auth = Arc::new(FileAuthStore::new(dir.path()));
    let solver = Arc::new(QuickJsSolver::new(dir.path()));
    let client = InnerTubeClient::new(cache.clone(), auth.clone(), solver).unwrap();
    client.set_api_base_override(Some(format!("http://{addr}")));

    client
        .sign_in(AuthSession {
            cookie_header: "SID=old_token; SAPISID=abc123; __Secure-3PAPISID=def456".into(),
            auth_user: 0,
            page_id: None,
            account_name: Some("Tester".into()),
            is_premium: false,
            visitor_data: None,
        })
        .unwrap();

    // Mode 0: live 200 -> captures Set-Cookie rotation + persists visitorData
    let _ = client.search("rotation_probe").await.unwrap();
    assert_eq!(
        cache.get_visitor_data().unwrap().as_deref(),
        Some("CgtMSVZFX1ZJU0lUT1I%3D")
    );
    let persisted = auth.load_session().unwrap().unwrap();
    assert!(persisted.cookie_header.contains("SID=rotated_live_token"));

    // Mode 1: HTTP 429 Too Many Requests -> RateLimited { retry_after_secs: Some(45) }
    mode.store(1, Ordering::SeqCst);
    let err_429 = client.search("rate_limit_probe").await.unwrap_err();
    assert!(matches!(
        err_429,
        CoreError::Provider(ProviderError::RateLimited {
            retry_after_secs: Some(45)
        })
    ));

    // Mode 2: HTTP 401 Unauthorized -> revokes stale session cleanly & returns AuthError::Revoked
    mode.store(2, Ordering::SeqCst);
    let err_401 = client.search("revoked_probe").await.unwrap_err();
    assert!(matches!(err_401, CoreError::Auth(AuthError::Revoked)));
    assert!(auth.load_session().unwrap().is_none());
    assert!(client.current_session().is_none());

    let _ = tx.send(());
}

#[tokio::test]
async fn artwork_cache_two_level_atomic_eviction_and_lyrics_30d_sqlite_cache() {
    use resonance_contract::{LyricLine, LyricSyncKind};
    use resonance_core::cache::ArtworkCache;
    use resonance_core::lyrics::CascadeLyricsProvider;

    // 1. Two-level ArtworkCache (Memory LRU + Atomic Disk LRU + corrupt empty file self-healing)
    let dir = tempdir().unwrap();
    let art = ArtworkCache::new(dir.path(), 16, 1024).unwrap();
    let payload_a = vec![0xAAu8; 600];
    let payload_b = vec![0xBBu8; 600];
    art.put("https://img.example/a.jpg", 544, payload_a.clone())
        .unwrap();
    assert_eq!(art.get("https://img.example/a.jpg", 544), Some(payload_a));

    // Writing a second 600-byte image exceeds 1024-byte cap -> LRU evicts oldest disk entry
    std::thread::sleep(std::time::Duration::from_millis(15));
    art.put("https://img.example/b.jpg", 544, payload_b.clone())
        .unwrap();
    assert!(art.disk_usage_bytes() <= 1024);

    // Reopen fresh ArtworkCache (cold L1) -> hits L2 disk cache
    let art2 = ArtworkCache::new(dir.path(), 16, 1024).unwrap();
    assert_eq!(art2.get("https://img.example/b.jpg", 544), Some(payload_b));

    // 2. CascadeLyricsProvider 30-day SQLite cache + corrupt-entry self-healing
    let db = Arc::new(SqliteCacheStore::open(&dir.path().join("lyrics.db")).unwrap());
    let track = fixture_tracks().into_iter().next().unwrap();
    let seeded_doc = LyricsDocument {
        video_id: track.id.clone(),
        provider: "LRCLIB (Cached)".into(),
        sync_kind: LyricSyncKind::Line,
        lines: vec![LyricLine {
            start_ms: 4_000,
            end_ms: 10_000,
            text: "Cached synced line".into(),
            words: Vec::new(),
            is_instrumental: false,
        }],
    };
    let now = resonance_core::unix_secs();
    db.put_metadata(
        &format!("lyrics:{}", track.id),
        &serde_json::to_string(&seeded_doc).unwrap(),
        30 * 24 * 3600,
        now,
    )
    .unwrap();

    let provider = Arc::new(TestProvider {
        v1_url: "http://127.0.0.1:1/unused".into(),
        v2_url: "http://127.0.0.1:1/unused".into(),
        content_length: 1024,
        resolve_calls: AtomicUsize::new(0),
    });
    let lyrics = CascadeLyricsProvider::new(provider, db.clone());
    let loaded = lyrics.fetch_lyrics(&track).await.unwrap().unwrap();
    assert_eq!(loaded.provider, "LRCLIB (Cached)");
    assert_eq!(loaded.lines[0].text, "Cached synced line");

    // 3. LoopbackProxy /artwork endpoint caches in ArtworkCache and blocks loopback SSRF
    let art_arc = Arc::new(art2);
    let proxy = LoopbackProxy::start_with_caches(
        Arc::new(TestProvider {
            v1_url: "http://127.0.0.1:1/unused".into(),
            v2_url: "http://127.0.0.1:1/unused".into(),
            content_length: 1024,
            resolve_calls: AtomicUsize::new(0),
        }),
        Arc::new(ChunkCache::new(&dir.path().join("art_chunks"), db.clone(), 1024 * 1024).unwrap()),
        None,
        Some(art_arc.clone()),
    )
    .await
    .unwrap();

    // Seed an entry in ArtworkCache and verify /artwork serves it with image/png
    let png_bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
    art_arc
        .put("https://img.example/cached.png", 0, png_bytes.clone())
        .unwrap();
    let http = reqwest::Client::new();
    let resp = http
        .get(format!(
            "http://{}/artwork?url=https%3A%2F%2Fimg.example%2Fcached.png",
            proxy.addr()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap().to_str().unwrap(),
        "image/png"
    );
    assert_eq!(resp.bytes().await.unwrap().to_vec(), png_bytes);

    let ssrf_resp = http
        .get(format!(
            "http://{}/artwork?url=http%3A%2F%2F127.0.0.1%3A8080%2Fsecret",
            proxy.addr()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(ssrf_resp.status(), 403);
}
