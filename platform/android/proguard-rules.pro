# Keep JNI entrypoints and Media3 Foreground Service classes across R8/ProGuard minification.
-keep class dev.resonance.ytmusic.ResonanceNativeBridge { *; }
-keep class dev.resonance.ytmusic.MainActivity { *; }
-keep class dev.resonance.ytmusic.PlaybackForegroundService { *; }
-keep class dev.resonance.ytmusic.KeystoreBridge { *; }
-keep class dev.resonance.ytmusic.WebViewSignInActivity { *; }
-keep class dev.resonance.ytmusic.NativeMediaBridge { *; }
-keep class dev.resonance.ytmusic.GoogleSignInWebViewActivity { *; }
-keep class com.longbridge.nsh.** { *; }
-keepclasseswithmembernames class * {
    native <methods>;
}
