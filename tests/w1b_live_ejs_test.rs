//! Offline verification tests for W1B deltas:
//! - QuickJS sandbox memory-limit recovery
//! - PlayerScriptFetcher SSRF host allowlist & HTTPS enforcement
//! - RemoteConfigSyncService 304 Not Modified ETag preservation & strict Ed25519

use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use resonance_contract::AppPaths;
use resonance_core::cache::SqliteCacheStore;
use resonance_core::innertube::ClientMatrix;
use resonance_core::innertube::remote_config::RemoteConfigSyncService;
use resonance_core::innertube::remote_config::ed25519_dalek::{Signer, SigningKey};
use resonance_core::solver::{ChallengeInput, PlayerScriptFetcher, QuickJsSolver};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

type RouteEntry = (StatusCode, Option<String>, Vec<u8>);
type RouteMap = Arc<Mutex<HashMap<String, RouteEntry>>>;

async fn spawn_mock_server(routes: RouteMap) -> (SocketAddr, oneshot::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, mut rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut rx => break,
                Ok((stream, _)) = listener.accept() => {
                    let io = TokioIo::new(stream);
                    let routes = routes.clone();
                    tokio::spawn(async move {
                        let svc = service_fn(move |req: Request<Incoming>| {
                            let routes = routes.clone();
                            async move {
                                let path = req.uri().path().to_owned();
                                let if_none_match = req
                                    .headers()
                                    .get("if-none-match")
                                    .and_then(|v| v.to_str().ok())
                                    .map(str::to_owned);
                                let entry = routes.lock().unwrap().get(&path).cloned();
                                let (status, etag, body) = entry.unwrap_or((
                                    StatusCode::NOT_FOUND,
                                    None,
                                    b"not found".to_vec(),
                                ));
                                if let (Some(req_etag), Some(srv_etag)) = (if_none_match.as_deref(), etag.as_deref())
                                    && req_etag == srv_etag
                                {
                                    return Ok::<_, hyper::Error>(
                                        Response::builder()
                                            .status(StatusCode::NOT_MODIFIED)
                                            .body(Full::new(Bytes::new()))
                                            .unwrap(),
                                    );
                                }
                                let mut builder = Response::builder().status(status);
                                if let Some(e) = etag {
                                    builder = builder.header("ETag", e);
                                }
                                Ok::<_, hyper::Error>(
                                    builder.body(Full::new(Bytes::from(body))).unwrap(),
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

    (addr, tx)
}

fn test_app_paths(root: &TempDir) -> AppPaths {
    let data_dir = root.path().join("data");
    let cache_dir = root.path().join("cache");
    let config_dir = root.path().join("config");
    let downloads_dir = data_dir.join("downloads");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(&downloads_dir).unwrap();
    AppPaths {
        data_dir,
        cache_dir,
        config_dir,
        downloads_dir,
    }
}

#[tokio::test]
async fn quickjs_sandbox_enforces_memory_cap_and_recovers_cleanly() {
    let tmp = TempDir::new().unwrap();
    let solver = QuickJsSolver::new(tmp.path());

    // 1. Memory-bomb payload exceeds 64 MB heap cap and fails cleanly.
    let oom_js = r#"
        function solveN(n) {
            var arr = [];
            for (var i = 0; i < 10000000; i++) {
                arr.push(new Array(1024).join("x"));
            }
            return n;
        }
        function solveSig(s) { return s; }
    "#;
    let oom_res = solver
        .solve(ChallengeInput {
            player_hash: "oomhash1".to_owned(),
            solver_js: oom_js.to_owned(),
            n_param: Some("abc".to_owned()),
            sig_param: None,
        })
        .await;
    assert!(oom_res.is_err(), "memory bomb should fail under 64MB cap");

    // 2. Subsequent valid solve recreates the QuickJS context and succeeds.
    let good_js = r#"
        function solveN(n) { return "ok_" + n; }
        function solveSig(s) { return "sig_" + s; }
    "#;
    let ok_res = solver
        .solve(ChallengeInput {
            player_hash: "goodhash".to_owned(),
            solver_js: good_js.to_owned(),
            n_param: Some("xyz".to_owned()),
            sig_param: Some("123".to_owned()),
        })
        .await
        .unwrap();
    assert_eq!(ok_res.n_param.as_deref(), Some("ok_xyz"));
    assert_eq!(ok_res.sig_param.as_deref(), Some("sig_123"));
}

#[tokio::test]
async fn player_script_fetcher_rejects_ssrf_hosts_and_non_https() {
    let tmp = TempDir::new().unwrap();
    let fetcher =
        PlayerScriptFetcher::new(tmp.path().join("js_solver")).with_allow_localhost(false);

    let bad_host = fetcher
        .fetch_player_script(
            "abcd1234",
            "https://evil.example.com/s/player/abcd1234/base.js",
        )
        .await;
    assert!(bad_host.is_err());

    let bad_scheme = fetcher
        .fetch_player_script(
            "abcd1234",
            "http://music.youtube.com/s/player/abcd1234/base.js",
        )
        .await;
    assert!(bad_scheme.is_err());
}

#[tokio::test]
async fn remote_config_304_not_modified_preserves_cached_matrix() {
    let tmp = TempDir::new().unwrap();
    let paths = test_app_paths(&tmp);
    let cache = Arc::new(SqliteCacheStore::open(&paths.data_dir).unwrap());
    let solver = Arc::new(QuickJsSolver::new(&paths.cache_dir));

    let signing_key = SigningKey::from_bytes(&[11u8; 32]);
    let verifying_key = signing_key.verifying_key();

    let mut matrix = ClientMatrix::embedded_default().unwrap();
    matrix.updated_at_unix_secs += 500;
    matrix.source = "etag-verified".to_owned();
    let matrix_bytes = serde_json::to_vec_pretty(&matrix).unwrap();
    let sig_hex = hex::encode(signing_key.sign(&matrix_bytes).to_bytes());

    let routes: RouteMap = Arc::new(Mutex::new(HashMap::new()));
    routes.lock().unwrap().insert(
        "/client_matrix.json".to_owned(),
        (StatusCode::OK, Some("\"v1\"".to_owned()), matrix_bytes),
    );
    routes.lock().unwrap().insert(
        "/client_matrix.json.sig".to_owned(),
        (StatusCode::OK, None, sig_hex.into_bytes()),
    );

    let (addr, shutdown) = spawn_mock_server(routes).await;
    let svc = RemoteConfigSyncService::with_verifying_key(
        paths.cache_dir.join("remote_config"),
        cache,
        solver,
        Some(format!("http://{addr}")),
        verifying_key,
    )
    .unwrap()
    .with_allow_insecure_localhost(true);

    let first = svc.sync_client_matrix(true).await.unwrap();
    assert!(first.is_some());

    // Second forced sync sends If-None-Match: "v1" -> 304 Not Modified -> Ok(None)
    let second = svc.sync_client_matrix(true).await.unwrap();
    assert!(second.is_none());
    assert_eq!(
        svc.load_cached_matrix_sync().unwrap().source,
        "etag-verified"
    );

    let _ = shutdown.send(());
}
