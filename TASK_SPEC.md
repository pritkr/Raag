# TASK_SPEC — Option C: Persistent `OfflineStore` (SQLite `downloads` + Atomic `.part` Worker + `lofty` Tagging + `LoopbackProxy` Local-Disk Bypass)

## Scope & Invariants
- Target crate boundaries: `crates/contract` (frozen traits/types), `crates/core` (SQLite migration v2, atomic download worker, `lofty` tagger, `LoopbackProxy` local-disk bypass, `PlaybackController` offline fast-path), `platform/` (storage dirs + `.nomedia`), `src/` (`live_engine.rs`, `store.rs`, `screens/library.rs`).
- Preserve all existing constructors (`LoopbackProxy::start`, `PlaybackController::new`) via additive builders (`start_with_offline`, `with_offline_store`) so existing test suites compile unchanged.
- Invariant preservation: `AuthMode` isolation (`ANDROID_VR`/`IOS` never receive cookies/`SAPISIDHASH`), `127.0.0.1` `LoopbackProxy` single audio egress, zero blocking I/O on `src/store.rs` NSH UI thread, zero raw color literals in `src/screens/`.

---

## 1. Exact `crates/contract` Signatures

### `crates/contract/src/error.rs`
Add `OfflineStorageError` and integrate into `CoreError` without breaking existing variants:
```rust
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum OfflineStorageError {
    #[error("invalid video id for offline storage: {0}")]
    InvalidVideoId(String),
    #[error("path traversal rejected: {0}")]
    PathTraversal(String),
    #[error("offline quota exceeded (used {used_bytes}, limit {limit_bytes})")]
    QuotaExceeded { used_bytes: u64, limit_bytes: u64 },
    #[error("insufficient free disk space (available {available_bytes}, required {required_bytes})")]
    InsufficientSpace { available_bytes: u64, required_bytes: u64 },
    #[error("download cancelled for {video_id}")]
    Cancelled { video_id: String },
    #[error("download not found: {video_id}")]
    NotFound { video_id: String },
    #[error("integrity mismatch for {video_id}: expected {expected}, got {actual}")]
    IntegrityMismatch {
        video_id: String,
        expected: String,
        actual: String,
    },
    #[error("tagging failed: {0}")]
    Tagging(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("sqlite error: {0}")]
    Sqlite(String),
}
```
Add variant to `CoreError`:
```rust
    #[error("offline storage error: {0}")]
    OfflineStorage(#[from] OfflineStorageError),
```

### `crates/contract/src/domain.rs`
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Paused,
    Tagging,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub video_id: TrackId,
    pub track: Track,
    pub status: DownloadStatus,
    pub itag: u32,
    pub codec: AudioCodec,
    pub mime_type: String,
    /// Relative filename inside downloads root (e.g. `dQw4w9WgXcQ_140.m4a`). Never an absolute path.
    pub relative_path: Option<String>,
    pub bytes_downloaded: u64,
    pub total_bytes: u64,
    pub sha256_hex: Option<String>,
    pub tags_embedded: bool,
    pub created_at_unix_secs: u64,
    pub updated_at_unix_secs: u64,
    pub completed_at_unix_secs: Option<u64>,
    /// Redacted human-readable failure reason (must never contain signed URLs or query params).
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct OfflineStats {
    pub completed_count: u64,
    pub active_count: u64,
    pub total_bytes: u64,
    pub max_bytes: u64,
}
```

### `crates/contract/src/events.rs`
Add variant to `AppEvent`:
```rust
    DownloadUpdated(DownloadRecord),
    DownloadRemoved { video_id: TrackId },
```

### `crates/contract/src/traits.rs`
```rust
pub trait OfflineStore: Send + Sync {
    fn upsert_download(&self, record: &DownloadRecord) -> CoreResult<()>;
    fn update_download_progress(
        &self,
        video_id: &str,
        status: DownloadStatus,
        bytes_downloaded: u64,
        total_bytes: u64,
        updated_at_unix_secs: u64,
    ) -> CoreResult<()>;
    fn mark_download_completed(
        &self,
        video_id: &str,
        relative_path: &str,
        file_size_bytes: u64,
        sha256_hex: &str,
        tags_embedded: bool,
        completed_at_unix_secs: u64,
    ) -> CoreResult<()>;
    fn mark_download_failed(
        &self,
        video_id: &str,
        status: DownloadStatus,
        redacted_error: Option<&str>,
        updated_at_unix_secs: u64,
    ) -> CoreResult<()>;
    fn get_download(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>>;
    fn get_completed_download(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>>;
    fn list_downloads(&self) -> CoreResult<Vec<DownloadRecord>>;
    fn delete_download_record(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>>;
    fn offline_stats(&self, max_bytes: u64) -> CoreResult<OfflineStats>;
    /// Absolute canonical directory holding finalized offline media files.
    fn downloads_root(&self) -> std::path::PathBuf;
}
```
Re-export `DownloadRecord`, `DownloadStatus`, `OfflineStats`, `OfflineStorageError`, `OfflineStore` from `crates/contract/src/lib.rs`.

---

## 2. File-by-File Implementation Plan

### Dependencies (`Cargo.toml`, `crates/core/Cargo.toml`)
- `Cargo.toml` `[workspace.dependencies]`: add `lofty = "0.22"`.
- `crates/core/Cargo.toml` `[dependencies]`: add `lofty.workspace = true`.

### `crates/core/src/cache/db.rs` — SQLite Migration v2 + `OfflineStore` Impl
- Bump `const SCHEMA_VERSION: i32 = 2;`.
- Add field `downloads_dir: PathBuf` to `SqliteCacheStore` (`data_dir.join("downloads")`, `create_dir_all` in `open()`).
- Replace single `if version < SCHEMA_VERSION` check with ordered incremental migrations:
  - `if version < 1 { migrate_v1(&conn)?; }`
  - `if version < 2 { migrate_v2(&conn)?; }`
- Exact `migrate_v2(conn: &Connection) -> CoreResult<()>` DDL (executed inside `BEGIN IMMEDIATE; ... COMMIT;`):
```sql
CREATE TABLE IF NOT EXISTS downloads (
    video_id TEXT PRIMARY KEY,
    track_json TEXT NOT NULL,
    status TEXT NOT NULL,
    itag INTEGER NOT NULL,
    codec TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    relative_path TEXT,
    bytes_downloaded INTEGER NOT NULL DEFAULT 0,
    total_bytes INTEGER NOT NULL DEFAULT 0,
    sha256_hex TEXT,
    tags_embedded INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    completed_at INTEGER,
    error_message TEXT
);
CREATE INDEX IF NOT EXISTS idx_downloads_status ON downloads(status, updated_at DESC);
PRAGMA user_version = 2;
```
- Implement `OfflineStore for SqliteCacheStore`:
  - `status` serialized strictly via deterministic lowercase string enum (`"queued"`, `"downloading"`, `"paused"`, `"tagging"`, `"completed"`, `"failed"`, `"cancelled"`).
  - `codec` serialized strictly via `"opus"` / `"aac"`.
  - `get_completed_download`: `SELECT ... FROM downloads WHERE video_id = ?1 AND status = 'completed' AND relative_path IS NOT NULL`.
  - `list_downloads`: `ORDER BY COALESCE(completed_at, updated_at) DESC`.
  - `delete_download_record`: `BEGIN IMMEDIATE`, read row, `DELETE FROM downloads WHERE video_id = ?1`, `COMMIT`, return prior row.
  - `clear_all_caches()` MUST NOT delete `downloads` rows (explicit offline downloads survive transient cache clears).
  - Add helper `pub fn recover_interrupted_downloads(&self, now_unix_secs: u64) -> CoreResult<Vec<DownloadRecord>>`: transitions any rows in `'downloading'` or `'tagging'` to `'paused'` on startup.

### `crates/core/src/offline/mod.rs` (new module, exported from `crates/core/src/lib.rs`)
Split into `mod.rs`, `worker.rs`, `tagger.rs`:

1. **Path & Validation (`crates/core/src/offline/mod.rs`)**:
   - `pub const DEFAULT_MAX_OFFLINE_BYTES: u64 = 5 * 1024 * 1024 * 1024;` (5 GB)
   - `pub const MAX_SINGLE_TRACK_BYTES: u64 = 512 * 1024 * 1024;` (512 MB DoS guard)
   - `pub fn validate_video_id(video_id: &str) -> CoreResult<()>` — strict `1..=64` ASCII `[A-Za-z0-9_-]`; reject `..`, `/`, `\`, NUL.
   - `pub fn container_ext(codec: AudioCodec, mime_type: &str) -> &'static str` — `"m4a"` for `AudioCodec::Aac` / `audio/mp4`, `"webm"` for `AudioCodec::Opus` / `audio/webm`.
   - `pub fn final_filename(video_id: &str, itag: u32, codec: AudioCodec, mime_type: &str) -> CoreResult<String>` — `{video_id}_{itag}.{ext}`.
   - `pub fn part_filename(final_name: &str) -> String` — `{final_name}.part`.
   - `pub fn resolve_confined_path(root: &Path, relative_name: &str) -> CoreResult<PathBuf>` — reject any `Component::ParentDir | RootDir | Prefix`, join with `root`, verify `symlink_metadata` is not a symlink, verify `canonicalize(parent) == canonicalize(root)`.
   - `pub fn redact_url(raw: &str) -> String` — strip `?query` and `#fragment` (keeps `scheme://host/path` only; returns `"<redacted-url>"` if unparseable).

2. **Atomic `.part` Download Worker (`crates/core/src/offline/worker.rs`)**:
   - Struct `DownloadManager`:
     ```rust
     pub struct DownloadManager {
         provider: Arc<dyn MusicProvider>,
         offline: Arc<dyn OfflineStore>,
         chunks: Arc<ChunkCache>,
         artwork: Arc<ArtworkCache>,
         lyrics: Arc<dyn LyricsProvider>,
         bus: EventBus,
         http: reqwest::Client,
         concurrency: Arc<Semaphore>, // default permits = 2
         max_offline_bytes: u64,
         active_jobs: Arc<tokio::sync::Mutex<HashMap<TrackId, JobHandle>>>,
     }
     ```
   - `struct JobHandle { cancel_tx: tokio::sync::watch::Sender<bool>, job_gen: u64 }`
   - Public async methods:
     - `pub fn new(...) -> CoreResult<Self>`
     - `pub async fn startup_recovery(&self) -> CoreResult<usize>`:
       - Reconciles DB rows in `Downloading`/`Tagging` -> `Paused` (if `.part` exists and length matches `bytes_downloaded`) or `Queued` (if `.part` missing).
       - Deletes orphan `.part` / `.tmp` files in `downloads_root()` whose `video_id` is absent from `downloads`.
     - `pub async fn enqueue(&self, track: Track) -> CoreResult<DownloadRecord>`:
       - Validates `video_id`. Idempotent: if already `Completed` and file exists & passes size check, returns existing record; if already `Downloading`/`Queued`, returns existing record.
       - Checks quota (`offline_stats.total_bytes < max_offline_bytes`).
       - Inserts `Queued` row, emits `AppEvent::DownloadUpdated`, spawns worker task.
     - `pub async fn pause(&self, video_id: &str) -> CoreResult<()>` — signals `cancel_tx.send(true)`, keeps `.part`, marks `Paused`.
     - `pub async fn resume(&self, video_id: &str) -> CoreResult<DownloadRecord>` — transitions `Paused`/`Failed` -> `Queued` and respawns worker resuming from `.part` byte offset.
     - `pub async fn cancel(&self, video_id: &str) -> CoreResult<()>` — signals `cancel_tx`, awaits/detaches, deletes `.part`, marks `Cancelled`.
     - `pub async fn delete(&self, video_id: &str) -> CoreResult<()>` — cancels active job first (waits for `active_jobs` removal or drops handle), deletes DB row via `delete_download_record`, removes finalized file + any `.part` file, emits `AppEvent::DownloadRemoved`.
   - Worker execution loop (`run_download_job`):
     1. Acquire `concurrency.acquire_owned().await` inside `tokio::select!` with `cancel_rx`.
     2. Resolve stream via `provider.resolve_stream(&track.id).await` (strictly inherits `ClientMatrix` `AuthMode` isolation; never attaches browser cookies to CDN GETs).
     3. Validate `stream.content_length <= MAX_SINGLE_TRACK_BYTES` and `used + content_length <= max_offline_bytes`.
     4. Open `<final>.part` with `OpenOptions::create(true).read(true).write(true)`. Determine resume offset `offset = part_len.min(total_bytes)` aligned down to `CHUNK_SIZE` (`256 * 1024`) via `set_len(aligned_offset)` + `seek(SeekFrom::Start(aligned_offset))` so resumed bytes never duplicate or gap.
     5. Loop `chunk_idx` from `aligned_offset / CHUNK_SIZE` until `offset >= total_bytes`:
        - Check `*cancel_rx.borrow()` before each chunk.
        - **ChunkCache Reuse**: call `chunks.read_chunk(&video_id, itag, chunk_idx)` first; if intact hit, write directly to `.part` with zero network I/O.
        - **Upstream Range Fetch**: `GET stream.url` with `Range: bytes={start}-{end}` inside `tokio::select! { _ = cancel_rx.changed() => Cancelled, resp = send => ... }`.
        - **HTTP 403 mid-download**: invalidate local `StreamInfo`, call `provider.resolve_stream(&video_id).await` (max 2 re-resolves per chunk), retry exact byte range without discarding `.part`.
        - **HTTP 429 mid-download**: parse `Retry-After` (clamped `2..=60` secs) else exponential backoff `1s, 2s, 4s, 8s` (+ jitter up to 250ms, max 4 attempts); cancel-aware sleep via `tokio::select!`; on exhaustion mark `Paused` with redacted error (keep `.part` for resume).
        - Write chunk bytes to `.part`, update `bytes_downloaded`, persist progress to SQLite + emit `AppEvent::DownloadUpdated` throttled to every `>= 256 KB` or `>= 250 ms`.
     6. Flush + `sync_all()` `.part` file handle and close it.
     7. Transition DB status to `DownloadStatus::Tagging` and emit `AppEvent::DownloadUpdated`.
     8. Fetch artwork bytes (`artwork.get(url, 512)` else single GET + `artwork.put`) and lyrics (`lyrics.fetch_lyrics(&track)`), both best-effort with 5s timeout.
     9. Run `tagger::embed_metadata(&part_path, &track, artwork_bytes.as_deref(), lyrics.as_ref())` inside `tokio::task::spawn_blocking`.
     10. Compute final `Sha256` hex digest + `final_len = metadata(&part_path).len()` on `spawn_blocking`.
     11. Check cancellation one final time; if cancelled, remove `.part` and exit cleanly.
     12. Atomic commit: `fs::rename(&part_path, &final_path)` (+ parent dir `sync_all()` on `#[cfg(unix)]`).
     13. Call `offline.mark_download_completed(...)` and emit `AppEvent::DownloadUpdated`.

3. **`lofty` Metadata / Art / Lyrics Tagger (`crates/core/src/offline/tagger.rs`)**:
   - `pub fn embed_metadata(part_path: &Path, track: &Track, artwork: Option<&[u8]>, lyrics: Option<&LyricsDocument>) -> CoreResult<bool>`
   - Uses `lofty::probe::Probe::open(part_path)?.guess_file_type()?.read()`:
     - Writable container check:
       - `FileType::Mp4` (`.m4a`, AAC itags `141/140/139`) -> `TagType::Mp4Ilst`
       - `FileType::Opus` / `FileType::Flac` / `FileType::Mpeg` -> primary tag type
       - `FileType::Matroska` (raw `.webm` Opus) or unsupported -> return `Ok(false)` without mutating bytes (non-fatal; metadata/lyrics remain in SQLite).
     - Writes via `lofty::tag::Accessor` + `ItemKey`:
       - `set_title(track.title.clone())`
       - `set_artist(track.artist_display())`
       - `set_album(album.title.clone())` (if `track.album.is_some()`)
       - `ItemKey::Lyrics`: serialized synced LRC (`[mm:ss.xx]line`) if `sync_kind != Plain`, else newline-joined plain text.
       - Front cover: `Picture::new_unchecked(PictureType::CoverFront, Some(mime_from_magic(bytes)), None, bytes.to_vec())` (accept only validated JPEG/PNG/WebP magic bytes, max 4 MB).
     - Calls `tag.save_to_path(part_path, WriteOptions::default())`.
     - If `lofty` returns an error, log warning (no URLs/secrets) and return `Ok(false)` (never corrupt audio: verify file length > 0; if tagging mutated/truncated on error, fail job so `.part` is not promoted). Better: write tags only when magic/header parses cleanly, and verify `Probe::open(part_path)` succeeds post-save before returning `Ok(true)`.

### `crates/core/src/proxy/mod.rs` — `LoopbackProxy` Local-Disk Bypass
- Extend `ProxyState` with `offline: Option<Arc<dyn OfflineStore>>`.
- Keep `LoopbackProxy::start(provider, chunks)` delegating to `LoopbackProxy::start_with_offline(provider, chunks, None)`.
- Add `pub async fn start_with_offline(provider: Arc<dyn MusicProvider>, chunks: Arc<ChunkCache>, offline: Option<Arc<dyn OfflineStore>>) -> CoreResult<Self>`.
- In `handle_request`:
  1. Validate `video_id` via `crate::offline::validate_video_id(video_id)` (return `400 Bad Request` on invalid characters or `..`).
  2. Enforce `request_gen < state.active_generation.load(Ordering::SeqCst)` -> `409 Conflict` BEFORE disk/network I/O.
  3. **Local-Disk Bypass (Priority 0)**:
     - Call `try_serve_offline_file(&state, video_id, range_header.as_deref(), request_gen).await`:
     - Query `offline.get_completed_download(video_id)`.
     - If `Some(rec)` with `Some(rel_path)`:
       - Resolve `path = resolve_confined_path(&offline.downloads_root(), &rel_path)`.
       - Open `tokio::fs::File::open(&path).await` immediately (holding open FD prevents mid-read race if file is unlinked on POSIX).
       - Check `file.metadata().await?.len()`: if `0` or `len != rec.total_bytes` (when `rec.total_bytes > 0`), log warning, call `offline.mark_download_failed(video_id, DownloadStatus::Failed, Some("offline file missing or truncated"), unix_secs())`, and return `None` to transparently fall through to `ChunkCache` / network!
       - Parse `Range` header via existing `parse_byte_range(hdr, file_len)` (`416 Range Not Satisfiable` on invalid range).
       - Seek to `req_start`, read exact slice `req_end - req_start + 1` in bounded 256 KB reads, re-checking `active_generation` between reads (return `409 Conflict` immediately on track skip).
       - Return `206 Partial Content` (or `200 OK` if full file) with `Content-Type: rec.mime_type`, `Accept-Ranges: bytes`, `Content-Range`, `Content-Length`, and custom internal header `X-Resonance-Source: offline-disk` (useful for deterministic test assertions).
     - Zero calls to `MusicProvider::resolve_stream`, zero `reqwest` traffic, zero `ChunkCache` writes.

### `crates/core/src/playback/mod.rs` — Offline Fast-Path & 50% Pre-Resolution Bypass
- Add `offline: Option<Arc<dyn OfflineStore>>` field to `PlaybackController` (default `None` in `new()`).
- Add builder `pub fn with_offline_store(mut self, offline: Arc<dyn OfflineStore>) -> Self`.
- In `load_and_play_current`:
  - Before `self.provider.resolve_stream(&track.id).await`, check `self.offline.as_ref().and_then(|o| o.get_completed_download(&track.id).ok().flatten())`:
  - If `Some(rec)` and `resolve_confined_path(&o.downloads_root(), rel).is_ok_and(|p| p.is_file())`:
    - Skip network resolution completely; use `(self.proxy.stream_url_with_gen(&track.id, rec.itag, proxy_gen), track.duration_ms, true)`.
- In `tick_position` (50% next-track pre-resolution):
  - Skip spawning `provider.resolve_stream(&next_video_id)` if `next_video_id` already has a `Completed` download in `self.offline`.

### `platform/desktop.rs` & `platform/android.rs`
- `platform/desktop.rs`: add `pub fn ensure_downloads_dir(data_dir: &Path) -> std::io::Result<PathBuf>` creating `data_dir.join("downloads")` (mode `0o700` on Unix).
- `platform/android.rs`: add `pub fn ensure_downloads_dir(data_dir: &Path) -> std::io::Result<PathBuf>` creating `data_dir.join("downloads")` + idempotently writing empty `.nomedia` sentinel file so Android `MediaStore` does not scan internal downloads.

### `src/live_engine.rs`, `src/store.rs`, `src/screens/library.rs`
- `src/live_engine.rs`:
  - Fix `SqliteCacheStore::open(&dir)` (pass directory `dir`, not `dir.join("resonance.db")`).
  - Instantiate `ArtworkCache`, `DownloadManager`, call `rt.block_on(downloads.startup_recovery())`.
  - Pass `Some(cache.clone())` to `LoopbackProxy::start_with_offline` and `.with_offline_store(cache.clone())` to `PlaybackController`.
  - Subscribe to `bus.subscribe()` on the background Tokio runtime to forward `AppEvent::DownloadUpdated` -> `LiveUpdate::DownloadUpdated` and `AppEvent::DownloadRemoved` -> `LiveUpdate::DownloadRemoved`.
  - Add `LiveUpdate` variants: `DownloadsLoaded(Vec<DownloadRecord>)`, `DownloadUpdated(DownloadRecord)`, `DownloadRemoved(TrackId)`.
  - Add non-blocking dispatchers: `dispatch_enqueue_download(Track)`, `dispatch_pause_download(TrackId)`, `dispatch_resume_download(TrackId)`, `dispatch_cancel_download(TrackId)`, `dispatch_delete_download(TrackId)`.
- `src/store.rs`:
  - Add `pub downloads: HashMap<TrackId, DownloadRecord>` to `AppState` (zero blocking I/O on UI thread).
  - Handle new `LiveUpdate` variants in `apply_live_update`.
  - Expose `downloads_list() -> Vec<DownloadRecord>`, `download_record(video_id: &str) -> Option<DownloadRecord>`, `enqueue_download(track: Track)`, `pause_download(video_id: &str)`, `resume_download(video_id: &str)`, `delete_download(video_id: &str)`.
- `src/screens/library.rs`:
  - Replace `LibraryFilter::Downloads` heuristic (`audio_chunk_count > 0`) with `store::downloads_list()` filtered to `Completed` + active progress rows (all controls `>= 48dp`, theme tokens only, zero raw colors).

---

## 3. Security & Failure Matrix

| Scenario / Threat | Detection & Required Behavior | Invariant Enforced |
| :--- | :--- | :--- |
| **HTTP 403 mid-download** (expired ~6h `googlevideo.com` URL or Wi-Fi/5G IP handoff) | Catch `403` on chunk fetch, drop stale `StreamInfo`, call `MusicProvider::resolve_stream(&video_id)` (max 2 re-resolves/chunk), retry exact `Range: bytes=start-end` without truncating `.part`. Fail to `Paused` (preserving `.part`) if retries exhaust. | Resumable download; `AuthMode` isolation stays inside `InnerTubeClient`. |
| **HTTP 429 rate limit** during download | Honor `Retry-After` clamped to `[2s, 60s]`, else exponential backoff `1s/2s/4s/8s` + jitter (max 4 retries) inside cancel-aware `tokio::select!`. On exhaustion transition to `Paused` (retaining `.part`) with redacted `"Rate limited (429)"` message. | Zero spin-loops; immediate abort on cancel/pause. |
| **Track switch race on `LoopbackProxy` offline read** | `PlaybackController::load_and_play_current` calls `proxy.advance_generation()` + increments `play_generation`. `try_serve_offline_file` checks `request_gen < active_generation` before opening FD AND between 256 KB read slices, returning `409 Conflict` immediately. | Monotonic cancellation; superseded track never blocks or clobbers active audio. |
| **Delete download while `LoopbackProxy` is streaming it** | `LoopbackProxy` opens FD once per request; on Unix, unlinking while FD is open completes the in-flight range safely; subsequent requests see DB row removed and fall through to `ChunkCache`/network (or `502` if offline). `delete()` cancels active writer job BEFORE unlinking `.part`/final file. | Zero panics, zero torn reads, no half-written file left behind. |
| **Concurrent enqueue / duplicate download for same `video_id`** | Serialize job registration under `active_jobs: Mutex<HashMap<TrackId, JobHandle>>` + SQLite `PRIMARY KEY(video_id)`. Second `enqueue` returns existing record without spawning a second writer to the same `.part` file. | Single writer per `.part` file. |
| **Crash / kill during download or `lofty` tagging** | Writer mutates only `<id>_<itag>.<ext>.part`; final filename only appears via atomic `fs::rename` AFTER `fsync` + tagging + SHA-256. On next boot `startup_recovery()` transitions `Downloading`/`Tagging` -> `Paused`/`Queued` and removes orphan `.part` files. | `LoopbackProxy` never serves partial or half-tagged files. |
| **Corrupt / externally deleted finalized file** | `try_serve_offline_file` checks file existence and `metadata.len() == rec.total_bytes`; on mismatch marks row `Failed` (`"offline file missing or truncated"`) and transparently falls through to `ChunkCache`/network. | Self-healing storage; never serves truncated audio. |
| **Signed URL / cookie / secret leakage** | `StreamInfo.url` is NEVER stored in SQLite `downloads` table. All errors and `log::*!` calls pass URLs through `redact_url()` (strips `?sig=`, `?n=`, `?pot=`, `?ip=`). Download HTTP client sends NO `Cookie` or `Authorization` (`SAPISIDHASH`) headers to `googlevideo.com`. | Zero secrets in SQLite, logs, `AppEvent`, or UI banners; `ANDROID_VR`/`IOS` `AnonymousOnly` untouched. |
| **Path traversal / symlink escape via `video_id`** | `validate_video_id` enforces `^[A-Za-z0-9_-]{1,64}$`. SQLite stores relative filename only. `resolve_confined_path` rejects `..`/absolute paths/symlinks and asserts canonical parent equals `downloads_root()`. | Confined strictly to app downloads directory. |
| **Disk exhaustion DoS** | Reject downloads with `content_length > 512 MB` (`MAX_SINGLE_TRACK_BYTES`) or `used + content_length > max_offline_bytes`. Artwork embed capped at 4 MB with magic-byte validation (JPEG/PNG/WebP only). | Bounded disk & RAM (`< 100 MB` RSS budget). |
| **WebM/Opus container tagging limitation** | If `lofty` reports `FileType::Matroska` (WebM) or non-writable format, skip file-tag write cleanly (`tags_embedded = false`) and preserve metadata/lyrics in SQLite. Never corrupt audio frames. | Guaranteed playable offline files across both AAC `.m4a` and Opus `.webm`. |

---

## 4. Offline Verification Tests

Add new integration test file `tests/offline_downloads.rs` (100% offline using local `hyper` mock upstream server + `tempfile::TempDir` + mock `MusicProvider` / `LyricsProvider` / `AudioBackend`).

### Exact Mocks
- `MockUpstreamCdn`: local `127.0.0.1:0` `hyper` server supporting `Range: bytes=start-end`, configurable script per `(chunk_index, attempt)` returning `206`, `403`, or `429` (with `Retry-After: 0` in tests), plus an `AtomicUsize` request counter.
- `MockDownloadProvider`: implements `MusicProvider`, returns `StreamInfo` pointing to `MockUpstreamCdn` with synthetic signed query params (`?sig=SECRET_SIG&n=SECRET_N&pot=SECRET_POT&ip=1.2.3.4`), increments `resolve_calls: AtomicUsize`, rotates URL version on re-resolve.
- `minimal_valid_m4a_bytes() -> Vec<u8>`: deterministic in-memory valid MP4/M4A fixture (`ftyp` + `moov`/`mvhd`/`trak`/`udta` + `mdat` payload >= 512 KB to span >= 2 chunks) readable & writable by `lofty`.

### Exact Test Cases in `tests/offline_downloads.rs`
1. `sqlite_v1_to_v2_migration_preserves_existing_data_and_creates_downloads_table`
   - Opens temp DB, initializes v1 schema with `kv_store` + `listening_history` rows, re-opens via `SqliteCacheStore::open`, asserts `PRAGMA user_version == 2`, prior rows intact, and `OfflineStore` CRUD works; asserts `clear_all_caches()` does NOT delete `downloads` rows.
2. `atomic_part_download_commits_final_file_and_embeds_lofty_m4a_tags`
   - Runs `DownloadManager::enqueue` against `MockUpstreamCdn` serving `minimal_valid_m4a_bytes()` + PNG cover + synced `LyricsDocument`.
   - Asserts `.part` file does not exist post-completion, finalized `<id>_140.m4a` exists, `sha256_hex` matches disk bytes, `tags_embedded == true`, and re-reading finalized file with `lofty::read_from_path` yields exact `title`, `artist`, `album`, `CoverFront` picture bytes, and synced LRC `ItemKey::Lyrics`.
3. `download_worker_reuses_chunk_cache_without_redundant_network_fetches`
   - Pre-populates chunk `0` in `ChunkCache`, enqueues download for 2-chunk track, asserts `MockUpstreamCdn` received range request ONLY for chunk `1` (request count == 1).
4. `download_worker_recovers_mid_download_http_403_via_url_reresolution`
   - Scripts `MockUpstreamCdn` to return `206` on chunk 0, `403` on chunk 1 first attempt, `206` on chunk 1 after re-resolved URL.
   - Asserts `resolve_calls == 2`, download reaches `Completed`, and final file SHA-256 matches full payload.
5. `download_worker_handles_http_429_backoff_and_preserves_part_on_exhaustion`
   - Scripts `429` for 2 attempts then `206` -> asserts `Completed`.
   - Second track scripts persistent `429` -> asserts status `Paused`, `.part` file preserved on disk with partial bytes, `resume()` completes download cleanly.
6. `download_cancel_and_pause_abort_cleanly_without_leaving_corrupt_final_file`
   - Blocks chunk 1 on a barrier, invokes `cancel(video_id)`, asserts `.part` removed, finalized file absent, status `Cancelled`; repeats with `pause(video_id)` and asserts `.part` retained and `resume()` finishes cleanly.
7. `startup_recovery_transitions_interrupted_downloads_and_purges_orphan_parts`
   - Inserts a `Downloading` DB row + matching `.part`, plus an orphan `orphan_140.m4a.part` file; calls `startup_recovery()`; asserts DB row becomes `Paused` and orphan `.part` is deleted.
8. `loopback_proxy_serves_completed_download_from_local_disk_with_zero_network_calls`
   - Completes download, resets `MockUpstreamCdn` + `MockDownloadProvider` counters to 0, starts `LoopbackProxy::start_with_offline`.
   - Issues full `GET` and `Range: bytes=1024-4095` requests; asserts `206 Partial Content`, `Content-Range`, `X-Resonance-Source: offline-disk`, exact byte equality, `resolve_calls == 0`, and `upstream_requests == 0`.
9. `loopback_proxy_enforces_generation_cancellation_and_self_heals_missing_offline_file`
   - Advances proxy generation and sends request with stale `gen=0` -> asserts `409 Conflict` without reading disk.
   - Deletes finalized file on disk behind `OfflineStore`'s back, issues proxy GET -> asserts row transitions to `Failed` and proxy transparently falls back to `MockDownloadProvider`/`MockUpstreamCdn` with `200 OK`.
10. `playback_controller_skips_provider_resolution_for_completed_offline_track`
    - Wires `PlaybackController::new(...).with_offline_store(cache)`.
    - Calls `play_track` on completed offline track -> asserts `resolve_calls == 0`, `snapshot.offline_mode == true`, `snapshot.state == PlayerState::Playing`.
    - Advances `tick_position` past 50% when next queued track is also offline -> asserts `resolve_calls` remains `0`.
11. `security_path_traversal_quota_and_secret_redaction_gates`
    - Rejects `"../etc/passwd"`, `"a/b"`, `""`, and 65-char IDs with `OfflineStorageError::InvalidVideoId` (and `400 Bad Request` on `LoopbackProxy`).
    - Rejects download exceeding `max_offline_bytes` with `OfflineStorageError::QuotaExceeded`.
    - Forces download failure and inspects SQLite `downloads` row + emitted `AppEvent::DownloadUpdated` + formatted `CoreError` strings: asserts `"SECRET_SIG"`, `"SECRET_N"`, `"SECRET_POT"`, `"SAPISID"` never appear anywhere.

### Exact `cargo test` Commands
```bash
cargo check --workspace
cargo test --test offline_downloads -- --nocapture
cargo test --test proxy_and_fault_injection --test core_unit_and_snapshots --test architecture_gates_v2
cargo test --workspace
```
