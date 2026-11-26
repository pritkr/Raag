# W1C — Hybrid `ytmapi-rs` + `rustypipe` Browse, Real Live Discovery, Search Suggestions & Account Library

> **Wave:** Wave 1 (Parallel Agent 1C — Runs concurrently with W1A, W1B, W1D, W1E)
> **Exclusive Write Scope:**
> - `crates/core/src/providers/{mod.rs,hybrid.rs,ytmapi_adapter.rs,rustypipe_adapter.rs,atr_sync.rs}` (new)
> - `crates/core/src/innertube/parser/{explore.rs,suggestions.rs,artist.rs,album.rs}` (new + additive exports in `parser/mod.rs`)
> - `src/store/catalog_slice.rs`
> - `src/screens/{explore.rs,search.rs}`
> - `tests/w1c_hybrid_browse_and_discovery_test.rs` (new)
> **Read-Only Dependencies (From Wave 0):** `crates/contract/src/catalog_ext.rs` (`LibrarySnapshot`, `ExplorePage`, `MoodCategory`, `SearchCategory`, `SearchSuggestion`, `ArtistDetail`, `AlbumDetail`, `CatalogExtProvider`, `BrowseDelegate`).
> **Stitch Hooks (For Wave 1.5 Integration Agent):**
> - Instantiate `HybridMusicProvider::new(legacy_client, cache, paths)` in `src/live_engine.rs`
> - Forward `CatalogCommand::FetchExplore`, `FetchMoodDetail`, `FetchSearchSuggestions`, `SearchCategorized`, `SearchContinuation`, `FetchArtist`, `FetchAlbum`, `RecordSearchHistory` in `src/live_engine.rs` & `src/store/mod.rs`
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{shell,home,library,now_playing,queue,lyrics,settings}.rs`, `platform/**`.

---

## 1. Ground-Truth Problems Fixed

1. **Static Hardcoded Discovery:** `home.rs` mood chips and `explore.rs` charts/genres are hardcoded Rust string arrays that simply redirect to a raw text search (`run_search("Chill")`), rather than querying real YouTube Music InnerTube browse endpoints (`FEmusic_explore`, `FEmusic_moods_and_genres` with `params`).
2. **Flat, Rudimentary Search:** No live search suggestions (`/music/get_search_suggestions`), no persisted local search history, no category tabs (`Top`, `Songs`, `Videos`, `Albums`, `Artists`, `Playlists`, `Podcasts`), and no continuation pagination (`on_load_more`).
3. **Brittle Single-Parser Fragility:** Hand-maintained raw JSON walking breaks whenever Google renames a single renderer wrapper. Combining `ytmapi-rs` (MIT, mirrors `ytmusicapi`) + `rustypipe` (GPL-3.0, resilient NewPipe-style parser) + native InnerTube fallback behind a clean `HybridMusicProvider` gives multi-layer resilience with zero UI coupling.
4. **Incomplete Library & Missing Recommendation Loop:** `Albums`, `Artists`, `Podcasts`, and `Uploads` are synthesized/stubbed rather than backed by real YTM library endpoints (`FEmusic_liked_albums`, `FEmusic_library_corpus_track_artists`, `FEmusic_library_privately_owned_tracks`), and plays never report ATR/watchtime back to YouTube Music (so recommendations go stale).

---

## 2. Target Hybrid Provider Architecture

```text
┌────────────────────────────────────────────────────────────────────────────┐
│  NSH UI (`explore.rs`, `search.rs`, `home.rs`, W2B `artist.rs`/`album.rs`) │
└───────────────────────────────────────┬────────────────────────────────────┘
                                        │ (non-blocking channel)
┌───────────────────────────────────────▼────────────────────────────────────┐
│  HybridMusicProvider (implements `MusicProvider` + `CatalogExtProvider`)   │
│                                                                            │
│  [1] SQLite SWR Cache (`metadata_cache`, ETag / TTL per endpoint)          │
│      • Instant stale return + background revalidate (zero UI spinners)     │
│                                                                            │
│  [2] Primary Browse: `YtmapiBrowseAdapter` (`ytmapi-rs`, `rustls`, MIT)    │
│      • Authenticated cookie / OAuth session sync (`sync_session()`)        │
│      • Explore, Charts (by GL country), Moods+Genres, Search Suggestions,  │
│        Categorized Search + Continuations, Artist, Album, Full Library     │
│                                                                            │
│  [3] Fallback Browse & Stream Backup: `RustyPipeAdapter` (GPL-3.0, rustls) │
│      • Automatic circuit-breaker failover when `ytmapi-rs` errors/drifts   │
│      • Uses `paths.cache_dir/rustypipe/` for visitor/client persistence    │
│                                                                            │
│  [4] Native InnerTube Fallback (`InnerTubeClient` + `parser/*`)            │
│      • Always compiled even without optional features (100% offline tests) │
│                                                                            │
│  [5] Opt-In Watchtime / ATR Sync (`AtrWatchtimeReporter`)                  │
│      • Honors `SettingsV1::atr_watchtime_sync`; fires playback/watchtime   │
│        pings on authenticated tracks (NEVER on local files or anonymous!)  │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Detailed Subsystem Specifications

### 3.1 `HybridMusicProvider` & Circuit-Breaker Failover (`crates/core/src/providers/hybrid.rs`)
- **Feature-Gated Clean Compilation:**
  - `#[cfg(feature = "provider-ytmapi")]` enables `YtmapiBrowseAdapter`.
  - `#[cfg(feature = "provider-rustypipe")]` enables `RustyPipeAdapter`.
  - Native `InnerTubeClient` adapter is **always** available as Tier 3 fallback so unit tests and minimal builds compile rapidly and deterministically.
- **Per-Adapter Circuit Breaker:**
  - Track `consecutive_failures: AtomicU32` and `cooldown_until_unix_secs: AtomicU64`.
  - After 3 consecutive parse/HTTP-5xx failures on Primary, trip circuit for 120 seconds and route immediately to Fallback (avoiding double-timeout latency on every screen click).
  - Auth errors (`401`/`403` invalid cookie) do **not** trip the parser circuit breaker; they surface cleanly as `CoreError::Auth(SessionExpired)`.
- **Session Synchronization:** `sync_session(session: Option<&AuthSession>)` atomically rebuilds authenticated headers for all active adapters while strictly enforcing `ANDROID_VR`/`IOS` anonymous isolation.

### 3.2 Real Live Discovery (`Explore`, `Charts`, `Moods & Genres`, Dynamic Home Chips)
1. **Dynamic Home Mood Chips (`FEmusic_home`):**
   - Parse `chipCloudRenderer.chips[*]` from the `FEmusic_home` response (`{ label, browse_id: "FEmusic_home", params: Option<String> }`).
   - Tapping a mood chip calls `browse_home_with_params(params)` (caching per `home:chip:{hash}`, TTL 300s) instead of jumping to Search!
   - Keep a `"Clear Filter"` active chip state to return to the unfiltered Home feed.
2. **Real Explore Screen (`src/screens/explore.rs`):**
   - Backed by `CatalogExtProvider::fetch_explore(country_code: Option<&str>)` → `FEmusic_explore` + `FEmusic_charts` + `FEmusic_moods_and_genres`.
   - Sections rendered with clean horizontal card carousels & responsive grids:
     - **New Albums & Singles** (artwork card → opens Album screen in W2B / collection view today)
     - **Top Charts** with a real **Country Picker** (`Global`, `United States`, `India`, `United Kingdom`, `Japan`, `South Korea`, `Germany`, `Brazil` — using ISO-3166 `gl` params!) showing Top Songs, Top Music Videos, and Trending Artists.
     - **Moods & Genres** color-accented semantic tile grid (`theme().chart_1..chart_5` / semantic accents — zero raw color literals!) where tapping a tile calls `fetch_mood_playlists(browse_id, params)` and displays real curated mood playlists.
   - Graceful offline/skeleton states: show structured shimmer skeletons during initial load; when offline, show cached SWR Explore snapshot with subtle `"Showing cached Explore"` header hint.

### 3.3 Live Search Autocomplete, Local Search History & Categorized Search (`src/screens/search.rs`)
1. **Debounced Autocomplete (`/music/get_search_suggestions`):**
   - Input typing is debounced at **200 ms** on the UI/store edge (`effect` + `timer(200ms)`, respecting CJK IME `on_composition_change` so pinyin/kana never triggers premature queries!).
   - Returns structured `Vec<SearchSuggestion>`:
     - `QueryCompletion { text, runs }` (tapping fills input & executes search)
     - `DirectEntity { kind: Song | Artist | Album, title, subtitle, browse_id_or_video_id, artwork_url }` (tapping plays song or opens entity directly!).
   - Cancel stale in-flight suggestion tasks using generation tokens so fast typing never displays out-of-order results.
2. **Local Persisted Search History (SQLite):**
   - Store up to 50 recent queries in `metadata_cache` (`search:history:v1`, protected or recreated cleanly) with deduplication (re-searching moves query to top).
   - When the search bar is focused and empty, show recent queries with one-tap `×` remove per entry and `"Clear history"`.
3. **Categorized Search Tabs & Infinite Pagination:**
   - Tabs: `Top`, `Songs`, `Videos`, `Albums`, `Artists`, `Playlists`, `Podcasts` (mapped to exact YTM InnerTube search `params` tokens: e.g., Songs `EgWKAQIIAWoKEAkQBRAKEAMQBA%3D%3D`, Albums `EgWKAQIYAWoKEAkQBRAKEAMQBA%3D%3D`, Artists `EgWKAQIgAWoKEAkQBRAKEAMQBA%3D%3D`, Playlists `EgeKAQQoADgBagwQDhAKEAMQBRAJEAQ%3D`, Podcasts `EgWKAQJQAWoKEAkQBRAKEAMQBA%3D%3D`).
   - Switching category tab reuses cached tab results or triggers categorized search; scrolling to bottom invokes `virtual_list.on_load_more` passing `continuation` token to append seamlessly with zero duplicate IDs.

### 3.4 Full Account Library Hydration & Entity Queries (Feeds W0 + W2B)
Implement `CatalogExtProvider` methods consumed by W0 Library & W2B Entity Screens:
- `fetch_library_snapshot()` — parallel fault-isolated `tokio::join!` over:
  - Liked Songs (`FEmusic_liked_videos` / `LM`)
  - Library Playlists (`FEmusic_liked_playlists`)
  - Saved Albums (`FEmusic_liked_albums`)
  - Subscribed Artists (`FEmusic_library_corpus_track_artists`)
  - Listening History (`FEmusic_history`)
  - Saved Podcasts (`FEmusic_library_podcasts` — empty gracefully if user has none, never fake hardcoded cards!)
  - Uploads (`FEmusic_library_privately_owned_tracks`)
- `fetch_artist(channel_id: &str) -> CoreResult<ArtistDetail>` (header, description, subscriber count, shuffle/radio IDs, top songs, albums, singles, videos, related artists, subscribed state).
- `fetch_album(browse_id: &str) -> CoreResult<AlbumDetail>` (title, artists, year, track count, duration, artwork, explicit flag, numbered tracks, audio playlist ID, other versions).

### 3.5 Opt-In Watchtime / ATR Sync (`crates/core/src/providers/atr_sync.rs`)
- Respects `SettingsV1::atr_watchtime_sync` (disabled automatically when offline, signed out, or playing a local file).
- When a YouTube track crosses **30 seconds of continuous playback** (or 50% duration for short tracks), sends authenticated `playback` + `watchtime` stats ping using the `playbackTracking.videostatsPlaybackUrl` / `videostatsWatchtimeUrl` extracted during `player` resolution (generating a compliant random 16-char `cpn` per play session).
- Fire-and-forget on background Tokio task with 5s timeout; failures are logged at `debug` (redacted) and never disturb playback.

---

## 4. Deterministic Offline Verification (`tests/w1c_hybrid_browse_and_discovery_test.rs`)

1. `hybrid_provider_fails_over_on_primary_error_and_trips_circuit_breaker` — injects 3 failures into mock primary adapter; asserts automatic fallback to secondary, circuit open on 4th call (0 primary attempts), and recovery after cooldown.
2. `explore_and_mood_drilldown_parse_real_innertube_fixtures` — replays recorded `FEmusic_explore` and `FEmusic_moods_and_genres` JSON fixtures; asserts charts, new releases, and mood `params` drill-down parse cleanly.
3. `search_suggestions_debounce_and_history_deduplication` — tests `/music/get_search_suggestions` parsing (both query completions & direct song/artist entities) + search history MRU dedup & single-item deletion.
4. `categorized_search_and_continuation_pagination_deduplicate_ids` — runs `Songs` and `Albums` filtered searches + continuation page append, asserting zero duplicate track IDs.
5. `atr_watchtime_reporter_never_fires_for_local_or_anonymous_tracks` — verifies strict privacy fence when `atr_watchtime_sync = false`, signed out, or `track.id.starts_with("local:")`.
6. `explore_and_search_screens_render_cleanly_in_headless_nsh` — renders `explore_screen` and `search_screen` in `nsh_testing::Headless` verifying category tab switch, suggestion list, and zero raw color literals.

```bash
cargo test --test w1c_hybrid_browse_and_discovery_test -- --nocapture
```
