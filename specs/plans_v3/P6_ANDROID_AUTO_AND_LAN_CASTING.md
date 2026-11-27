# P6: Android Auto MediaLibraryService & Real LAN Casting (Chromecast / DLNA / AirPlay)

> **Wave:** B (Parallel, after Stitch Gate A)
> **Status of Existing Code:** `crates/core/src/casting/*` (`airplay.rs`, `chromecast.rs`, `discovery.rs`, `dlna.rs`, `manager.rs`), `crates/core/src/proxy/lan_relay.rs`, `platform/android/src/main/kotlin/dev/resonance/ytmusic/{AndroidAutoBrowseTree.kt,PlaybackForegroundService.kt}`, `platform/android/res/xml/automotive_app_desc.xml`, and `src/ui/controls/cast_picker_sheet.rs` exist, **but**:
> 1. `CastManager` is never started in `live_engine`; `select_cast_device` previously faked a connected session (we just made it honest — now wire real discovery & protocol transport!).
> 2. `lan_relay.rs` needs hardened ephemeral per-session token auth and strict RFC 1918 LAN-only binding so cast receivers can pull transcoded/proxied audio safely without exposing `LoopbackProxy` to the internet.
> 3. `AndroidAutoBrowseTree.kt` & `PlaybackForegroundService.kt` need live JNI callbacks backed by Rust's `Home`, `Library`, `Playlists`, `Offline Downloads`, and `Search` slices (plus Media3 `MediaLibraryService` voice search `onSearch` + car-safe pagination!).
> **Exclusive Owned Paths:** `crates/core/src/casting/*`, `crates/core/src/proxy/lan_relay.rs`, `platform/android/src/main/kotlin/dev/resonance/ytmusic/AndroidAutoBrowseTree.kt`, `platform/android/src/main/kotlin/dev/resonance/ytmusic/PlaybackForegroundService.kt`, `platform/android/res/xml/automotive_app_desc.xml`, `src/ui/controls/cast_picker_sheet.rs`, `src/live_engine/handlers/casting.rs`, `tests/p6_casting_auto_test.rs`.

---

## 1. Deep Technical Research & Architecture

### 1.1 Real LAN Discovery & Cast Protocol Stack (`crates/core/src/casting/`)

| Protocol | Discovery Mechanism | Transport & Control | Audio Payload Delivery |
| :--- | :--- | :--- | :--- |
| **Chromecast (Google Cast)** | mDNS (`mdns-sd`) `_googlecast._tcp.local.` (TXT records: `fn`, `md`, `id`, `ca` audio capability bit) | CastV2 TLS (`port 8009`) length-prefixed Protobuf (`CastMessage`), namespaces `connection`, `heartbeat` (5s ping/pong), `receiver` (`LAUNCH` Default Media Receiver `CC1AD845`), `media` (`LOAD`, `PLAY`, `PAUSE`, `SEEK`, `SET_VOLUME`) | `LanRelay` HTTP stream (`audio/mp4` AAC 140 or `audio/webm` Opus 251) with `Content-Length` + `Accept-Ranges: bytes` + rich metadata (`MusicTrackMediaMetadata`: title, artist, album, artwork URL) |
| **DLNA / UPnP AV** | SSDP UDP multicast `239.255.255.250:1900` (`ST: urn:schemas-upnp-org:device:MediaRenderer:1`) -> fetch XML Device Description -> locate `AVTransport:1` & `RenderingControl:1` `controlURL` | SOAP XML over HTTP (`SetAVTransportURI` with DIDL-Lite XML metadata, `Play`, `Pause`, `Seek`, `Stop`, `GetPositionInfo`, `SetVolume`) | `LanRelay` HTTP stream (`audio/mp4` / `audio/mpeg` / `audio/flac`) — prefer M4A/AAC (`itag 140`) over WebM/Opus for maximum speaker/AVR compatibility! |
| **AirPlay 1 / RAOP (Experimental)** | mDNS `_airplay._tcp.local.` & `_raop._tcp.local.` | HTTP `/play` (AirPlay 1 URL playback on compatible receivers) or mark AirPlay 2 FairPlay-encrypted devices honestly as `"Requires AirPlay 2 pairing (unsupported)"` rather than silently failing! | `LanRelay` HTTP M4A stream |

### 1.2 Security Architecture for `LanRelay` (`crates/core/src/proxy/lan_relay.rs`)
Cast devices on your Wi-Fi cannot connect to `127.0.0.1` — they need a LAN IP (`192.168.x.x` / `10.x.x.x` / `172.16-31.x.x`). Exposing an open proxy on `0.0.0.0` would be a severe SSRF/credential leak vulnerability. Therefore `LanRelay` MUST enforce:
1. **Active-Session-Only Lifecycle:** `LanRelay` listener binds ONLY when a Cast session is actively started and **immediately shuts down** when casting disconnects!
2. **Ephemeral Unpredictable Capability URL:** Serve ONLY `/cast/<32_hex_session_token>/<track_id>.<ext>` using constant-time token comparison (`subtle::ConstantTimeEq`). Rotate token on every new cast session.
3. **Strict Peer IP Pinning:** Reject any TCP connection whose `peer_addr().ip()` does NOT match the connected Cast receiver's IP address (or RFC 1918 subnet for multi-homed speakers)!
4. **Zero Upstream Secrets Exposed:** Never forward user cookies or signed YouTube URLs to the cast receiver; `LanRelay` fetches chunks server-side via `LoopbackProxy`/`ChunkCache` (or local offline/local-media file) and streams raw audio bytes to the receiver.

### 1.3 Android Auto (`MediaLibraryService`) & Media3 Foreground Hardening
Google enforces strict Android Auto UX & safety guidelines:
1. **Manifest & Service Setup:**
   - `PlaybackForegroundService` extends `androidx.media3.session.MediaLibraryService` (replacing plain `MediaSessionService`).
   - Declare `android.media.browse.MediaBrowserService` + `androidx.media3.session.MediaLibraryService` intent filters + `<meta-data android:name="com.google.android.gms.car.application" android:resource="@xml/automotive_app_desc" />`.
   - Maintain `RECEIVER_NOT_EXPORTED` on all dynamic BroadcastReceivers (becoming noisy / audio focus) and `FOREGROUND_SERVICE_MEDIA_PLAYBACK` permission!
2. **Car-Safe Browse Tree (`AndroidAutoBrowseTree.kt`):**
   - Root (`root`) exposes 4 concise car-friendly tabs (strictly obeying Android Auto's max 4 root tabs rule!):
     1. `tab_home` (**Home / Quick Picks**) — top 12 quick picks + recent mixes
     2. `tab_library` (**Library**) — Liked Songs + User Playlists
     3. `tab_downloads` (**Downloads**) — Offline Vault tracks & pinned albums (100% available in tunnels/no-signal driving!)
     4. `tab_queue` (**Up Next**) — current active queue
   - Implement `onSearch(query)` & `onGetSearchResult(query)` so Google Assistant voice commands (*"Play Lo-Fi Beats on Raag"*) execute real `HybridCatalogProvider::search()` via JNI and start playback!
   - Paginate `onGetChildren(parentId, page, pageSize)` cleanly and attach high-contrast artwork URIs (`ContentProvider` or cached HTTPS thumbnails) so car dashboards render crisp album art.

---

## 2. Concrete Implementation Plan (Step-by-Step)

### Step 1: Real Discovery & Cast Protocols (`crates/core/src/casting/`)
- **`discovery.rs`:** Wire real `mdns-sd` (`_googlecast._tcp.local.`, `_airplay._tcp.local.`) and async UDP SSDP (`tokio::net::UdpSocket`) with deduplication by `device_id` and stale-device eviction after 60s.
- **`chromecast.rs`:** Implement real CastV2 TLS connection (`tokio-rustls` with Cast receiver self-signed cert verifier scoped strictly to the discovered LAN IP), protobuf framing, 5s `PING`/`PONG` heartbeat watchdog, `LOAD` with `LanRelay` URL + artwork/title/artist metadata, live `MEDIA_STATUS` polling for position/duration sync, and `SET_VOLUME`.
- **`dlna.rs`:** Implement XML device description parser (strict XXE-safe parser like `roxmltree`, never entity-expanding!), DIDL-Lite XML escaping, `SetAVTransportURI`, `Play`, `Pause`, `Seek`, `GetPositionInfo`, and `SetVolume`.
- **`manager.rs`:** Unify active session state machine (`Disconnected -> Connecting -> Connected -> Reconnecting -> Failed(reason)`), auto-advancing queue on receiver `IDLE(FINISHED)` status!

### Step 2: Hardened `LanRelay` (`crates/core/src/proxy/lan_relay.rs`)
- Bind ephemeral port (`:0`) on the active LAN interface matching the receiver subnet.
- Enforce constant-time 128-bit session token + peer IP allowlist + Range `206` streaming from `ChunkCache` / `OfflineVault` / `LocalMediaStore`.
- Drop listener immediately on `disconnect()`.

### Step 3: Live Wiring & UI (`src/live_engine/handlers/casting.rs` + `cast_picker_sheet.rs`)
- Start mDNS/SSDP discovery only while the Cast Picker Sheet is open (or when user clicks `"Scan Devices"`) — zero background battery drain when not casting!
- Show honest discovery states (`"Scanning local Wi-Fi…"`, `"No Chromecast or DLNA speakers found on this network"`, `"Connecting to Living Room Speaker…"`), protocol badge (`Chromecast` / `DLNA` / `AirPlay`), live volume slider, and `"Disconnect"` button.
- Wire `PlaybackController` so Play/Pause/Seek/Next/Volume route to the active `CastManager` session while local speaker audio is paused cleanly.

### Step 4: Android Auto JNI Browse Tree (`AndroidAutoBrowseTree.kt` + `PlaybackForegroundService.kt`)
- Add JNI read-only JSON endpoints `nativeAutoGetChildren(parentId, page, pageSize)` and `nativeAutoSearch(query)` in `platform/android.rs` backed by `store` / `HybridCatalogProvider` / `OfflineVault`.
- Populate `MediaItem`s with `MediaMetadata` (`MEDIA_TYPE_FOLDER_PLAYLISTS`, `MEDIA_TYPE_MUSIC`, `isBrowsable`, `isPlayable`) and artwork URI.

---

## 3. Security Invariants & Acceptance Tests (`tests/p6_casting_auto_test.rs`)

1. **LanRelay Peer IP & Token Isolation Test:**
   - Start `LanRelay` pinned to `127.0.0.1` with token `T`.
   - Assert request with wrong token returns `403`.
   - Assert request with valid token returns `206` byte-range audio AND response headers contain zero upstream `Cookie` or `googlevideo.com` URLs.
   - Assert calling `shutdown()` closes the TCP listener immediately.
2. **XXE-Safe DLNA XML Parser Test:** Feed malicious XML `<!DOCTYPE foo [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>` to `dlna.rs` parser -> assert clean rejection with zero file reads.
3. **CastV2 Framing & Heartbeat Test:** Spin up mock local TLS CastV2 receiver -> verify `CONNECT` -> `LAUNCH(CC1AD845)` -> `LOAD` -> `PLAY` -> `SEEK` -> `PAUSE` sequence and heartbeat watchdog recovery.
4. **Android Security Gate Test:** Verify `AndroidManifest.xml` keeps `usesCleartextTraffic` disabled (`network_security_config.xml` allows cleartext on `127.0.0.1` only), `RECEIVER_NOT_EXPORTED` remains intact, and `automotive_app_desc.xml` declares `<uses name="media" />`.
