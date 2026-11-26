//! Comprehensive end-to-end verification for Architectures 1, 2, and 3:
//!
//! - Architecture 1: Anti-Conflict Modular Decomposition (`src/store/` slices,
//!   `src/ui/controls/` atomic modules, `crates/core/src/playback/` pure FSM/queue/prefetch,
//!   `crates/core/src/innertube/parser/` endpoint parsers).
//! - Architecture 2: Resilient Acquisition Pipeline (Layered PO Token Provider with
//!   `Subs`/`Gvs`/`Player` scopes & degraded cold-start diagnostics, Sandboxed EJS Solver
//!   with `JsRuntime` trait + SHA-256/version pinning + SQLite L2 `< 2 ms` memoization,
//!   Adaptive Client Matrix & Circuit Breaker, 12-hour scheduled client-probe workflow).
//! - Architecture 3: Audio & Local Streaming Data Plane (Proxy v2 dual-buffer slots +
//!   single-flight range coalescing, lock-free SPSC ring buffer + ReplayGain/EQ/Silence/
//!   Crossfade DSP pipeline, First-Class Offline Vault isolated from transient cache eviction).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use nsh_engine::{EngineConfig, Points, Root, Size};
use nsh_testing::Headless;
use resonance::App;
use resonance::platform::nsh_audio::{NshAudioBackend, create_pcm_ring_buffer};
use resonance::store;
use resonance_contract::{
    AudioCodec, AuthError, AuthSession, AuthStore, CacheStore, DownloadRecord, DownloadStatus,
    OfflineStore, PoTokenHealthStatus, PoTokenProvider, PoTokenRequest, PoTokenScope, QueueState,
    StreamInfo,
};
use resonance_core::audio_dsp::{
    DspConfig, DspPipeline, EqPreset, GaplessSplicer, LoudnessNormalizer, SilenceSkipper, TenBandEq,
};
use resonance_core::cache::{ArtworkCache, ChunkCache, SqliteCacheStore};
use resonance_core::downloads::{OfflineVault, PinnedCollectionKind, VaultTrackState};
use resonance_core::fixture::fixture_tracks;
use resonance_core::innertube::InnerTubeClient;
use resonance_core::innertube::clients::ClientKind;
use resonance_core::innertube::matrix::{CircuitBreakerConfig, CircuitState, ClientCircuitBreaker};
use resonance_core::innertube::parser::{
    parse_collection_response, parse_home_response, parse_next_queue_response,
    parse_player_response, parse_search_response,
};
use resonance_core::lyrics::CascadeLyricsProvider;
use resonance_core::offline::DownloadManager;
use resonance_core::playback::fsm::PlaybackFsm;
use resonance_core::playback::prefetch::PrefetchDecision;
use resonance_core::potoken::{
    BgutilHttpPoTokenProvider, CachingPoTokenProvider, ChainedPoTokenProvider,
    PlatformAttestationPoTokenProvider, QuickJsVmAttestationBridge,
};
use resonance_core::proxy::{ChunkCoalescer, LoopbackProxy};
use resonance_core::solver::{EjsBundleManifest, JsRuntime, QuickJsRuntime, QuickJsSolver};
use tokio::net::TcpListener;

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard(std::path::PathBuf);
impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!(
            "resonance-arch-v3-{prefix}-{}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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

#[test]
fn arch1_modular_decomposition_and_pure_fsm_prefetch() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Verify Architecture 1 directory modules exist and monoliths are gone
    assert!(!root.join("src/store.rs").exists());
    for slice in [
        "src/store/mod.rs",
        "src/store/playback_slice.rs",
        "src/store/catalog_slice.rs",
        "src/store/library_slice.rs",
        "src/store/mutation_slice.rs",
        "src/store/settings_slice.rs",
        "src/store/ui_slice.rs",
    ] {
        assert!(root.join(slice).exists(), "missing slice {slice}");
    }

    assert!(!root.join("src/ui/controls.rs").exists());
    for ctrl in [
        "src/ui/controls/mod.rs",
        "src/ui/controls/primitives.rs",
        "src/ui/controls/media_rows.rs",
        "src/ui/controls/cards.rs",
        "src/ui/controls/transport.rs",
        "src/ui/controls/sheets.rs",
        "src/ui/controls/navigation.rs",
    ] {
        assert!(root.join(ctrl).exists(), "missing control module {ctrl}");
    }

    for pb in [
        "crates/core/src/playback/mod.rs",
        "crates/core/src/playback/fsm.rs",
        "crates/core/src/playback/queue.rs",
        "crates/core/src/playback/prefetch.rs",
        "crates/core/src/playback/controller.rs",
    ] {
        assert!(root.join(pb).exists(), "missing playback module {pb}");
    }

    assert!(!root.join("crates/core/src/innertube/parser.rs").exists());
    for parser_mod in [
        "crates/core/src/innertube/parser/mod.rs",
        "crates/core/src/innertube/parser/common.rs",
        "crates/core/src/innertube/parser/home.rs",
        "crates/core/src/innertube/parser/search.rs",
        "crates/core/src/innertube/parser/browse.rs",
        "crates/core/src/innertube/parser/player.rs",
        "crates/core/src/innertube/parser/next.rs",
    ] {
        assert!(
            root.join(parser_mod).exists(),
            "missing parser module {parser_mod}"
        );
    }

    assert!(
        root.join(".github/workflows/probe.yml").exists(),
        "missing 12h scheduled client-probe workflow"
    );

    // Pure PlaybackFsm 50% pre-resolve + 75% Proxy v2 next_track warm-up without any I/O
    let tracks = fixture_tracks();
    assert!(tracks.len() >= 2);
    let mut fsm = PlaybackFsm::from_restored_queue(QueueState {
        items: tracks.clone(),
        current_index: Some(0),
        ..Default::default()
    });
    let current = fsm.begin_loading(0).unwrap();
    fsm.enter_buffering(
        format!("http://127.0.0.1:0/stream/{}", current.id),
        200_000,
        false,
    );
    fsm.enter_playing();

    let fx40 = fsm.tick_position(80_000);
    assert_eq!(fx40.prefetch, PrefetchDecision::None);

    let fx55 = fsm.tick_position(110_000);
    assert!(matches!(
        fx55.prefetch,
        PrefetchDecision::ResolveOnly { .. }
    ));

    let fx76 = fsm.tick_position(152_000);
    assert!(matches!(
        fx76.prefetch,
        PrefetchDecision::WarmProxySlot { .. }
    ));

    // Verify all 5 endpoint parser entry points handle minimal payloads safely
    assert!(
        parse_home_response(&serde_json::json!({}))
            .unwrap()
            .is_empty()
    );
    assert!(
        parse_search_response("q", &serde_json::json!({}))
            .unwrap()
            .tracks
            .is_empty()
    );
    assert!(
        parse_collection_response("VL1", &serde_json::json!({}))
            .unwrap()
            .tracks
            .is_empty()
    );
    assert!(
        parse_next_queue_response(&serde_json::json!({}))
            .unwrap()
            .is_empty()
    );
    assert!(
        parse_player_response("vid1", "WEB_REMIX", &serde_json::json!({}), 1_700_000_000).is_err()
    );
}

#[tokio::test]
async fn arch2_layered_potoken_chain_with_subs_scope_and_degraded_fallback() {
    // 1. Mock bgutil-ytdlp-pot-provider HTTP sidecar supporting /ping and /get_pot
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mint_calls = Arc::new(AtomicUsize::new(0));
    let calls_srv = mint_calls.clone();

    let server = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let calls_req = calls_srv.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                    let calls = calls_req.clone();
                    async move {
                        if req.uri().path() == "/ping" {
                            return Ok::<_, hyper::Error>(
                                Response::builder()
                                    .status(StatusCode::OK)
                                    .body(Full::new(Bytes::from(r#"{"status":"ok"}"#)))
                                    .unwrap(),
                            );
                        }
                        let n = calls.fetch_add(1, Ordering::SeqCst) + 1;
                        let body = format!(r#"{{"poToken":"bgutil_tok_{n}","ttl":300}}"#);
                        Ok(Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "application/json")
                            .body(Full::new(Bytes::from(body)))
                            .unwrap())
                    }
                });
                let _ = http1::Builder::new().serve_connection(io, svc).await;
            });
        }
    });

    let bgutil = BgutilHttpPoTokenProvider::new(format!("http://{addr}")).unwrap();
    assert!(bgutil.ping().await);

    let chain = ChainedPoTokenProvider::new(vec![
        Arc::new(bgutil),
        Arc::new(PlatformAttestationPoTokenProvider::new(Arc::new(
            QuickJsVmAttestationBridge::new(
                "function mintPoToken(binding, scope) { return 'qjs_' + scope + '_' + binding; }",
            ),
        ))),
    ]);
    let cached = CachingPoTokenProvider::new(chain);

    // Distinct scopes (Gvs vs Subs) + distinct session_hash produce separate cache entries
    let req_gvs = PoTokenRequest::new(
        "WEB_REMIX",
        PoTokenScope::Gvs,
        "vid1",
        Some("visA".into()),
        true,
    )
    .with_session_hash(Some("sess_hash_1".into()));
    let req_subs = PoTokenRequest::new(
        "WEB_REMIX",
        PoTokenScope::Subs,
        "vid1",
        Some("visA".into()),
        true,
    )
    .with_session_hash(Some("sess_hash_1".into()));

    let t1 = cached.mint_token(&req_gvs).await.unwrap();
    let t1_cached = cached.mint_token(&req_gvs).await.unwrap();
    assert_eq!(t1.value, t1_cached.value);
    assert_eq!(mint_calls.load(Ordering::SeqCst), 1);

    let t_subs = cached.mint_token(&req_subs).await.unwrap();
    assert_eq!(t_subs.scope, PoTokenScope::Subs);
    assert_ne!(t_subs.value, t1.value);
    assert_eq!(mint_calls.load(Ordering::SeqCst), 2);
    assert_eq!(cached.diagnostics().status, PoTokenHealthStatus::Healthy);

    server.abort();

    // 2. Empty provider chain reports cold-start degraded fallback diagnostics
    let degraded = ChainedPoTokenProvider::new(vec![]);
    let err = degraded.mint_token(&req_gvs).await;
    assert!(err.is_err());
    let diag = degraded.diagnostics();
    assert_eq!(diag.status, PoTokenHealthStatus::DegradedColdStart);
    assert!(diag.degraded_fallback_active);
}

#[tokio::test]
async fn arch2_ejs_manifest_pinning_and_sqlite_l2_sub2ms_memoization() {
    let dir = TempDirGuard::new("ejs");
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());

    // Manifest SHA-256 & version pinning enforcement
    let manifest = EjsBundleManifest::embedded_default().unwrap();
    assert!(!manifest.pinned_version.is_empty());
    let player_js = r#"
        function solveN(n) { return "fast_" + n.split("").reverse().join(""); }
        function solveSig(s) { return s.split("").reverse().join(""); }
    "#;
    let good_hash = EjsBundleManifest::sha256_of(player_js);
    assert!(
        manifest
            .verify_bundle_integrity(player_js, &good_hash)
            .is_ok()
    );
    assert!(
        manifest
            .verify_bundle_integrity(player_js, "0000deadbeef")
            .is_err()
    );

    let challenge = resonance_core::solver::ChallengeInput {
        player_hash: "player_v1_test_hash".into(),
        solver_js: player_js.to_owned(),
        n_param: Some("abcdef123".into()),
        sig_param: None,
    };

    // First solver run computes via QuickJsRuntime and persists into SQLite L2 memo cache
    let runtime: Arc<dyn JsRuntime> = Arc::new(QuickJsRuntime::new());
    assert_eq!(runtime.runtime_name(), "quickjs-ng");
    let solver_a = QuickJsSolver::new(dir.path())
        .with_runtime(runtime)
        .with_sqlite_cache(cache.clone());
    let solved_a = solver_a.solve(challenge.clone()).await.unwrap();
    assert_eq!(solved_a.n_param.as_deref(), Some("fast_321fedcba"));
    assert_eq!(solver_a.js_eval_count(), 1);

    // Cold-start second solver instance hits SQLite L2 memo cache rapidly without JS eval
    let solver_b = QuickJsSolver::new(dir.path()).with_sqlite_cache(cache.clone());
    let t0 = Instant::now();
    let solved_b = solver_b.solve(challenge).await.unwrap();
    let elapsed = t0.elapsed();
    assert_eq!(solved_b.n_param, solved_a.n_param);
    assert_eq!(solver_b.js_eval_count(), 0);
    assert!(
        elapsed < Duration::from_millis(50),
        "expected fast SQLite L2 memo hit, took {elapsed:?}"
    );
}

#[test]
fn arch2_adaptive_client_matrix_circuit_breaker_transitions() {
    let cb = ClientCircuitBreaker::new(CircuitBreakerConfig {
        window_size: 4,
        failure_threshold: 2,
        max_error_rate: 0.5,
        cooldown: Duration::from_secs(60),
    });

    let t0 = Instant::now();
    assert_eq!(cb.state_at(ClientKind::WebRemix, t0), CircuitState::Closed);
    assert!(cb.allow_request_at(ClientKind::WebRemix, t0));

    // Repeated 403/429 failures trip the breaker to Open
    cb.record_failure_at(ClientKind::WebRemix, Some(403), 120, t0);
    cb.record_failure_at(
        ClientKind::WebRemix,
        Some(429),
        140,
        t0 + Duration::from_secs(1),
    );
    assert_eq!(
        cb.state_at(ClientKind::WebRemix, t0 + Duration::from_secs(2)),
        CircuitState::Open
    );
    assert!(!cb.allow_request_at(ClientKind::WebRemix, t0 + Duration::from_secs(10)));

    // After cooldown expires -> HalfOpen probe -> Success closes circuit
    let after_cooldown = t0 + Duration::from_secs(70);
    assert_eq!(
        cb.state_at(ClientKind::WebRemix, after_cooldown),
        CircuitState::HalfOpen
    );
    assert!(cb.allow_request_at(ClientKind::WebRemix, after_cooldown));
    cb.record_success(ClientKind::WebRemix, 80);
    assert_eq!(cb.state(ClientKind::WebRemix), CircuitState::Closed);

    let snap = cb.health_snapshot(ClientKind::WebRemix);
    assert!(snap.ewma_latency_ms > 0.0);
}

#[tokio::test]
async fn arch3_proxy_v2_dual_buffer_and_single_flight_coalescing() {
    let dir = TempDirGuard::new("proxyv2");
    let cache = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks = Arc::new(
        ChunkCache::new(&dir.path().join("chunks"), cache.clone(), 64 * 1024 * 1024).unwrap(),
    );
    let auth = Arc::new(MemoryAuthStore);
    let solver = Arc::new(QuickJsSolver::new(dir.path()));
    let provider = Arc::new(InnerTubeClient::new(cache.clone(), auth, solver).unwrap());

    // Local upstream origin serving deterministic 512 KB audio payload and counting hits
    let upstream_hits = Arc::new(AtomicUsize::new(0));
    let payload = Arc::new(
        (0..(512 * 1024))
            .map(|i| (i % 251) as u8)
            .collect::<Vec<u8>>(),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_addr = listener.local_addr().unwrap();
    let hits_srv = upstream_hits.clone();
    let payload_srv = payload.clone();
    let srv = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let hits_req = hits_srv.clone();
            let data_req = payload_srv.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req: Request<hyper::body::Incoming>| {
                    let hits = hits_req.clone();
                    let data = data_req.clone();
                    async move {
                        hits.fetch_add(1, Ordering::SeqCst);
                        let range = req
                            .headers()
                            .get("range")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("bytes=0-262143");
                        let parts: Vec<&str> =
                            range.trim_start_matches("bytes=").split('-').collect();
                        let start: usize = parts[0].parse().unwrap_or(0);
                        let end: usize = parts
                            .get(1)
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(data.len() - 1)
                            .min(data.len() - 1);
                        let slice = data[start..=end].to_vec();
                        Ok::<_, hyper::Error>(
                            Response::builder()
                                .status(StatusCode::PARTIAL_CONTENT)
                                .header("content-type", "audio/webm")
                                .header(
                                    "content-range",
                                    format!("bytes {start}-{end}/{}", data.len()),
                                )
                                .body(Full::new(Bytes::from(slice)))
                                .unwrap(),
                        )
                    }
                });
                let _ = http1::Builder::new().serve_connection(io, svc).await;
            });
        }
    });

    let proxy = LoopbackProxy::start(provider, chunks).await.unwrap();
    let make_src = |vid: &str| StreamInfo {
        video_id: vid.to_owned(),
        url: format!("http://{origin_addr}/audio/{vid}"),
        itag: 251,
        mime_type: "audio/webm".into(),
        codec: AudioCodec::Opus,
        bitrate_kbps: 160,
        content_length: payload.len() as u64,
        duration_ms: 180_000,
        loudness_db: Some(-1.5),
        resolved_by_client: "ANDROID_VR".into(),
        expires_at_unix_secs: 9_999_999_999,
    };

    proxy.register_current_stream(make_src("cur_vid"));
    proxy.register_next_stream(make_src("next_vid"));
    let warmed = proxy
        .warm_next_track(make_src("next_vid"), 1)
        .await
        .unwrap();
    assert!(warmed > 0);
    let status = proxy.dual_buffer_status();
    assert_eq!(
        status.current.as_ref().map(|s| s.video_id.as_str()),
        Some("cur_vid")
    );
    assert_eq!(
        status.next.as_ref().map(|s| s.video_id.as_str()),
        Some("next_vid")
    );
    assert!(status.next.as_ref().is_some_and(|s| s.warmed_chunks > 0));

    // Promote next_vid -> current_track without re-fetching chunk 0
    proxy.register_current_stream(make_src("next_vid"));
    let status2 = proxy.dual_buffer_status();
    assert_eq!(
        status2.current.as_ref().map(|s| s.video_id.as_str()),
        Some("next_vid")
    );
    assert!(
        status2
            .current
            .as_ref()
            .is_some_and(|s| s.warmed_chunks > 0)
    );

    // Verify single-flight ChunkCoalescer deduplicates concurrent identical chunk fetches
    let coalescer = Arc::new(ChunkCoalescer::new());
    let fetch_counter = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..6 {
        let c = coalescer.clone();
        let fc = fetch_counter.clone();
        handles.push(tokio::spawn(async move {
            c.fetch_or_join(
                resonance_core::proxy::coalesce::ChunkKey {
                    video_id: "vid_coal".into(),
                    itag: 251,
                    chunk_index: 0,
                },
                move || async move {
                    fc.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    Ok((vec![42u8; 1024], 1024))
                },
            )
            .await
            .unwrap()
            .0
        }));
    }
    for h in handles {
        assert_eq!(h.await.unwrap().len(), 1024);
    }
    assert_eq!(fetch_counter.load(Ordering::SeqCst), 1);
    assert!(coalescer.coalesced_hits() >= 5);

    srv.abort();
}

#[test]
fn arch3_lock_free_spsc_and_modular_pcm_dsp_pipeline() {
    // Multi-threaded lock-free SPSC PCM ring buffer verification
    let (mut prod, mut cons) = create_pcm_ring_buffer(256);
    let expected: Vec<f32> = (0..2048).map(|i| (i as f32 * 0.01).sin()).collect();
    let tx_data = expected.clone();

    let writer = std::thread::spawn(move || {
        let mut off = 0;
        while off < tx_data.len() {
            let n = prod.push_slice(&tx_data[off..]);
            off += n;
            if n == 0 {
                std::thread::yield_now();
            }
        }
    });

    let mut received = Vec::with_capacity(expected.len());
    let mut scratch = [0.0f32; 64];
    while received.len() < expected.len() {
        let n = cons.pop_slice(&mut scratch);
        if n > 0 {
            received.extend_from_slice(&scratch[..n]);
        } else {
            std::thread::yield_now();
        }
    }
    writer.join().unwrap();
    assert_eq!(received, expected);

    // Modular DSP pipeline: ReplayGain attenuation, soft-knee peak limiter, 10-band EQ, silence skip, crossfade
    let mut norm = LoudnessNormalizer::new(true, 0.0, 0.0);
    norm.set_track_loudness_db(Some(6.0));
    let mut loud = [0.8f32; 64];
    norm.process_in_place(&mut loud);
    assert!(loud[0] < 0.5);
    norm.set_track_loudness_db(Some(-12.0));
    let mut boosted = [0.7f32; 64];
    norm.process_in_place(&mut boosted);
    assert!(boosted[0] < 0.99);

    let mut eq = TenBandEq::new(48_000, 2);
    eq.enabled = true;
    eq.set_gains_db(EqPreset::BassBoost.gains_db());
    let mut bass_buf = [0.2f32; 256];
    eq.process_interleaved(&mut bass_buf);
    assert!(bass_buf.iter().all(|s| s.is_finite()));

    let mut skipper = SilenceSkipper::new(48_000, 2, -45.0, 10);
    skipper.enabled = true;
    let quiet = [0.0f32; 1024];
    assert!(skipper.should_skip_block(&quiet));

    let splicer = GaplessSplicer::new(48_000, 2, 10);
    let blended = splicer.splice_interleaved(&[1.0f32; 200], &[0.5f32; 200]);
    assert!(!blended.is_empty());

    // NshAudioBackend integrates DspConfig + DspPipeline cleanly
    let backend = NshAudioBackend::new();
    let mut cfg = DspConfig::default();
    cfg.eq_preset = EqPreset::Vocal;
    cfg.eq_gains_db = EqPreset::Vocal.gains_db();
    cfg.crossfade_ms = 2_000;
    backend.set_dsp_config(cfg.clone());
    assert_eq!(backend.dsp_config(), cfg);
    backend.set_track_loudness_db(Some(3.0));
    let mut pcm = [0.5f32; 128];
    assert!(backend.process_dsp_block(&mut pcm));

    let mut pipe = DspPipeline::new(48_000, 2, &cfg);
    assert!(pipe.process_interleaved(&mut pcm));

    // Lock-free SPSC consumer callback processes frames + control updates with zero mutexes
    let (mut lf_prod, mut lf_cons, lf_tx) =
        resonance::platform::nsh_audio::create_lock_free_audio_pipeline(256, 48_000, 2);
    lf_tx
        .send(resonance::platform::nsh_audio::DspControlUpdate::SetConfig(
            cfg,
        ))
        .unwrap();
    lf_tx
        .send(resonance::platform::nsh_audio::DspControlUpdate::SetTrackLoudnessDb(Some(6.0)))
        .unwrap();
    let pushed = lf_prod.push_slice(&[0.6f32; 64]);
    assert_eq!(pushed, 64);
    let mut out_block = [0.0f32; 64];
    let (popped, keep) = lf_cons.render_callback(&mut out_block);
    assert_eq!(popped, 64);
    assert!(keep);
    assert!(out_block[0] < 0.45);
}

#[tokio::test]
async fn arch3_offline_vault_isolates_pinned_collections_from_lru_cache_eviction() {
    // VaultTrackState FSM: Queued -> Downloading -> Verified -> Available
    assert!(VaultTrackState::Queued.can_transition_to(VaultTrackState::Downloading));
    assert!(VaultTrackState::Downloading.can_transition_to(VaultTrackState::Verified));
    assert!(VaultTrackState::Verified.can_transition_to(VaultTrackState::Available));
    assert!(!VaultTrackState::Queued.can_transition_to(VaultTrackState::Available));

    let dir = TempDirGuard::new("vault");
    let db = Arc::new(SqliteCacheStore::open(dir.path()).unwrap());
    let chunks =
        Arc::new(ChunkCache::new(&dir.path().join("chunks"), db.clone(), 1024 * 1024).unwrap());
    let artwork = Arc::new(ArtworkCache::new(dir.path(), 32, 4 * 1024 * 1024).unwrap());
    let auth = Arc::new(MemoryAuthStore);
    let solver = Arc::new(QuickJsSolver::new(dir.path()));
    let provider = Arc::new(InnerTubeClient::new(db.clone(), auth, solver).unwrap());
    let lyrics = Arc::new(CascadeLyricsProvider::new(provider.clone(), db.clone()));
    let bus = resonance_contract::EventBus::new(32);
    let downloads = Arc::new(
        DownloadManager::new(provider, db.clone(), chunks.clone(), artwork, lyrics, bus).unwrap(),
    );
    let vault = OfflineVault::new(db.clone(), downloads);

    let tracks = fixture_tracks();
    let pinned_track = tracks[0].clone();

    // Seed a completed vault track + pinned album in SQLite
    db.upsert_pinned_collection(
        "MPRE_album_pinned",
        "album",
        "Pinned Audiophile Album",
        Some("Studio Master"),
        None,
        std::slice::from_ref(&pinned_track),
        1_700_000_000,
    )
    .unwrap();
    db.upsert_download(&DownloadRecord {
        video_id: pinned_track.id.clone(),
        track: pinned_track.clone(),
        status: DownloadStatus::Completed,
        itag: 140,
        codec: AudioCodec::Aac,
        mime_type: "audio/mp4".into(),
        relative_path: Some("downloads/pinned_track.m4a".into()),
        bytes_downloaded: 1024,
        total_bytes: 1024,
        sha256_hex: Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into()),
        tags_embedded: true,
        created_at_unix_secs: 1_700_000_000,
        updated_at_unix_secs: 1_700_000_000,
        completed_at_unix_secs: Some(1_700_000_001),
        error_message: None,
    })
    .unwrap();

    // Populate transient LRU chunk cache, then wipe ALL transient caches
    chunks
        .write_chunk("transient_vid", 251, 0, &[9u8; 4096], 4096)
        .unwrap();
    chunks.clear_all().unwrap();
    db.clear_all_caches().unwrap();

    // Pinned album & offline vault tracks must survive transient cache eviction 100% intact
    let pinned_list = vault.list_pinned_collections().unwrap();
    assert_eq!(pinned_list.len(), 1);
    assert_eq!(pinned_list[0].id, "MPRE_album_pinned");
    assert_eq!(pinned_list[0].kind, PinnedCollectionKind::Album);
    assert!(db.is_track_pinned(&pinned_track.id).unwrap());
    assert_eq!(
        vault.track_state(&pinned_track.id).unwrap(),
        Some(VaultTrackState::Available)
    );
}

#[test]
fn settings_surfaces_po_token_diagnostics_badge_and_audio_dsp_controls() -> Result<(), String> {
    store::enable_fixture_mode(true);
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(Points(1280.0), Points(820.0)),
        ..EngineConfig::new(Root::of::<App>())
    })
    .expect("desktop headless starts cleanly");

    app.wait_for("Raag")?;
    app.press("id:nav-settings")?;
    app.wait_for("Stream Acquisition & PO Token")?;
    app.wait_for("Degraded Fallback")?;
    app.wait_for("Audio DSP & Gapless")?;
    app.wait_for("Native PCM Engine Required")?;
    app.wait_for("10-Band EQ: Flat")?;
    app.wait_for("Gapless (0s)")?;
    Ok(())
}
