# Contributing to Resonance

Thank you for your interest in contributing to Resonance!

## Development Guidelines

1. **Pure Rust NSH UI:**
   - Keep `nsh-core`, `nsh-base`, `nsh-engine`, and `nsh-testing` pinned to `=0.6.1`.
   - Compose screens in `src/screens/` exclusively from `src/ui/` and semantic tokens in `theme.json` (`nsh_base::theme()`). Do not introduce raw color literals (`rgb(`, `rgba(`, `0xFF`) in `src/screens/` or `src/ui/controls.rs`.
   - Keep interactive mobile targets `>= 48dp`, use `.pb_safe_area()`, avoid backdrop blur in virtualized lists, and honor `motion::duration_ms`.
   - Never implement app screens or state in `src/App.tsx` (it is only a preview bridge to `nsh_preview`).

2. **Threading & Performance:**
   - Never block the NSH engine thread (`src/store.rs`, `platform/nsh_audio.rs`).
   - Run async network, SQLite, and playback controller work on the dedicated Tokio runtime in `src/live_engine.rs` over `crossbeam_channel`.

3. **Security & Auth Isolation:**
   - Never send browser cookies to `ANDROID_VR` or `IOS` InnerTube clients.
   - Keep Android `usesCleartextTraffic` disabled globally (`127.0.0.1` only in `platform/android/res/xml/network_security_config.xml`) and keep `RECEIVER_NOT_EXPORTED`.
   - Redact cookies, authorization headers, and signed URLs in logs.

4. **Tooling:**
   - Use `cargo` / `nsh` for Rust workflows and `bun` / `bunx` for JS/TS preview tooling.
   - Do not edit generated `.nsh/rust/` files directly.

## Before Opening a Pull Request

Run the workspace checks and test suites:

```sh
cargo check --workspace
cargo test
cargo run --bin resonance -- --self-check
nsh check
```

All contributions submitted to this repository are licensed under `GPL-3.0-or-later` (see [LICENSE](LICENSE)).
