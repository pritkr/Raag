# Resonance (`Raag`) — Master Architecture & Hydrogen Parallel Execution Orchestrator (v2)

> **Authoritative Execution Suite:** `/specs/plans/` supersedes the earlier draft specs in `/specs/roadmap/` with deeply researched, production-grade architectures covering **every product, engine, audio, UI/UX, and platform feature** (explicitly **excluding Ecosystem Integrations** such as Last.fm/ListenBrainz scrobbling, Discord Rich Presence, and cloud settings sync).
>
> **Core Guarantee:** Every parallel Hydrogen wave enforces **100% disjoint file ownership**, a **frozen additive `crates/contract/` boundary**, **zero-OpenSSL (`rustls`-only)** Android `arm64-v8a` compatibility, **GPL-3.0-or-later** license cleanliness, and **deterministic offline test suites** so subagents can execute concurrently with zero merge conflicts.

---

## 1. Scope Boundary & Complete Feature Coverage Matrix

### Explicitly Excluded (Per Product Scope)
- Third-party scrobbling (Last.fm, ListenBrainz)
- Discord Rich Presence
- External cloud settings/playlist backup & cross-device sync services

### Complete Included Plan Inventory (13 Deep-Research Specs + Master Orchestrator)

| Plan ID | Document | Wave & Mode | Single Authoritative Scope Delivered |
| :--- | :--- | :---: | :--- |
| **W0** | [`W0_FOUNDATION_PERSISTENCE_AND_CONTRACTS.md`](./W0_FOUNDATION_PERSISTENCE_AND_CONTRACTS.md) | **Wave 0** *(Sequential)* | OS persistent `AppPaths` (`directories` + Android `filesDir` + `/tmp` migration), `SettingsV1` SQLite persistence (immune to `clear_all_caches()`), working Light/Dark/System theme switching, real Fisher–Yates shuffle deck, boot-time Account Truth library hydration, NSH `=0.7.0` version sync, and frozen additive `crates/contract/` + `Cargo.toml` deps |
| **W1A** | [`W1A_UI_CRAFT_UX_POLISH_AND_ERGONOMICS.md`](./W1A_UI_CRAFT_UX_POLISH_AND_ERGONOMICS.md) | **Wave 1** *(Parallel A)* | Desktop responsive layout fix, mobile bottom nav & `>= 48dp` mood chip fix, heart color bug, queue artwork, SVG repeat icons, truthful codec badge, Settings/Library separation, consumer copy de-jargoning, universal Track Context Menu (`...`), Playlist CRUD UI, Download action surface, synced lyrics auto-scroll with user-override hysteresis, Sleep Timer, Playback Speed/Pitch controls, full global keyboard shortcut map, Queue Up Next / Autoplay ergonomics |
| **W1B** | [`W1B_LIVE_INNERTUBE_EJS_AND_REMOTE_CONFIG.md`](./W1B_LIVE_INNERTUBE_EJS_AND_REMOTE_CONFIG.md) | **Wave 1** *(Parallel B)* | Live two-stage `base.js` + `yt-dlp-ejs` AST challenge solver in sandboxed QuickJS-NG (`rquickjs 0.12.0`), automatic YouTube Music Premium detection (`AccountProfileProbe`), fail-closed Ed25519 signed remote config (`client_matrix.json` & `solver_manifest.json`) with anti-rollback & `AuthMode` isolation checks |
| **W1C** | [`W1C_HYBRID_BROWSE_SEARCH_AND_DISCOVERY.md`](./W1C_HYBRID_BROWSE_SEARCH_AND_DISCOVERY.md) | **Wave 1** *(Parallel C)* | Hybrid `ytmapi-rs` (primary) + `rustypipe` (fallback) + native parser + SQLite SWR cache; real live `FEmusic_explore` / Charts / `FEmusic_moods_and_genres` drill-down; dynamic Home mood chips; live `/music/get_search_suggestions` autocomplete + SQLite search history + categorized search tabs & continuations; real Account Library endpoints & opt-in ATR watchtime sync |
| **W1D** | [`W1D_UNIFIED_AUDIO_CONTROLLER_AND_DESKTOP_NATIVE.md`](./W1D_UNIFIED_AUDIO_CONTROLLER_AND_DESKTOP_NATIVE.md) | **Wave 1** *(Parallel D)* | Single-source-of-truth `PlaybackController` ↔ `NshAudioControl`/`NshAudioTelemetry` loop with monotonic generation fencing, seek-echo elimination, real 50%/75% prefetch & SponsorBlock skipping; default-on Desktop `keyring` + `souvlaki` (MPRIS/SMTC) with Windows `HWND` recovery; truthful `AudioDspCapability` gating |
| **W1E** | [`W1E_ANDROID_NATIVE_HOST_AND_JNI.md`](./W1E_ANDROID_NATIVE_HOST_AND_JNI.md) | **Wave 1** *(Parallel E)* | Deduplicate Kotlin sources, replace `MainActivity.kt` stub with real NSH Rust host bootstrap + predictive back + edge-to-edge insets, full bidirectional `jni = "0.21"` bridge, Media3 `PlaybackForegroundService` audio focus/becoming-noisy handling, `AndroidKeystoreAuthStore`, `; wv`-stripped Android WebView sign-in |
| **W2A** | [`W2A_NATIVE_PCM_DSP_GAPLESS_AUDIO_ENGINE.md`](./W2A_NATIVE_PCM_DSP_GAPLESS_AUDIO_ENGINE.md) | **Wave 2** *(Parallel A)* | True audiophile native PCM pipeline: Desktop `symphonia` + dual-slot gapless decoder + WSOLA pitch-preserving time-stretcher + existing `DspPipeline` (10-band RBJ biquad EQ, EBU R128 ReplayGain, Silence Skip, Equal-Power Crossfade) + `rubato` RT-safe resampling + lock-free SPSC ring buffer + `cpal` RT callback; Android ExoPlayer custom `AudioProcessor` calling Rust `DspPipeline` via zero-copy direct `ByteBuffer` JNI |
| **W2B** | [`W2B_ENTITY_SCREENS_AND_OFFLINE_VAULT.md`](./W2B_ENTITY_SCREENS_AND_OFFLINE_VAULT.md) | **Wave 2** *(Parallel B)* | Dedicated Artist & Album entity screens (adaptive hero, discography carousels, top songs, subscribe, related artists); First-Class Offline Vault UX (one-tap album/playlist pin, `"Keep Liked Songs Offline"`, ref-counted orphan-safe unpin, `SettingsV1` storage quota, Wi-Fi-only guard, SHA-256 disk integrity audit/repair) |
| **W2C** | [`W2C_AUTH_FLOWS_AND_LOCAL_MEDIA_IMPORT.md`](./W2C_AUTH_FLOWS_AND_LOCAL_MEDIA_IMPORT.md) | **Wave 2** *(Parallel C)* | Unified `AuthSignInOrchestrator`: Embedded Desktop/Android WebView sign-in (`HttpOnly` check + ephemeral teardown) + RFC 8628 YouTube TV OAuth Device-Code flow (QR/code sheet, refresh tokens) + hidden Advanced cookie paste; Local Audio Library Import (`lofty` incremental stat-diff scanner, deduplicated artwork, ReplayGain/lyrics extraction, confined `/local/<track_id>` proxy) |
| **W2D** | [`W2D_REAL_LISTEN_TOGETHER_ROOMS.md`](./W2D_REAL_LISTEN_TOGETHER_ROOMS.md) | **Wave 2** *(Parallel D)* | Real multi-user synchronized listening: host-authoritative rooms, `XXXX-XXXX` join codes, WebSocket primary (`tokio-tungstenite`) + HTTP long-poll fallback, Cristian/NTP median-RTT clock offset $\theta$, 3-zone hysteresis drift controller (`<60ms` deadband / `60–450ms` smooth rate slew / `>=450ms` scheduled hard seek), standalone self-hostable `crates/room-relay` (`axum`) |
| **W3A** | [`W3A_ZERO_SETUP_POTOKEN_AND_SIGNED_CI.md`](./W3A_ZERO_SETUP_POTOKEN_AND_SIGNED_CI.md) | **Wave 3** *(Parallel A)* | 5-tier zero-setup PO-Token chain (offscreen WebView BotGuard → `rustypipe-botguard` → `bgutil-pot` → VM script → cold-start anonymous), proactive 80% TTL refresh, single-flight mint coalescer, automated 12h `.github/workflows/probe.yml` signing & publishing `client_matrix.json` + `solver_manifest.json` |
| **W3B** | [`W3B_NATIVE_AUTO_UPDATER.md`](./W3B_NATIVE_AUTO_UPDATER.md) | **Wave 3** *(Parallel B)* | Native NSH auto-updater (`nsh/updater`, Ed25519 `latest.json.sig`), Stable/Beta channels in `SettingsV1`, audio-aware `"Install on Quit"`, Flatpak/Snap/Homebrew/read-only Linux package-manager detection, signed Android APK update prompt |
| **W3C** | [`W3C_ANDROID_AUTO_AND_LAN_CASTING.md`](./W3C_ANDROID_AUTO_AND_LAN_CASTING.md) | **Wave 3** *(Parallel C)* | Android Auto Media3 `MediaLibraryService` + driver-safe offline-first browse tree + Google Assistant voice search; Pure-`rustls` Chromecast CastV2 (`tokio-rustls` + `prost` + `mdns-sd`), DLNA/UPnP `AVTransport:1` (`rupnp`), native macOS AirPlay, and session-scoped IP-pinned 256-bit-token `LanCastRelay` |

---

## 2. Parallel Hydrogen Execution DAG

```text
====================================================================================
WAVE 0 — SEQUENTIAL FOUNDATION (1 Subagent — Must Complete & Pass CI Before Wave 1)
====================================================================================
  [ W0: OS AppPaths, SettingsV1, Theme Switch, Real Shuffle, Account Hydration,
        NSH =0.7.0 Sync, Frozen crates/contract/* & Workspace Cargo.toml Deps ]
                                       │
       ┌───────────────┬───────────────┼───────────────┬───────────────┐
       ▼               ▼               ▼               ▼               ▼
====================================================================================
WAVE 1 — CORE RELIABILITY, UI CRAFT & PLATFORM SPRINT (Up to 5 Parallel Subagents)
====================================================================================
  [ Agent 1A ]    [ Agent 1B ]    [ Agent 1C ]    [ Agent 1D ]    [ Agent 1E ]
  UI Craft,       Live EJS AST,   Hybrid Browse,  Unified Audio   Android Real
  Layout Fixes,   Premium Probe,  Live Explore,   Controller &    NSH Host &
  Menus, Lyrics,  Ed25519 Remote  Suggestions &   Desktop Native  Bidirectional
  Sleep, Keys     Config          Account Lib     (MPRIS/Keyring) JNI Bridge
       │               │               │               │               │
       └───────────────┴───────────────┼───────────────┴───────────────┘
                                       ▼
====================================================================================
WAVE 1.5 — INTEGRATION STITCH #1 (1 Subagent — ~20 min)
====================================================================================
  • Wire W1A..W1E into hub files: `src/live_engine.rs`, `src/store/mod.rs`,
    `src/screens/shell.rs`, `src/screens/settings.rs`, `src/lib.rs`
  • Gate: `cargo test --workspace --features desktop-native` + `nsh check`
                                       │
             ┌─────────────────────────┼─────────────────────────┬─────────────────────────┐
             ▼                         ▼                         ▼                         ▼
====================================================================================
WAVE 2 — AUDIOPHILE DSP, DIFFERENTIATION & AUTH/LOCAL SPRINT (Up to 4 Parallel Subagents)
====================================================================================
       [ Agent 2A ]              [ Agent 2B ]              [ Agent 2C ]              [ Agent 2D ]
       Native PCM DSP,           Artist & Album            WebView + TV              Real Multi-User
       Gapless/Crossfade,        Entity Screens &          OAuth Sign-In &           Listen Together
       WSOLA Speed &             Offline Vault             Local Media               Rooms & NTP/Slew
       Android JNI EQ            Quotas/Pinning            Import (lofty)            Sync + Relay
             │                         │                         │                         │
             └─────────────────────────┴─────────────────────────┴─────────────────────────┘
                                                   ▼
====================================================================================
WAVE 2.5 — INTEGRATION STITCH #2 (1 Subagent — ~20 min)
====================================================================================
  • Wire W2A..W2D into `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/shell.rs`,
    `src/screens/settings.rs`, `crates/core/src/proxy/mod.rs` (`/local/`)
  • Flip `AudioDspCapability::NativePcmPipeline` where PCM engine is active
  • Gate: `cargo test --workspace --features desktop-native` + `nsh check`
                                                   │
                     ┌─────────────────────────────┼─────────────────────────────┐
                     ▼                             ▼                             ▼
====================================================================================
WAVE 3 — RESILIENCE, AUTO-UPDATE & LIVING-ROOM SPRINT (Up to 3 Parallel Subagents)
====================================================================================
               [ Agent 3A ]                  [ Agent 3B ]                  [ Agent 3C ]
               5-Tier Zero-Setup             Native Signed                 Android Auto
               PO-Token Chain &              NSH Auto-Updater              MediaLibrary &
               12h Signed Probe              (Audio-Aware Quit)            LAN Casting
                     │                             │                             │
                     └─────────────────────────────┼─────────────────────────────┘
                                                   ▼
====================================================================================
WAVE 3.5 — FINAL RELEASE STITCH & ARCHITECTURE GATES (1 Subagent)
====================================================================================
  • Wire W3A..W3C into `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/shell.rs`,
    `src/screens/settings.rs`, `crates/core/src/proxy/mod.rs` (`LanCastRelay`)
  • Verify zero `openssl-sys` on `aarch64-linux-android`
  • Full workspace test suite, headless UI checks, Android Gradle assemble, `nsh check`
```

---

## 3. Strict File Ownership Matrix (Zero Merge Conflicts Across All Waves)

> **Hard Rule for Every Hydrogen Subagent:** During parallel Waves 1, 2, and 3, an agent may **ONLY** create or modify files listed in its **Exclusive Write Scope**. Hub files (`src/live_engine.rs`, `src/store/mod.rs`, `src/screens/shell.rs`, `src/screens/settings.rs`, `crates/core/src/proxy/mod.rs`, `Cargo.toml`, `crates/contract/**`) are **STRICTLY READ-ONLY** during parallel waves and are stitched exclusively during **Wave 0**, **Wave 1.5**, **Wave 2.5**, and **Wave 3.5**.

| Path / Module | W0 | W1 Parallel Owner | W1.5 | W2 Parallel Owner | W2.5 | W3 Parallel Owner | W3.5 |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| `Cargo.toml`, `crates/{contract,core}/Cargo.toml`, `crates/contract/src/**` | **W0** | RO | Verify | RO *(W2D adds `crates/room-relay/`)* | Verify | RO | Verify |
| `crates/core/src/storage/**`, `crates/core/src/cache/db.rs` | **W0** | RO | Verify | RO | Verify | RO | Verify |
| **Hubs:** `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{shell,settings}.rs`, `src/lib.rs` | **W0** | **FORBIDDEN** | **STITCH** | **FORBIDDEN** | **STITCH** | **FORBIDDEN** | **STITCH** |
| `crates/core/src/proxy/mod.rs` | RO | RO | RO | RO *(W2C writes `local_route.rs`)* | **STITCH** | RO *(W3C writes `lan_relay.rs`)* | **STITCH** |
| `src/ui/**`, `src/screens/{home,now_playing,queue,lyrics,library}.rs`, `src/store/{ui_slice,mutation_slice,library_slice}.rs` | W0 *(slice hooks)* | **W1A ONLY** | Wire | RO | Verify | RO | Verify |
| `crates/core/src/solver/**`, `crates/core/src/innertube/{remote_config,account_probe}.rs`, `assets/ejs/**`, `config/client_matrix.json` | RO | **W1B ONLY** | Wire | RO | Verify | RO | Verify |
| `crates/core/src/providers/**`, `src/store/catalog_slice.rs`, `src/screens/{explore,search}.rs`, `crates/core/src/innertube/parser/**` | RO | **W1C ONLY** | Wire | RO | Verify | RO | Verify |
| `platform/nsh_audio.rs`, `platform/desktop.rs`, `platform/mod.rs`, `crates/core/src/playback/**`, `src/store/{playback_slice,settings_slice}.rs`, `src/screens/settings_dsp_card.rs` | RO | **W1D ONLY** | Wire | RO *(W2A adds `pcm_engine/`)* | Wire | RO | Verify |
| `platform/android.rs`, `platform/android/**` | RO | **W1E ONLY** | Wire | **W2A ONLY** *(new `ResonanceDspAudioProcessor.kt` only)* | Wire | **W3C ONLY** *(MediaLibraryService upgrade)* | Wire |
| `crates/core/src/audio_dsp/**`, `crates/core/src/pcm_engine/**`, `platform/pcm_backend.rs` | RO | RO | RO | **W2A ONLY** | Wire | RO | Verify |
| `src/screens/{artist,album}.rs`, `crates/core/src/downloads/{vault,quota}.rs`, `src/ui/controls/vault_widgets.rs` | RO | RO | RO | **W2B ONLY** | Wire | RO | Verify |
| `crates/core/src/auth_flows/**`, `crates/core/src/local_media/**`, `crates/core/src/proxy/local_route.rs`, `src/screens/{auth_sign_in_sheet,local_files}.rs`, `src/store/local_media_slice.rs` | RO | RO | RO | **W2C ONLY** | Wire | RO | Verify |
| `crates/core/src/rooms/**`, `crates/room-relay/**`, `src/screens/listen_together.rs`, `src/store/rooms_slice.rs` | RO | RO | RO | **W2D ONLY** | Wire | RO | Verify |
| `crates/core/src/potoken/**`, `.github/workflows/probe.yml`, `src/bin/client_probe.rs` | RO | RO | RO | RO | RO | **W3A ONLY** | Wire |
| `src/updater/**`, `nsh.json` *(updater section)*, `.github/workflows/release.yml` *(updater manifest)* | RO | W1D *(desktop-native flags)* | Verify | RO | RO | **W3B ONLY** | Wire |
| `crates/core/src/casting/**`, `crates/core/src/proxy/lan_relay.rs`, `src/ui/controls/cast_sheet.rs` | RO | RO | RO | RO | RO | **W3C ONLY** | Wire |

---

## 4. Pure-`rustls` & `GPL-3.0-or-later` Dependency Ledger (Frozen in Wave 0)

Every workspace dependency is audited for **GPL-3.0-or-later** compatibility and **zero-OpenSSL** Android `arm64-v8a` cross-compilation:

| Crate | Version | License | Cargo Feature Configuration | Owner Plan(s) |
| :--- | :---: | :---: | :--- | :--- |
| `directories` | `6.0` | MIT / Apache-2.0 | default (pure Rust OS paths) | **W0** |
| `ed25519-dalek` | `2.1` | BSD-3-Clause | `features = ["pkcs8"]` | **W1B, W3A, W3B** |
| `ytmapi-rs` | `0.3` | MIT | `default-features = false, features = ["rustls"]` (feature-gated `provider-ytmapi`) | **W1C** |
| `rustypipe` | `0.11` | GPL-3.0 | `default-features = false, features = ["rustls-tls-webpki-roots", "userdata"]` (feature-gated `provider-rustypipe`) | **W1C, W3A** |
| `keyring` | `3.6` *(existing)* | MIT / Apache-2.0 | Desktop default (`apple-native`, `windows-native`, `sync-secret-service`, `crypto-rust`) | **W1D** |
| `souvlaki` | `0.8` *(existing)* | MIT | Desktop default (OS media keys / MPRIS / SMTC) | **W1D** |
| `jni` | `0.21` | MIT / Apache-2.0 | `[target.'cfg(target_os = "android")'.dependencies]` | **W1E, W2A, W3C** |
| `symphonia` | `0.5` | MPL-2.0 | `default-features = false, features = ["aac", "flac", "isomp4", "mkv", "mp3", "ogg", "pcm", "vorbis", "wav", "opt-simd"]` + `symphonia-adapter-libopus` or pure-Rust Opus (`audiopus` / `libopus_static` or `opus` feature check) | **W2A** |
| `cpal` | `0.15` | Apache-2.0 | `[target.'cfg(not(target_os = "android"))'.dependencies]` (CoreAudio / WASAPI / ALSA-PipeWire) | **W2A** |
| `rubato` | `0.16` | MIT | Pure Rust RT-safe FFT/Sinc resampler | **W2A, W2D** |
| `lofty` | `0.22` *(existing)* | MIT / Apache-2.0 | Reused with zero new deps (offline tagging + local import) | **W2B, W2C** |
| `tokio-tungstenite` | `0.26` | MIT | `default-features = false, features = ["connect", "rustls-tls-webpki-roots"]` | **W2D** |
| `axum` | `0.8` | MIT | Only inside standalone `crates/room-relay/` (never linked into client!) | **W2D** |
| `mdns-sd` | `0.13` | MIT / Apache-2.0 | Pure Rust mDNS | **W3C** |
| `prost` + `tokio-rustls` | `0.13` / `0.26` | Apache-2.0 / MIT | Pure Rust CastV2 protobuf + TLS (zero OpenSSL!) | **W3C** |
| `rupnp` | `2.0` | MIT / Apache-2.0 | Pure Rust SSDP + SOAP (`rustls`) | **W3C** |

> **Mandatory CI Audit Command (after Wave 0 & Wave 3.5):**
> `cargo tree --target aarch64-linux-android -i openssl-sys` MUST exit non-zero (`package ID specification 'openssl-sys' did not match any packages`).

---

## 5. Non-Negotiable Architecture & Taste Invariants

Every Hydrogen subagent MUST enforce all 7 invariants (verified by `tests/architecture_gates_v2.rs` and per-plan test suites):

1. **NSH `=0.7.0` Single-Source Pin:** Never upgrade or drift `nsh-core`, `nsh-base`, `nsh-engine`, `nsh-testing`. Never invent NSH methods—verify against `.agents/skills/nsh/references/rust.md` and `rust-ui.md`.
2. **Non-Blocking NSH UI Engine Thread:** Zero `block_on`, SQLite writes, QuickJS eval, `lofty` folder scans, HTTP/WebSocket calls, mDNS, or JNI disk I/O on the NSH engine thread. All async work executes on the `resonance-core` Tokio runtime and publishes over `crossbeam_channel`.
3. **Real-Time Audio Thread Purity (Wave 2A):** Inside the `cpal` audio callback and Android `AudioProcessor::queueInput` JNI entry point: **zero heap allocations, zero locks/mutexes, zero syscalls, zero logging**. Communicate exclusively via lock-free atomics and `SpscRingBuffer`.
4. **Design Taste & Token Discipline:** Zero `rgb(`, `rgba(`, or `0xFF` color literals in `src/screens/` or `src/ui/controls/`. All colors, radii, and spacing derive from `theme.json` / `src/ui/tokens.rs`. Mobile interactive targets `>= 48dp` (use `.min_h(px(48.0))` or `.hit_slop(...)`), `.pb_safe_area()`, zero blur inside `virtual_list`, honor `motion::duration_ms` / reduced motion, and **zero engineering jargon** in consumer-facing UI copy.
5. **Strict Auth & Network Isolation:**
   - `ANDROID_VR` and `IOS` (`AuthMode::AnonymousOnly`) NEVER receive browser cookies, `SAPISIDHASH`, or Web GVS `&pot=` tokens.
   - Local file proxy `/local/<track_id>` enforces canonical path confinement inside user-approved `local_roots`.
   - `LanCastRelay` binds transiently ONLY during active cast sessions, requires a 256-bit random path token, and rejects any peer IP other than the active receiver.
   - Android keeps `usesCleartextTraffic="false"` (`127.0.0.1` only in `network_security_config.xml`) and `RECEIVER_NOT_EXPORTED`.
   - All logs, errors, and toasts pass through `redact_secrets()`.
6. **Honest UI Principle:** Never ship a dead toggle, fake feature name, or hardcoded codec/status badge. Every visible control must either drive real behavior or be explicitly capability-gated.
7. **Deterministic Offline Testing:** Every plan includes `tests/<plan_id>_test.rs` running 100% offline against `tempfile::TempDir` and local `127.0.0.1:0` mocks.

---

## 6. Hydrogen Subagent Launch Template

Copy this prompt when launching any parallel subagent in Waves 1–3:

```text
You are executing Resonance plan `/specs/plans/<PLAN_FILE>.md` as a parallel Hydrogen subagent.
1. Read `AGENTS.md`, `/specs/plans/00_MASTER_PARALLEL_ORCHESTRATOR.md` (§3 & §5), and `/specs/plans/<PLAN_FILE>.md` in full before editing.
2. STRICTLY respect your Exclusive Write Scope. NEVER modify forbidden hub files (`Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/shell.rs`, `src/screens/settings.rs`, `src/lib.rs`, `crates/core/src/proxy/mod.rs`) unless explicitly listed in your plan's Exclusive Write Scope.
3. Implement every module, edge-case guard, and offline test specified in your plan with zero `unwrap()` on untrusted input and zero raw color literals in UI code.
4. Run `cargo check --workspace --all-targets` and your plan's `cargo test --test <test_name>` suite, plus `cargo fmt --all -- --check`, and report any stitch hooks required for the wave integration agent.
```
