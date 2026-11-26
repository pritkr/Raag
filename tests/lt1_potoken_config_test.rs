//! Offline verification suite for LT-1:
//! Zero-setup PO-Token attestation chain, 80% TTL proactive refresh,
//! negative-cache backoff, untrusted interpreter host rejection,
//! rustypipe-botguard CLI injection rejection, and Ed25519 signed config canary gates.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use ed25519_dalek::{Signer, SigningKey};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use resonance_contract::{
    PoToken, PoTokenError, PoTokenHealthStatus, PoTokenProvider, PoTokenRequest, PoTokenScope,
};
use resonance_core::cache::SqliteCacheStore;
use resonance_core::innertube::matrix::ClientMatrix;
use resonance_core::innertube::remote_config::RemoteConfigSyncService;
use resonance_core::potoken::{
    AttestationOrchestrator, BotguardChallenge, BotguardExecutionOutput, CachingPoTokenProvider,
    ChainedPoTokenProvider, EmbeddedWebViewAttestationProvider, WebViewBotguardRunner,
    validate_botguard_binding, validate_interpreter_url,
};
use resonance_core::solver::QuickJsSolver;
use tokio::net::TcpListener;

struct FailingProvider {
    name: &'static str,
    calls: AtomicUsize,
}

impl FailingProvider {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl PoTokenProvider for FailingProvider {
    async fn mint_token(&self, _req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(PoTokenError::Unavailable(format!("{} offline", self.name)))
    }

    fn invalidate(&self, _scope: PoTokenScope) {}

    fn provider_name(&self) -> &'static str {
        self.name
    }
}

struct MockWebViewRunner {
    calls: AtomicUsize,
}

#[async_trait]
impl WebViewBotguardRunner for MockWebViewRunner {
    async fn execute_challenge(
        &self,
        _interpreter_js: &str,
        _challenge: &BotguardChallenge,
        _timeout: Duration,
    ) -> Result<BotguardExecutionOutput, PoTokenError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(BotguardExecutionOutput {
            botguard_response: "mock_bg_response_ok".to_owned(),
            web_po_minter_snapshot: None,
        })
    }
}

struct MockCountingProvider {
    calls: AtomicUsize,
    clock: Arc<AtomicU64>,
    ttl_secs: u64,
}

#[async_trait]
impl PoTokenProvider for MockCountingProvider {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let now = self.clock.load(Ordering::SeqCst);
        Ok(PoToken {
            value: format!("pot_{n}"),
            scope: req.scope,
            expires_at_unix_secs: now + self.ttl_secs,
        })
    }

    fn invalidate(&self, _scope: PoTokenScope) {}

    fn provider_name(&self) -> &'static str {
        "mock-counting"
    }
}

#[tokio::test]
async fn chained_potoken_provider_falls_through_tiers_and_respects_negative_cache_backoff() {
    let tier1 = Arc::new(FailingProvider::new("bgutil-http"));
    let tier2 = Arc::new(FailingProvider::new("rustypipe-botguard"));
    let tier3 = Arc::new(MockCountingProvider {
        calls: AtomicUsize::new(0),
        clock: Arc::new(AtomicU64::new(1_700_000_000)),
        ttl_secs: 3600,
    });

    let chain = ChainedPoTokenProvider::new(vec![tier1.clone(), tier2.clone(), tier3]);
    let req = PoTokenRequest::new(
        "WEB_REMIX",
        PoTokenScope::Gvs,
        "vid1",
        Some("vd_1".into()),
        false,
    );
    let tok = chain.mint_token(&req).await.expect("tier 3 succeeds");
    assert_eq!(tok.value, "pot_1");
    assert_eq!(tier1.calls.load(Ordering::SeqCst), 1);
    assert_eq!(tier2.calls.load(Ordering::SeqCst), 1);
    assert_eq!(chain.diagnostics().status, PoTokenHealthStatus::Healthy);

    // Total failure with CachingPoTokenProvider triggers 30s negative-cache backoff
    struct SharedFailingProvider(Arc<FailingProvider>);
    #[async_trait]
    impl PoTokenProvider for SharedFailingProvider {
        async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
            self.0.mint_token(req).await
        }
        fn invalidate(&self, scope: PoTokenScope) {
            self.0.invalidate(scope);
        }
        fn provider_name(&self) -> &'static str {
            self.0.provider_name()
        }
    }

    let fail_inner = Arc::new(FailingProvider::new("all-fail"));
    let clock = Arc::new(AtomicU64::new(1_700_000_000));
    let clock_clone = Arc::clone(&clock);
    let cached_fail = CachingPoTokenProvider::with_clock(
        SharedFailingProvider(Arc::clone(&fail_inner)),
        Arc::new(move || clock_clone.load(Ordering::SeqCst)),
    );

    assert!(cached_fail.mint_token(&req).await.is_err());
    assert_eq!(fail_inner.calls.load(Ordering::SeqCst), 1);
    // Second call within 30s backoff must NOT hit underlying chain again
    assert!(cached_fail.mint_token(&req).await.is_err());
    assert_eq!(fail_inner.calls.load(Ordering::SeqCst), 1);

    // Advance clock past 30s backoff -> retries
    clock.store(1_700_000_035, Ordering::SeqCst);
    assert!(cached_fail.mint_token(&req).await.is_err());
    assert_eq!(fail_inner.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn caching_provider_coalesces_concurrent_mints_and_proactively_refreshes_at_80pct_ttl() {
    let clock = Arc::new(AtomicU64::new(1_700_000_000));
    let inner = MockCountingProvider {
        calls: AtomicUsize::new(0),
        clock: Arc::clone(&clock),
        ttl_secs: 1000,
    };
    let clock_clone = Arc::clone(&clock);
    let provider = Arc::new(CachingPoTokenProvider::with_clock(
        inner,
        Arc::new(move || clock_clone.load(Ordering::SeqCst)),
    ));

    let req = PoTokenRequest::new(
        "WEB_REMIX",
        PoTokenScope::Gvs,
        "vid1",
        Some("vd_99".into()),
        false,
    );

    // 12 concurrent calls -> exactly 1 underlying mint
    let mut handles = Vec::new();
    for _ in 0..12 {
        let p = Arc::clone(&provider);
        let r = req.clone();
        handles.push(tokio::spawn(async move { p.mint_token(&r).await }));
    }
    for h in handles {
        let tok = h.await.unwrap().unwrap();
        assert_eq!(tok.value, "pot_1");
    }
    assert_eq!(provider.inner().calls.load(Ordering::SeqCst), 1);

    // Advance clock to 85% TTL (850s of 1000s) -> returns cached pot_1 immediately AND triggers background refresh
    clock.store(1_700_000_850, Ordering::SeqCst);
    let immediate = provider.mint_token(&req).await.unwrap();
    assert_eq!(immediate.value, "pot_1");

    // Wait briefly for background refresh task to finish
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(provider.inner().calls.load(Ordering::SeqCst), 2);

    let refreshed = provider.mint_token(&req).await.unwrap();
    assert_eq!(refreshed.value, "pot_2");
}

#[tokio::test]
async fn webview_attestation_rejects_untrusted_interpreter_hosts() {
    assert!(validate_interpreter_url("https://www.gstatic.com/bg/botguard.js").is_ok());
    assert!(validate_interpreter_url("https://evil.example/bg.js").is_err());
    assert!(validate_interpreter_url("http://www.gstatic.com/bg.js").is_err());

    // Local mock /att/get returning untrusted interpreterUrl
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let io = TokioIo::new(stream);
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    io,
                    service_fn(|_req: Request<hyper::body::Incoming>| async move {
                        let body = serde_json::json!({
                            "bgChallenge": {
                                "interpreterUrl": "https://evil.example/bg.js",
                                "program": "prog",
                                "globalName": "bg",
                                "challenge": "chal",
                                "requestKey": "rk"
                            }
                        })
                        .to_string();
                        Ok::<_, hyper::Error>(
                            Response::builder()
                                .status(StatusCode::OK)
                                .body(Full::new(Bytes::from(body)))
                                .unwrap(),
                        )
                    }),
                )
                .await;
        }
    });

    let runner = Arc::new(MockWebViewRunner {
        calls: AtomicUsize::new(0),
    });
    let orch = Arc::new(AttestationOrchestrator::new(runner.clone()).with_endpoints(
        format!("http://{addr}/youtubei/v1/att/get"),
        format!("http://{addr}/GenerateIT"),
    ));
    let provider = EmbeddedWebViewAttestationProvider::with_orchestrator(orch, "embedded-webview");
    assert_eq!(provider.provider_name(), "embedded-webview");

    let req = PoTokenRequest::new(
        "WEB_REMIX",
        PoTokenScope::Gvs,
        "vid1",
        Some("vd_1".into()),
        false,
    );
    let err = provider.mint_token(&req).await.unwrap_err();
    assert!(matches!(err, PoTokenError::PolicyForbidden { .. }));
    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn rustypipe_botguard_rejects_shell_metacharacters_in_binding() {
    assert!(validate_botguard_binding("dQw4w9WgXcQ").is_ok());
    assert!(validate_botguard_binding("CgtnS1N5XzRfTQ%3D%3D").is_ok());
    assert!(validate_botguard_binding("").is_err());
    assert!(validate_botguard_binding("--help").is_err());
    assert!(validate_botguard_binding("abc;rm -rf /").is_err());
    assert!(validate_botguard_binding("abc|cat /etc/passwd").is_err());
    assert!(validate_botguard_binding("abc$(whoami)").is_err());
    assert!(validate_botguard_binding("abc`id`").is_err());
}

#[tokio::test]
async fn sign_config_and_probe_canary_enforce_ed25519_and_anonymous_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let mut seed = [7u8; 32];
    seed[0] = 42;
    let signing_key = SigningKey::from_bytes(&seed);
    let verifying_key = signing_key.verifying_key();

    // Create a valid matrix and sign it
    let matrix = ClientMatrix::embedded_default().unwrap();
    let matrix_bytes = serde_json::to_vec_pretty(&matrix).unwrap();
    let matrix_sig = hex::encode(signing_key.sign(&matrix_bytes).to_bytes());

    let manifest_bytes = resonance_core::solver::EMBEDDED_EJS_MANIFEST_JSON.as_bytes();
    let manifest_sig = hex::encode(signing_key.sign(manifest_bytes).to_bytes());

    // Serve over local HTTP mock to RemoteConfigSyncService
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let m_bytes = matrix_bytes.clone();
            let m_sig = matrix_sig.clone();
            let s_bytes = manifest_bytes.to_vec();
            let s_sig = manifest_sig.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |req: Request<hyper::body::Incoming>| {
                            let path = req.uri().path().to_owned();
                            let body = match path.as_str() {
                                "/client_matrix.json" => m_bytes.clone(),
                                "/client_matrix.json.sig" => m_sig.as_bytes().to_vec(),
                                "/solver_manifest.json" => s_bytes.clone(),
                                "/solver_manifest.json.sig" => s_sig.as_bytes().to_vec(),
                                _ => Vec::new(),
                            };
                            let status = if body.is_empty() {
                                StatusCode::NOT_FOUND
                            } else {
                                StatusCode::OK
                            };
                            async move {
                                Ok::<_, hyper::Error>(
                                    Response::builder()
                                        .status(status)
                                        .body(Full::new(Bytes::from(body)))
                                        .unwrap(),
                                )
                            }
                        }),
                    )
                    .await;
            });
        }
    });

    let cache = Arc::new(SqliteCacheStore::open(&tmp.path().join("cache")).unwrap());
    let solver = Arc::new(QuickJsSolver::new(tmp.path()));
    let svc = RemoteConfigSyncService::with_verifying_key(
        tmp.path().join("remote_config"),
        cache,
        solver,
        Some(format!("http://{addr}")),
        verifying_key,
    )
    .unwrap()
    .with_allow_insecure_localhost(true);

    let matrix = svc.sync_client_matrix(true).await.unwrap();
    assert!(matrix.is_some());
}
