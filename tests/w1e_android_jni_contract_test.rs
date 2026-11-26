use std::fs;
use std::path::Path;
use std::sync::Arc;

use resonance::platform::android::{
    AndroidMediaBridge, AndroidTelemetryEvent, BridgeCall, CMD_AUDIO_FOCUS_GAINED,
    CMD_AUDIO_FOCUS_LOST_TRANSIENT, CMD_BECOMING_NOISY, CMD_NEXT, CMD_PAUSE, CMD_PLAY,
    CMD_PREVIOUS, CMD_SEEK_MS, CMD_STOP, CMD_TOGGLE_PLAY_PAUSE, MockAndroidBridgeInvoker,
    STATE_BUFFERING, STATE_ENDED, STATE_ERROR, STATE_IDLE, STATE_PAUSED, STATE_PLAYING,
    STATE_READY, handle_inbound_auth_cancelled, handle_inbound_back_pressed, is_loopback_http_url,
    set_android_back_can_pop,
};
use resonance_contract::{AudioBackend, NowPlayingMetadata};

#[test]
fn no_duplicate_kotlin_files_exist_at_android_root() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for legacy in [
        "platform/android/MainActivity.kt",
        "platform/android/KeystoreBridge.kt",
        "platform/android/PlaybackForegroundService.kt",
    ] {
        assert!(
            !root.join(legacy).exists(),
            "duplicate legacy Kotlin file must be deleted: {legacy}"
        );
    }

    let kt_dir = root.join("platform/android/src/main/kotlin/dev/resonance/ytmusic");
    for canonical in [
        "MainActivity.kt",
        "ResonanceNativeBridge.kt",
        "PlaybackForegroundService.kt",
        "KeystoreBridge.kt",
        "WebViewSignInActivity.kt",
    ] {
        assert!(
            kt_dir.join(canonical).exists(),
            "missing canonical Kotlin file: {canonical}"
        );
    }
}

#[test]
fn android_security_manifest_and_network_config_enforce_invariants() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(root.join("platform/android/AndroidManifest.xml")).unwrap();
    assert!(manifest.contains("android:usesCleartextTraffic=\"false\""));
    assert!(!manifest.contains("usesCleartextTraffic=\"true\""));
    assert!(manifest.contains("android:foregroundServiceType=\"mediaPlayback\""));
    assert!(manifest.contains("android:enableOnBackInvokedCallback=\"true\""));
    assert!(manifest.contains("WebViewSignInActivity"));

    let net_cfg =
        fs::read_to_string(root.join("platform/android/res/xml/network_security_config.xml"))
            .unwrap();
    assert!(net_cfg.contains("<base-config cleartextTrafficPermitted=\"false\""));
    assert!(net_cfg.contains("<domain includeSubdomains=\"false\">127.0.0.1</domain>"));

    let proguard = fs::read_to_string(root.join("platform/android/proguard-rules.pro")).unwrap();
    assert!(proguard.contains("-keep class dev.resonance.ytmusic.ResonanceNativeBridge { *; }"));
    assert!(proguard.contains("-keep class dev.resonance.ytmusic.WebViewSignInActivity { *; }"));
    assert!(proguard.contains("native <methods>;"));
}

#[test]
fn jni_telemetry_and_command_enum_discriminants_match_kotlin_constants() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bridge_kt =
        fs::read_to_string(root.join(
            "platform/android/src/main/kotlin/dev/resonance/ytmusic/ResonanceNativeBridge.kt",
        ))
        .unwrap();

    let expected_pairs = [
        ("CMD_PLAY", CMD_PLAY),
        ("CMD_PAUSE", CMD_PAUSE),
        ("CMD_TOGGLE_PLAY_PAUSE", CMD_TOGGLE_PLAY_PAUSE),
        ("CMD_NEXT", CMD_NEXT),
        ("CMD_PREVIOUS", CMD_PREVIOUS),
        ("CMD_SEEK_MS", CMD_SEEK_MS),
        ("CMD_BECOMING_NOISY", CMD_BECOMING_NOISY),
        (
            "CMD_AUDIO_FOCUS_LOST_TRANSIENT",
            CMD_AUDIO_FOCUS_LOST_TRANSIENT,
        ),
        ("CMD_AUDIO_FOCUS_GAINED", CMD_AUDIO_FOCUS_GAINED),
        ("CMD_STOP", CMD_STOP),
        ("STATE_ERROR", STATE_ERROR),
        ("STATE_IDLE", STATE_IDLE),
        ("STATE_BUFFERING", STATE_BUFFERING),
        ("STATE_READY", STATE_READY),
        ("STATE_ENDED", STATE_ENDED),
        ("STATE_PLAYING", STATE_PLAYING),
        ("STATE_PAUSED", STATE_PAUSED),
    ];

    for (name, rust_val) in expected_pairs {
        let needle = format!("const val {name} = {rust_val}");
        assert!(
            bridge_kt.contains(&needle),
            "Kotlin ResonanceNativeBridge.kt missing `{needle}` matching Rust constant"
        );
    }
}

#[test]
fn loopback_url_validator_rejects_non_loopback_cleartext() {
    assert!(is_loopback_http_url("http://127.0.0.1:41234/stream/abc"));
    assert!(is_loopback_http_url("http://localhost:8080/stream/xyz"));
    assert!(!is_loopback_http_url("http://example.com/stream/abc"));
    assert!(!is_loopback_http_url("http://192.168.1.5:8080/stream/abc"));
    assert!(!is_loopback_http_url("https://127.0.0.1:41234/stream/abc"));
}

#[tokio::test]
async fn android_speed_webview_cancel_and_predictive_back_contracts() {
    let invoker = Arc::new(MockAndroidBridgeInvoker::new());
    let bridge = AndroidMediaBridge::with_invoker(invoker.clone());
    let mut tel_rx = bridge.take_telemetry_receiver().unwrap();

    bridge
        .load_and_play(
            "http://127.0.0.1:43210/stream/w1e",
            NowPlayingMetadata {
                track_id: "w1e-1".to_owned(),
                title: "Raag Bhairav".to_owned(),
                artist: "Resonance".to_owned(),
                album: None,
                artwork_url: None,
                duration_ms: 180_000,
            },
        )
        .await
        .unwrap();
    bridge.set_playback_speed(1.25, true).await.unwrap();
    bridge.launch_sign_in_webview().unwrap();

    assert!(invoker.calls().contains(&BridgeCall::SetPlaybackSpeed {
        speed: 1.25,
        preserve_pitch: true,
    }));
    assert!(invoker.calls().contains(&BridgeCall::LaunchSignInWebView));

    handle_inbound_auth_cancelled();
    assert!(tel_rx.try_recv().is_err());

    set_android_back_can_pop(false);
    assert!(!handle_inbound_back_pressed());
    set_android_back_can_pop(true);
    assert!(handle_inbound_back_pressed());
    set_android_back_can_pop(false);
}
