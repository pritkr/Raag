# LT-3 — Android Auto (`MediaLibraryService`) & Cross-Platform LAN Casting (Chromecast CastV2, DLNA/UPnP, AirPlay) with Secure Ephemeral LAN Relay

> **Wave:** Wave 2 (Long-Term P2) — Parallelizable with LT-1 and LT-2 (requires ST-2 & ST-4 completed in Wave 1)
> **Parallelism Boundary:**
> - **Exclusive Write Scope:** `crates/core/src/casting/` (new: `mod.rs`, `discovery.rs`, `chromecast.rs`, `dlna.rs`, `airplay.rs`, `manager.rs`), `crates/core/src/proxy/lan_relay.rs` (new), `src/store/casting_slice.rs` (new), `src/ui/controls/cast_sheet.rs` (new), `platform/android/res/xml/automotive_app_desc.xml` (new), `platform/android/src/main/kotlin/dev/resonance/ytmusic/AndroidAutoBrowseTree.kt` (new), `platform/android/src/main/kotlin/dev/resonance/ytmusic/PlaybackForegroundService.kt` (upgrade `MediaSessionService` → `MediaLibraryService`), `platform/android/AndroidManifest.xml` (Android Auto metadata + dual intent filter), `tests/lt3_android_auto_and_casting_test.rs` (new).
> - **Forbidden Edits:** `crates/core/src/potoken/**`, `src/updater/**`, `nsh.json` (except Wave 3 adding local LAN note if needed), `.github/workflows/**`, `src/live_engine.rs`, `src/store/mod.rs`.

---

## 1. Deep Research & Architectural Foundations

### Part A — Android Auto via AndroidX Media3 `MediaLibraryService`
Android Auto (and Android Automotive OS) does **not** render custom app UI—it binds to the app's `MediaLibraryService`, queries a structured media browse tree (`onGetLibraryRoot`, `onGetChildren`, `onGetItem`, `onSearch`, `onGetSearchResult`), and renders distraction-optimized car screens natively.
- **Crucial Media3 + Android Auto Requirements (Verified):**
  1. Upgrade `PlaybackForegroundService` from `MediaSessionService` to **`MediaLibraryService`** returning a **`MediaLibrarySession`**.
  2. Declare **BOTH** intent-filter actions in `AndroidManifest.xml` for full car head-unit compatibility:
     - `androidx.media3.session.MediaLibraryService` (modern Media3)
     - `android.media.browse.MediaBrowserService` (legacy Android Auto fallback)
  3. Add `<meta-data android:name="com.google.android.gms.car.application" android:resource="@xml/automotive_app_desc" />` pointing to `<automotiveApp><uses name="media" /></automotiveApp>`.
  4. Explicitly set `.setIsBrowsable(true/false)` and `.setIsPlayable(true/false)` on EVERY `MediaMetadata` item—Android Auto silently drops items missing these flags!
  5. **Never assume client-side paging:** Android Auto frequently requests `page = 0, pageSize = Int.MAX_VALUE`; cap returned list sizes reasonably (e.g., 100 items per folder) and push dynamic updates via `mediaLibrarySession.notifyChildrenChanged(parentId)`.
  6. **Offline-First Driving UX:** Cars routinely drive through tunnels and dead zones! Place **`Downloads`** and **`Liked Songs` (cached)** at the top level of the root browse tree, serve artwork via an in-process `ContentProvider` backed by `ArtworkCache` so cover art never stalls on cellular, and resolve every track through the local `127.0.0.1` `LoopbackProxy` (which transparently replays cached chunks and completed offline vault files!).

### Part B — Cross-Platform LAN Casting (Chromecast, DLNA/UPnP, AirPlay)
The single hardest security & protocol problem when casting from an InnerTube music client:
> **Why can't we just send YouTube's `googlevideo.com` URL directly to Chromecast / DLNA speakers?**
> 1. YouTube stream URLs are **IP-bound, User-Agent-checked, and expire / 403 mid-stream** (and may fail CORS or TLS checks on older DLNA speakers).
> 2. **Offline Vault tracks** and **256 KB cached chunks** live ONLY on the user's device—there is no remote URL!
> 3. **Resonance's `LoopbackProxy` is strictly bound to `127.0.0.1`**, which external LAN speakers CANNOT reach!

**The Clean Architectural Solution: `LanCastRelay` (Session-Scoped, IP-Pinned, 256-Bit Bearer-Token LAN Proxy)**
We NEVER relax `LoopbackProxy`'s `127.0.0.1` invariant! Instead, when—and **ONLY when**—the user explicitly connects to a cast device, `CastManager` starts an ephemeral **`LanCastRelay`** server:
1. Determines the exact local LAN interface IPv4 address routing toward the target cast device's IP (via unconnected UDP socket `connect((target_ip, 8009))` → `local_addr()`—zero packets sent, works on macOS/Linux/Windows/Android!).
2. Binds `LanCastRelay` strictly to `<local_lan_ip>:0` (never `0.0.0.0`!).
3. Enforces **4 defense-in-depth security gates** on every incoming HTTP request:
   - **Target Subnet Gate:** Refuses to start if target device IP is not RFC1918 (`10/8`, `172.16/12`, `192.168/16`) or IPv4 link-local (`169.254/16`).
   - **Strict Peer IP Pinning:** Rejects (`403 Forbidden`) any HTTP connection whose `remote_addr.ip() != paired_cast_device_ip`! Nobody else on the Wi-Fi network can touch the relay.
   - **256-Bit Per-Session Cryptographic Path Token:** URLs are `http://<lan_ip>:<port>/cast/<64-hex-token>/<video_id>.m4a`, compared in constant time (`subtle`/byte-loop).
   - **Immediate Teardown:** Dropping the `CastSession` shuts down the `LanCastRelay` listener immediately (0 open LAN ports when idle!).
4. Delegates actual byte-range audio fetching directly to Resonance's existing `LoopbackProxy` / `ChunkCache` / `OfflineStore`—meaning **Chromecast & DLNA speakers automatically get offline-vault playback, 256 KB caching, and transparent mid-stream 403 re-resolution!**
5. **Codec Compatibility for Cast Receivers:** Request **AAC `.m4a` (`141`/`140`)** preference when resolving streams for Cast/DLNA targets (many DLNA smart TVs, Sonos, and older receivers cannot decode WebM Opus `251`, whereas MP4/AAC `audio/mp4` plays universally!).

---

## 2. Exact `crates/contract` Signatures (Added in Wave 0)

### `crates/contract/src/casting.rs`
```rust
use std::net::IpAddr;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::{CoreResult, NowPlayingMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CastProtocol {
    Chromecast,
    DlnaUpnp,
    AirPlayNative,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastDevice {
    pub id: String,
    pub name: String,
    pub model: Option<String>,
    pub protocol: CastProtocol,
    pub address: IpAddr,
    pub port: u16,
    /// UPnP AVTransport control URL (DLNA only).
    pub control_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CastSessionState {
    #[default]
    Disconnected,
    Connecting { device_name: String },
    Connected {
        device: CastDevice,
        playing: bool,
        position_ms: u64,
        duration_ms: u64,
        volume_pct: u8,
        muted: bool,
    },
    Error(String),
}

#[derive(Debug, Clone)]
pub struct CastMediaPayload {
    /// Ephemeral IP-pinned LAN relay URL (`http://<lan_ip>:<port>/cast/<token>/<id>.m4a`).
    pub stream_url: String,
    pub content_type: String, // e.g. "audio/mp4"
    pub metadata: NowPlayingMetadata,
    pub start_position_ms: u64,
    pub autoplay: bool,
}

#[async_trait]
pub trait CastTransport: Send + Sync {
    async fn connect(&self, device: &CastDevice) -> CoreResult<()>;
    async fn load_media(&self, payload: CastMediaPayload) -> CoreResult<()>;
    async fn play(&self) -> CoreResult<()>;
    async fn pause(&self) -> CoreResult<()>;
    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()>;
    async fn set_volume(&self, volume: f32) -> CoreResult<()>;
    async fn set_muted(&self, muted: bool) -> CoreResult<()>;
    async fn poll_status(&self) -> CoreResult<Option<(bool, u64)>>; // (is_playing, position_ms)
    async fn disconnect(&self) -> CoreResult<()>;
}
```

---

## 3. Detailed Architecture & File-by-File Implementation Plan

### 3.1 Android Auto Browse Tree (`platform/android/.../`)
- **`platform/android/res/xml/automotive_app_desc.xml`:**
  ```xml
  <?xml version="1.0" encoding="utf-8"?>
  <automotiveApp>
      <uses name="media" />
  </automotiveApp>
  ```
- **`AndroidManifest.xml`:**
  - Add `<meta-data android:name="com.google.android.gms.car.application" android:resource="@xml/automotive_app_desc" />`.
  - Update `<service android:name=".PlaybackForegroundService" android:exported="true" android:foregroundServiceType="mediaPlayback">` with both `androidx.media3.session.MediaLibraryService` and `android.media.browse.MediaBrowserService` intent filters.
- **`AndroidAutoBrowseTree.kt` + Rust JNI Provider (`platform/android.rs` in Wave 3 stitch):**
  - Clean 2-level car-safe hierarchy (Android Auto recommends `<= 4` top-level tabs for quick glanceability):
    ```text
    "root" (Browsable)
     ├─ "tab_home"      → Home Quick Picks & Recent Mixes (from SWR SQLite cache + live refresh)
     ├─ "tab_downloads" → Offline Vault & Completed Downloads (100% instant offline!)
     ├─ "tab_library"   → Liked Songs + User Playlists + Saved Albums
     └─ "tab_queue"     → Current Active Queue (playable tracks)
    ```
  - **Voice Search (`onSearch` + `onGetSearchResult`):**
    - Handles `"Hey Google, play <song/artist> on Raag"` via `MediaLibrarySession.Callback.onSearch(...)` → dispatches to Rust `HybridMusicProvider::search()` (with instant fallback to searching local Offline Downloads if offline!) → calls `mediaLibrarySession.notifySearchResultChanged(browser, query, count, params)`.
  - **Direct Playback (`onAddMediaItems`):**
    - When Android Auto selects track `mediaId = "track:<video_id>"`, intercepts in `onAddMediaItems`, calls Rust `PlaybackController::play_track(video_id)` (which pre-registers the stream on `127.0.0.1` `LoopbackProxy` and returns the local `http://127.0.0.1:<port>/stream/<video_id>` URI), and feeds that `MediaItem` into ExoPlayer!
  - **Async Execution:** Every `MediaLibrarySession.Callback` runs off the Android main thread via `ListeningExecutorService` / Kotlin coroutine `suspendCoroutine` + `CallbackToFutureAdapter` returning `ListenableFuture<LibraryResult<...>>`—zero ANRs on car head units.

### 3.2 LAN Discovery (`crates/core/src/casting/discovery.rs`)
- Starts **only while the Cast Device Picker sheet is open** (zero background battery/mDNS drain when not casting!).
- Runs two concurrent discovery tasks on Tokio:
  1. **mDNS (`mdns-sd` crate — pure Rust):**
     - Browses `_googlecast._tcp.local.` → extracts `fn` (friendly name), `md` (model), `id`, IPv4 address, port (default `8009`). Filters out Chromecast device groups cleanly (`ca` capability bitmask checks audio support!).
     - Browses `_airplay._tcp.local.` / `_raop._tcp.local.` (for device presence display on non-Apple platforms; Apple platforms prefer native AirPlay route picker).
  2. **SSDP / UPnP (`rupnp` crate — pure Rust):**
     - Searches `urn:schemas-upnp-org:device:MediaRenderer:1` (3s MX window) → fetches device description XML (capped at 64 KB, strict RFC1918 host validation to prevent SSRF!) → extracts friendly name, model, and `AVTransport:1` / `RenderingControl:1` control URLs.
- Deduplicates devices by `(protocol, id)` and emits sorted `Vec<CastDevice>` updates to the UI sheet.

### 3.3 Pure-Rust Chromecast CastV2 Client (`crates/core/src/casting/chromecast.rs`)
**Critical Engineering Choice:** Existing crate `rust_cast` pulls in `openssl`, which breaks Android `arm64-v8a` cross-compilation and violates Resonance's pure-`rustls` invariant! Implementing the core CastV2 audio sender directly with **`tokio-rustls` + `prost`** takes ~350 lines of clean Rust and works identically on Linux, macOS, Windows, and Android:
- **Wire Protocol:**
  - TLS connection to `<device_ip>:8009` using a scoped `rustls` verifier (Chromecasts use self-signed device certs; validate TLS handshake + LAN IP pin).
  - Length-prefixed big-endian `u32` + `CastMessage` protobuf (`protocol_version = CASTV2_1_0`, `source_id = "sender-0"`, `destination_id`, `namespace`, `payload_utf8`).
- **Session Lifecycle:**
  1. `urn:x-cast:com.google.cast.tp.connection` → `{"type":"CONNECT"}` to `receiver-0`.
  2. Start background 5s heartbeat loop on `urn:x-cast:com.google.cast.tp.heartbeat` (`PING` ↔ `PONG`; disconnect cleanly after 3 missed pongs).
  3. `urn:x-cast:com.google.cast.receiver` → `{"type":"LAUNCH","appId":"CC1AD845"}` (Google Default Media Receiver) → capture `transportId` & `sessionId`.
  4. Send `CONNECT` to `<transportId>`, then send media commands on `urn:x-cast:com.google.cast.media`:
     - `LOAD` with `contentId = lan_relay_url`, `streamType = "BUFFERED"`, `contentType = "audio/mp4"`, `metadata = { metadataType: 3 (MUSIC_TRACK), title, artist, albumName, images: [{ url: artwork_url }] }`, `currentTime = start_position_ms / 1000.0`, `autoplay`.
     - `PLAY`, `PAUSE`, `SEEK`, `STOP` (using receiver's `mediaSessionId`).
     - Receiver volume via `urn:x-cast:com.google.cast.receiver` → `SET_VOLUME`.
  5. Listen to `MEDIA_STATUS` messages: when `playerState == "IDLE"` and `idleReason == "FINISHED"`, notify `PlaybackController` so the **next track in the queue auto-advances seamlessly on the Chromecast!**

### 3.4 DLNA / UPnP Renderer (`crates/core/src/casting/dlna.rs`)
- Uses `rupnp` SOAP calls against `urn:schemas-upnp-org:service:AVTransport:1` & `RenderingControl:1`:
  - `SetAVTransportURI(InstanceID=0, CurrentURI=lan_relay_url, CurrentURIMetaData=didl_lite_xml)` (properly XML-escaped DIDL-Lite `<item>` with `upnp:class` = `object.item.audioItem.musicTrack`, title, artist, album, `upnp:albumArtURI`, `res protocolInfo="http-get:*:audio/mp4:*"`).
  - `Play(Speed="1")`, `Pause()`, `Stop()`, `Seek(Unit="REL_TIME", Target="HH:MM:SS")`.
  - `GetPositionInfo()` + `GetTransportInfo()` polled every 2s while connected to drive UI progress slider and auto-advance on `STOPPED` at track end.
  - `RenderingControl::SetVolume` (0..100) and `SetMute`.

### 3.5 AirPlay Support (`crates/core/src/casting/airplay.rs`)
- **macOS (and future iOS):** Expose a native `AVRoutePickerView` via NSH's `#[native_components(macos)]` (`platform/macos_airplay.rs` or native button hook) — gives genuine, zero-latency AirPlay 2 multi-room audio routing straight from Apple's OS audio stack with zero DRM/FairPlay headaches!
- **Linux / Windows / Android:** AirPlay 2 senders require Apple HomeKit/FairPlay pairing that frequently breaks in OSS implementations; focus primary cross-platform casting on **Chromecast CastV2 + DLNA/UPnP** (covering 95%+ of smart speakers, TVs, Sonos, Kodi, and AV receivers) while keeping the `CastTransport` trait open for community RAOP plugins.

### 3.6 Secure Ephemeral `LanCastRelay` (`crates/core/src/proxy/lan_relay.rs`)
- Instantiated by `CastManager` when connecting to a `CastDevice`, dropped immediately on `disconnect()`.
- **Routing & Security Implementation:**
  ```rust
  pub struct LanCastRelay {
      bind_addr: SocketAddr,          // <local_lan_ip>:<ephemeral_port>
      allowed_peer_ip: IpAddr,        // strictly paired CastDevice.address
      session_token: [u8; 32],        // 256-bit OsRng token (64 hex chars)
      shutdown_tx: oneshot::Sender<()>,
  }
  ```
  - Incoming `hyper` request validation:
    1. Reject if `peer_addr.ip() != self.allowed_peer_ip` → `403 Forbidden`.
    2. Parse path `/cast/<token_hex>/<video_id>.<ext>` (validate `video_id` regex `[A-Za-z0-9_-]{1,64}`).
    3. Constant-time compare `<token_hex>` against `self.session_token` → `403 Forbidden` on mismatch.
    4. Forward `Range` / `HEAD` / `GET` internally to `LoopbackProxy`'s stream handler (`127.0.0.1`), returning `200`/`206` with `Accept-Ranges: bytes`, `Content-Type: audio/mp4`, `Content-Length`, and `Cache-Control: no-store`.
  - Never logs `session_token` (extend `redact_secrets()` to redact `/cast/[0-9a-f]{64}/` → `/cast/[REDACTED_CAST_TOKEN]/`).

### 3.7 Cast Manager & UI Sheet (`crates/core/src/casting/manager.rs`, `src/ui/controls/cast_sheet.rs`)
- **Seamless Audio Handover (`CastManager`):**
  - When connecting to a `CastDevice`: pauses local audio (`nsh/audio` or ExoPlayer), starts `LanCastRelay`, resolves current track (preferring AAC `.m4a` for Cast/DLNA compatibility), and calls `transport.load_media(...)` starting at `current_position_ms`!
  - While connected: `PlaybackController` routes Play/Pause/Seek/Next/Prev/Volume to `CastTransport` instead of local speakers; UI volume slider controls the Cast receiver's volume.
  - On Disconnect: tears down `LanCastRelay` and resumes local playback cleanly from the cast device's last known `position_ms`.
- **UI (`src/ui/controls/cast_sheet.rs`):**
  - Cast icon button (`IconKind::Cast`, `>= 48dp`) in Now Playing & Mini-Player bar (lights up with `theme().primary` when connected).
  - Opens bottom sheet (mobile) / popover dialog (desktop) listing discovered Chromecast & DLNA devices with protocol badges, active connection status, receiver volume slider, and `"Disconnect"` action—100% `theme()` semantic tokens, zero raw colors.

---

## 4. Security, Privacy & Concurrency Edge Cases

1. **Zero Permanent Open LAN Ports:** `LanCastRelay` ONLY binds while a user-initiated `CastSession` is active and shuts down deterministically via `Drop` / `oneshot` on disconnect, error, or app exit.
2. **SSRF Prevention in UPnP/DLNA:** Reject any SSDP `LOCATION` XML URL pointing to non-RFC1918/public IPs, `127.0.0.1` (preventing a malicious LAN device from tricking Resonance into hitting its own `LoopbackProxy` or `bgutil` sidecar!), or non-HTTP schemes.
3. **Constant-Time Token Check & Secret Redaction:** Prevents timing leaks and log exposure of cast relay URLs.
4. **Network Switch / Wi-Fi Drop During Cast:** 3 missed CastV2 heartbeats (15s) or 3 consecutive DLNA SOAP timeouts cleanly tear down `LanCastRelay`, transition `CastSessionState::Disconnected`, show a redacted status toast, and preserve local queue position.
5. **Android Auto Security (`MediaLibraryService`):** Verify calling package / controller info where appropriate; serve only read-only browse metadata and loopback media URIs (`127.0.0.1`), never exposing auth cookies or signed `googlevideo.com` URLs to external car head units.

---

## 5. Offline Verification Test Suite (`tests/lt3_android_auto_and_casting_test.rs`)

1. `lan_cast_relay_enforces_peer_ip_pinning_bearer_token_and_teardown` — spawns `LanCastRelay` on `127.0.0.1:0` (test-mode loopback override) backed by a mock `LoopbackProxy`; verifies valid token + `Range: bytes=0-1023` returns `206 Partial Content`, wrong token returns `403`, non-matching peer IP returns `403`, and `drop(relay)` closes the socket immediately.
2. `lan_cast_relay_rejects_public_non_rfc1918_target_ips` — attempts to pair with `8.8.8.8`; asserts immediate security rejection.
3. `dlna_didl_lite_xml_escapes_special_characters_and_rejects_ssrf_locations` — tests track titles containing `& < > " '` (valid XML escaping) and asserts SSDP location `http://127.0.0.1:4416/get_pot` or `http://169.254.169.254/latest/meta-data` is rejected!
4. `chromecast_castv2_framing_and_media_load_protocol_roundtrip` — connects `ChromecastTransport` to a local TLS `tokio-rustls` mock CastV2 server on `127.0.0.1:0`; verifies `CONNECT` → `LAUNCH(CC1AD845)` → `LOAD` → `PAUSE` → `SEEK` → `IDLE(FINISHED)` auto-advance event.
5. `cast_manager_handover_pauses_local_and_resumes_at_cast_position_on_disconnect` — verifies smooth position handover in both directions without losing queue state.
6. `android_auto_manifest_and_browse_tree_contract_gates` — verifies `automotive_app_desc.xml`, dual `MediaLibraryService` + `MediaBrowserService` intent filters in `AndroidManifest.xml`, and `setIsBrowsable`/`setIsPlayable` flags in `AndroidAutoBrowseTree.kt`.
7. `cast_sheet_headless_ui_renders_devices_and_zero_raw_colors` — verifies `cast_sheet` in `nsh_testing::Headless` (`390×844` & `1200×800`) with `>= 48dp` targets and zero raw color literals.

```bash
cargo test --test lt3_android_auto_and_casting_test --features lan-casting -- --nocapture
cargo test --test proxy_and_fault_injection --test phase4_hardening_and_qa
```
