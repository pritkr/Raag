//! Listen Together wire protocol helpers, Crockford Base32 room codes,
//! TLS URL validation, local-track sync guard, and secret redaction (`H2.3`).

use rand::Rng;
pub use resonance_contract::{
    AuthoritativePlaybackState, QueuePermission, RoomParticipant, RoomRole, RoomTrackRef,
    RoomWireMessage,
};
use serde::{Deserialize, Serialize};
use url::Url;

/// Unambiguous Crockford Base32 alphabet (excludes I, L, O, U).
const CROCKFORD_BASE32: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Generates an 8-character Crockford Base32 room code formatted as `XXXX-XXXX` (40 bits of entropy).
pub fn generate_room_code() -> String {
    let mut rng = rand::rng();
    let mut chars = [b'0'; 8];
    for ch in &mut chars {
        let idx = rng.random_range(0..32);
        *ch = CROCKFORD_BASE32[idx];
    }
    let left = std::str::from_utf8(&chars[..4]).unwrap_or("RAAG");
    let right = std::str::from_utf8(&chars[4..]).unwrap_or("0000");
    format!("{left}-{right}")
}

/// Generates a 256-bit random hex `host_secret` (never logged or persisted to disk).
pub fn generate_host_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex::encode(bytes)
}

/// Returns `true` if `video_id` is a local-only file track (`local:*`) that cannot
/// be streamed by remote peers.
pub fn is_local_track(video_id: &str) -> bool {
    video_id.starts_with("local:")
}

/// Validates that the configured relay URL uses `https://` or `wss://`,
/// allowing `http://` or `ws://` ONLY for loopback (`127.0.0.1` / `localhost`).
pub fn validate_relay_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|e| format!("invalid relay URL: {e}"))?;
    let host = url.host_str().unwrap_or_default();
    let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";
    match url.scheme() {
        "https" | "wss" => Ok(url),
        "http" | "ws" if is_loopback => Ok(url),
        other => Err(format!(
            "relay URL must use https:// or wss:// (got {other}:// for non-loopback host {host})"
        )),
    }
}

/// Converts an `http(s)://` relay base URL into a `ws(s)://` URL.
pub fn to_ws_url(base: &Url, path_and_query: &str) -> Result<Url, String> {
    let mut ws = base.clone();
    match ws.scheme() {
        "https" => ws
            .set_scheme("wss")
            .map_err(|_| "failed to set wss".to_owned())?,
        "http" => ws
            .set_scheme("ws")
            .map_err(|_| "failed to set ws".to_owned())?,
        "wss" | "ws" => {}
        other => return Err(format!("unsupported scheme: {other}")),
    }
    let base_str = ws.as_str().trim_end_matches('/');
    let rel = path_and_query.trim_start_matches('/');
    Url::parse(&format!("{base_str}/{rel}")).map_err(|e| format!("invalid ws URL: {e}"))
}

/// Redacts room secrets (`host_secret`, `x-host-secret`) in addition to standard
/// Resonance auth/cookie/signed-URL redaction.
pub fn redact_room_secrets(input: &str) -> String {
    let base = crate::innertube::auth::redact_secrets(input);
    let mut out = String::with_capacity(base.len());
    for line in base.split_inclusive('\n') {
        let lower = line.to_ascii_lowercase();
        if lower.contains("host_secret") || lower.contains("x-host-secret") {
            let has_nl = line.ends_with('\n');
            out.push_str("[REDACTED_ROOM_SECRET_LINE]");
            if has_nl {
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomRequest {
    pub display_name: String,
    pub queue_permission: QueuePermission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomResponse {
    pub room_code: String,
    pub host_peer_id: String,
    pub host_secret: String,
    pub snapshot: RoomWireMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinRoomRequest {
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinRoomResponse {
    pub peer_id: String,
    pub snapshot: RoomWireMessage,
}

/// Maximum WebSocket / HTTP JSON control frame size (16 KB).
pub const MAX_FRAME_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomCommandRequest {
    pub peer_id: String,
    pub host_secret: Option<String>,
    pub message: RoomWireMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteSkipRequest {
    pub peer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteSkipResponse {
    pub votes: usize,
    pub threshold: usize,
    pub skipped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickParticipantRequest {
    pub peer_id: String,
    pub host_secret: Option<String>,
    pub target_peer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionRequest {
    pub peer_id: String,
    pub emoji: String,
}
