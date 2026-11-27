# Master Hydrogen Parallel Execution Orchestrator (`specs/plans_v3`)

> **Project:** Resonance (`Raag`) — Pure Rust YouTube Music client (`GPL-3.0-or-later`, NSH `=0.7.0`)
> **Targets:** Desktop (macOS, Linux, Windows) & Android API 26+ (`arm64-v8a`)
> **Scope:** Complete production roadmap for **all remaining Resonance features**, **explicitly excluding ecosystem integration support** (no Last.fm, no ListenBrainz, no Discord Rich Presence, no third-party cloud sync).
> **Prerequisite Baseline Already Completed:**
> 1. Real `rquickjs` EJS player-script solver wired into `InnerTubeClient` + true incremental `LoopbackProxy` streaming (`ProxyBody` over bounded `mpsc` channel).
> 2. Real RFC 8628 TV OAuth device-code auth + Android WebView sign-in launch + elimination of dishonest UI mocks/toasts.
> 3. Interactive desktop top-edge seek slider, real track context menu in desktop player bar, atomic `play_queue`, and non-destructive `Clear Up Next`.
> 4. Authentic 4-tab YouTube Music mobile bottom navigation (`Home | Explore | Search | Library`), decluttered mobile track rows, `>= 48dp` touch targets, and fixed skeleton fractions.
> 5. Authentic YouTube Music Home (in-place mood chips, deduplicated shelves), Explore (live New Releases, live Charts, live Moods & Genres), and 3-tab `Up Next | Lyrics | Related` panels across Desktop Side Panel, Desktop Now Playing, and Mobile Now Playing.

---

## 1. Anti-Potemkin Execution Philosophy (Zero Dead Code)

Every previous audit of this repository revealed a single recurring failure mode: **beautiful isolated modules under `crates/core/src/*` that were never wired into `src/live_engine/` or `src/store/`** (for example, `pcm_engine`, `audio_dsp`, `local_media`, `rooms`, `casting`, and `offline` workers).

Every Hydrogen subagent executing a plan in `specs/plans_v3/` MUST obey the **Vertical Slice Law**:

1. **No Unwired Modules Allowed:** Every new or updated service in `crates/core/` MUST be constructed in `src/live_engine/boot.rs`, commanded from `src/live_engine/commands.rs`, projected via `LiveUpdate` into `src/store/`, and surfaced cleanly in `src/screens/` or `src/ui/controls/`.
2. **No Dishonest UI States:** Never display `"Connected"`, `"Up to date"`, `"Copied"`, or `"Active"` unless the real backend operation succeeded and returned a verified state object. If a platform capability is unsupported or offline, state so clearly.
3. **Hard `AGENTS.md` Invariants (Gate-Enforced):**
   - Keep `nsh-core`, `nsh-base`, `nsh-engine`, `nsh-testing` pinned at `=0.7.0`.
   - Zero raw color literals (`rgb(`, `rgba(`, `0xFF`) in `src/screens/` and `src/ui/controls/`.
   - Minimum `48dp` (`MOBILE_MIN_TOUCH_TARGET`) interactive touch targets on mobile.
   - Zero `.backdrop_blur(...)` inside `virtual_list` rows.
   - Never block the NSH engine thread (`src/store/*`, `platform/nsh_audio.rs`); all network, SQLite, decoding, and filesystem I/O run on Tokio / dedicated real-time audio threads bridged via `crossbeam_channel` or lock-free ring buffers.
   - Never send browser cookies (`Cookie` / `SAPISIDHASH`) to `ANDROID_VR` or `IOS` InnerTube clients.
   - Redact all cookies, OAuth tokens, signed stream URLs, and ephemeral LAN relay tokens from logs, crash reports, and UI status banners.
   - Preserve `GPL-3.0-or-later` headers/license compatibility; never introduce proprietary or GPL-incompatible crates.

---

## 2. Parallel Wave DAG & Zero-Conflict File Ownership Matrix

To run Hydrogen subagents **simultaneously without merge conflicts**, plans are grouped into **2 parallel execution waves** plus a **thin Final Stitch Pass**.

```
                   ┌──────────────────────────────────────────────┐
                   │   WAVE A (4 Parallel Hydrogen Subagents)     │
                   │  Disjoint crate & screen ownership — 0 merge │
                   ├──────────────────────────────────────────────┤
                   │  P1: Native PCM + Symphonia + CPAL + DSP     │
                   │  P2: Zero-Setup PO Token & Signed Config CI  │
                   │  P3: Offline Vault + Playlist CRUD & Sync    │
                   │  P4: Local Media Library & Audio Import      │
                   └──────────────────────┬───────────────────────┘
                                          │
                                   [Stitch Gate A]
                     (wire boot.rs / commands.rs / mod.rs enums)
                                          │
                   ┌──────────────────────▼───────────────────────┐
                   │   WAVE B (3 Parallel Hydrogen Subagents)     │
                   │  Disjoint crate & screen ownership — 0 merge │
                   ├──────────────────────────────────────────────┤
                   │  P5: Real Listen Together Rooms & Relay      │
                   │  P6: Android Auto & LAN Casting              │
                   │  P7: Native Auto-Updater, Packaging & Polish │
                   └──────────────────────┬───────────────────────┘
                                          │
                                   [Stitch Gate B]
                    (cargo test --workspace + headless UI gates)
```

### Disjoint File Ownership Table

| Plan | Exclusive Owned Paths (Write Allowed) | Shared Integration Files (ONLY edited in Stitch Pass, OR via additive trait/module hooks) |
| :--- | :--- | :--- |
| **P1** (`P1_NATIVE_PCM_SYMPHONIA_CPAL_DSP_ENGINE.md`) | `crates/core/src/pcm_engine/*`, `crates/core/src/audio_dsp/*`, `platform/pcm_backend.rs`, `platform/nsh_audio.rs`, `src/screens/settings_dsp_card.rs`, `tests/p1_pcm_dsp_test.rs` | `src/live_engine/boot.rs`, `src/live_engine/commands.rs`, `src/store/playback_actions.rs` |
| **P2** (`P2_ZERO_SETUP_POTOKEN_AND_SIGNED_REMOTE_CONFIG_INFRA.md`) | `crates/core/src/potoken/*`, `crates/core/src/innertube/remote_config.rs`, `crates/core/src/innertube/matrix.rs`, `src/bin/sign_config.rs`, `.github/workflows/probe.yml`, `config/client_matrix.json`, `platform/android/.../BotGuardAttestationBridge.kt`, `tests/p2_potoken_config_test.rs` | `src/live_engine/boot.rs`, `src/screens/settings.rs` |
| **P3** (`P3_OFFLINE_VAULT_PLAYLIST_CRUD_AND_SYNC.md`) | `crates/core/src/offline/*`, `crates/core/src/innertube/mutations.rs`, `crates/core/src/cache/db/offline.rs`, `src/store/mutation_slice.rs`, `src/store/library_slice.rs`, `src/screens/library.rs`, `src/ui/controls/add_to_playlist_sheet.rs`, `tests/p3_offline_mutations_test.rs` | `src/live_engine/commands.rs`, `src/store/mod.rs` |
| **P4** (`P4_LOCAL_MEDIA_LIBRARY_AND_AUDIO_IMPORT.md`) | `crates/core/src/local_media/*`, `crates/core/src/proxy/local_route.rs`, `src/store/local_media_slice.rs`, `src/screens/local_files.rs`, `platform/android/.../SafFolderPicker.kt`, `tests/p4_local_media_test.rs` | `src/live_engine/commands.rs`, `src/screens/library.rs` (tab hook) |
| **P5** (`P5_REAL_LISTEN_TOGETHER_ROOMS_AND_RELAY.md`) | `crates/core/src/rooms/*`, `crates/room-relay/*`, `src/store/rooms_slice.rs`, `src/screens/listen_together.rs`, `tests/p5_rooms_relay_test.rs` | `src/live_engine/commands.rs`, `src/live_engine/mod.rs` |
| **P6** (`P6_ANDROID_AUTO_AND_LAN_CASTING.md`) | `crates/core/src/casting/*`, `crates/core/src/proxy/lan_relay.rs`, `platform/android/src/main/kotlin/.../AndroidAutoBrowseTree.kt`, `platform/android/src/main/kotlin/.../PlaybackForegroundService.kt`, `platform/android/res/xml/automotive_app_desc.xml`, `src/ui/controls/cast_picker_sheet.rs`, `tests/p6_casting_auto_test.rs` | `src/live_engine/commands.rs`, `src/store/app_actions.rs` |
| **P7** (`P7_NATIVE_AUTO_UPDATER_PACKAGING_AND_POLISH.md`) | `crates/core/src/updater/*`, `src/store/updater_slice.rs`, `src/screens/settings_updater_card.rs`, `platform/desktop.rs`, `.github/workflows/release.yml`, `scripts/shrink-android-apk.py`, `tests/p7_updater_packaging_test.rs` | `src/live_engine/commands.rs`, `src/ui/controls/context_menu.rs` (clipboard hook) |

> **Rule for Parallel Subagents:** During Wave A (or Wave B), each subagent implements its entire core crate, platform bridge, store slice, UI screen, and integration tests inside its **Exclusive Owned Paths**, and defines a clean, self-contained `pub fn register_*` / `handle_*_command` module function (e.g., `src/live_engine/handlers/p1_audio.rs`, `src/live_engine/handlers/p3_offline.rs`) so the Stitch Pass is a 10-line enum dispatch with zero merge friction!

---

## 3. Shared Integration Contract for Stitch Passes

To prevent 7 subagents from colliding on `src/live_engine/mod.rs` and `src/live_engine/commands.rs`, create per-domain handler submodules under `src/live_engine/handlers/`:

- `src/live_engine/handlers/mod.rs` — re-exports domain handlers
- `src/live_engine/handlers/audio_dsp.rs` (Owned by **P1**)
- `src/live_engine/handlers/potoken_config.rs` (Owned by **P2**)
- `src/live_engine/handlers/offline_mutations.rs` (Owned by **P3**)
- `src/live_engine/handlers/local_media.rs` (Owned by **P4**)
- `src/live_engine/handlers/rooms.rs` (Owned by **P5**)
- `src/live_engine/handlers/casting.rs` (Owned by **P6**)
- `src/live_engine/handlers/updater.rs` (Owned by **P7**)

Each handler exposes:
- `pub enum <Domain>Command { ... }`
- `pub enum <Domain>Update { ... }`
- `pub async fn handle(ctx: &EngineContext, cmd: <Domain>Command)`

Then `EngineCommand` and `LiveUpdate` simply gain one nested variant per domain (e.g., `EngineCommand::Rooms(RoomsCommand)`), reducing Stitch Pass merges to ~15 lines total!

---

## 4. Verification & Acceptance Gates (Per Subagent + Master)

Every subagent MUST complete these checks before marking its task done:

1. **Compile & Clippy Gate:** `cargo check --workspace --all-targets` and `cargo clippy --workspace --all-targets -- -D warnings`.
2. **Unit + Integration Test Gate:** `cargo test --test p<N>_*` + `cargo test --workspace`.
3. **Architecture & UI Invariant Gate:**
   - Zero `rgb(`, `rgba(`, `0xFF` in `src/screens/` and `src/ui/controls/`.
   - Zero `.backdrop_blur` inside `virtual_list` closures.
   - Zero `unwrap()` / `expect()` in production runtime paths (`crates/core/src/`, `src/live_engine/`).
   - Zero secret/cookie/token leaks in `tracing` or `status_banner` formatting.
4. **Honesty Audit:** Grep modified files for `TODO`, `stub`, `mock`, `fake`, or optimistic success toasts disconnected from real backend outcomes.

---

## 5. Copy-Pasteable Hydrogen Subagent Dispatch Prompts

### Wave A Dispatch (Launch 4 Subagents in Parallel)

- **Subagent P1:** *"Read `AGENTS.md`, `specs/plans_v3/00_MASTER_HYDROGEN_ORCHESTRATOR.md`, and `specs/plans_v3/P1_NATIVE_PCM_SYMPHONIA_CPAL_DSP_ENGINE.md`. Implement P1 strictly within its Exclusive Owned Paths plus `src/live_engine/handlers/audio_dsp.rs`. Add full unit & audio-pipeline tests in `tests/p1_pcm_dsp_test.rs`. Run `cargo test` and verify zero clippy warnings."*
- **Subagent P2:** *"Read `AGENTS.md`, `specs/plans_v3/00_MASTER_HYDROGEN_ORCHESTRATOR.md`, and `specs/plans_v3/P2_ZERO_SETUP_POTOKEN_AND_SIGNED_REMOTE_CONFIG_INFRA.md`. Implement P2 strictly within its Exclusive Owned Paths plus `src/live_engine/handlers/potoken_config.rs`. Add tests in `tests/p2_potoken_config_test.rs`."*
- **Subagent P3:** *"Read `AGENTS.md`, `specs/plans_v3/00_MASTER_HYDROGEN_ORCHESTRATOR.md`, and `specs/plans_v3/P3_OFFLINE_VAULT_PLAYLIST_CRUD_AND_SYNC.md`. Implement P3 strictly within its Exclusive Owned Paths plus `src/live_engine/handlers/offline_mutations.rs`. Add tests in `tests/p3_offline_mutations_test.rs`."*
- **Subagent P4:** *"Read `AGENTS.md`, `specs/plans_v3/00_MASTER_HYDROGEN_ORCHESTRATOR.md`, and `specs/plans_v3/P4_LOCAL_MEDIA_LIBRARY_AND_AUDIO_IMPORT.md`. Implement P4 strictly within its Exclusive Owned Paths plus `src/live_engine/handlers/local_media.rs`. Add tests in `tests/p4_local_media_test.rs`."*

### Wave B Dispatch (Launch 3 Subagents in Parallel after Stitch Gate A)

- **Subagent P5:** *"Read `AGENTS.md`, `specs/plans_v3/00_MASTER_HYDROGEN_ORCHESTRATOR.md`, and `specs/plans_v3/P5_REAL_LISTEN_TOGETHER_ROOMS_AND_RELAY.md`. Implement P5 strictly within its Exclusive Owned Paths plus `src/live_engine/handlers/rooms.rs`. Add WebSocket + long-poll + drift tests in `tests/p5_rooms_relay_test.rs`."*
- **Subagent P6:** *"Read `AGENTS.md`, `specs/plans_v3/00_MASTER_HYDROGEN_ORCHESTRATOR.md`, and `specs/plans_v3/P6_ANDROID_AUTO_AND_LAN_CASTING.md`. Implement P6 strictly within its Exclusive Owned Paths plus `src/live_engine/handlers/casting.rs`. Add discovery + CastV2 + DLNA + LAN relay security tests in `tests/p6_casting_auto_test.rs`."*
- **Subagent P7:** *"Read `AGENTS.md`, `specs/plans_v3/00_MASTER_HYDROGEN_ORCHESTRATOR.md`, and `specs/plans_v3/P7_NATIVE_AUTO_UPDATER_PACKAGING_AND_POLISH.md`. Implement P7 strictly within its Exclusive Owned Paths plus `src/live_engine/handlers/updater.rs`. Add signature verification + atomic install + clipboard tests in `tests/p7_updater_packaging_test.rs`."*
