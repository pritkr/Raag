# AGENTS.md — `src/` (Pure Rust NSH UI & Runtime Bridge)

Load official `.agents/skills/nsh` and `.agents/skills/nsh-design-guides` when designing or editing UI/reactivity. Pin stays `=0.7.0` (check local Cargo crate sources if upstream docs differ).

- **Entry & Capabilities:** `nsh.json` (`"rust": "."`, `"root": "App"`, `"theme": "theme.json"`). Add any new external domain to `nsh.json` `capabilities.network.hosts`. `src/lib.rs` exports `#[native_module("resonance/native")]` and `#[component] pub fn App()`.
- **State & Threading:** `src/store/` (independent domain-scoped `global::<Slice>()` signals) runs on the NSH engine thread and must stay non-blocking. `src/live_engine/` (`AppEngine` + `EngineCommand`) owns Tokio, `PlaybackController`, `LoopbackProxy`, and SQLite off-thread, sending `LiveUpdate` via `crossbeam_channel`.
- **`nsh/audio` Driver (`platform/nsh_audio.rs`):** Keep the `AudioEvent` `spawn` pump scoped to `App()` (outside `effect`) with a monotonic generation guard so `effect` re-runs don't drop events.
- **Design System (`src/ui/` & `src/screens/`):** Compose screens strictly from `src/ui/` (`tokens.rs`, `controls.rs`, `icons.rs`) + `nsh_base::theme()`. Keep `MOBILE_MIN_TOUCH_TARGET = 48.0`, `.pb_safe_area()`, `motion::duration_ms`, zero raw color literals (`rgb(`, `rgba(`, `0xFF`), and zero blur in virtualized lists.
- **Web Preview (`src/App.tsx`, `server.ts`):** Proxies live Rust `nsh_preview` (`127.0.0.1:3001`). Run `cargo build --bin nsh_preview && bun run dev`. Never duplicate app screens/state in TSX.
