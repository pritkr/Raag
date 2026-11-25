# ST-4 — Android Native Host Integration, Bidirectional JNI Bridge & Hardware Keystore Auth

> **Wave:** Wave 1 (Short-Term P1) — Parallelizable with ST-1, ST-2, ST-3
> **Parallelism Boundary:**
> - **Exclusive Write Scope:** `platform/android.rs`, `platform/android/**` (Kotlin sources, `AndroidManifest.xml`, `build.gradle.kts`, `proguard-rules.pro`, `res/**`), `platform/mod.rs` (only the `#[cfg(target_os = "android")]` branches of `create_platform_auth_store` and `platform_auth_status`), `tests/st4_android_jni_contract_test.rs` (new).
> - **Forbidden Edits:** `platform/desktop.rs`, `platform/nsh_audio.rs`, `crates/core/src/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/**`.
> - **Wave Coordination Note:** LT-3 (Android Auto `MediaLibraryService`) runs in Wave 2 *after* ST-4 finishes, upgrading `PlaybackForegroundService.kt` cleanly without conflicting with ST-4.

---

## 1. Problem Statement & Audit Findings

Direct code audit of `platform/android.rs` and `platform/android/` uncovered five blockers preventing a real Android `arm64-v8a` app from working:

1. **Duplicate Kotlin Files:** Identical copies of `MainActivity.kt`, `PlaybackForegroundService.kt`, and `KeystoreBridge.kt` exist both at `platform/android/*.kt` AND `platform/android/src/main/kotlin/dev/resonance/ytmusic/*.kt`. Tests or developers can edit one copy while Gradle compiles the other.
2. **Placeholder `MainActivity.kt`:** `MainActivity.kt` currently mounts two static Android `TextView`s (`"Resonance" / "Pure Rust YouTube Music • arm64-v8a + Media3 Foreground Service"`) instead of hosting the NSH Rust UI surface or wiring Android `filesDir`/`cacheDir` + `JavaVM` into `libresonance.so`.
3. **One-Way Stub JNI (`platform/android.rs`):**
   - `AndroidMediaBridge::load_and_play`, `play`, `pause`, `stop`, `seek_ms`, `set_volume`, and `update_now_playing` only update local Rust atomics and call `log::info!`. They **never call Kotlin `PlaybackForegroundService`**!
   - Conversely, `PlaybackForegroundService.playFromLoopbackProxy(...)` and `configureGoogleSignInWebView(...)` exist in Kotlin, but **nothing invokes them**.
   - Media3 ExoPlayer playback progress, buffering, track completion (`STATE_ENDED`), and media-button events (play/pause/next/prev/seek from lock-screen / Bluetooth AVRCP) aren't bridged back to Rust `PlaybackController`.
4. **`KeystoreBridge.kt` Disconnected from Rust `AuthStore`:** `platform/mod.rs` returns plain `FileAuthStore` on Android instead of calling `KeystoreBridge.{write,read,delete}Secret` (`AndroidKeyStore` + AES-256-GCM).
5. **Android 14+ Foreground Service Lifecycle:** `MainActivity.kt:26` calls plain `startService(...)` instead of `ContextCompat.startForegroundService(...)` / `MediaController` binding, and lacks `POST_NOTIFICATIONS` runtime permission request on API 33+.

---

## 2. Target Bidirectional Android Architecture

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│                         Android Host Process (Kotlin)                        │
│                                                                              │
│  MainActivity (Hosts NSH Surface + Lifecycle + Auth WebView Sheet)           │
│   ├─ onCreate():                                                             │
│   │   • nativeInitHost(appContext, filesDir, cacheDir)  ──► Stores JavaVM +  │
│   │                                                        GlobalRef(Bridge) │
│   │   • Connects MediaController to PlaybackForegroundService                │
│   │   • Mounts NSH Android native root view (App())                          │
│   ├─ launchGoogleSignInWebView():                                            │
│   │   • Modal dialog with `; wv` & `Version/X.Y` stripped UA                 │
│   │   • Captures SAPISID / __Secure-3PAPISID → nativeOnAuthCookiesCaptured() │
│   │   • Immediately clears & destroys WebView                                │
│                                                                              │
│  PlaybackForegroundService (Media3 MediaSessionService + ExoPlayer)          │
│   ├─ Streams ONLY from `http://127.0.0.1:<port>/stream/...`                  │
│   ├─ Handles AudioFocus, ACTION_AUDIO_BECOMING_NOISY (RECEIVER_NOT_EXPORTED) │
│   ├─ Player.Listener → nativeOnPlaybackStateChanged / Progress / Ended       │
│   └─ ForwardingPlayer → intercepts Next/Prev/Play/Pause/Seek → Rust          │
│                                                                              │
│  KeystoreBridge (AndroidKeyStore AES-256-GCM)                                │
│   └─ Called synchronously from Rust `AndroidKeystoreAuthStore` (Tokio thread)│
└──────────────────────────────▲───────────────────────────────┬───────────────┘
                               │ Outbound JNI (Rust → Kotlin)  │ Inbound JNI
                               │ (via cached JavaVM +          │ (Kotlin → Rust)
                               │  GlobalRef<NativeHostBridge>) │
┌──────────────────────────────┴───────────────────────────────▼───────────────┐
│                      Rust Core (`libresonance.so`, arm64-v8a)                │
│                                                                              │
│  platform/android.rs:                                                        │
│   ├─ AndroidJniHost (OnceLock<JavaVM>, GlobalRef to ResonanceNativeBridge)   │
│   ├─ AndroidMediaBridge (impl AudioBackend) → invokes Kotlin ExoPlayer       │
│   ├─ AndroidKeystoreAuthStore (impl AuthStore) → invokes KeystoreBridge      │
│   └─ AndroidSignInLauncher → triggers Kotlin `; wv`-stripped WebView dialog  │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Detailed File-by-File Implementation Plan

### 3.1 Single Source of Truth for Kotlin Files (`platform/android/`)
- **Delete duplicate root copies:** Remove `platform/android/MainActivity.kt`, `platform/android/PlaybackForegroundService.kt`, and `platform/android/KeystoreBridge.kt`. Keep ONLY `platform/android/src/main/kotlin/dev/resonance/ytmusic/*.kt`.
- **Update QA/Architecture Gate Tests:** Verify `tests/architecture_gates_v2.rs` and `tests/phase4_hardening_and_qa.rs` read from `platform/android/src/main/kotlin/dev/resonance/ytmusic/` (if any test still checks the legacy root path, update those paths cleanly in ST-4 or keep a documented path check).

### 3.2 Dedicated Kotlin JNI Façade (`ResonanceNativeBridge.kt` — new)
Instead of scattering JNI reflection across `Activity` and `Service` classes (which breaks under R8/ProGuard and Activity recreation!), centralize ALL Rust ↔ Kotlin calls in one `@Keep` singleton object `dev.resonance.ytmusic.ResonanceNativeBridge`:

```kotlin
@Keep
object ResonanceNativeBridge {
    @Volatile private var appContext: Context? = null
    @Volatile private var serviceRef: WeakReference<PlaybackForegroundService>? = null
    @Volatile private var activityRef: WeakReference<MainActivity>? = null

    // Called from Rust (any Tokio background thread) — dispatches ExoPlayer ops onto Main Looper!
    @JvmStatic fun audioLoadAndPlay(url: String, title: String, artist: String, album: String, artworkUrl: String, durationMs: Long, generation: Long)
    @JvmStatic fun audioPlay()
    @JvmStatic fun audioPause()
    @JvmStatic fun audioStop()
    @JvmStatic fun audioSeekMs(positionMs: Long)
    @JvmStatic fun audioSetVolume(volume: Float)
    @JvmStatic fun audioUpdateMetadata(title: String, artist: String, album: String, artworkUrl: String, playing: Boolean, positionMs: Long)

    // Called from Rust `AndroidKeystoreAuthStore` (thread-safe, runs on Tokio thread)
    @JvmStatic fun keystoreWrite(key: String, plaintext: String): Boolean
    @JvmStatic fun keystoreRead(key: String): String?
    @JvmStatic fun keystoreDelete(key: String)

    // Called from Rust UI Settings "Sign In with Google" button (posts to Main Looper)
    @JvmStatic fun launchSignInWebView()

    // Inbound native hooks into libresonance.so
    @JvmStatic external fun nativeInitHost(bridgeClass: Class<ResonanceNativeBridge>, filesDir: String, cacheDir: String)
    @JvmStatic external fun nativeOnMediaCommand(opcode: Int, argMs: Long)
    @JvmStatic external fun nativeOnPlaybackProgress(generation: Long, positionMs: Long, durationMs: Long)
    @JvmStatic external fun nativeOnPlaybackState(generation: Long, stateCode: Int, errorMsg: String?)
    @JvmStatic external fun nativeOnAuthCookiesCaptured(cookieHeader: String)
}
```
- **Why this design is rock-solid:**
  - Every `audio*` method dispatches to `Handler(Looper.getMainLooper()).post { ... }` because Media3 `ExoPlayer` **crashes with `IllegalStateException` if touched off the Android main thread**, whereas Rust `AudioBackend` runs on Tokio worker threads!
  - If `PlaybackForegroundService` is not yet started when `audioLoadAndPlay` arrives, `ResonanceNativeBridge` starts it via `ContextCompat.startForegroundService(ctx, intent)` and queues the pending `LoadAndPlay` request until `onServiceBound` drains it.
  - Because `ResonanceNativeBridge` uses `ApplicationContext` for `KeystoreBridge`, Keystore reads/writes survive Activity destruction and background playback seamlessly.

### 3.3 Safe Bidirectional Rust JNI Layer (`platform/android.rs`)
- Use the `jni` crate (`0.21`) under `#[cfg(target_os = "android")]` (zero raw `*mut c_void` pointer casting bugs!).
- **Testable Trait Seam (`AndroidBridgeInvoker`):**
  - Define an internal trait `pub trait AndroidBridgeInvoker: Send + Sync` with methods mirroring `ResonanceNativeBridge` (`audio_load_and_play`, `audio_play`, `audio_pause`, `audio_seek_ms`, `keystore_write`, `keystore_read`, `keystore_delete`, `launch_sign_in_webview`).
  - On `#[cfg(target_os = "android")]`: `JniBridgeInvoker` attaches the current Tokio thread via `vm.attach_current_thread()`, invokes the static Kotlin method on the cached `GlobalRef` of `ResonanceNativeBridge`, checks and clears any `env.exception_check()` (redacting exceptions before returning `CoreError`), and detaches cleanly.
  - On Desktop / `cargo test`: `MockAndroidBridgeInvoker` implements `AndroidBridgeInvoker` so the entire `AndroidMediaBridge`, `AndroidKeystoreAuthStore`, command dispatch, and generation filtering logic are 100% unit-tested in `cargo test` on Linux/macOS CI!
- **`AndroidKeystoreAuthStore` (`impl AuthStore`):**
  - Uses key `"ytmusic_auth_session_v1"` in `KeystoreBridge`.
  - Validates `SAPISID` / `__Secure-3PAPISID` presence before encrypting.
  - Falls back deterministically to `0o600` `FileAuthStore` ONLY if `KeystoreBridge` throws a hardware-keystore error (e.g. broken OEM Keymaster), and clears BOTH Keystore and fallback file on `clear_session()` to prevent credential resurrection.
  - Updates `platform_auth_status()` to report `CredentialBackendKind::OsKeyring` (Android Hardware Keystore) vs `EncryptedFallbackFile`.

### 3.4 Media3 `PlaybackForegroundService.kt` Upgrades
- **ForwardingPlayer for Lock-Screen & Bluetooth AVRCP Next/Previous:**
  - Wrap `ExoPlayer` in a Media3 `ForwardingPlayer` inside `MediaSession.Builder(this, forwardingPlayer)` so `seekToNext()`, `seekToPrevious()`, `play()`, `pause()`, and `seekTo(pos)` forward straight to `ResonanceNativeBridge.nativeOnMediaCommand(...)`—keeping Rust `PlaybackController` as the single source of truth for queue order, shuffle, repeat, and SponsorBlock!
- **Progress & Lifecycle Pumping:**
  - Attach `Player.Listener`:
    - `onPlaybackStateChanged(STATE_ENDED)` → `nativeOnPlaybackState(currentGen, STATE_ENDED, null)`
    - `onPlaybackStateChanged(STATE_BUFFERING)` / `onIsPlayingChanged(isPlaying)` → `nativeOnPlaybackState(...)`
    - `onPlayerError(error)` → `nativeOnPlaybackState(currentGen, STATE_ERROR, redactError(error))`
  - Run a lightweight `Handler` 500 ms progress ticker ONLY while `player.isPlaying == true`, reporting `nativeOnPlaybackProgress(currentGen, player.currentPosition, player.duration.coerceAtLeast(0))` so 50% prefetch and SponsorBlock auto-skip work identically on Android even when the screen is locked!
- **Loopback URL Security Guard in Kotlin:**
  - Before calling `exoPlayer.setMediaItem(Uri.parse(url))`, assert `uri.host == "127.0.0.1" || uri.host == "localhost"`. Refuse any non-loopback HTTP URL!

### 3.5 NSH Android Host Integration (`MainActivity.kt` & `build.gradle.kts`)
- **Unify `nsh package --target android` and Custom Kotlin Services:**
  - NSH's `nsh package --target android` (using the H0.0 single-source pinned NSH version) packages the Rust NSH UI root; document and configure the Android host activity so `MainActivity` initializes `ResonanceNativeBridge.nativeInitHost(ResonanceNativeBridge::class.java, filesDir.absolutePath, cacheDir.absolutePath)` before mounting the NSH native view (and requests `POST_NOTIFICATIONS` on Android 13+ when starting playback).
  - Update `platform/android/proguard-rules.pro` to `-keep class dev.resonance.ytmusic.ResonanceNativeBridge { *; }` and `-keep class dev.resonance.ytmusic.KeystoreBridge { *; }` so R8 minification never strips JNI entrypoints!
  - Verify `AndroidManifest.xml` keeps:
    - `android:usesCleartextTraffic="false"`
    - `android:networkSecurityConfig="@xml/network_security_config"` (`127.0.0.1` / `localhost` only)
    - `FOREGROUND_SERVICE_MEDIA_PLAYBACK` + `POST_NOTIFICATIONS` permissions.

---

## 4. Security, Privacy & Edge Cases

1. **JNI Thread Attachment Leaks:** Always use `jni::JavaVM::attach_current_thread()` (auto-detached via `AttachGuard` or thread-local `attach_current_thread_permanently` for long-lived Tokio workers) and ALWAYS check `env.exception_check()` after every Kotlin call—an unhandled Java exception crashes the JVM on the very next JNI call!
2. **Cookie Leak Prevention Across JNI:** Never pass raw `cookie_header` through Android `Intent` extras (which can be logged by `dumpsys activity`); pass directly over in-process JNI `nativeOnAuthCookiesCaptured` and wipe the temporary string reference immediately.
3. **WebView Sign-In Hardening:** Enforce `; wv` & `Version/X.Y` removal, restrict cookie capture strictly to `https://music.youtube.com` (check `Uri.parse(url).scheme == "https" && uri.host == "music.youtube.com"`—never `.startsWith` alone which `https://music.youtube.com.evil.com` could bypass!), and destroy WebView immediately after capture.
4. **OOM / Background Kill:** When Android kills the UI Activity while music plays in the background, `PlaybackForegroundService` keeps `libresonance.so`'s `LoopbackProxy` and Tokio runtime alive in-process; when the user reopens the app, `MainActivity` re-attaches without restarting `LoopbackProxy`.

---

## 5. Offline Verification Test Suite (`tests/st4_android_jni_contract_test.rs`)

Runs on Linux/macOS/Windows CI without an Android emulator via `MockAndroidBridgeInvoker` + static Kotlin/XML/ProGuard contract checks:

1. `android_media_bridge_invokes_outbound_load_play_pause_seek_and_rejects_non_loopback` — verifies `AndroidMediaBridge` dispatches every `AudioBackend` call to `AndroidBridgeInvoker` with generation tags and rejects non-loopback URLs.
2. `android_keystore_auth_store_encrypts_loads_falls_back_and_prevents_resurrection` — tests healthy Keystore path, simulated Keymaster failure fallback to `0o600` file, and dual-wipe on `clear_session()`.
3. `android_inbound_jni_commands_and_telemetry_flow_into_playback_controller` — exercises all 9 media command opcodes (`Play..AudioFocusGained`), `Progress`, and `Ended` generation filtering.
4. `android_repo_hygiene_and_security_contract_gates` — asserts:
   - Zero duplicate `.kt` files at `platform/android/*.kt` (only under `src/main/kotlin/dev/resonance/ytmusic/`).
   - `ResonanceNativeBridge` and `KeystoreBridge` keep rules exist in `proguard-rules.pro`.
   - `PlaybackForegroundService.kt` uses `RECEIVER_NOT_EXPORTED`, strict host check `uri.host == "music.youtube.com"` for WebView cookies, and `127.0.0.1` check before ExoPlayer playback.
   - `AndroidManifest.xml` keeps `usesCleartextTraffic="false"`.

```bash
cargo test --test st4_android_jni_contract_test -- --nocapture
cargo test --test phase4_hardening_and_qa --test architecture_gates_v2
# When Android NDK + Gradle are present:
cargo check --target aarch64-linux-android --lib
gradle -p platform/android compileDebugKotlin
```
