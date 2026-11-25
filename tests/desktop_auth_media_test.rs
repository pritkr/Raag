//! Offline verification tests for Desktop Keyring Auth Persistence, Media Controls
//! (`souvlaki` projection + command dispatch), secret redaction, and SAPISIDHASH isolation.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use resonance::platform::desktop::{
    DesktopAudioBridge, DesktopAuthStore, DesktopPlaybackProjection, SecretVault, coalesce_seek_by,
    map_souvlaki_playback,
};
use resonance_contract::{
    AudioBackend, AuthError, AuthSession, AuthStore, CredentialBackendKind, MediaCommand,
    NowPlayingMetadata,
};
use resonance_core::innertube::FileAuthStore;
use resonance_core::innertube::auth::{build_auth_headers, compute_sapisidhash, redact_secrets};
use resonance_core::innertube::clients::{ANDROID_VR, IOS, WEB_REMIX};
use tempfile::tempdir;

struct InMemorySecretVault {
    fail_mode: AtomicBool,
    fail_with_secret_leak: AtomicBool,
    secret: Mutex<Option<String>>,
}

impl InMemorySecretVault {
    fn new() -> Self {
        Self {
            fail_mode: AtomicBool::new(false),
            fail_with_secret_leak: AtomicBool::new(false),
            secret: Mutex::new(None),
        }
    }

    fn set_fail(&self, fail: bool) {
        self.fail_mode.store(fail, Ordering::SeqCst);
    }

    fn raw_secret(&self) -> Option<String> {
        self.secret.lock().unwrap().clone()
    }
}

impl SecretVault for InMemorySecretVault {
    fn get_secret(&self) -> Result<Option<String>, String> {
        if self.fail_mode.load(Ordering::SeqCst) {
            if self.fail_with_secret_leak.load(Ordering::SeqCst) {
                return Err("keyring error: Cookie: SAPISID=secret_leak_value_123".to_owned());
            }
            return Err("org.freedesktop.secrets service unavailable".to_owned());
        }
        Ok(self.secret.lock().unwrap().clone())
    }

    fn set_secret(&self, val: &str) -> Result<(), String> {
        if self.fail_mode.load(Ordering::SeqCst) {
            if self.fail_with_secret_leak.load(Ordering::SeqCst) {
                return Err("keyring write failed for SAPISID=secret_leak_value_123".to_owned());
            }
            return Err("org.freedesktop.secrets locked".to_owned());
        }
        *self.secret.lock().unwrap() = Some(val.to_owned());
        Ok(())
    }

    fn delete_secret(&self) -> Result<(), String> {
        if self.fail_mode.load(Ordering::SeqCst) {
            return Err("org.freedesktop.secrets unavailable".to_owned());
        }
        *self.secret.lock().unwrap() = None;
        Ok(())
    }
}

fn sample_session() -> AuthSession {
    AuthSession {
        cookie_header: "SAPISID=test_sapisid_abc; __Secure-3PAPISID=test_3pa_xyz; SID=sid123"
            .to_owned(),
        auth_user: 0,
        page_id: None,
        visitor_data: Some("visitor-123".to_owned()),
        account_name: Some("Test User".to_owned()),
        is_premium: true,
    }
}

#[test]
fn keyring_store_saves_loads_and_clears_session_without_touching_fallback_file() {
    let dir = tempdir().unwrap();
    let vault = Arc::new(InMemorySecretVault::new());
    let store = DesktopAuthStore::new(vault.clone(), FileAuthStore::new(dir.path()));
    let fallback_path = dir.path().join("keystore_session.json");

    let session = sample_session();
    store.save_session(&session).unwrap();

    assert!(vault.raw_secret().is_some());
    assert!(!fallback_path.exists());
    assert_eq!(store.backend_kind(), CredentialBackendKind::OsKeyring);

    let loaded = store.load_session().unwrap().expect("session should load");
    assert_eq!(loaded, session);

    store.clear_session().unwrap();
    assert!(vault.raw_secret().is_none());
    assert!(store.load_session().unwrap().is_none());
}

#[test]
fn keyring_store_falls_back_to_0600_file_when_os_keyring_unavailable() {
    let dir = tempdir().unwrap();
    let vault = Arc::new(InMemorySecretVault::new());
    vault.set_fail(true);
    let store = DesktopAuthStore::new(vault.clone(), FileAuthStore::new(dir.path()));
    let fallback_path = dir.path().join("keystore_session.json");

    let session = sample_session();
    store.save_session(&session).unwrap();

    assert!(fallback_path.exists());
    assert_eq!(
        store.backend_kind(),
        CredentialBackendKind::EncryptedFallbackFile
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&fallback_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    let loaded = store.load_session().unwrap().expect("fallback session");
    assert_eq!(loaded, session);

    store.clear_session().unwrap();
    assert!(!fallback_path.exists());
}

#[test]
fn keyring_store_rejects_cookie_header_missing_sapisid() {
    let dir = tempdir().unwrap();
    let vault = Arc::new(InMemorySecretVault::new());
    let store = DesktopAuthStore::new(vault.clone(), FileAuthStore::new(dir.path()));
    let fallback_path = dir.path().join("keystore_session.json");

    let invalid = AuthSession {
        cookie_header: "PREF=tz=UTC; VISITOR_INFO1_LIVE=abc".to_owned(),
        auth_user: 0,
        page_id: None,
        visitor_data: None,
        account_name: None,
        is_premium: false,
    };

    let err = store.save_session(&invalid).unwrap_err();
    assert!(matches!(err, AuthError::MissingCookie(_)));
    assert!(vault.raw_secret().is_none());
    assert!(!fallback_path.exists());
}

#[test]
fn keyring_store_clear_wipes_both_vault_and_fallback_preventing_resurrection() {
    let dir = tempdir().unwrap();
    let vault = Arc::new(InMemorySecretVault::new());
    let fallback = FileAuthStore::new(dir.path());
    let session = sample_session();

    // Seed both vault and fallback file.
    fallback.save_session(&session).unwrap();
    let store = DesktopAuthStore::new(vault.clone(), FileAuthStore::new(dir.path()));
    vault
        .set_secret(&serde_json::to_string(&session).unwrap())
        .unwrap();

    assert!(dir.path().join("keystore_session.json").exists());
    assert!(vault.raw_secret().is_some());

    store.clear_session().unwrap();
    assert!(vault.raw_secret().is_none());
    assert!(!dir.path().join("keystore_session.json").exists());

    // Simulate keyring becoming unavailable after sign-out: stale credentials must not resurrect.
    vault.set_fail(true);
    assert_eq!(store.load_session().unwrap(), None);
}

#[tokio::test]
async fn desktop_bridge_dispatches_all_media_commands_to_playback_controller() {
    let bridge = DesktopAudioBridge::with_hwnd(None);
    let mut rx = bridge
        .take_command_receiver()
        .expect("command receiver available once");
    assert!(bridge.take_command_receiver().is_none());

    let meta = NowPlayingMetadata {
        track_id: "vid123".to_owned(),
        title: "Midnight Reflections".to_owned(),
        artist: "Resonance Ensemble".to_owned(),
        album: Some("Nocturne".to_owned()),
        artwork_url: Some("https://example.invalid/art.jpg".to_owned()),
        duration_ms: 210_000,
    };
    bridge
        .load_and_play("http://127.0.0.1:4567/stream/vid123", meta.clone())
        .await
        .unwrap();
    assert_eq!(
        bridge.active_stream_url().as_deref(),
        Some("http://127.0.0.1:4567/stream/vid123")
    );
    bridge.update_now_playing(meta.clone(), true, 12_345).await;
    assert_eq!(bridge.position_ms().await, 12_345);
    assert_eq!(bridge.last_metadata(), Some(meta));

    let expected = vec![
        MediaCommand::Play,
        MediaCommand::Pause,
        MediaCommand::TogglePlayPause,
        MediaCommand::Next,
        MediaCommand::Previous,
        MediaCommand::SeekMs(42_000),
    ];
    for cmd in &expected {
        bridge.inject_command(*cmd);
    }

    let mut received = Vec::new();
    for _ in 0..expected.len() {
        received.push(rx.recv().await.expect("command delivered"));
    }
    assert_eq!(received, expected);
}

#[test]
fn desktop_playback_projection_maps_idle_playing_paused_and_seek_clamping() {
    assert_eq!(
        map_souvlaki_playback(false, 0, false),
        DesktopPlaybackProjection::Stopped
    );
    assert_eq!(
        map_souvlaki_playback(true, 15_000, false),
        DesktopPlaybackProjection::Stopped
    );
    assert_eq!(
        map_souvlaki_playback(true, 15_000, true),
        DesktopPlaybackProjection::Playing {
            position_ms: 15_000
        }
    );
    assert_eq!(
        map_souvlaki_playback(false, 42_000, true),
        DesktopPlaybackProjection::Paused {
            position_ms: 42_000
        }
    );

    // SeekBy underflow and overflow clamping
    assert_eq!(coalesce_seek_by(5_000, false, 15_000, Some(180_000)), 0);
    assert_eq!(
        coalesce_seek_by(170_000, true, 25_000, Some(180_000)),
        180_000
    );
    assert_eq!(
        coalesce_seek_by(30_000, true, 10_000, Some(180_000)),
        40_000
    );
}

#[test]
fn desktop_auth_errors_and_status_redact_cookie_secrets() {
    let dir = tempdir().unwrap();
    let vault = Arc::new(InMemorySecretVault::new());
    vault.set_fail(true);
    vault.fail_with_secret_leak.store(true, Ordering::SeqCst);

    let store = DesktopAuthStore::new(vault, FileAuthStore::new(dir.path()));
    let session = sample_session();
    // Fallback save succeeds even when keyring fails with secret-tainted error message.
    store.save_session(&session).unwrap();

    let status = store.status_for_session(Some(&session));
    assert_eq!(status.backend, CredentialBackendKind::EncryptedFallbackFile);
    assert!(status.signed_in);
    assert!(status.has_sapisid);
    assert!(status.has_3papisid);
    assert_eq!(status.account_name.as_deref(), Some("Test User"));

    let tainted = "keyring failure\nCookie: SAPISID=secret_123\nAuthorization: SAPISIDHASH 1_abc\nhttps://rr1.googlevideo.com/videoplayback?sig=xyz\n";
    let redacted = redact_secrets(tainted);
    assert!(!redacted.contains("secret_123"));
    assert!(!redacted.contains("SAPISIDHASH"));
    assert!(!redacted.contains("videoplayback?sig=xyz"));
    assert!(redacted.contains("[REDACTED_SENSITIVE_LINE]"));
}

#[test]
fn sapisidhash_isolation_allows_web_remix_and_blocks_android_vr_and_ios() {
    let session = sample_session();

    let hash = compute_sapisidhash(&session.cookie_header, WEB_REMIX.origin, 1_700_000_000)
        .expect("SAPISIDHASH computed for WEB_REMIX");
    assert!(hash.contains("SAPISIDHASH 1700000000_"));
    assert!(hash.contains("SAPISID3PHASH 1700000000_"));

    let headers: HashMap<&'static str, String> =
        build_auth_headers(&WEB_REMIX, &session, 1_700_000_000)
            .expect("WEB_REMIX allows browser cookies")
            .into_iter()
            .collect();
    assert!(headers.contains_key("Cookie"));
    assert!(headers.contains_key("Authorization"));

    let vr_err = build_auth_headers(&ANDROID_VR, &session, 1_700_000_000).unwrap_err();
    assert!(matches!(vr_err, AuthError::IsolationViolation(_)));

    let ios_err = build_auth_headers(&IOS, &session, 1_700_000_000).unwrap_err();
    assert!(matches!(ios_err, AuthError::IsolationViolation(_)));
}
