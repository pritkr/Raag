# Resonance — Pure Rust Native YouTube Music Client (`nsh.dev`) Architecture (v2 Hardened)

## 1. Phase 0 Walking-Skeleton Gate & Platform Fallbacks (Zero Assumed NSH APIs)

NSH's Rust path (`=0.6.1`) provides **native `nsh/audio`** (`nsh_core::prelude::audio_player`, enabled via `"audio": true` in `nsh.json`) for streaming HTTP/local audio, while leaving Android background playback, lock-screen `MediaSession`, Desktop MPRIS/SMTC media keys, `; wv`-stripped WebView cookie capture, and hardware Keystore outside `nsh/audio`'s scope. Therefore:
- **Exact Pin:** All NSH crates are strictly pinned in `Cargo.toml` to `=0.6.1`.
- **Native `nsh/audio` Primary Player (`platform/nsh_audio.rs`):** Mounted in `App()` on the NSH engine thread (`mount_nsh_audio_driver()`), driving `nsh_core::prelude::audio_player` directly against Resonance's `127.0.0.1` Loopback HTTP Proxy (`Open`, `Resume`, `Pause`, `Seek`, `Set`, `Close`) and pumping `AudioEvent`s (`Load`, `Playing`, `Paused`, `Buffer`, `Progress`, `Ended`) back into reactive store state.
- **4 Mandatory Gate 0 Proofs (`crates/core/src/walking_skeleton.rs` & `tests/architecture_gates_v2.rs`):**
  1. **Locked-Screen Playback & Notification Controls:** Native `nsh/audio` (`platform/nsh_audio.rs`) + `souvlaki` MPRIS/SMTC media keys on Desktop; thin Kotlin Media3 ExoPlayer + `MediaSessionService` host shim (`platform/android/PlaybackForegroundService.kt`) with `AudioFocus`, `ACTION_AUDIO_BECOMING_NOISY` (`RECEIVER_NOT_EXPORTED`), and scoped `network_security_config.xml` (`127.0.0.1` loopback only) for Android background/lock-screen playback.
  2. **WebView Cookie Capture:** Android WebView shim stripping `; wv` and `Version/X.Y` (`PlaybackForegroundService.configureGoogleSignInWebView`) to bypass `403: disallowed_useragent`, capturing `SAPISID` / `__Secure-3PAPISID` and destroying the WebView immediately.
  3. **Hardware-Backed Keystore Secret Storage:** `AndroidKeyStore` + `AES/GCM/NoPadding` 256-bit shim (`platform/android/KeystoreBridge.kt`) on Android; OS Keychain / encrypted store on Desktop.
  4. **Native QuickJS-NG (`rquickjs >= 0.12.0`) EJS Solver Execution:** Verified execution on a dedicated worker thread with 10s interrupt watchdog.

## 2. Updatable `yt-dlp-ejs` Solver Bundle (NOT Raw `base.js`)
- Raw `base.js` evaluation fails against modern YouTube players. Resonance embeds an **updatable `yt-dlp-ejs` asset manifest** (`assets/ejs/solver_manifest.json`, AST extraction via `meriyah` / SWC, `min_quickjs_ng_version: "0.12.0"`).
- Hot-updatable at runtime via `QuickJsSolver::install_ejs_bundle_update` with SHA-256 verification so player churn (e.g. `"found 0 sig function possibilities"`) is remediated without recompiling the app.
- **Proactive Signed-In Solving & Per-Player-ID `n`-Param Cache:** Signed-in `WEB_REMIX` playback invokes EJS proactively, and solved `n`-parameters are cached in memory keyed by `(player_id, raw_n)` to eliminate duplicate JS evaluations across tracks.

## 3. Data-Driven `client_matrix.json`, Nightly Probe Harness & Explicit PO-Token Policy
- Hard-coded client chains and unsupported client attributions are eliminated.
- **`config/client_matrix.json`** (overridable at runtime from remote config) defines active clients, signed-in vs anonymous priorities, `AuthMode` isolation, and explicit **`PoTokenPolicy`**:
  - `WEB_REMIX`: `RequiredForWebFreeBypassedByPremium` (free/anonymous web needs GVS PO token; Premium subscribers bypass GVS PO token and proactively use EJS).
  - `ANDROID_VR`: `AnonymousOnly` + `NotRequired` (NEVER receives browser cookies).
  - `TV_SAMSUNG`: `TvOAuthOnly` (disabled unless TV OAuth is provisioned; Android TV / iOS TV deprecated clients excluded).
  - `IOS`: `RequiredIosGuard` (disabled by default; drops browser cookies).
- **Nightly Probe Binary (`src/bin/client_probe.rs`):** Tests every client against fixed probe videos (free, Premium-only itag, age-restricted; cookies on & off) and atomically writes `client_matrix.json`.

## 4. License Policy & `MusicProvider` Reuse Seam
- **App License:** `GPL-3.0-or-later` (see [LICENSE](LICENSE)).
- **Reuse Over Hand-Written Parsers:** `crates/core/src/innertube/provider_adapter.rs` provides `CompositeMusicProvider` separating `BrowseDelegate` (designed to wrap MIT-licensed `ytmapi-rs` for browse/search/home/library) from Resonance's owned `PlaybackResolver` (`ClientMatrix` + `yt-dlp-ejs` + `LoopbackProxy`).
- **Study Isolation:** `_refs/` is marked strictly study-only and gitignored.

## 5. Performance Budgets & 50% Next-Track Pre-Resolution
To decisively beat cold-start playback latency (~5s in comparable clients):
- **Cold Time-to-First-Audio:** `< 3.0 s` (measured core/proxy cold path `< 20 ms`)
- **Warm / Cached Time-to-First-Audio:** `< 1.5 s` (measured cached first byte `< 15 ms`)
- **Process Memory Budget:** `< 100 MB` RSS (asserted live in CI via `/proc/self/statm`)
- **50% Next-Track Pre-Resolution:** `PlaybackController::tick_position` automatically pre-resolves the next queued track once progress crosses `>= 50%` and pre-registers it with `LoopbackProxy`.

## 6. Decoder-Driven Audio Format Preference Matrix (`crates/core/src/innertube/formats.rs`)
Audio format priority is selected by **native platform decoder support** and **account tier**, not raw bitrate alone:

| Target Platform | Native Audio Stack | Premium Preference | Free / Anonymous Preference | WebM Opus? |
| :--- | :--- | :--- | :--- | :---: |
| **Android** | Media3 ExoPlayer / MediaCodec | `774 → 141 → 251 → 140 → 139` | `251 → 140 → 139` | Yes |
| **Desktop** | `rodio` + Symphonia (AAC-first `.m4a`) | `141 → 774 → 140 → 251 → 139` | `140 → 251 → 139` | Fallback |
| **iOS** | `AVPlayer` | `141 → 140 → 139` | `140 → 139` | **No (Excluded)** |

All playback continues to route through the in-process Rust **Loopback HTTP Streaming Proxy (`127.0.0.1`)** with 256 KB atomic chunk caching and transparent **mid-stream HTTP 403 URL re-resolution**.
