# P3: Production Offline Vault, Audio Tagging & Full YouTube Music Playlist CRUD

> **Wave:** A (Parallel)
> **Status of Existing Code:** `crates/core/src/offline/*` (`quota.rs`, `tagger.rs`, `vault.rs`, `worker.rs`), `crates/core/src/cache/db/offline.rs`, `crates/core/src/innertube/mutations.rs`, `src/store/mutation_slice.rs`, and `src/ui/controls/add_to_playlist_sheet.rs` exist, **but**:
> 1. `OfflineDownloadWorker` is never started on the Tokio runtime in `live_engine` with real HTTP range resume, atomic rename, artwork embedding, and quota eviction.
> 2. `LoopbackProxy` / `PlaybackController` needs deterministic offline-first file lookup so downloaded songs play instantaneously in Airplane Mode with zero network requests.
> 3. Playlist CRUD lacks full **Create Playlist**, **Rename / Edit Description / Privacy**, **Delete Playlist**, **Reorder tracks (`ACTION_MOVE_VIDEO_BEFORE`)**, **Remove tracks (`ACTION_REMOVE_VIDEO` using `setVideoId`)**, and **Offline Mutation Journal Replay** with optimistic rollback!
> **Exclusive Owned Paths:** `crates/core/src/offline/*`, `crates/core/src/innertube/mutations.rs`, `crates/core/src/cache/db/offline.rs`, `src/store/mutation_slice.rs`, `src/store/library_slice.rs`, `src/screens/library.rs`, `src/ui/controls/add_to_playlist_sheet.rs`, `src/live_engine/handlers/offline_mutations.rs`, `tests/p3_offline_mutations_test.rs`.

---

## 1. Deep Technical Research & Architecture

### 1.1 Crash-Safe Offline Vault Architecture (`crates/core/src/offline/`)

```
Enqueue(track, quality)
  │
  ▼
SQLite `offline_downloads` table (WAL mode, status = 'Queued')
  │
  ▼
OfflineWorkerPool (configurable concurrency: default 2 workers, Wi-Fi-only guard)
  ├─ 1. Quota Pre-Check (`quota.rs`): verify `used_bytes + est_bytes <= max_vault_bytes`
  │     (evict oldest unpinned LRU cache chunks if needed; NEVER evict user-pinned albums!)
  ├─ 2. Stream Resolve: call `HybridCatalogProvider::resolve_stream(video_id)` choosing
  │     preferred offline format (Opus 251 -> M4A 140; Premium Opus 774/AAC 141 when available)
  ├─ 3. Resumable Chunked Download -> `<vault_dir>/.tmp/<video_id>.part`
  │     ├─ Supports HTTP `Range: bytes=<offset>-` resume across app restarts/pauses
  │     ├─ Re-resolves stream URL once if mid-download 403 expiration occurs
  │     ├─ Verifies `Content-Length` match + running SHA-256 checksum
  │     └─ Throttled progress events (`DownloadProgress` max every 250ms -> NSH store)
  ├─ 4. Metadata & Cover Art Tagging (`tagger.rs` using `lofty`)
  │     ├─ Writes Title, Artist(s), Album, Track Number, Release Year, Video ID custom tag
  │     ├─ Embeds JPEG/WebP album artwork (resized <= 600x600px, <= 250KB)
  │     └─ Also caches synced lyrics JSON sidecar in SQLite `offline_lyrics` for offline karaoke!
  └─ 5. Atomic Commit: `fs::rename(.part -> <vault_dir>/<video_id>.<ext>)` + SQLite transaction
        updating `status = 'Completed'`, `file_path`, `sha256`, `file_size_bytes`.
```

### 1.2 Offline-First Playback Integration
- Before calling `resolve_stream()` over the network, `PlaybackController` queries `OfflineVault::completed_file_for(video_id)`:
  - If `Some(path)` and `path.exists()`: play directly from local file (via `PcmAudioEngine` local `File` source or `LoopbackProxy` `/local-vault/<video_id>` byte-range endpoint) — **0 ms network latency, works 100% offline**.
  - If file was deleted externally on disk: mark DB row `Failed("File missing on disk")`, notify user cleanly, and fall back to streaming if online.

### 1.3 Full YouTube Music Playlist CRUD & Offline Mutation Queue (`mutations.rs`)
YouTube Music InnerTube mutation endpoints (all require authenticated `WEB_REMIX` with `SAPISIDHASH` or valid TV OAuth bearer):

| Action | InnerTube Endpoint | Payload Contract | Notes |
| :--- | :--- | :--- | :--- |
| **Create Playlist** | `POST /youtubei/v1/playlist/create` | `{ title, description, privacyStatus: "PRIVATE"\|"UNLISTED"\|"PUBLIC", videoIds: [...] }` | Returns `{ playlistId }`; map temp local ID `local_pl_<uuid>` -> server `VLPL...` |
| **Add Track(s)** | `POST /youtubei/v1/browse/edit_playlist` | `{ playlistId, actions: [{ action: "ACTION_ADD_VIDEO", addedVideoId }] }` | Returns `playlistEditResults[].playlistEditVideoAddedResultData.setVideoId` — **MUST persist `setVideoId`!** |
| **Remove Track** | `POST /youtubei/v1/browse/edit_playlist` | `{ playlistId, actions: [{ action: "ACTION_REMOVE_VIDEO", setVideoId, removedVideoId }] }` | YouTube **requires `setVideoId`** (not just `videoId`) to remove a specific duplicate occurrence! |
| **Reorder Track** | `POST /youtubei/v1/browse/edit_playlist` | `{ playlistId, actions: [{ action: "ACTION_MOVE_VIDEO_BEFORE", setVideoId, movedSetVideoIdSuccessor }] }` | Omit `movedSetVideoIdSuccessor` to move to end of playlist |
| **Rename / Edit** | `POST /youtubei/v1/browse/edit_playlist` | `{ playlistId, actions: [{ action: "ACTION_SET_PLAYLIST_NAME", playlistName }, { action: "ACTION_SET_PLAYLIST_DESCRIPTION", playlistDescription }, { action: "ACTION_SET_PLAYLIST_PRIVACY", playlistPrivacy }] }` | Validates non-empty title (`<= 150` chars) |
| **Delete Playlist** | `POST /youtubei/v1/playlist/delete` | `{ playlistId }` | Guard against deleting system playlists (`LM` Liked Music) |
| **Like / Unlike / Dislike** | `POST /youtubei/v1/like/{like,removelike,dislike}` | `{ target: { videoId } }` | Already partially wired; add persistent retry journal |
| **Subscribe / Unsubscribe** | `POST /youtubei/v1/subscription/{subscribe,unsubscribe}` | `{ channelIds: [channelId] }` | Already partially wired; add persistent retry journal |

#### Persistent Optimistic Mutation Journal (SQLite `pending_mutations`)
- Every user write (`Like`, `AddToPlaylist`, `RemoveFromPlaylist`, `CreatePlaylist`, `Reorder`, `Rename`, `Delete`) is:
  1. Applied **optimistically** in `library_slice` / `mutation_slice` with an inverse rollback snapshot.
  2. Persisted to SQLite `pending_mutations(id, op_json, created_at, retry_count, next_retry_at)`.
  3. Flushed sequentially by a background `MutationSyncWorker` (respecting causal order: e.g., `CreatePlaylist` completes and rewrites `local_pl_<uuid>` -> real `playlistId` in subsequent queued `AddToPlaylist` ops!).
  4. On transient network error (`5xx`, timeout, offline): exponential backoff (`5s -> 15s -> 60s -> 5m`) and badge `"Pending sync (N)"` in Library.
  5. On permanent `4xx` rejection (`401`/`403`/`400`): execute rollback snapshot and surface a clear status banner explaining why the server rejected the edit.

---

## 2. Concrete Implementation Plan (Step-by-Step)

### Step 1: Hardened Offline Vault & Worker (`crates/core/src/offline/`)
- Complete `worker.rs`:
  - Respect `OfflinePolicy { max_vault_bytes, wifi_only, auto_download_liked, preferred_bitrate, concurrency: 2 }`.
  - Use `tokio::sync::Semaphore` for concurrency limit + `HashMap<String, CancellationToken>` for instant Pause / Cancel / Delete.
  - Cleanup orphaned `.part` files older than 7 days on startup (`vault.rs::sweep_orphans()`).
  - Verify `tagger.rs` uses `lofty::Probe` to write Vorbis comments (Opus/FLAC) and iTunes MP4 atoms (M4A) cleanly without corrupting container headers (test re-opening tagged file with Symphonia!).
- Complete `CollectionPinManager`: pinning an Album or Playlist automatically enqueues all missing tracks, tracks aggregate `CollectionOfflineProgress { completed, total, failed, bytes }`, and unpinning removes tracks only if not pinned by another collection!

### Step 2: Full Playlist CRUD & Journal (`mutations.rs` + `offline.rs`)
- Add SQLite migration for `pending_mutations` and `playlist_track_set_ids(playlist_id, video_id, set_video_id, position)`.
- Implement `create_playlist`, `rename_playlist`, `delete_playlist`, `add_to_playlist`, `remove_from_playlist`, `reorder_playlist_track` in `crates/core/src/innertube/mutations.rs`.
- Parse `setVideoId` when loading playlist details (`parser/browse.rs` / `ytmapi_adapter.rs`) and after `ACTION_ADD_VIDEO` so removal/reordering works without extra round-trips.

### Step 3: UI Integration (`library.rs`, `add_to_playlist_sheet.rs`, `handlers/offline_mutations.rs`)
- **`add_to_playlist_sheet.rs`:**
  - Add inline `"+ New Playlist"` form (Title input, Privacy selector `Private / Unlisted / Public`, `"Create & Add"` primary button).
  - Show real playlists from `library_slice` (zero hardcoded stubs), with duplicate-track warning pill if track is already inside that playlist.
- **`library.rs`:**
  - Add `"Downloads"` filter tab showing storage quota bar, active download progress rows (Pause / Resume / Cancel / Delete buttons `>= 48dp`), `"Pause All"` / `"Resume All"`, and pinned Offline Albums/Playlists.
  - Add `"+ New Playlist"` CTA in Playlists tab + playlist overflow menu (`Rename`, `Delete`, `Pin for Offline`).
  - Show subtle `"Syncing N changes…"` pill only when `pending_mutations_count > 0`.

---

## 3. Acceptance Criteria & Verification Tests (`tests/p3_offline_mutations_test.rs`)

1. **Resumable Atomic Download Test:** Serve a 1 MB binary payload from local mock HTTP server that drops connection at byte 400,000; pause/resume worker -> assert HTTP `Range: bytes=400000-` resumes cleanly, verifies SHA-256, renames `.part` atomically, and leaves zero `.part` files behind.
2. **Tagging Round-Trip Test:** Tag a real minimal `.m4a` and `.opus` file via `tagger.rs` with title/artist/album/artwork -> re-read tags with `lofty` AND decode audio frames -> assert metadata matches 100% and container remains valid.
3. **Reference-Counted Unpinning Test:** Pin Album A (`[t1, t2]`) and Playlist B (`[t2, t3]`). Unpin Album A -> assert `t1` is deleted, `t2` is preserved because Playlist B still pins it.
4. **Causal Offline Mutation Replay Test:** While offline, queue `CreatePlaylist("Roadtrip")` (`temp_id`) + `AddTrack(temp_id, "vid1")`. Come online -> verify mock InnerTube receives `/playlist/create` returning `VLPL_REAL`, then `/browse/edit_playlist` with `playlistId == "VLPL_REAL"` and stores returned `setVideoId`.
5. **4xx Rollback Test:** Mock server returns `403` on `RenamePlaylist` -> verify local title reverts to previous name and status banner reports failure honestly.
