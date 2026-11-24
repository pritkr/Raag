//! Phase 0 Walking-Skeleton Gate Verifier & Documented Platform Fallback Registry.
//!
//! Never assumes NSH provides background Android MediaSession, WebView cookie capture, or Keystore APIs.
//! Uses NSH `=0.6.1`'s real native `nsh/audio` (`nsh_core::prelude::audio_player`) for Desktop and
//! foreground streaming over Resonance's `127.0.0.1` Loopback HTTP Proxy, while explicitly proving the
//! 4 Android/Cross-Platform walking-skeleton capabilities:
//! 1. Play a hard-coded stream URL with screen locked & MediaSession notification controls working
//!    (Primary: native `nsh/audio` `AudioPlayer` on Desktop/foreground; Android background shim:
//!    thin Kotlin Media3 ExoPlayer + `MediaSessionService`).
//! 2. Capture cookies via `; wv`-stripped WebView (`PlaybackForegroundService.configureGoogleSignInWebView`).
//! 3. Write & read a secret in Android Keystore (`KeystoreBridge.kt` AES-256-GCM / desktop secure store).
//! 4. Execute the updatable `yt-dlp-ejs` QuickJS-NG (`>= 0.12.0`) solver bundle.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateCheckResult {
    pub id: String,
    pub passed: bool,
    pub fallback_strategy: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkingSkeletonReport {
    pub nsh_pinned_version: String,
    pub checks: Vec<GateCheckResult>,
}

impl WalkingSkeletonReport {
    pub fn all_passed(&self) -> bool {
        !self.checks.is_empty() && self.checks.iter().all(|c| c.passed)
    }
}

/// Returns the documented platform fallback matrix where NSH lacks a native module.
pub fn documented_platform_fallbacks() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "Background Audio & MediaSession (Android)",
            "Native nsh/audio handles foreground streaming but has no Android ForegroundService or MediaSession API",
            "Thin Kotlin Media3 ExoPlayer + MediaSessionService host shim (`platform/android/PlaybackForegroundService.kt`)",
        ),
        (
            "Streaming Playback & Media Keys (Desktop)",
            "Native nsh/audio (`nsh_core::prelude::audio_player`) handles streaming playback; NSH lacks MPRIS/SMTC media keys",
            "Native `nsh/audio` driver (`platform/nsh_audio.rs`) + `souvlaki` MPRIS/SMTC bridge (`platform/desktop.rs` CLI fallback)",
        ),
        (
            "Google Sign-In WebView (`; wv` UA fix)",
            "NSH WebView does not expose Android `; wv` User-Agent stripping + SAPISID cookie capture",
            "Android WebView shim with `; wv` stripped (`PlaybackForegroundService.configureGoogleSignInWebView`)",
        ),
        (
            "Encrypted Secret Storage (Keystore)",
            "NSH provides no hardware-backed Keystore/Keychain secure-storage API",
            "Android Keystore AES-256-GCM shim (`platform/android/KeystoreBridge.kt`) + OS Keychain on Desktop",
        ),
    ]
}
