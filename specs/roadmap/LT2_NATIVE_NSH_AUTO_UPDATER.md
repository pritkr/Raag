# LT-2 — Native NSH Auto-Updater (`nsh/updater` + `nsh update-manifest`), Channel Management & Multi-Channel Release Distribution

> **Wave:** Wave 2 (Long-Term P2) — Parallelizable with LT-1 and LT-3
> **Parallelism Boundary:**
> - **Exclusive Write Scope:** `src/updater/` (new: `mod.rs`, `service.rs`, `channels.rs`), `src/store/updater_slice.rs` (new), `src/screens/settings_updater_card.rs` (new), `nsh.json` (adding `"updater"` block + GitHub/CDN hosts in `capabilities.network.hosts`), `.github/workflows/release.yml` (adding `nsh update-manifest`, signing, and package-manager publishing jobs), `packaging/` (new: Homebrew formula template, Flatpak manifest, AUR `PKGBUILD.tmpl`, Winget manifest template, Obtainium config JSON), `tests/lt2_native_nsh_updater_test.rs` (new).
> - **Forbidden Edits:** `crates/core/src/potoken/**`, `crates/core/src/casting/**`, `platform/android/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/settings.rs` (Wave 3 stitch composes `settings_updater_card()`).

---

## 1. Deep Research: Native NSH Auto-Updater Mechanics

Per the repo's official NSH tooling specification (`.agents/skills/nsh/references/tooling.md` lines 219–253), NSH provides a **first-class native updater (`nsh/updater` / `nsh_core::updater`)** with cryptographic manifest signing and zero third-party updater frameworks (no Sparkle/WinSparkle/Squirrel glue required!):

### Verified NSH Platform Behavior Matrix

| Platform | Native NSH Updater Behavior | Exact User Experience in Resonance |
| :--- | :--- | :--- |
| **macOS (`.app`)** | Checks `latest.json` + `.sig`, downloads archive, verifies Ed25519 + SHA-256, atomically replaces `.app` bundle, supports **`install_and_restart()`** and **`install_on_quit()`** | In-app progress bar → `"Restart to Update"` or `"Install on Quit"` (default non-disruptive choice while music is playing!) |
| **Windows (`.zip`/installer)** | Checks, downloads, verifies signature, replaces & restarts or installs on quit | Same seamless 1-click flow without UAC spam for user-dir installs |
| **Linux (`.tar.gz` / packaged)** | Checks, downloads, verifies, replaces & restarts; gracefully detects distro-managed installs (`/usr/bin` / Flatpak / AUR read-only prefix) and falls back to release link + package-manager hint | Automatic in-place update for `.tar.gz` / AppImage; clean `"Update via Flatpak / apt / pacman"` notice for system packages |
| **Android (APK)** | `check_for_update` returns `can_install = false` + verified release `url` | Opens release page or triggers Obtainium / F-Droid update link with displayed SHA-256 checksum |
| **Dev Run (`nsh run`)** | `check_for_update` resolves `None` (`null`) automatically | Never interrupts local development or headless tests |

---

## 2. Cryptographic Trust & `nsh.json` Configuration Architecture

### 2.1 Why Self-Hosted Signed Manifest + GitHub Release Hosting (Instead of Unsigned GitHub-Only)
`nsh.json` supports two updater modes:
- Simple GitHub mode: `"updater": { "github": "owner/repo" }`
- **Cryptographically Signed Multi-Channel mode (Recommended for Resonance):**
  ```json
  {
    "name": "Raag",
    "id": "dev.resonance.ytmusic",
    "version": "0.2.0",
    "build": 2,
    "updater": {
      "url": "https://github.com/<owner>/<repo>/releases/latest/download/latest.json",
      "publicKey": "<ed25519-base64-public-key-from-nsh-keygen>",
      "channels": {
        "beta": "https://github.com/<owner>/<repo>/releases/download/beta-latest/latest.json"
      }
    }
  }
  ```
- **Why Signed Multi-Channel Mode is Superior:**
  1. Enables **opt-in `Beta` channel** (`checkForUpdate({ channel: "beta" })` — NSH enforces that code may ONLY pass declared channel names from `nsh.json`, preventing runtime URL injection!).
  2. Requires Ed25519 `latest.json.sig` verification via `publicKey` (generated once via `nsh keygen keys/update`, with `keys/update.secret` stored strictly in GitHub Actions Encrypted Secrets and never committed).
  3. Zero extra CDN cost — GitHub Releases serves `latest.json`, `latest.json.sig`, and platform archives globally over HTTPS.
  4. Add `"github.com"`, `"objects.githubusercontent.com"`, and `"release-assets.githubusercontent.com"` to `capabilities.network.hosts` in `nsh.json` so NSH's native capability sandbox allows updater fetches without `ERR_ACCESS_DENIED`.

### 2.2 Exact `crates/contract` Types (Added in Wave 0)

#### `crates/contract/src/updater.rs`
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

impl UpdateChannel {
    pub fn nsh_channel_name(self) -> Option<&'static str> {
        match self {
            Self::Stable => None,       // default top-level updater.url
            Self::Beta => Some("beta"), // declared in nsh.json updater.channels.beta
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Beta => "Beta",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub release_notes: String,
    pub download_url: String,
    pub can_install_in_place: bool,
    pub published_at: Option<String>,
    pub sha256_hex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    UpToDate { checked_at_unix_secs: u64 },
    Available(UpdateInfo),
    Downloading {
        info: UpdateInfo,
        received_bytes: u64,
        total_bytes: Option<u64>,
    },
    ReadyToInstall {
        info: UpdateInfo,
        install_on_quit_scheduled: bool,
    },
    Error(String),
}
```

---

## 3. File-by-File Implementation Plan

### 3.1 Decoupled Native NSH Updater Driver (`src/updater/`)
Because NSH Rust crates use the strict single-source pin synchronized in H0.0 (`Cargo.toml`), isolate all `nsh_core` updater calls behind a clean trait seam `pub trait NativeUpdaterBackend: Send + Sync`:
- **`src/updater/service.rs`:**
  - `NshNativeUpdaterBackend`:
    - Calls NSH's native Rust updater APIs (verified against the resolved `nsh-core` crate source before compiling — never guessed!) for:
      - `check_for_update(channel: UpdateChannel) -> Result<Option<UpdateInfo>, String>`
      - `download_update(on_progress: Box<dyn Fn(u64, Option<u64>) + Send>) -> Result<(), String>`
      - `install_and_restart() -> Result<(), String>`
      - `install_on_quit() -> Result<(), String>`
      - `open_release_url(url: &str) -> Result<(), String>` (Android / system-packaged Linux fallback using NSH `open_url` with `capabilities.openUrl: { "schemes": ["https"] }`!).
  - **Read-Only Package-Manager Detection (Linux):**
    - Before marking `can_install_in_place = true` on Linux, check if running inside Flatpak (`/.flatpak-info` exists), Snap (`SNAP` env var), or non-writable `/opt/Raag` / `/usr/bin` prefix. If read-only, set `can_install_in_place = false` and surface a clean package-manager update instruction instead of failing mid-replace with `EACCES`!
  - **Audio-Aware Update Etiquette:**
    - Silent startup check runs **15 seconds after boot** (zero impact on `< 20 ms` cold start!) and respects a persisted `auto_check_updates: bool` preference (default `true`) + 12-hour minimum check interval + `"Skip this version"` dismissal stored in SQLite `metadata_cache`.
    - Never force-restart while audio is playing: if `PlaybackController` is currently `Playing`, highlight `"Install on Quit"` as the primary action and `"Restart Now"` as secondary!

### 3.2 Reactive Store Slice & Settings UI Card (`src/store/updater_slice.rs` & `src/screens/settings_updater_card.rs`)
- **`UpdaterSlice`:** Holds `channel: UpdateChannel`, `auto_check: bool`, `skipped_version: Option<String>`, `phase: UpdatePhase`, and `current_version: &'static str` (`env!("CARGO_PKG_VERSION")`).
- **`settings_updater_card.rs`:**
  - Composed strictly from `src/ui/controls/` + `theme()` semantic tokens (zero raw hex/RGB literals, `>= 48dp` touch targets, `tabular_nums` for MB progress).
  - Displays:
    - Current version badge (`Raag v0.2.0 • Stable`)
    - Channel toggle (`Stable` / `Beta`) + `"Automatic Update Checks"` switch
    - `"Check for Updates"` button (`test_id("updater-check-btn")`)
    - When `Available`: concise release notes preview + `"Download & Update"` (Desktop) or `"View Release"` (Android / Flatpak) + `"Skip Version"`
    - When `Downloading`: reactive progress bar (`received / total` MB)
    - When `ReadyToInstall`: `"Install on Quit"` (primary when playing) + `"Restart Now"` (`test_id("updater-restart-btn")`)
- **Non-Intrusive Top Banner:** When a background check finds a newer non-skipped version, shows a single dismissible pill/banner linking to Settings (never a blocking modal popup on launch!).

### 3.3 Release CI Pipeline Upgrade (`.github/workflows/release.yml` & `packaging/`)
Upgrade the existing `release` job in `.github/workflows/release.yml`:
1. **Standardized NSH Archive Outputs:**
   - macOS: `Raag-macos-arm64.tar.gz` / `.zip` + `.dmg` (produced via `nsh package --target macos`)
   - Windows: `Raag-windows-x64.zip` + Inno Setup `.exe` (produced via `nsh package --target windows`)
   - Linux: `Raag-linux-x64.tar.gz` + `.deb` (produced via `nsh package --target linux`)
   - Android: `Raag-android-arm64-v8a.apk` (signed & shrunk via `scripts/shrink-android-apk.py`)
2. **Signed Manifest Generation Step (in `release` job after all platform artifacts upload):**
   ```bash
   # Decrypt NSH_UPDATER_PRIVATE_KEY secret into ephemeral ramdisk file (0600)
   nsh update-manifest \
     artifacts/macos-bundle/*.zip \
     artifacts/windows-installer/*.zip \
     artifacts/linux-bundle/*.tar.gz \
     artifacts/android-apk/*.apk \
     --notes RELEASE_NOTES.md \
     --key "$TMP_UPDATE_SECRET_KEY" \
     -o artifacts/manifest/
   # Produces latest.json + latest.json.sig — uploaded alongside binaries in GitHub Release!
   ```
   - For prerelease tags matching `v*-beta*` or `v*-rc*`, publish to the `beta-latest` GitHub Release so Beta channel subscribers receive it while Stable users are untouched.
3. **Ecosystem Package Templates (`packaging/`):**
   - `packaging/homebrew/raag.rb.tmpl` (macOS Homebrew Cask auto-bumped with SHA-256)
   - `packaging/flatpak/dev.resonance.ytmusic.yml` (Linux Flathub sandbox permissions: audio, network, MPRIS D-Bus `org.mpris.MediaPlayer2.resonance`)
   - `packaging/obtainium/raag-obtainium.json` (1-click Android Obtainium import config tracking GitHub Release `arm64-v8a` APKs with verification)
   - `packaging/fdroid/dev.resonance.ytmusic.yml` (reproducible build recipe for F-Droid / IzzyOnDroid)

---

## 4. Security, Privacy & Concurrency Edge Cases

1. **Anti-Downgrade & Semver Validation:** Even if a signed manifest serves an older or equal semver (`0.1.0 <= 0.2.0`), refuse installation unless explicitly forced in dev tests.
2. **Signature Failure Fails Closed:** Any missing `.sig`, invalid Ed25519 signature, or SHA-256 mismatch aborts before extraction and transitions to `UpdatePhase::Error` with zero partial writes left in the app binary directory.
3. **No Concurrent Downloads:** Single-flight guard in `UpdaterSlice` / `UpdaterService` ignores duplicate `"Check"` or `"Download"` clicks while `Checking` or `Downloading`.
4. **URL Allowlist on Android `openUrl`:** Before invoking `open_url(&info.download_url)`, verify scheme is `https` and host matches `github.com` (or configured updater origin)—preventing open-redirect abuse.
5. **Private Key Protection:** CI writes `NSH_UPDATER_PRIVATE_KEY` with `umask 077` into `$RUNNER_TEMP`, never echoes it, and `shred`/`rm -f`s it in an `if: always()` cleanup step.

---

## 5. Offline Verification Test Suite (`tests/lt2_native_nsh_updater_test.rs`)

1. `updater_state_machine_handles_check_download_progress_and_install_on_quit` — uses `MockNativeUpdaterBackend` to walk `Idle -> Checking -> Available -> Downloading(50%) -> ReadyToInstall(install_on_quit=true)` and verifies duplicate checks are coalesced.
2. `updater_respects_skipped_version_and_semver_downgrade_guard` — asserts skipped version is suppressed on background auto-check (but shown on explicit manual check) and older semver is rejected.
3. `updater_linux_readonly_or_flatpak_detection_disables_inplace_install` — verifies read-only/Flatpak prefix flips `can_install_in_place = false` and surfaces package-manager guidance.
4. `updater_channel_switch_passes_only_declared_nsh_channels` — asserts `Stable -> None`, `Beta -> Some("beta")`, and validates `nsh.json` declares `updater.channels.beta` + required `capabilities.network.hosts` + `capabilities.openUrl`.
5. `settings_updater_card_headless_ui_and_zero_raw_colors` — renders `settings_updater_card()` in `nsh_testing::Headless` across `Idle`, `Available`, `Downloading`, and `ReadyToInstall` phases, asserting `>= 48dp` targets and zero raw color literals.

```bash
cargo test --test lt2_native_nsh_updater_test -- --nocapture
cargo test --test phase4_hardening_and_qa
```
