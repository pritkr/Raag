# SPEC B — Desktop `keyring` Persistence + Sign-In Flow + `souvlaki` Media Controls

> Parallelism boundary: owns `crates/contract/src/desktop_auth.rs`, `platform/desktop.rs`, `platform/mod.rs` (`create_platform_auth_store`), and desktop sign-in UI in `src/screens/settings.rs`. Zero edits to `crates/core/src/innertube/` or `crates/core/src/proxy/`.

## 1. Exact `crates/contract` Signatures

### `crates/contract/src/desktop_auth.rs` (new, re-exported in `lib.rs`)
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialBackendKind {
    OsKeyring,
    EncryptedFallbackFile,
    MemoryOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthBackendStatus {
    pub backend: CredentialBackendKind,
    pub signed_in: bool,
    pub account_name: Option<String>,
    pub has_sapisid: bool,
    pub has_3papisid: bool,
}
```
### `crates/contract/src/error.rs` (additive variant on `AuthError`)
```rust
    #[error("os keyring unavailable ({0}); fell back to 0600 file store")]
    KeyringUnavailable(String),
```
Uses existing frozen `AuthStore` (`load_session`, `save_session`, `clear_session`), `AudioBackend`, `NowPlayingMetadata`, and `MediaCommand` without changing any existing method signatures.

## 2. File-by-File Implementation Plan

- **`Cargo.toml`**:
  - Add `[target.'cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))'.dependencies]`: `keyring = { version = "3.6", optional = true, features = ["apple-native", "windows-native", "sync-secret-service", "crypto-rust"] }`.
  - Add feature `desktop-keyring = ["dep:keyring"]`; keep `desktop-mpris = ["dep:souvlaki"]`; add convenience feature `desktop-native = ["desktop-mpris", "desktop-keyring"]`.
- **`platform/desktop.rs` — `KeyringAuthStore` (`impl AuthStore`)**:
  - Constants: `KEYRING_SERVICE = "dev.resonance.ytmusic"`, `KEYRING_USER = "ytmusic_auth_session_v1"`.
  - Trait seam for offline tests: `pub trait SecretVault: Send + Sync { fn get_secret(&self) -> Result<Option<String>, String>; fn set_secret(&self, val: &str) -> Result<(), String>; fn delete_secret(&self) -> Result<(), String>; }`.
  - `OsKeyringVault` (`#[cfg(feature = "desktop-keyring")]`) wraps `keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)` mapping `keyring::Error::NoEntry` -> `Ok(None)`.
  - `DesktopAuthStore::new(vault: Arc<dyn SecretVault>, fallback: FileAuthStore)` implements `AuthStore`:
    - `load_session`: tries `vault.get_secret()` first; if `Ok(Some(json))` deserializes `AuthSession`; on vault `Err` (headless Linux without D-Bus SecretService), logs single `warn!` (no secrets) and reads `FileAuthStore` (`0o600`).
    - `save_session`: validates `SAPISID` or `__Secure-3PAPISID` present before write; writes JSON to `vault.set_secret` and removes stale fallback file; if vault errors, falls back to `FileAuthStore::save_session` (`0o600` atomic rename).
    - `clear_session`: clears BOTH `vault.delete_secret()` (ignoring `NoEntry`) and `FileAuthStore::clear_session()`.
    - `pub fn backend_kind(&self) -> CredentialBackendKind`.
- **`platform/desktop.rs` — Cross-Platform `souvlaki` Media Controls**:
  - Replace `std::mem::forget(controls)` in `DesktopAudioBridge` with owned `controls: Mutex<Option<MediaControls>>` behind `#[cfg(all(feature = "desktop-mpris", any(target_os = "linux", target_os = "macos", target_os = "windows")))]`.
  - Map `MediaControlEvent::{Play, Pause, Stop, Toggle, Next, Previous, SetPosition, SeekBy}` -> `MediaCommand` into `cmd_tx` (coalesce `SeekBy` using `position_ms.load(SeqCst)`).
  - In `AudioBackend::load_and_play`, `play`, `pause`, `stop`, `seek_ms`, `update_now_playing`:
    - Push `MediaMetadata { title, artist, album, cover_url, duration }` via `controls.set_metadata(...)`.
    - Push `MediaPlayback::Playing { progress: Some(MediaPosition(Duration::from_millis(pos))) }` / `Paused` / `Stopped` via `controls.set_playback(...)`.
  - Add pure helper `pub fn map_souvlaki_playback(playing: bool, position_ms: u64, has_track: bool) -> DesktopPlaybackProjection` so state projection is unit-tested on headless Linux without D-Bus/HWND.
  - Platform guard: on Windows, accept optional `hwnd: Option<*mut c_void>` via `DesktopAudioBridge::with_hwnd` (SMTC requires valid HWND; skip `MediaControls::new` cleanly if `None` on Windows without panicking).
- **`platform/mod.rs`**:
  - Add `pub fn create_platform_auth_store(data_dir: &Path) -> Arc<dyn AuthStore>` returning `DesktopAuthStore` on desktop targets and `FileAuthStore` on Android/fallback.
- **`src/live_engine.rs` & `src/store.rs`**:
  - Replace `FileAuthStore::new(&dir)` in `init_runtime()` with `crate::platform::create_platform_auth_store(&dir)`.
  - Spawn background task pumping `audio.take_command_receiver()` -> `controller.handle_media_command(cmd)` -> `LiveUpdate::PlaybackUpdated(snapshot)` (non-blocking `crossbeam_channel`).
  - Add `LiveUpdate::AuthStatus(AuthBackendStatus)` so UI displays active backend (`OS Keychain` vs `Local 0600 File`) without blocking UI thread.
- **`src/screens/settings.rs`**:
  - Desktop Sign-In card: masked/paste cookie input (`test_id("desktop-cookie-input")`), helper instructions for copying `Cookie` header from `music.youtube.com`, `"Sign In"` button (`>= 48dp`, `test_id("desktop-signin-btn")`), `"Sign Out"` button (`>= 48dp`, `test_id("desktop-signout-btn")`) clearing input buffer immediately on submit/sign-out.
  - Display `AuthBackendStatus` badge using `theme()` tokens only (zero raw color literals, zero cookie echo in UI text).

## 3. Security & Concurrency Edge Cases

- **Zero plaintext secret leaks**: Never log `cookie_header`, `SAPISID`, `__Secure-3PAPISID`, or serialized `AuthSession` JSON; clear UI input `Signal<String>` immediately after `sign_in_with_cookies()` dispatch; redact any keyring error text via `redact_secrets()`.
- **Headless / locked keyring fallback**: Linux CI or SSH sessions without `org.freedesktop.secrets` must never panic; degrade deterministically to `0o600` `FileAuthStore` and report `CredentialBackendKind::EncryptedFallbackFile`.
- **Sign-out completeness**: `clear_session()` MUST wipe both OS keyring entry AND fallback file so stale credentials cannot resurrect if keyring becomes unavailable later.
- **No `souvlaki` UI-thread blocking**: All `MediaControls::set_metadata` / `set_playback` calls happen inside async `AudioBackend` methods on the background Tokio runtime (never on NSH reactive UI thread); lock `controls` briefly and drop before any `.await`.
- **Rapid track switch**: `update_now_playing` overwrites `last_meta` atomically; stale `MediaCommand::SeekMs` clamped against current track duration inside `PlaybackController::seek_ms`.
- **Windows SMTC `hwnd: None`**: Guard `MediaControls::new` failure gracefully instead of panicking or unwrapping.

## 4. Offline Verification Tests (`tests/desktop_keyring_and_mpris.rs`)

Mocks: `InMemorySecretVault` (toggleable `fail_mode: AtomicBool` to simulate locked/missing D-Bus keyring) + `TempDir` + `DesktopAudioBridge`.
1. `keyring_store_saves_loads_and_clears_session_without_touching_fallback_file` — saves valid `AuthSession`, asserts `InMemorySecretVault` holds payload, fallback file absent, `load_session()` round-trips, `clear_session()` wipes vault.
2. `keyring_store_falls_back_to_0600_file_when_os_keyring_unavailable` — sets `fail_mode = true`, saves session, asserts `0o600` file created on Unix, `load_session()` succeeds, `clear_session()` removes fallback file.
3. `keyring_store_rejects_cookie_header_missing_sapisid` — asserts `save_session` returns `Err(AuthError::MissingCookie(_))` and writes nothing to vault or disk.
4. `keyring_store_clear_wipes_both_vault_and_fallback_preventing_resurrection` — seeds both vault + fallback, calls `clear_session()`, flips `fail_mode = true`, asserts `load_session() == Ok(None)`.
5. `desktop_bridge_dispatches_all_media_commands_to_playback_controller` — injects `Play`, `Pause`, `TogglePlayPause`, `Next`, `Previous`, `SeekMs(42_000)` via `DesktopAudioBridge::inject_command`, verifies `take_command_receiver()` delivers exact sequence and `update_now_playing` tracks position/metadata.
6. `desktop_playback_projection_maps_idle_playing_paused_and_seek_clamping` — unit-tests `map_souvlaki_playback` + relative seek underflow/overflow clamping.
7. `desktop_auth_errors_and_status_redact_cookie_secrets` — triggers vault error with secret-tainted string and verifies `redact_secrets` strips `SAPISID`/`Cookie:` before surfacing.

```bash
cargo test --test desktop_keyring_and_mpris -- --nocapture
cargo check --workspace --features desktop-native
```
