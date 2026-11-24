# Resonance — Phase 5 Polish & Hardened Architecture v2 Report

## 1. Six Architecture Hardening Upgrades (Verified in `tests/architecture_gates_v2.rs`)
| # | Architectural Requirement | Implementation & Proof | Status |
| :--- | :--- | :--- | :---: |
| 1 | **Walking-Skeleton Gate 0 + Exact NSH Pin + Fallbacks** | NSH crates strictly pinned to `=0.6.1` in `Cargo.toml`. Zero assumed NSH APIs. Four-part Android/Desktop Gate 0 verified in `crates/core/src/walking_skeleton.rs`: (a) Media3 ExoPlayer + `MediaSessionService` host shim (`platform/android/PlaybackForegroundService.kt`) / Desktop `rodio`+Symphonia+`souvlaki` fallback (`platform/desktop.rs`), (b) `; wv`-stripped WebView cookie capture, (c) `AndroidKeyStore` AES-256-GCM shim (`platform/android/KeystoreBridge.kt`), (d) native `rquickjs >= 0.12.0` solver execution. | **PASS** |
| 2 | **Updatable `yt-dlp-ejs` Solver Bundle + Per-Player-ID `n`-Param Cache** | `assets/ejs/solver_manifest.json` + `QuickJsSolver::install_ejs_bundle_update` (SHA-256 verified hot-reload without recompiling). Signed-in `WEB_REMIX` invokes EJS proactively; `(player_id, raw_n)` cache eliminates repeat JS executions (`js_eval_count` stays `1`). | **PASS** |
| 3 | **Data-Driven `client_matrix.json` + Nightly Probe + PO-Token Policy** | Replaced hard-coded chains with remote-updatable `config/client_matrix.json`, explicit `PoTokenPolicy` (Free web requires GVS PO token; Premium bypasses GVS PO token; iOS requires `iOSGuard`), and standalone `src/bin/client_probe.rs` probe harness binary. | **PASS** |
| 4 | **`MusicProvider` Reuse Seam + License Audit (`Apache-2.0`)** | `CompositeMusicProvider` (`crates/core/src/innertube/provider_adapter.rs`) splits `BrowseDelegate` (MIT `ytmapi-rs` wrapper seam) from Resonance-owned `PlaybackResolver`. GPL libraries (RustyPipe) excluded; `_refs/` gitignored as study-only. | **PASS** |
| 5 | **Performance Budgets + 50% Next-Track Pre-Resolution** | `PlaybackController::tick_position` auto-pre-resolves the upcoming queue track once playback crosses `>= 50%`. Measured gates: Cold TTFA `996.95 ms` (`< 3000 ms`), Warm TTFA `6.89 ms` (`< 1500 ms`), RSS `17.90 MB` (`< 100 MB`). | **PASS** |
| 6 | **Decoder-Driven Target Audio Format Matrix** | `crates/core/src/innertube/formats.rs` gates itags by native decoder + account tier: Android (`774/251` Opus-first), Desktop (`141/140` AAC-first `.m4a`), iOS (`141/140` AAC-only; WebM Opus strictly excluded). Mid-stream HTTP 403 re-resolution preserved in `LoopbackProxy`. | **PASS** |

## 2. Phase 5 UI Polish & Ergonomics (`tests/phase5_polish_and_ergonomics.rs`)
- **Persistent Mini-Player:** Added 2px reactive hairline progress bar + dynamic track monogram tile (`M` → `夜` → `A`), strictly stacked above mobile bottom nav with zero overlap.
- **Now Playing (Priority #1):** Added stream/offline telemetry badge (`Opus 251 • Proxy` ↔ `Offline Cache`), Explicit (`E`) badge, and accessible `>= 48dp` Volume & Mute strip (`Volume 100%` / `−` / `+` / Mute).
- **Synced Lyrics (Priority #2):** Added `Synced • <provider>` badge, active line progress counter (`Line X / Y`), and `-5s` / `+5s` micro-seek controls.
- **Queue (Priority #3):** Added tabular-nums total queue runtime summary (`4 tracks in queue • 15:26 total`) plus quick `Shuffle` and `Repeat` pills.
- **Library & Settings + Shell:** Added Offline Mode toggle, active theme checkmark indicator, visual cache storage quota bar, global dismissible status toast, and decoupled desktop keyboard shortcut mapping (`Space`, `←`/`→`, `N`/`P`, `L`, `M`, `S`, `R`).
