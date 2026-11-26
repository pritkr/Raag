# W0 — Foundation: OS Persistence, `SettingsV1`, Theme Switching, Real Shuffle, Account Truth & Contract Freeze

> **Wave:** Wave 0 (**Mandatory Sequential Foundation** — Execute in full before launching Wave 1 parallel Hydrogen agents)
> **Exclusive Write Scope:**
> - `Cargo.toml`, `crates/contract/Cargo.toml`, `crates/core/Cargo.toml`
> - `crates/contract/src/{lib.rs,storage.rs,settings.rs,catalog_ext.rs,audio_ext.rs,rooms.rs,local_media.rs,updater.rs,casting.rs}`
> - `crates/core/src/storage/{mod.rs,paths.rs}`, `crates/core/src/lib.rs`, `crates/core/src/cache/db.rs`
> - `src/live_engine.rs`, `src/main.rs`, `src/store/{mod.rs,library_slice.rs,settings_slice.rs,playback_slice.rs,ui_slice.rs}`
> - `README.md`, `ARCHITECTURE.md`, `tests/architecture_gates_v2.rs`, `src/App.tsx`, `src/bin/nsh_preview.rs` (NSH `=0.7.0` sync + `cargo fmt --all`)
> - `tests/w0_foundation_test.rs` (new)
> **Forbidden Edits:** None (Wave 0 runs alone to freeze shared contracts and hub primitives).

---

## 1. Ground-Truth Problems Fixed in Wave 0

1. **Volatile Storage (`src/live_engine.rs:100-104`, `src/main.rs`):** Defaults to `std::env::temp_dir().join("resonance-ytmusic-live")`. SQLite cache, downloads, history, queue, and fallback auth cookies vanish on OS reboot or `/tmp` cleanup.
2. **Zero Settings Persistence (`src/store/settings_slice.rs`):** Every user setting resets to default on app restart, and `SqliteCacheStore::clear_all_caches()` (`crates/core/src/cache/db.rs`) unconditionally runs `DELETE FROM metadata_cache`, which would wipe settings stored there unless guarded.
3. **Broken Theme Switch (`src/store/settings_slice.rs`):** `set_theme_choice()` mutates an enum in memory without ever updating NSH's active color appearance (`System` / `Light` / `Dark`).
4. **Fake Shuffle (`src/store/mod.rs:915-928` & `playback_slice.rs`):** `play_all_shuffled()` calls `tracks.reverse()`, and `toggle_shuffle()` only flips a boolean without generating a shuffled playback order or honoring it in `next_track()` / `prev_track()`.
5. **Missing Account Truth (`src/live_engine.rs`):** `InnerTubeClient::liked_songs()` is dead code; boot never hydrates Liked Songs or user playlists from SQLite snapshot or network.
6. **Version Drift:** `ARCHITECTURE.md` and UI strings still reference NSH `0.6.1` while `Cargo.toml` pins `=0.7.0`.
7. **Parallel Contention Risk:** Without freezing additive `crates/contract/` types and workspace `Cargo.toml` dependencies upfront, Waves 1–3 would constantly collide on `Cargo.toml` and `crates/contract/src/lib.rs`.

---

## 2. Deep Architecture & Technical Specifications

### 2.1 Canonical OS Persistent Directories (`AppPaths`)

Create `crates/contract/src/storage.rs` and `crates/core/src/storage/paths.rs`:

```rust
// crates/contract/src/storage.rs
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPaths {
    /// Persistent user state: `resonance.db`, fallback `auth_session.json`, `downloads/`.
    pub data_dir: PathBuf,
    /// Evictable caches: `chunks/`, `artwork/`, `js_solver/`, `ejs/`, `remote_config/`, `rustypipe/`.
    pub cache_dir: PathBuf,
    /// User-editable overrides (`client_matrix.override.json`).
    pub config_dir: PathBuf,
    /// Dedicated offline media root (`data_dir/downloads`, `0o700`, `.nomedia`).
    pub downloads_dir: PathBuf,
}
```

**Deterministic Resolution Precedence (`resolve_app_paths()`):**
1. **`RESONANCE_DATA_DIR` env var** (deterministic tests, CI, portable USB mode): derives `cache_dir = root/cache`, `config_dir = root/config`, `downloads_dir = root/downloads`.
2. **Android native paths** via `static ANDROID_APP_PATHS: OnceLock<AppPaths>` initialized by `set_android_app_paths(files_dir, cache_dir)` (called from W1E JNI `nativeInitPaths` before Tokio starts).
3. **Desktop OS paths** via `directories::ProjectDirs::from("dev", "resonance", "Raag")`:
   - **macOS:** `~/Library/Application Support/dev.resonance.Raag` (data/config), `~/Library/Caches/dev.resonance.Raag` (cache).
   - **Linux:** `$XDG_DATA_HOME/raag`, `$XDG_CACHE_HOME/raag`, `$XDG_CONFIG_HOME/raag`.
   - **Windows:** `%LOCALAPPDATA%\resonance\Raag\data`, `%LOCALAPPDATA%\resonance\Raag\cache`, `%APPDATA%\resonance\Raag\config`.
4. **Headless container fallback:** `$HOME/.raag` (only fall back to `temp_dir()` if `$HOME` is unset, logging a warning).

**Filesystem Hardening & Idempotent Migration:**
- Create all 4 directories with `0o700` on Unix (`std::os::unix::fs::PermissionsExt`).
- Touch empty `.nomedia` files in `downloads_dir`, `cache_dir/chunks`, and `cache_dir/artwork` so Android `MediaStore` never indexes internal fragments.
- **One-Time Migration (`migrate_legacy_temp_dir_if_needed(&paths)`):**
  - Condition: `!paths.data_dir.join("resonance.db").exists()` AND `std::env::temp_dir().join("resonance-ytmusic-live/resonance.db").exists()`.
  - Action: Atomically copy `resonance.db` (plus `-wal`/`-shm` if present), `auth_session.json` (preserving `0o600`), and `downloads/` into `paths.data_dir`; write `.migrated_from_temp_v1` marker. Never overwrite existing persistent user data.

---

### 2.2 `SettingsV1` Persistence & SQLite Protection

**Critical SQLite Guard (`crates/core/src/cache/db.rs`):**
Update `SqliteCacheStore::clear_all_caches()` so settings and pinned collections are **never** wiped when the user clears transient caches:
```sql
DELETE FROM metadata_cache WHERE cache_key NOT LIKE 'settings:%' AND cache_key NOT LIKE 'library:%';
DELETE FROM audio_chunks;
```

**Canonical Schema (`crates/contract/src/settings.rs`):**
```rust
pub const SETTINGS_METADATA_KEY: &str = "settings:v1";
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineQualityPreference {
    DataSaver64,
    #[default]
    Standard160,
    High256,
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
    pub playback_speed: f32,               // 0.5..=2.0, default 1.0
    pub preserve_pitch: bool,              // default true
    pub offline_mode: bool,
    pub wifi_only_downloads: bool,         // default true
    pub offline_quality: OfflineQualityPreference,
    pub keep_liked_songs_offline: bool,    // default false
    pub offline_quota_bytes: u64,          // default 5 GiB; 0 = unlimited
    pub volume: f32,                       // 0.0..=1.6 (1.0 = 100%, up to 1.6 boost)
    pub autoplay_radio: bool,              // default true
    pub atr_watchtime_sync: bool,          // opt-in YTM recommendation sync, default true
    pub update_channel: String,            // "stable" | "beta"
    pub local_music_dirs: Vec<String>,     // canonical folder paths for W2C
}
```

**Hydration & Debounced Persistence Pipeline:**
- **Boot (`src/live_engine.rs`):** Synchronously read `cache.get_metadata("settings:v1", now)` before network init. Deserialize with `#[serde(default)]`; on corrupt JSON, log a redacted warning and recover with `SettingsV1::default()`. Emit `LiveUpdate::SettingsLoaded(settings)` before `LiveUpdate::Ready`.
- **Mutation (`src/store/mod.rs` → `live_engine::persist_settings`):** Send updated `SettingsV1` over the non-blocking `crossbeam_channel` command bus. The Tokio worker coalesces rapid drags (volume, EQ sliders) with a **250 ms debounce timer** and flushes immediately on discrete toggles and graceful app shutdown.

---

### 2.3 Working Light / Dark / System Theme Switching

In `src/store/settings_slice.rs` & `src/ui/tokens.rs`:
- Inspect `nsh_base` / `nsh_core` `set_appearance` / `Appearance` / theme provider API in `nsh =0.7.0`.
- When `SettingsSlice::set_theme_choice(choice)` is called (both from user tap in Settings AND on `SettingsLoaded` hydration):
  1. Persist `SettingsV1::theme`.
  2. Apply the active NSH appearance override (`Appearance::System` / `Appearance::Light` / `Appearance::Dark`, or reactive theme context if `nsh_core` exposes `set_appearance_override`) so every `theme().*` `AdaptiveColor` switches immediately across the entire window without requiring a restart.
  3. Verify in headless test that switching `Dark -> Light -> System` updates resolved surface colors deterministically.

---

### 2.4 Real Non-Repeating Fisher–Yates Shuffle Engine (`ShuffleDeck`)

Replace the broken `tracks.reverse()` in `src/store/mod.rs:915-928` and add a proper `ShuffleDeck` in `crates/contract/src/domain.rs` (or `crates/core/src/playback/queue.rs` + `src/store/playback_slice.rs`):

```rust
/// Deterministic-testable, non-repeating shuffle order preserving original queue order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ShuffleDeck {
    /// Original unshuffled indices into `QueueState::items` in shuffled playback order.
    pub order: Vec<usize>,
    /// Current cursor inside `order`.
    pub cursor: usize,
}
```

**Exact Invariants:**
1. **Enabling Shuffle (`toggle_shuffle(true)`):**
   - Keeps the currently playing track at `order[0]` (`cursor = 0`) so playback never jumps abruptly!
   - Shuffles all remaining indices `[0..len] \ {current_idx}` using `rand::seq::SliceRandom::shuffle(&mut rng)` (Fisher–Yates, $O(n)$, uniform permutation).
2. **Play All Shuffled (`play_all_shuffled(tracks)`):**
   - Loads `tracks` in their natural order into `QueueState::items`, enables `shuffle = true`, picks a uniformly random starting track via Fisher–Yates, and plays `order[0]`.
3. **Disabling Shuffle (`toggle_shuffle(false)`):**
   - Restores `current_index = Some(order[cursor])` so playback continues seamlessly from the exact current track in natural album/playlist order.
4. **Queue Mutations While Shuffled:**
   - `Play Next` inserts the new track's index directly at `order[cursor + 1]`.
   - `Add to Queue` appends the new index to the end of `order`.
   - Removing index `removed`: removes `removed` from `order`, decrements every index `> removed`, and clamps `cursor`.
5. **Repeat All Wrap-Around:** Reaching the end of `order` with `RepeatMode::All` re-shuffles a fresh permutation ensuring the first track of the new cycle != last track of the previous cycle (when `len > 1`).

---

### 2.5 Boot-Time Account Truth & Offline Library Snapshot

1. **Snapshot Key:** `library:snapshot:v1` stored in `metadata_cache` (protected from `clear_all_caches()`, TTL 300s SWR).
2. **Boot Sequence (`src/live_engine.rs`):**
   - **Phase A (`< 10 ms`, offline-safe):** Read `library:snapshot:v1` + `cache.load_history(200)`. Immediately emit `LiveUpdate::LibraryLoaded(snapshot)` alongside `LiveUpdate::Ready`.
   - **Phase B (Async, when signed in & `!offline_mode`):** Spawn background `refresh_library_snapshot()` calling `tokio::join!(provider.liked_songs(), provider.browse_collection("FEmusic_liked_playlists"))` (W1C upgrades this cleanly to `CatalogExtProvider::fetch_library_snapshot()`). Deduplicate history by `(track.id, played_at_unix_secs / 60)`, save to `library:snapshot:v1`, emit `LiveUpdate::LibraryLoaded(snapshot)`.
   - **Sign-Out Privacy Fence:** On `sign_out()`, call `cache.evict_metadata("library:snapshot:v1")` and emit `LiveUpdate::LibraryLoaded(LibrarySnapshot::default())`.

---

### 2.6 Wave 0 Frozen Additive Contract & Workspace `Cargo.toml` Freeze

Add all additive contract modules in `crates/contract/src/` now so Waves 1–3 never edit `crates/contract/src/lib.rs` or `Cargo.toml`:
- `storage.rs`: `AppPaths`
- `settings.rs`: `SettingsV1`, `PersistedTheme`, `PersistedEqPreset`, `OfflineQualityPreference`
- `catalog_ext.rs`: `LibrarySnapshot`, `LibraryCollectionRef`, `ExplorePage`, `ExploreChart`, `MoodCategory`, `SearchCategory`, `SearchSuggestion`, `ArtistDetail`, `AlbumDetail`, `CatalogExtProvider` trait, `BrowseDelegate` trait
- `audio_ext.rs`: `AudioDspCapability`, `AudioControlCommand`, `AudioTelemetryEvent`, `SleepTimerState`, `PlaybackSpeedConfig`, `CodecBadgeInfo`
- `rooms.rs`: `RoomRole`, `RoomPermissions`, `RoomMember`, `RoomStateSnapshot`, `RoomCommand`, `RoomEvent`, `ClockSyncSample`
- `local_media.rs`: `LocalTrackRecord`, `LocalScanProgress`, `LocalLibraryStats`
- `updater.rs`: `UpdateChannel`, `UpdateManifest`, `UpdateStatus`
- `casting.rs`: `CastProtocol`, `CastDevice`, `CastSessionState`, `CastCommand`

Update root `Cargo.toml` with the exact `rustls`-only workspace dependencies from `00_MASTER_PARALLEL_ORCHESTRATOR.md` §4 (keeping `provider-ytmapi` / `provider-rustypipe` / `pcm-engine` cleanly feature-gated or additive so existing tests compile unchanged).

Sync NSH version `=0.7.0` uniformly across `README.md`, `ARCHITECTURE.md`, `AGENTS.md`, `src/App.tsx`, `src/bin/nsh_preview.rs`, and `tests/architecture_gates_v2.rs`, and run `cargo fmt --all`.

---

## 3. Deterministic Offline Verification (`tests/w0_foundation_test.rs`)

1. `app_paths_resolves_env_override_and_migrates_legacy_temp_idempotently` — verifies `0o700`, `.nomedia`, legacy `/tmp` migration once, and never overwriting existing DB.
2. `settings_v1_persists_across_restart_and_survives_clear_all_caches` — saves non-default `SettingsV1`, runs `clear_all_caches()`, reloads DB, asserts every field intact.
3. `settings_v1_corrupt_json_recovers_to_defaults_without_panic` — injects truncated JSON at `settings:v1`, asserts clean fallback.
4. `shuffle_deck_produces_true_permutation_keeps_current_track_and_restores_order` — verifies statistical non-reversal across 20 tracks, `order[0] == current_idx`, `Play Next` insertion at `cursor + 1`, and clean restore on unshuffle.
5. `library_snapshot_hydrates_offline_and_clears_on_sign_out` — verifies offline boot load from `library:snapshot:v1` and complete wipe on sign-out.
6. `nsh_version_and_no_openssl_android_gate` — asserts single-source `0.7.0` sync and `cargo check --workspace --all-targets` green.

```bash
cargo fmt --all -- --check
cargo test --test w0_foundation_test -- --nocapture
cargo test --test architecture_gates_v2
```
