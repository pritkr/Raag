//! Pure-Rust Chromecast CastV2 Audio Sender (`tokio-rustls` + `prost`).
//!
//! Zero `openssl-sys` dependencies; works identically on macOS, Linux, Windows, and Android.
//! Implements length-prefixed big-endian `u32` + `CastMessage` protobuf framing,
//! 5s `PING`/`PONG` heartbeat (3-missed-pong disconnect), `LAUNCH(CC1AD845)`,
//! `LOAD`/`PLAY`/`PAUSE`/`SEEK`/`STOP`/`SET_VOLUME`, and `IDLE(FINISHED)` auto-advance events.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use prost::Message;
use resonance_contract::{
    CastDevice, CastMediaPayload, CastTransport, CoreError, CoreResult, PlaybackError,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme,
};

pub const CASTV2_NS_CONNECTION: &str = "urn:x-cast:com.google.cast.tp.connection";
pub const CASTV2_NS_HEARTBEAT: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
pub const CASTV2_NS_RECEIVER: &str = "urn:x-cast:com.google.cast.receiver";
pub const CASTV2_NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
pub const DEFAULT_MEDIA_RECEIVER_APP_ID: &str = "CC1AD845";
pub const CASTV2_SENDER_ID: &str = "sender-0";
pub const CASTV2_RECEIVER_ID: &str = "receiver-0";

const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const MAX_MISSED_PONGS: u32 = 3;

/// CastV2 protobuf wire message (`extensions.api.cast_channel.CastMessage`).
#[derive(Clone, PartialEq, Message)]
pub struct CastMessage {
    #[prost(int32, tag = "1")]
    pub protocol_version: i32,
    #[prost(string, tag = "2")]
    pub source_id: String,
    #[prost(string, tag = "3")]
    pub destination_id: String,
    #[prost(string, tag = "4")]
    pub namespace: String,
    #[prost(int32, tag = "5")]
    pub payload_type: i32,
    #[prost(string, optional, tag = "6")]
    pub payload_utf8: Option<String>,
    #[prost(bytes = "vec", optional, tag = "7")]
    pub payload_binary: Option<Vec<u8>>,
}

impl CastMessage {
    pub fn utf8(
        destination_id: impl Into<String>,
        namespace: impl Into<String>,
        json_payload: String,
    ) -> Self {
        Self {
            protocol_version: 0,
            source_id: CASTV2_SENDER_ID.to_owned(),
            destination_id: destination_id.into(),
            namespace: namespace.into(),
            payload_type: 0,
            payload_utf8: Some(json_payload),
            payload_binary: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastEvent {
    TrackFinished,
    Disconnected(String),
}

/// Scoped TLS verifier for LAN Chromecast devices (which present self-signed device certificates).
#[derive(Debug)]
struct LanCastCertVerifier;

impl ServerCertVerifier for LanCastCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ED25519,
        ]
    }
}

pub async fn write_cast_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    msg: &CastMessage,
) -> CoreResult<()> {
    let encoded = msg.encode_to_vec();
    let len = encoded.len() as u32;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
    writer
        .write_all(&encoded)
        .await
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
    writer
        .flush()
        .await
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
    Ok(())
}

pub async fn read_cast_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> CoreResult<CastMessage> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
            format!("invalid CastV2 frame length {len}"),
        )));
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
    CastMessage::decode(&buf[..])
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))
}

struct SessionState {
    outbound_tx: mpsc::Sender<CastMessage>,
    transport_id: Arc<RwLock<Option<String>>>,
    media_session_id: Arc<AtomicI64>,
    is_playing: Arc<std::sync::atomic::AtomicBool>,
    position_ms: Arc<AtomicU64>,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

/// Pure-Rust Chromecast CastV2 client implementing [`CastTransport`].
pub struct ChromecastTransport {
    session: Mutex<Option<SessionState>>,
    request_id: AtomicU64,
    events_tx: broadcast::Sender<CastEvent>,
}

impl Default for ChromecastTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl ChromecastTransport {
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(32);
        Self {
            session: Mutex::new(None),
            request_id: AtomicU64::new(1),
            events_tx,
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<CastEvent> {
        self.events_tx.subscribe()
    }

    fn next_req_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_message(&self, msg: CastMessage) -> CoreResult<()> {
        let guard = self.session.lock().await;
        let Some(sess) = guard.as_ref() else {
            return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
                "chromecast not connected".to_owned(),
            )));
        };
        sess.outbound_tx.send(msg).await.map_err(|_| {
            CoreError::Playback(PlaybackError::BackendUnavailable(
                "cast channel closed".into(),
            ))
        })
    }

    async fn current_transport_id(&self) -> CoreResult<String> {
        let tid_arc = {
            let guard = self.session.lock().await;
            let Some(sess) = guard.as_ref() else {
                return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
                    "chromecast not connected".to_owned(),
                )));
            };
            Arc::clone(&sess.transport_id)
        };
        // Wait briefly for LAUNCH RECEIVER_STATUS if still in-flight
        for _ in 0..40 {
            if let Some(tid) = tid_arc.read().await.clone() {
                return Ok(tid);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err(CoreError::Playback(PlaybackError::BackendUnavailable(
            "chromecast receiver transportId not ready".to_owned(),
        )))
    }
}

fn build_tls_connector() -> CoreResult<TlsConnector> {
    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(LanCastCertVerifier))
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

#[async_trait]
impl CastTransport for ChromecastTransport {
    async fn connect(&self, device: &CastDevice) -> CoreResult<()> {
        self.disconnect().await?;

        let tcp = TcpStream::connect((device.address, device.port))
            .await
            .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
        let connector = build_tls_connector()?;
        let server_name = ServerName::try_from(device.address.to_string())
            .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
        let tls_stream: TlsStream<TcpStream> = connector
            .connect(server_name, tcp)
            .await
            .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;

        let (mut reader, mut writer) = tokio::io::split(tls_stream);
        let (outbound_tx, mut outbound_rx) = mpsc::channel::<CastMessage>(64);
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

        let transport_id: Arc<RwLock<Option<String>>> = Arc::new(RwLock::new(None));
        let media_session_id = Arc::new(AtomicI64::new(0));
        let is_playing = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let position_ms = Arc::new(AtomicU64::new(0));
        let missed_pongs = Arc::new(AtomicU32::new(0));

        // 1. CONNECT to receiver-0
        outbound_tx
            .send(CastMessage::utf8(
                CASTV2_RECEIVER_ID,
                CASTV2_NS_CONNECTION,
                json!({ "type": "CONNECT" }).to_string(),
            ))
            .await
            .map_err(|_| {
                CoreError::Playback(PlaybackError::BackendUnavailable(
                    "send connect failed".into(),
                ))
            })?;

        // 2. LAUNCH default media receiver
        let launch_id = self.next_req_id();
        outbound_tx
            .send(CastMessage::utf8(
                CASTV2_RECEIVER_ID,
                CASTV2_NS_RECEIVER,
                json!({
                    "type": "LAUNCH",
                    "appId": DEFAULT_MEDIA_RECEIVER_APP_ID,
                    "requestId": launch_id
                })
                .to_string(),
            ))
            .await
            .map_err(|_| {
                CoreError::Playback(PlaybackError::BackendUnavailable(
                    "send launch failed".into(),
                ))
            })?;

        let tid_ref = Arc::clone(&transport_id);
        let msid_ref = Arc::clone(&media_session_id);
        let playing_ref = Arc::clone(&is_playing);
        let pos_ref = Arc::clone(&position_ms);
        let missed_ref = Arc::clone(&missed_pongs);
        let events_tx = self.events_tx.clone();
        let tx_for_heartbeat = outbound_tx.clone();

        tokio::spawn(async move {
            let mut hb_interval = tokio::time::interval(HEARTBEAT_INTERVAL);
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    Some(msg) = outbound_rx.recv() => {
                        if write_cast_frame(&mut writer, &msg).await.is_err() {
                            let _ = events_tx.send(CastEvent::Disconnected("write error".into()));
                            break;
                        }
                    }
                    _ = hb_interval.tick() => {
                        let missed = missed_ref.fetch_add(1, Ordering::Relaxed);
                        if missed >= MAX_MISSED_PONGS {
                            let _ = events_tx.send(CastEvent::Disconnected("heartbeat timeout".into()));
                            break;
                        }
                        let _ = tx_for_heartbeat.try_send(CastMessage::utf8(
                            CASTV2_RECEIVER_ID,
                            CASTV2_NS_HEARTBEAT,
                            json!({ "type": "PING" }).to_string(),
                        ));
                    }
                    frame_res = read_cast_frame(&mut reader) => {
                        let Ok(frame) = frame_res else {
                            let _ = events_tx.send(CastEvent::Disconnected("eof".into()));
                            break;
                        };
                        if frame.namespace == CASTV2_NS_HEARTBEAT
                            && let Some(ref payload) = frame.payload_utf8
                        {
                            if payload.contains("\"PING\"") {
                                let _ = tx_for_heartbeat.try_send(CastMessage::utf8(
                                    CASTV2_RECEIVER_ID,
                                    CASTV2_NS_HEARTBEAT,
                                    json!({ "type": "PONG" }).to_string(),
                                ));
                            } else if payload.contains("\"PONG\"") {
                                missed_ref.store(0, Ordering::Relaxed);
                            }
                        } else if frame.namespace == CASTV2_NS_RECEIVER
                            && let Some(ref payload) = frame.payload_utf8
                            && let Ok(val) = serde_json::from_str::<serde_json::Value>(payload)
                            && let Some(tid) = val["status"]["applications"][0]["transportId"].as_str()
                        {
                            let mut guard = tid_ref.write().await;
                            let newly_acquired = guard.as_deref() != Some(tid);
                            *guard = Some(tid.to_owned());
                            drop(guard);
                            if newly_acquired {
                                let _ = tx_for_heartbeat.try_send(CastMessage::utf8(
                                    tid,
                                    CASTV2_NS_CONNECTION,
                                    json!({ "type": "CONNECT" }).to_string(),
                                ));
                            }
                        } else if frame.namespace == CASTV2_NS_MEDIA
                            && let Some(ref payload) = frame.payload_utf8
                            && let Ok(val) = serde_json::from_str::<serde_json::Value>(payload)
                            && let Some(st) = val["status"].as_array().and_then(|a| a.first())
                        {
                            if let Some(msid) = st["mediaSessionId"].as_i64() {
                                msid_ref.store(msid, Ordering::Relaxed);
                            }
                            if let Some(ct) = st["currentTime"].as_f64() {
                                pos_ref.store((ct * 1000.0).max(0.0) as u64, Ordering::Relaxed);
                            }
                            let pstate = st["playerState"].as_str().unwrap_or("");
                            let idle_reason = st["idleReason"].as_str().unwrap_or("");
                            playing_ref.store(pstate == "PLAYING" || pstate == "BUFFERING", Ordering::Relaxed);
                            if pstate == "IDLE" && idle_reason == "FINISHED" {
                                let _ = events_tx.send(CastEvent::TrackFinished);
                            }
                        }
                    }
                }
            }
        });

        *self.session.lock().await = Some(SessionState {
            outbound_tx,
            transport_id,
            media_session_id,
            is_playing,
            position_ms,
            shutdown_tx: Some(shutdown_tx),
        });

        Ok(())
    }

    async fn load_media(&self, payload: CastMediaPayload) -> CoreResult<()> {
        let tid = self.current_transport_id().await?;
        let req_id = self.next_req_id();
        let images = payload
            .metadata
            .artwork_url
            .as_ref()
            .map(|u| vec![json!({ "url": u })])
            .unwrap_or_default();

        let msg = json!({
            "type": "LOAD",
            "requestId": req_id,
            "autoplay": payload.autoplay,
            "currentTime": (payload.start_position_ms as f64) / 1000.0,
            "media": {
                "contentId": payload.stream_url,
                "streamType": "BUFFERED",
                "contentType": payload.content_type,
                "metadata": {
                    "metadataType": 3,
                    "title": payload.metadata.title,
                    "artist": payload.metadata.artist,
                    "albumName": payload.metadata.album.unwrap_or_default(),
                    "images": images
                }
            }
        });
        self.send_message(CastMessage::utf8(tid, CASTV2_NS_MEDIA, msg.to_string()))
            .await
    }

    async fn play(&self) -> CoreResult<()> {
        let tid = self.current_transport_id().await?;
        let msid = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|s| s.media_session_id.load(Ordering::Relaxed))
            .unwrap_or(1);
        let msg = json!({
            "type": "PLAY",
            "requestId": self.next_req_id(),
            "mediaSessionId": msid
        });
        self.send_message(CastMessage::utf8(tid, CASTV2_NS_MEDIA, msg.to_string()))
            .await
    }

    async fn pause(&self) -> CoreResult<()> {
        let tid = self.current_transport_id().await?;
        let msid = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|s| s.media_session_id.load(Ordering::Relaxed))
            .unwrap_or(1);
        let msg = json!({
            "type": "PAUSE",
            "requestId": self.next_req_id(),
            "mediaSessionId": msid
        });
        self.send_message(CastMessage::utf8(tid, CASTV2_NS_MEDIA, msg.to_string()))
            .await
    }

    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        let tid = self.current_transport_id().await?;
        let msid = self
            .session
            .lock()
            .await
            .as_ref()
            .map(|s| s.media_session_id.load(Ordering::Relaxed))
            .unwrap_or(1);
        let msg = json!({
            "type": "SEEK",
            "requestId": self.next_req_id(),
            "mediaSessionId": msid,
            "currentTime": (position_ms as f64) / 1000.0
        });
        self.send_message(CastMessage::utf8(tid, CASTV2_NS_MEDIA, msg.to_string()))
            .await
    }

    async fn set_volume(&self, volume: f32) -> CoreResult<()> {
        let msg = json!({
            "type": "SET_VOLUME",
            "requestId": self.next_req_id(),
            "volume": { "level": volume.clamp(0.0, 1.0) }
        });
        self.send_message(CastMessage::utf8(
            CASTV2_RECEIVER_ID,
            CASTV2_NS_RECEIVER,
            msg.to_string(),
        ))
        .await
    }

    async fn set_muted(&self, muted: bool) -> CoreResult<()> {
        let msg = json!({
            "type": "SET_VOLUME",
            "requestId": self.next_req_id(),
            "volume": { "muted": muted }
        });
        self.send_message(CastMessage::utf8(
            CASTV2_RECEIVER_ID,
            CASTV2_NS_RECEIVER,
            msg.to_string(),
        ))
        .await
    }

    async fn poll_status(&self) -> CoreResult<Option<(bool, u64)>> {
        let guard = self.session.lock().await;
        Ok(guard.as_ref().map(|s| {
            (
                s.is_playing.load(Ordering::Relaxed),
                s.position_ms.load(Ordering::Relaxed),
            )
        }))
    }

    async fn disconnect(&self) -> CoreResult<()> {
        let mut guard = self.session.lock().await;
        if let Some(mut sess) = guard.take() {
            if let Some(tx) = sess.shutdown_tx.take() {
                let _ = tx.send(());
            }
        }
        Ok(())
    }
}
