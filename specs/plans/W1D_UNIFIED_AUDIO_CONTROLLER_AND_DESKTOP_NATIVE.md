# W1D — Unified Single-Source-of-Truth Audio Controller, Default Desktop Native & Truthful DSP Gate

> **Wave:** Wave 1 (Parallel Agent 1D — Runs concurrently with W1A, W1B, W1C, W1E)
> **Exclusive Write Scope:**
> - `platform/nsh_audio.rs`, `platform/desktop.rs`, `platform/mod.rs`
> - `crates/core/src/playback/{controller.rs,fsm.rs,prefetch.rs,queue.rs,mod.rs}`
> - `src/store/{playback_slice.rs,settings_slice.rs}`
> - `src/screens/settings_dsp_card.rs` (new isolated DSP capability card stitched into `settings.rs` in Wave 1.5)
> - `.github/workflows/ci.yml`, `.github/workflows/release.yml` (Desktop `desktop-native` feature & `libdbus-1-dev` wiring only)
> - `tests/w1d_unified_audio_and_desktop_native_test.rs` (new)
> **Read-Only Dependencies (From Wave 0):** `crates/contract/src/audio_ext.rs` (`AudioDspCapability`, `AudioControlCommand`, `AudioTelemetryEvent`, `CodecBadgeInfo`, `SleepTimerState`, `PlaybackSpeedConfig`).
> **Stitch Hooks (For Wave 1.5 Integration Agent):**
> - Route UI transport dispatches in `src/store/mod.rs` through `live_engine` → `PlaybackController` when `!is_fixture_mode()`
> - Mount `settings_dsp_card::dsp_settings_section()` inside `src/screens/settings.rs`
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{shell,settings,home,library,now_playing,queue,lyrics,explore,search}.rs`, `platform/android.rs`, `platform/android/**`.

---

## 1. Ground-Truth Defects Eliminated

1. **Split-Brain Playback Engine:**
   - `PlaybackController` (Tokio) emits `NshAudioControl` commands into `NshAudioBackend`'s channel (`take_control_receiver()`), which **nothing reads**.
   - Simultaneously, `mount_nsh_audio_driver()` (UI thread) drives `nsh_core::prelude::audio_player` directly from `store::audio_driver_snapshot()`.
   - **Broken capabilities caused by split-brain:** SponsorBlock auto-skip never runs, 50% next-track pre-resolution and 75% `LoopbackProxy::warm_next_track()` never trigger, `on_track_ended()` autoplay radio and Repeat modes desync, and SQLite queue resume positions aren't persisted cleanly.
2. **Desktop Native Features Compiled Out by Default:**
   - `desktop-keyring` (`keyring`) and `desktop-mpris` (`souvlaki`) are behind opt-in features absent from default desktop builds and `.github/workflows/release.yml`.
   - `platform/nsh_audio.rs` contains a leaking duplicate `souvlaki` initializer (`std::mem::forget(controls)`) passing `hwnd: None` and never syncing metadata/playback state, duplicating `platform/desktop.rs`.
3. **Placebo DSP Toggles:**
   - Settings displays active switches for 10-band EQ, ReplayGain, Skip Silence, and Crossfade even though URL-based `nsh/audio` bypasses `DspPipeline`. Until Wave 2A mounts the custom Rust PCM audio sink, UI must truthfully communicate DSP capability status.

---

## 2. Target Unidirectional Audio Control Architecture

```text
┌────────────────────────────────────────────────────────────────────────────┐
│  NSH UI Thread (Strict Non-Blocking Reactive Layer)                        │
│                                                                            │
│  User Tap / Key Shortcut ──► store::dispatch_transport(cmd) ─────────────┐ │
│                                                                          │ │
│  mount_nsh_audio_driver() [Pure Executor + Telemetry Producer]           │ │
│   ├─ Non-blocking drain of `NSH_AUDIO_BUS.control_rx` (16–32 ms tick)    │ │
│   ├─ Executes `LoadAndPlay(gen, url)`, `Play`, `Pause`, `SeekMs`, `Vol`  │ │
│   └─ Publishes `AudioTelemetryEvent` (gen, Progress/Ended/State/Failed) ─┤ │
└─────────────────────────────────▲────────────────────────────────────────┼─┘
                                  │                                        │
        AudioControlCommand       │       AudioTelemetryEvent +            │
        + LiveUpdate::Playback    │       User Transport Intents           ▼
┌─────────────────────────────────┴──────────────────────────────────────────┐
│  Background Tokio Runtime (`resonance-core`)                               │
│                                                                            │
│  PlaybackController  ◄═══ SINGLE SOURCE OF TRUTH ═══►  SqliteCacheStore    │
│   ├─ Monotonic `active_generation: u64` fencing (zero double-skips!)       │
│   ├─ Real `ShuffleDeck` (W0) + `RepeatMode` + `autoplay_next` radio        │
│   ├─ `SleepTimer` countdown & 3s smooth fade-out driver                    │
│   ├─ SponsorBlock `skipped_segments: HashSet<usize>` (zero re-trigger)     │
│   ├─ 50% progress → Next-Track `resolve_stream` + `register_next_stream`   │
│   ├─ 75% progress → `LoopbackProxy::warm_next_track(4 chunks = 1 MB)`      │
│   ├─ 5s throttled SQLite queue resume persistence                          │
│   └─ Drives `CompositeAudioBackend`:                                       │
│        1. Active Sink (`NshAudioBackend` in Wave 1 → `PcmAudioBackend` W2A)│
│        2. `SystemMediaControlsBridge` (`souvlaki` MPRIS / SMTC / NowPlay)  │
└────────────────────────────────────────────────────────────────────────────┘
```

**Fixture Mode Compatibility Rule:** When `is_fixture_mode()` is true, synchronous store helpers remain deterministic so `tests/headless_ui.rs`, `tests/phase4_hardening_and_qa.rs`, and `tests/phase5_polish_and_ergonomics.rs` stay 100% green without spawning real OS audio devices.

---

## 3. Detailed Implementation Specification

### 3.1 Single Owned `SystemMediaControlsBridge` (`platform/desktop.rs`)
- Delete the duplicate `init_system_media_controls` + `std::mem::forget` leak from `platform/nsh_audio.rs`.
- Build `pub struct SystemMediaControlsBridge` in `platform/desktop.rs`:
  - Owns `Mutex<Option<souvlaki::MediaControls>>` cleanly (proper `Drop` on shutdown).
  - **Windows SMTC `HWND` Lazy Attachment:** Exposes `pub fn attach_window_hwnd(&self, hwnd: *mut std::ffi::c_void)` so early runtime boot without a window handle degrades cleanly and attaches SMTC as soon as the NSH desktop surface exposes its native `HWND`.
  - **Linux MPRIS Offline Artwork:** Resolves local cached artwork `file://` path from `ArtworkCache` first, falling back to HTTPS `artwork_url` so GNOME/KDE lockscreens still display album art offline.
  - **Headless Safety:** If D-Bus / MPRIS / macOS NowPlaying initialization fails (e.g., headless Linux CI container), log once at `debug`/`warn` and operate as an in-memory no-op while preserving `inject_command(MediaCommand)` for tests.
  - Projects `Play`, `Pause`, `Toggle`, `Next`, `Previous`, `SeekBy`, `SetPosition`, `SetVolume`, `Stop` events directly into `PlaybackController`'s command channel.

### 3.2 Lock-Free Bus & Generation-Fenced Driver (`platform/nsh_audio.rs`)
- Static `NSH_AUDIO_BUS: OnceLock<NshAudioBus>` containing two `crossbeam_channel::unbounded()` pipes:
  - `control_tx` / `control_rx`: `AudioControlCommand` (`LoadAndPlay { generation, proxy_url, start_position_ms, autoplay, playback_speed }`, `Play`, `Pause`, `Stop`, `SeekMs { generation, position_ms }`, `SetVolume(f32)`, `SetPlaybackSpeed(f32)`).
  - `telemetry_tx` / `telemetry_rx`: `AudioTelemetryEvent` (`Loaded { generation, duration_ms }`, `Playing { generation }`, `Paused { generation }`, `Buffering { generation, active }`, `Progress { generation, position_ms, duration_ms }`, `Ended { generation }`, `Failed { generation, message }`).
- **Seek-Echo Prevention Invariant:** `AudioEvent::Progress` updates UI slider & `PlaybackController` state, and **NEVER** writes back an `AudioControlCommand::SeekMs`. Only explicit user scrub, `±5s/±15s`, lyric tap, OS media key seek, or SponsorBlock auto-skip emit `SeekMs`.
- **Progress Throttling:** Forward `AudioEvent::Progress` to `telemetry_tx` only when `|pos - last_pos| >= 250 ms` or state changes, preventing channel saturation.
- **Clean Teardown:** `cleanup()` drops `ActiveNativePlayer` and emits `Paused`/`Stopped` without blocking.

### 3.3 `PlaybackController` Unified State Machine (`crates/core/src/playback/controller.rs`)
- **Generation Fencing:**
  - Every track load calls `let gen = self.proxy.advance_generation();` and stores `active_generation = gen`.
  - `handle_audio_telemetry(event)` **discards** any event whose `event.generation != active_generation`—eliminating double-skips when a user hits `Next` milliseconds before the previous track emits `Ended`.
- **Full Progress Hook (`tick_position(position_ms)`):**
  1. **SponsorBlock:** Checks active track segments; skips any segment whose index is NOT in `self.skipped_segments` (`HashSet<usize>` cleared on track change), inserts index before issuing `SeekMs`, and emits `AppEvent::SponsorBlockSkipped`.
  2. **50% Pre-Resolution:** At `position_ms * 2 >= duration_ms`, resolves the next queue track's `StreamInfo` in background (`resolve_stream`) and registers it in `LoopbackProxy::register_next_stream`.
  3. **75% Pre-Warming:** At `position_ms * 4 >= duration_ms * 3`, invokes `LoopbackProxy::warm_next_track(next_stream, 4)` (1 MB = ~60s of Opus audio warmed in `ChunkCache` for instant zero-stall track transition!).
  4. **Crash-Safe Resume Persistence:** Every 5 seconds of playback delta, persists `QueueState { resume_position_ms, .. }` to SQLite.
  5. **Codec Badge Projection:** Emits real `CodecBadgeInfo` derived from active `StreamInfo` (`codec`, `bitrate_kbps`, `itag`, `resolved_by_client`, `is_offline`, `is_local`) to `PlaybackSlice` so Now Playing displays truthful metadata.
  6. **Sleep Timer Tick:** Evaluates active `SleepTimerState`; smoothly attenuates volume over the last 3,000 ms, pauses at 0, restores saved volume.
- **Full Track End Handling (`on_track_ended(generation)`):**
  - Checks `SleepTimerState::EndOfCurrentTrack` first → pauses cleanly and clears timer.
  - Honors `RepeatMode::One` → seeks to 0 & plays.
  - Advances via `ShuffleDeck` / natural index; at end of queue with `RepeatMode::All` → wraps (re-shuffling if shuffled); with `autoplay_radio == true` → fetches `provider.autoplay_next(&last_video_id)` and continues seamlessly.

### 3.4 Truthful DSP Capability Card (`src/screens/settings_dsp_card.rs`)
- Reads `store::dsp_capability() -> AudioDspCapability`:
  - `AudioDspCapability::ExternalUrlStreamOnly` (Wave 1 default with `nsh/audio`): Displays an honest `"Native PCM Engine Required"` badge; renders DSP controls in disabled/preview state explaining clearly that direct system playback is active and 10-band EQ / ReplayGain / Silence Skip / Crossfade activate automatically once the native PCM audio engine (Wave 2A) is enabled.
  - `AudioDspCapability::NativePcmPipeline` (flipped automatically by Wave 2A!): Unlocks full interactive 10-band EQ sliders, presets, ReplayGain, Skip Silence, Crossfade slider (`0s – 12s`), and Live Audio Engine status!

### 3.5 Desktop Native Default & CI/Release Enablement
- On `#[cfg(not(target_os = "android"))]`, ensure `desktop-native` (`desktop-keyring`, `desktop-mpris`) is built in CI (`cargo check --workspace --features desktop-native`) and enabled in `.github/workflows/release.yml` for macOS, Windows, and Linux (installing `libdbus-1-dev` on Ubuntu runners).
- Keep `DesktopAuthStore` automatic `0o600` file fallback whenever OS Keyring is unavailable (headless SSH/Docker/Wayland without Secret Service).

---

## 4. Deterministic Offline Verification (`tests/w1d_unified_audio_and_desktop_native_test.rs`)

1. `generation_fence_blocks_stale_ended_and_progress_events` — switches track (gen 1 → gen 2), injects stale `Ended { generation: 1 }`, asserts active track stays gen 2 with zero double-skip.
2. `progress_telemetry_triggers_sponsorblock_once_then_50pct_resolve_and_75pct_warm` — drives `PlaybackController::handle_audio_telemetry` across 0% → 100%; verifies SponsorBlock skips once (no re-trigger loop on stale tick), 50% resolves next track, 75% warms chunks in `LoopbackProxy`, and `Ended` advances cleanly.
3. `seek_echo_never_occurs_from_progress_ticks` — feeds 20 `Progress` ticks and asserts zero outbound `AudioControlCommand::SeekMs` are emitted.
4. `system_media_controls_projects_metadata_and_handles_injected_commands` — tests `SystemMediaControlsBridge` metadata/playback sync and `Next`/`TogglePlayPause`/`SeekMs` roundtrip.
5. `dsp_capability_card_renders_honest_gated_vs_active_states` — headless UI test verifying both `ExternalUrlStreamOnly` (honest disabled state) and `NativePcmPipeline` (interactive controls) with zero raw color literals.

```bash
cargo test --test w1d_unified_audio_and_desktop_native_test --features desktop-native -- --nocapture
cargo test --test nsh_native_audio --test desktop_auth_media_test
```
