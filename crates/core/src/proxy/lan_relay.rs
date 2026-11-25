//! Session-scoped, IP-pinned, 256-bit bearer-token LAN Relay for Chromecast & DLNA casting.
//!
//! Security gates enforced on every connection & request:
//! 1. Target Subnet Gate: refuses non-RFC1918 / non-link-local target IPs (unless explicit test loopback).
//! 2. Interface Pinning: binds strictly to `<local_lan_ip>:0` (never `0.0.0.0`).
//! 3. Peer IP Pinning: rejects (`403 Forbidden`) any HTTP peer whose IP != paired `CastDevice.address`.
//! 4. 256-Bit Cryptographic Path Token: `/cast/<64-hex-token>/<video_id>.<ext>` verified in constant time.
//! 5. Immediate Teardown: `Drop` / `shutdown()` closes the listener immediately (0 open LAN ports when idle).

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, RANGE,
};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use resonance_contract::{CoreError, CoreResult, PlaybackError};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Returns `true` if `ip` is a private RFC1918 or IPv4 link-local (`169.254/16`) LAN address.
pub fn is_allowed_lan_target(ip: IpAddr, allow_loopback_for_test: bool) -> bool {
    if allow_loopback_for_test && ip.is_loopback() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            // Unique local (fc00::/7) or link-local unicast (fe80::/10)
            let seg0 = v6.segments()[0];
            (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
        }
    }
}

/// Determines the local interface IP routing toward `target` without sending any packets.
pub fn resolve_local_lan_ip(target: IpAddr) -> CoreResult<IpAddr> {
    if target.is_loopback() {
        return Ok(target);
    }
    let bind_unspecified = match target {
        IpAddr::V4(_) => "0.0.0.0:0",
        IpAddr::V6(_) => "[::]:0",
    };
    let sock = UdpSocket::bind(bind_unspecified)
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
    sock.connect(SocketAddr::new(target, 8009))
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
    sock.local_addr()
        .map(|a| a.ip())
        .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))
}

/// Constant-time byte-slice equality check to prevent timing side-channels on the 256-bit token.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Redacts `/cast/<64-hex-token>/` segments from logs and diagnostics.
pub fn redact_cast_secrets(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(idx) = rest.find("/cast/") {
        out.push_str(&rest[..idx + 6]);
        let after = &rest[idx + 6..];
        if let Some(slash) = after.find('/') {
            let candidate = &after[..slash];
            if candidate.len() == 64 && candidate.bytes().all(|b| b.is_ascii_hexdigit()) {
                out.push_str("[REDACTED_CAST_TOKEN]");
                rest = &after[slash..];
                continue;
            }
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

pub fn validate_cast_video_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

struct RelayState {
    allowed_peer_ip: IpAddr,
    token_hex: String,
    loopback_proxy_addr: SocketAddr,
    preferred_itag: Option<u32>,
    http: reqwest::Client,
}

/// Ephemeral, session-scoped LAN HTTP relay forwarding byte-range audio from `LoopbackProxy`.
pub struct LanCastRelay {
    bind_addr: SocketAddr,
    allowed_peer_ip: IpAddr,
    token_hex: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
}

impl LanCastRelay {
    /// Starts an ephemeral `LanCastRelay` bound strictly to `<local_lan_ip>:0` for `target_ip`.
    pub async fn start(
        target_ip: IpAddr,
        loopback_proxy_addr: SocketAddr,
        preferred_itag: Option<u32>,
    ) -> CoreResult<Self> {
        Self::start_inner(target_ip, loopback_proxy_addr, preferred_itag, false).await
    }

    /// Test-only constructor allowing loopback peer `127.0.0.1` / `::1`.
    pub async fn start_for_test(
        target_ip: IpAddr,
        loopback_proxy_addr: SocketAddr,
        preferred_itag: Option<u32>,
    ) -> CoreResult<Self> {
        Self::start_inner(target_ip, loopback_proxy_addr, preferred_itag, true).await
    }

    async fn start_inner(
        target_ip: IpAddr,
        loopback_proxy_addr: SocketAddr,
        preferred_itag: Option<u32>,
        allow_loopback_for_test: bool,
    ) -> CoreResult<Self> {
        if !is_allowed_lan_target(target_ip, allow_loopback_for_test) {
            return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
                format!("refused LanCastRelay for non-RFC1918 target IP {target_ip}"),
            )));
        }

        let local_ip = resolve_local_lan_ip(target_ip)?;
        let listener = TcpListener::bind(SocketAddr::new(local_ip, 0))
            .await
            .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;
        let bind_addr = listener
            .local_addr()
            .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;

        let mut token_bytes = [0u8; 32];
        rand::fill(&mut token_bytes);
        let token_hex = hex::encode(token_bytes);

        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| CoreError::Playback(PlaybackError::BackendUnavailable(e.to_string())))?;

        let state = Arc::new(RelayState {
            allowed_peer_ip: target_ip,
            token_hex: token_hex.clone(),
            loopback_proxy_addr,
            preferred_itag,
            http,
        });

        let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accept_res = listener.accept() => {
                        let Ok((stream, peer_addr)) = accept_res else {
                            break;
                        };
                        let state = Arc::clone(&state);
                        tokio::spawn(async move {
                            let io = TokioIo::new(stream);
                            let svc = service_fn(move |req| {
                                handle_relay_request(req, peer_addr, Arc::clone(&state))
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .serve_connection(io, svc)
                                .await;
                        });
                    }
                }
            }
        });

        Ok(Self {
            bind_addr,
            allowed_peer_ip: target_ip,
            token_hex,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn allowed_peer_ip(&self) -> IpAddr {
        self.allowed_peer_ip
    }

    /// Returns the session-scoped stream URL (`http://<lan_ip>:<port>/cast/<64-hex>/<video_id>.m4a`).
    pub fn stream_url(&self, video_id: &str, ext: &str) -> CoreResult<String> {
        if !validate_cast_video_id(video_id) {
            return Err(CoreError::Playback(PlaybackError::BackendUnavailable(
                format!("invalid track id: {video_id}"),
            )));
        }
        let clean_ext = if ext.is_empty() || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
            "m4a"
        } else {
            ext
        };
        Ok(format!(
            "http://{}/cast/{}/{}.{}",
            self.bind_addr, self.token_hex, video_id, clean_ext
        ))
    }

    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for LanCastRelay {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn handle_relay_request(
    req: Request<Incoming>,
    peer_addr: SocketAddr,
    state: Arc<RelayState>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    // Gate 1: Strict peer IP pinning
    if peer_addr.ip() != state.allowed_peer_ip {
        return Ok(forbidden("peer IP mismatch"));
    }

    if *req.method() != Method::GET && *req.method() != Method::HEAD {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(Full::new(Bytes::new()))
            .unwrap());
    }

    // Gate 2 & 3: Path `/cast/<64-hex-token>/<video_id>.<ext>` + constant-time token check
    let path = req.uri().path();
    let Some(rest) = path.strip_prefix("/cast/") else {
        return Ok(forbidden("invalid path"));
    };
    let Some((token_part, file_part)) = rest.split_once('/') else {
        return Ok(forbidden("invalid path"));
    };
    if !constant_time_eq(token_part.as_bytes(), state.token_hex.as_bytes()) {
        return Ok(forbidden("invalid cast token"));
    }

    let video_id = file_part
        .split_once('.')
        .map(|(id, _)| id)
        .unwrap_or(file_part);
    if !validate_cast_video_id(video_id) {
        return Ok(Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(Full::new(Bytes::from_static(b"invalid video id")))
            .unwrap());
    }

    // Gate 4: Forward Range/HEAD/GET internally to 127.0.0.1 LoopbackProxy
    let upstream_url = match state.preferred_itag {
        Some(itag) => format!(
            "http://{}/stream/{}?itag={}",
            state.loopback_proxy_addr, video_id, itag
        ),
        None => format!("http://{}/stream/{}", state.loopback_proxy_addr, video_id),
    };

    let mut upstream_req = if *req.method() == Method::HEAD {
        state.http.head(&upstream_url)
    } else {
        state.http.get(&upstream_url)
    };
    if let Some(range_hdr) = req.headers().get(RANGE).and_then(|v| v.to_str().ok()) {
        upstream_req = upstream_req.header(RANGE, range_hdr);
    }

    let upstream_resp = match upstream_req.send().await {
        Ok(r) => r,
        Err(_) => {
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Full::new(Bytes::from_static(b"loopback proxy unavailable")))
                .unwrap());
        }
    };

    let status = StatusCode::from_u16(upstream_resp.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut builder = Response::builder()
        .status(status)
        .header(ACCEPT_RANGES, "bytes")
        .header(CACHE_CONTROL, "no-store");

    let content_type = upstream_resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("audio/mp4")
        .to_owned();
    builder = builder.header(CONTENT_TYPE, content_type);

    if let Some(cl) = upstream_resp
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
    {
        builder = builder.header(CONTENT_LENGTH, cl);
    }
    if let Some(cr) = upstream_resp
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
    {
        builder = builder.header(CONTENT_RANGE, cr);
    }

    let body_bytes = if *req.method() == Method::HEAD {
        Bytes::new()
    } else {
        upstream_resp.bytes().await.unwrap_or_default()
    };

    let _ = req.into_body().collect().await;
    Ok(builder.body(Full::new(body_bytes)).unwrap())
}

fn forbidden(msg: &'static str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header(CACHE_CONTROL, "no-store")
        .body(Full::new(Bytes::from_static(msg.as_bytes())))
        .unwrap()
}
