# P7: Native Signed Auto-Updater, Cross-Platform Packaging, Clipboard & UX Polish

> **Wave:** B (Parallel, after Stitch Gate A)
> **Status of Existing Code:** `src/store/updater_slice.rs`, `src/screens/settings_updater_card.rs`, `platform/desktop.rs`, `.github/workflows/release.yml`, and `scripts/shrink-android-apk.py` exist, **but**:
> 1. `check_for_updates_now()` was previously hardcoded to immediately return `"Raag is up to date"` (we just replaced it with an honest unconfigured banner — now build the real Ed25519-signed updater engine in `crates/core/src/updater/`!).
> 2. Native OS clipboard integration (`arboard` on Desktop, `ClipboardManager` JNI on Android) is not wired yet, so `"Share Track Link"` / `"Share Room Code"` currently show the text in a banner instead of writing directly to the OS clipboard.
> 3. Desktop media key & MPRIS2 (Linux) / SMTC (Windows) / MPNowPlayingInfoCenter (macOS) integration in `platform/desktop.rs` needs full artwork + seekbar position sync + release packaging (`DMG`, `AppImage`/`Flatpak`, `MSI`/`NSIS`, shrunk signed `arm64-v8a` APK).
> **Exclusive Owned Paths:** `crates/core/src/updater/*`, `src/store/updater_slice.rs`, `src/screens/settings_updater_card.rs`, `platform/desktop.rs`, `.github/workflows/release.yml`, `scripts/shrink-android-apk.py`, `src/live_engine/handlers/updater.rs`, `tests/p7_updater_packaging_test.rs`.

---

## 1. Deep Technical Research & Architecture

### 1.1 Cryptographically Verified Auto-Updater (`crates/core/src/updater/`)
An auto-updater is a high-value supply-chain target. Never trust raw GitHub Releases HTTP responses without cryptographic signature verification:

1. **Signed Release Manifest (`latest-<channel>.json`):**
   ```json
   {
     "schema_version": 1,
     "channel": "stable",
     "version": "0.2.0",
     "published_at": "2026-12-01T00:00:00Z",
     "release_notes_md": "## What's New\n- Native PCM DSP engine...",
     "release_url": "https://github.com/<owner>/resonance/releases/tag/v0.2.0",
     "artifacts": {
       "macos-aarch64": { "url": "https://...", "sha256": "...", "size_bytes": 18450000 },
       "macos-x86_64":  { "url": "https://...", "sha256": "...", "size_bytes": 19120000 },
       "linux-x86_64-appimage": { "url": "https://...", "sha256": "...", "size_bytes": 17200000 },
       "windows-x86_64-msi":    { "url": "https://...", "sha256": "...", "size_bytes": 16900000 },
       "android-arm64-v8a-apk": { "url": "https://...", "sha256": "...", "size_bytes": 21500000 }
     }
   }
   ```
   Accompanied by detached signature `latest-<channel>.json.sig` (Ed25519 signature over `b"resonance-release-manifest-v1\n" || manifest_bytes`).

2. **Verification & Anti-Downgrade Pipeline:**
   - Enforce **HTTPS-only** download URLs (reject `http://` and non-allowlisted domains except explicit test overrides).
   - Verify Ed25519 manifest signature against pinned release public key (`ed25519-dalek::VerifyingKey::verify_strict`).
   - Compare semver (`semver::Version::parse`) — ignore versions `<= current_version` or matching `skipped_version`.
   - Stream artifact download to `<cache_dir>/updates/<version>.part` with live progress events (`Downloading { bytes, total }`) -> compute SHA-256 -> **abort & delete immediately** on hash mismatch -> atomic rename to `<version>.verified`.

3. **Playback-Safe Staging & Platform Install:**
   - **Never interrupt active audio playback!** If `player_state == Playing`, present `"Update Ready — Install on Quit"` vs `"Restart Now"` (already modeled in `settings_updater_card.rs`!).
   - **Linux AppImage / Binary:** Atomic `rename()` + `chmod 0755` + `execv` relaunch (skip auto-replace if running inside Flatpak/Nix/system read-only `/usr` — show `"Managed by package manager; open release page"` honestly!).
   - **macOS `.app`:** Extract verified `.tar.gz`/`.zip` into staging dir inside `Caches`, verify Apple code signature (`codesign --verify`), swap atomically via small helper trampoline on quit.
   - **Windows:** Launch verified `.msi` / installer or atomic `.exe.old` swap trampoline on exit.
   - **Android:** Save verified `.apk` into `FileProvider` cache dir and fire `Intent.ACTION_VIEW` (`application/vnd.android.package-archive`, `FLAG_GRANT_READ_URI_PERMISSION`) so Android's native package installer validates signing cert continuity!

### 1.2 Native OS Media Controls & Clipboard Integration (`platform/desktop.rs`)
1. **OS Media Transport Controls (`souvlaki` crate, MIT):**
   - Integrates natively with macOS `MPNowPlayingInfoCenter` / `MPRemoteCommandCenter`, Windows SMTC (`SystemMediaTransportControls`), and Linux D-Bus `MPRIS2` (`org.mpris.MediaPlayer2.raag`).
   - Sync metadata (Title, Artist, Album, local cached artwork file path, Duration) on track change; sync playback state & position on Play/Pause/Seek; handle hardware keyboard media keys (`PlayPause`, `Next`, `Previous`, `Stop`, `Seek`) cleanly via `crossbeam_channel` into `AppEngine`!
2. **Native OS Clipboard (`arboard` on Desktop, `ClipboardManager` JNI on Android):**
   - Add `platform::clipboard::copy_text(text: &str) -> Result<(), String>`.
   - Upgrade `"Share Track Link"` (`context_menu.rs`, `sheets.rs`) and `"Share Room Code"` (`shell.rs`) to write to the real OS clipboard and display `"Link copied to clipboard"` ONLY when `copy_text(...)` returns `Ok(())`, falling back honestly to `"Share link: {url}"` if clipboard is unavailable (e.g., headless Wayland/CI).

### 1.3 Automated Cross-Platform Release Packaging (`.github/workflows/release.yml`)
- Matrix builds triggered on signed `v*.*.*` git tags:
  - `macos-aarch64` & `macos-x86_64` (`.dmg` + `.tar.gz` updater archive, universal metadata)
  - `linux-x86_64` (`AppImage` + `.deb` + `.desktop` icon + AppStream metainfo)
  - `windows-x86_64` (`.msi` / `.zip`)
  - `android-arm64-v8a` (NDK release build -> `scripts/shrink-android-apk.py` R8/ProGuard + strip debug symbols + zipalign + apksigner)
- Generate `latest-stable.json` (or `latest-beta.json` for `-rc`/`-beta` tags), sign with CI Ed25519 release secret, and upload all artifacts + manifest + `.sig` to GitHub Releases.

---

## 2. Concrete Implementation Plan (Step-by-Step)

### Step 1: Build `crates/core/src/updater/`
- `manifest.rs`: `ReleaseManifest`, `PlatformArtifact`, Ed25519 signature verifier, semver comparator, `current_platform_key()`.
- `downloader.rs`: HTTPS-only resumable downloader with max size cap (`150 MB`), SHA-256 verification, atomic `.part -> .verified` rename, cancellation token.
- `installer.rs`: Platform-specific install / install-on-quit hooks + read-only install detection (`is_externally_managed()` for Flatpak/Arch/Homebrew/Nix).

### Step 2: Wire `src/live_engine/handlers/updater.rs` & `settings_updater_card.rs`
- Connect `CheckForUpdates`, `DownloadUpdate`, `CancelUpdateDownload`, `SkipVersion`, `InstallOnQuit`, `RestartToInstall` to `crates/core/src/updater/`.
- Support `Stable` and `Beta` channels persisted in SQLite `kv_state`, plus quiet startup background check (at most once every 12 hours when `auto_check == true`).
- Ensure every phase (`Idle`, `Checking`, `UpToDate`, `Available`, `Downloading`, `ReadyToInstall`, `Error`) reflects real verified network/crypto outcomes.

### Step 3: Wire Native OS Media Keys & Clipboard (`platform/desktop.rs`)
- Complete `souvlaki::MediaControls` event loop on Desktop (bridging hardware keys & OS lockscreen seekbar directly to `EngineCommand`).
- Add `platform::clipboard::copy_text` with `arboard` on desktop + Android `ClipboardManager` JNI hook, and connect honest copy/fallback in `context_menu.rs`, `sheets.rs`, and `shell.rs`.

### Step 4: Polish & Visual Accessibility QA
- Audit all keyboard shortcuts (`Space`, `Left`/`Right`, `Up`/`Down`, `N`/`P`, `L`, `M`, `/`, `Escape`) so typing in ANY text field (`search`, `collection-filter`, `cookie-input`, `room-code`, `playlist-title`) suppresses global shortcuts via `is_text_input_focused()`.
- Verify `motion::duration_ms` honors `Reduce Motion` toggle across all transitions.

---

## 3. Security Invariants & Acceptance Tests (`tests/p7_updater_packaging_test.rs`)

1. **Ed25519 Manifest Tamper & Downgrade Test:**
   - Generate ephemeral Ed25519 keypair in test; sign valid manifest `v9.9.9` -> assert `CheckResult::Available`.
   - Flip 1 bit in manifest JSON -> assert `UpdaterError::InvalidSignature`.
   - Sign valid manifest for `v0.0.1` -> assert `CheckResult::UpToDate` (downgrade rejected).
2. **SHA-256 Artifact Tamper Test:** Serve mismatched bytes from local mock server -> assert `.part` file is deleted and `UpdaterPhase::Error` reports checksum mismatch.
3. **HTTPS-Only Enforcement Test:** Manifest pointing to `http://example.com/raag.dmg` -> assert rejected before any network request is made.
4. **Externally Managed Package Detection Test:** Simulate `FLATPAK_ID` env var -> assert installer reports read-only managed mode cleanly instead of failing with `EACCES`.
5. **Clipboard Fallback Honesty Test:** Verify failed clipboard write falls back gracefully without panicking or claiming `"Link copied"`.
