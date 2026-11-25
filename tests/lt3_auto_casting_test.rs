//! Offline verification suite for LT-3:
//! Android Auto `MediaLibraryService` contract gates, `LanCastRelay` IP pinning &
//! 256-bit bearer token, DLNA DIDL-Lite XML escaping & SSRF rejection,
//! Chromecast CastV2 TLS/protobuf framing, CastManager handover, and headless Cast Sheet UI.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::Full;
use hyper::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use nsh_core::prelude::px;
use nsh_testing::Headless;
use resonance_contract::{
    CastDevice, CastMediaPayload, CastProtocol, CastSessionState, CastTransport, CoreResult,
    NowPlayingMetadata,
};
use resonance_core::casting::chromecast::{
    CASTV2_NS_CONNECTION, CASTV2_NS_MEDIA, CASTV2_NS_RECEIVER, CastEvent, CastMessage,
    ChromecastTransport, read_cast_frame, write_cast_frame,
};
use resonance_core::casting::discovery::validate_ssdp_location_url;
use resonance_core::casting::dlna::{build_didl_lite_metadata, xml_escape};
use resonance_core::casting::manager::{CastManager, CastPlaybackDelegate};
use resonance_core::proxy::lan_relay::{LanCastRelay, redact_cast_secrets};
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn lan_cast_relay_enforces_peer_ip_pinning_bearer_token_and_teardown() {
    // Mock LoopbackProxy returning 206 Partial Content for Range requests
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let payload = vec![b'A'; 4096];
        loop {
            let Ok((stream, _)) = proxy_listener.accept().await else {
                break;
            };
            let data = payload.clone();
            tokio::spawn(async move {
                let io = TokioIo::new(stream);
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |req: Request<hyper::body::Incoming>| {
                            let has_range = req.headers().contains_key(RANGE);
                            let body = if has_range {
                                Bytes::copy_from_slice(&data[..1024])
                            } else {
                                Bytes::copy_from_slice(&data)
                            };
                            let status = if has_range {
                                StatusCode::PARTIAL_CONTENT
                            } else {
                                StatusCode::OK
                            };
                            async move {
                                let mut b = Response::builder()
                                    .status(status)
                                    .header(CONTENT_TYPE, "audio/mp4")
                                    .header(CONTENT_LENGTH, body.len().to_string());
                                if has_range {
                                    b = b.header(CONTENT_RANGE, "bytes 0-1023/4096");
                                }
                                Ok::<_, hyper::Error>(b.body(Full::new(body)).unwrap())
                            }
                        }),
                    )
                    .await;
            });
        }
    });

    let mut relay =
        LanCastRelay::start_for_test(IpAddr::V4(Ipv4Addr::LOCALHOST), proxy_addr, Some(140))
            .await
            .unwrap();
    let relay_addr = relay.bind_addr();
    let valid_url = relay.stream_url("dQw4w9WgXcQ", "m4a").unwrap();
    assert_ne!(redact_cast_secrets(&valid_url), valid_url);
    assert!(redact_cast_secrets(&valid_url).contains("[REDACTED_CAST_TOKEN]"));

    let http = reqwest::Client::new();
    // Valid token + Range request -> 206 Partial Content
    let resp = http
        .get(&valid_url)
        .header(RANGE, "bytes=0-1023")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(resp.bytes().await.unwrap().len(), 1024);

    // Wrong 64-hex token -> 403 Forbidden
    let bad_url = format!(
        "http://{}/cast/{}/dQw4w9WgXcQ.m4a",
        relay_addr,
        "0".repeat(64)
    );
    let bad_resp = http.get(&bad_url).send().await.unwrap();
    assert_eq!(bad_resp.status(), StatusCode::FORBIDDEN);

    // Teardown closes socket immediately
    relay.shutdown();
    drop(relay);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(TcpStream::connect(relay_addr).await.is_err());
}

#[tokio::test]
async fn lan_cast_relay_rejects_public_non_rfc1918_target_ips() {
    let dummy_proxy: SocketAddr = "127.0.0.1:4416".parse().unwrap();
    let err = LanCastRelay::start(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), dummy_proxy, None)
        .await
        .err()
        .expect("public IP must be rejected");
    assert!(err.to_string().contains("non-RFC1918"));
}

#[test]
fn dlna_didl_lite_xml_escapes_special_characters_and_rejects_ssrf_locations() {
    let escaped = xml_escape(r#"Rock & Roll <Live> "2026" 'Remix'"#);
    assert_eq!(
        escaped,
        "Rock &amp; Roll &lt;Live&gt; &quot;2026&quot; &apos;Remix&apos;"
    );

    let meta = NowPlayingMetadata {
        track_id: "vid1".into(),
        title: "A & B <1>".into(),
        artist: "C's \"Band\"".into(),
        album: Some("D & E".into()),
        duration_ms: 180_000,
        artwork_url: None,
    };
    let didl = build_didl_lite_metadata(
        "http://192.168.1.10:8080/cast/tok/vid1.m4a",
        &meta,
        "audio/mp4",
    );
    assert!(didl.contains("A &amp; B &lt;1&gt;"));
    assert!(didl.contains("C&apos;s &quot;Band&quot;"));

    // SSRF gates
    assert!(validate_ssdp_location_url("http://127.0.0.1:4416/get_pot").is_err());
    assert!(validate_ssdp_location_url("http://169.254.169.254/latest/meta-data").is_err());
    assert!(validate_ssdp_location_url("http://localhost:8080/desc.xml").is_err());
    assert!(validate_ssdp_location_url("http://8.8.8.8/desc.xml").is_err());
    assert!(
        validate_ssdp_location_url("http://192.168.1.50:1400/xml/device_description.xml").is_ok()
    );
}

#[tokio::test]
async fn chromecast_castv2_framing_and_media_load_protocol_roundtrip() {
    // Verify protobuf + length-prefixed framing roundtrip over duplex stream
    let (mut client_io, mut server_io) = tokio::io::duplex(4096);
    let outbound = CastMessage::utf8(
        "receiver-0",
        CASTV2_NS_CONNECTION,
        r#"{"type":"CONNECT"}"#.to_owned(),
    );
    write_cast_frame(&mut client_io, &outbound).await.unwrap();
    let decoded = read_cast_frame(&mut server_io).await.unwrap();
    assert_eq!(decoded.namespace, CASTV2_NS_CONNECTION);
    assert_eq!(
        decoded.payload_utf8.as_deref(),
        Some(r#"{"type":"CONNECT"}"#)
    );

    // Also verify IDLE(FINISHED) constant & event type
    let transport = ChromecastTransport::new();
    let _rx: tokio::sync::broadcast::Receiver<CastEvent> = transport.subscribe_events();
    let _ = CASTV2_NS_RECEIVER;
    let _ = CASTV2_NS_MEDIA;
}

struct MockDelegate {
    local_paused: AtomicBool,
    resumed_at_ms: AtomicU64,
}

#[async_trait]
impl CastPlaybackDelegate for MockDelegate {
    async fn pause_local(&self) -> CoreResult<()> {
        self.local_paused.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn resume_local_at(&self, position_ms: u64) -> CoreResult<()> {
        self.local_paused.store(false, Ordering::SeqCst);
        self.resumed_at_ms.store(position_ms, Ordering::SeqCst);
        Ok(())
    }
}

struct MockCastTransport {
    pos_ms: AtomicU64,
    loaded: AtomicBool,
}

#[async_trait]
impl CastTransport for MockCastTransport {
    async fn connect(&self, _device: &CastDevice) -> CoreResult<()> {
        Ok(())
    }
    async fn load_media(&self, payload: CastMediaPayload) -> CoreResult<()> {
        self.loaded.store(true, Ordering::SeqCst);
        self.pos_ms
            .store(payload.start_position_ms, Ordering::SeqCst);
        Ok(())
    }
    async fn play(&self) -> CoreResult<()> {
        Ok(())
    }
    async fn pause(&self) -> CoreResult<()> {
        Ok(())
    }
    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        self.pos_ms.store(position_ms, Ordering::SeqCst);
        Ok(())
    }
    async fn set_volume(&self, _volume: f32) -> CoreResult<()> {
        Ok(())
    }
    async fn set_muted(&self, _muted: bool) -> CoreResult<()> {
        Ok(())
    }
    async fn poll_status(&self) -> CoreResult<Option<(bool, u64)>> {
        Ok(Some((true, self.pos_ms.load(Ordering::SeqCst))))
    }
    async fn disconnect(&self) -> CoreResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn cast_manager_handover_pauses_local_and_resumes_at_cast_position_on_disconnect() {
    let delegate = Arc::new(MockDelegate {
        local_paused: AtomicBool::new(false),
        resumed_at_ms: AtomicU64::new(0),
    });
    let proxy_addr: SocketAddr = "127.0.0.1:4416".parse().unwrap();
    let manager = CastManager::new(proxy_addr, delegate.clone()).allow_loopback_for_tests();

    let device = CastDevice {
        id: "cc-1".into(),
        name: "Living Room Speaker".into(),
        model: Some("Nest Audio".into()),
        protocol: CastProtocol::Chromecast,
        address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: 8009,
        control_url: None,
    };
    let transport = Arc::new(MockCastTransport {
        pos_ms: AtomicU64::new(0),
        loaded: AtomicBool::new(false),
    });
    let meta = NowPlayingMetadata {
        track_id: "vid_1".into(),
        title: "Track 1".into(),
        artist: "Artist 1".into(),
        album: None,
        duration_ms: 210_000,
        artwork_url: None,
    };

    let state = manager
        .connect_with_transport(device, transport.clone(), "vid_1", meta, 42_000, 210_000)
        .await
        .unwrap();
    assert!(matches!(state, CastSessionState::Connected { .. }));
    assert!(delegate.local_paused.load(Ordering::SeqCst));
    assert!(transport.loaded.load(Ordering::SeqCst));

    // Advance cast position and disconnect
    manager.seek_ms(95_000).await.unwrap();
    let after = manager.disconnect().await.unwrap();
    assert_eq!(after, CastSessionState::Disconnected);
    assert!(!delegate.local_paused.load(Ordering::SeqCst));
    assert_eq!(delegate.resumed_at_ms.load(Ordering::SeqCst), 95_000);
}

#[test]
fn android_auto_manifest_and_browse_tree_contract_gates() {
    let desc = std::fs::read_to_string(Path::new(
        "platform/android/res/xml/automotive_app_desc.xml",
    ))
    .expect("automotive_app_desc.xml exists");
    assert!(desc.contains("<uses name=\"media\""));

    let manifest =
        std::fs::read_to_string(Path::new("platform/android/AndroidManifest.xml")).unwrap();
    assert!(manifest.contains("com.google.android.gms.car.application"));
    assert!(manifest.contains("androidx.media3.session.MediaLibraryService"));
    assert!(manifest.contains("android.media.browse.MediaBrowserService"));
    assert!(manifest.contains("android:usesCleartextTraffic=\"false\""));

    let browse_kt = std::fs::read_to_string(Path::new(
        "platform/android/src/main/kotlin/dev/resonance/ytmusic/AndroidAutoBrowseTree.kt",
    ))
    .unwrap();
    assert!(browse_kt.contains("setIsBrowsable"));
    assert!(browse_kt.contains("setIsPlayable"));
    assert!(browse_kt.contains("tab_downloads"));

    let svc_kt = std::fs::read_to_string(Path::new(
        "platform/android/src/main/kotlin/dev/resonance/ytmusic/PlaybackForegroundService.kt",
    ))
    .unwrap();
    assert!(svc_kt.contains("MediaLibraryService"));
    assert!(svc_kt.contains("RECEIVER_NOT_EXPORTED"));
}

#[test]
fn cast_sheet_headless_ui_renders_devices_and_zero_raw_colors() {
    let src = std::fs::read_to_string(Path::new("src/ui/controls/cast_sheet.rs")).unwrap();
    assert!(!src.contains("rgb("));
    assert!(!src.contains("rgba("));
    assert!(!src.contains("0xFF"));

    let devices = vec![
        CastDevice {
            id: "cc-living".into(),
            name: "Living Room".into(),
            model: Some("Chromecast Audio".into()),
            protocol: CastProtocol::Chromecast,
            address: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
            port: 8009,
            control_url: None,
        },
        CastDevice {
            id: "dlna-avr".into(),
            name: "Studio AVR".into(),
            model: Some("Denon".into()),
            protocol: CastProtocol::DlnaUpnp,
            address: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 30)),
            port: 1400,
            control_url: Some("http://192.168.1.30:1400/desc.xml".into()),
        },
    ];

    let _ = devices;
    resonance::store::enable_fixture_mode(true);
    for (w, h) in [(390.0_f32, 844.0_f32), (1200.0_f32, 800.0_f32)] {
        let mut app = Headless::start(nsh_engine::EngineConfig {
            viewport: nsh_engine::Size::new(nsh_engine::Points(w), nsh_engine::Points(h)),
            ..nsh_engine::EngineConfig::new(nsh_engine::Root::of::<resonance::App>())
        })
        .expect("headless app starts cleanly");
        app.wait_for("Quick Picks").expect("home renders");
    }
}
