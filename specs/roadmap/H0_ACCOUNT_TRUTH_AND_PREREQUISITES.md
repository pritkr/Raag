# H0 — "Account Truth" & Pre-Requisites (Wave 0 Sequential Foundation — Weeks 1–2)

> **Wave:** Wave 0 (Mandatory Sequential Foundation — Execute **strictly in order H0.0 → H0.5** BEFORE fanning out Wave 1 parallel Hydrogen subagents!)
> **Why Sequential First:** Every downstream feature (ST-1..ST-4, H2, H3, LT-1..LT-3) compounds on real persisted OS paths, real persisted settings, real hydrated account library state, working download/playlist UI surfaces, and a green CI baseline.
> **Exclusive Write Scope (Wave 0):**
> - **H0.0:** `Cargo.toml`, `crates/*/Cargo.toml`, `crates/contract/src/{lib.rs,storage.rs,catalog_ext.rs,settings.rs,rooms.rs,local_media.rs,updater.rs,casting.rs}`, `crates/core/src/storage/{mod.rs,paths.rs}` (new), `src/live_engine.rs` (paths + H0.1–H0.5 runtime hooks), `src/main.rs`, `README.md`, `ARCHITECTURE.md`, `AGENTS.md`, `src/AGENTS.md`, `tests/AGENTS.md`, `.github/workflows/release.yml`, `tests/architecture_gates_v2.rs`, `src/bin/nsh_preview.rs`, `src/App.tsx` (version sync + `cargo fmt --all`).
> - **H0.1–H0.5:** `crates/core/src/cache/db.rs` (preserve `settings:%` during `clear_all_caches()`), `src/store/{mod.rs,library_slice.rs,settings_slice.rs,mutation_slice.rs,ui_slice.rs}`, `src/ui/controls/{media_rows.rs,sheets.rs}`, `src/screens/{now_playing.rs,library.rs,settings.rs,shell.rs}`, `tests/h0_account_truth_test.rs` (new).
> - **Explicitly Deferred (Zero Duplication):**
>   - Live `yt-dlp-ejs` / Premium probe / Ed25519 remote config → **ST-1**
>   - `ytmapi-rs` / `rustypipe` provider backend replacement & live Explore/Charts → **ST-3**
>   - Dedicated Artist & Album Entity screens, Vault quota UX, Real Listen Together → **H2**
>   - Embedded Desktop WebView & TV OAuth sign-in, Local-file import → **H3**

---

## 1. Execution Sequence Overview

```text
Day 1        ┌──────────────────────────────────────────────────────────────┐
             │ H0.0: Unblock CI + NSH Version Sync + Persistent AppPaths    │
             │       + Wave 0 Additive `crates/contract/` Freeze            │
             └──────────────────────────────┬───────────────────────────────┘
                                            ▼
Days 2–4     ┌──────────────────────────────────────────────────────────────┐
             │ H0.1: Boot-Time Library Hydration + SQLite Offline Snapshot  │
             └──────────────────────────────┬───────────────────────────────┘
                                            ▼
Days 4–6     ┌──────────────────────────────────────────────────────────────┐
             │ H0.2: SettingsV1 Persistence via CacheStore::put_metadata    │
             │       (Protected from `clear_all_caches()`)                  │
             └──────────────────────────────┬───────────────────────────────┘
                                            ▼
Days 6–8     ┌──────────────────────────────────────────────────────────────┐
             │ H0.3: Download Action Surface (Track Rows + Now Playing)     │
             └──────────────────────────────┬───────────────────────────────┘
                                            ▼
Days 8–11    ┌──────────────────────────────────────────────────────────────┐
             │ H0.4: Playlist Writes ("Add to Playlist" Sheet, Remove,      │
             │       Reorder + Optimistic Generation Rollback)              │
             └──────────────────────────────┬───────────────────────────────┘
                                            ▼
Days 11–14   ┌──────────────────────────────────────────────────────────────┐
             │ H0.5: Wire `OfflineVault` ("Pinned Collections") + Honest    │
             │       Rename "Listen Together" → "Radio Mix"                 │
             └──────────────────────────────────────────────────────────────┘
```

---

## 2. Detailed Specification per Milestone

### 2.1 H0.0 — Unblock CI, Single-Source NSH Version Sync, Persistent `AppPaths` & Wave 0 Contract Freeze (Day 1)

#### A. CI Formatting & Single-Source-of-Truth NSH Version Sync
1. Run `cargo fmt --all` across the entire workspace so `cargo fmt --all -- --check` passes cleanly on Day 1.
2. **NSH Version Audit & Sync (`0.7.0`):**
   - Current repository state pins `=0.6.1` in `Cargo.toml`, `Cargo.lock`, `tests/architecture_gates_v2.rs:45-46,92`, `README.md`, `ARCHITECTURE.md`, `AGENTS.md`, `src/AGENTS.md`, `tests/AGENTS.md`, `.github/workflows/release.yml`, `src/bin/nsh_preview.rs:581`, and `src/App.tsx:446`.
   - **Rule:** Never let docs/CI say `0.7.0` while `Cargo.toml` pins `=0.6.1` (that causes `nsh-cli` / release build mismatch and breaks `tests/architecture_gates_v2.rs`).
   - **Action:**
     1. Upgrade `nsh-core`, `nsh-base`, `nsh-engine`, `nsh-testing` in root `Cargo.toml` to `=0.7.0` and run `cargo update -p nsh-core -p nsh-base -p nsh-engine -p nsh-testing` to verify `0.7.0` resolves cleanly on crates.io and check for any NSH `0.7.0` API deltas (`cargo check --workspace --all-targets`). *(If `0.7.0` is not yet published on the target registry, keep `=0.6.1` everywhere uniformly rather than creating drift; otherwise update ALL of the following atomically to `0.7.0`:)*
     2. Sync the exact pinned version string (`0.7.0`) across:
        - `Cargo.toml` & `Cargo.lock`
        - `README.md`
        - `ARCHITECTURE.md`
        - `.github/workflows/release.yml` (`nsh-cli` install version & build metadata)
        - `AGENTS.md`, `src/AGENTS.md`, `tests/AGENTS.md`
        - `tests/architecture_gates_v2.rs` (lines 2, 45, 46, 92)
        - `src/bin/nsh_preview.rs` (line 581) & `src/App.tsx` (line 446)

#### B. Canonical Persistent Storage (`AppPaths` — Single Authoritative Owner)
Replace all production `std::env::temp_dir()` usages (`src/live_engine.rs:96` and `src/main.rs:28`; note `src/bin/client_probe.rs:119` is an ephemeral CI probe scratch dir and stays temp) with one canonical `crates/core/src/storage/paths.rs` module consumed by every downstream spec (ST-1, ST-4, H2, H3, LT-1, LT-2):

```rust
// crates/contract/src/storage.rs
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// Canonical OS-compliant directory layout for Resonance (`Raag`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPaths {
    /// Persistent user state: `resonance.db`, fallback `auth_session.json`, `downloads/`.
    pub data_dir: PathBuf,
    /// Evictable caches: `chunks/`, `artwork/`, `js_solver/`, `ejs/`, `remote_config/`, `rustypipe/`.
    pub cache_dir: PathBuf,
    /// User-editable configuration overrides (optional `client_matrix.override.json`).
    pub config_dir: PathBuf,
    /// Dedicated offline media root (`data_dir/downloads`, `0o700` on Unix, `.nomedia` on Android).
    pub downloads_dir: PathBuf,
}
```

- **Resolution Precedence (`crates/core/src/storage/paths.rs::resolve_app_paths()`):**
  1. `RESONANCE_DATA_DIR` environment variable (preserves deterministic tests, CI, and portable installs; derives `cache_dir = data_dir/cache`, `config_dir = data_dir/config`, `downloads_dir = data_dir/downloads`).
  2. Registered Android native paths (`OnceLock<AppPaths>` set via `set_android_app_paths(files_dir, cache_dir)` from ST-4's JNI `nativeInitPaths`).
  3. Desktop OS-standard directories via `directories::ProjectDirs::from("dev", "resonance", "Raag")`:
     - **macOS:** Data `~/Library/Application Support/dev.resonance.Raag`, Cache `~/Library/Caches/dev.resonance.Raag`, Config `~/Library/Application Support/dev.resonance.Raag`
     - **Linux:** Data `$XDG_DATA_HOME/raag` (`~/.local/share/raag`), Cache `$XDG_CACHE_HOME/raag` (`~/.cache/raag`), Config `$XDG_CONFIG_HOME/raag`
     - **Windows:** Data `%LOCALAPPDATA%\resonance\Raag\data` (or `data_local_dir()`), Cache `%LOCALAPPDATA%\resonance\Raag\cache`, Config `%APPDATA%\resonance\Raag\config`
  4. Last-resort headless container fallback: `$HOME/.raag` (never `temp_dir()` unless `$HOME` itself is unset).
- **Hardening & One-Time Migration:**
  - Create all 4 directories with `0o700` permissions on Unix (`PermissionsExt::from_mode(0o700)`).
  - Write empty `.nomedia` sentinel inside `downloads_dir`, `cache_dir/chunks`, and `cache_dir/artwork` so Android `MediaStore` never indexes transient chunks.
  - **Idempotent Legacy Migration (`migrate_legacy_temp_dir_if_needed(&paths)`):** If `paths.data_dir/resonance.db` does **not** exist AND `temp_dir()/resonance-ytmusic-live/resonance.db` exists, copy `resonance.db`, `auth_session.json`, and `downloads/` atomically into `paths.data_dir`, write `.migrated_from_temp_v1`, and emit an info log (zero secrets). Never overwrite existing persistent user data.

#### C. Wave 0 Additive Contract & Dependency Freeze
In the same Wave 0 commit, add ALL additive `crates/contract/src/` modules (`storage.rs`, `settings.rs`, `catalog_ext.rs`, `rooms.rs`, `local_media.rs`, `updater.rs`, `casting.rs`) and workspace `Cargo.toml` dependencies (all `default-features = false` + `rustls` — see `00_MASTER_EXECUTION_ORCHESTRATOR.md` §4 & §6) so Wave 1 and Wave 2 Hydrogen subagents NEVER contend on `Cargo.toml` or `crates/contract/src/lib.rs`!

---

### 2.2 H0.1 — Boot-Time Library Hydration & Offline SQLite Snapshot

#### Problem
`InnerTubeClient::liked_songs()` (`crates/core/src/innertube/client.rs:578`) and `browse_collection("FEmusic_liked_playlists")` already exist, yet `src/live_engine.rs` NEVER calls them on boot or after sign-in. Consequently, signed-in users launch to an empty Library until they manually search, and cold-start offline launches show zero account truth.

#### Architecture & Data Flow
1. **Contract Snapshot (`crates/contract/src/catalog_ext.rs`):**
   Use the canonical `LibrarySnapshot` struct (shared identically by H0.1 today and upgraded cleanly by ST-3's `ytmapi-rs` adapter in Wave 1 with ZERO UI changes!):
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
   pub struct LibrarySnapshot {
       pub liked_songs: Vec<Track>,
       pub playlists: Vec<LibraryCollectionRef>,
       pub albums: Vec<LibraryCollectionRef>,
       pub artists: Vec<LibraryCollectionRef>,
       pub history: Vec<Track>,
       pub podcasts: Vec<LibraryCollectionRef>,
       pub uploads: Vec<Track>,
       pub fetched_at_unix_secs: u64,
   }
   ```
2. **SQLite Offline Snapshot Key:** `library:snapshot:v1` stored via `CacheStore::put_metadata("library:snapshot:v1", &json, /* ttl_secs */ 300, now)`.
3. **Cold-Start Boot Sequence (`src/live_engine.rs`):**
   - **Step 1 (Synchronous Local Hydration, `< 10 ms`):** Before touching the network, read `cache.get_metadata("library:snapshot:v1", now)` AND `cache.load_history(200)`. If a cached `LibrarySnapshot` exists (even if stale/offline!), immediately emit `LiveUpdate::LibraryLoaded(snapshot)` alongside `LiveUpdate::Ready`. Cold-start offline launches immediately display the user's real Liked Songs, Playlists, Albums, Artists, and History!
   - **Step 2 (Async Network Refresh when Signed In):** If `provider.current_session().is_some()` and `!offline_mode`, spawn a background Tokio task `refresh_library_snapshot()`:
     - Calls `provider.liked_songs().await` + `provider.browse_collection("FEmusic_liked_playlists").await` (fault-isolated via `tokio::join!` so a partial endpoint error never blocks Liked Songs; ST-3 later swaps this cleanly to `CatalogExtProvider::fetch_library_snapshot()`).
     - Merges local SQLite listening history (`cache.load_history(200)`) with remote history (deduplicated by `(track.id, played_at_unix_secs / 60)`).
     - Persists updated `LibrarySnapshot` to SQLite `metadata_cache` under `"library:snapshot:v1"`.
     - Emits `LiveUpdate::LibraryLoaded(snapshot)` to the NSH UI thread.
   - **Step 3 (Sign-In & Sign-Out Triggers):**
     - Immediately trigger `refresh_library_snapshot()` after `sign_in_with_cookies()` (and later H3's WebView/TV OAuth sign-in).
     - On `sign_out()`, call `cache.evict_metadata("library:snapshot:v1")` and emit `LiveUpdate::LibraryLoaded(LibrarySnapshot::default())` so account data never leaks across users.
4. **Store Integration (`src/store/library_slice.rs` & `src/store/mod.rs`):**
   - Add `pub playlists: Vec<LibraryCollectionRef>`, `pub albums: Vec<LibraryCollectionRef>`, `pub artists: Vec<LibraryCollectionRef>`, `pub library_loading: bool`, `pub library_hydrated_at: Option<u64>` to `LibrarySlice`.
   - `LibrarySlice::apply_snapshot(&mut self, snap: LibrarySnapshot)` atomically updates `liked_tracks`, `playlists`, `albums`, `artists`, and synchronizes `is_liked` on `playback.player.current_track`.

---

### 2.3 H0.2 — `SettingsV1` Persistence via `CacheStore::put_metadata`

#### Critical Bug Prevention Discovered in `crates/core/src/cache/db.rs:814`
`SqliteCacheStore::clear_all_caches()` currently runs:
```sql
DELETE FROM metadata_cache; DELETE FROM audio_chunks;
```
**Mandatory Fix in H0.2:** Change line 814 of `crates/core/src/cache/db.rs` to:
```sql
DELETE FROM metadata_cache WHERE cache_key NOT LIKE 'settings:%';
DELETE FROM audio_chunks;
```
This guarantees that persisting `SettingsV1` via `CacheStore::put_metadata("settings:v1", ...)` survives `"Clear Transient Caches"` in Settings without ever wiping user preferences!

#### Schema (`crates/contract/src/settings.rs`)
```rust
use serde::{Deserialize, Serialize};

pub const SETTINGS_METADATA_KEY: &str = "settings:v1";
/// Effectively non-expiring TTL (100 years) so settings never expire from SWR metadata_cache.
pub const SETTINGS_TTL_SECS: u64 = 100 * 365 * 24 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedTheme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedEqPreset {
    #[default]
    Flat,
    BassBoost,
    VocalBoost,
    TrebleBoost,
    LateNight,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsV1 {
    pub schema_version: u32,
    pub theme: PersistedTheme,
    pub reduced_motion: bool,
    pub sponsorblock_enabled: bool,
    pub sponsorblock_categories: Vec<String>,
    pub loudness_normalization: bool,
    pub silence_skip_enabled: bool,
    pub eq_preset: PersistedEqPreset,
    pub eq_gains_db: [f32; 10],
    pub crossfade_ms: u32,
    pub offline_mode: bool,
    pub volume: f32,                 // clamped 0.0..=1.0
    pub offline_quota_bytes: u64,    // default 5 GiB; 0 = Unlimited (consumed by H2.2)
    pub update_channel: String,      // "stable" | "beta" (consumed by LT-2)
    pub local_music_dirs: Vec<String>, // persisted folder roots (consumed by H3.2)
}

impl Default for SettingsV1 {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: PersistedTheme::System,
            reduced_motion: false,
            sponsorblock_enabled: true,
            sponsorblock_categories: vec![
                "music_offtopic".into(),
                "sponsor".into(),
                "intro".into(),
                "outro".into(),
            ],
            loudness_normalization: true,
            silence_skip_enabled: false,
            eq_preset: PersistedEqPreset::Flat,
            eq_gains_db: [0.0; 10],
            crossfade_ms: 0,
            offline_mode: false,
            volume: 0.85,
            offline_quota_bytes: 5 * 1024 * 1024 * 1024,
            update_channel: "stable".into(),
            local_music_dirs: Vec::new(),
        }
    }
}
```

#### Hydration & Debounced Persistence Flow
1. **Startup Hydration (`src/live_engine.rs`):**
   - Load `cache.get_metadata(SETTINGS_METADATA_KEY, now)`. Deserialize with `#[serde(default)]` so forward/backward field additions never fail; if corrupt JSON, log warning and fall back to `SettingsV1::default()`.
   - Apply `volume`, `offline_mode`, `sponsorblock_enabled` directly to `PlaybackController` before emitting `LiveUpdate::SettingsLoaded(settings)`.
   - `SettingsSlice::apply_persisted_settings(settings)` hydrates `theme_choice`, `reduced_motion`, `dsp_config`, `offline_quota_bytes`, etc., and applies the NSH theme mode immediately.
2. **Non-Blocking Persistence on Mutation:**
   - Every settings toggle/slider change in `src/store/mod.rs` (`set_theme`, `set_reduced_motion`, `toggle_sponsorblock`, `set_loudness_normalization`, `set_silence_skip`, `cycle_eq_preset`, `cycle_crossfade_ms`, `toggle_offline_mode`, `set_volume`) constructs `SettingsV1` and calls `live_engine::persist_settings(settings)`.
   - `live_engine` debounces rapid volume slider drags (250 ms coalesce window on the Tokio background worker, flush immediately on discrete toggles) and calls `cache.put_metadata(SETTINGS_METADATA_KEY, &json, SETTINGS_TTL_SECS, unix_secs())`. Zero SQLite writes on the NSH UI thread!

---

### 2.4 H0.3 — Download Action Surface (Track Row Menus + Now Playing)

#### Problem
`store::enqueue_download`, `pause_download`, `resume_download`, `cancel_download`, and `delete_download` already exist and talk to `DownloadManager`, but track rows (`src/ui/controls/media_rows.rs`) and `src/screens/now_playing.rs` do not expose download actions or per-track offline state badges.

#### UI & State Wiring
1. **Unified Per-Track Download Badge Helper (`src/ui/controls/media_rows.rs`):**
   - Read reactive `store::download_record(&track_id)` (`Option<DownloadRecord>`):
     - `None` → idle action `"Download"` (`IconKind::Download`)
     - `Some(DownloadStatus::Queued)` → pill/badge `"Queued"` (tap → pause/cancel menu)
     - `Some(DownloadStatus::Downloading | Tagging)` → progress indicator `"XX%"` (`bytes_downloaded * 100 / total_bytes.max(1)`)
     - `Some(DownloadStatus::Paused)` → badge `"Paused"` (tap → `"Resume Download"`)
     - `Some(DownloadStatus::Completed)` → semantic primary check badge `"Offline"` (`IconKind::CheckCircle`) — tap menu offers `"Remove Download"`
     - `Some(DownloadStatus::Failed | Cancelled)` → destructive retry badge `"Retry"` (tap → `store::enqueue_download(track)`)
2. **Track Action Sheet (`src/ui/controls/sheets.rs`):**
   - Upgrade the trailing `MoreVertical` (`⋯`) button on `compact_mix_row` and `track_row_view` (`>= 48dp` touch target on mobile) to open a reusable `TrackActionSheet` (`store::open_track_sheet(track, context)`):
     - `"Play Next"`
     - `"Add to Queue"`
     - `"Add to Playlist..."` (wires into H0.4!)
     - `"Download"` / `"Pause Download"` / `"Resume Download"` / `"Remove Offline Copy"` (state-aware via `store::download_record(&track.id)`)
     - `"Go to Album"` / `"Go to Artist"` (wired to entity navigation; enhanced by H2.1!)
     - *(When inside a user-owned playlist)* `"Remove from Playlist"` (wires into H0.4!)
3. **Now Playing Action Bar (`src/screens/now_playing.rs`):**
   - Add a dedicated reactive Download button (`>= 48dp` mobile touch target, `test_id("now-playing-download-btn")`) beside Like & Queue controls:
     - Shows `IconKind::Download` (idle), live `%` label (downloading), `IconKind::CheckCircle` (offline), or retry state.
     - Single press enqueues download when idle, resumes when paused/failed, or opens confirmation to remove when already offline.
   - Zero raw color literals; strictly semantic `theme()` tokens!

---

### 2.5 H0.4 — Playlist Writes (`Add to Playlist`, Remove & Reorder with Optimistic Rollback)

#### Problem
`live_engine::dispatch_edit_playlist(playlist_id, op)` and `MutationWorker` already support `PlaylistEditOp::{AddVideo, RemoveVideo, MoveVideoBefore}`, and `MutationSlice` handles optimistic rollback for likes—but playlist edits have zero UI surface and `MutationSlice::rollback_mutation` currently ignores `MutationCommand::EditPlaylist`!

#### Architecture & Optimistic Rollback Design
1. **Optimistic Snapshot Stack (`src/store/mutation_slice.rs`):**
   - Extend `MutationSlice` with a bounded pending-edit rollback map keyed by `(playlist_id, op_signature)`:
     ```rust
     pub struct PendingPlaylistEdit {
         pub playlist_id: String,
         pub previous_tracks: Vec<Track>,
     }
     ```
   - Before dispatching `MutationCommand::EditPlaylist { playlist_id, op }`:
     - Save a snapshot of the target playlist's current `tracks` in `pending_playlist_edits`.
     - Apply the mutation **optimistically** to `CatalogSlice::selected_collection` (if open) and `LibrarySlice::playlists` track counts:
       - `AddVideo { video_id }` → append track to open playlist & increment count + show toast `"Added to <Playlist>"`.
       - `RemoveVideo { video_id, set_video_id }` → remove track immediately.
       - `MoveVideoBefore { set_video_id, predecessor_set_video_id }` → reorder track in-place immediately.
     - Dispatch `live_engine::dispatch_edit_playlist(playlist_id, op)` (and update SQLite SWR playlist cache on success).
   - **Rollback on `LiveUpdate::MutationFailed { command: MutationCommand::EditPlaylist { playlist_id, .. }, reason }`:**
     - Pop the saved `previous_tracks` snapshot, restore `CatalogSlice::selected_collection` & `LibrarySlice::playlists` count deterministically, decrement `pending_mutations`, and display a redacted error toast (`"Playlist update failed: <reason>"`).
2. **UI Surfaces (`src/ui/controls/sheets.rs` & `src/screens/library.rs`):**
   - **`AddToPlaylistSheet` (`test_id("add-to-playlist-sheet")`):**
     - Opened from `TrackActionSheet` (`"Add to Playlist..."`) or Now Playing menu.
     - Lists all signed-in user playlists from `LibrarySlice::playlists` (with artwork + track count) + a `"New Playlist"` quick-create row (wired to `CreatePlaylist` when signed in).
     - Clicking a playlist immediately runs the optimistic `AddVideo` mutation and closes the sheet.
     - If signed out, displays an honest inline prompt (`"Sign in to save tracks to YouTube Music playlists"`) linking to Account Settings.
   - **Playlist Detail Track Management (`src/screens/library.rs` / collection view):**
     - For user-owned playlists, expose per-row `"Move Up"`, `"Move Down"` (computing `set_video_id` & `predecessor_set_video_id` for `MoveVideoBefore`), and `"Remove from Playlist"` (`RemoveVideo`) inside the row's action menu (`>= 48dp` targets, keyboard-accessible).

---

### 2.6 H0.5 — `OfflineVault` Decision ("Pinned Collections") & Honest `"Radio Mix"` Rename

#### Decision 1: Surface `OfflineVault` (Do NOT Delete!)
`crates/core/src/downloads/vault.rs` is already production-grade: it persists `pinned_collections` & `pinned_collection_tracks` in SQLite v3, ensures `clear_all_caches()` never deletes pinned collections, tracks per-track download states, and verifies files on disk. Leaving `#[allow(dead_code)] vault: Arc<OfflineVault>` in `src/live_engine.rs:83-84` wastes working infrastructure!

- **H0.5 Wiring:**
  1. Remove `#[allow(dead_code)]` on `vault: Arc<OfflineVault>` in `src/live_engine.rs:83-84`.
  2. Add `LiveUpdate::PinnedCollectionsLoaded(Vec<PinnedCollectionSummary>)` (hydrating `vault.list_pinned_collections()` on boot and after every pin/unpin).
  3. Expose `live_engine::pin_collection(collection)`, `live_engine::unpin_collection(collection_id)`, and `store::pinned_collections()` in `LibrarySlice`.
  4. In `src/screens/library.rs` under the **Downloads** filter (`LibraryFilter::Downloads`), add a clean **"Pinned Collections"** section above individual downloaded tracks showing each pinned album/playlist with `(completed / total)` track count, `"Play Offline"`, and `"Unpin"` buttons (`test_id("pinned-collections-section")`).
  5. *(H2.2 later enriches this exact wired foundation with one-tap Pin headers on dedicated Album/Artist screens, collection-level MB progress bars, ref-counted orphan cleanup, and configurable storage quota enforcement.)*

#### Decision 2: Honest Naming — Rename `"Listen Together"` → `"Radio Mix"` Until H2.3 Ships
Currently `src/screens/settings.rs:611` renders `"Listen Together"` over a purely local radio-mix queue generator—misleading users into expecting networked co-listening.
- **Exact Edits:**
  1. `src/screens/settings.rs:611` & doc comments: Change heading `"Listen Together"` → `"Radio Mix"` and subtitle copy to accurately describe instant mood/seed radio mixes.
  2. `src/screens/shell.rs:407-409`: Update sidebar/navigation label `"Listen Together"` → `"Radio Mix"`.
  3. `src/store/ui_slice.rs:13`: Keep `AppTab::RadioMix` (or alias cleanly) and keep `test_id("nav-radio-mix")` (updating any matching references in `tests/headless_ui.rs` and `tests/phase5_polish_and_ergonomics.rs` in the same commit so every headless UI test stays green!).
  4. In **H2.3**, this exact tab evolves cleanly into `"Radio & Rooms"` once real host-authoritative networked Listen Together exists!

---

## 3. Offline Verification Test Suite (`tests/h0_account_truth_test.rs`)

1. `app_paths_uses_os_dirs_env_override_0700_and_migrates_legacy_temp_once` — verifies `RESONANCE_DATA_DIR`, non-temp default, Unix `0o700`, `.nomedia`, and idempotent one-time migration without clobbering existing DB.
2. `library_hydration_populates_from_sqlite_offline_and_refreshes_when_signed_in` — verifies cold-start offline read of `library:snapshot:v1`, live `liked_songs()` hydration emitting `LiveUpdate::LibraryLoaded`, and complete cache eviction on `sign_out()`.
3. `settings_v1_persists_via_put_metadata_and_survives_clear_all_caches` — persists custom theme, SponsorBlock, EQ preset, crossfade, offline mode, volume, and quota via `CacheStore::put_metadata("settings:v1", ...)`; calls `clear_all_caches()`; asserts all transient cache rows are cleared while `SettingsV1` hydrates identically across restart!
4. `download_action_surface_reflects_queued_downloading_offline_and_failed_states` — `nsh_testing::Headless` test verifying track row menu & Now Playing download button dispatch `enqueue_download` and render reactive status states.
5. `playlist_edit_add_remove_reorder_optimistically_updates_and_rolls_back_on_failure` — verifies optimistic UI update for `AddVideo`, `RemoveVideo`, and `MoveVideoBefore`, followed by exact state restoration on `LiveUpdate::MutationFailed`.
6. `offline_vault_pinned_collections_surface_and_honest_radio_mix_label` — verifies `OfflineVault::pin_collection` surfaces in Library Downloads, survives `clear_all_caches()`, and verifies UI renders `"Radio Mix"` (not `"Listen Together"`).

```bash
cargo fmt --all -- --check
cargo test --test h0_account_truth_test -- --nocapture
cargo test --test architecture_gates_v2 --test headless_ui --test phase5_polish_and_ergonomics
```
