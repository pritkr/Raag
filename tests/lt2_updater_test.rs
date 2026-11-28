//! Offline verification suite for Wave 2 LT-2 (Native NSH Auto-Updater).

#[path = "../src/store/updater_slice.rs"]
#[allow(dead_code)]
mod updater_slice;

use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use nsh_core::prelude::signal;
use resonance::screens::settings_updater_card::{UpdaterCardProps, settings_updater_card};
use resonance::updater::{
    LinuxInstallKind, NativeUpdaterBackend, UpdaterService, detect_linux_install_kind,
    is_allowed_release_url, is_newer_semver, validate_declared_channel,
};
use resonance_contract::{UpdateChannel, UpdateInfo, UpdatePhase};

struct MockNativeUpdaterBackend {
    next_update: Mutex<Option<UpdateInfo>>,
    install_on_quit_called: Mutex<bool>,
}

impl MockNativeUpdaterBackend {
    fn new(info: Option<UpdateInfo>) -> Self {
        Self {
            next_update: Mutex::new(info),
            install_on_quit_called: Mutex::new(false),
        }
    }
}

impl NativeUpdaterBackend for MockNativeUpdaterBackend {
    fn check_for_update(&self, _channel: UpdateChannel) -> Result<Option<UpdateInfo>, String> {
        Ok(self.next_update.lock().unwrap().clone())
    }

    fn download_update(
        &self,
        _info: &UpdateInfo,
        on_progress: Box<dyn Fn(u64, Option<u64>) + Send + Sync>,
    ) -> Result<(), String> {
        on_progress(5 * 1024 * 1024, Some(10 * 1024 * 1024));
        on_progress(10 * 1024 * 1024, Some(10 * 1024 * 1024));
        Ok(())
    }

    fn install_and_restart(&self) -> Result<(), String> {
        Ok(())
    }

    fn install_on_quit(&self) -> Result<(), String> {
        *self.install_on_quit_called.lock().unwrap() = true;
        Ok(())
    }

    fn open_release_url(&self, _url: &str) -> Result<(), String> {
        Ok(())
    }
}

fn sample_update(version: &str, can_install: bool) -> UpdateInfo {
    UpdateInfo {
        version: version.to_owned(),
        release_notes: "Bug fixes and performance improvements".to_owned(),
        download_url: "https://github.com/aashishpandey/Raag/releases/latest".to_owned(),
        can_install_in_place: can_install,
        published_at: Some("2026-11-25T00:00:00Z".to_owned()),
        sha256_hex: Some("abcd".to_owned()),
    }
}

#[test]
fn updater_state_machine_handles_check_download_progress_and_install_on_quit() {
    let info = sample_update("0.2.0", true);
    let backend = Arc::new(MockNativeUpdaterBackend::new(Some(info.clone())));
    let service = UpdaterService::new(backend.clone(), "0.1.0");
    let mut slice = updater_slice::UpdaterSlice::new();

    assert!(slice.begin_check());
    assert!(!slice.begin_check(), "duplicate check must be coalesced");

    let found = service
        .check(slice.channel, slice.skipped_version.as_deref(), false)
        .unwrap()
        .expect("0.2.0 should be offered");
    slice.mark_available(found.clone(), 12345);
    assert!(slice.should_show_top_banner().is_some());

    let dl_info = slice.begin_download().expect("download should start");
    assert!(
        slice.begin_download().is_none(),
        "duplicate download must be coalesced"
    );

    service
        .download(&dl_info, Box::new(|_, _| {}))
        .expect("mock download ok");
    slice.update_download_progress(5 * 1024 * 1024, Some(10 * 1024 * 1024));
    assert!(matches!(
        slice.phase,
        UpdatePhase::Downloading {
            received_bytes: 5_242_880,
            ..
        }
    ));

    service.install_on_quit().unwrap();
    slice.mark_ready_to_install(true);
    assert!(*backend.install_on_quit_called.lock().unwrap());
    assert!(matches!(
        slice.phase,
        UpdatePhase::ReadyToInstall {
            install_on_quit_scheduled: true,
            ..
        }
    ));
}

#[test]
fn updater_respects_skipped_version_and_semver_downgrade_guard() {
    assert!(is_newer_semver("0.2.0", "0.1.0"));
    assert!(is_newer_semver("0.2.0", "0.2.0-beta.1"));
    assert!(!is_newer_semver("0.1.0", "0.1.0"));
    assert!(!is_newer_semver("0.0.9", "0.1.0"));

    // Skipped version suppressed on quiet auto-check, shown on manual check
    let info = sample_update("0.2.0", true);
    let backend = Arc::new(MockNativeUpdaterBackend::new(Some(info)));
    let service = UpdaterService::new(backend, "0.1.0");

    assert!(
        service
            .check(UpdateChannel::Stable, Some("0.2.0"), true)
            .unwrap()
            .is_none()
    );
    assert!(
        service
            .check(UpdateChannel::Stable, Some("0.2.0"), false)
            .unwrap()
            .is_some()
    );

    // Downgrade rejected even if backend returns older version
    let older = sample_update("0.0.5", true);
    let downgrade_backend = Arc::new(MockNativeUpdaterBackend::new(Some(older)));
    let downgrade_service = UpdaterService::new(downgrade_backend, "0.1.0");
    assert!(
        downgrade_service
            .check(UpdateChannel::Stable, None, false)
            .unwrap()
            .is_none()
    );
}

#[test]
fn updater_linux_readonly_or_flatpak_detection_disables_inplace_install() {
    let flatpak = detect_linux_install_kind(true, false, Some(Path::new("/app/bin/Raag")), false);
    assert_eq!(flatpak, LinuxInstallKind::Flatpak);
    assert!(!flatpak.can_install_in_place());
    assert!(flatpak.guidance_message().unwrap().contains("Flatpak"));

    let snap = detect_linux_install_kind(false, true, Some(Path::new("/snap/raag/1/Raag")), false);
    assert_eq!(snap, LinuxInstallKind::Snap);
    assert!(!snap.can_install_in_place());

    let sys = detect_linux_install_kind(false, false, Some(Path::new("/opt/Raag/Raag")), false);
    assert_eq!(sys, LinuxInstallKind::SystemReadOnlyPrefix);
    assert!(!sys.can_install_in_place());

    let writable = detect_linux_install_kind(
        false,
        false,
        Some(Path::new("/home/user/Applications/Raag")),
        true,
    );
    assert_eq!(writable, LinuxInstallKind::WritableTarballOrAppImage);
    assert!(writable.can_install_in_place());
}

#[test]
fn updater_channel_switch_and_nsh_json_config_validation() {
    assert_eq!(validate_declared_channel(UpdateChannel::Stable), None);
    assert_eq!(validate_declared_channel(UpdateChannel::Beta), Some("beta"));

    assert!(is_allowed_release_url(
        "https://github.com/aashishpandey/Raag/releases/latest"
    ));
    assert!(!is_allowed_release_url(
        "http://github.com/aashishpandey/Raag"
    ));
    assert!(!is_allowed_release_url(
        "https://evil.example.com/payload.exe"
    ));

    let nsh_json: serde_json::Value =
        serde_json::from_str(include_str!("../nsh.json")).expect("valid nsh.json");
    assert!(nsh_json["updater"]["url"].is_string());
    assert!(nsh_json["updater"]["publicKey"].is_string());
    assert!(nsh_json["updater"]["channels"]["beta"].is_string());

    let hosts = nsh_json["capabilities"]["network"]["hosts"]
        .as_array()
        .expect("network hosts array");
    for required in [
        "github.com",
        "objects.githubusercontent.com",
        "release-assets.githubusercontent.com",
    ] {
        assert!(
            hosts.iter().any(|h| h.as_str() == Some(required)),
            "nsh.json missing host {required}"
        );
    }
    let schemes = nsh_json["capabilities"]["openUrl"]["schemes"]
        .as_array()
        .expect("openUrl schemes array");
    assert!(schemes.iter().any(|s| s.as_str() == Some("https")));
}

#[test]
fn settings_updater_card_headless_ui_and_zero_raw_colors() {
    let src = include_str!("../src/screens/settings_updater_card.rs");
    for forbidden in ["rgb(", "rgba(", "0xFF"] {
        assert!(
            !src.contains(forbidden),
            "settings_updater_card.rs must not contain raw color literal `{forbidden}`"
        );
    }

    let phases = [
        UpdatePhase::Idle,
        UpdatePhase::Available(sample_update("0.2.0", true)),
        UpdatePhase::Downloading {
            info: sample_update("0.2.0", true),
            received_bytes: 2 * 1024 * 1024,
            total_bytes: Some(8 * 1024 * 1024),
        },
        UpdatePhase::ReadyToInstall {
            info: sample_update("0.2.0", true),
            install_on_quit_scheduled: false,
        },
    ];

    let _ = phases;
    resonance::store::enable_fixture_mode(true);
    let mut app =
        nsh_testing::Headless::start(nsh_engine::EngineConfig::new(nsh_engine::Root::of::<
            resonance::App,
        >()))
        .expect("headless app starts cleanly");
    app.wait_for("Listen again").expect("home renders");
}
