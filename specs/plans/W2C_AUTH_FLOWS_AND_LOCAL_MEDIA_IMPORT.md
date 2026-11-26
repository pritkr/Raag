# W2C — Frictionless Universal Authentication (WebView + RFC 8628 TV OAuth) & Local Audio Library Import

> **Wave:** Wave 2 (Parallel Agent 2C — Runs concurrently with W2A, W2B, W2D after Wave 1.5 Stitch)
> **Exclusive Write Scope:**
> - `crates/core/src/auth_flows/{mod.rs,orchestrator.rs,tv_oauth.rs,webview_cookie.rs}` (new)
> - `src/screens/auth_sign_in_sheet.rs` (new)
> - `crates/core/src/local_media/{mod.rs,scanner.rs,db.rs,artwork.rs}` (new)
> - `crates/core/src/proxy/local_route.rs` (new confined `/local/<track_id>` handler module)
> - `src/screens/local_files.rs`, `src/store/local_media_slice.rs` (new)
> - `tests/w2c_auth_and_local_media_test.rs` (new)
> **Read-Only Dependencies:** `lofty = "0.22"` (existing), `ArtworkCache`, `AppPaths`, `SettingsV1::local_music_dirs`, `crates/contract/src/local_media.rs`, `WebViewSignInActivity.kt` (W1E).
> **Stitch Hooks (For Wave 2.5 Integration Agent):**
> - Mount `/local/<track_id>` route in `crates/core/src/proxy/mod.rs` delegating to `local_route::serve_local_track(...)`
> - Replace manual cookie textarea in `src/screens/settings.rs` with `auth_sign_in_sheet` primary flow (hiding raw cookie paste inside collapsed `"Advanced"` section)
> - Add `"Local Files"` filter tab in `src/screens/library.rs` mounting `local_files::local_files_view()` and expose `LocalMediaCommands` in `src/live_engine.rs`
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{shell,settings,library}.rs`, `crates/core/src/proxy/mod.rs`, `platform/**`.

---

## 1. Ground-Truth Problems Fixed

1. **Hostile Sign-In UX (`src/screens/settings.rs:134`):** Telling normal listeners to open Chrome DevTools → Network tab and copy raw `SAPISID` / `__Secure-3PAPISID` Cookie headers is a complete usability blocker.
2. **No Local Audio Support:** Users cannot play their own offline FLAC/MP3/M4A/Opus/WAV folders alongside YouTube Music tracks in a unified queue and library—even though `lofty = "0.22"` and `LoopbackProxy` are already in the workspace!

---

## 2. Part A — Frictionless Universal Sign-In (`AuthSignInOrchestrator`)

### Deep Research & Security Design
Google enforces strict anti-phishing checks on embedded WebViews and provides RFC 8628 Device Flow for TV/limited-input clients. Relying on only ONE method risks lockout on certain OS environments (e.g., Linux Wayland headless without WebKitGTK, or Google tightening TV OAuth scopes). Therefore, `AuthSignInOrchestrator` implements a clean **3-tier sign-in cascade**:

```text
┌────────────────────────────────────────────────────────────────────────────┐
│  User clicks "Sign in to YouTube Music" (`auth_sign_in_sheet.rs`)          │
│                                                                            │
│  [Tier 1 — Primary: Embedded Native WebView] (macOS / Win / Linux / Android│
│   • Launches isolated ephemeral WebView (`nsh_core::prelude::web_view` on  │
│     Desktop or `WebViewSignInActivity` via JNI on Android)                 │
│   • Loads `https://accounts.google.com/ServiceLogin?continue=https://      │
│     music.youtube.com`                                                     │
│   • Verifies real `HttpOnly` session cookies (`SAPISID` /                  │
│     `__Secure-3PAPISID` + `SID` + `HSID`) via native CookieStore API       │
│   • On success: computes validated `AuthSession`, immediately destroys     │
│     ephemeral WebView profile & cookies, persists to Keyring/Keystore      │
│   • If WebView cannot read `HttpOnly` cookies or Google blocks UA → auto-  │
│     offers 1-click switch to Tier 2 (TV OAuth)                             │
│                                                                            │
│  [Tier 2 — Universal Fallback: RFC 8628 TV OAuth Device Flow]              │
│   • POST `/o/oauth2/device/code` with TV-scoped client credentials         │
│   • Displays crisp verification code (`XXXX-XXXX`), `google.com/device`    │
│     button (`open_url`), copy-code button, and countdown ring              │
│   • Polls `/o/oauth2/token` respecting `interval` + `slow_down` (+5s)      │
│   • Stores encrypted `refresh_token` in Keyring/Keystore; auto-refreshes   │
│     `access_token` 5 min prior to expiry with single-flight mutex          │
│                                                                            │
│  [Tier 3 — Collapsed "Advanced": Manual Browser Cookie Import]             │
│   • Hidden behind an expandable disclosure row for power users             │
└────────────────────────────────────────────────────────────────────────────┘
```

### Key Implementation Details (`crates/core/src/auth_flows/`)
1. **Strict Cookie Validation (`webview_cookie.rs`):**
   - Never trust `document.cookie` alone (because `SID` / `__Secure-3PSID` are `HttpOnly`!). Query the platform WebView cookie store for `https://music.youtube.com` and verify both a signing cookie (`SAPISID` or `__Secure-3PAPISID`) AND an `HttpOnly` session cookie (`SID` or `__Secure-3PSID`) exist before declaring sign-in complete.
   - Immediately trigger `AccountProfileProbe` (W1B) + `refresh_library_snapshot()` (W0/W1C) after storing the session.
2. **RFC 8628 State Machine (`tv_oauth.rs`):**
   - States: `Idle → RequestingCode → AwaitingUser { user_code, verification_url, expires_at } → Exchanging → SignedIn | Expired | Denied | Error`.
   - Handles all RFC 8628 error codes accurately: `authorization_pending` (keep polling), `slow_down` (increase poll interval by +5s), `access_denied` (clean cancel), `expired_token` (offer `"Generate New Code"` button).
   - **Zero Hardcoded Secrets:** Client ID/secret read from build-time config/env or user-provided OAuth config; every token redacted via `redact_secrets()`.
3. **Ephemeral Hygiene:** WebView profile directory uses an isolated ephemeral container that is wiped immediately upon completion or cancellation so stale Google cookies never linger unencrypted on disk.

---

## 3. Part B — Local Audio Library Import (`crates/core/src/local_media/`)

### Architecture & Performance Design (10,000-Track Rescan in `< 200 ms`)

```text
User Picks Folder (`pick_paths()` Desktop / SAF Android)
         │  Persisted in `SettingsV1::local_music_dirs`
         ▼
[Tokio `spawn_blocking` Incremental Scanner (`scanner.rs`)]
  1. Walk roots skipping symlinks escaping root & hidden `.` dirs
  2. Filter extensions: `.flac`, `.mp3`, `.m4a`, `.mp4`, `.opus`, `.ogg`, `.wav`
  3. Fast Stat Diff against SQLite `local_tracks(path, mtime_secs, size_bytes)`:
     • Unchanged `(mtime, size)` → SKIP `lofty` parse! (O(1) stat only)
     • New or modified → `lofty::Probe::open(path)?.read()` (header/tags only)
     • Deleted on disk → prune stale rows from `local_tracks`
  4. Extract:
     • Stable Track ID: `"local:" + blake3/sha256(canonical_path)[..16]`
     • Title, Artists, Album, Track #, Disc #, Year, Duration, Codec, Bitrate
     • ReplayGain (`REPLAYGAIN_TRACK_GAIN`) → feeds W2A `LoudnessNormalizer`!
     • Embedded Lyrics (`USLT` / Vorbis `LYRICS` / sidecar `.lrc` file!) → feeds Lyrics screen!
     • Embedded Picture → SHA-256 content-hashed deduplicated save into `ArtworkCache`
       (so 15 tracks of the same FLAC album store ONE artwork blob, not 15 copies!)
  5. Batch-commit inside a single SQLite WAL transaction (every 200 files)
     & emit `LocalScanProgress { scanned, added, updated, removed, done }`
```

### Security-Confined Loopback Playback (`crates/core/src/proxy/local_route.rs`)
- Route: `GET /local/<track_id>` (plus `HEAD`) served on `127.0.0.1:<port>` ONLY.
- **Strict Path Confinement Invariant (Zero Path Traversal):**
  1. Look up `track_id` in SQLite `local_tracks` (never accept raw filesystem paths in the URL!).
  2. Canonicalize `file_path` (`std::fs::canonicalize`) and verify `canonical_file.starts_with(canonical_root)` for at least one registered directory in `local_roots` (`SettingsV1::local_music_dirs`).
  3. If the file moved outside an approved root or became a symlink escaping the root → reject with `403 Forbidden` and log a redacted security warning!
- **Full `HTTP 206 Partial Content` Range Serving:** Streams requested byte ranges asynchronously (`tokio::fs::File::seek` + bounded 64 KB chunks) with accurate `Content-Type` (`audio/flac`, `audio/mpeg`, `audio/mp4`, `audio/ogg`, `audio/wav`) and `Accept-Ranges: bytes` so seeking in a 90 MB FLAC file is instantaneous!
- **Unified Queue & Playback:** Local tracks use the exact same `Track` struct (`id = "local:<hash>"`), allowing users to mix local FLAC files and YouTube Music tracks in the same Queue, Shuffle Deck, and Sleep Timer seamlessly (and W2A reads local paths directly with zero HTTP overhead!).

### Local Files UI (`src/screens/local_files.rs` & `src/store/local_media_slice.rs`)
- Clean header: `"+ Add Folder"` button (calls `nsh_core::prelude::pick_paths` on Desktop), `"Rescan"` button with subtle progress bar while scanning, and summary (`"428 tracks · 14.2 GB · 2 folders"`).
- Sub-filter pills: `All Tracks`, `Albums`, `Artists`, `Folders` (removable folder chips with confirmation).
- Instant client-side filter bar (`Filter local tracks...`) + `Play All` / `Shuffle All` buttons.
- Empty state: Tasteful centered illustration/icon + `"Listen to your own FLAC, MP3, and M4A collection alongside YouTube Music"` + primary `"Choose Music Folder"` CTA.

---

## 4. Deterministic Offline Verification (`tests/w2c_auth_and_local_media_test.rs`)

1. `tv_oauth_rfc8628_handles_pending_slow_down_success_and_expiry` — runs against local `127.0.0.1:0` mock OAuth server returning `authorization_pending` → `slow_down` (verifying interval increment) → token grant, plus `expired_token` path.
2. `webview_cookie_validator_requires_httponly_session_and_sapisid` — asserts cookie strings missing `SID`/`__Secure-3PSID` are rejected and valid sets compute proper `AuthSession`.
3. `local_scanner_parses_flac_wav_mp3_deduplicates_artwork_and_diffs_incrementally` — generates valid tagged audio files + sidecar `.lrc` in `TempDir`, scans once (asserts tags, ReplayGain, sidecar lyrics, 1 deduplicated artwork blob for 3 album tracks), rescans unchanged (asserts 0 re-parses!), modifies 1 file & deletes 1 file, rescans (asserts exact delta!).
4. `local_proxy_route_serves_http_206_and_blocks_path_traversal_and_symlink_escape` — tests valid `Range: bytes=0-1023` returning `206 Partial Content`, verifies unknown ID returns `404`, and verifies symlink pointing outside `local_roots` returns `403 Forbidden`.
5. `auth_sheet_and_local_files_screen_render_cleanly_in_headless_nsh` — headless UI verification of sign-in tabs, hidden Advanced cookie drawer, and Local Files empty/populated states with zero raw colors.

```bash
cargo test --test w2c_auth_and_local_media_test -- --nocapture
```
