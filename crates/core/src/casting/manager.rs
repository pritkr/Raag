//! Seamless Audio Handover (`CastManager`) between local playback and LAN Cast targets.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use resonance_contract::{
    CastDevice, CastMediaPayload, CastProtocol, CastSessionState, CastTransport, CoreResult,
    NowPlayingMetadata,
};
use tokio::sync::RwLock;

use crate::casting::airplay::AirPlayTransport;
use crate::casting::chromecast::ChromecastTransport;
use crate::casting::dlna::DlnaTransport;
use crate::proxy::lan_relay::LanCastRelay;

/// Preferred AAC `.m4a` itag (140) for universal Chromecast/DLNA compatibility.
pub const CAST_PREFERRED_AAC_ITAG: u32 = 140;

/// Hook for pausing/resuming local audio (`nsh/audio` or Android ExoPlayer) during cast handover.
#[async_trait]
pub trait CastPlaybackDelegate: Send + Sync {
    async fn pause_local(&self) -> CoreResult<()>;
    async fn resume_local_at(&self, position_ms: u64) -> CoreResult<()>;
}

struct ActiveCast {
    device: CastDevice,
    transport: Arc<dyn CastTransport>,
    relay: LanCastRelay,
    last_position_ms: u64,
    duration_ms: u64,
    volume_pct: u8,
    muted: bool,
    playing: bool,
}

pub struct CastManager {
    loopback_proxy_addr: SocketAddr,
    delegate: Arc<dyn CastPlaybackDelegate>,
    active: RwLock<Option<ActiveCast>>,
    allow_loopback_relay_for_test: bool,
}

impl CastManager {
    pub fn new(loopback_proxy_addr: SocketAddr, delegate: Arc<dyn CastPlaybackDelegate>) -> Self {
        Self {
            loopback_proxy_addr,
            delegate,
            active: RwLock::new(None),
            allow_loopback_relay_for_test: false,
        }
    }

    pub fn allow_loopback_for_tests(mut self) -> Self {
        self.allow_loopback_relay_for_test = true;
        self
    }

    fn build_transport(protocol: CastProtocol) -> Arc<dyn CastTransport> {
        match protocol {
            CastProtocol::Chromecast => Arc::new(ChromecastTransport::new()),
            CastProtocol::DlnaUpnp => Arc::new(DlnaTransport::new()),
            CastProtocol::AirPlayNative => Arc::new(AirPlayTransport),
        }
    }

    /// Connects to `device`, pauses local playback, starts an ephemeral `LanCastRelay`,
    /// and loads `video_id` starting at `current_position_ms`.
    pub async fn connect_and_handover(
        &self,
        device: CastDevice,
        video_id: &str,
        metadata: NowPlayingMetadata,
        current_position_ms: u64,
        duration_ms: u64,
    ) -> CoreResult<CastSessionState> {
        self.connect_with_transport(
            device.clone(),
            Self::build_transport(device.protocol),
            video_id,
            metadata,
            current_position_ms,
            duration_ms,
        )
        .await
    }

    /// Testable handover entry point accepting a custom [`CastTransport`] (e.g. mock server).
    pub async fn connect_with_transport(
        &self,
        device: CastDevice,
        transport: Arc<dyn CastTransport>,
        video_id: &str,
        metadata: NowPlayingMetadata,
        current_position_ms: u64,
        duration_ms: u64,
    ) -> CoreResult<CastSessionState> {
        // Tear down any prior session first
        let _ = self.disconnect().await;

        let relay = if self.allow_loopback_relay_for_test {
            LanCastRelay::start_for_test(
                device.address,
                self.loopback_proxy_addr,
                Some(CAST_PREFERRED_AAC_ITAG),
            )
            .await?
        } else {
            LanCastRelay::start(
                device.address,
                self.loopback_proxy_addr,
                Some(CAST_PREFERRED_AAC_ITAG),
            )
            .await?
        };

        if let Err(err) = transport.connect(&device).await {
            relay.shutdown();
            return Err(err);
        }
        if let Err(err) = self.delegate.pause_local().await {
            let _ = transport.disconnect().await;
            relay.shutdown();
            return Err(err);
        }

        let stream_url = match relay.stream_url(video_id, "m4a") {
            Ok(url) => url,
            Err(err) => {
                let _ = transport.disconnect().await;
                relay.shutdown();
                let _ = self.delegate.resume_local_at(current_position_ms).await;
                return Err(err);
            }
        };
        if let Err(err) = transport
            .load_media(CastMediaPayload {
                stream_url,
                content_type: "audio/mp4".to_owned(),
                metadata,
                start_position_ms: current_position_ms,
                autoplay: true,
            })
            .await
        {
            let _ = transport.disconnect().await;
            relay.shutdown();
            let _ = self.delegate.resume_local_at(current_position_ms).await;
            return Err(err);
        }

        let state = CastSessionState::Connected {
            device: device.clone(),
            playing: true,
            position_ms: current_position_ms,
            duration_ms,
            volume_pct: 100,
            muted: false,
        };

        *self.active.write().await = Some(ActiveCast {
            device,
            transport,
            relay,
            last_position_ms: current_position_ms,
            duration_ms,
            volume_pct: 100,
            muted: false,
            playing: true,
        });

        Ok(state)
    }

    pub async fn play(&self) -> CoreResult<()> {
        let mut guard = self.active.write().await;
        if let Some(ref mut cast) = *guard {
            cast.transport.play().await?;
            cast.playing = true;
        }
        Ok(())
    }

    pub async fn pause(&self) -> CoreResult<()> {
        let mut guard = self.active.write().await;
        if let Some(ref mut cast) = *guard {
            cast.transport.pause().await?;
            cast.playing = false;
        }
        Ok(())
    }

    pub async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        let mut guard = self.active.write().await;
        if let Some(ref mut cast) = *guard {
            cast.transport.seek_ms(position_ms).await?;
            cast.last_position_ms = position_ms;
        }
        Ok(())
    }

    pub async fn set_volume(&self, volume: f32) -> CoreResult<()> {
        let mut guard = self.active.write().await;
        if let Some(ref mut cast) = *guard {
            cast.transport.set_volume(volume).await?;
            cast.volume_pct = (volume.clamp(0.0, 1.0) * 100.0).round() as u8;
        }
        Ok(())
    }

    pub async fn poll_and_sync(&self) -> CoreResult<CastSessionState> {
        let mut guard = self.active.write().await;
        let Some(ref mut cast) = *guard else {
            return Ok(CastSessionState::Disconnected);
        };
        if let Some((playing, pos_ms)) = cast.transport.poll_status().await? {
            cast.playing = playing;
            cast.last_position_ms = pos_ms;
        }
        Ok(CastSessionState::Connected {
            device: cast.device.clone(),
            playing: cast.playing,
            position_ms: cast.last_position_ms,
            duration_ms: cast.duration_ms,
            volume_pct: cast.volume_pct,
            muted: cast.muted,
        })
    }

    /// Disconnects the active cast session, tears down `LanCastRelay`, and resumes local playback
    /// from the cast device's last known `position_ms`.
    pub async fn disconnect(&self) -> CoreResult<CastSessionState> {
        let taken = self.active.write().await.take();
        if let Some(mut cast) = taken {
            if let Ok(Some((_, pos))) = cast.transport.poll_status().await {
                cast.last_position_ms = pos;
            }
            let _ = cast.transport.disconnect().await;
            cast.relay.shutdown();
            self.delegate.resume_local_at(cast.last_position_ms).await?;
        }
        Ok(CastSessionState::Disconnected)
    }

    pub async fn state(&self) -> CastSessionState {
        let guard = self.active.read().await;
        match guard.as_ref() {
            None => CastSessionState::Disconnected,
            Some(cast) => CastSessionState::Connected {
                device: cast.device.clone(),
                playing: cast.playing,
                position_ms: cast.last_position_ms,
                duration_ms: cast.duration_ms,
                volume_pct: cast.volume_pct,
                muted: cast.muted,
            },
        }
    }
}
