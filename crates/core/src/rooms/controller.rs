//! Host-authoritative Listen Together `RoomController` with automatic
//! WebSocket -> HTTP Long-Poll fallback and NTP clock sync (`H2.3`).

use resonance_contract::{
    AuthoritativePlaybackState, QueuePermission, RoomParticipant, RoomRole, RoomTrackRef,
    RoomWireMessage,
};
use tokio::sync::mpsc;
use url::Url;

use crate::rooms::clock_sync::{ClockSyncFilter, DriftAction, DriftGuard};
use crate::rooms::long_poll::LongPollClient;
use crate::rooms::protocol::{
    CreateRoomRequest, JoinRoomRequest, RoomCommandRequest, is_local_track, to_ws_url,
    validate_relay_url,
};
use crate::rooms::ws_client::{WsRoomConnection, WsTransportError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomTransportKind {
    WebSocket,
    HttpLongPoll,
}

impl RoomTransportKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::WebSocket => "WebSocket",
            Self::HttpLongPoll => "HTTP Long-Poll",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomStateSnapshot {
    pub room_code: String,
    pub role: RoomRole,
    pub peer_id: String,
    pub host_peer_id: String,
    pub queue_permission: QueuePermission,
    pub participants: Vec<RoomParticipant>,
    pub playback: AuthoritativePlaybackState,
    pub transport: RoomTransportKind,
    pub clock_offset_ms: i64,
    pub local_track_blocked: bool,
    pub connected: bool,
}

/// Manages a single Listen Together room session (Host or Guest),
/// transparent WS -> HTTP Long-Poll failover, and guest drift correction.
pub struct RoomController {
    relay_url: Url,
    long_poll: LongPollClient,
    ws: Option<WsRoomConnection>,
    transport: RoomTransportKind,
    ws_failures: u8,
    room_code: String,
    peer_id: String,
    host_secret: Option<String>,
    role: RoomRole,
    host_peer_id: String,
    queue_permission: QueuePermission,
    participants: Vec<RoomParticipant>,
    playback: AuthoritativePlaybackState,
    clock_filter: ClockSyncFilter,
    drift_guard: DriftGuard,
    local_track_blocked: bool,
    connected: bool,
}

impl RoomController {
    /// Creates a new room on `relay_url` as Host and connects transport.
    pub async fn create_room(
        relay_url_raw: &str,
        display_name: &str,
        queue_permission: QueuePermission,
    ) -> Result<Self, String> {
        let relay_url = validate_relay_url(relay_url_raw)?;
        let long_poll = LongPollClient::new(relay_url.clone())?;
        let created = long_poll
            .create_room(&CreateRoomRequest {
                display_name: display_name.to_owned(),
                queue_permission,
            })
            .await?;

        let mut ctrl = Self::from_initial_snapshot(
            relay_url,
            long_poll,
            created.room_code,
            created.host_peer_id,
            Some(created.host_secret),
            RoomRole::Host,
            created.snapshot,
        )?;
        ctrl.connect_transport().await?;
        Ok(ctrl)
    }

    /// Joins an existing room by code as Guest and connects transport.
    pub async fn join_room(
        relay_url_raw: &str,
        room_code: &str,
        display_name: &str,
    ) -> Result<Self, String> {
        let relay_url = validate_relay_url(relay_url_raw)?;
        let long_poll = LongPollClient::new(relay_url.clone())?;
        let joined = long_poll
            .join_room(
                room_code,
                &JoinRoomRequest {
                    display_name: display_name.to_owned(),
                },
            )
            .await?;

        let mut ctrl = Self::from_initial_snapshot(
            relay_url,
            long_poll,
            room_code.to_owned(),
            joined.peer_id,
            None,
            RoomRole::Guest,
            joined.snapshot,
        )?;
        ctrl.connect_transport().await?;
        Ok(ctrl)
    }

    fn from_initial_snapshot(
        relay_url: Url,
        long_poll: LongPollClient,
        room_code: String,
        peer_id: String,
        host_secret: Option<String>,
        role: RoomRole,
        snapshot: RoomWireMessage,
    ) -> Result<Self, String> {
        let (host_peer_id, queue_permission, participants, playback) = match snapshot {
            RoomWireMessage::RoomSnapshot {
                host_peer_id,
                queue_permission,
                participants,
                playback,
                ..
            } => (host_peer_id, queue_permission, participants, playback),
            other => {
                return Err(format!("expected RoomSnapshot, got {other:?}"));
            }
        };

        Ok(Self {
            relay_url,
            long_poll,
            ws: None,
            transport: RoomTransportKind::WebSocket,
            ws_failures: 0,
            room_code,
            peer_id,
            host_secret,
            role,
            host_peer_id,
            queue_permission,
            participants,
            playback,
            clock_filter: ClockSyncFilter::new(),
            drift_guard: DriftGuard::new(),
            local_track_blocked: false,
            connected: true,
        })
    }

    /// Connects WebSocket primary, automatically falling back to HTTP Long-Poll
    /// on upgrade rejection (`400/403/502`) or 2 consecutive WS failures.
    pub async fn connect_transport(&mut self) -> Result<RoomTransportKind, String> {
        let mut path = format!("v1/rooms/{}/ws?peer_id={}", self.room_code, self.peer_id);
        if let Some(secret) = &self.host_secret {
            path.push_str(&format!("&host_secret={secret}"));
        }
        let ws_url = to_ws_url(&self.relay_url, &path)?;

        match WsRoomConnection::connect(&ws_url).await {
            Ok(conn) => {
                self.ws = Some(conn);
                self.transport = RoomTransportKind::WebSocket;
                self.ws_failures = 0;
                Ok(RoomTransportKind::WebSocket)
            }
            Err(err) => {
                self.ws_failures = self.ws_failures.saturating_add(1);
                if err.should_fallback_to_long_poll() || self.ws_failures >= 2 {
                    self.ws = None;
                    self.transport = RoomTransportKind::HttpLongPoll;
                    return Ok(RoomTransportKind::HttpLongPoll);
                }
                // Try one immediate retry before falling back.
                match WsRoomConnection::connect(&ws_url).await {
                    Ok(conn) => {
                        self.ws = Some(conn);
                        self.transport = RoomTransportKind::WebSocket;
                        self.ws_failures = 0;
                        Ok(RoomTransportKind::WebSocket)
                    }
                    Err(WsTransportError::HttpRejected(_) | WsTransportError::Connection(_)) => {
                        self.ws = None;
                        self.transport = RoomTransportKind::HttpLongPoll;
                        Ok(RoomTransportKind::HttpLongPoll)
                    }
                    Err(WsTransportError::Protocol(e)) => Err(e),
                }
            }
        }
    }

    pub fn transport(&self) -> RoomTransportKind {
        self.transport
    }

    pub fn snapshot(&self) -> RoomStateSnapshot {
        RoomStateSnapshot {
            room_code: self.room_code.clone(),
            role: self.role,
            peer_id: self.peer_id.clone(),
            host_peer_id: self.host_peer_id.clone(),
            queue_permission: self.queue_permission,
            participants: self.participants.clone(),
            playback: self.playback.clone(),
            transport: self.transport,
            clock_offset_ms: self.clock_filter.offset_ms(),
            local_track_blocked: self.local_track_blocked,
            connected: self.connected,
        }
    }

    /// Host broadcasts canonical playback state; local-only tracks (`local:*`) are
    /// guarded and never broadcast to remote peers.
    pub async fn host_sync_state(
        &mut self,
        mut playback: AuthoritativePlaybackState,
    ) -> Result<(), String> {
        if self.role != RoomRole::Host {
            return Err("only host can broadcast HostSyncState".to_owned());
        }
        if playback
            .current_track
            .as_ref()
            .is_some_and(|t| is_local_track(&t.video_id))
        {
            self.local_track_blocked = true;
            playback.current_track = None;
            playback.is_playing = false;
        } else {
            self.local_track_blocked = false;
        }
        playback.queue.retain(|t| !is_local_track(&t.video_id));

        self.send_wire_message(RoomWireMessage::HostSyncState { playback })
            .await
    }

    pub async fn guest_suggest_track(&mut self, track: RoomTrackRef) -> Result<(), String> {
        if is_local_track(&track.video_id) {
            self.local_track_blocked = true;
            return Err("Local track cannot be synced to remote peers".to_owned());
        }
        self.send_wire_message(RoomWireMessage::GuestSuggestTrack { track })
            .await
    }

    pub async fn send_clock_ping(&mut self, client_send_ms: u64) -> Result<(), String> {
        self.send_wire_message(RoomWireMessage::ClockPing { client_send_ms })
            .await
    }

    pub async fn leave(&mut self) -> Result<(), String> {
        self.connected = false;
        let _ = self.send_wire_message(RoomWireMessage::Leave).await;
        if let Some(ws) = self.ws.as_mut() {
            ws.close().await;
        }
        self.ws = None;
        Ok(())
    }

    pub async fn send_wire_message(&mut self, msg: RoomWireMessage) -> Result<(), String> {
        if self.transport == RoomTransportKind::WebSocket
            && let Some(ws) = self.ws.as_mut()
        {
            if ws.send(&msg).await.is_ok() {
                return Ok(());
            }
            self.ws = None;
            self.transport = RoomTransportKind::HttpLongPoll;
        }

        self.long_poll
            .send_command(
                &self.room_code,
                &RoomCommandRequest {
                    peer_id: self.peer_id.clone(),
                    host_secret: self.host_secret.clone(),
                    message: msg,
                },
            )
            .await
    }

    /// Receives the next wire message over WebSocket or HTTP Long-Poll, updating
    /// internal room state and clock sync filter.
    pub async fn recv_next(
        &mut self,
        local_now_ms: u64,
    ) -> Result<Option<RoomWireMessage>, String> {
        let msg = match self.transport {
            RoomTransportKind::WebSocket => {
                if let Some(ws) = self.ws.as_mut() {
                    match ws.recv().await {
                        Some(Ok(m)) => m,
                        Some(Err(_)) | None => {
                            self.ws = None;
                            self.transport = RoomTransportKind::HttpLongPoll;
                            self.long_poll
                                .poll_snapshot(
                                    &self.room_code,
                                    &self.peer_id,
                                    self.playback.revision,
                                    25_000,
                                )
                                .await?
                        }
                    }
                } else {
                    self.transport = RoomTransportKind::HttpLongPoll;
                    self.long_poll
                        .poll_snapshot(
                            &self.room_code,
                            &self.peer_id,
                            self.playback.revision,
                            25_000,
                        )
                        .await?
                }
            }
            RoomTransportKind::HttpLongPoll => {
                self.long_poll
                    .poll_snapshot(
                        &self.room_code,
                        &self.peer_id,
                        self.playback.revision,
                        25_000,
                    )
                    .await?
            }
        };

        self.apply_wire_message(&msg, local_now_ms);
        Ok(Some(msg))
    }

    pub fn apply_wire_message(&mut self, msg: &RoomWireMessage, local_now_ms: u64) {
        match msg {
            RoomWireMessage::ClockPong {
                client_send_ms,
                server_recv_ms,
                server_send_ms,
            } => {
                self.clock_filter.record_pong(
                    *client_send_ms,
                    *server_recv_ms,
                    *server_send_ms,
                    local_now_ms,
                );
            }
            RoomWireMessage::RoomSnapshot {
                host_peer_id,
                queue_permission,
                participants,
                playback,
                ..
            } => {
                self.host_peer_id = host_peer_id.clone();
                self.role = if host_peer_id == &self.peer_id {
                    RoomRole::Host
                } else {
                    RoomRole::Guest
                };
                self.queue_permission = *queue_permission;
                self.participants = participants.clone();
                self.playback = playback.clone();
            }
            _ => {}
        }
    }

    pub fn evaluate_guest_drift(&mut self, local_pos_ms: u64, local_now_ms: u64) -> DriftAction {
        if self.role == RoomRole::Host {
            return DriftAction::Ignore;
        }
        self.drift_guard.evaluate(
            &self.playback,
            local_pos_ms,
            local_now_ms,
            self.clock_filter.offset_ms(),
        )
    }
}

/// Wave 3 Stitch hook: forwards room snapshots over a channel for `live_engine.rs`.
pub fn wire_h2_room_events(
    tx: mpsc::UnboundedSender<RoomStateSnapshot>,
    controller: &RoomController,
) {
    let _ = tx.send(controller.snapshot());
}
