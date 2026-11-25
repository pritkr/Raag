//! Listen Together: Host-authoritative synced rooms (`H2.3`).
//!
//! - Dual-transport: WebSocket primary (`tokio-tungstenite` + `rustls`) with
//!   automatic HTTP Long-Poll fallback (`reqwest` + `rustls`).
//! - NTP-style median clock sync filter + 400ms hysteresis drift guard.
//! - Zero audio proxying: exchanges only lightweight JSON control metadata.

pub mod clock_sync;
pub mod controller;
pub mod long_poll;
pub mod protocol;
pub mod ws_client;

pub use clock_sync::{
    ClockSyncFilter, DRIFT_DEADBAND_MS, DriftAction, DriftGuard, SEEK_COOLDOWN_MS,
};
pub use controller::{RoomController, RoomStateSnapshot, RoomTransportKind, wire_h2_room_events};
pub use long_poll::LongPollClient;
pub use protocol::{
    CreateRoomRequest, CreateRoomResponse, JoinRoomRequest, JoinRoomResponse, RoomCommandRequest,
    generate_host_secret, generate_room_code, is_local_track, redact_room_secrets,
    validate_relay_url,
};
pub use ws_client::{WsRoomConnection, WsTransportError};
