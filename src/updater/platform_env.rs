//! Platform & package-manager environment detector for safe self-updates.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallMode {
    SelfUpdate,
    ExternallyManaged { manager_name: String },
    SignedApkInstallPrompt,
}

impl InstallMode {
    pub fn can_self_update(&self) -> bool {
        matches!(self, Self::SelfUpdate)
    }

    pub fn external_manager_name(&self) -> Option<&str> {
        match self {
            Self::ExternallyManaged { manager_name } => Some(manager_name.as_str()),
            _ => None,
        }
    }
}

/// Deterministic classifier for Flatpak, Snap, Nix, Homebrew, AppImage, Android, and writable installs.
#[allow(clippy::too_many_arguments)]
pub fn detect_install_mode(
    flatpak_info_exists: bool,
    snap_env_present: bool,
    nix_store_path: bool,
    homebrew_path: bool,
    appimage_env_present: bool,
    exe_path: Option<&Path>,
    exe_dir_writable: bool,
    is_android: bool,
) -> InstallMode {
    if is_android {
        return InstallMode::SignedApkInstallPrompt;
    }
    if flatpak_info_exists {
        return InstallMode::ExternallyManaged {
            manager_name: "Flatpak".to_owned(),
        };
    }
    if snap_env_present {
        return InstallMode::ExternallyManaged {
            manager_name: "Snap".to_owned(),
        };
    }
    if nix_store_path {
        return InstallMode::ExternallyManaged {
            manager_name: "Nix".to_owned(),
        };
    }
    if homebrew_path {
        return InstallMode::ExternallyManaged {
            manager_name: "Homebrew".to_owned(),
        };
    }
    if appimage_env_present {
        return InstallMode::SelfUpdate;
    }
    if let Some(p) = exe_path {
        let s = p.to_string_lossy();
        if (s.starts_with("/usr/") || s.starts_with("/opt/")) && !exe_dir_writable {
            return InstallMode::ExternallyManaged {
                manager_name: "System Package Manager".to_owned(),
            };
        }
    }
    if !exe_dir_writable {
        return InstallMode::ExternallyManaged {
            manager_name: "System Package Manager".to_owned(),
        };
    }
    InstallMode::SelfUpdate
}

/// Probes the live host environment.
pub fn detect_current_install_mode() -> InstallMode {
    let flatpak = Path::new("/.flatpak-info").exists() || std::env::var_os("FLATPAK_ID").is_some();
    let snap = std::env::var_os("SNAP").is_some();
    let appimage = std::env::var_os("APPIMAGE").is_some();
    let exe = std::env::current_exe().ok();
    let exe_str = exe
        .as_deref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let nix = exe_str.starts_with("/nix/store/");
    let homebrew =
        exe_str.starts_with("/opt/homebrew/") || exe_str.starts_with("/usr/local/Cellar/");
    let writable = exe
        .as_deref()
        .and_then(|p| p.parent())
        .map(|dir| {
            std::fs::metadata(dir)
                .map(|m| !m.permissions().readonly())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    detect_install_mode(
        flatpak,
        snap,
        nix,
        homebrew,
        appimage,
        exe.as_deref(),
        writable,
        cfg!(target_os = "android"),
    )
}
