//! Platform Audio & Media Lifecycle Bridges.
//!
//! - **Primary Streaming Playback:** Native `nsh/audio` (`nsh_core::prelude::audio_player`)
//!   in `platform/nsh_audio.rs`, streaming directly from Resonance's local Rust Loopback HTTP Proxy.
//! - **Platform Lifecycle Shims:** Thin bridges for capabilities outside `nsh/audio`'s scope:
//!   - Android background playback, lock-screen `MediaSession`, Bluetooth AVRCP, and
//!     `ACTION_AUDIO_BECOMING_NOISY` (`platform/android/PlaybackForegroundService.kt`).
//!   - Desktop MPRIS / SMTC / NowPlaying media keys (`souvlaki` in `nsh_audio.rs` / `desktop.rs`).

pub mod android;
pub mod desktop;
pub mod nsh_audio;

use std::sync::Arc;
use resonance_contract::AudioBackend;

/// Creates the appropriate platform `AudioBackend` for the compiled target.
pub fn create_platform_audio_backend() -> Arc<dyn AudioBackend> {
    #[cfg(target_os = "android")]
    {
        Arc::new(android::AndroidMediaBridge::new())
    }
    #[cfg(not(target_os = "android"))]
    {
        Arc::new(nsh_audio::NshAudioBackend::new())
    }
}
