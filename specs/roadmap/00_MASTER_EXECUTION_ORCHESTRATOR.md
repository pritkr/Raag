# Resonance (`Raag`) — Master Architecture & Hydrogen Parallel Execution Orchestrator

> **Purpose:** This master index coordinates **10 deduplicated, deeply researched architecture & implementation specs** covering the full Horizon roadmap: **H0 ("Account Truth" & Pre-Requisites)**, **H1 / Short-Term (ST-1..ST-4)**, **H2 (Differentiation)**, **H3 (Ecosystem)**, and **Long-Term (LT-1..LT-3)**. Every subsystem has **exactly ONE canonical, robust solution and ONE spec owner**, and every parallel Hydrogen wave enforces **100% disjoint file ownership** so subagents execute concurrently with zero merge conflicts.

---

## 1. Deduplicated Spec Inventory & Scope Map

| Spec ID | Document | Horizon / Wave | Single Authoritative Scope Delivered | Primary Crates / Surfaces |
| :--- | :--- | :---: | :--- | :--- |
| **H0** | [`H0_ACCOUNT_TRUTH_AND_PREREQUISITES.md`](./H0_ACCOUNT_TRUTH_AND_PREREQUISITES.md) | **Wave 0** (Weeks 1–2, Sequential) | **H0.0** `cargo fmt --all`, single-source NSH version sync (`0.7.0`), replace `temp_dir()` with OS `AppPaths` (`directories` + Android `filesDir`), Wave 0 additive `crates/contract/` freeze & `Cargo.toml` deps; **H0.1** Boot-time Library hydration (`liked_songs()` → `LiveUpdate::LibraryLoaded` + SQLite offline snapshot); **H0.2** `SettingsV1` JSON persistence via `CacheStore::put_metadata` (protected from `clear_all_caches()`); **H0.3** Download action surface in track rows & Now Playing; **H0.4** Playlist writes (`AddVideo`/`RemoveVideo`/`MoveVideoBefore` + optimistic rollback); **H0.5** Wire `OfflineVault` as `"Pinned Collections"` + honest `"Listen Together"` → `"Radio Mix"` rename | `crates/contract/`, `crates/core/src/storage/paths.rs`, `crates/core/src/cache/db.rs`, `src/live_engine.rs`, `src/store/*`, `src/ui/controls/*`, `src/screens/{now_playing,library,settings,shell}.rs` |
| **ST-1** | [`ST1_PERSISTENCE_EJS_REMOTE_CONFIG.md`](./ST1_PERSISTENCE_EJS_REMOTE_CONFIG.md) | **Wave 1** (Parallel A) | Live `base.js` + `yt-dlp-ejs` (`jsc`) two-stage AST solver, `AccountProfileProbe` (Premium tier auto-detection + `set_premium_account`), fail-closed Ed25519 remote config client (`client_matrix.json` & `solver_manifest.json`). *(Consumes H0.0 `AppPaths` read-only.)* | `crates/core/src/solver/`, `crates/core/src/innertube/{remote_config,account_probe}.rs`, `crates/core/src/storage/bootstrap.rs` |
| **ST-2** | [`ST2_UNIFIED_AUDIO_AND_DESKTOP_NATIVE.md`](./ST2_UNIFIED_AUDIO_AND_DESKTOP_NATIVE.md) | **Wave 1** (Parallel B) | Single-source-of-truth audio loop unifying `PlaybackController` + `NshAudioBackend` + `mount_nsh_audio_driver`, `desktop-native` (`keyring` + `souvlaki`) enabled by default on Desktop & release CI, truthful DSP capability gating | `platform/nsh_audio.rs`, `platform/desktop.rs`, `crates/core/src/playback/`, `.github/workflows/release.yml` |
| **ST-3** | [`ST3_LIVE_YTM_BROWSE_YTMAPI_RUSTYPIPE_AUTH.md`](./ST3_LIVE_YTM_BROWSE_YTMAPI_RUSTYPIPE_AUTH.md) | **Wave 1** (Parallel C) | Hybrid `ytmapi-rs` (MIT) + `rustypipe` (GPL-3.0) provider replacement behind `BrowseDelegate`/`PlaybackResolver`/`CatalogExtProvider`, live Explore/Charts/Moods/Categorized Search/Continuations, Artist & Album backend queries (for H2.1), ATR watchtime sync | `crates/core/src/providers/`, `src/store/catalog_slice.rs`, `src/screens/{explore,search}.rs` |
| **ST-4** | [`ST4_ANDROID_NATIVE_HOST_AND_JNI.md`](./ST4_ANDROID_NATIVE_HOST_AND_JNI.md) | **Wave 1** (Parallel D) | Deduplicate Kotlin sources, real NSH Android host activity, full bidirectional JNI bridge (`ResonanceNativeBridge.kt` ↔ `platform/android.rs` + `nativeInitPaths` feeding H0.0 `AppPaths`), `AndroidKeystoreAuthStore`, `; wv`-stripped Android WebView sign-in | `platform/android.rs`, `platform/android/src/main/kotlin/...`, `platform/android/build.gradle.kts` |
| **H2** | [`H2_DIFFERENTIATION_ENTITIES_VAULT_LISTEN_TOGETHER.md`](./H2_DIFFERENTIATION_ENTITIES_VAULT_LISTEN_TOGETHER.md) | **Wave 2** (Parallel E) | **H2.1** Dedicated Artist & Album entity screens (discography, related artists, `"Start Radio"`); **H2.2** Offline Vault UX (one-tap album/playlist pin headers, collection progress, ref-counted orphan-safe unpin, `SettingsV1` quota enforcement); **H2.3** Real Listen Together (host-authoritative room service, `XXXX-XXXX` join codes, WebSocket primary + HTTP long-poll fallback, NTP offset + 400ms hysteresis drift sync, optional `crates/room-relay`) | `src/screens/{artist,album,listen_together}.rs`, `crates/core/src/downloads/quota.rs`, `crates/core/src/rooms/`, `crates/room-relay/`, `src/store/rooms_slice.rs` |
| **H3** | [`H3_ECOSYSTEM_AUTH_AND_LOCAL_IMPORT.md`](./H3_ECOSYSTEM_AUTH_AND_LOCAL_IMPORT.md) | **Wave 2** (Parallel F) | **H3.1** Single unified Embedded Desktop WebView + RFC 8628 TV OAuth Device-Code Sign-In (`AuthSignInOrchestrator`, `HttpOnly` verification, ephemeral WebView teardown); **H3.2** Local-File Import (user folders, incremental `lofty` `0.22` header-only FLAC/MP3/M4A/Opus/WAV scanner, deduplicated embedded artwork into `ArtworkCache`, unified playback via confined `LoopbackProxy` `/local/<track_id>`) | `crates/core/src/auth_flows/`, `src/screens/auth_sign_in_sheet.rs`, `crates/core/src/local_media/`, `crates/core/src/proxy/local_route.rs`, `src/screens/local_files.rs`, `src/store/local_media_slice.rs` |
| **LT-1** | [`LT1_ZERO_SETUP_POTOKEN_AND_SIGNED_CONFIG.md`](./LT1_ZERO_SETUP_POTOKEN_AND_SIGNED_CONFIG.md) | **Wave 2** (Parallel G) | 5-tier zero-setup PO-Token chain (hidden Desktop/Android WebView BotGuard + `rustypipe-botguard` + `bgutil`), proactive 80% TTL refresh, automated 12h `probe.yml` → Ed25519 signed config publisher | `crates/core/src/potoken/`, `.github/workflows/probe.yml`, `src/bin/client_probe.rs` |
| **LT-2** | [`LT2_NATIVE_NSH_AUTO_UPDATER.md`](./LT2_NATIVE_NSH_AUTO_UPDATER.md) | **Wave 2** (Parallel H) | Native NSH auto-updater (`nsh/updater`, `nsh update-manifest`, `latest.json.sig` Ed25519), Stable/Beta channel persisted in H0.2 `SettingsV1`, audio-aware `"Install on Quit"`, Linux Flatpak/read-only detection | `src/updater/`, `nsh.json`, `.github/workflows/release.yml` (updater manifest step) |
| **LT-3** | [`LT3_ANDROID_AUTO_AND_LAN_CASTING.md`](./LT3_ANDROID_AUTO_AND_LAN_CASTING.md) | **Wave 2** (Parallel I) | Android Auto Media3 `MediaLibraryService` + offline browse tree & voice search; pure-`rustls` Chromecast CastV2, DLNA/UPnP AVTransport, native macOS AirPlay + session-scoped IP-pinned 256-bit-token `LanCastRelay` | `crates/core/src/casting/`, `crates/core/src/proxy/lan_relay.rs`, `platform/android/.../PlaybackForegroundService.kt`, `src/ui/controls/cast_sheet.rs` |

---

## 2. Single-Solution Deduplication Ledger (Zero Competing Subsystems)

| Subsystem | Canonical Robust Solution | Single Authoritative Owner | Consumed By |
| :--- | :--- | :---: | :--- |
| **OS Persistent Directories** | `AppPaths` via `directories::ProjectDirs` (Desktop) + JNI `filesDir`/`cacheDir` (Android) + `RESONANCE_DATA_DIR` + one-time `/tmp` migration | **H0.0** | ST-1, ST-4, H2, H3, LT-1, LT-2 |
| **User Settings Persistence** | `SettingsV1` JSON via `CacheStore::put_metadata("settings:v1", ...)` + `clear_all_caches()` exclusion (`cache_key NOT LIKE 'settings:%'`) | **H0.2** | ST-2 (DSP), H2.2 (quota), H3.2 (local folders), LT-2 (update channel) |
| **Offline Collections & Quota** | Existing SQLite `OfflineVault` (`crates/core/src/downloads/vault.rs`) surfaced in H0.5 + enriched with ref-counted orphan cleanup & quota checks in H2.2 | **H0.5 → H2.2** | H0.3, H2.1 (Album header), LT-3 (Android Auto offline tree) |
| **Browse & Metadata Engine** | `HybridMusicProvider`: `ytmapi-rs` (primary) → `rustypipe` (fallback) → legacy `InnerTubeClient` + SQLite SWR cache | **ST-3** | H0.1 (`LibrarySnapshot`), H2.1 (Artist/Album), LT-3 (Auto) |
| **Playback Audio Pipeline** | Single `PlaybackController` ↔ `LoopbackProxy` (`/stream/`, `/local/`) ↔ `NshAudioBackend` ↔ `mount_nsh_audio_driver` | **ST-2** | H2.3 (Rooms), H3.2 (Local files), LT-3 (Casting) |
| **Desktop & Universal Sign-In** | `AuthSignInOrchestrator`: Embedded NSH WebView (primary w/ `HttpOnly` check) + RFC 8628 TV OAuth (universal) + Cookie Paste (advanced) | **H3.1** | ST-1 (`AccountProfileProbe`), ST-3 (`sync_session`), H0.1 (`LibraryLoaded`) |
| **Local Audio Tagging & Import** | Workspace's existing `lofty = "0.22"` + incremental SQLite `local_tracks` + `ArtworkCache` + confined `LoopbackProxy` `/local/<id>` | **H3.2** | Unified Queue, Lyrics, Android Auto, LAN Casting |
| **Signed Remote Config Envelope** | Raw bytes + companion `.sig` (64-byte Ed25519) + anti-rollback monotonic timestamp + `AuthMode` isolation check | **ST-1** (Verifier) + **LT-1** (CI Publisher) | Startup hot-config & EJS bundle updates |
| **Co-Listening Naming & UX** | H0.5 renames fake `"Listen Together"` → `"Radio Mix"`; H2.3 upgrades tab to `"Radio & Rooms"` with real host-authoritative WS + long-poll sync | **H0.5 → H2.3** | `src/screens/settings.rs`, `src/screens/shell.rs`, `src/screens/listen_together.rs` |
| **PO-Token Minting** | 5-tier `ChainedPoTokenProvider` (WebView BotGuard → `rustypipe-botguard` → `bgutil` → VM → Cold-Start) + 80% TTL refresh | **LT-1** | `InnerTubeClient` & `RustyPipePlaybackAdapter` |

---

## 3. Dependency DAG & Parallel Hydrogen Execution Waves

Execute Hydrogen subagents strictly by **Wave**. Within Waves 1 and 2, all parallel subagents have **100% disjoint write scopes**.

```text
WAVE 0 (Sequential Foundation — Execute H0.0 → H0.5 in Order)
  └─► H0 ("Account Truth" + Pre-Requisites + Wave 0 Contract/Cargo Freeze)
      • H0.0: cargo fmt --all, NSH version sync, OS AppPaths, additive crates/contract/* & Cargo.toml
      • H0.1: Boot-time Library Hydration + SQLite offline snapshot
      • H0.2: SettingsV1 persistence (protected from clear_all_caches)
      • H0.3: Download action surface (track rows + Now Playing)
      • H0.4: Playlist writes (Add/Remove/Reorder + optimistic rollback)
      • H0.5: Wire OfflineVault ("Pinned Collections") + honest "Radio Mix" rename

                ┌──────────────────────┼──────────────────────┬──────────────────────┐
                ▼                      ▼                      ▼                      ▼
WAVE 1 (Short-Term Parallel Sprint — Up to 4 Hydrogen Subagents Simultaneously)
  ┌─────────────────────┐ ┌─────────────────────┐ ┌─────────────────────┐ ┌─────────────────────┐
  │  Agent A: ST-1      │ │  Agent B: ST-2      │ │  Agent C: ST-3      │ │  Agent D: ST-4      │
  │  Live EJS, Premium  │ │  Unified Audio +    │ │  ytmapi-rs +        │ │  Android Host &     │
  │  & Remote Config    │ │  Desktop Native     │ │  rustypipe + Explore│ │  Bidirectional JNI  │
  └──────────┬──────────┘ └──────────┬──────────┘ └──────────┬──────────┘ └──────────┬──────────┘
             └───────────────────────┼───────────────────────┴───────────────────────┘
                                     ▼
WAVE 1.5 (Integration Stitch — 1 Subagent, ~20 min)
  └─► Wire ST-1..ST-4 into `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/settings.rs`
      Run `cargo test --workspace --features desktop-native` + `nsh check`

        ┌───────────────┬───────────────┬───────────────┬───────────────┐
        ▼               ▼               ▼               ▼               ▼
WAVE 2 (Differentiation, Ecosystem & Long-Term Parallel Sprint — Up to 5 Hydrogen Subagents Simultaneously)
  ┌───────────┐   ┌───────────┐   ┌───────────┐   ┌───────────┐   ┌───────────┐
  │ Agent E:  │   │ Agent F:  │   │ Agent G:  │   │ Agent H:  │   │ Agent I:  │
  │ H2 Entity │   │ H3 Auth + │   │ LT-1 Zero │   │ LT-2 NSH  │   │ LT-3 Auto │
  │ Vault+Room│   │ LocalFile │   │ PO-Token  │   │ Updater   │   │ + Casting │
  └─────┬─────┘   └─────┬─────┘   └─────┬─────┘   └─────┬─────┘   └─────┬─────┘
        └───────────────┴───────────────┴───────────────┴───────────────┘
                                        ▼
WAVE 3 (Final Integration Stitch, Architecture Gates & Release Dry-Run — 1 Subagent)
  └─► Wire H2, H3, LT-1..LT-3 into `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/shell.rs`,
      `src/screens/settings.rs`, and `crates/core/src/proxy/mod.rs` (`/local/` + `LanCastRelay`)
      Run full workspace tests, Android Gradle assemble, `nsh check`, `client_probe --offline-sim`
```

---

## 4. Strict File Ownership Matrix (Zero-Conflict Guarantee Across All 10 Specs)

> **Rule for Hydrogen Orchestrator:** During parallel Waves 1 and 2, subagents may ONLY write to their **Exclusive Write Scope**. Shared hub files (`src/live_engine.rs`, `src/store/mod.rs`, `src/screens/settings.rs`, `src/screens/shell.rs`, `crates/core/src/proxy/mod.rs`) are touched ONLY during **Wave 0 (H0)**, **Wave 1.5 Stitch**, and **Wave 3 Stitch**.

| Shared / Module Path | Wave 0 (H0) | Wave 1 (ST-1..4) | Wave 1.5 Stitch | Wave 2 (H2, H3, LT-1..3) | Wave 3 Stitch |
| :--- | :---: | :---: | :---: | :---: | :---: |
| `Cargo.toml`, `crates/*/Cargo.toml`, `crates/contract/src/**` | **H0.0 WRITE** | Read-Only | Verify | Read-Only (except new `crates/room-relay` in H2) | Verify |
| `crates/core/src/storage/paths.rs`, `crates/core/src/cache/db.rs` | **H0 WRITE** | Read-Only | Verify | Read-Only | Verify |
| `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{settings,shell}.rs` | **H0 WRITE** | **FORBIDDEN** | **WRITE** | **FORBIDDEN** | **WRITE** |
| `crates/core/src/solver/**`, `crates/core/src/innertube/{remote_config,account_probe}.rs` | Read-Only | **ST-1 ONLY** | Verify | Read-Only | Verify |
| `platform/nsh_audio.rs`, `platform/desktop.rs`, `crates/core/src/playback/**` | Read-Only | **ST-2 ONLY** | Verify | Read-Only | Verify |
| `crates/core/src/providers/**`, `src/store/catalog_slice.rs`, `src/screens/{explore,search}.rs` | Read-Only | **ST-3 ONLY** | Verify | Read-Only | Verify |
| `platform/android/**` | Read-Only | **ST-4 ONLY** | Verify | **LT-3 ONLY** (`MediaLibraryService` upgrade) | Verify |
| `src/screens/{artist,album,listen_together}.rs`, `crates/core/src/downloads/quota.rs`, `crates/core/src/rooms/**`, `crates/room-relay/**`, `src/store/rooms_slice.rs` | Read-Only | Read-Only | Read-Only | **H2 ONLY** | Wire |
| `crates/core/src/auth_flows/**`, `src/screens/auth_sign_in_sheet.rs`, `crates/core/src/local_media/**`, `crates/core/src/proxy/local_route.rs`, `src/screens/local_files.rs`, `src/store/local_media_slice.rs` | Read-Only | Read-Only | Read-Only | **H3 ONLY** | Wire |
| `crates/core/src/potoken/**`, `.github/workflows/probe.yml`, `src/bin/client_probe.rs` | Read-Only | Read-Only | Read-Only | **LT-1 ONLY** | Wire |
| `src/updater/**`, `nsh.json` | Read-Only | Read-Only | Read-Only | **LT-2 ONLY** | Wire |
| `crates/core/src/casting/**`, `crates/core/src/proxy/lan_relay.rs`, `src/ui/controls/cast_sheet.rs` | Read-Only | Read-Only | Read-Only | **LT-3 ONLY** | Wire |
| `.github/workflows/release.yml` | H0.0 (NSH sync) | **ST-2** (desktop-native) | Verify | **LT-2** (updater manifest) | Verify |

---

## 5. License & Pure-`rustls` Dependency Audit (`GPL-3.0-or-later`)

The workspace `LICENSE` and `Cargo.toml` are authoritatively **`GPL-3.0-or-later`**, unlocking clean compatibility with every chosen crate while preserving zero-OpenSSL Android `arm64-v8a` cross-compilation:

| Crate | Version | License | TLS / Cross-Compile Configuration | Single Role |
| :--- | :---: | :---: | :--- | :--- |
| `lofty` | `0.22` *(already in workspace!)* | MIT / Apache-2.0 | Pure Rust — **reused with zero new deps** | Offline tag writing + **H3.2 Local-File Import** |
| `directories` | `6.0` | MIT / Apache-2.0 | Pure Rust OS dirs | **H0.0** Desktop persistent `AppPaths` |
| `ed25519-dalek` | `2.1` | BSD-3-Clause | Pure Rust (`features = ["pkcs8"]`) | **ST-1 / LT-1** signed remote config verification |
| `ytmapi-rs` | `0.3` | MIT | `default-features = false, features = ["rustls"]` (optional `provider-ytmapi`) | **ST-3** Primary YTM `BrowseDelegate` & `CatalogExtProvider` |
| `rustypipe` | `0.11` | GPL-3.0 | `default-features = false, features = ["rustls-tls-webpki-roots", "userdata"]` (optional `provider-rustypipe`) | **ST-3 / LT-1** Fallback browse/playback + `rustypipe-botguard` |
| `jni` | `0.21` | MIT / Apache-2.0 | `[target.'cfg(target_os = "android")'.dependencies]` only | **ST-4 / LT-3** Bidirectional Rust ↔ Kotlin JNI bridge |
| `tokio-tungstenite` | `0.26` | MIT | `default-features = false, features = ["connect", "rustls-tls-webpki-roots"]` | **H2.3** Listen Together WebSocket client |
| `axum` | `0.8` | MIT | Only inside standalone `crates/room-relay` (not linked into client!) | **H2.3** Optional self-hostable Listen Together relay |
| `mdns-sd` | `0.13` | MIT / Apache-2.0 | Pure Rust mDNS | **LT-3** Chromecast & AirPlay LAN discovery |
| `prost` + `tokio-rustls` | `0.13` / `0.26` | Apache-2.0 / MIT | Pure Rust TLS (replaces OpenSSL `rust_cast`!) | **LT-3** Chromecast CastV2 protocol |
| `rupnp` | `2.0` | MIT / Apache-2.0 | Pure Rust SSDP + SOAP (`rustls`) | **LT-3** DLNA/UPnP `AVTransport:1` |

> **Critical Android Cross-Compilation Invariant:** Never introduce `openssl-sys` or `native-tls` via default crate features. Run `cargo tree --target aarch64-linux-android -i openssl-sys` (must fail with "package not found") after Wave 0!

---

## 6. Non-Negotiable Architecture Invariants (Enforced Across Every Spec)

1. **Single-Source NSH Pin:** Keep `nsh-core`, `nsh-base`, `nsh-engine`, `nsh-testing` pinned uniformly across `Cargo.toml`, `README.md`, `ARCHITECTURE.md`, `AGENTS.md`, `release.yml`, and `tests/architecture_gates_v2.rs` (synced in H0.0). Never invent NSH APIs.
2. **Non-Blocking NSH UI Engine Thread:** Zero Tokio `block_on`, SQLite writes, QuickJS eval, `lofty` folder scans, WebSocket/HTTP calls, mDNS, or JNI disk I/O on the NSH UI thread. All async work stays on `resonance-core` Tokio runtime and publishes over `crossbeam_channel` / `EventBus`.
3. **Zero Raw Colors & Mobile Ergonomics:** Zero `rgb(`, `rgba(`, or `0xFF` literals in `src/screens/` or `src/ui/controls/`. All interactive elements `>= 48dp` on mobile, `.pb_safe_area()`, no blur inside `virtual_list`, honor `motion::duration_ms` / reduced motion.
4. **Strict Auth & Security Isolation:**
   - `ANDROID_VR` and `IOS` (`AuthMode::AnonymousOnly`) NEVER receive browser cookies, `SAPISIDHASH`, or Web GVS `&pot=` tokens.
   - Local file proxy `/local/<track_id>` strictly enforces canonical path confinement inside registered `local_roots`.
   - Android `usesCleartextTraffic="false"` globally (`127.0.0.1` only in `network_security_config.xml`) and `RECEIVER_NOT_EXPORTED` on all receivers.
   - Every log/error/toast string passes through `redact_secrets()` (stripping `SAPISID`, `Cookie`, `Authorization`, `pot=`, `poToken`, OAuth tokens, `host_secret`, signed `googlevideo.com` params, and LAN cast tokens).
5. **Deterministic Offline Testing:** Every spec ships an offline test suite (`tests/<spec_name>_test.rs`) using `tempfile::TempDir` + local `127.0.0.1:0` mock servers so `cargo test --workspace` passes with zero internet access.
