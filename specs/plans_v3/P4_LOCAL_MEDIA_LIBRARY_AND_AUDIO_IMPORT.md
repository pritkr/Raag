# P4: Local Media Library, Metadata Tagging & Cross-Platform Folder Import

> **Wave:** A (Parallel)
> **Status of Existing Code:** `crates/core/src/local_media/*` (`artwork.rs`, `scanner.rs`, `store.rs`), `crates/core/src/proxy/local_route.rs`, `src/store/local_media_slice.rs`, and `src/screens/local_files.rs` exist as isolated components, **but**:
> 1. `local_files.rs` is not mounted inside `Library` (or reachable from main navigation) and `LocalMediaScanner` is never spawned in `live_engine`.
> 2. There is no real native OS folder picker (Desktop `rfd` / XDG portal; Android Storage Access Framework `Intent.ACTION_OPEN_DOCUMENT_TREE` + `MediaStore` audio scan).
> 3. `LoopbackProxy` `/local-file/<id>` route is not registered in `LoopbackProxy` router nor connected to `PlaybackController` for `local:<hash>` track IDs.
> **Exclusive Owned Paths:** `crates/core/src/local_media/*`, `crates/core/src/proxy/local_route.rs`, `src/store/local_media_slice.rs`, `src/screens/local_files.rs`, `platform/android/src/main/kotlin/dev/resonance/ytmusic/SafFolderPicker.kt`, `src/live_engine/handlers/local_media.rs`, `tests/p4_local_media_test.rs`.

---

## 1. Deep Technical Research & Architecture

### 1.1 Supported Formats & Zero-Copy Local Playback
Local libraries contain heterogeneous files (`FLAC`, `MP3`, `M4A/AAC`, `ALAC`, `Opus`, `Ogg Vorbis`, `WAV`).
- **Tag Extraction (`lofty`):** Extract `title`, `artists` (splitting multi-value frames + `;`/`/` when safe), `album`, `album_artist`, `track_number`, `disc_number`, `year`, `duration_ms`, `bitrate_kbps`, `sample_rate_hz`, `bit_depth`, `replaygain_track_gain_db`, and embedded cover art (`PictureType::CoverFront` -> deduplicated by SHA-256 into `<cache_dir>/local_art/<hash>.jpg` resized to `<= 600x600`).
- **Fallback Cover Art:** If a file lacks embedded artwork, check sibling directory for `cover.jpg`, `folder.jpg`, `front.png`, `album.jpg` (case-insensitive) before falling back to deterministic monogram gradient.
- **Deterministic Track IDs:** `local:<blake3_or_sha256_prefix_16(canonical_path)>` — stable across rescans, clearly distinguished from 11-char YouTube `videoId`s so:
  - InnerTube `/next` and Like/Playlist mutations never send a `local:*` ID to YouTube servers.
  - Listen Together rooms (`P5`) cleanly emit `LocalTrackWarning` instead of breaking remote peers.
  - Now Playing displays the honest `"Local Audio • FLAC 24-bit / 96 kHz"` badge.

### 1.2 Incremental Scanner & Security Boundary (`crates/core/src/local_media/scanner.rs`)
1. **Path Canonicalization & Allowlist Security:**
   - Every watched folder is stored in SQLite `local_folders(id, canonical_path, added_at, last_scanned_at)`.
   - `local_route.rs` MUST **NEVER** accept arbitrary filesystem paths from HTTP query params! Instead, it resolves `/local-file/<track_id>` strictly by looking up `track_id` in SQLite `local_tracks`, verifying the resolved canonical path still resides inside an authorized `local_folders` root (preventing symlink escape / path traversal), and serving HTTP `200`/`206` byte ranges with accurate `Content-Type` and `Accept-Ranges: bytes`.
2. **Fast Incremental Rescan:**
   - Walk watched folders on a dedicated `tokio::task::spawn_blocking` thread using `walkdir` (skip hidden dotfiles, ignore symlink loops `follow_links(false)`).
   - Compare `(file_size, mtime_unix_secs)` against SQLite `local_tracks`. Skip `lofty` tag parsing for unchanged files -> re-scanning a 10,000-track FLAC library takes `< 250ms`!
   - Prune deleted files from SQLite and garbage-collect unreferenced artwork hashes.
   - Stream progress (`LocalScanProgress { scanned, added, updated, removed, current_folder }`) throttled every 150ms to `local_media_slice`.

### 1.3 Native Folder Pickers (Desktop & Android)
- **Desktop (macOS / Linux / Windows):** Use `rfd::AsyncFileDialog::new().set_title("Choose Music Folder").pick_folder().await` (uses native Cocoa `NSOpenPanel`, XDG Desktop Portal on Linux/Flatpak, Win32 `IFileOpenDialog` — zero GTK runtime dependency issues).
- **Android API 26+:**
  - Create `platform/android/src/main/kotlin/dev/resonance/ytmusic/SafFolderPicker.kt` supporting:
    1. One-tap `"Scan Device Music"` using `MediaStore.Audio.Media.EXTERNAL_CONTENT_URI` (requires `READ_MEDIA_AUDIO` on API 33+ or `READ_EXTERNAL_STORAGE` on API 26–32).
    2. Custom folder selection via `Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)` with `takePersistableUriPermission(uri, FLAG_GRANT_READ_URI_PERMISSION)`.
  - Pass scanned metadata or `content://` file descriptor (`openFileDescriptor(uri, "r")`) across JNI into Rust (`LocalMediaStore`).

---

## 2. Concrete Implementation Plan (Step-by-Step)

### Step 1: Harden `crates/core/src/local_media/` & `proxy/local_route.rs`
- **`scanner.rs`:**
  - Implement incremental mtime/size diffing, cancellation token, sibling `cover.jpg` discovery, and multi-format extension filter (`flac`, `mp3`, `m4a`, `aac`, `opus`, `ogg`, `wav`, `alac`, `aiff`).
  - Extract technical audio format metadata (`codec`, `sample_rate_hz`, `bitrate_kbps`) for display in Now Playing & Local Files rows.
- **`store.rs`:**
  - Persist `local_folders`, `local_tracks`, and `local_art_refs` in SQLite (with FTS5 or indexed `title`/`artist`/`album` columns for instant `< 5ms` local search!).
  - Support querying grouped by **Songs**, **Albums**, **Artists**, and **Folders**.
- **`proxy/local_route.rs`:**
  - Register `/local-file/{id}` and `/local-art/{hash}` on `LoopbackProxy` (127.0.0.1 only, with per-session bearer/path token matching `LoopbackProxy` auth).
  - Support full RFC 7233 `Range: bytes=start-end` streaming via `tokio::fs::File` + `ReaderStream` (bounded 64 KB chunks, zero full-file RAM buffering!).
  - Reject any path escaping registered `local_folders` via `dunce::canonicalize` / `std::fs::canonicalize`.

### Step 2: Handler & Store Integration (`src/live_engine/handlers/local_media.rs`)
- Commands: `AddFolder(PathBuf)`, `PickFolderNative`, `RemoveFolder(String)`, `RescanAll`, `CancelScan`, `SearchLocal(String)`.
- On `PlaybackController::play_track(track)` when `track.id.starts_with("local:")`:
  - Bypass InnerTube entirely.
  - On Desktop Native PCM (`P1`): hand `PathBuf` directly to `SeekableStreamSource::LocalFile(path)`.
  - On URL backend fallback: hand `http://127.0.0.1:<port>/local-file/<id>?token=<tok>` to `AudioBackend`.

### Step 3: Mount & Polish `src/screens/local_files.rs` inside `Library`
- Mount `Local Files` as a first-class filter pill in `src/screens/library.rs` (`Playlists | Liked Songs | Downloads | Local Files`).
- Inside `local_files.rs`:
  - Watched folders bar with `"+ Add Folder"` (opens native picker), `"Rescan"`, and per-folder remove button (`>= 48dp`).
  - Live scan progress indicator (`"Scanning 412 / 1,280 files…"` + Cancel button).
  - Sub-tabs (`Songs | Albums | Artists`) + instant local filter input (`"Filter local songs…"`).
  - Virtualized `virtual_list` track rows (zero backdrop blur, `>= 48dp` targets) with `"Play All"` / `"Shuffle"` using atomic `store::play_queue(...)`!

---

## 3. Security Invariants & Acceptance Tests (`tests/p4_local_media_test.rs`)

1. **Path Traversal & Symlink Escape Test:** Place a symlink inside a watched temp folder pointing to `/etc/passwd` (or a file outside the watched root) -> assert scanner skips it AND `local_route.rs` rejects any out-of-root path with HTTP `403`.
2. **Incremental Rescan Speed & Diff Test:** Create a temp directory with 20 valid synthetic WAV/FLAC files; run scan 1 -> assert 20 added; modify 1 file & delete 2 files -> run scan 2 -> assert `added=0, updated=1, removed=2` and unchanged files were NOT re-parsed.
3. **Byte-Range Streaming Test:** Request `Range: bytes=0-1023` and `Range: bytes=1024-2047` from `/local-file/<id>` -> assert HTTP `206 Partial Content`, exact `Content-Range` header, and byte-identical slice.
4. **No InnerTube Leakage Test:** Play a `local:*` track -> assert InnerTube client receives **0** `/player` or `/next` requests.
