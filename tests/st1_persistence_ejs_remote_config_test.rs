use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use resonance_contract::{AppPaths, AuthSession};
use resonance_core::cache::SqliteCacheStore;
use resonance_core::innertube::remote_config::ed25519_dalek::{Signer, SigningKey};
use resonance_core::innertube::{
    AccountProfileProbe, AccountTier, ClientMatrix, FileAuthStore, InnerTubeClient,
    RemoteConfigSyncService,
};
use resonance_core::solver::{EjsBundleManifest, PlayerScriptFetcher, QuickJsSolver};
use resonance_core::storage::bootstrap_st1_services;
use serde_json::json;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

type RouteMap = Arc<Mutex<HashMap<String, (StatusCode, Vec<u8>)>>>;

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
                                let entry = routes.lock().unwrap().get(&path).cloned();
                                let (status, body) = entry.unwrap_or((StatusCode::NOT_FOUND, b"not found".to_vec()));
                                Ok::<_, hyper::Error>(
                                    Response::builder()
                                        .status(status)
                                        .body(Full::new(Bytes::from(body)))
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

#[test]
fn bootstrap_st1_places_ejs_and_remote_config_under_app_paths_cache_dir() {
    let tmp = TempDir::new().unwrap();
    let paths = test_app_paths(&tmp);
    let cache = Arc::new(SqliteCacheStore::open(&paths.data_dir).unwrap());

    let services = bootstrap_st1_services(&paths, cache, None).unwrap();

    assert!(paths.cache_dir.join("js_solver").is_dir());
    assert!(paths.cache_dir.join("ejs").is_dir());
    assert!(paths.cache_dir.join("remote_config").is_dir());

    // Never pollute data_dir with cache-only ST-1 directories.
    assert!(!paths.data_dir.join("js_solver").exists());
    assert!(!paths.data_dir.join("ejs").exists());
    assert!(!paths.data_dir.join("remote_config").exists());

    assert_eq!(services.solver.runtime_name(), "quickjs-ng");
}

#[tokio::test]
async fn ejs_jsc_protocol_preprocesses_player_once_and_memoizes_n_and_sig() {
    let tmp = TempDir::new().unwrap();
    let paths = test_app_paths(&tmp);
    let cache = Arc::new(SqliteCacheStore::open(&paths.data_dir).unwrap());
    let solver = QuickJsSolver::new(&paths.cache_dir).with_sqlite_cache(cache);

    let mock_base_js = "var _yt_player = {}; function nFn(x){return 'n_' + x;}";
    let mock_jsc_bundle = r#"
        function jsc(input) {
            if (input.type === "player") {
                var responses = input.requests.map(function(r) {
                    var data = {};
                    r.challenges.forEach(function(c) {
                        data[c] = r.type + "_solved_" + c;
                    });
                    return { type: "result", data: data };
                });
                return {
                    type: "result",
                    preprocessed_player: "PREPROCESSED_AST_V1",
                    responses: responses
                };
            }
            if (input.type === "preprocessed") {
                if (input.preprocessed_player !== "PREPROCESSED_AST_V1") {
                    return { type: "error", error: "missing preprocessed_player" };
                }
                var responses = input.requests.map(function(r) {
                    var data = {};
                    r.challenges.forEach(function(c) {
                        data[c] = r.type + "_solved_" + c;
                    });
                    return { type: "result", data: data };
                });
                return { type: "result", responses: responses };
            }
            return { type: "error", error: "unknown input type" };
        }
    "#;

    let player_hash = "abc12345";
    let pre_path = paths
        .cache_dir
        .join("js_solver")
        .join(format!("pre_{player_hash}.js"));
    assert!(!pre_path.exists());

    // 1. Cold pass: preprocesses base.js -> writes pre_<hash>.js
    let cold = solver
        .solve_with_player(
            player_hash,
            mock_base_js,
            mock_jsc_bundle,
            Some("n1".to_owned()),
            Some("s1".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(cold.n_param.as_deref(), Some("n_solved_n1"));
    assert_eq!(cold.sig_param.as_deref(), Some("sig_solved_s1"));
    assert_eq!(solver.js_eval_count(), 1);
    assert!(pre_path.exists());
    assert_eq!(
        fs::read_to_string(&pre_path).unwrap(),
        "PREPROCESSED_AST_V1"
    );

    // 2. Warm pass with a new n2: uses Preprocessed path (requires pre_path content)
    let warm = solver
        .solve_with_player(
            player_hash,
            "UNUSED_RAW_ON_WARM_PATH",
            mock_jsc_bundle,
            Some("n2".to_owned()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(warm.n_param.as_deref(), Some("n_solved_n2"));
    assert_eq!(solver.js_eval_count(), 2);

    // 3. Repeat identical n2: hits L1/L2 memo cache with js_eval_count unchanged & < 5 ms
    let t0 = Instant::now();
    let memo_hit = solver
        .solve_with_player(
            player_hash,
            "UNUSED",
            mock_jsc_bundle,
            Some("n2".to_owned()),
            None,
        )
        .await
        .unwrap();
    assert!(t0.elapsed().as_millis() < 5);
    assert_eq!(memo_hit.n_param.as_deref(), Some("n_solved_n2"));
    assert_eq!(solver.js_eval_count(), 2);
}

#[tokio::test]
async fn ejs_solver_evicts_old_player_bundles_keeping_latest_three() {
    let tmp = TempDir::new().unwrap();
    let js_solver_dir = tmp.path().join("js_solver");
    fs::create_dir_all(&js_solver_dir).unwrap();

    let routes: RouteMap = Arc::new(Mutex::new(HashMap::new()));
    let hashes = ["1111aaaa", "2222bbbb", "3333cccc", "4444dddd", "5555eeee"];
    for hash in hashes {
        routes.lock().unwrap().insert(
            format!("/s/player/{hash}/player_ias.vflset/en_US/base.js"),
            (StatusCode::OK, format!("// player {hash}").into_bytes()),
        );
    }

    let (addr, shutdown) = spawn_mock_server(routes).await;
    let fetcher = PlayerScriptFetcher::new(js_solver_dir.clone()).with_allow_localhost(true);

    for hash in hashes {
        let url = format!("http://{addr}/s/player/{hash}/player_ias.vflset/en_US/base.js");
        fetcher.fetch_player_script(hash, &url).await.unwrap();
        // Ensure distinct mtimes across iterations on filesystems with coarse timestamps.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let remaining: Vec<String> = fs::read_dir(&js_solver_dir)
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("raw_"))
        .collect();
    assert_eq!(remaining.len(), 3, "expected 3 bundles, got {remaining:?}");
    let _ = shutdown.send(());
}

#[tokio::test]
async fn account_probe_detects_premium_and_free_accounts_and_preserves_on_network_error() {
    let tmp = TempDir::new().unwrap();
    let paths = test_app_paths(&tmp);
    let cache = Arc::new(SqliteCacheStore::open(&paths.data_dir).unwrap());
    let auth_store = Arc::new(FileAuthStore::new(paths.data_dir.join("auth.json")));
    let solver = Arc::new(QuickJsSolver::new(&paths.cache_dir));

    let routes: RouteMap = Arc::new(Mutex::new(HashMap::new()));
    let premium_json = json!({
        "actions": [{
            "openPopupAction": {
                "popup": {
                    "multiPageMenuRenderer": {
                        "header": {
                            "activeAccountHeaderRenderer": {
                                "accountName": { "runs": [{ "text": "Aashish Premium" }] },
                                "channelHandle": { "runs": [{ "text": "@aashish" }] },
                                "accountPhoto": { "thumbnails": [{ "url": "https://example.com/avatar.jpg" }] }
                            }
                        },
                        "sections": [{
                            "multiPageMenuSectionRenderer": {
                                "items": [{
                                    "compactLinkRenderer": {
                                        "title": { "runs": [{ "text": "Paid memberships" }] },
                                        "icon": { "iconType": "MANAGE_MEMBERSHIP" },
                                        "navigationEndpoint": {
                                            "browseEndpoint": { "browseId": "SPunlimited" }
                                        }
                                    }
                                }]
                            }
                        }]
                    }
                }
            }
        }]
    });
    routes.lock().unwrap().insert(
        "/youtubei/v1/account/account_menu".to_owned(),
        (StatusCode::OK, serde_json::to_vec(&premium_json).unwrap()),
    );

    let (addr, shutdown) = spawn_mock_server(routes.clone()).await;
    let client = InnerTubeClient::new(cache, auth_store, solver).unwrap();
    client.set_api_base_override(Some(format!("http://{addr}/youtubei/v1")));
    client
        .sign_in(AuthSession {
            cookie_header: "SAPISID=test_sid; __Secure-3PAPISID=test_3p".to_owned(),
            auth_user: 0,
            page_id: None,
            visitor_data: None,
            account_name: None,
            is_premium: false,
        })
        .unwrap();

    // 1. Premium detection
    let profile = AccountProfileProbe::probe_and_update(&client)
        .await
        .unwrap();
    assert!(profile.is_premium);
    assert_eq!(profile.account_name, "Aashish Premium");
    assert_eq!(profile.channel_handle.as_deref(), Some("@aashish"));
    assert_eq!(client.account_tier(), AccountTier::PremiumSignedIn);

    // 2. Transient 500 error preserves prior PremiumSignedIn state
    routes.lock().unwrap().insert(
        "/youtubei/v1/account/account_menu".to_owned(),
        (StatusCode::INTERNAL_SERVER_ERROR, b"upstream 500".to_vec()),
    );
    assert!(
        AccountProfileProbe::probe_and_update(&client)
            .await
            .is_err()
    );
    assert_eq!(client.account_tier(), AccountTier::PremiumSignedIn);
    assert!(client.current_session().unwrap().is_premium);

    // 3. Free account detection (upsell present)
    let free_json = json!({
        "actions": [{
            "openPopupAction": {
                "popup": {
                    "multiPageMenuRenderer": {
                        "header": {
                            "activeAccountHeaderRenderer": {
                                "accountName": { "runs": [{ "text": "Free Listener" }] }
                            }
                        },
                        "sections": [{
                            "multiPageMenuSectionRenderer": {
                                "items": [{
                                    "compactLinkRenderer": {
                                        "title": { "runs": [{ "text": "Get Music Premium" }] },
                                        "navigationEndpoint": {
                                            "browseEndpoint": { "browseId": "SPunlimited" }
                                        }
                                    }
                                }]
                            }
                        }]
                    }
                }
            }
        }]
    });
    routes.lock().unwrap().insert(
        "/youtubei/v1/account/account_menu".to_owned(),
        (StatusCode::OK, serde_json::to_vec(&free_json).unwrap()),
    );
    let free_profile = AccountProfileProbe::probe_and_update(&client)
        .await
        .unwrap();
    assert!(!free_profile.is_premium);
    assert_eq!(client.account_tier(), AccountTier::FreeSignedIn);

    let _ = shutdown.send(());
}

#[tokio::test]
async fn remote_config_accepts_valid_ed25519_signature_and_rejects_tampered_or_rollback() {
    let tmp = TempDir::new().unwrap();
    let paths = test_app_paths(&tmp);
    let cache = Arc::new(SqliteCacheStore::open(&paths.data_dir).unwrap());
    let solver = Arc::new(QuickJsSolver::new(&paths.cache_dir));

    let signing_key = SigningKey::from_bytes(&[7u8; 32]);
    let verifying_key = signing_key.verifying_key();

    let mut valid_matrix = ClientMatrix::embedded_default().unwrap();
    valid_matrix.updated_at_unix_secs += 1000;
    valid_matrix.source = "ci-signed-test".to_owned();
    let valid_bytes = serde_json::to_vec_pretty(&valid_matrix).unwrap();
    let valid_sig_hex = hex::encode(signing_key.sign(&valid_bytes).to_bytes());

    let routes: RouteMap = Arc::new(Mutex::new(HashMap::new()));
    routes.lock().unwrap().insert(
        "/client_matrix.json".to_owned(),
        (StatusCode::OK, valid_bytes.clone()),
    );
    routes.lock().unwrap().insert(
        "/client_matrix.json.sig".to_owned(),
        (StatusCode::OK, valid_sig_hex.clone().into_bytes()),
    );

    let (addr, shutdown) = spawn_mock_server(routes.clone()).await;
    let svc = RemoteConfigSyncService::with_verifying_key(
        paths.cache_dir.join("remote_config"),
        cache,
        solver,
        Some(format!("http://{addr}")),
        verifying_key,
    )
    .unwrap()
    .with_allow_insecure_localhost(true);

    // 1. Valid signature accepted & persisted
    let synced = svc.sync_client_matrix(true).await.unwrap().unwrap();
    assert_eq!(synced.source, "ci-signed-test");
    assert!(svc.matrix_cache_path().exists());

    // 2. 1-byte tampered payload rejected
    let mut tampered = valid_bytes.clone();
    tampered[5] ^= 0x01;
    routes
        .lock()
        .unwrap()
        .insert("/client_matrix.json".to_owned(), (StatusCode::OK, tampered));
    assert!(svc.sync_client_matrix(true).await.is_err());

    // 3. Rollback (older timestamp) rejected even if validly signed
    let mut older_matrix = valid_matrix.clone();
    older_matrix.updated_at_unix_secs -= 500;
    let older_bytes = serde_json::to_vec_pretty(&older_matrix).unwrap();
    let older_sig = hex::encode(signing_key.sign(&older_bytes).to_bytes());
    routes.lock().unwrap().insert(
        "/client_matrix.json".to_owned(),
        (StatusCode::OK, older_bytes),
    );
    routes.lock().unwrap().insert(
        "/client_matrix.json.sig".to_owned(),
        (StatusCode::OK, older_sig.into_bytes()),
    );
    assert!(svc.sync_client_matrix(true).await.is_err());

    // 4. Hard security invariant: ANDROID_VR with cookies_or_anonymous rejected even if validly signed
    let mut evil_matrix = valid_matrix.clone();
    evil_matrix.updated_at_unix_secs += 500;
    for c in &mut evil_matrix.clients {
        if c.kind == "android_vr" {
            c.auth_mode = "cookies_or_anonymous".to_owned();
        }
    }
    let evil_bytes = serde_json::to_vec_pretty(&evil_matrix).unwrap();
    let evil_sig = hex::encode(signing_key.sign(&evil_bytes).to_bytes());
    routes.lock().unwrap().insert(
        "/client_matrix.json".to_owned(),
        (StatusCode::OK, evil_bytes),
    );
    routes.lock().unwrap().insert(
        "/client_matrix.json.sig".to_owned(),
        (StatusCode::OK, evil_sig.into_bytes()),
    );
    assert!(svc.sync_client_matrix(true).await.is_err());

    let _ = shutdown.send(());
}

#[tokio::test]
async fn remote_solver_manifest_downloads_and_verifies_sha256_before_hot_swap() {
    let tmp = TempDir::new().unwrap();
    let paths = test_app_paths(&tmp);
    let cache = Arc::new(SqliteCacheStore::open(&paths.data_dir).unwrap());
    let solver = Arc::new(QuickJsSolver::new(&paths.cache_dir));

    let signing_key = SigningKey::from_bytes(&[9u8; 32]);
    let verifying_key = signing_key.verifying_key();

    let routes: RouteMap = Arc::new(Mutex::new(HashMap::new()));
    let (addr, shutdown) = spawn_mock_server(routes.clone()).await;

    let good_js = "function jsc(i){ return { type: 'result', responses: [] }; }";
    let good_sha = EjsBundleManifest::sha256_of(good_js);

    let mut bad_manifest = EjsBundleManifest::embedded_default().unwrap();
    bad_manifest.bundle_version = "0.5.1-test".to_owned();
    bad_manifest.pinned_version = String::new();
    bad_manifest.sha256_hex = "0".repeat(64);
    bad_manifest.updatable_remote_url = format!("http://{addr}/yt.solver.lib.min.js");

    let bad_bytes = serde_json::to_vec_pretty(&bad_manifest).unwrap();
    let bad_sig = hex::encode(signing_key.sign(&bad_bytes).to_bytes());
    routes.lock().unwrap().insert(
        "/solver_manifest.json".to_owned(),
        (StatusCode::OK, bad_bytes),
    );
    routes.lock().unwrap().insert(
        "/solver_manifest.json.sig".to_owned(),
        (StatusCode::OK, bad_sig.into_bytes()),
    );
    routes.lock().unwrap().insert(
        "/yt.solver.lib.min.js".to_owned(),
        (StatusCode::OK, good_js.as_bytes().to_vec()),
    );

    let svc = RemoteConfigSyncService::with_verifying_key(
        paths.cache_dir.join("remote_config"),
        cache,
        solver.clone(),
        Some(format!("http://{addr}")),
        verifying_key,
    )
    .unwrap()
    .with_allow_insecure_localhost(true);

    // 1. SHA-256 mismatch aborts update and keeps previous bundle version active
    assert!(svc.sync_solver_manifest(true).await.is_err());
    assert_eq!(
        solver.active_ejs_manifest().unwrap().bundle_version,
        EjsBundleManifest::embedded_default()
            .unwrap()
            .bundle_version
    );

    // 2. Valid SHA-256 installs update atomically
    let mut good_manifest = bad_manifest.clone();
    good_manifest.sha256_hex = good_sha;
    let good_bytes = serde_json::to_vec_pretty(&good_manifest).unwrap();
    let good_sig = hex::encode(signing_key.sign(&good_bytes).to_bytes());
    routes.lock().unwrap().insert(
        "/solver_manifest.json".to_owned(),
        (StatusCode::OK, good_bytes),
    );
    routes.lock().unwrap().insert(
        "/solver_manifest.json.sig".to_owned(),
        (StatusCode::OK, good_sig.into_bytes()),
    );

    let installed = svc.sync_solver_manifest(true).await.unwrap().unwrap();
    assert_eq!(installed.bundle_version, "0.5.1-test");
    assert_eq!(
        solver.active_ejs_manifest().unwrap().bundle_version,
        "0.5.1-test"
    );

    let _ = shutdown.send(());
}
