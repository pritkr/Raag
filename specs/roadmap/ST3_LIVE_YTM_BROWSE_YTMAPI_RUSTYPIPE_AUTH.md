# ST-3 — Hybrid `ytmapi-rs` + `rustypipe` Provider Engine, Live Explore/Charts/Moods, Categorized Search & ATR Watchtime Sync

> **Wave:** Wave 1 (Short-Term P1) — Parallelizable with ST-1, ST-2, ST-4
> **Deduplication Boundary (Zero Overlap):**
> - **ST-3 Owns:** Replacing hand-written InnerTube browse/playback parsers with the **Hybrid `ytmapi-rs` (MIT) + `rustypipe` (GPL-3.0)** provider engine (`crates/core/src/providers/`), implementing `CatalogExtProvider` (Explore, Charts, Moods, Categorized Search, Continuations, full `LibrarySnapshot`, `ArtistDetail`, `AlbumDetail`, `create_playlist`), live `src/screens/explore.rs`, and automatic ATR watchtime sync.
> - **Explicitly Owned Elsewhere (DO NOT duplicate here):**
>   - Boot-time Library hydration, Download action badges/menus, Playlist write UI sheets, `OfflineVault` Pinned Collections UI → **H0** (Wave 0)
>   - Dedicated Artist & Album Entity screens, Offline Vault quota UX → **H2** (Wave 2 — consumes ST-3's `fetch_artist`/`fetch_album`)
>   - Embedded Desktop WebView & TV OAuth Sign-In UI + `tv_oauth.rs` → **H3.1** (Wave 2 — feeds `BrowserToken`/`OAuthToken` into ST-3's `sync_session()`)
> - **Exclusive Write Scope:** `crates/core/src/providers/` (new: `mod.rs`, `ytmapi_adapter.rs`, `rustypipe_adapter.rs`, `hybrid.rs`, `mappers.rs`, `atr_reporter.rs`), `src/store/catalog_slice.rs`, `src/screens/explore.rs`, `src/screens/search.rs` (category chips + suggestions + continuations), `tests/st3_hybrid_providers_and_live_catalog_test.rs` (new).
> - **Forbidden Edits:** `crates/core/src/solver/**`, `crates/core/src/potoken/**`, `crates/core/src/playback/**`, `platform/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/settings.rs`, `src/screens/library.rs`.

---

## 1. Deep Research: Replacing Hand-Written InnerTube Parsers with `ytmapi-rs` + `rustypipe`

### Why Replace Hand-Written Parsers?
YouTube Music's InnerTube UI JSON (`musicShelfRenderer`, `musicCarouselShelfRenderer`, `musicResponsiveListItemRenderer`, `musicTwoRowItemRenderer`) mutates constantly across regions, A/B experiments, and signed-in vs anonymous sessions. Maintaining custom JSON walkers in `crates/core/src/innertube/parser/` for Home, Explore, Charts, Moods, Search, Library, Artist pages, Album pages, Continuations, and History is an unsustainable maintenance sink.

### Legal & Technical Comparison (`GPL-3.0-or-later` Workspace)

| Dimension | `ytmapi-rs` (`0.3.x`, MIT) | `rustypipe` (`0.11.x`, GPL-3.0) | Resonance Owned `InnerTubeClient` |
| :--- | :--- | :--- | :--- |
| **License Compatibility** | MIT → 100% compatible with `GPL-3.0-or-later` | GPL-3.0 → 100% compatible with `GPL-3.0-or-later` | Owned (`GPL-3.0-or-later`) |
| **Strengths** | Purpose-built for **YouTube Music** (inspired by `ytmusicapi`). Rich typed queries: Home, Search (Songs/Albums/Artists/Playlists/Podcasts/Suggestions), Explore, Charts, Moods & Genres, Artist discography, Album, Playlist, Watch/Up-Next, **Authenticated Library** (Liked Songs, Playlists, Albums, Artists, History, Uploads), **Authenticated Mutations** (Like, Edit Playlist, Create/Delete Playlist). | Battle-tested **YouTube + YTM** extraction (inspired by NewPipe). Excellent anonymous/public Music Home, Search, Artist/Album/Playlist, **multi-client `/player` stream resolution**, built-in JS deobfuscation, and native **`rustypipe-botguard` PO-Token integration**! | Strict `ClientMatrix`, `AuthMode` isolation (`ANDROID_VR`/`IOS` zero-cookie), decoder-aware format matrix (`Android` Opus vs `Desktop` AAC vs `iOS` AAC-only), `rquickjs` `yt-dlp-ejs` hot-updates, 256 KB `LoopbackProxy`. |
| **Weaknesses** | Does **not** resolve raw audio stream URLs (`/player` cipher/PO-token); requires valid `BrowserToken` (cookies) or OAuth for private library calls. | Less focused on private YTM library management/playlist editing than `ytmapi-rs`; heavier dep tree if default features left on. | Hand-written browse parsers only cover basic Home, Search (songs-only), and simple collection playlists. |
| **TLS / Android Safety** | `default-features = false, features = ["rustls"]` — pure Rust TLS! | `default-features = false, features = ["rustls-tls-webpki-roots", "userdata"]` — pure Rust TLS! | Already uses `reqwest` + `rustls-tls`. |

### Optimal Hybrid Architecture Decision: **"Best of Both Worlds Behind Frozen Contracts"**

Rather than ripping out Resonance's battle-hardened `LoopbackProxy` + `ClientMatrix` + `QuickJsSolver`, we compose **three complementary engines** behind Resonance's existing `CompositeMusicProvider` (`BrowseDelegate` + `PlaybackResolver`):

```text
                       ┌───────────────────────────────────────────┐
                       │     HybridMusicProvider (impl MusicProvider│
                       │        + CatalogExtProvider + Mutations)   │
                       └─────────────────────┬─────────────────────┘
                                             │
            ┌────────────────────────────────┴────────────────────────────────┐
            ▼                                                                 ▼
┌──────────────────────────────────────────┐    ┌──────────────────────────────────────────┐
│  BROWSE / METADATA / LIBRARY / MUTATIONS │    │  PLAYBACK STREAM RESOLUTION              │
│  (Circuit-Breaker + SWR SQLite Cache)    │    │  (Strict AuthMode + Decoder Matrix)      │
│                                          │    │                                          │
│  1. Primary: `YtmapiBrowseAdapter`       │    │  1. Primary: `InnerTubeClient`           │
│     • Uses `ytmapi-rs` (`BrowserToken`   │    │     (`ClientMatrix` + `yt-dlp-ejs` +     │
│       or H3.1 `OAuthToken`, unauth guest)│    │      `PoTokenProvider` + Format Matrix)  │
│     • Full YTM: Home, Explore, Charts,   │    │                                          │
│       Moods, Multi-tab Search, Library   │    │  2. Fallback: `RustyPipePlaybackAdapter` │
│       (Playlists/Albums/Artists/History/ │    │     (`rustypipe::client::RustyPipeQuery` │
│       Liked/Uploads), Artist & Album     │    │      anonymous/TV/web player + botguard  │
│       entities (for H2.1), Watch Next    │    │      mapped through Resonance's          │
│                                          │    │      `select_best_format_for_platform()`)│
│  2. Fallback: `RustyPipeBrowseAdapter`   │    │                                          │
│     • Anonymous resilience when YTM web  │    └──────────────────────────────────────────┘
│       schema shifts faster than ytmapi-rs│
│     • Maps `rustypipe::model::Music*`    │
│                                          │
│  3. Last Resort: Legacy `InnerTubeClient`│
│     parsers (zero-dependency offline/    │
│     feature-disabled build fallback)     │
└──────────────────────────────────────────┘
```

**Why this hybrid architecture wins:**
- If YouTube breaks `ytmapi-rs`'s Home/Search parser overnight, `RustyPipeBrowseAdapter` transparently catches the parse error and keeps browsing alive while `ytmapi-rs` updates.
- If YouTube breaks `InnerTubeClient`'s `/player` chain, `RustyPipePlaybackAdapter` provides a completely independent fallback resolver—while still passing all candidate streams through Resonance's **decoder-driven format matrix** (`formats.rs`) and **256 KB `LoopbackProxy`**!
- Both adapters live behind optional Cargo features (`provider-ytmapi`, `provider-rustypipe`, enabled by default in release builds) so minimal/hermetic builds still compile cleanly.

---

## 2. Exact `crates/contract` Extensions (Frozen in Wave 0)

### `crates/contract/src/catalog_ext.rs`
```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::{ArtistRef, CoreResult, HomeSection, Playlist, SearchResults, Track};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchCategory {
    All,
    Songs,
    Albums,
    Artists,
    Playlists,
    Podcasts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PagedResult<T> {
    pub items: Vec<T>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoodCategory {
    pub title: String,
    pub params: String,
    pub accent_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorePage {
    pub new_releases: Vec<Playlist>,
    pub charts: Vec<HomeSection>,
    pub moods_and_genres: Vec<MoodCategory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryCollectionRef {
    pub browse_id: String,
    pub playlist_id: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub artwork_url: Option<String>,
    pub track_count: Option<u32>,
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumDetail {
    pub browse_id: String,
    pub audio_playlist_id: Option<String>,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub release_year: Option<String>,
    pub release_type: String,
    pub description: Option<String>,
    pub artwork_url: Option<String>,
    pub total_duration_ms: u64,
    pub tracks: Vec<Track>,
    pub other_versions: Vec<LibraryCollectionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistDetail {
    pub channel_id: String,
    pub name: String,
    pub description: Option<String>,
    pub subscribers: Option<String>,
    pub is_subscribed: bool,
    pub artwork_url: Option<String>,
    pub radio_playlist_id: Option<String>,
    pub shuffle_playlist_id: Option<String>,
    pub top_songs: Vec<Track>,
    pub albums: Vec<LibraryCollectionRef>,
    pub singles: Vec<LibraryCollectionRef>,
    pub related_artists: Vec<LibraryCollectionRef>,
}

#[async_trait]
pub trait CatalogExtProvider: Send + Sync {
    async fn explore(&self) -> CoreResult<ExplorePage>;
    async fn mood_playlists(&self, params: &str) -> CoreResult<Vec<LibraryCollectionRef>>;
    async fn search_categorized(
        &self,
        query: &str,
        category: SearchCategory,
        continuation: Option<&str>,
    ) -> CoreResult<SearchResults>;
    async fn search_suggestions(&self, prefix: &str) -> CoreResult<Vec<String>>;
    async fn fetch_library_snapshot(&self) -> CoreResult<LibrarySnapshot>;
    async fn fetch_artist(&self, channel_id: &str) -> CoreResult<ArtistDetail>;
    async fn fetch_album(&self, browse_id: &str) -> CoreResult<AlbumDetail>;
    async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> CoreResult<String>;
}
```

---

## 3. File-by-File Implementation Plan

### 3.1 `ytmapi-rs` Adapter (`crates/core/src/providers/ytmapi_adapter.rs`)
- Wraps `tokio::sync::RwLock<YtmapiClientState>`:
  - `Unauthenticated(ytmapi_rs::YtMusic<ytmapi_rs::auth::NoAuthToken>)`
  - `Browser(ytmapi_rs::YtMusic<ytmapi_rs::auth::BrowserToken>)`
  - `OAuth(ytmapi_rs::YtMusic<ytmapi_rs::auth::OAuthToken>)` *(fed seamlessly by H3.1 TV OAuth!)*
- **Auth Sync:** Exposes `sync_session(&self, session: Option<&AuthSession>) -> CoreResult<()>`:
  - Converts Resonance's `AuthSession.cookie_header` + `auth_user` + `page_id` into `ytmapi_rs::auth::BrowserToken` (or OAuth tokens into `OAuthToken`) with zero extra login step.
  - Uses `ytmapi-rs`'s `rustls` client (`reqwest` builder with Resonance's `visitorData` & `hl`/`gl`).
- **Query Mapping (`mappers.rs`):**
  - `home()` → `GetHomeQuery` → `Vec<HomeSection>`
  - `explore()` → `GetMoodCategoriesQuery` + `GetChartsQuery` + `GetNewReleases` → `ExplorePage`
  - `search_categorized()` → `SearchQuery::new(q).with_filter(...)` (`SongsFilter`, `AlbumsFilter`, `ArtistsFilter`, `CommunityPlaylistsFilter`, `PodcastsFilter`) + continuation token support
  - `search_suggestions()` → `GetSearchSuggestionsQuery`
  - `fetch_library_snapshot()` → fault-isolated `tokio::join!` of `GetPlaylistQuery("LM")`, `GetLibraryPlaylistsQuery`, `GetLibraryAlbumsQuery`, `GetLibraryArtistsQuery`, `GetHistoryQuery`, `GetLibraryUploadSongsQuery` (drops straight into H0.1's `LiveUpdate::LibraryLoaded`!)
  - `fetch_artist(channel_id)` → `GetArtistQuery` → `ArtistDetail` (consumed by H2.1!)
  - `fetch_album(browse_id)` → `GetAlbumQuery` → `AlbumDetail` (consumed by H2.1!)
  - `browse_collection(browse_id)` → dispatches `GetAlbumQuery` for `MPREb_*` / `OLAK5uy_*` vs `GetPlaylistQuery` for `VL*` / `PL*` / `RD*`
  - `autoplay_next(video_id)` → `GetWatchPlaylistQuery::from_video_id(video_id)`
  - `create_playlist(...)` → `CreatePlaylistQuery`
- **SWR SQLite Cache Layer:** All read queries read/write Resonance's existing `SqliteCacheStore` with per-endpoint TTLs (Home 15m, Explore 30m, Library 5m, Artist/Album 60m, Search 5m), preserving offline viewing and instant UI paint.

### 3.2 `rustypipe` Adapter (`crates/core/src/providers/rustypipe_adapter.rs`)
- Initializes `rustypipe::client::RustyPipe::builder().storage_dir(paths.cache_dir.join("rustypipe")).build()`:
  - **Strict Auth Isolation:** RustyPipe runs strictly in **anonymous mode** for fallback browse and fallback player resolution unless TV OAuth is explicitly enabled. Browser cookies are NEVER forwarded into RustyPipe's anonymous Android/VR/iOS clients.
- Implements:
  - **`FallbackBrowseDelegate`:**
    - `home()` → `rp.query().music_home().await`
    - `search()` → `rp.query().music_search_tracks(query)` / `music_search_albums(query)`
    - `fetch_album()` / `browse_collection()` → `rp.query().music_album(id)` / `rp.query().music_playlist(id)`
    - `fetch_artist()` → `rp.query().music_artist(id)`
    - `autoplay_next()` → `rp.query().music_radio_track(video_id)`
  - **`RustyPipePlaybackAdapter` (`impl PlaybackResolver`):**
    - Calls `rp.query().player_from_client(video_id, ClientType::Desktop)` / `ClientType::Android` (respecting Resonance's `AccountTier` & `TargetPlatform`).
    - Maps RustyPipe's `AudioStream` list directly through Resonance's `select_best_format_for_platform()` (`crates/core/src/innertube/formats.rs`) so Android still gets Opus-first (`774/251`), Desktop gets AAC-first (`141/140`), and iOS strictly excludes Opus!
    - Connects cleanly to `rustypipe-botguard` when configured (see LT-1).

### 3.3 `HybridMusicProvider` (`crates/core/src/providers/hybrid.rs`)
- Implements `MusicProvider` + `CatalogExtProvider` + `InnerTubeMutations`.
- **Failover Policy:**
  - Browse/Search/Explore/Artist/Album: Try `YtmapiBrowseAdapter` → on parse/schema error, record metric and fall back to `RustyPipeBrowseAdapter` → on failure, fall back to legacy `InnerTubeClient` parsers or stale SWR SQLite cache.
  - Playback: Try Resonance `InnerTubeClient::resolve_stream` (`ClientMatrix` + `yt-dlp-ejs` + `PoTokenProvider`) → if all matrix clients fail or circuit-break, fall back to `RustyPipePlaybackAdapter` before returning `ProviderError::NoStream`.

### 3.4 Live Explore, Categorized Search & Automatic ATR Watchtime Sync
- **`src/screens/explore.rs` & `src/store/catalog_slice.rs`:**
  - Replace hardcoded `CHART_CARDS`, `MOODS_AND_GENRES`, and `curated_discovery_mixes()` in live mode with reactive bindings to `store::explore_page()` (`ExplorePage` fetched from `CatalogExtProvider::explore()`). Keep fixture data strictly when `is_fixture_mode() == true`.
  - Clicking a Mood card fetches `mood_playlists(params)` instead of running a raw text search.
- **`src/screens/search.rs`:**
  - Wire category filter chips (`All`, `Songs`, `Albums`, `Artists`, `Playlists`, `Podcasts`) to `CatalogExtProvider::search_categorized`, live debounced `search_suggestions` (200 ms), and infinite-scroll `continuation` pagination (deduplicating IDs so `virtual_list` keys never collide).
- **Automatic ATR Watchtime Reporter (`crates/core/src/providers/atr_reporter.rs`):**
  - Subscribes to `EventBus` playback ticks; emits `WatchtimeReport` via `MutationWorker` at 30s played, on pause, and on track end/switch when signed in, syncing Listening History back to YouTube Music automatically!

---

## 4. Security, Privacy & Concurrency Edge Cases

1. **Zero Cookie Contamination Across Providers:** `RustyPipeBrowseAdapter` and `RustyPipePlaybackAdapter` must NEVER receive browser cookies from `YtmapiBrowseAdapter`. Only `YtmapiBrowseAdapter` (`WEB_REMIX`) and `InnerTubeClient`'s `CookiesOrAnonymous` clients touch `SAPISID`.
2. **Pagination Deduplication:** Continuation fetches deduplicate track/collection IDs against already-loaded pages to avoid duplicate `virtual_list` keys.
3. **Fault-Isolated Library Sub-Queries:** Failure in `GetLibraryUploadSongsQuery` or Podcasts never breaks Liked Songs, Playlists, Albums, or Artists.
4. **Graceful Feature Degradation:** If built with `--no-default-features` (without `provider-ytmapi` / `provider-rustypipe`), `HybridMusicProvider` compiles cleanly using Resonance's built-in `InnerTubeClient` parsers.

---

## 5. Offline Verification Test Suite (`tests/st3_hybrid_providers_and_live_catalog_test.rs`)

1. `hybrid_provider_falls_back_from_ytmapi_to_rustypipe_on_parse_error` — injects failing primary `BrowseDelegate` + healthy secondary `BrowseDelegate`; verifies seamless failover and SWR SQLite cache population.
2. `hybrid_playback_falls_back_to_rustypipe_and_enforces_platform_format_matrix` — forces `InnerTubeClient` matrix failure; verifies `RustyPipePlaybackAdapter` resolves stream and selects AAC on Desktop / Opus on Android / excludes Opus on iOS.
3. `library_snapshot_fault_isolates_individual_tabs` — fails Uploads endpoint while Liked Songs + Playlists succeed; verifies partial `LibrarySnapshot` returns cleanly and feeds H0.1's `LiveUpdate::LibraryLoaded`.
4. `categorized_search_suggestions_and_continuation_deduplicate_ids` — verifies continuation token pagination appends new results while stripping duplicate IDs.
5. `atr_watchtime_reporter_fires_at_30s_pause_and_track_end_when_signed_in_only` — verifies zero ATR traffic when signed out and exactly-once milestone reporting when signed in.

```bash
cargo test --test st3_hybrid_providers_and_live_catalog_test --features "provider-ytmapi,provider-rustypipe" -- --nocapture
cargo test --test potoken_mutations_test --test offline_store_test
```
