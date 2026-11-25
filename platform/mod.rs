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

use std::path::Path;
use std::sync::Arc;
use resonance_contract::{
    AudioBackend, AuthBackendStatus, AuthSession, AuthStore, CredentialBackendKind,
};
use resonance_core::innertube::auth::extract_cookie;

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

/// Creates the appropriate platform `AuthStore` for the compiled target:
/// - Desktop with `desktop-keyring`: `DesktopAuthStore` (OS Keychain / SecretService + `0o600` fallback)
/// - Android / headless fallback: `FileAuthStore` (`0o600` atomic file store)
pub fn create_platform_auth_store(data_dir: &Path) -> Arc<dyn AuthStore> {
    #[cfg(all(
        feature = "desktop-keyring",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    {
        Arc::new(desktop::DesktopAuthStore::with_os_keyring(data_dir))
    }
    #[cfg(not(all(
        feature = "desktop-keyring",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )))]
    {
        Arc::new(resonance_core::innertube::FileAuthStore::new(data_dir))
    }
}

/// Builds an `AuthBackendStatus` snapshot for the current platform and optional session.
pub fn platform_auth_status(session: Option<&AuthSession>) -> AuthBackendStatus {
    let backend = {
        #[cfg(all(
            feature = "desktop-keyring",
            any(target_os = "linux", target_os = "macos", target_os = "windows")
        ))]
        {
            desktop::active_desktop_backend_kind()
        }
        #[cfg(not(all(
            feature = "desktop-keyring",
            any(target_os = "linux", target_os = "macos", target_os = "windows")
        )))]
        {
            CredentialBackendKind::EncryptedFallbackFile
        }
    };
    AuthBackendStatus {
        backend,
        signed_in: session.is_some(),
        account_name: session.and_then(|s| s.account_name.clone()),
        has_sapisid: session
            .and_then(|s| extract_cookie(&s.cookie_header, "SAPISID"))
            .is_some(),
        has_3papisid: session
            .and_then(|s| extract_cookie(&s.cookie_header, "__Secure-3PAPISID"))
            .is_some(),
    }
}
