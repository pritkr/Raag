# W3B — Signed Native NSH Auto-Updater (Audio-Aware "Install on Quit", Channels & Package-Manager Awareness)

> **Wave:** Wave 3 (Parallel Agent 3B — Runs concurrently with W3A, W3C after Wave 2.5 Stitch)
> **Exclusive Write Scope:**
> - `src/updater/{mod.rs,manifest.rs,verify.rs,platform_env.rs,manager.rs,ui.rs}` (new)
> - `nsh.json` (updater config section only)
> - `.github/workflows/release.yml` (signed `latest.json` + `latest.json.sig` manifest generation & upload step)
> - `tests/w3b_native_updater_test.rs` (new)
> **Read-Only Dependencies:** `crates/contract/src/updater.rs` (`UpdateChannel`, `UpdateManifest`, `UpdateStatus`), `SettingsV1::update_channel` (W0), `ed25519-dalek` (`Cargo.toml`), `AppPaths` (W0).
> **Stitch Hooks (For Wave 3.5 Integration Agent):**
> - Start background startup check (`30s` delayed, non-blocking) in `src/live_engine.rs`
> - Mount `updater::ui::update_settings_card()` in `src/screens/settings.rs` and non-intrusive `"Update ready"` sidebar/header badge in `src/screens/shell.rs`
> - Invoke `updater::manager::apply_staged_update_on_quit()` during desktop graceful shutdown in `src/main.rs`
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{shell,settings}.rs`, `crates/core/**`, `platform/**`.

---

## 1. Product Taste & Engineering Requirements for a Music Player Updater

Music players have a unique UX constraint that most desktop apps get wrong: **never interrupt active music playback with an intrusive modal or sudden restart!**

1. **Zero Mid-Song Interruptions:** Updates check quietly 30 seconds after launch (and every 24h while idle), download in the background at low priority into `paths.cache_dir/updates/`, verify cryptographically, and offer **"Install on Quit"** (default) or **"Restart Now"** when the user chooses.
2. **Fail-Closed Cryptographic Trust:** Every update manifest (`latest.json` / `beta.json`) is Ed25519-signed (`.sig`) using a compiled-in release public key, and every downloaded artifact is SHA-256 verified against the signed manifest BEFORE staging or invoking any OS installer.
3. **Package-Manager Intelligence (Zero Broken Linux Self-Updates):** On Linux, attempting to overwrite `/usr/bin/resonance` inside Flatpak, Snap, Nix, Arch AUR, or a read-only `/usr` fails with `EACCES`/`EROFS`. Detect externally managed installs upfront and show a helpful `"Update v0.2.0 available via Flatpak / Homebrew / your package manager"` link instead of a broken download button!

---

## 2. Target Architecture

```text
┌────────────────────────────────────────────────────────────────────────────┐
│  UpdateManager (Tokio Background Worker — Zero UI Thread Blocking)         │
│                                                                            │
│  [1] Platform Install-Mode Detection (`platform_env.rs`)                   │
│      • Flatpak (`/.flatpak-info` or `FLATPAK_ID`) → ExternallyManaged      │
│      • Snap (`SNAP` env) → ExternallyManaged                               │
│      • Nix (`/nix/store/...`) → ExternallyManaged                          │
│      • Homebrew (`/opt/homebrew/` or `/usr/local/Cellar/`) → External      │
│      • Linux AppImage (`APPIMAGE` env) / Writable User Install → SelfUpdate│
│      • macOS `.app` bundle (writable) / Windows NSIS/MSI → SelfUpdate      │
│      • Android APK → SignedApkInstallPrompt (`ACTION_INSTALL_PACKAGE`)     │
│                                                                            │
│  [2] Signed Manifest Fetch & Verify (`manifest.rs` + `verify.rs`)          │
│      • Channel URL: Stable (`latest.json`) | Beta (`beta.json`)            │
│      • Ed25519 `verify_strict(manifest_bytes, sig_bytes)`                  │
│      • Strict SemVer comparison (`remote_ver > current_ver`, no downgrade) │
│      • Strict HTTPS URL allowlist (`github.com/<org>/.../releases/...`)    │
│                                                                            │
│  [3] Resumable Background Download & Staging (`manager.rs`)                │
│      • Downloads platform artifact to `cache_dir/updates/<ver>.part`       │
│      • Computes streaming SHA-256; rejects & deletes on 1-bit mismatch     │
│      • Atomically renames to `staged.<ver>.<ext>` + marks `ReadyToInstall` │
│                                                                            │
│  [4] Audio-Aware Install Handoff                                           │
│      • If `is_playing`: defaults to `InstallOnQuit` (never forces restart!)│
│      • User clicks "Restart & Update" → flushes `SettingsV1` & `QueueState`│
│        to SQLite, pauses audio cleanly, delegates to NSH updater / OS hook │
│      • On Android: fires `FileProvider` URI intent for verified APK        │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Detailed Implementation Specification

### 3.1 Signed Manifest Schema (`src/updater/manifest.rs`)
Compatible with NSH's `nsh update-manifest` output format + explicit SHA-256 verification:

```json
{
  "schema_version": 1,
  "version": "0.2.0",
  "channel": "stable",
  "published_at_unix_secs": 1765000000,
  "release_notes_md": "### What's New\n- Native 10-band EQ & true gapless playback\n- Zero-setup WebView sign-in",
  "platforms": {
    "darwin-aarch64": { "url": "https://github.com/.../Raag-0.2.0-arm64.dmg", "sha256": "...", "size_bytes": 18420000 },
    "darwin-x86_64":  { "url": "https://github.com/.../Raag-0.2.0-x64.dmg",   "sha256": "...", "size_bytes": 19110000 },
    "windows-x86_64": { "url": "https://github.com/.../Raag-0.2.0-setup.exe", "sha256": "...", "size_bytes": 16890000 },
    "linux-x86_64":   { "url": "https://github.com/.../Raag-0.2.0.AppImage",  "sha256": "...", "size_bytes": 17540000 },
    "android-arm64":  { "url": "https://github.com/.../Raag-0.2.0-arm64.apk", "sha256": "...", "size_bytes": 14200000 }
  }
}
```

### 3.2 Update State Machine (`crates/contract/src/updater.rs` & `src/updater/manager.rs`)
```text
Idle ──► Checking ──► UpToDate
            │
            ├──► ExternallyManagedAvailable { version, manager_name, release_url }
            │
            ├──► Available { version, notes, size_bytes }
            │       │ (auto-download or user click)
            │       ▼
            │    Downloading { version, bytes_done, total_bytes }
            │       │ (SHA-256 verified)
            │       ▼
            │    ReadyToInstall { version, staged_path, install_on_quit: bool }
            │
            └──► Error(redacted_reason)
```
- **Stale Staging Cleanup:** On every launch, delete any `cache_dir/updates/*` artifacts whose version `<= current_version` so disk space is automatically reclaimed after updating.
- **No Downgrade Attack:** Enforce strict SemVer ordering (`remote > current`) when staying on the same channel; switching `Beta -> Stable` warns the user cleanly before offering a channel switch.

### 3.3 Release Pipeline Integration (`.github/workflows/release.yml` & `nsh.json`)
- Add updater metadata (`endpoints`, `pubkey`, `channel`) to `nsh.json`.
- In `.github/workflows/release.yml` final `publish-manifest` job (runs after macOS, Windows, Linux, and Android artifacts upload):
  1. Compute SHA-256 and byte sizes of all uploaded release binaries.
  2. Emit `latest.json` (or `beta.json` for prerelease tags `v*-beta.*` / `v*-rc.*`).
  3. Sign raw bytes with `RESONANCE_UPDATER_ED25519_PRIVKEY` secret → `latest.json.sig`.
  4. Verify signature in CI before publishing to GitHub Release assets.

### 3.4 Restraint-First Updater UI (`src/updater/ui.rs`)
- **Settings → About & Updates Card:**
  - Shows current version (`Raag v0.1.0`), Release Channel segmented picker (`Stable` / `Beta`, persisted via W0 `SettingsV1`), `"Check for Updates"` button, and collapsible release notes rendered with `nsh_base::text_view().markdown(...)`.
  - When `ReadyToInstall`: displays `"v0.2.0 is ready"` with `"Install on Quit"` toggle (enabled by default while playing!) and `"Restart & Update"` button.
- **Subtle Sidebar / Header Pill:** Only visible when `Available` or `ReadyToInstall` — small calm accent dot + `"Update ready"`, zero intrusive popups over album art!

---

## 4. Deterministic Offline Verification (`tests/w3b_native_updater_test.rs`)

1. `updater_verifies_ed25519_manifest_and_sha256_artifact_end_to_end` — serves signed `latest.json` + `.sig` + binary artifact from local `127.0.0.1:0` HTTP server; verifies staging, SHA-256 check, and `ReadyToInstall` state.
2. `updater_rejects_bad_signature_sha256_mismatch_and_version_downgrade` — tests (a) tampered manifest rejected, (b) 1-bit flipped binary deleted immediately, (c) older version `0.0.9` ignored as `UpToDate`.
3. `platform_env_detector_identifies_flatpak_snap_nix_homebrew_and_appimage` — unit tests environment/path classifier deterministically without touching host OS files.
4. `audio_aware_update_defaults_to_install_on_quit_while_playing` — verifies active playback sets `install_on_quit = true` and never forces immediate restart.
5. `updater_settings_card_renders_cleanly_in_headless_nsh` — headless UI verification across `UpToDate`, `Available`, `ReadyToInstall`, and `ExternallyManagedAvailable` states.

```bash
cargo test --test w3b_native_updater_test -- --nocapture
```
