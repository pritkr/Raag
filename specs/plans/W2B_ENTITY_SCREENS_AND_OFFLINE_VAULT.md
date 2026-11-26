# W2B — Dedicated Artist & Album Entity Screens & First-Class Offline Vault UX

> **Wave:** Wave 2 (Parallel Agent 2B — Runs concurrently with W2A, W2C, W2D after Wave 1.5 Stitch)
> **Exclusive Write Scope:**
> - `src/screens/{artist.rs,album.rs}` (new)
> - `src/ui/controls/vault_widgets.rs` (new reusable Pin/Quota/Progress controls)
> - `crates/core/src/downloads/{vault.rs,quota.rs,state.rs}` (`quota.rs` new, extend `vault.rs` cleanly)
> - `tests/w2b_entities_and_offline_vault_test.rs` (new)
> **Read-Only Dependencies:** `ArtistDetail` & `AlbumDetail` (`crates/contract/src/catalog_ext.rs`), `CatalogExtProvider` (W1C), `OfflineVault` (`crates/core/src/downloads/vault.rs`), `SettingsV1` (`offline_quota_bytes`, `wifi_only_downloads`, `keep_liked_songs_offline`).
> **Stitch Hooks (For Wave 2.5 Integration Agent):**
> - Add `EntityRoute::{Artist(String), Album(String)}` navigation stack hooks into `src/store/mod.rs` and `src/screens/shell.rs`
> - Wire `"Go to Artist"` / `"Go to Album"` from W1A `track_context_menu` and clickable artist/album links across Home/Search/Now Playing
> - Mount `vault_widgets::offline_storage_settings_card()` in `src/screens/settings.rs` and `"Pinned Offline"` section in `src/screens/library.rs`
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{shell,settings,home,library,now_playing,queue,lyrics,explore,search}.rs`, `platform/**`.

---

## 1. Ground-Truth Gaps Addressed

1. **No Real Entity Navigation:** Clicking an artist anywhere currently triggers a crude text search (`run_search(artist_name)`), and clicking an album opens a generic `"PLAYLIST"` detail screen inside `AppTab::Home`—losing release year, disc/track numbers, artist discography, singles vs. albums separation, and related artists.
2. **Dormant `OfflineVault`:** `crates/core/src/downloads/vault.rs` already implements `pinned_collections` in SQLite, SHA-256 disk verification, and `DownloadManager` delegation—yet `live_engine.rs` marks it `#[allow(dead_code)]`, unpinning does not ref-count orphaned tracks, and storage quotas / Wi-Fi-only rules aren't enforced!

---

## 2. Design Taste & Entity Screen Specifications

### 2.1 Dedicated Artist Screen (`src/screens/artist.rs`)

**Visual & Interaction Architecture (Following NSH Design Guides):**
- **Adaptive Hero Header:**
  - Full-bleed responsive header tinted subtly via `AmbientPalette` extracted from the artist's banner/avatar image (never harsh neon, legible in both Light and Dark themes).
  - Top bar: Back button (`>= 48dp` on mobile, native back stack pop), Artist Name (`28–36pt` bold, `maxFontSizeMultiplier` capped), monthly listeners / subscriber count in `muted_foreground`.
  - **Primary Action Row:**
    - `Shuffle` (Primary filled button — loads top songs / shuffle playlist with W0 Fisher–Yates shuffle)
    - `Start Radio` (Secondary outline button — seeds `autoplay_next` with artist radio ID)
    - `Subscribe / Subscribed` (Stateful toggle button with optimistic update & rollback)
- **Structured Content Hierarchy:**
  1. **Top Songs** (shows top 5 compact tracks with `"Show all"` expansion toggle; each row uses the shared `track_row_view` with artwork, like, download, and `...` context menu).
  2. **Albums** (horizontal scrollable card carousel on mobile, responsive multi-column grid on desktop; cards show square artwork, title, and release year; tapping pushes `EntityRoute::Album(browse_id)`).
  3. **Singles & EPs** (separate carousel so flagship studio albums aren't buried under 40 singles).
  4. **Music Videos** (when present, 16:9 thumbnail cards).
  5. **Fans Also Like (Related Artists)** (circular avatar cards pushing `EntityRoute::Artist(channel_id)`).
  6. **About** (collapsible artist bio truncated to 3 lines with `"Read more"` toggle).
- **States:** Structured skeleton layout while loading (`artist.loading`), offline cached SWR fallback banner if viewed offline, clean error retry card on network failure.

### 2.2 Dedicated Album Screen (`src/screens/album.rs`)

**Visual & Interaction Architecture:**
- **Desktop Split Header / Mobile Centered Hero:**
  - Crisp square cover art (`220px` desktop / `240px` mobile, concentric `radius_card`, subtle elevation).
  - Metadata block: `ALBUM` / `EP` / `SINGLE` type eyebrow, Album Title, clickable Artist link (pushes `EntityRoute::Artist`), `Year · N tracks · Total Duration`, Explicit `[E]` badge when applicable.
  - **Action Row:**
    - `Play` (Primary button — replaces queue with album tracks in exact disc/track order, turns off shuffle cleanly)
    - `Shuffle` (Secondary button — W0 Fisher–Yates shuffle)
    - `Pin Offline` (`vault_widgets::collection_pin_button(...)` — one-tap Offline Vault pin showing live `"Pinned (7/12)"` → `"Available Offline"` badge!)
    - `Save to Library` (Heart/Bookmark toggle) + `...` Overflow (`Play Next`, `Add Album to Queue`, `Add All to Playlist`).
- **Numbered Tracklist:**
  - Uses track index column (`01`, `02`, ... in `tabular_nums()`) instead of repeating identical album cover thumbnails on every single row!
  - Multi-disc albums display clean hairline `Disc 1` / `Disc 2` section headers.
  - Highlights currently playing track with subtle equalizer icon in the number lane.
- **Footer Section:** `"Other Versions"` (Deluxe / Explicit / Clean editions) and `"More by {Artist}"` carousel.

---

## 3. First-Class Offline Vault, Ref-Counted Cleanup & Quota Engine

### 3.1 Reference-Counted Orphan-Safe Unpinning (`crates/core/src/downloads/vault.rs`)

**Critical Edge Case:** Suppose Track X belongs to both **Pinned Album A** and **Pinned Playlist B** (or was explicitly downloaded by the user as a standalone track). When the user unpins Album A, Track X **MUST NOT** be deleted!

**Schema & Algorithm:**
- Track standalone manual downloads with a lightweight SQLite column or metadata set (`explicit_download: bool` on `downloads` or `pinned_collections` row `id = "__explicit_singles__"`).
- `OfflineVault::unpin_collection_with_cleanup(&self, collection_id: &str) -> CoreResult<UnpinReport>`:
  1. Fetch track IDs belonging to `collection_id`.
  2. Delete `collection_id` from `pinned_collections` inside a SQLite transaction.
  3. For each track ID from step 1: check `self.db.is_track_pinned(&video_id)?` (which checks all remaining rows in `pinned_collections` including `"__explicit_singles__"` and `"__liked_songs__"` if `keep_liked_songs_offline` is enabled).
  4. Only tracks with **reference count == 0** are deleted via `self.manager.delete(&video_id).await` (removing file + SQLite row + freeing quota bytes).
  5. Return `UnpinReport { unpinned_id, deleted_tracks, retained_shared_tracks, reclaimed_bytes }` so the UI can show an honest toast: `"Removed offline album · Freed 84 MB (2 tracks kept by other playlists)"`.

### 3.2 Smart Quota & Network Policy Engine (`crates/core/src/downloads/quota.rs`)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaAdmission {
    Allowed { remaining_bytes: u64 },
    BlockedQuotaExceeded { used_bytes: u64, quota_bytes: u64, needed_bytes: u64 },
    DeferredMeteredNetwork,
}

pub struct VaultQuotaGuard {
    pub quota_bytes: u64,          // 0 = Unlimited
    pub wifi_only: bool,
    pub is_metered_network: bool,  // fed by OS/Android ConnectivityManager or Desktop hint
}
```

- **Pre-Enqueue Admission Check:** Before downloading a track or pinning a collection, estimate required bytes (`duration_ms * target_kbps / 8` when `content_length == 0`, default ~4.5 MB/track at 160 kbps).
  - If `quota_bytes > 0 && used + needed > quota_bytes`: return `BlockedQuotaExceeded` and surface a friendly NSH `alert_dialog()` offering `"Increase Storage Limit"` or `"Manage Downloads"` — never silently fail halfway through an album!
  - If `wifi_only && is_metered_network`: enqueue tracks in `DownloadStatus::Paused` with clear status `"Waiting for Wi-Fi"` and auto-resume when network flips to unmetered.
- **Auto-Sync `"Keep Liked Songs Offline"`:**
  - When `SettingsV1::keep_liked_songs_offline == true`, every `LiveUpdate::LibraryLoaded` / `ToggleLike(true)` automatically pins `id = "__liked_songs__"` and diffs newly liked songs into the download queue (respecting quota & Wi-Fi rules).
- **SHA-256 Vault Integrity Audit & Repair:**
  - `OfflineVault::audit_all_tracks()` iterates completed records, verifies file existence + SHA-256 hash against SQLite (`verify_track_on_disk`), marks any corrupted/missing file `Failed("File corrupted — ready to re-download")`, and provides one-tap `"Repair N Tracks"` in Settings → Storage.
- **Embedded Metadata & Lyrics Bundling (`lofty = "0.22"`):**
  - Ensure downloaded files carry embedded Title, Artist, Album, Track Number, Cover Art (`ArtworkCache`), and synced LRC lyrics (`USLT` / Vorbis `LYRICS`) so offline playback shows full artwork & lyrics with zero network!

### 3.3 Reusable Vault UI Components (`src/ui/controls/vault_widgets.rs`)
- `collection_pin_button(collection_id, kind, title, tracks)`: 3 states (`Pin Offline`, `Downloading (4/10) · Cancel`, `Pinned Offline · Tap to Unpin` with native `alert_dialog` confirm).
- `offline_storage_settings_card()`: Segmented storage usage bar (`Offline Vault` vs `Transient Playback Cache` vs `Free Quota`), Quota selector (`2 GB`, `5 GB`, `10 GB`, `25 GB`, `Unlimited`), `Download on Wi-Fi only` switch, `Keep Liked Songs Offline` switch, `Audio Quality` picker (`Data Saver 64k`, `Standard 160k`, `High 256k`), and `"Verify Offline Files"` button.

---

## 4. Deterministic Offline Verification (`tests/w2b_entities_and_offline_vault_test.rs`)

1. `ref_counted_unpin_retains_shared_tracks_and_deletes_orphans` — pins Album A (`[t1, t2, t3]`) and Playlist B (`[t2, t4]`); unpins Album A; asserts `t1` and `t3` deleted from disk & DB while `t2` and `t4` remain intact and verified.
2. `quota_guard_blocks_over_budget_and_defers_on_metered_wifi_only` — tests exact byte threshold admission, unlimited (`0`) bypass, and `DeferredMeteredNetwork` pause/resume transition.
3. `vault_integrity_audit_detects_bitrot_and_marks_repairable` — flips 1 byte in a committed offline file on disk, runs `audit_all_tracks()`, asserts `IntegrityMismatch` detected cleanly without panicking.
4. `artist_and_album_screens_render_full_hierarchy_in_headless_nsh` — renders `artist_screen` and `album_screen` with multi-disc album & artist fixture data in `nsh_testing::Headless`, verifying numbered tracklist, disc headers, Pin Offline button, and zero raw color literals.

```bash
cargo test --test w2b_entities_and_offline_vault_test -- --nocapture
cargo test --test offline_store_test
```
