# AGENTS.md — `crates/` (`contract` & `core`)

- **`crates/contract/` (Frozen Seam):** Trait boundaries (`MusicProvider`, `AudioBackend`, `AuthStore`, `CacheStore`, `LyricsProvider`, `SponsorBlockProvider`), domain types, `PlayerState`, `EventBus`, errors. Keep trait changes synced with mocks in `tests/`.
- **InnerTube & `ClientMatrix` (`crates/core/src/innertube/`):**
  - `CompositeMusicProvider` (`provider_adapter.rs`) splits `BrowseDelegate` from `PlaybackResolver`.
  - Use `config/client_matrix.json` (`ClientMatrix`, `AuthMode`, `PoTokenPolicy`) — no hard-coded fallback chains. `ANDROID_VR` is `AnonymousOnly` and `IOS` drops browser cookies.
  - Format priority (`formats.rs`): Android Opus-first (`774/141/251/140/139`), Desktop AAC-first (`141/774/140/251/139`), iOS AAC-only (WebM Opus excluded).
  - Probe harness: `cargo run --bin client_probe`.
- **EJS Solver (`crates/core/src/solver/`):** `rquickjs` (`0.12.0`) worker with 10s interrupt watchdog, `assets/ejs/solver_manifest.json`, SHA-256 verified hot-updates (`install_ejs_bundle_update`), and `(player_id, raw_n)` memoization.
- **Proxy, Cache & Playback:** `LoopbackProxy` (`127.0.0.1`) serves streams over 256 KB `ChunkCache`, cancels stale generations with `409`, and re-resolves mid-stream `403`s. `PlaybackController::tick_position` pre-resolves the next track at `>= 50%`.
- **Verify:** `cargo test --test core_unit_and_snapshots --test proxy_and_fault_injection --test stage2_and_stage3_verification --test architecture_gates_v2`
