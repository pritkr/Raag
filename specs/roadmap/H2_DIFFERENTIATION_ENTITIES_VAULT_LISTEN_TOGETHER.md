# H2 — Differentiation: Dedicated Album & Artist Entity Screens, Offline Vault UX & Quota, and Real Listen Together

> **Wave:** Wave 2 (Parallel Sprint — Parallelizable with H3, LT-1, LT-2, LT-3)
> **Dependencies:** Builds directly on **H0** (`SettingsV1`, `OfflineVault` Pinned Collections, honest `"Radio Mix"` tab) and **ST-3** (`CatalogExtProvider::{fetch_artist, fetch_album}` via `ytmapi-rs`/`rustypipe`).
> **Parallelism Boundary:**
> - **Exclusive Write Scope:**
>   - `src/screens/artist.rs` (new), `src/screens/album.rs` (new), `src/screens/listen_together.rs` (new)
>   - `crates/core/src/downloads/quota.rs` (new — quota evaluator + ref-counted orphan cleanup)
>   - `crates/core/src/rooms/` (new — `mod.rs`, `protocol.rs`, `clock_sync.rs`, `ws_client.rs`, `long_poll.rs`, `controller.rs`)
>   - `crates/room-relay/` (new optional self-hostable lightweight Rust `axum` room relay binary crate)
>   - `src/store/rooms_slice.rs` (new)
>   - `tests/h2_entities_vault_listen_together_test.rs` (new)
> - **Forbidden Edits in Wave 2:** `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/shell.rs`, `src/screens/settings.rs`, `Cargo.toml`, `platform/**`, `crates/core/src/providers/**`. (Export clean `wire_h2_*` helpers connected in Wave 3 Stitch!)

---

## 1. H2.1 — Dedicated Album & Artist Entity Screens

### Problem
Currently, tapping an album or artist falls back to a generic flat `selected_collection` track list with no discography shelves, no related artists, no album metadata (year, total runtime, release type), and no one-tap `"Start Radio"` seed action.

### Data Contracts (Frozen in Wave 0 `crates/contract/src/catalog_ext.rs`)
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumDetail {
    pub browse_id: String,
    pub audio_playlist_id: Option<String>,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub release_year: Option<String>,
    pub release_type: String, // "Album" | "EP" | "Single"
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
```
*(Backed by ST-3's `HybridMusicProvider` via `ytmapi-rs` `GetArtistQuery`/`GetAlbumQuery` with `rustypipe` `music_artist`/`music_album` failover + 60-minute SQLite SWR caching!)*

### Screen Architectures (100% Semantic `theme.json` Tokens, Zero Blur in Virtual Lists)

#### A. Dedicated Artist Screen (`src/screens/artist.rs`)
- **Navigation Route:** `EntityRoute::Artist { channel_id: String }` pushed onto a proper typed navigation stack in `UiSlice` so Back/Forward history (`Alt+Left` / `Alt+Right` / mobile Back) seamlessly traverses `Home → Artist → Album → Related Artist`!
- **Layout:**
  1. **Atmospheric Hero Header:** Ambient palette extracted via `crates/core/src/palette.rs` from `artist.artwork_url`, circular/squircle focal portrait, display name, subscriber count, collapsible `"About"` bio toggle.
  2. **Primary Action Row (`>= 48dp` mobile targets):**
     - `"Play Top Songs"` (replaces queue with `top_songs`)
     - `"Start Radio"` (`test_id("artist-start-radio-btn")` — seeds autoplay/watch-next via `artist.radio_playlist_id` or `top_songs[0]`)
     - `"Shuffle"`
     - `"Subscribe"` / `"Subscribed"` (dispatches authenticated subscription mutation when signed in)
  3. **Top Songs Section:** Numbered `track_row_view` list (top 5 collapsed with `"Show all"` expansion; includes H0.3 download badges and H0.4 playlist menu).
  4. **Discography Shelves:** Horizontal scrollable `media_card` shelves for **Albums** and **Singles & EPs** — clicking any card pushes `EntityRoute::Album { browse_id }`.
  5. **Related Artists Shelf:** Circular artist cards — clicking pushes `EntityRoute::Artist { channel_id }`.

#### B. Dedicated Album Screen (`src/screens/album.rs`)
- **Navigation Route:** `EntityRoute::Album { browse_id: String }`.
- **Layout:**
  1. **Split Hero (Desktop) / Stacked Hero (Mobile):** 220px artwork (`artwork_box`) with ambient palette tint, release type badge (`Album` / `EP` / `Single`), clickable Artist attribution link (pushes `EntityRoute::Artist`), release year + track count + total formatted runtime (`"12 songs • 44 min"`).
  2. **Primary Action Bar (`>= 48dp` targets):**
     - `"Play"` / `"Shuffle"`
     - `"Start Radio"` (seeds radio from album's lead track / `audio_playlist_id`)
     - **One-Tap `"Pin Offline"` Toggle** (wired directly to H2.2 `OfflineVault`!)
     - `"Add Album to Playlist"` / `"Share"`
  3. **Numbered Tracklist:** Track index (`1..N`, `tabular_nums`), title, explicit badge, guest artist subtitle (only when differing from primary album artist), duration, H0.3 per-track offline state badge, and `TrackActionSheet`.
  4. **Other Versions Shelf:** Deluxe / Explicit / Instrumental editions if returned by provider.

---

## 2. H2.2 — Offline Vault UX: One-Tap Pinning, Collection Progress & Storage Quota Management

> **Deduplication Note:** Builds cleanly on **H0.5**'s wired `OfflineVault` (`crates/core/src/downloads/vault.rs`) and **H0.2**'s `SettingsV1::offline_quota_bytes`. Zero duplicate database tables!

### 2.1 One-Tap Collection Pinning & Aggregate Progress
- **Surface Locations:**
  - Album screen header (`src/screens/album.rs`)
  - Playlist detail header (`src/screens/library.rs`)
  - Library → Downloads → `"Pinned Collections"` management panel (`src/screens/library.rs`)
- **Collection State Machine (`CollectionOfflineProgress`):**
  Computed deterministically in `crates/core/src/downloads/quota.rs` from `PinnedCollection` + `HashMap<TrackId, DownloadRecord>`:
  - `Unpinned` → Button: `"Pin Offline"` (`IconKind::Download`)
  - `Syncing { completed, total, bytes_downloaded, bytes_total }` → Progress bar + `"Downloading 4 / 12 (38 MB)"` + `"Pause All"` button
  - `Paused { completed, total }` → `"Paused (4 / 12)"` + `"Resume All"` button
  - `Complete { total, total_bytes }` → Primary badge `"Offline • 12 tracks (94 MB)"` + `"Unpin"` action
  - `PartialFailed { completed, failed, total }` → `"10 / 12 • 2 failed"` + `"Retry Failed"` button

### 2.2 Ref-Counted Orphan-Safe Unpin Cleanup (`crates/core/src/downloads/quota.rs`)
**Critical Invariant:** A track may belong to BOTH a pinned Album AND a pinned Playlist (or be manually downloaded individually).
- When the user clicks `"Unpin Collection"`:
  1. Remove the collection from `pinned_collections` & `pinned_collection_tracks` (`vault.unpin_collection(collection_id)`).
  2. For each `video_id` that was in the unpinned collection, check `cache.is_track_pinned(&video_id)`:
     - If `true` (still referenced by another pinned collection) → **KEEP the audio file on disk!**
     - If `false` → delete the track file + `downloads` row (`vault.delete_track(&video_id)`), reclaiming storage immediately.
  3. Also support `"Pin Collection"` incremental sync: if a signed-in user adds new songs to a pinned playlist, `OfflineVault::sync_pinned_collection()` enqueues ONLY the delta tracks not yet completed on disk!

### 2.3 Pre-Flight Storage Quota Management
- **Quota Presets in Settings & Library Downloads Storage Bar (`SettingsV1::offline_quota_bytes`):**
  - Presets: `1 GB`, `2 GB`, `5 GB` (default), `10 GB`, `25 GB`, `Unlimited (0)`.
  - Visual segmented storage bar showing: `Pinned Offline Vault (MB)` + `Transient Audio/Artwork Cache (MB)` vs `Configured Quota`.
- **Pre-Flight Check Before Enqueueing (`QuotaEvaluator::check_before_enqueue`):**
  - Estimate incoming bytes (`track_count * estimated_avg_bytes`, default `4.5 MB/track` for 160 kbps Opus / `7.5 MB/track` for 256 kbps Premium, refined once upstream `Content-Length` arrives).
  - If `quota > 0` and `current_vault_bytes + incoming_bytes > quota`:
    - Return structured `CoreError::OfflineStorage(OfflineStorageError::QuotaExceeded { used_bytes, quota_bytes, needed_bytes })`.
    - UI displays a clear non-destructive prompt (`"Offline Vault quota reached (4.9 / 5.0 GB) — Increase limit or unpin a collection"` with one-tap `"Manage Storage"` button) instead of failing mid-download or silently deleting pinned albums!
  - **Immutable Separation Invariant:** Transient `ChunkCache` LRU eviction NEVER touches `OfflineVault` files, and `OfflineVault` NEVER silently evicts user-pinned albums without explicit user confirmation.

---

## 3. H2.3 — Real Listen Together: Host-Authoritative Synced Room Service

> **Evolution from H0.5:** H0.5 honestly renamed the local mix generator to `"Radio Mix"`. Once H2.3 ships, that tab upgrades cleanly to **"Radio & Rooms"** with two distinct, honest sub-modes:
> 1. **Radio Mix** (local instant seed/mood radio)
> 2. **Listen Together** (real networked host-authoritative rooms with join codes)

### 3.1 Architecture: Why Host-Authoritative + Dual-Transport (WebSocket + Long-Poll)?
- **Why Host-Authoritative:** Eliminates split-brain queue conflicts and peer-to-peer NAT traversal failures (symmetric mobile CGNAT breaks WebRTC P2P without expensive TURN relays). One designated **Host** owns the canonical playback state (`track_id`, `queue`, `is_playing`, `authoritative_position_ms`, `server_epoch_ms`, `revision`); a thin, stateless/in-memory **Room Relay** stamps monotonic server timestamps (`server_now_ms`) and fans out snapshots to all connected guests.
- **Why WebSocket Primary + Automatic HTTP Long-Poll Fallback:**
  - **Primary (`wss://<relay>/v1/rooms/<code_or_id>/ws`):** Full-duplex low-latency frames using `tokio-tungstenite` (`rustls-tls-webpki-roots` — zero OpenSSL!).
  - **Automatic Fallback (`GET /v1/rooms/<code_or_id>/poll?since_rev=<u64>&timeout_ms=25000` + `POST /v1/rooms/<code_or_id>/cmd`):** If corporate firewalls, captive portals, or mobile proxies drop WebSocket upgrades (`HTTP 400/403/502` or 2 consecutive WS handshake failures), `RoomTransport` seamlessly falls back to HTTP/1.1–2 long-polling over `reqwest` (`rustls`) with zero room disconnect!
- **Privacy & Legal Safety:** The relay NEVER streams copyrighted audio! It exchanges **only lightweight JSON control metadata** (`video_id`, track title/artist metadata, play/pause/seek timestamps, queue order). Every participant resolves & plays audio independently through their own local Resonance `PlaybackController` + `LoopbackProxy`!

```text
┌───────────────────────────┐       WSS (or HTTP Long-Poll Fallback)       ┌───────────────────────────┐
│   HOST (Resonance Client) │ ───────────────────────────────────────────► │   Room Relay Server       │
│  • Owns PlaybackController│                                              │  (`crates/room-relay`,    │
│  • Emits HostStateUpdate  │ ◄─────────────────────────────────────────── │   self-hostable Axum OR   │
│    on play/pause/seek/skip│        ClockPong + ParticipantEvents         │   configurable relay URL) │
│  • Optional Guest-Queue   │                                              │  • Stamps server_now_ms   │
│    approval mode          │                                              │  • 6-char room code       │
└───────────────────────────┘                                              │  • Ephemeral in-RAM state │
                                                                           │  • Zero audio proxying  │
                                                                           └─────────────┬─────────────┘
                                                                                         │
                                                          WSS / Long-Poll Fan-Out        │
                                                                                         ▼
                                                                           ┌───────────────────────────┐
                                                                           │  GUEST (Resonance Client) │
                                                                           │  • NTP ClockSyncFilter    │
                                                                           │  • Hysteresis Drift Guard │
                                                                           │    (<400ms ignore,        │
                                                                           │     >=400ms smooth seek)  │
                                                                           │  • Local LoopbackProxy    │
                                                                           └───────────────────────────┘
```

### 3.2 Exact Wire Protocol (`crates/contract/src/rooms.rs`)
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomRole {
    Host,
    Guest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueuePermission {
    HostOnly,
    GuestsCanAdd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomTrackRef {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub duration_ms: u64,
    pub artwork_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoritativePlaybackState {
    pub revision: u64,
    pub current_track: Option<RoomTrackRef>,
    pub queue: Vec<RoomTrackRef>,
    pub is_playing: bool,
    /// Playback position at `anchor_server_ms` (in milliseconds).
    pub anchor_position_ms: u64,
    /// Monotonic relay server UTC epoch milliseconds when anchor was stamped.
    pub anchor_server_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomWireMessage {
    // Clock synchronization (NTP 4-timestamp style)
    ClockPing { client_send_ms: u64 },
    ClockPong { client_send_ms: u64, server_recv_ms: u64, server_send_ms: u64 },

    // Authoritative room state
    RoomSnapshot {
        room_code: String, // e.g. "RAAG-7K4M" (unambiguous Crockford Base32 alphabet)
        host_peer_id: String,
        queue_permission: QueuePermission,
        participants: Vec<RoomParticipant>,
        playback: AuthoritativePlaybackState,
    },

    // Host commands
    HostSyncState { playback: AuthoritativePlaybackState },
    TransferHost { new_host_peer_id: String },

    // Guest actions
    GuestSuggestTrack { track: RoomTrackRef },
    Leave,
    Error { code: String, message: String },
}
```

### 3.3 Clock Synchronization & Hysteresis Drift Correction (`crates/core/src/rooms/clock_sync.rs`)
To avoid audio stuttering while keeping listeners tightly synchronized across heterogeneous devices and networks:
1. **NTP-Style Median Filter (Rolling Window of Last 8 Samples):**
   - On connect (burst of 3 pings separated by 500 ms) and every 15 seconds thereafter, send `ClockPing { client_send_ms }` using `std::time::Instant` mapped to a monotonic client epoch.
   - Upon receiving `ClockPong`, compute:
     $$\text{RTT} = (T_3 - T_1) - (T_{\text{server\_send}} - T_{\text{server\_recv}})$$
     $$\text{offset} = \frac{(T_{\text{server\_recv}} - T_1) + (T_{\text{server\_send}} - T_3)}{2}$$
   - Reject high-RTT outlier spikes (`RTT > 2 * median_rtt`) and take the median offset over the sliding window.
2. **Expected Position Computation on Guest:**
   - When `is_playing == true`:
     $$\text{expected\_pos\_ms} = \text{anchor\_position\_ms} + \max(0,\; (\text{local\_now\_ms} + \text{clock\_offset\_ms}) - \text{anchor\_server\_ms})$$
   - When `is_playing == false`: `expected_pos_ms = anchor_position_ms`.
3. **Hysteresis Deadband & Anti-Thrash Cooldown:**
   - Compute `drift_ms = (local_pos_ms as i64 - expected_pos_ms as i64).abs()`.
   - **`drift_ms < 400 ms`:** Ignore completely (deadband — imperceptible to human listeners, zero audio glitches!).
   - **`drift_ms >= 400 ms` sustained across 2 consecutive ticks (or immediately on explicit Host seek/track-change revision bump):** Issue a single `PlaybackController::seek(expected_pos_ms)` with a **3-second post-seek cooldown** so decode/buffer latency never triggers a seek oscillation loop!
   - **Pre-Warming Next Track:** Guests automatically feed `playback.queue.first()` into `LoopbackProxy::warm_next_track()` so track transitions stay synchronized within `< 200 ms`!

### 3.4 Optional Self-Hostable Relay (`crates/room-relay/`) & Room Security
- Ship a tiny (~300 LOC) standalone binary crate `crates/room-relay` (`axum` + `tokio`, zero database required, pure in-RAM rooms with automatic 30-minute idle eviction) so anyone can self-host `resonance-room-relay` on a VPS/home server or configure a custom relay URL in Settings (`SettingsV1::room_relay_url`).
- **Security Hardening:**
  - Room codes use 8 characters from unambiguous Crockford Base32 (`XXXX-XXXX`, 40 bits of entropy) + per-room rate limits (`max 32 participants`, `max 60 messages/min/peer`).
  - Host authority is authenticated with a per-session random 256-bit `host_secret` issued on `POST /v1/rooms` (stored only in memory, never logged, redacted by `redact_secrets()`).
  - Strictly TLS (`wss://` / `https://`) except `ws://127.0.0.1` / `http://127.0.0.1` in tests or local LAN self-hosting.
  - Guests cannot hijack playback; if the Host disconnects for `> 45s`, host authority automatically transfers to the oldest connected guest (or closes gracefully).
  - **Local-File Tracks Guard:** If the Host plays a local-only track (`H3.2`, `video_id.starts_with("local:")`), the room UI displays an honest badge (`"Local track cannot be synced to remote peers"`) and skips broadcasting unresolvable local paths!

---

## 4. Offline Verification Test Suite (`tests/h2_entities_vault_listen_together_test.rs`)

1. `artist_and_album_entity_screens_render_discography_related_and_start_radio` — `nsh_testing::Headless` test loading mock `ArtistDetail` & `AlbumDetail`; verifies Top Songs, Albums/Singles shelves, Related Artists navigation push/pop, and `"Start Radio"` dispatch.
2. `offline_vault_unpin_preserves_tracks_shared_by_other_pinned_collections` — pins Album A (`[t1, t2]`) and Playlist B (`[t2, t3]`); unpins Album A; asserts `t1` is deleted from disk while `t2` and `t3` remain intact on disk!
3. `offline_vault_quota_evaluator_blocks_over_limit_without_evicting_pinned_collections` — configures 10 MB quota, pins 8 MB album, attempts 6 MB album -> returns `QuotaExceeded` error while leaving existing 8 MB album untouched.
4. `listen_together_clock_sync_rejects_rtt_outliers_and_enforces_400ms_hysteresis` — feeds synthetic RTT spikes and drifts (`150 ms`, `380 ms`, `650 ms`); asserts zero seeks under `400 ms`, single seek at `650 ms`, and 3s cooldown preventing oscillation.
5. `listen_together_ws_primary_and_http_long_poll_fallback_sync_host_and_guest` — spins up in-process `room-relay` on `127.0.0.1:0`; tests (a) Host creates room → Guest joins via WebSocket → Host play/pause/seek/queue syncs to Guest; (b) simulates WebSocket upgrade `502` → verifies automatic HTTP long-poll fallback completes state sync cleanly; (c) Host disconnect promotes Guest.

```bash
cargo test --test h2_entities_vault_listen_together_test -- --nocapture
cargo test --test offline_store_test --test headless_ui
```
