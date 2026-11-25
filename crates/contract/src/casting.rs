//! LAN casting (Chromecast, DLNA/UPnP, AirPlay) contracts.

use std::net::IpAddr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{CoreResult, NowPlayingMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CastProtocol {
    Chromecast,
    DlnaUpnp,
    AirPlayNative,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastDevice {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub protocol: CastProtocol,
    pub address: IpAddr,
    pub port: u16,
    /// UPnP AVTransport control URL (DLNA only).
    pub control_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CastSessionState {
    #[default]
    Disconnected,
    Connecting {
        device_name: String,
    },
    Connected {
        device: CastDevice,
        playing: bool,
        position_ms: u64,
        duration_ms: u64,
        volume_pct: u8,
        muted: bool,
    },
    Error(String),
}

#[derive(Debug, Clone)]
pub struct CastMediaPayload {
    /// Ephemeral IP-pinned LAN relay URL (`http://<lan_ip>:<port>/cast/<token>/<id>.m4a`).
    pub stream_url: String,
    pub content_type: String,
    pub metadata: NowPlayingMetadata,
    pub start_position_ms: u64,
    pub autoplay: bool,
}

#[async_trait]
pub trait CastTransport: Send + Sync {
    async fn connect(&self, device: &CastDevice) -> CoreResult<()>;
    async fn load_media(&self, payload: CastMediaPayload) -> CoreResult<()>;
    async fn play(&self) -> CoreResult<()>;
    async fn pause(&self) -> CoreResult<()>;
    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()>;
    async fn set_volume(&self, volume: f32) -> CoreResult<()>;
    async fn set_muted(&self, muted: bool) -> CoreResult<()>;
    async fn poll_status(&self) -> CoreResult<Option<(bool, u64)>>;
    async fn disconnect(&self) -> CoreResult<()>;
}
