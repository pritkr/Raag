# W1E — Android Native Host Bootstrap, Kotlin Deduplication & Full Bidirectional JNI Bridge

> **Wave:** Wave 1 (Parallel Agent 1E — Runs concurrently with W1A, W1B, W1C, W1D)
> **Exclusive Write Scope:**
> - `platform/android.rs`
> - `platform/android/**` (delete duplicate root `platform/android/{MainActivity,KeystoreBridge,PlaybackForegroundService}.kt`, consolidate exclusively into `platform/android/src/main/kotlin/dev/resonance/ytmusic/`)
> - `tests/w1e_android_jni_contract_test.rs` (new — host-runnable unit tests for JNI contract serialization, security XML verification, and state bridge)
> **Read-Only Dependencies (From Wave 0):** `AppPaths` + `set_android_app_paths(files_dir, cache_dir)` (`crates/core/src/storage/paths.rs`), `jni = "0.21"` (`Cargo.toml` android target deps).
> **Stitch Hooks (For Wave 1.5 Integration Agent):**
> - Wire `AndroidMediaBridge` + `AndroidKeystoreAuthStore` factory selection in `src/live_engine.rs` under `#[cfg(target_os = "android")]`
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/**`, `src/screens/**`, `platform/{desktop,nsh_audio}.rs`, `crates/core/**`.

---

## 1. Ground-Truth Defects Eliminated

1. **Non-Functional Stub `MainActivity.kt`:** Currently renders two static Android `TextView`s (`"Resonance"` and `"Pure Rust YouTube Music..."`) and **never boots the NSH Rust UI**.
2. **Zero Rust ↔ Kotlin JNI Implementation:** `platform/android.rs` (`AndroidMediaBridge`) only mutates local atomics and logs; it never calls `PlaybackForegroundService.playFromLoopbackProxy()`, `KeystoreBridge`, or an Android WebView sign-in activity.
3. **Duplicate Kotlin Sources:** Identical/diverging files exist at both `platform/android/*.kt` AND `platform/android/src/main/kotlin/dev/resonance/ytmusic/*.kt`, creating maintenance hazards and Gradle source-set ambiguity.
4. **Missing Lifecycle & Audio Focus Handling:** Android audio apps require `AudioManager.OnAudioFocusChangeListener` (ducking for navigation prompts, pausing on phone calls), `ACTION_AUDIO_BECOMING_NOISY` (pausing immediately when Bluetooth/wired headphones disconnect), and predictive back gesture handoff to NSH.

---

## 2. Target Android Native + JNI Architecture

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│  Android App Process (`dev.resonance.ytmusic`, API 26+, arm64-v8a)           │
│                                                                              │
│  ┌────────────────────────────────────┐   ┌────────────────────────────────┐ │
│  │ MainActivity (NSH Host Activity)   │   │ PlaybackForegroundService      │ │
│  │ • EdgeToEdge + Safe-Area Insets    │   │ • Jetpack Media3 ExoPlayer     │ │
│  │ • Predictive Back → NSH Nav        │   │ • MediaSession + Notification  │ │
│  │ • Calls nativeInitPaths(files,cache│   │ • AudioFocus + BecomingNoisy   │ │
│  │ • Hosts WebViewAuthDialogFragment  │   │ • Streams 127.0.0.1 loopback   │ │
│  └─────────────────┬──────────────────┘   └───────────────┬────────────────┘ │
│                    │                                      │                  │
│                    ▼                                      ▼                  │
│          ┌──────────────────────────────────────────────────────────┐        │
│          │ ResonanceNativeBridge.kt (Singleton JNI Gateway)         │        │
│          │  Kotlin → Rust: nativeInitPaths, nativeOnTelemetry,      │        │
│          │                 nativeOnMediaCommand, nativeOnAuthCookies│        │
│          │  Rust → Kotlin: loadAndPlay, play, pause, stop, seekMs,  │        │
│          │                 setVolume, setSpeed, updateMetadata,     │        │
│          │                 keystoreSave/Load/Clear, launchWebSignIn │        │
│          └────────────────────────────┬─────────────────────────────┘        │
│                                       │ JNI (`jni = "0.21"`, GlobalRef VM)   │
│          ┌────────────────────────────▼─────────────────────────────┐        │
│          │ Rust Core (`libresonance.so` cdylib, Tokio + NSH Engine) │        │
│          │  • `platform/android.rs`: `AndroidMediaBridge`,          │        │
│          │    `AndroidKeystoreAuthStore`, `AndroidWebViewAuth`      │        │
│          └──────────────────────────────────────────────────────────┘        │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Detailed Implementation Blueprint

### 3.1 Single-Source Kotlin Tree Cleanup
- **Delete** the 3 duplicate root files:
  - `platform/android/MainActivity.kt`
  - `platform/android/KeystoreBridge.kt`
  - `platform/android/PlaybackForegroundService.kt`
- Keep ALL Kotlin code strictly under `platform/android/src/main/kotlin/dev/resonance/ytmusic/`:
  - `MainActivity.kt`
  - `ResonanceNativeBridge.kt` (new canonical JNI bridge)
  - `PlaybackForegroundService.kt`
  - `KeystoreBridge.kt`
  - `WebViewSignInActivity.kt` (new secure in-app Google sign-in host)
- Update `platform/android/proguard-rules.pro` with explicit `-keep class dev.resonance.ytmusic.ResonanceNativeBridge { *; }` and `-keepclasseswithmembernames` rules for all `external fun` and JNI callback methods so R8 release shrinking never strips JNI symbols.

### 3.2 Real NSH Host `MainActivity.kt`
1. Load `System.loadLibrary("resonance")` cleanly in companion `init`.
2. In `onCreate`:
   - Enable `WindowCompat.setDecorFitsSystemWindows(window, false)` (edge-to-edge drawing; NSH handles `.p_safe_area()`).
   - Register `ResonanceNativeBridge.bindApplicationContext(applicationContext)` and call `ResonanceNativeBridge.nativeInitPaths(filesDir.absolutePath, cacheDir.absolutePath)` **before** mounting the NSH native view so Wave 0 `AppPaths` resolves to Android app-private storage (`0o700`, zero external storage permissions needed!).
   - Mount the NSH Rust Android surface/host container (using NSH `=0.7.0`'s Android host entry point) rather than placeholder `TextView`s.
   - Register `OnBackPressedCallback` forwarding system/predictive back events to NSH (`nativeOnBackPressed(): Boolean`); if NSH returns `false` (root screen), move task to back so audio continues playing in the foreground service instead of killing the process!
   - Bind `PlaybackForegroundService` via `MediaController.Builder` so lock-screen controls stay synchronized.

### 3.3 Full Bidirectional JNI Bridge (`ResonanceNativeBridge.kt` ↔ `platform/android.rs`)
- **JavaVM Retention (Thread-Safe):**
  - Store `static JVM: OnceLock<jni::JavaVM>` and `static BRIDGE_CLASS: OnceLock<jni::objects::GlobalRef>` during `JNI_OnLoad` / `nativeRegisterBridge`.
  - Any Tokio background thread calling into Kotlin attaches safely via `jvm.attach_current_thread()` (using `jni::AttachGuard`, never leaking local JNI frames by wrapping calls in `env.with_local_frame(32, ...)`!).
- **Rust → Kotlin Methods (`AndroidMediaBridge` implementing `AudioBackend`):**
  - `loadAndPlay(generation: Long, proxyUrl: String, title: String, artist: String, album: String?, artworkUrl: String?, durationMs: Long, startPosMs: Long)`
  - `play()`, `pause()`, `stop()`, `seekMs(generation: Long, positionMs: Long)`, `setVolume(volume: Float)`, `setPlaybackSpeed(speed: Float, preservePitch: Boolean)`
  - `updateNowPlaying(title: String, artist: String, album: String?, artworkUrl: String?, playing: Boolean, positionMs: Long, durationMs: Long)`
- **Kotlin → Rust Callbacks (`external fun`):**
  - `nativeInitPaths(filesDir: String, cacheDir: String)` → calls `resonance_core::storage::paths::set_android_app_paths(...)`
  - `nativeOnPlaybackTelemetry(eventKind: Int, generation: Long, positionMs: Long, durationMs: Long, errorMsg: String?)` → pushes `AudioTelemetryEvent` into `PlaybackController`'s channel (identical telemetry contract as Desktop W1D!).
  - `nativeOnMediaCommand(commandKind: Int, argLong: Long)` → maps Media3 notification/Bluetooth headset AVRCP buttons (`Play`, `Pause`, `Next`, `Previous`, `SeekMs`) directly into `PlaybackController`.
  - `nativeOnWebViewCookies(cookieHeader: String, userAgent: String)` / `nativeOnWebViewCancelled()`.

### 3.4 Production-Grade `PlaybackForegroundService.kt` (Media3)
- **Audio Focus & Becoming Noisy:**
  - Configure ExoPlayer with `AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MUSIC).build()` and `setAudioAttributes(attrs, /* handleAudioFocus = */ true)` + `setHandleAudioBecomingNoisy(true)` (automatically pauses on headphone unplug/Bluetooth disconnect and ducks cleanly during GPS navigation!).
- **Progress Telemetry Emitter:**
  - Lightweight 250 ms `Handler(Looper.getMainLooper())` tick active **ONLY** while `exoPlayer.isPlaying` (zero battery drain when paused!) pushing `Progress(generation, currentPosition, duration)` via `nativeOnPlaybackTelemetry`.
- **Playback State & Error Forwarding:**
  - `Player.Listener` forwards `STATE_READY` (`Loaded`), `STATE_BUFFERING` (`Buffering`), `STATE_ENDED` (`Ended`), `onIsPlayingChanged` (`Playing`/`Paused`), and `onPlayerError` (`Failed` with redacted message).
- **Security Invariants Preserved:**
  - Verify `AndroidManifest.xml` keeps `android:usesCleartextTraffic="false"` globally, `network_security_config.xml` permits cleartext **exclusively** for `127.0.0.1` (`LoopbackProxy`), and all runtime `BroadcastReceiver`s register with `Context.RECEIVER_NOT_EXPORTED`.
  - `PlaybackForegroundService` validates every `proxyUrl` starts with `http://127.0.0.1:` before passing to `ExoPlayer` (rejects any non-loopback HTTP URL).

### 3.5 Hardware-Backed `AndroidKeystoreAuthStore` & `; wv`-Stripped WebView Sign-In
1. **`AndroidKeystoreAuthStore` (`platform/android.rs` → `KeystoreBridge.kt`):**
   - `save_session` serializes `AuthSession` JSON and calls `KeystoreBridge.encryptAndStore("auth_session", json)` (AES-256-GCM key stored in AndroidKeyStore hardware TEE/StrongBox, random 12-byte IV prepended to ciphertext in `filesDir/keystore_auth.bin`).
   - `load_session` decrypts and parses; `clear_session` deletes ciphertext and zeroizes buffer.
2. **`WebViewSignInActivity.kt` (Consumed by W2C `AuthSignInOrchestrator`):**
   - Launches isolated Android `WebView` pointing to `https://accounts.google.com/ServiceLogin?continue=https://music.youtube.com`.
   - **Critical Google Sign-In Compatibility:** Strips `; wv` from `WebSettings.userAgentString` (Google blocks sign-in when `; wv` is present in UA) while keeping authentic Chrome Mobile version tokens.
   - Polls `CookieManager.getInstance().getCookie("https://music.youtube.com")` on `onPageFinished` / `shouldOverrideUrlLoading`; once both `SAPISID` (or `__Secure-3PAPISID`) and `SID` are present and URL host is `music.youtube.com`, captures cookies, invokes `nativeOnWebViewCookies(...)`, immediately calls `CookieManager.removeAllCookies` + `WebStorage.deleteAllData()` + `webView.destroy()`, and finishes the activity (ephemeral sign-in hygiene!).

---

## 4. Deterministic Verification (`tests/w1e_android_jni_contract_test.rs`)

Because full Android APK builds run on Android SDK CI runners, include host-runnable structural & contract tests plus Gradle compile verification:
1. `no_duplicate_kotlin_files_exist_at_android_root` — asserts `platform/android/{MainActivity,KeystoreBridge,PlaybackForegroundService}.kt` are removed and canonical files exist under `src/main/kotlin/dev/resonance/ytmusic/`.
2. `android_security_manifest_and_network_config_enforce_invariants` — parses `AndroidManifest.xml` & `network_security_config.xml` asserting `usesCleartextTraffic="false"`, `127.0.0.1`-only cleartext, `mediaPlayback` foreground service type, and `proguard-rules.pro` JNI keep rules.
3. `jni_telemetry_and_command_enum_discriminants_match_kotlin_constants` — parses `ResonanceNativeBridge.kt` event/command constants and verifies exact 1:1 integer parity with Rust `platform/android.rs` constants.
4. `loopback_url_validator_rejects_non_loopback_cleartext` — unit tests URL guard accepting `http://127.0.0.1:41234/stream/abc` and rejecting `http://example.com/...` and `http://192.168.1.5/...`.

```bash
cargo test --test w1e_android_jni_contract_test -- --nocapture
# When Android NDK/SDK is present:
cargo check --target aarch64-linux-android --lib
```
