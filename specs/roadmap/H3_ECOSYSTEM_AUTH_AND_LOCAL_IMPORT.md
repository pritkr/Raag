# H3 — Ecosystem: Single Unified Embedded Desktop OAuth / WebView Sign-In & Local-File Import

> **Wave:** Wave 2 (Parallel Sprint — Parallelizable with H2, LT-1, LT-2, LT-3)
> **Dependencies:** Builds on **H0** (`AppPaths`, `SettingsV1::local_music_dirs`, `LibraryLoaded` hydration), **ST-1** (`AccountProfileProbe`), and **ST-3** (`ytmapi-rs` `BrowserToken` / `OAuthToken` support).
> **Deduplication Ownership Guarantee:**
> - **H3.1 is the SINGLE authoritative owner of Desktop Sign-In UX (Embedded NSH WebView + RFC 8628 TV OAuth Device-Code + Cookie Fallback).** ST-3 intentionally does NOT duplicate sign-in sheets or OAuth clients.
> - **H3.2 reuses the workspace's existing `lofty = "0.22"` crate, existing `ArtworkCache`, and existing `LoopbackProxy`** — zero duplicate taggers, zero parallel audio engines!
> **Parallelism Boundary:**
> - **Exclusive Write Scope:**
>   - `crates/core/src/auth_flows/` (new — `mod.rs`, `tv_oauth.rs`, `orchestrator.rs`, `cookie_extract.rs`)
>   - `src/screens/auth_sign_in_sheet.rs` (new — single unified sign-in modal sheet)
>   - `crates/core/src/local_media/` (new — `mod.rs`, `scanner.rs`, `store.rs`, `artwork.rs`)
>   - `crates/core/src/proxy/local_route.rs` (new — confined `/local/<track_id>` byte-range handler)
>   - `src/screens/local_files.rs` (new — Local Library & Folder Manager view)
>   - `src/store/local_media_slice.rs` (new)
>   - `tests/h3_ecosystem_auth_and_local_import_test.rs` (new)
> - **Forbidden Edits in Wave 2:** `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/settings.rs`, `crates/core/src/proxy/mod.rs` (Wave 3 Stitch mounts `local_route::handle_local_request` in 6 lines), `platform/**`.

---

## 1. H3.1 — Single Unified Embedded Desktop WebView & TV OAuth Sign-In

### Problem
Manual browser DevTools cookie extraction (`SAPISID=...; __Secure-3PAPISID=...`) is a high-friction barrier for normal desktop users. At the same time, Linux users on minimal/headless distributions may lack WebKitGTK 6.0, and YouTube's `TV_SAMSUNG` (`TvOAuthOnly`) client requires an OAuth token rather than browser cookies.

### Canonical Architecture: `AuthSignInOrchestrator` (One Reliable Funnel)

Every sign-in method flows into ONE unified `AuthSession` lifecycle:

```text
┌────────────────────────────────────────────────────────────────────────────┐
│            Unified Sign-In Modal (`src/screens/auth_sign_in_sheet.rs`)     │
├────────────────────────────┬───────────────────────────┬───────────────────┤
│ 1. Primary (Desktop):      │ 2. Universal Fallback:    │ 3. Advanced:      │
│    Embedded NSH WebView    │    YouTube TV OAuth       │    Manual Cookie  │
│    (WKWebView / WebView2 / │    (RFC 8628 Device Code) │    Header Paste   │
│     WebKitGTK via          │    Works on Linux w/o     │    (collapsible   │
│     capabilities.webview)  │    WebKitGTK & Steam Deck │     disclosure)   │
└──────────────┬─────────────┴─────────────┬─────────────┴─────────┬─────────┘
               │                           │                       │
               ▼                           ▼                       ▼
┌────────────────────────────────────────────────────────────────────────────┐
│                   `AuthSignInOrchestrator` (`crates/core/src/auth_flows/`) │
│  1. Validates required tokens (`SAPISID`/`__Secure-3PAPISID` or OAuth)     │
│  2. Destroys ephemeral WebView & wipes WebView cookie jar immediately      │
│  3. Persists encrypted credentials to `DesktopAuthStore` (OS Keychain via  │
│     `keyring` / ST-2) or `AndroidKeystoreAuthStore` (ST-4)                 │
│  4. Syncs credentials to `InnerTubeClient` + `YtmapiBrowseAdapter` (ST-3)  │
│  5. Runs ST-1 `AccountProfileProbe` (real name, avatar, `is_premium`)      │
│  6. Triggers H0.1 `refresh_library_snapshot()` → `LiveUpdate::LibraryLoaded│
└────────────────────────────────────────────────────────────────────────────┘
```

### 1.1 Primary Path: Embedded NSH WebView Sign-In (macOS, Windows, Linux with WebKitGTK)
- **Capability Configuration:** `nsh.json` already allows `accounts.google.com`, `music.youtube.com`, `www.youtube.com` with `"input": true`.
- **Flow (`src/screens/auth_sign_in_sheet.rs`):**
  1. Render modal sheet (`test_id("auth-sign-in-sheet")`) containing an embedded NSH `webview()` navigating to:
     `https://accounts.google.com/ServiceLogin?service=youtube&hl=en&continue=https%3A%2F%2Fmusic.youtube.com%2F`
  2. **User-Agent Hygiene:** Ensure the embedded WebView User-Agent does NOT include Android `; wv` or suspicious automation tokens so Google OAuth never rejects the login with `"This browser or app may not be secure"`.
  3. **Completion Detection:** Listen to navigation URL changes; once the URL host equals `music.youtube.com` and the path is `/` (or `/library`), extract `Cookie` values (`SAPISID`, `__Secure-3PAPISID`, `HSID`, `SSID`, `APISID`, `SID`, `__Secure-1PSID`, `__Secure-3PSID`) via NSH WebView cookie bridge / JS hook (falling back to native cookie store inspection for `HttpOnly` cookies like `SID`/`__Secure-3PSID`!).
  4. **Critical `HttpOnly` Cookie Catch:** `document.cookie` in JS can read `SAPISID` / `__Secure-3PAPISID` (which are NOT `HttpOnly`), **but `SID` / `__Secure-3PSID` ARE `HttpOnly`!** Therefore:
     - Prefer native WebView cookie store API (`WKHTTPCookieStore` on macOS, `ICoreWebView2CookieManager` on Windows, `WebKitCookieManager` on Linux — exposed via NSH webview cookie API or small platform helper).
     - Validate that BOTH `SAPISID`/`__Secure-3PAPISID` AND session cookies (`SID` or `__Secure-3PSID`) are present before declaring success (`crates/core/src/auth_flows/cookie_extract.rs`).
     - If a given platform's NSH WebView build does not yet expose `HttpOnly` cookie reads, automatically guide the user seamlessly to **Path 2 (TV OAuth Device-Code)** with a single click—never silently storing an incomplete cookie jar that 401s!
  5. **Ephemeral Teardown:** Immediately unmount the WebView element and clear its transient WebView browsing profile once `AuthSession` is persisted in OS Keychain.

### 1.2 Universal Path: RFC 8628 YouTube TV OAuth Device-Code Flow (`crates/core/src/auth_flows/tv_oauth.rs`)
- Works reliably on **100% of platforms** (macOS, Windows, Wayland/X11 Linux without WebKitGTK, Steam Deck, Android TV) AND unlocks Resonance's `TV_SAMSUNG` (`AuthMode::TvOAuthOnly`) client in `client_matrix.json` AND `ytmapi-rs`'s `OAuthToken`!
- **Protocol Flow:**
  1. `POST https://oauth2.googleapis.com/device/code` with client ID (`RESONANCE_OAUTH_CLIENT_ID` / user-provided or app TV OAuth client credentials) and scope `https://www.googleapis.com/auth/youtube`.
  2. Display large copyable `user_code` (e.g., `ABCD-EFGH`), clickable verification URL (`https://www.google.com/device`), and live countdown timer in `auth_sign_in_sheet.rs`.
  3. Spawn cancellable background Tokio polling loop (`POST https://oauth2.googleapis.com/token`):
     - Respects `interval` seconds.
     - Handles `authorization_pending` (keep polling), `slow_down` (add +5s to interval), `expired_token` (offer `"Generate New Code"`), and `access_denied` (clean cancel).
  4. On `200 OK`: Store `access_token`, `refresh_token`, and `expires_at_unix_secs` encrypted inside `AuthSession` (`OS Keychain` / `Android Keystore`).
  5. **Single-Flight Token Refresh:** `TvOAuthTokenManager` wraps refresh in a `tokio::sync::Mutex<()>` and proactively refreshes when `now + 120s >= expires_at_unix_secs`, preventing stampeding refresh requests across concurrent API calls.
  6. **Strict Secret Redaction:** `access_token`, `refresh_token`, and `device_code` are NEVER logged and are registered with `redact_secrets()`.

### 1.3 Advanced Fallback: Manual Cookie Header Paste
- Preserved cleanly under a collapsed `"Advanced: Paste Cookie Header"` accordion inside `auth_sign_in_sheet.rs` for power users / CI smoke verification—zero loss of existing functionality.

---

## 2. H3.2 — Local-File Import (`lofty` + SQLite + Unified `LoopbackProxy` Playback)

### Architectural Insight: Zero Second Audio Pipeline!
Instead of inventing a separate local-file player (which would break Queue mixing, Lyrics, ReplayGain, Android Auto, and LAN Casting), **local files become first-class `Track` items** with deterministic IDs (`local:<blake3_or_sha256_prefix_16>`) served through `LoopbackProxy` at `http://127.0.0.1:<port>/local/<track_id>`!

Because they flow through the exact same `PlaybackController` + `LoopbackProxy`:
- Users can **freely mix YouTube Music tracks and local FLAC/MP3/Opus files in the same Queue**!
- Local tracks automatically get **synced LRCLIB / Better Lyrics** (plus embedded `USLT`/Vorbis `LYRICS` tags read via `lofty`!), **ReplayGain**, **Ambient Artwork Palette**, **System Media Controls (MPRIS/SMTC/NowPlaying)**, **Android Auto browsing**, and **LAN Casting**!

### 2.1 Contracts & SQLite Schema (Migration v4)

#### Contract (`crates/contract/src/local_media.rs` — Frozen in Wave 0)
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalFolderRoot {
    pub path: String,
    pub added_at_unix_secs: u64,
    pub last_scanned_unix_secs: Option<u64>,
    pub track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalTrackRecord {
    pub track_id: String,         // "local:<16-hex-sha256-of-canonical-path>"
    pub canonical_path: String,
    pub root_path: String,
    pub file_size_bytes: u64,
    pub mtime_unix_secs: u64,
    pub mime_type: String,        // "audio/flac" | "audio/mpeg" | "audio/mp4" | "audio/ogg" | "audio/wav"
    pub sample_rate_hz: Option<u32>,
    pub bitrate_kbps: Option<u32>,
    pub track: Track,             // Standard Resonance `Track` with `artwork_url = Some("http://127.0.0.1:<port>/local-art/<hash>")`
    pub embedded_lyrics: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalScanProgress {
    pub scanning: bool,
    pub files_seen: u32,
    pub files_indexed: u32,
    pub files_skipped_unchanged: u32,
    pub files_removed_missing: u32,
    pub errors: u32,
}
```

#### SQLite Migration v4 (`crates/core/src/local_media/store.rs` or `db.rs` v4 hook)
```sql
CREATE TABLE IF NOT EXISTS local_roots (
    root_path TEXT PRIMARY KEY,
    added_at INTEGER NOT NULL,
    last_scanned_at INTEGER
);

CREATE TABLE IF NOT EXISTS local_tracks (
    track_id TEXT PRIMARY KEY,
    canonical_path TEXT NOT NULL UNIQUE,
    root_path TEXT NOT NULL,
    file_size_bytes INTEGER NOT NULL,
    mtime_unix_secs INTEGER NOT NULL,
    mime_type TEXT NOT NULL,
    sample_rate_hz INTEGER,
    bitrate_kbps INTEGER,
    artwork_hash TEXT,
    embedded_lyrics TEXT,
    track_json TEXT NOT NULL,
    indexed_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_local_tracks_root ON local_tracks(root_path);
CREATE INDEX IF NOT EXISTS idx_local_tracks_artist_album ON local_tracks(track_id);
```

### 2.2 High-Speed Incremental `lofty` Scanner (`crates/core/src/local_media/scanner.rs`)
- **Supported Formats (Already Supported by `lofty = "0.22"` in Workspace!):**
  - `.flac` (`audio/flac`)
  - `.mp3` (`audio/mpeg`)
  - `.m4a` / `.mp4` / `.aac` (`audio/mp4`)
  - `.ogg` / `.opus` (`audio/ogg`)
  - `.wav` (`audio/wav`)
  - *(macOS/Desktop decoder note: verify format compatibility with `nsh/audio` native decoder; macOS `AVAudioEngine` / Windows Media Foundation / Linux GStreamer all decode FLAC, MP3, M4A, WAV natively, and Opus on Linux/Android/macOS 14+.)*
- **Incremental Fast-Path (Re-scans 10,000 tracks in `< 200 ms`):**
  1. Walk each configured `LocalFolderRoot` on a dedicated blocking worker (`tokio::task::spawn_blocking`, never blocking UI or async executors).
  2. Ignore hidden files/dirs (`.` prefix) and symlinks that escape the authorized `root_path` (path-traversal & symlink-loop protection via `std::fs::canonicalize` + `canonical.starts_with(&canonical_root)`).
  3. Look up existing `(file_size_bytes, mtime_unix_secs)` from SQLite:
     - **Unchanged?** Skip `lofty` file read completely (`files_skipped_unchanged += 1`)!
     - **New or Modified?** Open with `lofty::probe::Probe::open(&path)?.guess_file_type()?.read()` (header-only parse — never decodes PCM audio!).
       - Extract `title` (fallback: filename stem), `artist` (fallback: `"Unknown Artist"`), `album`, `track_number`, `year`, `duration_ms` (`properties.duration().as_millis()`), `sample_rate`, `audio_bitrate`, and embedded `ItemKey::Lyrics`!
       - **Embedded Artwork Extraction (`artwork.rs`):** Extract `PictureType::CoverFront` (fallback: first picture, capped at `MAX_EMBEDDED_ARTWORK_BYTES = 4 MB`). Hash bytes with SHA-256 (`art_hash`) and store once in `ArtworkCache` under key `local_art:<art_hash>` — deduplicating identical album covers across all 15 tracks of an album automatically!
  4. **Prune Deleted Files:** After walking a root, delete any `local_tracks` rows under that `root_path` whose `canonical_path` was not seen during the walk.
  5. Emit throttled `LocalScanProgress` updates (every 25 files or 150 ms) over `EventBus` so the UI shows smooth progress without flooding the NSH UI thread.

### 2.3 Confined `LoopbackProxy` Route (`crates/core/src/proxy/local_route.rs`)
- **Routes Mounted on `127.0.0.1:<port>`:**
  - `GET /local/<track_id>` → serves local audio file with full `HTTP 206 Partial Content` / `Accept-Ranges: bytes` seeking, exact `Content-Type` (`audio/flac`, `audio/mpeg`, etc.), and `X-Resonance-Source: local-file`.
  - `GET /local-art/<art_hash>` → serves cached embedded cover art bytes from `ArtworkCache` (`image/jpeg` / `image/png` / `image/webp`) with immutable `Cache-Control`.
- **Zero Arbitrary File Read (Security Invariant):**
  - The URL NEVER accepts a filesystem path! It accepts strictly a `local:<16-hex>` ID looked up in SQLite `local_tracks`.
  - Before opening the file, re-verify `canonical_path` is still confined inside a registered `local_roots` directory and is a regular file (not a FIFO/device/symlink outside the root).
  - Returns `404 Not Found` if the user moved/deleted the file, triggering a clean toast (`"Local file moved or unavailable — re-scan folder"`).

### 2.4 Folder Picking & UI Surface (`src/screens/local_files.rs`)
- **Desktop Folder Picker:** Uses NSH native file dialog (`pick_paths` directory mode, verified against `.agents/skills/nsh/references/rust.md`; fallback to path input field in headless/test environments).
- **Android Folder Picker:** Uses Android Storage Access Framework (`Intent.ACTION_OPEN_DOCUMENT_TREE` with persistable read URI permissions) or `READ_MEDIA_AUDIO` via ST-4's `ResonanceNativeBridge` (gracefully deferred to read-only scan of selected trees).
- **Library Integration:**
  - Wire Library → **`Uploads / Local`** tab (`LibraryFilter::Uploads` renamed cleanly to `"Local & Uploads"` or dedicated `"Local Files"` filter chip) showing:
    - Folder Manager header (`"Add Music Folder"`, `"Rescan"`, live scan progress banner, folder list with track counts & `"Remove Folder"` button).
    - Sub-tabs: `Tracks`, `Albums`, `Artists` (grouped deterministically in-memory from `LocalMediaSlice::tracks`).
    - Search integration: Local tracks appear in an optional `"Local Files"` section inside `Search` results so users can search their entire hybrid library in one place!
- **Embedded Lyrics Cascade Hook:** When `CascadeLyricsProvider` receives a `local:*` track, it checks `local_tracks.embedded_lyrics` (Priority 0 — instant offline LRC/plain lyrics!) before falling through to LRCLIB / Better Lyrics.

---

## 3. Security, Privacy & Performance Edge Cases

1. **WebView Cookie Hygiene:** Ephemeral WebView teardown + `HttpOnly` verification prevents half-authenticated sessions and leaves zero residual browser state.
2. **OAuth Secret Isolation:** TV OAuth `refresh_token` and `access_token` live strictly in OS Keychain / Android Keystore (or encrypted fallback), never in SQLite `metadata_cache` or logs.
3. **Symlink Escape & Zip-Bomb Artwork Protection:** Local scanner strictly rejects symlinks resolving outside registered `local_roots`, ignores files `> 2 GB`, and caps extracted artwork at `4 MB`.
4. **Zero UI Thread Stalls:** Directory walking, `lofty` tag parsing, SHA-256 hashing, and SQLite batch upserts (chunked in transactions of 100 tracks) run strictly on `tokio::task::spawn_blocking`.

---

## 4. Offline Verification Test Suite (`tests/h3_ecosystem_auth_and_local_import_test.rs`)

1. `tv_oauth_device_code_flow_handles_pending_slow_down_success_and_single_flight_refresh` — local `127.0.0.1:0` mock OAuth server tests `authorization_pending` → `slow_down` → `200 OK`, verifies concurrent calls share one refresh request, and asserts `redact_secrets()` strips tokens.
2. `cookie_extractor_validates_sapisid_and_httponly_session_cookies` — verifies incomplete `SAPISID`-only capture without session cookie is flagged cleanly, while valid full cookie jars produce a complete `AuthSession`.
3. `local_scanner_indexes_wav_flac_mp3_dedups_artwork_skips_unchanged_and_prunes_deleted` — creates synthetic valid tagged audio files (using `lofty` tag writer + minimal valid headers) in a `TempDir`; verifies (a) initial scan indexes all tracks & extracts artwork once per album; (b) second scan hits `files_skipped_unchanged == N`; (c) deleting 1 file prunes its SQLite row.
4. `local_scanner_blocks_symlink_escape_outside_registered_root` — places symlink pointing to `/etc/hosts` or outside `TempDir`; verifies scanner skips it and `/local/<id>` refuses out-of-root paths.
5. `loopback_proxy_local_route_serves_http_206_range_and_embedded_lyrics_priority_zero` — starts `LoopbackProxy` with local route; requests `Range: bytes=0-63` -> asserts `206 Partial Content`, `X-Resonance-Source: local-file`, and exact byte slice match.

```bash
cargo test --test h3_ecosystem_auth_and_local_import_test -- --nocapture
cargo test --test desktop_auth_media_test --test proxy_and_fault_injection
```
