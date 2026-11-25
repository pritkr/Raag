//! AirPlay transport stub/bridge.
//!
//! On macOS, AirPlay 2 multi-room routing is exposed natively via `AVRoutePickerView`
//! in the OS audio stack. Non-Apple platforms focus on Chromecast CastV2 + DLNA/UPnP.

use async_trait::async_trait;
use resonance_contract::{
    CastDevice, CastMediaPayload, CastTransport, CoreError, CoreResult, PlaybackError,
};

#[derive(Debug, Default)]
pub struct AirPlayTransport;

impl AirPlayTransport {
    pub fn is_native_picker_supported() -> bool {
        cfg!(target_os = "macos")
    }
}

#[async_trait]
impl CastTransport for AirPlayTransport {
    async fn connect(&self, _device: &CastDevice) -> CoreResult<()> {
        if cfg!(target_os = "macos") {
            Ok(())
        } else {
            Err(CoreError::Playback(PlaybackError::BackendUnavailable(
                "AirPlay 2 sender requires macOS native AVRoutePickerView".to_owned(),
            )))
        }
    }

    async fn load_media(&self, _payload: CastMediaPayload) -> CoreResult<()> {
        Ok(())
    }

    async fn play(&self) -> CoreResult<()> {
        Ok(())
    }

    async fn pause(&self) -> CoreResult<()> {
        Ok(())
    }

    async fn seek_ms(&self, _position_ms: u64) -> CoreResult<()> {
        Ok(())
    }

    async fn set_volume(&self, _volume: f32) -> CoreResult<()> {
        Ok(())
    }

    async fn set_muted(&self, _muted: bool) -> CoreResult<()> {
        Ok(())
    }

    async fn poll_status(&self) -> CoreResult<Option<(bool, u64)>> {
        Ok(None)
    }

    async fn disconnect(&self) -> CoreResult<()> {
        Ok(())
    }
}
