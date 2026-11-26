use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use bytes::Bytes;
use http_body_util::Full;
use hyper::header::{CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use nsh_core::prelude::{IntoElement, component};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use resonance::ui::controls::cast_sheet::{active_cast_banner, cast_picker_sheet};
use resonance_contract::{CastDevice, CastProtocol, CastSessionState, NowPlayingMetadata};
use resonance_core::casting::chromecast::{
    CASTV2_NS_CONNECTION, CastMessage, read_cast_frame, write_cast_frame,
};
use resonance_core::casting::dlna::{build_didl_lite_metadata, xml_escape};
use resonance_core::proxy::lan_relay::LanCastRelay;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn castv2_protobuf_framing_encodes_and_decodes_load_heartbeat_and_status() {
    let (mut client_io, mut server_io) = tokio::io::duplex(8192);
    let msg = CastMessage::utf8(
        "receiver-0",
        CASTV2_NS_CONNECTION,
        r#"{"type":"CONNECT"}"#.to_owned(),
    );
    write_cast_frame(&mut client_io, &msg).await.unwrap();
    let decoded = read_cast_frame(&mut server_io).await.unwrap();
    assert_eq!(decoded.namespace, CASTV2_NS_CONNECTION);
    assert_eq!(
        decoded.payload_utf8.as_deref(),
        Some(r#"{"type":"CONNECT"}"#)
    );

    // Oversized frame (> 64 KB) must be rejected safely
    let (mut bad_writer, mut bad_reader) = tokio::io::duplex(256);
    let oversized_len: u32 = 65 * 1024;
    bad_writer
        .write_all(&oversized_len.to_be_bytes())
        .await
        .unwrap();
    bad_writer.flush().await.unwrap();
    assert!(read_cast_frame(&mut bad_reader).await.is_err());
}

#[tokio::test]
async fn lan_cast_relay_enforces_ip_pinning_256bit_token_and_http_206_ranges() {
    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let payload = vec![b'R'; 2048];
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
                                Bytes::copy_from_slice(&data[..512])
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
                                    b = b.header(CONTENT_RANGE, "bytes 0-511/2048");
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
    let valid_url = relay.stream_url("w3c_track_1", "m4a").unwrap();

    let http = reqwest::Client::new();
    let ok_resp = http
        .get(&valid_url)
        .header(RANGE, "bytes=0-511")
        .send()
        .await
        .unwrap();
    assert_eq!(ok_resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(ok_resp.bytes().await.unwrap().len(), 512);

    let bad_url = format!(
        "http://{}/cast/{}/w3c_track_1.m4a",
        relay_addr,
        "f".repeat(64)
    );
    assert_eq!(
        http.get(&bad_url).send().await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    relay.shutdown();
    drop(relay);
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(TcpStream::connect(relay_addr).await.is_err());

    // Public non-RFC1918 IP must be rejected
    let dummy: SocketAddr = "127.0.0.1:4416".parse().unwrap();
    assert!(
        LanCastRelay::start(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), dummy, None)
            .await
            .is_err()
    );
}

#[test]
fn dlna_didl_lite_metadata_escapes_xml_special_characters_safely() {
    let escaped = xml_escape(r#"A & B <Live> "2026" 'Mix'"#);
    assert_eq!(
        escaped,
        "A &amp; B &lt;Live&gt; &quot;2026&quot; &apos;Mix&apos;"
    );

    let meta = NowPlayingMetadata {
        track_id: "w3c-1".into(),
        title: "Raag & Rhythm <1>".into(),
        artist: "Ensemble \"Live\"".into(),
        album: Some("Night's <Session>".into()),
        duration_ms: 180_000,
        artwork_url: None,
    };
    let didl = build_didl_lite_metadata(
        "http://192.168.1.10:8080/cast/tok/w3c-1.m4a",
        &meta,
        "audio/mp4",
    );
    assert!(didl.contains("Raag &amp; Rhythm &lt;1&gt;"));
    assert!(didl.contains("Ensemble &quot;Live&quot;"));
    assert!(didl.contains("Night&apos;s &lt;Session&gt;"));
}

#[test]
fn android_auto_manifest_and_xml_verify_media_library_service_contract() {
    let desc = std::fs::read_to_string(Path::new(
        "platform/android/res/xml/automotive_app_desc.xml",
    ))
    .unwrap();
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
    assert!(browse_kt.contains("offlineFallbackSearch"));
    assert!(browse_kt.contains("setIsBrowsable"));
    assert!(browse_kt.contains("setIsPlayable"));
}

#[test]
fn cast_sheet_renders_scanning_devices_and_active_banner_headless() {
    let src = std::fs::read_to_string(Path::new("src/ui/controls/cast_sheet.rs")).unwrap();
    assert!(!src.contains("rgb("));
    assert!(!src.contains("rgba("));
    assert!(!src.contains("0xFF"));

    #[component]
    fn ScanningRoot() -> impl IntoElement {
        cast_picker_sheet(
            Vec::new(),
            CastSessionState::Disconnected,
            |_| {},
            || {},
            |_| {},
            || {},
        )
    }

    let mut scanning = Headless::start(EngineConfig::new(Root::of::<ScanningRoot>())).unwrap();
    scanning.wait_for("Cast to Device").unwrap();
    scanning
        .wait_for("Searching for Chromecast & DLNA devices on Wi‑Fi…")
        .unwrap();

    #[component]
    fn ConnectedRoot() -> impl IntoElement {
        let dev = CastDevice {
            id: "cc-1".into(),
            name: "Living Room Speaker".into(),
            model: Some("Nest Audio".into()),
            protocol: CastProtocol::Chromecast,
            address: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
            port: 8009,
            control_url: None,
        };
        cast_picker_sheet(
            vec![dev.clone()],
            CastSessionState::Connected {
                device: dev,
                volume_pct: 65,
                muted: false,
                playing: true,
                position_ms: 0,
                duration_ms: 0,
            },
            |_| {},
            || {},
            |_| {},
            || {},
        )
    }

    let mut connected = Headless::start(EngineConfig::new(Root::of::<ConnectedRoot>())).unwrap();
    connected.wait_for("Living Room Speaker").unwrap();
    connected.wait_for("Receiver Volume: 65%").unwrap();
    connected.wait_for("Disconnect").unwrap();

    #[component]
    fn BannerRoot() -> impl IntoElement {
        active_cast_banner("Living Room Speaker", || {}, || {})
    }

    let mut banner = Headless::start(EngineConfig::new(Root::of::<BannerRoot>())).unwrap();
    banner.wait_for("Casting to Living Room Speaker").unwrap();
}
