# AGENTS.md — Resonance (`Raag`)

Pure Rust YouTube Music client (`GPL-3.0-or-later`) on **NSH `=0.7.0`** (`nsh.json`: `"root": "App"`, `"rust": "."`). Targets Desktop and Android API 26+ (`arm64-v8a`).

## Workspace & Nested Docs
- `crates/` ([crates/AGENTS.md](crates/AGENTS.md)) — `contract` (Phase 0 traits/state) + `core` (InnerTube `ClientMatrix`, `rquickjs` EJS solver, 3-tier cache, `LoopbackProxy`, `PlaybackController`, lyrics, SponsorBlock).
- `src/` ([src/AGENTS.md](src/AGENTS.md)) — Pure Rust NSH UI (`lib.rs`, `store.rs`, `live_engine.rs`, `ui/`, `screens/`), binaries (`main.rs`, `bin/client_probe.rs`, `bin/nsh_preview.rs`), plus `src/App.tsx`/`server.ts` web preview bridge.
- `platform/` ([platform/android/AGENTS.md](platform/android/AGENTS.md)) — `nsh_audio.rs` (`nsh/audio` driver), `desktop.rs`, `android.rs`, and Android Media3/Keystore/WebView Kotlin host.
- `tests/` ([tests/AGENTS.md](tests/AGENTS.md)) — Integration, headless UI, audio, proxy fault, and architecture gate suites.
- `.nsh/`, `target/`, `_refs/` — Generated/ignored. **Never** edit `.nsh/rust/`. `_refs/` is study-only.

## Hard Invariants (Enforced by `tests/`)
- **NSH pin:** Keep `nsh-core`, `nsh-base`, `nsh-engine`, `nsh-testing` at `=0.7.0`.
- **UI & Theme:** Compose `src/screens/` from `src/ui/` + `theme.json` (`nsh_base::theme()`). Zero raw color literals (`rgb(`, `rgba(`, `0xFF`) in `src/screens/` or `src/ui/controls.rs`; `>= 48dp` touch targets; `.pb_safe_area()`; no blur in virtual lists; honor `motion::duration_ms`. Never implement screens/state in `src/App.tsx`.
- **Threading:** Never block the NSH engine thread (`store.rs`, `platform/nsh_audio.rs`); run Tokio/network/SQLite in `live_engine.rs` over `crossbeam_channel`.
- **Security & Auth:** Never send browser cookies to `ANDROID_VR` or `IOS`. Keep Android `usesCleartextTraffic` disabled (`127.0.0.1` only) and `RECEIVER_NOT_EXPORTED`. Redact secrets/cookies/signed URLs in logs.
- **Licensing & JS:** `GPL-3.0-or-later`. Use `bun` / `bunx` (never `npm`/`npx`/`yarn`/`pnpm`).

## Core Commands & Skills
- Check: `cargo check --workspace` | `nsh check`
- Test: `cargo test` | `nsh test` (smoke: `cargo run --bin resonance -- --self-check`, `cargo run --bin resonance -- --fixture`)
- Build / Run: `nsh build` | `nsh run`
- Official NSH skills (`.agents/skills/`): `nsh`, `nsh-design-guides` (`https://nsh.dev/llms.txt`).
