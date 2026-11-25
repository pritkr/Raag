//! Desktop Auth Store (`keyring` + `0o600` `FileAuthStore` fallback) & Media Lifecycle Bridge:
//! - Persists `AuthSession` in OS Keychain / SecretService / Credential Manager when
//!   `desktop-keyring` is enabled, falling back deterministically to `0o600` `FileAuthStore`.
//! - Streams from the local Rust Loopback HTTP Proxy (`127.0.0.1`)
//! - Publishes Now Playing metadata & receives media keys via `souvlaki`
//!   (Linux MPRIS, Windows SMTC, macOS `MPNowPlayingInfoCenter`) when enabled.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
#[cfg(all(
    feature = "desktop-mpris",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use std::time::Duration;

/// Ensures `<data_dir>/downloads` exists with restrictive `0o700` permissions on Unix.
pub fn ensure_downloads_dir(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = data_dir.join("downloads");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

use async_trait::async_trait;
use resonance_contract::{
    AuthBackendStatus, AuthError, AuthSession, AuthStore, AudioBackend, CoreResult,
    CredentialBackendKind, MediaCommand, NowPlayingMetadata,
};
use resonance_core::innertube::FileAuthStore;
use resonance_core::innertube::auth::{extract_cookie, redact_secrets};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub const KEYRING_SERVICE: &str = "dev.resonance.ytmusic";
pub const KEYRING_USER: &str = "ytmusic_auth_session_v1";

static ACTIVE_DESKTOP_BACKEND: AtomicU8 = AtomicU8::new(1); // 0 = OsKeyring, 1 = EncryptedFallbackFile, 2 = MemoryOnly

fn encode_backend_kind(kind: CredentialBackendKind) -> u8 {
    match kind {
        CredentialBackendKind::OsKeyring => 0,
        CredentialBackendKind::EncryptedFallbackFile => 1,
        CredentialBackendKind::MemoryOnly => 2,
    }
}

fn decode_backend_kind(val: u8) -> CredentialBackendKind {
    match val {
        0 => CredentialBackendKind::OsKeyring,
        2 => CredentialBackendKind::MemoryOnly,
        _ => CredentialBackendKind::EncryptedFallbackFile,
    }
}

pub fn active_desktop_backend_kind() -> CredentialBackendKind {
    decode_backend_kind(ACTIVE_DESKTOP_BACKEND.load(Ordering::SeqCst))
}

/// Trait seam for OS credential storage, allowing deterministic offline tests without D-Bus/Keychain.
pub trait SecretVault: Send + Sync {
    fn get_secret(&self) -> Result<Option<String>, String>;
    fn set_secret(&self, val: &str) -> Result<(), String>;
    fn delete_secret(&self) -> Result<(), String>;
}

/// OS-native keyring vault backed by `keyring` (macOS Keychain, Windows Credential Manager,
/// Linux Secret Service / keyutils).
#[cfg(all(
    feature = "desktop-keyring",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
pub struct OsKeyringVault;

#[cfg(all(
    feature = "desktop-keyring",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
impl OsKeyringVault {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(all(
    feature = "desktop-keyring",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
impl Default for OsKeyringVault {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(
    feature = "desktop-keyring",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
impl SecretVault for OsKeyringVault {
    fn get_secret(&self) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|e| redact_secrets(&e.to_string()))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(redact_secrets(&e.to_string())),
        }
    }

    fn set_secret(&self, val: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|e| redact_secrets(&e.to_string()))?;
        entry
            .set_password(val)
            .map_err(|e| redact_secrets(&e.to_string()))
    }

    fn delete_secret(&self) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .map_err(|e| redact_secrets(&e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(redact_secrets(&e.to_string())),
        }
    }
}

/// Desktop `AuthStore` that prefers `SecretVault` (OS Keychain / SecretService) and falls back
/// deterministically to `0o600` `FileAuthStore` on headless/locked keyring environments.
pub struct DesktopAuthStore {
    vault: Arc<dyn SecretVault>,
    fallback: FileAuthStore,
    backend: Mutex<CredentialBackendKind>,
}

impl DesktopAuthStore {
    pub fn new(vault: Arc<dyn SecretVault>, fallback: FileAuthStore) -> Self {
        ACTIVE_DESKTOP_BACKEND.store(
            encode_backend_kind(CredentialBackendKind::OsKeyring),
            Ordering::SeqCst,
        );
        Self {
            vault,
            fallback,
            backend: Mutex::new(CredentialBackendKind::OsKeyring),
        }
    }

    #[cfg(all(
        feature = "desktop-keyring",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    pub fn with_os_keyring(data_dir: &Path) -> Self {
        Self::new(Arc::new(OsKeyringVault::new()), FileAuthStore::new(data_dir))
    }

    fn set_backend(&self, kind: CredentialBackendKind) {
        *self.backend.lock().unwrap() = kind;
        ACTIVE_DESKTOP_BACKEND.store(encode_backend_kind(kind), Ordering::SeqCst);
    }

    pub fn backend_kind(&self) -> CredentialBackendKind {
        *self.backend.lock().unwrap()
    }

    pub fn status_for_session(&self, session: Option<&AuthSession>) -> AuthBackendStatus {
        AuthBackendStatus {
            backend: self.backend_kind(),
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
}

impl AuthStore for DesktopAuthStore {
    fn load_session(&self) -> Result<Option<AuthSession>, AuthError> {
        match self.vault.get_secret() {
            Ok(Some(json)) => {
                let session = serde_json::from_str::<AuthSession>(&json).map_err(|e| {
                    AuthError::Keystore(format!(
                        "corrupt keyring session: {}",
                        redact_secrets(&e.to_string())
                    ))
                })?;
                self.set_backend(CredentialBackendKind::OsKeyring);
                Ok(Some(session))
            }
            Ok(None) => {
                // If a previous run fell back to file while keyring was unavailable, check fallback.
                match self.fallback.load_session()? {
                    Some(session) => {
                        self.set_backend(CredentialBackendKind::EncryptedFallbackFile);
                        Ok(Some(session))
                    }
                    None => {
                        self.set_backend(CredentialBackendKind::OsKeyring);
                        Ok(None)
                    }
                }
            }
            Err(err) => {
                log::warn!(
                    "OS keyring unavailable ({}); falling back to 0600 file store",
                    redact_secrets(&err)
                );
                self.set_backend(CredentialBackendKind::EncryptedFallbackFile);
                self.fallback.load_session()
            }
        }
    }

    fn save_session(&self, session: &AuthSession) -> Result<(), AuthError> {
        let has_sapisid = extract_cookie(&session.cookie_header, "SAPISID").is_some();
        let has_3papisid = extract_cookie(&session.cookie_header, "__Secure-3PAPISID").is_some();
        if !has_sapisid && !has_3papisid {
            return Err(AuthError::MissingCookie(
                "SAPISID or __Secure-3PAPISID".to_owned(),
            ));
        }

        let json = serde_json::to_string(session)
            .map_err(|e| AuthError::Keystore(redact_secrets(&e.to_string())))?;

        match self.vault.set_secret(&json) {
            Ok(()) => {
                // Remove any stale fallback file so old credentials cannot resurrect.
                let _ = self.fallback.clear_session();
                self.set_backend(CredentialBackendKind::OsKeyring);
                Ok(())
            }
            Err(err) => {
                log::warn!(
                    "OS keyring write failed ({}); falling back to 0600 file store",
                    redact_secrets(&err)
                );
                self.set_backend(CredentialBackendKind::EncryptedFallbackFile);
                self.fallback.save_session(session)
            }
        }
    }

    fn clear_session(&self) -> Result<(), AuthError> {
        // Always wipe BOTH vault and fallback so stale credentials cannot resurrect.
        if let Err(err) = self.vault.delete_secret() {
            log::warn!(
                "OS keyring delete failed during sign-out ({}); continuing fallback cleanup",
                redact_secrets(&err)
            );
        }
        self.fallback.clear_session()
    }
}

/// Pure projection of desktop playback state for `souvlaki` MPRIS/SMTC/NowPlaying updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopPlaybackProjection {
    Stopped,
    Playing { position_ms: u64 },
    Paused { position_ms: u64 },
}

pub fn map_souvlaki_playback(
    playing: bool,
    position_ms: u64,
    has_track: bool,
) -> DesktopPlaybackProjection {
    if !has_track {
        DesktopPlaybackProjection::Stopped
    } else if playing {
        DesktopPlaybackProjection::Playing { position_ms }
    } else {
        DesktopPlaybackProjection::Paused { position_ms }
    }
}

/// Coalesces a relative `SeekBy` delta against current position and optional track duration.
pub fn coalesce_seek_by(
    current_ms: u64,
    forward: bool,
    delta_ms: u64,
    duration_ms: Option<u64>,
) -> u64 {
    let target = if forward {
        current_ms.saturating_add(delta_ms)
    } else {
        current_ms.saturating_sub(delta_ms)
    };
    match duration_ms {
        Some(dur) if dur > 0 => target.min(dur),
        _ => target,
    }
}

pub struct DesktopAudioBridge {
    playing: AtomicBool,
    position_ms: Arc<AtomicU64>,
    duration_ms: Arc<AtomicU64>,
    volume_bits: AtomicU64,
    active_url: Mutex<Option<String>>,
    last_meta: Mutex<Option<NowPlayingMetadata>>,
    cmd_tx: UnboundedSender<MediaCommand>,
    cmd_rx: Mutex<Option<UnboundedReceiver<MediaCommand>>>,
    #[cfg(all(
        feature = "desktop-mpris",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    controls: Mutex<Option<souvlaki::MediaControls>>,
}

impl DesktopAudioBridge {
    pub fn new() -> Self {
        Self::with_hwnd(None)
    }

    pub fn with_hwnd(hwnd: Option<*mut c_void>) -> Self {
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let bridge = Self {
            playing: AtomicBool::new(false),
            position_ms: Arc::new(AtomicU64::new(0)),
            duration_ms: Arc::new(AtomicU64::new(0)),
            volume_bits: AtomicU64::new(1.0_f32.to_bits() as u64),
            active_url: Mutex::new(None),
            last_meta: Mutex::new(None),
            cmd_tx,
            cmd_rx: Mutex::new(Some(cmd_rx)),
            #[cfg(all(
                feature = "desktop-mpris",
                any(target_os = "linux", target_os = "macos", target_os = "windows")
            ))]
            controls: Mutex::new(None),
        };
        bridge.init_system_media_controls(hwnd);
        bridge
    }

    pub fn inject_command(&self, cmd: MediaCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn active_stream_url(&self) -> Option<String> {
        self.active_url.lock().unwrap().clone()
    }

    pub fn last_metadata(&self) -> Option<NowPlayingMetadata> {
        self.last_meta.lock().unwrap().clone()
    }

    #[cfg(all(
        feature = "desktop-mpris",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    fn init_system_media_controls(&self, hwnd: Option<*mut c_void>) {
        use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig, SeekDirection};

        #[cfg(target_os = "windows")]
        let Some(raw_hwnd) = hwnd else {
            log::debug!("Windows SMTC skipped: no HWND provided");
            return;
        };
        #[cfg(not(target_os = "windows"))]
        let raw_hwnd = {
            let _ = hwnd;
            None
        };
        #[cfg(target_os = "windows")]
        let raw_hwnd = Some(raw_hwnd);

        let tx = self.cmd_tx.clone();
        let pos_ref = self.position_ms.clone();
        let dur_ref = self.duration_ms.clone();
        let config = PlatformConfig {
            dbus_name: "resonance",
            display_name: "Resonance",
            hwnd: raw_hwnd,
        };
        match MediaControls::new(config) {
            Ok(mut controls) => {
                if let Err(e) = controls.attach(move |ev| {
                    let mapped = match ev {
                        MediaControlEvent::Play => Some(MediaCommand::Play),
                        MediaControlEvent::Pause => Some(MediaCommand::Pause),
                        MediaControlEvent::Stop => Some(MediaCommand::Stop),
                        MediaControlEvent::Toggle => Some(MediaCommand::TogglePlayPause),
                        MediaControlEvent::Next => Some(MediaCommand::Next),
                        MediaControlEvent::Previous => Some(MediaCommand::Previous),
                        MediaControlEvent::SetPosition(p) => {
                            Some(MediaCommand::SeekMs(p.0.as_millis() as u64))
                        }
                        MediaControlEvent::SeekBy(dir, dur) => {
                            let current = pos_ref.load(Ordering::SeqCst);
                            let duration = dur_ref.load(Ordering::SeqCst);
                            let forward = matches!(dir, SeekDirection::Forward);
                            let target = coalesce_seek_by(
                                current,
                                forward,
                                dur.as_millis() as u64,
                                (duration > 0).then_some(duration),
                            );
                            Some(MediaCommand::SeekMs(target))
                        }
                        _ => None,
                    };
                    if let Some(cmd) = mapped {
                        let _ = tx.send(cmd);
                    }
                }) {
                    log::warn!("failed to attach desktop media controls: {e:?}");
                    return;
                }
                *self.controls.lock().unwrap_or_else(|p| p.into_inner()) = Some(controls);
            }
            Err(e) => {
                log::warn!("desktop media controls unavailable: {e:?}");
            }
        }
    }

    #[cfg(not(all(
        feature = "desktop-mpris",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )))]
    fn init_system_media_controls(&self, _hwnd: Option<*mut c_void>) {
        log::debug!("desktop media controls bridge ready (in-process command bus)");
    }

    #[cfg(all(
        feature = "desktop-mpris",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    fn sync_souvlaki_metadata(&self, meta: &NowPlayingMetadata) {
        use souvlaki::MediaMetadata;
        let duration = (meta.duration_ms > 0).then(|| Duration::from_millis(meta.duration_ms));
        if let Ok(mut guard) = self.controls.lock()
            && let Some(controls) = guard.as_mut()
        {
            let _ = controls.set_metadata(MediaMetadata {
                title: Some(&meta.title),
                artist: Some(&meta.artist),
                album: meta.album.as_deref(),
                cover_url: meta.artwork_url.as_deref(),
                duration,
            });
        }
    }

    #[cfg(not(all(
        feature = "desktop-mpris",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )))]
    fn sync_souvlaki_metadata(&self, _meta: &NowPlayingMetadata) {}

    #[cfg(all(
        feature = "desktop-mpris",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    fn sync_souvlaki_playback(&self) {
        use souvlaki::{MediaPlayback, MediaPosition};
        let has_track = self.last_meta.lock().unwrap().is_some()
            || self.active_url.lock().unwrap().is_some();
        let playing = self.playing.load(Ordering::SeqCst);
        let pos = self.position_ms.load(Ordering::SeqCst);
        let playback = match map_souvlaki_playback(playing, pos, has_track) {
            DesktopPlaybackProjection::Stopped => MediaPlayback::Stopped,
            DesktopPlaybackProjection::Playing { position_ms } => MediaPlayback::Playing {
                progress: Some(MediaPosition(Duration::from_millis(position_ms))),
            },
            DesktopPlaybackProjection::Paused { position_ms } => MediaPlayback::Paused {
                progress: Some(MediaPosition(Duration::from_millis(position_ms))),
            },
        };
        if let Ok(mut guard) = self.controls.lock()
            && let Some(controls) = guard.as_mut()
        {
            let _ = controls.set_playback(playback);
        }
    }

    #[cfg(not(all(
        feature = "desktop-mpris",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )))]
    fn sync_souvlaki_playback(&self) {}
}

impl Default for DesktopAudioBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AudioBackend for DesktopAudioBridge {
    async fn load_and_play(&self, proxy_url: &str, meta: NowPlayingMetadata) -> CoreResult<()> {
        *self.active_url.lock().unwrap() = Some(proxy_url.to_owned());
        self.duration_ms.store(meta.duration_ms, Ordering::SeqCst);
        *self.last_meta.lock().unwrap() = Some(meta.clone());
        self.position_ms.store(0, Ordering::SeqCst);
        self.playing.store(true, Ordering::SeqCst);
        self.sync_souvlaki_metadata(&meta);
        self.sync_souvlaki_playback();
        log::info!(
            "DesktopAudioBridge: streaming via loopback proxy {}",
            redact_secrets(proxy_url)
        );
        Ok(())
    }

    async fn play(&self) -> CoreResult<()> {
        self.playing.store(true, Ordering::SeqCst);
        self.sync_souvlaki_playback();
        Ok(())
    }

    async fn pause(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        self.sync_souvlaki_playback();
        Ok(())
    }

    async fn stop(&self) -> CoreResult<()> {
        self.playing.store(false, Ordering::SeqCst);
        self.position_ms.store(0, Ordering::SeqCst);
        *self.active_url.lock().unwrap() = None;
        *self.last_meta.lock().unwrap() = None;
        self.sync_souvlaki_playback();
        Ok(())
    }

    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()> {
        self.position_ms.store(position_ms, Ordering::SeqCst);
        self.sync_souvlaki_playback();
        Ok(())
    }

    async fn set_volume(&self, volume: f32) -> CoreResult<()> {
        self.volume_bits
            .store(volume.clamp(0.0, 1.0).to_bits() as u64, Ordering::SeqCst);
        Ok(())
    }

    async fn position_ms(&self) -> u64 {
        self.position_ms.load(Ordering::SeqCst)
    }

    async fn update_now_playing(&self, meta: NowPlayingMetadata, playing: bool, position_ms: u64) {
        self.duration_ms.store(meta.duration_ms, Ordering::SeqCst);
        *self.last_meta.lock().unwrap() = Some(meta.clone());
        self.playing.store(playing, Ordering::SeqCst);
        self.position_ms.store(position_ms, Ordering::SeqCst);
        self.sync_souvlaki_metadata(&meta);
        self.sync_souvlaki_playback();
    }

    fn take_command_receiver(&self) -> Option<UnboundedReceiver<MediaCommand>> {
        self.cmd_rx.lock().unwrap().take()
    }
}
