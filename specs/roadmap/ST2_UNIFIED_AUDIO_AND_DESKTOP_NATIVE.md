# ST-2 — Unified Single-Source-of-Truth Audio Driver, Native Desktop Features & Truthful DSP Gating

> **Wave:** Wave 1 (Short-Term P0) — Parallelizable with ST-1, ST-3, ST-4
> **Parallelism Boundary:**
> - **Exclusive Write Scope:** `platform/nsh_audio.rs`, `platform/desktop.rs`, `platform/mod.rs` (Desktop factory helpers), `crates/core/src/playback/controller.rs`, `src/store/playback_slice.rs`, `src/store/settings_slice.rs`, `src/screens/settings_dsp_card.rs` (extracted/isolated DSP capability card for Wave 1.5 composition), `.github/workflows/release.yml` (Desktop feature flags only), `tests/st2_unified_audio_and_desktop_native_test.rs` (new).
> - **Forbidden Edits:** `platform/android.rs`, `platform/android/**`, `crates/core/src/innertube/**`, `crates/core/src/solver/**`, `crates/core/src/potoken/**`, `src/live_engine.rs`, `src/store/mod.rs`.

---

## 1. Problem Statement & Root-Cause Analysis

Inspecting `platform/nsh_audio.rs`, `platform/desktop.rs`, `crates/core/src/playback/controller.rs`, and `src/live_engine.rs` reveals three high-severity correctness & UX defects:

1. **Split-Brain Playback Loop:**
   - `PlaybackController` (on the Tokio runtime) calls `AudioBackend` (`NshAudioBackend`), which sends `NshAudioControl` events (`LoadAndPlay`, `Play`, `Pause`, `SeekMs`, `SetVolume`, `Stop`) into an `UnboundedSender<NshAudioControl>` — **but nothing ever calls `take_control_receiver()`!**
   - Meanwhile, `mount_nsh_audio_driver()` (on the NSH engine thread) independently observes reactive store signals (`store::audio_driver_snapshot()`) and spawns `nsh_core::prelude::audio_player(&target_url, desired_settings)`.
   - **Consequences:**
     - System media keys (`souvlaki` MPRIS/SMTC) talk to `PlaybackController`, which updates `NshAudioBackend`, but `seek_generation` / `desired_playing` can drift or echo-loop against `mount_nsh_audio_driver`.
     - `PlaybackController::tick_position()` (which drives **50% next-track pre-resolution** and **SponsorBlock auto-skip**!) is **never fed live `AudioEvent::Progress` ticks** from `nsh/audio`!
     - `AudioEvent::Ended` advances local `store` state instead of driving `PlaybackController::on_track_ended()` (breaking autoplay radio fetch & persisted queue resume!).
2. **Desktop Native Features Disabled in Releases & Duplicated `souvlaki` Code:**
   - `Cargo.toml` has `default = []`, and `.github/workflows/release.yml` builds without `--features desktop-native`. Users downloading macOS `.dmg`, Windows `.exe`, or Linux `.deb` get **neither OS Keychain (`desktop-keyring`) nor Media Keys/Now Playing (`desktop-mpris`)**!
   - `platform/nsh_audio.rs:375` has a stripped-down duplicate `souvlaki` initializer with `std::mem::forget(controls)` that **never calls `set_metadata` or `set_playback`** and passes `hwnd: None` (breaking Windows SMTC), whereas `platform/desktop.rs` already implemented the proper owned `DesktopAudioBridge` with full metadata/playback projection and Windows HWND guard!
3. **Dead DSP Controls in Settings:**
   - `src/screens/settings.rs` renders switches for ReplayGain/EBU R128, Skip Silence, 10-band EQ, and Crossfade, and `src/store/mod.rs` shows toast banners claiming they were enabled—yet `nsh/audio` streams HTTP URLs directly to the OS decoder and **never invokes `DspPipeline`**. Shipping placebo toggles destroys user trust.

---

## 2. Target Unified Architecture

```text
                ┌───────────────────────────────────────────────────────┐
                │          NSH UI Thread (Non-Blocking Reactive)        │
                │                                                       │
                │  User Press / Shortcut ──► store::dispatch_*() ──────┐│
                │                                                      ││
                │  mount_nsh_audio_driver()                            ││
                │   ├─ Consumes NshAudioControl (crossbeam non-block)  ││
                │   ├─ Drives nsh_core::prelude::audio_player(...)     ││
                │   └─ Emits NshAudioTelemetry (Progress/State/Ended) ─┤│
                └──────────────────────────────▲───────────────────────┼┘
                                               │                       │
                    NshAudioControl (Command)  │   Telemetry + User    │
                    + LiveUpdate::Playback     │   Intents (Channel)   ▼
                ┌──────────────────────────────┴───────────────────────┐
                │         Background Tokio Runtime (`resonance-core`)  │
                │                                                      │
                │  PlaybackController (SINGLE SOURCE OF TRUTH)         │
                │   ├─ Queue, Shuffle, Repeat, Autoplay Radio          │
                │   ├─ 50% Next-Track Prefetch (triggered by Progress) │
                │   ├─ SponsorBlock Auto-Skip (triggered by Progress)  │
                │   ├─ Watchtime / ATR session tracker (ST-3 hook)     │
                │   └─ Drives CompositeDesktopAudioBackend:            │
                │        1. NshAudioControl channel → nsh/audio player │
                │        2. DesktopMediaControls (souvlaki MPRIS/SMTC) │
                └──────────────────────────────────────────────────────┘
```

### Core Design Principle: Strict Unidirectional Control Flow
- **In Live Mode (`!is_fixture_mode()`):**
  - UI buttons, keyboard shortcuts, and `souvlaki` OS media keys ALL send commands to `PlaybackController` (Play, Pause, Seek, Next, Prev, Volume, Mute, Repeat, Shuffle).
  - `PlaybackController` is the **sole writer** of authoritative `PlayerSnapshot` and sends exact `NshAudioControl` commands to `mount_nsh_audio_driver()`.
  - `mount_nsh_audio_driver()` is a **pure executor + telemetry reporter**: it executes `NshAudioControl` on `nsh/audio` and forwards throttled `NshAudioTelemetry` (`Progress` every 250 ms, `Duration`, `Playing`, `Paused`, `Buffering`, `Ended`, `Failed`) back to `PlaybackController` via a lock-free `crossbeam_channel`.
  - `PlaybackController::on_telemetry()` updates position, triggers 50% prefetch & SponsorBlock skip (which issues a `SeekMs` back down `NshAudioControl`!), updates `souvlaki` progress, and emits `LiveUpdate::PlaybackUpdated(snapshot)` to the UI store.
- **In Fixture Mode (`is_fixture_mode()`):**
  - Keeps the deterministic synchronous local store transitions untouched so all existing `tests/headless_ui.rs`, `tests/phase4_hardening_and_qa.rs`, and `tests/phase5_polish_and_ergonomics.rs` pass identically.

---

## 3. Detailed File-by-File Implementation Plan

### 3.1 Eliminate Duplicate `souvlaki` & Compose `DesktopMediaControls` (`platform/desktop.rs` & `platform/nsh_audio.rs`)
- **In `platform/desktop.rs`:**
  - Extract `pub struct SystemMediaControlsBridge` (wrapping owned `Mutex<Option<souvlaki::MediaControls>>`, `position_ms`, `duration_ms`, `cmd_tx`, `cmd_rx`) with:
    - `new(hwnd: Option<*mut c_void>) -> Self` (no `std::mem::forget` leak! Properly drops on shutdown).
    - `sync_metadata(&self, meta: &NowPlayingMetadata)`
    - `sync_playback(&self, playing: bool, position_ms: u64, has_track: bool)`
    - `take_command_receiver(&self) -> Option<UnboundedReceiver<MediaCommand>>`
    - `inject_command(&self, cmd: MediaCommand)` (for headless testing).
  - **Windows SMTC HWND Recovery:** Add `pub fn attach_window_hwnd(&self, hwnd: *mut c_void)` so once the NSH Windows surface creates its native `HWND` (or via `raw-window-handle` / platform hook), SMTC attaches lazily if `hwnd` was `None` at early runtime boot.
  - **Linux MPRIS Artwork Fix:** Pass local cached artwork `file://` URI if available in `ArtworkCache`, falling back to HTTPS `artwork_url` (improves GNOME/KDE lockscreen reliability offline).
- **In `platform/nsh_audio.rs`:**
  - Remove the duplicate `souvlaki` block (`init_system_media_controls` lines 370–402).
  - Embed `media_controls: SystemMediaControlsBridge` inside `NshAudioBackend` on `#[cfg(not(target_os = "android"))]`.
  - Add a global static channel pair (`NSH_AUDIO_BUS: OnceLock<NshAudioBus>`) using `crossbeam_channel::unbounded()` connecting `NshAudioBackend` (Tokio side) ↔ `mount_nsh_audio_driver()` (NSH engine thread):
    - `control_tx` / `control_rx`: `NshAudioControl` (`LoadAndPlay`, `Play`, `Pause`, `Stop`, `SeekMs(u64)`, `SetVolume(f32)`, `SetLooped(bool)`).
    - `telemetry_tx` / `telemetry_rx`: `NshAudioTelemetry` (`Loaded { duration_ms }`, `StateChanged(PlayerState)`, `Buffering(bool)`, `Progress { position_ms, duration_ms }`, `Ended`, `Failed(String)`).
  - In `mount_nsh_audio_driver()`:
    - On live mode, drain `control_rx` every 32 ms (or woken immediately via an async `Notify` / polling task) and drive `nsh_core::prelude::audio_player` deterministically—eliminating seek-echo bugs forever because `NshAudioControl::SeekMs` is only sent on explicit user/SponsorBlock seeks, never echoed by `Progress`!
    - Forward `AudioEvent::Progress` (throttled to `>= 250 ms` delta to prevent channel flood), `AudioEvent::Ended`, `AudioEvent::Playing`, `AudioEvent::Paused`, `AudioEvent::Buffer`, `AudioEvent::Load`, and `AudioEvent::Failed` into `telemetry_tx`.
    - In `cleanup(...)`: send `NshAudioTelemetry::StateChanged(PlayerState::Stopped)` and drop `ActiveNativePlayer` cleanly.

### 3.2 `PlaybackController` Telemetry & Media-Control Integration (`crates/core/src/playback/controller.rs`)
- Add `pub async fn handle_audio_telemetry(&self, telemetry: AudioTelemetry) -> CoreResult<()>` on `PlaybackController`:
  - `AudioTelemetry::Loaded { duration_ms }` → updates active track duration if previously `0` (common on some streams).
  - `AudioTelemetry::Progress { position_ms, .. }` → delegates directly to existing `self.tick_position(position_ms).await`! This automatically activates:
    1. **SponsorBlock auto-skip** (jumps past `music_offtopic` segments and emits `AppEvent::SponsorBlockSkipped`).
    2. **50% Next-Track Pre-Resolution** (`prefetch.rs` resolves the next track URL & registers it in `LoopbackProxy` before track end).
    3. **Periodic queue resume persistence** (throttled every 5s so app crashes resume near the exact second).
    4. **OS Media Controls progress sync** (`audio.update_now_playing(...)` throttled every 1s).
  - `AudioTelemetry::Ended` → calls `self.on_track_ended().await` (respects `RepeatMode::One`, `RepeatMode::All`, advances queue, or fetches `autoplay_next` radio tracks when at end of queue!).
  - `AudioTelemetry::Failed(msg)` → transitions FSM to `PlayerState::Error(redact_secrets(&msg))`, publishes `AppEvent::Toast { is_error: true, .. }`, and emits updated `PlayerSnapshot`.
- Add `PlaybackController::set_repeat_mode`, `set_shuffle`, `set_volume`, `set_muted`, `seek_ms`, `toggle_play_pause` live command hooks so `store` in Wave 1.5 can route every transport control through the controller cleanly.

### 3.3 Truthful Audio DSP Capability Gating (`src/store/settings_slice.rs` & `src/screens/settings_dsp_card.rs`)
- Define `pub enum AudioDspCapability { ActivePcmPipeline, ExternalUrlStreamOnly }`:
  - Currently returns `AudioDspCapability::ExternalUrlStreamOnly` when using `nsh/audio` URL streaming (flips to `ActivePcmPipeline` automatically when the long-term custom Rust PCM sink is mounted).
- In `settings_dsp_card.rs`:
  - When `ExternalUrlStreamOnly`: render a clean, honest status badge (`"Planned • Native PCM Engine"`) and disable the EQ / ReplayGain / Skip Silence / Crossfade toggles with an informative subtitle explaining that `nsh/audio` uses direct OS decoding today and full PCM DSP is coming in the custom audio engine milestone.
  - Keep `DspConfig` state & unit tests intact so nothing breaks when the PCM sink ships.

### 3.4 Release CI & Default Desktop Feature Enablement (`.github/workflows/release.yml` & `Cargo.toml`)
- **In Wave 0 `Cargo.toml`:** Make `desktop-native` (`["desktop-mpris", "desktop-keyring"]`) work cleanly on macOS, Linux, and Windows, and either enable via `CARGO_FEATURES` in `release.yml` or add a target-gated default profile (while keeping `cargo test` on headless Linux CI working seamlessly thanks to `DesktopAuthStore`'s automatic `0o600` fallback and `souvlaki`'s graceful unavailable-D-Bus handling!).
- **In `.github/workflows/release.yml`:**
  - Verify Linux runner installs `libdbus-1-dev` (required for `souvlaki` MPRIS + `keyring` Secret Service on Linux!).
  - Pass `--features desktop-native` in fallback `cargo build --release` jobs and verify NSH packaging links the default desktop features cleanly.
  - Add a CI check step in `.github/workflows/ci.yml`: `cargo check --workspace --features desktop-native`.

---

## 4. Concurrency, Lifecycle & Edge Cases

1. **Seek Echo Prevention:** `AudioEvent::Progress` updates `PlaybackController` position and UI slider value, but MUST NOT generate an outbound `NshAudioControl::SeekMs`. Only user scrub, `-5s/+5s`, lyric-tap, OS media key seek, and SponsorBlock auto-skip emit `NshAudioControl::SeekMs`.
2. **Track Switch Race (`Ended` from Stale Generation):** Every `LoadAndPlay` carries a monotonic `generation: u64` inside `NshAudioControl` and `NshAudioTelemetry`. `PlaybackController` ignores any `Ended` or `Progress` telemetry whose `generation` does not match the current active track generation—preventing double-skipping tracks when a user hits "Next" right as a track finishes!
3. **SponsorBlock Re-Trigger Loop:** Keep a per-track `skipped_segments: HashSet<usize>` inside `PlaybackController` so skipping from `10.0s -> 25.0s` cannot re-trigger if `nsh/audio` emits one last stale `Progress(10_200)` before the seek settles.
4. **Headless Linux Without D-Bus / Wayland:** `SystemMediaControlsBridge` logs a single `debug`/`warn` and stays a no-op if D-Bus is absent (never panics, never blocks startup).
5. **Windows SMTC Without HWND:** Guards `hwnd.is_none()` gracefully until `attach_window_hwnd` is called.

---

## 5. Offline Verification Test Suite (`tests/st2_unified_audio_and_desktop_native_test.rs`)

1. `unified_audio_command_and_telemetry_roundtrip_drives_prefetch_and_sponsorblock` — feeds synthetic `AudioTelemetry::Progress` ticks through `PlaybackController::handle_audio_telemetry`; asserts (a) SponsorBlock segment triggers outbound `NshAudioControl::SeekMs` exactly once (no re-trigger loop), (b) crossing 50% triggers `LoopbackProxy` prefetch for track #2, and (c) `AudioTelemetry::Ended` advances queue to track #2 and emits new `LoadAndPlay` with incremented `generation`.
2. `stale_generation_ended_event_does_not_double_skip` — switches track (gen 1 → gen 2), then injects stale `AudioTelemetry::Ended { generation: 1 }`; asserts active track remains gen 2.
3. `system_media_controls_updates_metadata_and_handles_media_commands` — verifies `NshAudioBackend` updates `SystemMediaControlsBridge` metadata/playback on `load_and_play`/`pause`/`seek_ms`/`stop` and dispatches injected `Next`/`TogglePlayPause`/`SeekMs` commands into `PlaybackController`.
4. `dsp_capability_gate_reports_external_url_stream_honestly` — verifies `settings_dsp_card` renders truthful capability state in `nsh_testing::Headless` without raw color literals or dead-toggle claims.
5. `desktop_native_feature_compiles_and_falls_back_cleanly_headless` — compiled with `--features desktop-native`, verifies `DesktopAuthStore` + `NshAudioBackend` initialize without panicking on headless Linux.

```bash
cargo test --test st2_unified_audio_and_desktop_native_test --features desktop-native -- --nocapture
cargo test --test nsh_native_audio --test desktop_auth_media_test
```
