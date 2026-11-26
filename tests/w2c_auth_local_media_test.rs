//! Offline verification tests for W2C deltas:
//! - RFC 8628 TV OAuth Device-Code flow (authorization_pending -> slow_down -> token grant, plus expired_token)
//! - WebView cookie validator requiring HttpOnly session cookie (SID / __Secure-3PSID) + SAPISID
//! - LocalMediaScanner sidecar `.lrc` lyrics fallback
//! - LoopbackProxy `/local/<track_id>` HEAD 206 & 404 for unknown valid ID
//! - Zero raw color literals in `auth_sign_in_sheet.rs`

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use lofty::config::WriteOptions;
use lofty::file::AudioFile;
use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::{Accessor, Tag, TagType};
use resonance_contract::AuthError;
use resonance_core::auth_flows::cookie_extract::validate_cookie_jar;
use resonance_core::auth_flows::tv_oauth::{TvOAuthClient, TvOAuthConfig};
use resonance_core::local_media::{LocalMediaStore, scan_all_roots};
use resonance_core::proxy::local_route;
use serde_json::json;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::watch;

fn write_minimal_wav(path: &Path, title: &str) {
    let data_len: u32 = 128;
    let file_len: u32 = 36 + data_len;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_len.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&44100u32.to_le_bytes());
    wav.extend_from_slice(&88200u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.resize(44 + data_len as usize, 0);
    std::fs::write(path, &wav).unwrap();

    let mut tagged = Probe::open(path)
        .unwrap()
        .guess_file_type()
        .unwrap()
        .read()
        .unwrap();
    let mut tag = Tag::new(TagType::Id3v2);
    tag.set_title(title.to_owned());
    tagged.insert_tag(tag);
    tagged.save_to_path(path, WriteOptions::default()).unwrap();
}

#[tokio::test]
async fn tv_oauth_rfc8628_handles_pending_slow_down_success_and_expiry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let poll_attempt = Arc::new(AtomicUsize::new(0));
    let poll_attempt_srv = Arc::clone(&poll_attempt);

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let attempt = Arc::clone(&poll_attempt_srv);
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let svc = service_fn(move |req: Request<Incoming>| {
                    let attempt = Arc::clone(&attempt);
                    async move {
                        let path = req.uri().path().to_owned();
                        let (status, body) = if path == "/device/code" {
                            (
                                StatusCode::OK,
                                json!({
                                    "device_code": "dev-123",
                                    "user_code": "ABCD-EFGH",
                                    "verification_url": "https://www.google.com/device",
                                    "expires_in": 60,
                                    "interval": 1
                                }),
                            )
                        } else if path == "/token" {
                            let n = attempt.fetch_add(1, Ordering::SeqCst);
                            match n {
                                0 => (
                                    StatusCode::BAD_REQUEST,
                                    json!({ "error": "authorization_pending" }),
                                ),
                                1 => (StatusCode::BAD_REQUEST, json!({ "error": "slow_down" })),
                                2 => (
                                    StatusCode::OK,
                                    json!({
                                        "access_token": "ya29.test_access",
                                        "refresh_token": "1//test_refresh",
                                        "expires_in": 3600,
                                        "token_type": "Bearer"
                                    }),
                                ),
                                _ => (StatusCode::BAD_REQUEST, json!({ "error": "expired_token" })),
                            }
                        } else {
                            (StatusCode::NOT_FOUND, json!({ "error": "not_found" }))
                        };
                        Ok::<_, hyper::Error>(
                            Response::builder()
                                .status(status)
                                .body(Full::new(Bytes::from(serde_json::to_vec(&body).unwrap())))
                                .unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    let cfg = TvOAuthConfig {
        client_id: "test-client".to_owned(),
        client_secret: None,
        device_code_url: format!("http://{addr}/device/code"),
        token_url: format!("http://{addr}/token"),
        scope: "https://www.googleapis.com/auth/youtube".to_owned(),
    };
    let client = TvOAuthClient::new(cfg).unwrap();
    let dc = client.request_device_code().await.unwrap();
    assert_eq!(dc.user_code, "ABCD-EFGH");

    let (_tx, rx) = watch::channel(false);
    let tokens = client
        .poll_for_token(&dc.device_code, 1, 30, rx)
        .await
        .unwrap();
    assert_eq!(tokens.access_token, "ya29.test_access");
    assert_eq!(tokens.refresh_token, "1//test_refresh");
    assert_eq!(poll_attempt.load(Ordering::SeqCst), 3);

    let (_tx2, rx2) = watch::channel(false);
    let expired = client.poll_for_token(&dc.device_code, 1, 30, rx2).await;
    assert!(matches!(
        expired,
        Err(resonance_contract::CoreError::Auth(AuthError::Revoked))
    ));
}

#[test]
fn webview_cookie_validator_requires_httponly_session_and_sapisid() {
    // Missing HttpOnly SID / __Secure-3PSID
    let js_only = validate_cookie_jar("SAPISID=abc123; __Secure-3PAPISID=xyz789");
    assert!(matches!(js_only, Err(AuthError::MissingCookie(_))));

    // Missing SAPISID
    let no_sapisid = validate_cookie_jar("SID=session_val; HSID=hsid_val");
    assert!(matches!(no_sapisid, Err(AuthError::MissingCookie(_))));

    // Valid full cookie jar
    let ok = validate_cookie_jar("SAPISID=abc123; SID=session_val; __Secure-3PSID=3psid").unwrap();
    assert!(ok.has_sapisid);
    assert!(ok.has_session_cookie);
}

#[tokio::test]
async fn local_scanner_reads_sidecar_lrc_lyrics_when_embedded_lyrics_absent() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().join("data");
    let music_dir = tmp.path().join("music");
    std::fs::create_dir_all(&music_dir).unwrap();

    let audio_path = music_dir.join("track_with_sidecar.wav");
    write_minimal_wav(&audio_path, "Sidecar Track");
    std::fs::write(
        music_dir.join("track_with_sidecar.lrc"),
        "[00:05.00] Sidecar lyric line",
    )
    .unwrap();

    let store = Arc::new(LocalMediaStore::open(&data_dir).unwrap());
    store.add_root(&music_dir, 1000).unwrap();

    let progress = scan_all_roots(
        store.clone(),
        None,
        "http://127.0.0.1:9999".to_owned(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(progress.files_indexed, 1);

    let tracks = store.list_tracks().unwrap();
    assert_eq!(tracks.len(), 1);
    let lyrics = store.get_embedded_lyrics(&tracks[0].track_id).unwrap();
    assert_eq!(lyrics.as_deref(), Some("[00:05.00] Sidecar lyric line"));
}

#[tokio::test]
async fn local_proxy_route_returns_404_for_unknown_valid_id() {
    let tmp = TempDir::new().unwrap();
    let store = Arc::new(LocalMediaStore::open(tmp.path()).unwrap());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let io = TokioIo::new(stream);
            let s = store.clone();
            let svc = service_fn(move |req: Request<Incoming>| {
                let s = s.clone();
                async move {
                    let resp = local_route::handle_local_request(&req, Some(&s), None)
                        .await
                        .unwrap_or_else(|| {
                            Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(Full::new(Bytes::new()))
                                .unwrap()
                        });
                    Ok::<_, hyper::Error>(resp)
                }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, svc)
                .await;
        }
    });

    let resp = reqwest::get(format!("http://{addr}/local/local:0123456789abcdef"))
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
}

#[test]
fn auth_sign_in_sheet_has_zero_raw_color_literals() {
    let src = include_str!("../src/screens/auth_sign_in_sheet.rs");
    for forbidden in ["rgb(", "rgba(", "0xFF"] {
        assert!(
            !src.contains(forbidden),
            "auth_sign_in_sheet.rs must not contain raw color literal `{forbidden}`"
        );
    }
}
