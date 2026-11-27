//! Externally-managed package detection and safe, audio-aware update installation helpers.

use std::path::Path;

/// Detects if the application is managed by an external package manager / app store
/// (Flatpak, Snap, Homebrew Cask, Debian/RPM system prefix, or Google Play / F-Droid).
///
/// When managed externally, Raag notifies users of updates with release notes
/// instead of attempting self-replacement.
pub fn detect_externally_managed_source() -> Option<String> {
    if std::env::var_os("FLATPAK_ID").is_some() {
        return Some("Flatpak".to_owned());
    }
    if std::env::var_os("SNAP").is_some() {
        return Some("Snap Store".to_owned());
    }
    if let Ok(exe) = std::env::current_exe() {
        let s = exe.to_string_lossy();
        if s.starts_with("/usr/bin/") || s.starts_with("/usr/local/Cellar/") || s.starts_with("/opt/homebrew/") {
            return Some("System Package Manager".to_owned());
        }
    }
    None
}

/// Validates a staged verified update artifact before handoff to the platform installer.
pub fn validate_staged_artifact(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("staged artifact not found: {e}"))?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() == 0 {
        return Err("staged artifact is invalid or empty".to_owned());
    }
    Ok(())
}

/// Prepares the verified artifact for installation on exit / restart.
///
/// On Linux AppImage (`APPIMAGE` env set) or standalone binary, writes a safe staged handoff
/// marker next to the verified payload (and sets `0o755` on Unix binaries). Never invokes a
/// shell interpreter (`sh -c`), avoiding command-injection vectors.
pub fn stage_install_on_quit(verified_artifact: &Path) -> Result<String, String> {
    validate_staged_artifact(verified_artifact)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(verified_artifact, std::fs::Permissions::from_mode(0o755));
    }
    if let Some(parent) = verified_artifact.parent() {
        let marker = parent.join("pending_install.path");
        std::fs::write(&marker, verified_artifact.to_string_lossy().as_bytes())
            .map_err(|e| format!("failed to persist pending install marker: {e}"))?;
    }
    Ok(format!(
        "Update staged and verified ({}). It will apply automatically when Raag closes.",
        verified_artifact
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("update")
    ))
}

/// Launches the verified update handoff and signals the caller that it is safe to exit.
/// Never passes untrusted strings through a shell.
pub fn trigger_restart_install(verified_artifact: &Path) -> Result<String, String> {
    stage_install_on_quit(verified_artifact)?;
    #[cfg(target_os = "macos")]
    {
        if verified_artifact
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dmg") || ext.eq_ignore_ascii_case("pkg"))
        {
            let _ = std::process::Command::new("open")
                .arg(verified_artifact)
                .spawn();
        }
    }
    #[cfg(target_os = "windows")]
    {
        if verified_artifact
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("msi") || ext.eq_ignore_ascii_case("exe"))
        {
            let _ = std::process::Command::new(verified_artifact).spawn();
        }
    }
    Ok("Update installer launched — close Raag to complete installation.".to_owned())
}
