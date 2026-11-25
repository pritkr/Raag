//! Decoupled `NativeUpdaterBackend` trait + `NshNativeUpdaterBackend` + `UpdaterService`.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nsh_core::prelude::open_url;
use nsh_core::runtime::{UpdatePromptOptions, prompt_for_update};
use resonance_contract::{UpdateChannel, UpdateInfo};

use crate::updater::channels::{
    is_allowed_release_url, is_newer_semver, validate_declared_channel,
};

/// Silent startup check delay (15s after boot — zero impact on cold start).
pub const START_CHECK_DELAY_SECS: u64 = 15;

/// Minimum interval between automatic background update checks (12 hours).
pub const AUTO_CHECK_INTERVAL_SECS: u64 = 12 * 60 * 60;

/// Linux installation mode for read-only package-manager detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxInstallKind {
    WritableTarballOrAppImage,
    Flatpak,
    Snap,
    SystemReadOnlyPrefix,
}

impl LinuxInstallKind {
    pub fn can_install_in_place(self) -> bool {
        matches!(self, Self::WritableTarballOrAppImage)
    }

    pub fn guidance_message(self) -> Option<&'static str> {
        match self {
            Self::WritableTarballOrAppImage => None,
            Self::Flatpak => Some("Update via Flatpak (`flatpak update dev.resonance.ytmusic`)."),
            Self::Snap => Some("Update via Snap (`snap refresh raag`)."),
            Self::SystemReadOnlyPrefix => Some(
                "System-managed install detected; update via your distro package manager (apt/pacman).",
            ),
        }
    }
}

/// Detects whether the current Linux environment is read-only / distro-managed.
pub fn detect_linux_install_kind(
    flatpak_info_exists: bool,
    snap_env_present: bool,
    exe_path: Option<&Path>,
    exe_dir_writable: bool,
) -> LinuxInstallKind {
    if flatpak_info_exists {
        return LinuxInstallKind::Flatpak;
    }
    if snap_env_present {
        return LinuxInstallKind::Snap;
    }
    if let Some(p) = exe_path {
        let s = p.to_string_lossy();
        if (s.starts_with("/usr/") || s.starts_with("/opt/")) && !exe_dir_writable {
            return LinuxInstallKind::SystemReadOnlyPrefix;
        }
    }
    if !exe_dir_writable {
        return LinuxInstallKind::SystemReadOnlyPrefix;
    }
    LinuxInstallKind::WritableTarballOrAppImage
}

/// Checks the live host Linux environment for read-only / packaged installs.
pub fn current_linux_install_kind() -> LinuxInstallKind {
    let flatpak = Path::new("/.flatpak-info").exists();
    let snap = std::env::var_os("SNAP").is_some();
    let exe = std::env::current_exe().ok();
    let writable = exe
        .as_deref()
        .and_then(|p| p.parent())
        .map(|dir| {
            std::fs::metadata(dir)
                .map(|m| !m.permissions().readonly())
                .unwrap_or(false)
        })
        .unwrap_or(false);
    detect_linux_install_kind(flatpak, snap, exe.as_deref(), writable)
}

/// Clean trait seam isolating `nsh_core` updater calls for unit/offline testing.
pub trait NativeUpdaterBackend: Send + Sync {
    fn check_for_update(&self, channel: UpdateChannel) -> Result<Option<UpdateInfo>, String>;
    fn download_update(
        &self,
        info: &UpdateInfo,
        on_progress: Box<dyn Fn(u64, Option<u64>) + Send + Sync>,
    ) -> Result<(), String>;
    fn install_and_restart(&self) -> Result<(), String>;
    fn install_on_quit(&self) -> Result<(), String>;
    fn open_release_url(&self, url: &str) -> Result<(), String>;
}

/// Production NSH native updater backend.
#[derive(Default)]
pub struct NshNativeUpdaterBackend;

impl NshNativeUpdaterBackend {
    pub fn new() -> Self {
        Self
    }

    /// Invokes NSH's built-in `prompt_for_update` window/alert on the NSH engine thread.
    pub fn prompt_native_ui(channel: UpdateChannel, quiet: bool) {
        let mut opts = UpdatePromptOptions::default().quiet(quiet);
        if let Some(ch) = validate_declared_channel(channel) {
            opts = opts.channel(ch);
        }
        nsh_core::prelude::spawn(prompt_for_update(opts));
    }
}

impl NativeUpdaterBackend for NshNativeUpdaterBackend {
    fn check_for_update(&self, channel: UpdateChannel) -> Result<Option<UpdateInfo>, String> {
        let _declared = validate_declared_channel(channel);
        // In `nsh run` (dev mode), NSH's native updater resolves `None` without interrupting.
        // Packaged NSH apps dispatch through `prompt_native_ui` / `nsh_core::runtime::updater`.
        #[cfg(target_os = "linux")]
        {
            let kind = current_linux_install_kind();
            if !kind.can_install_in_place() {
                log::info!(
                    "updater: Linux install kind {:?} disables in-place replace",
                    kind
                );
            }
        }
        Ok(None)
    }

    fn download_update(
        &self,
        _info: &UpdateInfo,
        _on_progress: Box<dyn Fn(u64, Option<u64>) + Send + Sync>,
    ) -> Result<(), String> {
        Err("download_update requires a packaged NSH bundle".to_owned())
    }

    fn install_and_restart(&self) -> Result<(), String> {
        nsh_core::restart::spawn_after_exit().map_err(|e| e.to_string())
    }

    fn install_on_quit(&self) -> Result<(), String> {
        Ok(())
    }

    fn open_release_url(&self, url: &str) -> Result<(), String> {
        if !is_allowed_release_url(url) {
            return Err(format!("refused non-allowlisted release URL: {url}"));
        }
        let owned = url.to_owned();
        nsh_core::prelude::spawn(async move {
            let _ = open_url(owned).await;
        });
        Ok(())
    }
}

/// Coordinates single-flight update checks, semver anti-downgrade, and skipped-version policy.
pub struct UpdaterService {
    backend: Arc<dyn NativeUpdaterBackend>,
    current_version: String,
    in_flight: AtomicBool,
}

impl UpdaterService {
    pub fn new(backend: Arc<dyn NativeUpdaterBackend>, current_version: impl Into<String>) -> Self {
        Self {
            backend,
            current_version: current_version.into(),
            in_flight: AtomicBool::new(false),
        }
    }

    /// Checks for updates, enforcing single-flight, semver anti-downgrade, and skipped-version rules.
    pub fn check(
        &self,
        channel: UpdateChannel,
        skipped_version: Option<&str>,
        quiet: bool,
    ) -> Result<Option<UpdateInfo>, String> {
        if self
            .in_flight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("update check already in progress".to_owned());
        }
        let result = self.backend.check_for_update(channel);
        self.in_flight.store(false, Ordering::SeqCst);

        #[cfg_attr(not(target_os = "android"), allow(unused_mut))]
        let Some(mut info) = result? else {
            return Ok(None);
        };

        if !is_newer_semver(&info.version, &self.current_version) {
            return Ok(None);
        }

        if quiet && skipped_version == Some(info.version.as_str()) {
            return Ok(None);
        }

        #[cfg(target_os = "android")]
        {
            info.can_install_in_place = false;
        }

        Ok(Some(info))
    }

    pub fn download(
        &self,
        info: &UpdateInfo,
        on_progress: Box<dyn Fn(u64, Option<u64>) + Send + Sync>,
    ) -> Result<(), String> {
        if !info.can_install_in_place {
            return Err("in-place install unavailable on this platform".to_owned());
        }
        if self
            .in_flight
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err("update operation already in progress".to_owned());
        }
        let res = self.backend.download_update(info, on_progress);
        self.in_flight.store(false, Ordering::SeqCst);
        res
    }

    pub fn install_and_restart(&self) -> Result<(), String> {
        self.backend.install_and_restart()
    }

    pub fn install_on_quit(&self) -> Result<(), String> {
        self.backend.install_on_quit()
    }

    pub fn open_release_url(&self, url: &str) -> Result<(), String> {
        if !is_allowed_release_url(url) {
            return Err(format!("refused non-allowlisted release URL: {url}"));
        }
        self.backend.open_release_url(url)
    }
}
