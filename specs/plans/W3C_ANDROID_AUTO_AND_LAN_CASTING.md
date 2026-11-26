# W3C — Android Auto (`MediaLibraryService`) & Pure-`rustls` LAN Casting (Chromecast CastV2, DLNA/UPnP & macOS AirPlay)

> **Wave:** Wave 3 (Parallel Agent 3C — Runs concurrently with W3A, W3B after Wave 2.5 Stitch)
> **Exclusive Write Scope:**
> - `crates/core/src/casting/{mod.rs,discovery.rs,castv2.rs,castv2_proto.rs,dlna.rs,airplay.rs,controller.rs}` (new)
> - `crates/core/src/proxy/lan_relay.rs` (new session-scoped IP-pinned 256-bit-token LAN HTTP relay)
> - `src/ui/controls/cast_sheet.rs` (new Cast device picker & active cast banner)
> - `platform/android/src/main/kotlin/dev/resonance/ytmusic/{PlaybackForegroundService.kt,AutoBrowseTree.kt}` + `platform/android/res/xml/automotive_app_desc.xml` + `platform/android/AndroidManifest.xml` (Android Auto `MediaLibraryService` upgrade)
> - `tests/w3c_android_auto_and_lan_casting_test.rs` (new)
> **Read-Only Dependencies:** `crates/contract/src/casting.rs`, `mdns-sd = "0.13"`, `prost = "0.13"`, `tokio-rustls = "0.26"`, `rupnp = "2.0"` (all pure-Rust / `rustls`, ZERO OpenSSL!), `OfflineVault` (W2B), `local_media` (W2C), `LoopbackProxy`.
> **Stitch Hooks (For Wave 3.5 Integration Agent):**
> - Wire `CastController` into `src/live_engine.rs` (pausing local audio when casting begins and syncing position/volume/next-track with remote receiver)
> - Mount Cast button (`IconKind::Cast`) in `src/screens/now_playing.rs` / `src/screens/shell.rs` opening `cast_sheet::cast_device_sheet()`
> - Register `LanCastRelay` lifecycle hook with `LoopbackProxy` / `ChunkCache` / `OfflineVault` / `local_media`
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{shell,now_playing,settings}.rs`, `crates/core/src/proxy/mod.rs`.

---

## 1. Part A — Android Auto & Wear OS (`MediaLibraryService`)

### Deep Research: Why Android Auto Fails on Naive Media Services
1. **Intent-Filter Dual-Action Requirement:** Modern Android Auto & Wear OS connect via `androidx.media3.session.MediaLibraryService`, while older head units and Bluetooth systems bind via `android.media.browse.MediaBrowserService`. Declaring BOTH intent filters on `PlaybackForegroundService` (upgraded from `MediaSessionService` → `MediaLibraryService`) prevents silent connection loops.
2. **Rapid Poll / Bind-Unbind Resilience:** Android Auto routinely binds and unbinds the service while idle to populate dashboard recommendation cards. The browse tree MUST respond in `< 200 ms` from local SQLite (`library:snapshot:v1` + `OfflineVault` + `local_tracks`) **without blocking on network requests or crashing if the UI activity is dead!**
3. **Driver Safety & Distraction Guidelines:** Keep the browse hierarchy flat (max 2 levels deep), cap top-level tabs to 4–5 clear categories, and provide artwork via `ContentProvider` / loopback-safe cached URIs.

### Android Auto Architecture (`AutoBrowseTree.kt` + `PlaybackForegroundService.kt`)

```text
Android Auto Head Unit / Google Assistant Voice
       │ Binder IPC (`MediaLibrarySession.Callback`)
       ▼
PlaybackForegroundService : MediaLibraryService()
  ├─ onGetLibraryRoot() → "root" (IS_BROWSABLE)
  ├─ onGetChildren("root") → 4 Driver-Safe Tabs:
  │    1. "home_mixes"   ("Home & Mixes" — cached SWR Home sections)
  │    2. "liked_songs"  ("Liked Songs" — instant SQLite snapshot)
  │    3. "offline_vault"("Downloaded" — 100% tunnel-safe offline albums/tracks!)
  │    4. "library"      ("Playlists & Local" — user playlists + local FLAC/MP3)
  ├─ onSearch(query) / onGetSearchResult(query)
  │    └─ Voice command ("Play Radiohead on Resonance") → JNI → Rust
  │       `HybridMusicProvider::search(query)` (falls back to offline/local
  │       SQLite search automatically when cellular signal drops!)
  ├─ onPlaybackResumption()
  │    └─ Restores last persisted `QueueState` + `resume_position_ms` from SQLite
  │       when user turns car ignition on!
  └─ Manifest additions:
       • `<meta-data android:name="com.google.android.gms.car.application"
            android:resource="@xml/automotive_app_desc" />`
       • `res/xml/automotive_app_desc.xml`: `<automotiveApp><uses name="media"/></automotiveApp>`
```

---

## 2. Part B — Pure-`rustls` LAN Casting (Chromecast CastV2, DLNA/UPnP & macOS AirPlay)

### Why Pure-`rustls` + `LanCastRelay` Is Essential
1. **Zero OpenSSL Constraint:** Popular Rust cast crates like `rust_cast` pull in `openssl-sys`, which breaks `aarch64-linux-android` cross-compilation. Implementing CastV2 directly over `tokio-rustls` + `prost` (~400 LOC) is pure Rust, zero-OpenSSL, and works identically on macOS, Windows, Linux, and Android!
2. **Why We Need `LanCastRelay`:**
   - Authenticated YouTube Music streams (`SAPISID` / PO-token / IP-bound `googlevideo.com` URLs), Offline Vault files, and Local FLAC/MP3 files **cannot** be fetched directly by a Chromecast or DLNA speaker from the internet (and you must NEVER leak user cookies or signed tokens to a third-party speaker!).
   - Furthermore, Chromecast/DLNA receivers cannot reach `127.0.0.1`.
   - Therefore, Resonance starts a **transient, session-scoped `LanCastRelay`** on the active LAN interface ONLY while casting is active!

### Target LAN Casting & `LanCastRelay` Architecture

```text
┌────────────────────────────────────────────────────────────────────────────┐
│  `CastDiscoveryService` (Active ONLY while Cast Sheet is open or casting)  │
│   ├─ `mdns-sd`: `_googlecast._tcp.local` (Chromecast / Google Home / TV)   │
│   ├─ `rupnp` SSDP: `urn:schemas-upnp-org:device:MediaRenderer:1` (DLNA)    │
│   └─ macOS Native: AirPlay audio route hook (`AVRoutePickerView` bridge)   │
└───────────────────────────────────┬────────────────────────────────────────┘
                                    │ User picks Receiver (IP = R_ip)
                                    ▼
┌────────────────────────────────────────────────────────────────────────────┐
│  `LanCastRelay` (Security-Hardened Ephemeral HTTP Server)                  │
│   • Binds `<local_lan_ip>:0` ONLY on cast start; shuts down on disconnect! │
│   • Generates 256-bit cryptographically random `session_token` (64 hex)    │
│   • STRICT IP PINNING: drops ANY incoming TCP conn where `peer.ip() != R_ip│
│   • Routes:                                                                │
│       GET /cast/<session_token>/audio/<track_id>.<ext>  (HTTP 206 ranges!) │
│       GET /cast/<session_token>/art/<track_id>.jpg      (ArtworkCache)     │
│   • Data source priority: Local File → OfflineVault → ChunkCache/Upstream  │
│   • Zero cookies/PO-tokens exposed in URL; `session_token` redacted in logs│
└───────────────────────────────────┬────────────────────────────────────────┘
                                    │ `http://<lan_ip>:<port>/cast/<tok>/...`
        ┌───────────────────────────┴───────────────────────────┐
        ▼                                                       ▼
┌────────────────────────────────────────┐   ┌───────────────────────────────┐
│  Chromecast CastV2 (`castv2.rs`)       │   │  DLNA / UPnP (`dlna.rs`)      │
│  • `tokio-rustls` TCP:8009 (pinned TOFU│   │  • `rupnp` SOAP over HTTP     │
│    self-signed device cert verifier)   │   │  • `AVTransport:1`:           │
│  • 4-byte BE len + `prost` CastMessage │   │    `SetAVTransportURI`        │
│  • `tp.heartbeat` PING/PONG every 5s   │   │    (with DIDL-Lite XML meta:  │
│  • `receiver`: LAUNCH `CC1AD845`       │   │     title, artist, album art) │
│    (Google Default Media Receiver)     │   │    `Play`/`Pause`/`Seek`/`Stop│
│  • `media`: LOAD / PLAY / PAUSE / SEEK │   │  • `RenderingControl:1`:      │
│    / SET_VOLUME / GET_STATUS           │   │    `SetVolume` (0..=100)      │
│  • Auto-advances next queue track when │   │  • Polls `GetPositionInfo`    │
│    receiver emits `IDLE (FINISHED)`!   │   │    every 1s while playing     │
└────────────────────────────────────────┘   └───────────────────────────────┘
```

---

## 3. Deep Protocol & Security Implementation Details

### 3.1 Chromecast CastV2 Pure-Rust Implementation (`crates/core/src/casting/castv2.rs` & `castv2_proto.rs`)
- **Minimal Hand-Written `prost::Message` Schema (`castv2_proto.rs` — zero `protoc` build-tool dependency!):**
  ```rust
  #[derive(Clone, PartialEq, prost::Message)]
  pub struct CastMessage {
      #[prost(enumeration = "ProtocolVersion", tag = "1")]
      pub protocol_version: i32,
      #[prost(string, tag = "2")]
      pub source_id: String,
      #[prost(string, tag = "3")]
      pub destination_id: String,
      #[prost(string, tag = "4")]
      pub namespace: String,
      #[prost(enumeration = "PayloadType", tag = "5")]
      pub payload_type: i32,
      #[prost(string, optional, tag = "6")]
      pub payload_utf8: Option<String>,
      #[prost(bytes = "vec", optional, tag = "7")]
      pub payload_binary: Option<Vec<u8>>,
  }
  ```
- **Framing:** Every frame on the TLS stream is `[u32 big-endian length (capped at 64 KB)] [serialized CastMessage bytes]`.
- **TLS Verifier:** Chromecast devices present Google device-signed certificates on LAN IPs (`192.168.x.x`) that do not match public WebPKI DNS names. Implement a dedicated `CastDeviceCertVerifier` (`rustls::client::danger::ServerCertVerifier`) scoped **exclusively** to the CastV2 port `8009` socket (NEVER reused for general `reqwest` HTTPS traffic!) that validates the X.509 certificate structure and logs its SHA-256 fingerprint.
- **Session Lifecycle:**
  1. Connect `sender-0` → `receiver-0` on namespace `urn:x-cast:com.google.cast.tp.connection` (`{"type":"CONNECT"}`).
  2. Start background 5s `PING`/`PONG` heartbeat task on `urn:x-cast:com.google.cast.tp.heartbeat` (disconnects cleanly if 3 consecutive pongs are missed).
  3. Launch `CC1AD845` (Default Media Receiver) on `urn:x-cast:com.google.cast.receiver`, obtain `transportId`, send `CONNECT` to `transportId`.
  4. Send `LOAD` on `urn:x-cast:com.google.cast.media` with `contentId = relay_audio_url`, `contentType` (`audio/mp4` / `audio/webm` / `audio/flac` — note: Chromecast supports FLAC, Opus, AAC, MP3 natively!), `streamType = "BUFFERED"`, and `metadata` (`metadataType: 3` MusicTrack with title, artist, albumName, images `[relay_art_url]`).
  5. Track `mediaSessionId` and `playerState`; when `playerState == "IDLE"` and `idleReason == "FINISHED"`, notify `PlaybackController` to advance queue and `LOAD` the next track automatically!

### 3.2 DLNA / UPnP `AVTransport:1` (`crates/core/src/casting/dlna.rs`)
- Discover `MediaRenderer:1` devices via `rupnp::discover` (4s SSDP sweep).
- Inspect `GetProtocolInfo` sink MIME list so we send compatible MIME headers and clean XML-escaped DIDL-Lite metadata (`dc:title`, `upnp:artist`, `upnp:album`, `upnp:albumArtURI`).
- Handle both `SetAVTransportURI` + `Play` and `SetNextAVTransportURI` (where supported by receiver for gapless DLNA!).

### 3.3 `LanCastRelay` Security & Network Interface Detection (`crates/core/src/proxy/lan_relay.rs`)
- Determine the exact local LAN IP facing the selected receiver via a non-transmitting UDP `connect((receiver_ip, 8009))` + `socket.local_addr()` trick (works accurately across multi-homed laptops, VPNs, and Android Wi-Fi!).
- **Hard Security Checks on Every Incoming HTTP Request:**
  1. `peer_addr.ip() == allowed_receiver_ip` (reject any other LAN device immediately with connection close / `403`).
  2. Constant-time comparison of the 64-hex `session_token` path segment.
  3. Allow ONLY `GET` and `HEAD`; set `Cache-Control: no-store` and `Accept-Ranges: bytes`.
  4. Immediately drop the `TcpListener` and wipe `session_token` when the user stops casting or disconnects.

### 3.4 Cast UI (`src/ui/controls/cast_sheet.rs`)
- Cast icon in Now Playing & Desktop Player Bar (`IconKind::Cast`, filled accent color with subtle pulse ring when actively casting).
- Opening `cast_device_sheet()` triggers active mDNS + SSDP scan with inline scanning indicator and clean device list (`Living Room Speaker · Chromecast`, `Denon AVR · DLNA`, `HomePod · AirPlay`).
- Active Cast Banner across the top of Mini-Player / Desktop Player Bar: `"Casting to Living Room Speaker"` with remote volume slider and one-tap `"Disconnect"` (seamlessly resumes local playback at the exact remote timestamp if desired).

---

## 4. Deterministic Offline Verification (`tests/w3c_android_auto_and_lan_casting_test.rs`)

1. `castv2_protobuf_framing_encodes_and_decodes_load_heartbeat_and_status` — roundtrips `CastMessage` frames with 4-byte BE length prefixes, verifying malformed/oversized frames (`> 64 KB`) are rejected safely.
2. `lan_cast_relay_enforces_ip_pinning_256bit_token_and_http_206_ranges` — starts `LanCastRelay` pinned to `127.0.0.1` against mock audio bytes; asserts:
   - Valid token + `Range: bytes=0-511` returns `206 Partial Content` with exact bytes
   - Wrong token returns `403 Forbidden`
   - Non-pinned peer IP simulation returns `403 Forbidden`
   - Stop session closes listener immediately.
3. `dlna_didl_lite_metadata_escapes_xml_special_characters_safely` — tests titles/artists containing `&`, `<`, `>`, `"`, `'` to guarantee zero XML injection or malformed SOAP envelopes.
4. `android_auto_manifest_and_xml_verify_media_library_service_contract` — verifies `automotive_app_desc.xml` exists, `AndroidManifest.xml` declares both `MediaLibraryService` and `MediaBrowserService` intent filters plus car metadata, and `AutoBrowseTree.kt` handles offline fallback.
5. `cast_sheet_renders_scanning_devices_and_active_banner_headless` — headless NSH UI test verifying device selection, active cast state, and disconnect button with zero raw color literals.

```bash
cargo test --test w3c_android_auto_and_lan_casting_test -- --nocapture
```
