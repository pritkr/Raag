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
pub mod pcm_backend;

use resonance_contract::{AudioBackend, AuthBackendStatus, AuthSession, AuthStore};
use resonance_core::innertube::auth::extract_cookie;
use std::path::Path;
use std::sync::Arc;

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
/// - Android: `AndroidKeystoreAuthStore` (`AndroidKeyStore` AES-256-GCM + `0o600` fallback)
/// - Headless fallback: `FileAuthStore` (`0o600` atomic file store)
pub fn create_platform_auth_store(data_dir: &Path) -> Arc<dyn AuthStore> {
    #[cfg(target_os = "android")]
    {
        Arc::new(android::AndroidKeystoreAuthStore::with_android_keystore(
            data_dir,
        ))
    }
    #[cfg(all(
        not(target_os = "android"),
        feature = "desktop-keyring",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    {
        Arc::new(desktop::DesktopAuthStore::with_os_keyring(data_dir))
    }
    #[cfg(not(any(
        target_os = "android",
        all(
            not(target_os = "android"),
            feature = "desktop-keyring",
            any(target_os = "linux", target_os = "macos", target_os = "windows")
        )
    )))]
    {
        Arc::new(resonance_core::innertube::FileAuthStore::new(data_dir))
    }
}

/// Builds an `AuthBackendStatus` snapshot for the current platform and optional session.
pub fn platform_auth_status(session: Option<&AuthSession>) -> AuthBackendStatus {
    let backend = {
        #[cfg(target_os = "android")]
        {
            android::active_android_backend_kind()
        }
        #[cfg(all(
            not(target_os = "android"),
            feature = "desktop-keyring",
            any(target_os = "linux", target_os = "macos", target_os = "windows")
        ))]
        {
            desktop::active_desktop_backend_kind()
        }
        #[cfg(not(any(
            target_os = "android",
            all(
                not(target_os = "android"),
                feature = "desktop-keyring",
                any(target_os = "linux", target_os = "macos", target_os = "windows")
            )
        )))]
        {
            resonance_contract::CredentialBackendKind::EncryptedFallbackFile
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

/// Copies plain UTF-8 text to the native OS clipboard without invoking a shell.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Err("nothing to copy".to_owned());
    }
    #[cfg(target_os = "android")]
    {
        return android::copy_to_android_clipboard(text);
    }
    #[cfg(target_os = "macos")]
    {
        return pipe_to_clipboard_cmd("pbcopy", &[], text);
    }
    #[cfg(target_os = "windows")]
    {
        return pipe_to_clipboard_cmd("clip", &[], text);
    }
    #[cfg(all(
        not(target_os = "android"),
        not(target_os = "macos"),
        not(target_os = "windows")
    ))]
    {
        for (bin, args) in [
            ("wl-copy", &[][..]),
            ("xclip", &["-selection", "clipboard"][..]),
            ("xsel", &["--clipboard", "--input"][..]),
        ] {
            if pipe_to_clipboard_cmd(bin, args, text).is_ok() {
                return Ok(());
            }
        }
        Err("no clipboard utility (wl-copy, xclip, xsel) found".to_owned())
    }
}

#[cfg(not(target_os = "android"))]
fn pipe_to_clipboard_cmd(bin: &str, args: &[&str], text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{bin}: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("{bin} write: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("{bin} wait: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{bin} exited with {status}"))
    }
}
