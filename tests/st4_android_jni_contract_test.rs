use std::fs;
use std::path::Path;
use std::sync::Arc;

use resonance::platform::android::{
    ANDROID_KEYSTORE_KEY, AndroidKeystoreAuthStore, AndroidMediaBridge, AndroidTelemetryEvent,
    BridgeCall, MockAndroidBridgeInvoker, handle_inbound_auth_cookies,
    handle_inbound_media_command, handle_inbound_playback_progress, handle_inbound_playback_state,
};
use resonance_contract::{
    AudioBackend, AuthError, AuthSession, AuthStore, CredentialBackendKind, MediaCommand,
    NowPlayingMetadata, TrackId,
};
use resonance_core::innertube::FileAuthStore;
use tempfile::tempdir;

fn sample_meta() -> NowPlayingMetadata {
    NowPlayingMetadata {
        track_id: TrackId("st4-track-1".to_owned()),
        title: "Raag Yaman".to_owned(),
        artist: "Resonance Ensemble".to_owned(),
        album: Some("Evening Sessions".to_owned()),
        artwork_url: Some("https://lh3.googleusercontent.com/art".to_owned()),
        duration_ms: 245_000,
    }
}

#[tokio::test]
async fn android_media_bridge_invokes_outbound_load_play_pause_seek_and_rejects_non_loopback() {
    let invoker = Arc::new(MockAndroidBridgeInvoker::new());
    let bridge = AndroidMediaBridge::with_invoker(invoker.clone());
    let meta = sample_meta();

    // Non-loopback URLs must be rejected before crossing JNI.
    let err = bridge
        .load_and_play("https://evil.example.com/stream.webm", meta.clone())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("non-loopback"));
    assert!(invoker.calls().is_empty());

    // Loopback URL dispatches with generation = 1, followed by play/pause/seek/stop/volume/metadata.
    bridge
        .load_and_play("http://127.0.0.1:43210/stream/st4-track-1", meta.clone())
        .await
        .unwrap();
    bridge.play().await.unwrap();
    bridge.pause().await.unwrap();
    bridge.seek_ms(42_500).await.unwrap();
    bridge.set_volume(0.85).await.unwrap();
    bridge.update_now_playing(meta.clone(), true, 43_000).await;
    bridge.stop().await.unwrap();

    let calls = invoker.calls();
    assert_eq!(
        calls,
        vec![
            BridgeCall::LoadAndPlay {
                url: "http://127.0.0.1:43210/stream/st4-track-1".to_owned(),
                title: "Raag Yaman".to_owned(),
                artist: "Resonance Ensemble".to_owned(),
                duration_ms: 245_000,
                generation: 1,
            },
            BridgeCall::Play,
            BridgeCall::Pause,
            BridgeCall::SeekMs(42_500),
            BridgeCall::SetVolume(0.85),
            BridgeCall::UpdateMetadata {
                title: "Raag Yaman".to_owned(),
                playing: true,
                position_ms: 43_000,
            },
            BridgeCall::Stop,
        ]
    );
}

#[test]
fn android_keystore_auth_store_encrypts_loads_falls_back_and_prevents_resurrection() {
    let dir = tempdir().unwrap();
    let invoker = Arc::new(MockAndroidBridgeInvoker::new());
    let store = AndroidKeystoreAuthStore::new(invoker.clone(), FileAuthStore::new(dir.path()));

    // Missing required SAPISID / __Secure-3PAPISID cookie must be rejected.
    let bad_session = AuthSession {
        cookie_header: "PREF=tz=UTC".to_owned(),
        account_name: Some("Tester".to_owned()),
        saved_at_unix_secs: 1_700_000_000,
    };
    assert!(matches!(
        store.save_session(&bad_session),
        Err(AuthError::MissingCookie(_))
    ));

    // Healthy Keystore path saves to Keystore and reports OsKeyring.
    let session = AuthSession {
        cookie_header: "SAPISID=abc123; __Secure-3PAPISID=xyz789".to_owned(),
        account_name: Some("Tester".to_owned()),
        saved_at_unix_secs: 1_700_000_100,
    };
    store.save_session(&session).unwrap();
    assert_eq!(store.backend_kind(), CredentialBackendKind::OsKeyring);
    assert!(
        invoker
            .keystore_snapshot()
            .contains_key(ANDROID_KEYSTORE_KEY)
    );
    assert_eq!(store.load_session().unwrap(), Some(session.clone()));

    // Simulated Keymaster failure falls back deterministically to 0o600 FileAuthStore.
    invoker.set_keystore_write_failure(true);
    invoker.set_keystore_read_failure(true);
    store.save_session(&session).unwrap();
    assert_eq!(
        store.backend_kind(),
        CredentialBackendKind::EncryptedFallbackFile
    );
    assert_eq!(store.load_session().unwrap(), Some(session.clone()));

    // Dual-wipe on clear_session() must clear BOTH Keystore and fallback file so
    // old credentials cannot resurrect when Keystore recovers.
    invoker.set_keystore_write_failure(false);
    invoker.set_keystore_read_failure(false);
    store.clear_session().unwrap();
    assert!(
        !invoker
            .keystore_snapshot()
            .contains_key(ANDROID_KEYSTORE_KEY)
    );
    assert_eq!(store.load_session().unwrap(), None);
}

#[tokio::test]
async fn android_inbound_jni_commands_and_telemetry_flow_into_playback_controller() {
    let invoker = Arc::new(MockAndroidBridgeInvoker::new());
    let bridge = AndroidMediaBridge::with_invoker(invoker);
    let mut cmd_rx = bridge.take_command_receiver().unwrap();
    let mut tel_rx = bridge.take_telemetry_receiver().unwrap();

    // Exercise all 10 media command opcodes (Play..Stop).
    let expected = [
        (1, 0, MediaCommand::Play),
        (2, 0, MediaCommand::Pause),
        (3, 0, MediaCommand::TogglePlayPause),
        (4, 0, MediaCommand::Next),
        (5, 0, MediaCommand::Previous),
        (6, 12_345, MediaCommand::SeekMs(12_345)),
        (7, 0, MediaCommand::BecomingNoisy),
        (8, 0, MediaCommand::AudioFocusLostTransient),
        (9, 0, MediaCommand::AudioFocusGained),
        (10, 0, MediaCommand::Stop),
    ];
    for (opcode, arg, expected_cmd) in expected {
        assert_eq!(
            handle_inbound_media_command(opcode, arg),
            Some(expected_cmd)
        );
        assert_eq!(cmd_rx.recv().await.unwrap(), expected_cmd);
    }
    assert_eq!(handle_inbound_media_command(99, 0), None);

    // Advance active generation via load_and_play.
    bridge
        .load_and_play("http://127.0.0.1:45555/stream/track-a", sample_meta())
        .await
        .unwrap();
    let current_gen = bridge.current_generation();
    assert!(current_gen >= 1);

    // Stale generation progress/ended events must be filtered out.
    assert!(!handle_inbound_playback_progress(
        current_gen - 1,
        999,
        1000
    ));
    assert!(!handle_inbound_playback_state(current_gen - 1, 4, None));

    // Current generation progress & ended events flow through.
    assert!(handle_inbound_playback_progress(
        current_gen,
        55_000,
        245_000
    ));
    assert_eq!(bridge.position_ms().await, 55_000);
    assert!(handle_inbound_playback_state(current_gen, 4, None));

    assert_eq!(
        tel_rx.recv().await.unwrap(),
        AndroidTelemetryEvent::Progress {
            generation: current_gen,
            position_ms: 55_000,
            duration_ms: 245_000,
        }
    );
    assert_eq!(
        tel_rx.recv().await.unwrap(),
        AndroidTelemetryEvent::PlaybackState {
            generation: current_gen,
            state_code: 4,
            error_msg: None,
        }
    );

    // Cookie capture validates SAPISID / __Secure-3PAPISID presence.
    assert!(!handle_inbound_auth_cookies("PREF=1"));
    assert!(handle_inbound_auth_cookies("SAPISID=secret123"));
    assert_eq!(
        tel_rx.recv().await.unwrap(),
        AndroidTelemetryEvent::AuthCookiesCaptured {
            cookie_header: "SAPISID=secret123".to_owned(),
        }
    );
}

#[test]
fn android_repo_hygiene_and_security_contract_gates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    // 1. Zero duplicate .kt files at platform/android/*.kt
    for legacy in [
        "platform/android/MainActivity.kt",
        "platform/android/PlaybackForegroundService.kt",
        "platform/android/KeystoreBridge.kt",
    ] {
        assert!(
            !root.join(legacy).exists(),
            "duplicate legacy Kotlin file must be deleted: {legacy}"
        );
    }

    let kt_dir = root.join("platform/android/src/main/kotlin/dev/resonance/ytmusic");
    for file in [
        "ResonanceNativeBridge.kt",
        "MainActivity.kt",
        "PlaybackForegroundService.kt",
        "KeystoreBridge.kt",
    ] {
        assert!(kt_dir.join(file).exists(), "missing Kotlin file: {file}");
    }

    // 2. ProGuard keep rules cover ResonanceNativeBridge & KeystoreBridge
    let proguard = fs::read_to_string(root.join("platform/android/proguard-rules.pro")).unwrap();
    assert!(proguard.contains("-keep class dev.resonance.ytmusic.ResonanceNativeBridge { *; }"));
    assert!(proguard.contains("-keep class dev.resonance.ytmusic.KeystoreBridge { *; }"));

    // 3. PlaybackForegroundService.kt security invariants
    let svc = fs::read_to_string(kt_dir.join("PlaybackForegroundService.kt")).unwrap();
    assert!(svc.contains("RECEIVER_NOT_EXPORTED"));
    assert!(svc.contains("uri.scheme == \"https\" && uri.host == \"music.youtube.com\""));
    assert!(svc.contains("\"127.0.0.1\""));
    assert!(svc.contains("ForwardingPlayer"));

    // 4. AndroidManifest.xml keeps usesCleartextTraffic="false"
    let manifest = fs::read_to_string(root.join("platform/android/AndroidManifest.xml")).unwrap();
    assert!(manifest.contains("android:usesCleartextTraffic=\"false\""));
    assert!(!manifest.contains("usesCleartextTraffic=\"true\""));
}
