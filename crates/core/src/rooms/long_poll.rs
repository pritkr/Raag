//! HTTP Long-Poll fallback transport for Listen Together (`reqwest` + `rustls`).

use std::time::Duration;

use reqwest::Client;
use resonance_contract::RoomWireMessage;
use url::Url;

use crate::rooms::protocol::{
    CreateRoomRequest, CreateRoomResponse, JoinRoomRequest, JoinRoomResponse, RoomCommandRequest,
    redact_room_secrets,
};

#[derive(Clone)]
pub struct LongPollClient {
    http: Client,
    base_url: Url,
}

impl LongPollClient {
    pub fn new(base_url: Url) -> Result<Self, String> {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        Ok(Self { http, base_url })
    }

    fn endpoint(&self, rel: &str) -> String {
        let base = self.base_url.as_str().trim_end_matches('/');
        let rel = rel.trim_start_matches('/');
        format!("{base}/{rel}")
    }

    pub async fn create_room(&self, req: &CreateRoomRequest) -> Result<CreateRoomResponse, String> {
        let resp = self
            .http
            .post(self.endpoint("v1/rooms"))
            .json(req)
            .send()
            .await
            .map_err(|e| redact_room_secrets(&e.to_string()))?;
        if !resp.status().is_success() {
            return Err(format!("create_room failed with HTTP {}", resp.status()));
        }
        resp.json::<CreateRoomResponse>()
            .await
            .map_err(|e| format!("invalid create_room response: {e}"))
    }

    pub async fn join_room(
        &self,
        room_code: &str,
        req: &JoinRoomRequest,
    ) -> Result<JoinRoomResponse, String> {
        let resp = self
            .http
            .post(self.endpoint(&format!("v1/rooms/{room_code}/join")))
            .json(req)
            .send()
            .await
            .map_err(|e| redact_room_secrets(&e.to_string()))?;
        if !resp.status().is_success() {
            return Err(format!("join_room failed with HTTP {}", resp.status()));
        }
        resp.json::<JoinRoomResponse>()
            .await
            .map_err(|e| format!("invalid join_room response: {e}"))
    }

    pub async fn poll_snapshot(
        &self,
        room_code: &str,
        peer_id: &str,
        since_rev: u64,
        timeout_ms: u64,
    ) -> Result<RoomWireMessage, String> {
        let url = self.endpoint(&format!(
            "v1/rooms/{room_code}/poll?peer_id={peer_id}&since_rev={since_rev}&timeout_ms={timeout_ms}"
        ));
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| redact_room_secrets(&e.to_string()))?;
        if !resp.status().is_success() {
            return Err(format!("long-poll failed with HTTP {}", resp.status()));
        }
        resp.json::<RoomWireMessage>()
            .await
            .map_err(|e| format!("invalid poll snapshot: {e}"))
    }

    pub async fn send_command(
        &self,
        room_code: &str,
        cmd: &RoomCommandRequest,
    ) -> Result<(), String> {
        let resp = self
            .http
            .post(self.endpoint(&format!("v1/rooms/{room_code}/cmd")))
            .json(cmd)
            .send()
            .await
            .map_err(|e| redact_room_secrets(&e.to_string()))?;
        if !resp.status().is_success() {
            return Err(format!("room cmd failed with HTTP {}", resp.status()));
        }
        Ok(())
    }
}
