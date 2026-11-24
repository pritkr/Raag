# Resonance (`Raag`)

[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)
[![Rust Edition](https://img.shields.io/badge/Rust-2024-orange.svg)](Cargo.toml)
[![NSH](https://img.shields.io/badge/NSH-0.6.1-informational.svg)](https://nsh.dev)

**Resonance** is a Pure Rust YouTube Music client built on [Native Shell (`nsh.dev`)](https://nsh.dev), targeting **Desktop** (macOS, Linux, Windows) and **Android** (API 26+, `arm64-v8a`).

## Features

- **Pure Rust UI & Reactivity:** Built with `nsh-core` / `nsh-base` (`=0.6.1`), adaptive Desktop sidebar & Mobile bottom-navigation layouts, Light/Dark semantic theming ([`theme.json`](theme.json)), and `>= 48dp` touch targets.
- **In-Process Loopback HTTP Proxy (`127.0.0.1`):** Streams audio through a local Rust proxy with a 256 KB atomic chunk cache, SQLite WAL metadata cache, artwork LRU cache, monotonic request cancellation, and transparent mid-stream HTTP `403` URL re-resolution.
- **Native QuickJS-NG EJS Solver:** Uses `rquickjs` (`0.12.0`) on a dedicated worker thread with a 10s interrupt watchdog, SHA-256 verified hot-updatable `yt-dlp-ejs` manifests ([`assets/ejs/solver_manifest.json`](assets/ejs/solver_manifest.json)), and per-player `n`-parameter memoization.
- **Data-Driven InnerTube Client Matrix:** Configured via [`config/client_matrix.json`](config/client_matrix.json) with explicit `AuthMode` isolation (`ANDROID_VR` never receives browser cookies), `PoTokenPolicy`, decoder-driven format selection (Android Opus-first, Desktop AAC-first, iOS AAC-only), and a standalone `client_probe` binary.
- **Rich Playback Experience:** 50% next-track pre-resolution, synced karaoke lyrics cascade (YouTube Music → LRCLIB → Better Lyrics), SponsorBlock `music_offtopic` auto-skip, and deterministic `--fixture` QA mode.
- **Platform Bridges:** Native `nsh/audio` (`platform/nsh_audio.rs`), optional Desktop MPRIS/SMTC media keys (`desktop-mpris`), and thin Android Kotlin shims for Media3 `PlaybackForegroundService`, `AndroidKeyStore` AES-256-GCM, and `; wv`-stripped WebView sign-in.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design and performance budgets.

```text
├── crates/
│   ├── contract/        # Frozen Phase 0 traits, domain types, PlayerState, EventBus
│   └── core/            # InnerTube ClientMatrix, rquickjs solver, 3-tier cache, LoopbackProxy, PlaybackController
├── src/
│   ├── lib.rs           # NSH root App() component & native module registration
│   ├── store.rs         # Reactive NSH UI state (non-blocking engine thread)
│   ├── live_engine.rs   # Dedicated Tokio runtime & PlaybackController bridge
│   ├── ui/              # Semantic theme tokens, SVG icons, and reusable controls
│   ├── screens/         # Home, Search, Queue, Now Playing, Lyrics, Library, Shell
│   ├── main.rs          # Desktop engine entrypoint (--fixture, --self-check)
│   └── bin/             # client_probe & nsh_preview binaries
├── platform/
│   ├── nsh_audio.rs     # Native nsh/audio AudioPlayer driver
│   ├── desktop.rs       # Desktop fallback & media controls
│   └── android/         # Kotlin Media3 ForegroundService, KeystoreBridge, MainActivity
├── config/              # client_matrix.json
├── assets/ejs/          # solver_manifest.json
└── tests/               # Integration, headless NSH UI, proxy fault, and QA gate suites
```

## Quick Start

### Prerequisites

- **Rust toolchain** (Edition 2024)
- **[NSH CLI (`nsh`)](https://nsh.dev/docs/installation.md)** `0.6.1`
- **Optional (Android):** Android SDK (API 35, `minSdk 26`), NDK 28.2, JDK 17, Gradle
- **Optional (Web preview bridge):** [Bun](https://bun.sh)

### Desktop Check, Smoke Run & Tests

```sh
# Compile check across the workspace
cargo check --workspace

# Run full Rust & headless NSH UI test suite
cargo test

# Engine smoke check (starts SQLite WAL, ChunkCache, LoopbackProxy, PlaybackController)
cargo run --bin resonance -- --self-check

# Run with NSH development host
nsh check
nsh run
```

### Android Build (`arm64-v8a`)

```sh
cargo build --target aarch64-linux-android --release --lib
mkdir -p target/android-jniLibs/arm64-v8a
cp target/aarch64-linux-android/release/libresonance.so target/android-jniLibs/arm64-v8a/libresonance.so
gradle -p platform/android assembleDebug
```

Or package directly with NSH:

```sh
nsh package --target android --abi arm64-v8a
```

### Web Preview Harness (Development Only)

```sh
cargo build --bin nsh_preview
bun install
bun run dev
```

## Security & Privacy

- `ANDROID_VR` (`AnonymousOnly`) and `IOS` clients never receive browser cookies.
- Android `usesCleartextTraffic` is disabled globally; [`network_security_config.xml`](platform/android/res/xml/network_security_config.xml) permits cleartext solely for `127.0.0.1` (the in-process `LoopbackProxy`).
- Broadcast receivers use `RECEIVER_NOT_EXPORTED`, and logs redact cookies, auth headers, and signed stream URLs.
- See [SECURITY.md](SECURITY.md) for vulnerability reporting.

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) before opening a pull request.

## Disclaimer

Resonance is an independent open-source project and is not affiliated with, endorsed by, or connected to YouTube or Google LLC. Users are responsible for complying with applicable terms of service and copyright laws.

## License

Resonance is licensed under the **GNU General Public License v3.0 or later (`GPL-3.0-or-later`)**. See [LICENSE](LICENSE) for the full text.
