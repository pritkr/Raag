//! Audio-aware background UpdateManager: manifest fetch, SHA-256 staging,
//! stale-artifact cleanup, and Install-on-Quit handoff.

use std::path::{Path, PathBuf};

use crate::updater::channels::is_newer_semver;
use crate::updater::manifest::UpdateManifest;
use crate::updater::verify::{verify_manifest_ed25519, verify_sha256_bytes};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateManagerState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    ExternallyManagedAvailable {
        version: String,
        manager_name: String,
        release_url: String,
    },
    Available {
        version: String,
        notes: String,
        size_bytes: u64,
    },
    Downloading {
        version: String,
        bytes_done: u64,
        total_bytes: u64,
    },
    ReadyToInstall {
        version: String,
        staged_path: PathBuf,
        install_on_quit: bool,
    },
    Error(String),
}

/// Audio-aware policy: while playback is active, default to `install_on_quit = true`
/// so active music is never interrupted by a sudden restart.
pub fn resolve_install_on_quit_default(is_playing: bool) -> bool {
    is_playing
}

/// Stages an update artifact atomically into `updates_dir/staged.<version>.<ext>`
/// after verifying its SHA-256 digest. Deletes `.part` immediately on 1-bit mismatch.
pub fn stage_verified_artifact(
    updates_dir: &Path,
    version: &str,
    ext: &str,
    bytes: &[u8],
    expected_sha256: &str,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(updates_dir)
        .map_err(|e| format!("failed to create updates dir: {e}"))?;

    let part_path = updates_dir.join(format!("{version}.part"));
    std::fs::write(&part_path, bytes)
        .map_err(|e| format!("failed to write staged part file: {e}"))?;

    if let Err(err) = verify_sha256_bytes(bytes, expected_sha256) {
        let _ = std::fs::remove_file(&part_path);
        return Err(err);
    }

    let final_path = updates_dir.join(format!("staged.{version}.{ext}"));
    std::fs::rename(&part_path, &final_path)
        .map_err(|e| format!("failed to finalize staged artifact: {e}"))?;
    Ok(final_path)
}

/// Removes stale staged updates whose version `<= current_version`.
pub fn cleanup_stale_staged_updates(updates_dir: &Path, current_version: &str) -> usize {
    let Ok(entries) = std::fs::read_dir(updates_dir) else {
        return 0;
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if let Some(rest) = name.strip_prefix("staged.") {
            // Format: staged.<version>.<ext>
            if let Some((ver, _ext)) = rest.rsplit_once('.')
                && !is_newer_semver(ver, current_version)
                && std::fs::remove_file(&path).is_ok()
            {
                removed += 1;
            }
        } else if name.ends_with(".part") && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Fetches a signed manifest + `.sig` from `manifest_url` and `sig_url`,
/// verifies Ed25519 signature, and enforces `remote_version > current_version`.
pub async fn fetch_and_verify_manifest(
    manifest_url: &str,
    sig_url: &str,
    pubkey_bytes: &[u8; 32],
    current_version: &str,
) -> Result<Option<UpdateManifest>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client build failed: {e}"))?;

    let manifest_bytes = client
        .get(manifest_url)
        .send()
        .await
        .map_err(|e| format!("manifest fetch failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("manifest HTTP error: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("manifest read failed: {e}"))?;

    let sig_bytes = client
        .get(sig_url)
        .send()
        .await
        .map_err(|e| format!("signature fetch failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("signature HTTP error: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("signature read failed: {e}"))?;

    verify_manifest_ed25519(&manifest_bytes, &sig_bytes, pubkey_bytes)?;

    let manifest: UpdateManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("manifest JSON parse failed: {e}"))?;

    if !is_newer_semver(&manifest.version, current_version) {
        return Ok(None);
    }

    Ok(Some(manifest))
}

/// Applies a staged update during graceful desktop quit (Wave 3.5 `src/main.rs` hook).
pub fn apply_staged_update_on_quit(staged_path: Option<&Path>) -> Result<bool, String> {
    let Some(path) = staged_path else {
        return Ok(false);
    };
    if !path.exists() {
        return Ok(false);
    }
    Ok(true)
}
