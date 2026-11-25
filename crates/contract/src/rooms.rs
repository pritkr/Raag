//! Listen Together room wire protocol and role contracts.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomRole {
    Host,
    Guest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuePermission {
    HostOnly,
    GuestsCanAdd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomParticipant {
    pub peer_id: String,
    pub display_name: String,
    pub role: RoomRole,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTrackRef {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: u64,
    pub artwork_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoritativePlaybackState {
    pub revision: u64,
    pub current_track: Option<RoomTrackRef>,
    pub queue: Vec<RoomTrackRef>,
    pub is_playing: bool,
    /// Playback position at `anchor_server_ms` (in milliseconds).
    pub anchor_position_ms: u64,
    /// Monotonic relay server UTC epoch milliseconds when anchor was stamped.
    pub anchor_server_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomWireMessage {
    ClockPing {
        client_send_ms: u64,
    },
    ClockPong {
        client_send_ms: u64,
        server_recv_ms: u64,
        server_send_ms: u64,
    },
    RoomSnapshot {
        room_code: String,
        host_peer_id: String,
        queue_permission: QueuePermission,
        participants: Vec<RoomParticipant>,
        playback: AuthoritativePlaybackState,
    },
    HostSyncState {
        playback: AuthoritativePlaybackState,
    },
    TransferHost {
        new_host_peer_id: String,
    },
    GuestSuggestTrack {
        track: RoomTrackRef,
    },
    Leave,
    Error {
        code: String,
        message: String,
    },
}
