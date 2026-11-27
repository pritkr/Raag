//! Self-hostable, zero-DB, in-RAM Listen Together Room Relay (`H2.3`).
//!
//! - Exchanges ONLY lightweight JSON control metadata (zero audio proxying).
//! - Supports WebSocket primary (`/v1/rooms/:code/ws`) and HTTP Long-Poll fallback
//!   (`/v1/rooms/:code/poll` + `/v1/rooms/:code/cmd`).
//! - Enforces 256-bit `host_secret`, 32-participant cap, 60 msg/min/peer rate limit,
//!   45s host disconnect auto-promotion, and 30-min idle room eviction.

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use resonance_contract::{
    AuthoritativePlaybackState, QueuePermission, RoomParticipant, RoomRole, RoomWireMessage,
};
use resonance_core::rooms::{
    CreateRoomRequest, CreateRoomResponse, JoinRoomRequest, JoinRoomResponse,
    KickParticipantRequest, MAX_FRAME_BYTES, ReactionRequest, RoomCommandRequest, VoteSkipRequest,
    VoteSkipResponse, generate_host_secret, generate_room_code, is_local_track,
};
use tokio::net::TcpListener;
use tokio::sync::{RwLock, broadcast, watch};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{
    Message, handshake::derive_accept_key, protocol::Role as WsRole,
};

pub const MAX_ROOMS: usize = 10_000;
pub const MAX_PARTICIPANTS: usize = 32;
pub const MAX_MSGS_PER_MIN: usize = 60;

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub host_disconnect_timeout: Duration,
    pub idle_eviction: Duration,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            host_disconnect_timeout: Duration::from_secs(45),
            idle_eviction: Duration::from_secs(30 * 60),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn gen_peer_id() -> String {
    let mut b = [0u8; 8];
    rand::Rng::fill(&mut rand::rng(), &mut b);
    format!("peer-{}", hex::encode(b))
}

struct RoomState {
    room_code: String,
    host_peer_id: String,
    host_secret: Option<String>,
    queue_permission: QueuePermission,
    participants: Vec<RoomParticipant>,
    playback: AuthoritativePlaybackState,
    vote_skip_track_id: Option<String>,
    vote_skips: HashSet<String>,
    last_activity: Instant,
    host_disconnected_at: Option<Instant>,
    rate_windows: HashMap<String, VecDeque<Instant>>,
    tx: broadcast::Sender<RoomWireMessage>,
    rev_tx: watch::Sender<u64>,
}

impl RoomState {
    fn snapshot_msg(&self) -> RoomWireMessage {
        RoomWireMessage::RoomSnapshot {
            room_code: self.room_code.clone(),
            host_peer_id: self.host_peer_id.clone(),
            queue_permission: self.queue_permission,
            participants: self.participants.clone(),
            playback: self.playback.clone(),
        }
    }

    fn check_rate_limit(&mut self, peer_id: &str) -> bool {
        let now = Instant::now();
        let window = self.rate_windows.entry(peer_id.to_owned()).or_default();
        while let Some(&front) = window.front() {
            if now.duration_since(front) > Duration::from_secs(60) {
                window.pop_front();
            } else {
                break;
            }
        }
        if window.len() >= MAX_MSGS_PER_MIN {
            return false;
        }
        window.push_back(now);
        true
    }

    fn broadcast_snapshot(&self) {
        let _ = self.rev_tx.send(self.playback.revision);
        let _ = self.tx.send(self.snapshot_msg());
    }

    fn is_authorized_host(&self, peer_id: &str, provided_secret: Option<&str>) -> bool {
        if peer_id != self.host_peer_id {
            return false;
        }
        match self.host_secret.as_deref() {
            Some(expected) => {
                provided_secret.is_some_and(|p| constant_time_eq(p.as_bytes(), expected.as_bytes()))
            }
            None => provided_secret.is_none(),
        }
    }

    fn promote_oldest_guest(&mut self) -> bool {
        if let Some(next_host) = self
            .participants
            .iter_mut()
            .find(|p| p.peer_id != self.host_peer_id)
        {
            next_host.role = RoomRole::Host;
            let new_host_id = next_host.peer_id.clone();
            for p in &mut self.participants {
                p.role = if p.peer_id == new_host_id {
                    RoomRole::Host
                } else {
                    RoomRole::Guest
                };
            }
            self.host_peer_id = new_host_id;
            // Revoke the departed host's secret so the promoted guest is not locked out.
            self.host_secret = None;
            self.host_disconnected_at = None;
            self.playback.revision = self.playback.revision.saturating_add(1);
            self.broadcast_snapshot();
            true
        } else {
            false
        }
    }
}

struct RelayState {
    rooms: RwLock<HashMap<String, Arc<RwLock<RoomState>>>>,
    reject_ws_502: AtomicBool,
    config: RelayConfig,
}

pub struct RoomRelayHandle {
    addr: SocketAddr,
    state: Arc<RelayState>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl RoomRelayHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Test helper: force WebSocket upgrades to return `502 Bad Gateway` so
    /// clients exercise HTTP Long-Poll fallback.
    pub fn set_reject_ws_502(&self, reject: bool) {
        self.state.reject_ws_502.store(reject, Ordering::SeqCst);
    }

    /// Test/maintenance helper: runs host disconnect promotion & idle eviction pass immediately.
    pub async fn sweep(&self) {
        sweep_rooms(&self.state).await;
    }

    pub fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for RoomRelayHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

async fn sweep_rooms(state: &Arc<RelayState>) {
    let rooms_snapshot: Vec<(String, Arc<RwLock<RoomState>>)> = {
        let map = state.rooms.read().await;
        map.iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    };

    let mut to_evict = Vec::new();
    for (code, room_arc) in rooms_snapshot {
        let mut room = room_arc.write().await;
        if room.last_activity.elapsed() > state.config.idle_eviction {
            to_evict.push(code);
            continue;
        }
        if let Some(disc_at) = room.host_disconnected_at
            && disc_at.elapsed() >= state.config.host_disconnect_timeout
        {
            let host_id = room.host_peer_id.clone();
            room.participants.retain(|p| p.peer_id != host_id);
            if !room.promote_oldest_guest() {
                to_evict.push(code);
            }
        }
    }

    if !to_evict.is_empty() {
        let mut map = state.rooms.write().await;
        for code in to_evict {
            map.remove(&code);
        }
    }
}

pub async fn spawn_relay(
    bind_addr: SocketAddr,
    config: RelayConfig,
) -> Result<RoomRelayHandle, String> {
    let listener = TcpListener::bind(bind_addr)
        .await
        .map_err(|e| format!("failed to bind room-relay: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("failed to read local_addr: {e}"))?;

    let state = Arc::new(RelayState {
        rooms: RwLock::new(HashMap::new()),
        reject_ws_502: AtomicBool::new(false),
        config,
    });

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let bg_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            sweep_rooms(&bg_state).await;
        }
    });

    let srv_state = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue; };
                    let io = TokioIo::new(stream);
                    let st = Arc::clone(&srv_state);
                    tokio::spawn(async move {
                        let svc = service_fn(move |req| handle_http(req, Arc::clone(&st)));
                        let _ = hyper::server::conn::http1::Builder::new()
                            .serve_connection(io, svc)
                            .with_upgrades()
                            .await;
                    });
                }
            }
        }
    });

    Ok(RoomRelayHandle {
        addr,
        state,
        shutdown_tx: Some(shutdown_tx),
    })
}

fn json_resp<T: serde::Serialize>(status: StatusCode, val: &T) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(val).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap()
}

fn text_resp(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain")
        .body(Full::new(Bytes::from(msg.to_owned())))
        .unwrap()
}

fn parse_query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(k, v)| if k == key { Some(v) } else { None })
}

async fn handle_http(
    req: Request<Incoming>,
    state: Arc<RelayState>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let query = req.uri().query().unwrap_or("").to_owned();

    // POST /v1/rooms
    if method == Method::POST && path == "/v1/rooms" {
        if state.rooms.read().await.len() >= MAX_ROOMS {
            return Ok(text_resp(
                StatusCode::SERVICE_UNAVAILABLE,
                "relay capacity reached",
            ));
        }
        let Ok(body) = req.into_body().collect().await else {
            return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid body"));
        };
        let bytes = body.to_bytes();
        if bytes.len() > MAX_FRAME_BYTES {
            return Ok(text_resp(StatusCode::PAYLOAD_TOO_LARGE, "frame too large"));
        }
        let Ok(parsed) = serde_json::from_slice::<CreateRoomRequest>(&bytes) else {
            return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid JSON"));
        };

        let room_code = generate_room_code();
        let host_peer_id = gen_peer_id();
        let host_secret = generate_host_secret();
        let (tx, _) = broadcast::channel(64);
        let (rev_tx, _) = watch::channel(1u64);

        let room = RoomState {
            room_code: room_code.clone(),
            host_peer_id: host_peer_id.clone(),
            host_secret: Some(host_secret.clone()),
            queue_permission: parsed.queue_permission,
            participants: vec![RoomParticipant {
                peer_id: host_peer_id.clone(),
                display_name: parsed.display_name,
                role: RoomRole::Host,
            }],
            playback: AuthoritativePlaybackState {
                revision: 1,
                current_track: None,
                queue: Vec::new(),
                is_playing: false,
                anchor_position_ms: 0,
                anchor_server_ms: now_ms(),
            },
            vote_skip_track_id: None,
            vote_skips: HashSet::new(),
            last_activity: Instant::now(),
            host_disconnected_at: None,
            rate_windows: HashMap::new(),
            tx,
            rev_tx,
        };
        let snapshot = room.snapshot_msg();
        state
            .rooms
            .write()
            .await
            .insert(room_code.clone(), Arc::new(RwLock::new(room)));

        return Ok(json_resp(
            StatusCode::OK,
            &CreateRoomResponse {
                room_code,
                host_peer_id,
                host_secret,
                snapshot,
            },
        ));
    }

    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if segments.len() == 4 && segments[0] == "v1" && segments[1] == "rooms" {
        let code = segments[2];
        let action = segments[3];
        let room_arc = {
            let map = state.rooms.read().await;
            map.get(code).cloned()
        };
        let Some(room_arc) = room_arc else {
            return Ok(text_resp(StatusCode::NOT_FOUND, "room not found"));
        };

        // POST /v1/rooms/:code/join
        if method == Method::POST && action == "join" {
            let Ok(body) = req.into_body().collect().await else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid body"));
            };
            let Ok(parsed) = serde_json::from_slice::<JoinRoomRequest>(&body.to_bytes()) else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid JSON"));
            };
            let mut room = room_arc.write().await;
            if room.participants.len() >= MAX_PARTICIPANTS {
                return Ok(text_resp(StatusCode::TOO_MANY_REQUESTS, "room full"));
            }
            let peer_id = gen_peer_id();
            room.participants.push(RoomParticipant {
                peer_id: peer_id.clone(),
                display_name: parsed.display_name,
                role: RoomRole::Guest,
            });
            room.last_activity = Instant::now();
            room.playback.revision = room.playback.revision.saturating_add(1);
            let snapshot = room.snapshot_msg();
            room.broadcast_snapshot();
            return Ok(json_resp(
                StatusCode::OK,
                &JoinRoomResponse { peer_id, snapshot },
            ));
        }

        // GET /v1/rooms/:code/poll?peer_id=...&since_rev=...&timeout_ms=...
        if method == Method::GET && action == "poll" {
            let since_rev = parse_query_param(&query, "since_rev")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let timeout_ms = parse_query_param(&query, "timeout_ms")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(25_000)
                .min(30_000);

            let mut rev_rx = {
                let room = room_arc.read().await;
                if room.playback.revision > since_rev {
                    return Ok(json_resp(StatusCode::OK, &room.snapshot_msg()));
                }
                room.rev_tx.subscribe()
            };

            let _ = tokio::time::timeout(Duration::from_millis(timeout_ms), async {
                while *rev_rx.borrow() <= since_rev {
                    if rev_rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;

            let room = room_arc.read().await;
            return Ok(json_resp(StatusCode::OK, &room.snapshot_msg()));
        }

        // POST /v1/rooms/:code/cmd
        if method == Method::POST && action == "cmd" {
            let header_secret = req
                .headers()
                .get("x-host-secret")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let Ok(body) = req.into_body().collect().await else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid body"));
            };
            let bytes = body.to_bytes();
            if bytes.len() > MAX_FRAME_BYTES {
                return Ok(text_resp(StatusCode::PAYLOAD_TOO_LARGE, "frame too large"));
            }
            let Ok(cmd) = serde_json::from_slice::<RoomCommandRequest>(&bytes) else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid JSON"));
            };
            let secret = cmd.host_secret.as_deref().or(header_secret.as_deref());
            let mut room = room_arc.write().await;
            if let Err((status, msg)) =
                apply_room_message(&mut room, &cmd.peer_id, secret, cmd.message)
            {
                return Ok(text_resp(status, msg));
            }
            return Ok(json_resp(StatusCode::OK, &room.snapshot_msg()));
        }

        // POST /v1/rooms/:code/vote-skip
        if method == Method::POST && action == "vote-skip" {
            let Ok(body) = req.into_body().collect().await else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid body"));
            };
            let bytes = body.to_bytes();
            if bytes.len() > MAX_FRAME_BYTES {
                return Ok(text_resp(StatusCode::PAYLOAD_TOO_LARGE, "frame too large"));
            }
            let Ok(parsed) = serde_json::from_slice::<VoteSkipRequest>(&bytes) else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid JSON"));
            };
            let mut room = room_arc.write().await;
            if !room
                .participants
                .iter()
                .any(|p| p.peer_id == parsed.peer_id)
            {
                return Ok(text_resp(StatusCode::FORBIDDEN, "unknown peer"));
            }
            if !room.check_rate_limit(&parsed.peer_id) {
                return Ok(text_resp(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate limit exceeded",
                ));
            }
            room.last_activity = Instant::now();
            let current_id = room
                .playback
                .current_track
                .as_ref()
                .map(|t| t.video_id.clone());
            if room.vote_skip_track_id != current_id {
                room.vote_skip_track_id = current_id;
                room.vote_skips.clear();
            }
            room.vote_skips.insert(parsed.peer_id);
            let votes = room.vote_skips.len();
            let threshold = (room.participants.len() / 2) + 1;
            let skipped = votes >= threshold;
            if skipped {
                room.vote_skips.clear();
                let next_track = if room.playback.queue.is_empty() {
                    None
                } else {
                    Some(room.playback.queue.remove(0))
                };
                room.playback.current_track = next_track;
                room.playback.anchor_position_ms = 0;
                room.playback.anchor_server_ms = now_ms();
                room.playback.is_playing = room.playback.current_track.is_some();
                room.playback.revision = room.playback.revision.saturating_add(1);
                room.vote_skip_track_id = room
                    .playback
                    .current_track
                    .as_ref()
                    .map(|t| t.video_id.clone());
                room.broadcast_snapshot();
            }
            return Ok(json_resp(
                StatusCode::OK,
                &VoteSkipResponse {
                    votes,
                    threshold,
                    skipped,
                },
            ));
        }

        // POST /v1/rooms/:code/kick
        if method == Method::POST && action == "kick" {
            let header_secret = req
                .headers()
                .get("x-host-secret")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let Ok(body) = req.into_body().collect().await else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid body"));
            };
            let bytes = body.to_bytes();
            if bytes.len() > MAX_FRAME_BYTES {
                return Ok(text_resp(StatusCode::PAYLOAD_TOO_LARGE, "frame too large"));
            }
            let Ok(parsed) = serde_json::from_slice::<KickParticipantRequest>(&bytes) else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid JSON"));
            };
            let secret = parsed.host_secret.as_deref().or(header_secret.as_deref());
            let mut room = room_arc.write().await;
            if !room.is_authorized_host(&parsed.peer_id, secret) {
                return Ok(text_resp(StatusCode::FORBIDDEN, "host auth required"));
            }
            if parsed.target_peer_id != room.host_peer_id {
                room.participants
                    .retain(|p| p.peer_id != parsed.target_peer_id);
                room.vote_skips.remove(&parsed.target_peer_id);
                room.playback.revision = room.playback.revision.saturating_add(1);
                room.broadcast_snapshot();
            }
            return Ok(json_resp(StatusCode::OK, &room.snapshot_msg()));
        }

        // POST /v1/rooms/:code/reaction
        if method == Method::POST && action == "reaction" {
            let Ok(body) = req.into_body().collect().await else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid body"));
            };
            let bytes = body.to_bytes();
            if bytes.len() > MAX_FRAME_BYTES {
                return Ok(text_resp(StatusCode::PAYLOAD_TOO_LARGE, "frame too large"));
            }
            let Ok(parsed) = serde_json::from_slice::<ReactionRequest>(&bytes) else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid JSON"));
            };
            let mut room = room_arc.write().await;
            if !room.check_rate_limit(&parsed.peer_id) {
                return Ok(text_resp(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate limit exceeded",
                ));
            }
            if parsed.emoji.is_empty() || parsed.emoji.len() > 16 {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "invalid reaction"));
            }
            room.last_activity = Instant::now();
            return Ok(text_resp(StatusCode::OK, "ok"));
        }

        // GET /v1/rooms/:code/ws
        if method == Method::GET && action == "ws" {
            if state.reject_ws_502.load(Ordering::SeqCst) {
                return Ok(text_resp(
                    StatusCode::BAD_GATEWAY,
                    "ws upgrade blocked (502)",
                ));
            }
            let Some(ws_key) = req.headers().get(SEC_WEBSOCKET_KEY).cloned() else {
                return Ok(text_resp(StatusCode::BAD_REQUEST, "missing ws key"));
            };
            let peer_id = parse_query_param(&query, "peer_id")
                .unwrap_or("")
                .to_owned();
            let host_secret = parse_query_param(&query, "host_secret").map(str::to_owned);

            let accept = derive_accept_key(ws_key.as_bytes());
            let room_for_ws = Arc::clone(&room_arc);

            tokio::spawn(async move {
                if let Ok(upgraded) = hyper::upgrade::on(req).await {
                    let ws_stream = WebSocketStream::from_raw_socket(
                        TokioIo::new(upgraded),
                        WsRole::Server,
                        None,
                    )
                    .await;
                    run_ws_peer(ws_stream, room_for_ws, peer_id, host_secret).await;
                }
            });

            return Ok(Response::builder()
                .status(StatusCode::SWITCHING_PROTOCOLS)
                .header(CONNECTION, "Upgrade")
                .header(UPGRADE, "websocket")
                .header(SEC_WEBSOCKET_ACCEPT, accept)
                .body(Full::new(Bytes::new()))
                .unwrap());
        }
    }

    Ok(text_resp(StatusCode::NOT_FOUND, "not found"))
}

fn apply_room_message(
    room: &mut RoomState,
    peer_id: &str,
    host_secret: Option<&str>,
    msg: RoomWireMessage,
) -> Result<Option<RoomWireMessage>, (StatusCode, &'static str)> {
    if !room.check_rate_limit(peer_id) {
        return Err((StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded"));
    }
    room.last_activity = Instant::now();

    match msg {
        RoomWireMessage::ClockPing { client_send_ms } => {
            let t = now_ms();
            Ok(Some(RoomWireMessage::ClockPong {
                client_send_ms,
                server_recv_ms: t,
                server_send_ms: now_ms(),
            }))
        }
        RoomWireMessage::HostSyncState { mut playback } => {
            if !room.is_authorized_host(peer_id, host_secret) {
                return Err((StatusCode::FORBIDDEN, "host auth required"));
            }
            if playback
                .current_track
                .as_ref()
                .is_some_and(|t| is_local_track(&t.video_id))
            {
                playback.current_track = None;
                playback.is_playing = false;
            }
            playback.queue.retain(|t| !is_local_track(&t.video_id));
            playback.revision = room.playback.revision.saturating_add(1);
            playback.anchor_server_ms = now_ms();
            let next_track_id = playback.current_track.as_ref().map(|t| t.video_id.clone());
            if room.vote_skip_track_id != next_track_id {
                room.vote_skip_track_id = next_track_id;
                room.vote_skips.clear();
            }
            room.playback = playback;
            room.broadcast_snapshot();
            Ok(None)
        }
        RoomWireMessage::TransferHost { new_host_peer_id } => {
            if !room.is_authorized_host(peer_id, host_secret) {
                return Err((StatusCode::FORBIDDEN, "host auth required"));
            }
            if new_host_peer_id != room.host_peer_id
                && room
                    .participants
                    .iter()
                    .any(|p| p.peer_id == new_host_peer_id)
            {
                room.host_peer_id = new_host_peer_id;
                room.host_secret = None;
                for p in &mut room.participants {
                    p.role = if p.peer_id == room.host_peer_id {
                        RoomRole::Host
                    } else {
                        RoomRole::Guest
                    };
                }
                room.playback.revision = room.playback.revision.saturating_add(1);
                room.broadcast_snapshot();
            }
            Ok(None)
        }
        RoomWireMessage::GuestSuggestTrack { track } => {
            if room.queue_permission != QueuePermission::GuestsCanAdd
                && peer_id != room.host_peer_id
            {
                return Err((StatusCode::FORBIDDEN, "host-only queue"));
            }
            if is_local_track(&track.video_id) {
                return Err((StatusCode::BAD_REQUEST, "local tracks cannot be synced"));
            }
            room.playback.queue.push(track);
            room.playback.revision = room.playback.revision.saturating_add(1);
            room.broadcast_snapshot();
            Ok(None)
        }
        RoomWireMessage::Leave => {
            room.participants.retain(|p| p.peer_id != peer_id);
            if peer_id == room.host_peer_id {
                room.promote_oldest_guest();
            } else {
                room.playback.revision = room.playback.revision.saturating_add(1);
                room.broadcast_snapshot();
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

async fn run_ws_peer(
    ws: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    room_arc: Arc<RwLock<RoomState>>,
    peer_id: String,
    host_secret: Option<String>,
) {
    let (mut sink, mut stream) = ws.split();
    let (mut rx, initial) = {
        let mut room = room_arc.write().await;
        if peer_id == room.host_peer_id {
            room.host_disconnected_at = None;
        }
        (room.tx.subscribe(), room.snapshot_msg())
    };

    if let Ok(txt) = serde_json::to_string(&initial) {
        let _ = sink.send(Message::Text(txt.into())).await;
    }

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(txt))) => {
                        if txt.len() > MAX_FRAME_BYTES {
                            let err = RoomWireMessage::Error {
                                code: "frame_too_large".to_owned(),
                                message: "frame exceeds 16KB limit".to_owned(),
                            };
                            if let Ok(s) = serde_json::to_string(&err) {
                                let _ = sink.send(Message::Text(s.into())).await;
                            }
                            continue;
                        }
                        if let Ok(msg) = serde_json::from_str::<RoomWireMessage>(&txt) {
                            let reply = {
                                let mut room = room_arc.write().await;
                                apply_room_message(&mut room, &peer_id, host_secret.as_deref(), msg)
                            };
                            match reply {
                                Ok(Some(direct)) => {
                                    if let Ok(s) = serde_json::to_string(&direct) {
                                        let _ = sink.send(Message::Text(s.into())).await;
                                    }
                                }
                                Err((_status, err_msg)) => {
                                    let err = RoomWireMessage::Error {
                                        code: "room_error".to_owned(),
                                        message: err_msg.to_owned(),
                                    };
                                    if let Ok(s) = serde_json::to_string(&err) {
                                        let _ = sink.send(Message::Text(s.into())).await;
                                    }
                                }
                                Ok(None) => {}
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            bcast = rx.recv() => {
                if let Ok(msg) = bcast
                    && let Ok(txt) = serde_json::to_string(&msg)
                    && sink.send(Message::Text(txt.into())).await.is_err()
                {
                    break;
                }
            }
        }
    }

    let mut room = room_arc.write().await;
    if peer_id == room.host_peer_id {
        room.host_disconnected_at = Some(Instant::now());
    }
}
