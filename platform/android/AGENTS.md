# AGENTS.md — `platform/android/` (Android Host & Build)

Targets Android API 26+ (`targetSdk = 35`, `arm64-v8a`), pairing Pure Rust `libresonance.so` with Kotlin shims where NSH `=0.6.1` lacks Android background/keystore/WebView APIs.

- **Toolchain:** `.cargo/config.toml` sets NDK 28.2 `aarch64-linux-android26-clang`, `llvm-ar`, and bindgen sysroot. Use system `gradle` (no `gradlew` in repo). `build.gradle.kts` reads `jniLibs` from `../../target/android-jniLibs`.
- **Kotlin Shims:**
  - `MainActivity.kt` — Probes `KeystoreBridge`, starts `PlaybackForegroundService`.
  - `PlaybackForegroundService.kt` — Media3 ExoPlayer + `MediaSessionService`, `AudioFocus`, `ACTION_AUDIO_BECOMING_NOISY`, and `configureGoogleSignInWebView` (`; wv` / `Version/X.Y` UA stripping).
  - `KeystoreBridge.kt` — `AndroidKeyStore` + `AES/GCM/NoPadding` 256-bit secret storage.
- **Security Gates (`tests/phase4_hardening_and_qa.rs`):** Never set `usesCleartextTraffic="true"` in `AndroidManifest.xml`; keep `res/xml/network_security_config.xml` cleartext restricted to `127.0.0.1` only; keep `RECEIVER_NOT_EXPORTED` and `FOREGROUND_SERVICE_MEDIA_PLAYBACK`.
- **Build & Install:**
  ```sh
  cargo build --target aarch64-linux-android --release --lib && \
  mkdir -p target/android-jniLibs/arm64-v8a && \
  cp target/aarch64-linux-android/release/libresonance.so target/android-jniLibs/arm64-v8a/libresonance.so && \
  gradle -p platform/android assembleDebug
  ```
  (Or `nsh package --target android --abi arm64-v8a`.) Install with `adb install -r platform/android/build/outputs/apk/debug/resonance-android-debug.apk`.
