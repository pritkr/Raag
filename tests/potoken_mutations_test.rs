//! Offline mock-server verification suite for:
//! - Option A: `PoTokenProvider`, policy-gated `&pot=` attachment, `AuthMode::AnonymousOnly`
//!   isolation, `CachingPoTokenProvider` single-flight & invalidation, `SidecarPoTokenProvider`
//!   SSRF/timeout guards, and `PoToken` Debug/secret redaction.
//! - Option D: Authenticated InnerTube write mutations (`/like/*`, `/browse/edit_playlist`, `/atr`),
//!   `SAPISIDHASH` signing, SWR cache eviction, `MutationWorker` FIFO + ATR coalescing,
//!   `401` session revocation + queue drain, `429` backoff, and secret redaction.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use resonance_contract::{
    AppEvent, AuthSession, CacheStore, EventBus, InnerTubeMutations, LikeAction, MusicProvider,
    MutationCommand, MutationError, PlaylistEditOp, PoToken, PoTokenError, PoTokenProvider,
    PoTokenRequest, PoTokenScope, WatchtimeReport,
};
use resonance_core::cache::SqliteCacheStore;
use resonance_core::innertube::auth::redact_secrets;
use resonance_core::innertube::clients::ClientConfig;
use resonance_core::innertube::matrix::{ClientMatrix, PoTokenPolicy};
use resonance_core::innertube::{ClientKind, FileAuthStore, InnerTubeClient, MutationWorker};
use resonance_core::potoken::{CachingPoTokenProvider, SidecarPoTokenProvider};
use resonance_core::solver::QuickJsSolver;
use serde_json::{Value, json};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, oneshot};

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    query: String,
    authorization: Option<String>,
    cookie: Option<String>,
    origin: Option<String>,
    auth_user: Option<String>,
    body_json: Option<Value>,
}

#[derive(Debug, Clone)]
struct ScriptedResponse {
    status: u16,
    body: String,
    retry_after: Option<String>,
    delay_ms: u64,
}

impl ScriptedResponse {
    fn ok_json(val: Value) -> Self {
        Self {
            status: 200,
            body: val.to_string(),
            retry_after: None,
            delay_ms: 0,
        }
    }

    fn status_body(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            retry_after: None,
            delay_ms: 0,
        }
    }
}

struct MockInnerTubeServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    scripted: Arc<Mutex<VecDeque<ScriptedResponse>>>,
    default_player_url: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for MockInnerTubeServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl MockInnerTubeServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");
        let default_player_url =
            format!("{base_url}/videoplayback?expire=1999999999&n=abc123&sig=RAW_SIG");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let scripted = Arc::new(Mutex::new(VecDeque::new()));
        let (tx, mut rx) = oneshot::channel();

        let reqs_ref = requests.clone();
        let script_ref = scripted.clone();
        let player_url_ref = default_player_url.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => break,
                    Ok((stream, _)) = listener.accept() => {
                        let io = TokioIo::new(stream);
                        let reqs = reqs_ref.clone();
                        let script = script_ref.clone();
                        let player_url = player_url_ref.clone();
                        tokio::spawn(async move {
                            let svc = service_fn(move |req: Request<Incoming>| {
                                let reqs = reqs.clone();
                                let script = script.clone();
                                let player_url = player_url.clone();
                                async move {
                                    let method = req.method().as_str().to_owned();
                                    let path = req.uri().path().to_owned();
                                    let query = req.uri().query().unwrap_or("").to_owned();
                                    let authorization = req
                                        .headers()
                                        .get("authorization")
                                        .and_then(|v| v.to_str().ok())
                                        .map(str::to_owned);
                                    let cookie = req
                                        .headers()
                                        .get("cookie")
                                        .and_then(|v| v.to_str().ok())
                                        .map(str::to_owned);
                                    let origin = req
                                        .headers()
                                        .get("origin")
                                        .and_then(|v| v.to_str().ok())
                                        .map(str::to_owned);
                                    let auth_user = req
                                        .headers()
                                        .get("x-goog-authuser")
                                        .and_then(|v| v.to_str().ok())
                                        .map(str::to_owned);

                                    let collected = req.into_body().collect().await.unwrap().to_bytes();
                                    let body_json = if collected.is_empty() {
                                        None
                                    } else {
                                        serde_json::from_slice::<Value>(&collected).ok()
                                    };

                                    reqs.lock().await.push(RecordedRequest {
                                        method,
                                        path: path.clone(),
                                        query,
                                        authorization,
                                        cookie,
                                        origin,
                                        auth_user,
                                        body_json,
                                    });

                                    let scripted_resp = script.lock().await.pop_front();
                                    let chosen = scripted_resp.unwrap_or_else(|| {
                                        if path.ends_with("/player") {
                                            ScriptedResponse::ok_json(json!({
                                                "playabilityStatus": { "status": "OK" },
                                                "streamingData": {
                                                    "expiresInSeconds": "21540",
                                                    "adaptiveFormats": [{
                                                        "itag": 251,
                                                        "mimeType": "audio/webm; codecs=\"opus\"",
                                                        "bitrate": 160000,
                                                        "url": player_url,
                                                    }]
                                                }
                                            }))
                                        } else {
                                            ScriptedResponse::ok_json(json!({ "status": "STATUS_SUCCEEDED" }))
                                        }
                                    });

                                    if chosen.delay_ms > 0 {
                                        tokio::time::sleep(Duration::from_millis(chosen.delay_ms)).await;
                                    }

                                    let mut builder = Response::builder()
                                        .status(StatusCode::from_u16(chosen.status).unwrap())
                                        .header("content-type", "application/json");
                                    if let Some(ra) = chosen.retry_after {
                                        builder = builder.header("retry-after", ra);
                                    }
                                    Ok::<_, hyper::Error>(
                                        builder
                                            .body(Full::new(Bytes::from(chosen.body)))
                                            .unwrap(),
                                    )
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
            base_url,
            requests,
            scripted,
            default_player_url,
            shutdown: Some(tx),
        }
    }

    async fn push_response(&self, resp: ScriptedResponse) {
        self.scripted.lock().await.push_back(resp);
    }

    async fn take_requests(&self) -> Vec<RecordedRequest> {
        std::mem::take(&mut *self.requests.lock().await)
    }
}

struct MockPoTokenProvider {
    token_value: String,
    mint_calls: AtomicUsize,
    last_request: std::sync::Mutex<Option<PoTokenRequest>>,
    delay_ms: u64,
    fail_with: std::sync::Mutex<Option<PoTokenError>>,
}

impl MockPoTokenProvider {
    fn new(token_value: &str) -> Self {
        Self {
            token_value: token_value.to_owned(),
            mint_calls: AtomicUsize::new(0),
            last_request: std::sync::Mutex::new(None),
            delay_ms: 0,
            fail_with: std::sync::Mutex::new(None),
        }
    }

    fn calls(&self) -> usize {
        self.mint_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl PoTokenProvider for MockPoTokenProvider {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        let n = self.mint_calls.fetch_add(1, Ordering::SeqCst) + 1;
        *self.last_request.lock().unwrap() = Some(req.clone());
        if let Some(err) = self.fail_with.lock().unwrap().clone() {
            return Err(err);
        }
        Ok(PoToken {
            value: if n == 1 {
                self.token_value.clone()
            } else {
                format!("{}_{}", self.token_value, n)
            },
            scope: req.scope,
            expires_at_unix_secs: resonance_core::unix_secs() + 3600,
        })
    }

    fn invalidate(&self, _scope: PoTokenScope) {}

    fn provider_name(&self) -> &'static str {
        "mock"
    }
}

fn make_test_client(
    base_url: &str,
) -> (
    tempfile::TempDir,
    Arc<SqliteCacheStore>,
    Arc<InnerTubeClient>,
) {
    let dir = tempdir().unwrap();
    let cache = Arc::new(SqliteCacheStore::open(&dir.path().join("test.db")).unwrap());
    let auth = Arc::new(FileAuthStore::new(dir.path()));
    let solver = Arc::new(QuickJsSolver::new(dir.path()));
    let client = Arc::new(InnerTubeClient::new(cache.clone(), auth, solver).unwrap());
    client.set_api_base_override(Some(base_url.to_owned()));
    (dir, cache, client)
}

fn test_session() -> AuthSession {
    AuthSession {
        cookie_header: "SAPISID=test_sapisid_secret; __Secure-3PAPISID=test_3p_secret; SID=abc"
            .to_owned(),
        auth_user: 0,
        page_id: None,
        visitor_data: Some("CgtTestVisitorData".to_owned()),
        account_name: Some("Test User".to_owned()),
        is_premium: false,
    }
}

fn matrix_with_only(kind: ClientKind, policy: PoTokenPolicy) -> ClientMatrix {
    let mut matrix = ClientMatrix::embedded_default().unwrap();
    for entry in &mut matrix.clients {
        if entry.client_kind() == Some(kind) {
            entry.enabled = true;
            entry.po_token_policy = policy;
            entry.priority_signed_in = 1;
            entry.priority_anonymous = 1;
            entry.probe_pass_cookies_off = true;
            entry.probe_pass_cookies_on = true;
        } else {
            entry.enabled = false;
        }
    }
    matrix
}

// ============================================================================
// OPTION A TESTS (SPEC_A_POTOKEN.md)
// ============================================================================

#[tokio::test]
async fn web_remix_required_policy_mints_and_attaches_pot_param_and_integrity_dim() {
    let server = MockInnerTubeServer::start().await;
    let (_dir, _cache, client) = make_test_client(&server.base_url);
    client.update_client_matrix(matrix_with_only(
        ClientKind::WebRemix,
        PoTokenPolicy::Required,
    ));

    let mock_pot = Arc::new(MockPoTokenProvider::new("MOCK_POT"));
    client.set_po_token_provider(Some(mock_pot.clone()));

    let info = client.resolve_stream("vid_req_001").await.unwrap();
    assert_eq!(mock_pot.calls(), 1);
    assert!(
        info.url.contains("pot=MOCK_POT"),
        "expected &pot=MOCK_POT in {}",
        info.url
    );
    assert!(info.url.contains("sig=RAW_SIG"));

    let reqs = server.take_requests().await;
    assert_eq!(reqs.len(), 1);
    let body = reqs[0].body_json.as_ref().unwrap();
    assert_eq!(
        body["serviceIntegrityDimensions"]["poToken"],
        json!("MOCK_POT")
    );
}

#[tokio::test]
async fn web_remix_free_bypassed_by_premium_skips_pot_for_premium_and_requires_for_free() {
    let server = MockInnerTubeServer::start().await;
    let (_dir, _cache, client) = make_test_client(&server.base_url);
    client.update_client_matrix(matrix_with_only(
        ClientKind::WebRemix,
        PoTokenPolicy::RequiredForWebFreeBypassedByPremium,
    ));
    let mock_pot = Arc::new(MockPoTokenProvider::new("FREE_POT"));
    client.set_po_token_provider(Some(mock_pot.clone()));

    // Anonymous / free -> requires PO token
    let free_info = client.resolve_stream("vid_free_01").await.unwrap();
    assert_eq!(mock_pot.calls(), 1);
    assert!(free_info.url.contains("pot=FREE_POT"));

    // Premium signed-in -> bypasses PO token
    client.sign_in(test_session()).unwrap();
    client.set_premium_account(true);
    let prem_info = client.resolve_stream("vid_prem_02").await.unwrap();
    assert_eq!(mock_pot.calls(), 1, "premium must not call mint_token");
    assert!(
        !prem_info.url.contains("pot="),
        "premium stream must omit &pot=: {}",
        prem_info.url
    );
}

#[tokio::test]
async fn android_vr_and_ios_never_receive_cookies_sapisidhash_or_pot() {
    let server = MockInnerTubeServer::start().await;
    let (_dir, _cache, client) = make_test_client(&server.base_url);
    client.sign_in(test_session()).unwrap();

    let mock_pot = Arc::new(MockPoTokenProvider::new("SHOULD_NOT_BE_USED"));
    client.set_po_token_provider(Some(mock_pot.clone()));

    // Force ANDROID_VR (even if matrix policy were misconfigured to Required, hard guard blocks it)
    client.update_client_matrix(matrix_with_only(
        ClientKind::AndroidVr,
        PoTokenPolicy::Required,
    ));
    let info = client.resolve_stream("vid_avr_01").await.unwrap();
    assert_eq!(mock_pot.calls(), 0);
    assert!(!info.url.contains("pot="));

    let reqs = server.take_requests().await;
    assert_eq!(reqs.len(), 1);
    assert!(
        reqs[0].cookie.is_none(),
        "ANDROID_VR must never send Cookie"
    );
    assert!(
        reqs[0].authorization.is_none(),
        "ANDROID_VR must never send Authorization"
    );
    assert!(
        reqs[0]
            .body_json
            .as_ref()
            .unwrap()
            .get("serviceIntegrityDimensions")
            .is_none()
    );

    // Also verify IOS hard guard via PoTokenPolicy + AuthMode::AnonymousOnly
    let ios_cfg = ClientConfig::for_kind(ClientKind::Ios);
    assert!(!ios_cfg.allows_browser_cookies());
    assert!(!PoTokenPolicy::RequiredIosGuard.requires_gvs_po_token(client.account_tier()));
}

#[tokio::test]
async fn not_required_and_optional_policies_never_invoke_provider_or_attach_pot() {
    let server = MockInnerTubeServer::start().await;
    let (_dir, _cache, client) = make_test_client(&server.base_url);
    let mock_pot = Arc::new(MockPoTokenProvider::new("UNUSED_POT"));
    client.set_po_token_provider(Some(mock_pot.clone()));

    client.update_client_matrix(matrix_with_only(
        ClientKind::VisionOs,
        PoTokenPolicy::Optional,
    ));
    let v_info = client.resolve_stream("vid_vis_01").await.unwrap();
    assert_eq!(mock_pot.calls(), 0);
    assert_eq!(v_info.url, server.default_player_url);

    client.update_client_matrix(matrix_with_only(
        ClientKind::AndroidVr,
        PoTokenPolicy::NotRequired,
    ));
    let a_info = client.resolve_stream("vid_avr_02").await.unwrap();
    assert_eq!(mock_pot.calls(), 0);
    assert_eq!(a_info.url, server.default_player_url);
}

#[tokio::test]
async fn caching_potoken_provider_deduplicates_concurrent_mints_and_invalidates_on_403() {
    let inner = MockPoTokenProvider {
        token_value: "CACHED_POT".to_owned(),
        mint_calls: AtomicUsize::new(0),
        last_request: std::sync::Mutex::new(None),
        delay_ms: 40,
        fail_with: std::sync::Mutex::new(None),
    };
    let cached = Arc::new(CachingPoTokenProvider::new(inner));
    let req = PoTokenRequest {
        client_name: "WEB_REMIX".to_owned(),
        scope: PoTokenScope::Gvs,
        video_id: "vid_1".to_owned(),
        visitor_data: Some("vd_1".to_owned()),
        session_hash: None,
        signed_in: false,
    };

    let mut handles = Vec::new();
    for _ in 0..8 {
        let p = cached.clone();
        let r = req.clone();
        handles.push(tokio::spawn(async move { p.mint_token(&r).await.unwrap() }));
    }
    for h in handles {
        let tok = h.await.unwrap();
        assert_eq!(tok.value, "CACHED_POT");
    }

    // Verify invalidation forces re-mint
    cached.invalidate(PoTokenScope::Gvs);
    let second = cached.mint_token(&req).await.unwrap();
    assert_eq!(second.value, "CACHED_POT_2");
}

#[tokio::test]
async fn sidecar_provider_rejects_non_loopback_urls_and_handles_timeout() {
    let evil = SidecarPoTokenProvider::new("https://evil.example/get_pot");
    assert!(matches!(evil, Err(PoTokenError::Unavailable(_))));

    // Stalled local sidecar server -> times out cleanly and falls back to ANDROID_VR
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stalled_addr = listener.local_addr().unwrap();
    let (_stop_tx, mut stop_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                Ok((stream, _)) = listener.accept() => {
                    tokio::spawn(async move {
                        let _keep_open = stream;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    });
                }
            }
        }
    });

    let sidecar = SidecarPoTokenProvider::new(format!("http://{stalled_addr}/get_pot"))
        .unwrap()
        .with_timeout_ms(150)
        .unwrap();

    let server = MockInnerTubeServer::start().await;
    let (_dir, _cache, client) = make_test_client(&server.base_url);
    client.set_po_token_provider(Some(Arc::new(sidecar)));

    // Enable WEB_REMIX (Required) at prio 1 and ANDROID_VR (NotRequired) at prio 2
    let mut matrix = ClientMatrix::embedded_default().unwrap();
    for entry in &mut matrix.clients {
        match entry.client_kind() {
            Some(ClientKind::WebRemix) => {
                entry.enabled = true;
                entry.po_token_policy = PoTokenPolicy::Required;
                entry.priority_anonymous = 1;
            }
            Some(ClientKind::AndroidVr) => {
                entry.enabled = true;
                entry.po_token_policy = PoTokenPolicy::NotRequired;
                entry.priority_anonymous = 2;
                entry.probe_pass_cookies_off = true;
            }
            _ => entry.enabled = false,
        }
    }
    client.update_client_matrix(matrix);

    let fallback_info = client.resolve_stream("vid_fallback_01").await.unwrap();
    assert_eq!(fallback_info.resolved_by_client, "ANDROID_VR");
    assert!(!fallback_info.url.contains("pot="));
}

#[test]
fn potoken_debug_and_redact_secrets_never_leak_token_value() {
    let token = PoToken {
        value: "SUPER_SECRET_POT_XYZ".to_owned(),
        scope: PoTokenScope::Gvs,
        expires_at_unix_secs: 1_900_000_000,
    };
    let dbg = format!("{token:?}");
    assert!(!dbg.contains("SUPER_SECRET_POT_XYZ"));
    assert!(dbg.contains("[REDACTED_POT]"));

    let log_lines = "Stream: https://127.0.0.1:8080/play?pot=SUPER_SECRET_POT_XYZ&n=1\n\
                     Body: {\"poToken\":\"SUPER_SECRET_POT_XYZ\"}\n\
                     Safe: ok";
    let redacted = redact_secrets(log_lines);
    assert!(!redacted.contains("SUPER_SECRET_POT_XYZ"));
    assert!(redacted.contains("Safe: ok"));
}

// ============================================================================
// OPTION D TESTS (SPEC_D_MUTATIONS.md)
// ============================================================================

#[tokio::test]
async fn mutations_reject_anonymous_session_without_making_network_calls() {
    let server = MockInnerTubeServer::start().await;
    let (_dir, _cache, client) = make_test_client(&server.base_url);

    let r1 = client.set_like("vid_123", LikeAction::Like).await;
    let r2 = client
        .edit_playlist(
            "PL_123",
            &PlaylistEditOp::AddVideo {
                video_id: "vid_123".into(),
            },
        )
        .await;
    let r3 = client
        .report_atr(&WatchtimeReport {
            video_id: "vid_123".into(),
            cpn: "1234567890abcdef".into(),
            start_secs: 0,
            end_secs: 30,
            duration_secs: 180,
            is_final: false,
        })
        .await;

    assert!(matches!(r1, Err(MutationError::NotAuthenticated)));
    assert!(matches!(r2, Err(MutationError::NotAuthenticated)));
    assert!(matches!(r3, Err(MutationError::NotAuthenticated)));
    assert!(server.take_requests().await.is_empty());
}

#[tokio::test]
async fn set_like_signs_sapisidhash_hits_exact_endpoints_and_evicts_liked_cache() {
    let server = MockInnerTubeServer::start().await;
    let (_dir, cache, client) = make_test_client(&server.base_url);
    client.sign_in(test_session()).unwrap();

    cache
        .put_metadata(
            "collection:FEmusic_liked_videos",
            "{}",
            3600,
            resonance_core::unix_secs(),
        )
        .unwrap();

    client
        .set_like("vid_like_1", LikeAction::Like)
        .await
        .unwrap();
    assert!(
        cache
            .get_metadata(
                "collection:FEmusic_liked_videos",
                resonance_core::unix_secs()
            )
            .unwrap()
            .is_none(),
        "liked cache must be evicted"
    );

    client
        .set_like("vid_like_1", LikeAction::Dislike)
        .await
        .unwrap();
    client
        .set_like("vid_like_1", LikeAction::RemoveLike)
        .await
        .unwrap();

    let reqs = server.take_requests().await;
    assert_eq!(reqs.len(), 3);
    let paths: Vec<&str> = reqs.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["/like/like", "/like/dislike", "/like/removelike"]
    );
    for req in &reqs {
        assert!(
            req.authorization
                .as_deref()
                .is_some_and(|a| a.contains("SAPISIDHASH ")),
            "missing SAPISIDHASH"
        );
        assert!(
            req.cookie
                .as_deref()
                .is_some_and(|c| c.contains("SAPISID="))
        );
        assert_eq!(req.origin.as_deref(), Some("https://music.youtube.com"));
        assert_eq!(req.auth_user.as_deref(), Some("0"));
        assert_eq!(
            req.body_json.as_ref().unwrap()["target"]["videoId"],
            json!("vid_like_1")
        );
    }
}

#[tokio::test]
async fn edit_playlist_add_remove_move_payloads_and_cache_invalidation() {
    let server = MockInnerTubeServer::start().await;
    let (_dir, cache, client) = make_test_client(&server.base_url);
    client.sign_in(test_session()).unwrap();

    cache
        .put_metadata("collection:PL_ABC", "{}", 3600, resonance_core::unix_secs())
        .unwrap();

    client
        .edit_playlist(
            "PL_ABC",
            &PlaylistEditOp::AddVideo {
                video_id: "vid_1".into(),
            },
        )
        .await
        .unwrap();
    assert!(
        cache
            .get_metadata("collection:PL_ABC", resonance_core::unix_secs())
            .unwrap()
            .is_none()
    );

    client
        .edit_playlist(
            "PL_ABC",
            &PlaylistEditOp::RemoveVideo {
                video_id: "vid_1".into(),
                set_video_id: "SET_1".into(),
            },
        )
        .await
        .unwrap();

    client
        .edit_playlist(
            "PL_ABC",
            &PlaylistEditOp::MoveVideoBefore {
                set_video_id: "SET_2".into(),
                predecessor_set_video_id: Some("SET_1".into()),
            },
        )
        .await
        .unwrap();

    let reqs = server.take_requests().await;
    assert_eq!(reqs.len(), 3);
    assert_eq!(
        reqs[0].body_json.as_ref().unwrap()["actions"][0],
        json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": "vid_1" })
    );
    assert_eq!(
        reqs[1].body_json.as_ref().unwrap()["actions"][0],
        json!({ "action": "ACTION_REMOVE_VIDEO", "removedVideoId": "vid_1", "setVideoId": "SET_1" })
    );
    assert_eq!(
        reqs[2].body_json.as_ref().unwrap()["actions"][0],
        json!({
            "action": "ACTION_MOVE_VIDEO_BEFORE",
            "setVideoId": "SET_2",
            "movedSetVideoIdPredecessor": "SET_1"
        })
    );
}

#[tokio::test]
async fn report_atr_sends_signed_watchtime_params_and_rejects_invalid_ranges() {
    let server = MockInnerTubeServer::start().await;
    let (_dir, _cache, client) = make_test_client(&server.base_url);
    client.sign_in(test_session()).unwrap();

    let bad = client
        .report_atr(&WatchtimeReport {
            video_id: "vid_atr".into(),
            cpn: "1234567890abcdef".into(),
            start_secs: 50,
            end_secs: 10,
            duration_secs: 180,
            is_final: false,
        })
        .await;
    assert!(matches!(bad, Err(MutationError::InvalidInput(_))));

    client
        .report_atr(&WatchtimeReport {
            video_id: "vid_atr".into(),
            cpn: "1234567890abcdef".into(),
            start_secs: 10,
            end_secs: 45,
            duration_secs: 180,
            is_final: true,
        })
        .await
        .unwrap();

    let reqs = server.take_requests().await;
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].path, "/atr");
    assert!(reqs[0].query.contains("docid=vid_atr"));
    assert!(reqs[0].query.contains("cpn=1234567890abcdef"));
    assert!(reqs[0].query.contains("st=10"));
    assert!(reqs[0].query.contains("et=45"));
    assert!(reqs[0].query.contains("len=180"));
    assert!(reqs[0].query.contains("c=WEB_REMIX"));
    assert!(
        reqs[0]
            .authorization
            .as_deref()
            .is_some_and(|a| a.contains("SAPISIDHASH "))
    );
}

#[tokio::test]
async fn mutation_worker_preserves_fifo_order_and_coalesces_atr_pings() {
    let server = MockInnerTubeServer::start().await;
    let (_dir, _cache, client) = make_test_client(&server.base_url);
    client.sign_in(test_session()).unwrap();

    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();
    let worker = MutationWorker::spawn(client, bus);

    worker
        .enqueue(MutationCommand::SetLike {
            video_id: "vid_fifo".into(),
            action: LikeAction::Like,
        })
        .unwrap();
    worker
        .enqueue(MutationCommand::SetLike {
            video_id: "vid_fifo".into(),
            action: LikeAction::RemoveLike,
        })
        .unwrap();
    worker
        .enqueue(MutationCommand::SetLike {
            video_id: "vid_fifo".into(),
            action: LikeAction::Like,
        })
        .unwrap();

    for end in [5_u64, 10, 15, 20, 25] {
        worker
            .enqueue(MutationCommand::ReportAtr(WatchtimeReport {
                video_id: "vid_fifo".into(),
                cpn: "abcdef1234567890".into(),
                start_secs: 0,
                end_secs: end,
                duration_secs: 200,
                is_final: end == 25,
            }))
            .unwrap();
    }

    let mut completed = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while completed < 4 && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(AppEvent::MutationCompleted(_))) =
            tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
        {
            completed += 1;
        }
    }
    assert!(completed >= 4, "expected at least 4 completed mutations");

    let reqs = server.take_requests().await;
    let like_paths: Vec<&str> = reqs
        .iter()
        .filter(|r| r.path.starts_with("/like/"))
        .map(|r| r.path.as_str())
        .collect();
    assert_eq!(
        like_paths,
        vec!["/like/like", "/like/removelike", "/like/like"]
    );

    let atr_reqs: Vec<&RecordedRequest> = reqs.iter().filter(|r| r.path == "/atr").collect();
    assert!(
        atr_reqs.len() <= 2,
        "expected ATR pings to coalesce, got {}",
        atr_reqs.len()
    );
    assert!(
        atr_reqs.last().unwrap().query.contains("et=25"),
        "coalesced ATR must keep latest end_secs"
    );
}

#[tokio::test]
async fn mutation_401_revokes_session_and_drains_pending_queue() {
    let server = MockInnerTubeServer::start().await;
    server
        .push_response(ScriptedResponse::status_body(401, "unauthorized"))
        .await;

    let (_dir, _cache, client) = make_test_client(&server.base_url);
    client.sign_in(test_session()).unwrap();

    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();
    let worker = MutationWorker::spawn(client.clone(), bus);

    worker
        .enqueue(MutationCommand::SetLike {
            video_id: "v1".into(),
            action: LikeAction::Like,
        })
        .unwrap();
    worker
        .enqueue(MutationCommand::SetLike {
            video_id: "v2".into(),
            action: LikeAction::Like,
        })
        .unwrap();

    let mut saw_failed = false;
    let mut saw_signed_out = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while (!saw_failed || !saw_signed_out) && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(ev)) = tokio::time::timeout(Duration::from_millis(400), rx.recv()).await {
            match ev {
                AppEvent::MutationFailed { .. } => saw_failed = true,
                AppEvent::AuthStateChanged {
                    signed_in: false, ..
                } => saw_signed_out = true,
                _ => {}
            }
        }
    }

    assert!(saw_failed, "expected MutationFailed on 401");
    assert!(
        saw_signed_out,
        "expected AuthStateChanged(signed_in: false)"
    );
    assert!(
        client.current_session().is_none(),
        "session must be cleared"
    );
    let reqs = server.take_requests().await;
    assert_eq!(reqs.len(), 1, "subsequent queued mutations must be drained");
}

#[tokio::test]
async fn mutation_429_backoff_and_secret_redaction() {
    let server = MockInnerTubeServer::start().await;
    server
        .push_response(ScriptedResponse {
            status: 429,
            body: "rate limited".to_owned(),
            retry_after: Some("0".to_owned()),
            delay_ms: 0,
        })
        .await;
    server
        .push_response(ScriptedResponse::ok_json(json!({ "status": "OK" })))
        .await;

    let (_dir, _cache, client) = make_test_client(&server.base_url);
    client.sign_in(test_session()).unwrap();

    let bus = EventBus::new(64);
    let mut rx = bus.subscribe();
    let worker = MutationWorker::spawn(client.clone(), bus);

    worker
        .enqueue(MutationCommand::SetLike {
            video_id: "vid_429".into(),
            action: LikeAction::Like,
        })
        .unwrap();

    let mut completed_429 = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !completed_429 && tokio::time::Instant::now() < deadline {
        if let Ok(Ok(AppEvent::MutationCompleted(_))) =
            tokio::time::timeout(Duration::from_millis(500), rx.recv()).await
        {
            completed_429 = true;
        }
    }
    assert!(completed_429, "expected 429 retry to succeed");

    // Now test 500 containing SAPISID=LEAK -> must be redacted
    server
        .push_response(ScriptedResponse::status_body(
            500,
            "SAPISID=LEAK_SECRET_VALUE",
        ))
        .await;
    let err = client
        .set_like("vid_500", LikeAction::Like)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        !msg.contains("LEAK_SECRET_VALUE"),
        "error string leaked secret: {msg}"
    );
}
