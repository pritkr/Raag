//! Signed Native Auto-Updater service (`P7`).
//!
//! Coordinates fail-closed Ed25519 manifest checks, streaming SHA-256 downloads,
//! externally-managed package detection, and audio-aware install-on-quit staging.

pub mod downloader;
pub mod installer;
pub mod manifest;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ed25519_dalek::VerifyingKey;
use resonance_contract::{UpdateChannel, UpdateInfo, UpdatePhase};

use crate::unix_secs;
use crate::updater::downloader::download_and_verify_artifact;
use crate::updater::installer::{
    detect_externally_managed_source, stage_install_on_quit, trigger_restart_install,
};
use crate::updater::manifest::{
    DEFAULT_BETA_MANIFEST_URL, DEFAULT_STABLE_MANIFEST_URL, EMBEDDED_UPDATER_PUBKEY_HEX,
    MAX_MANIFEST_BYTES, current_platform_target, parse_verifying_key_hex,
    select_update_for_target, validate_updater_url, verify_and_parse_manifest,
};

#[derive(Clone)]
pub struct UpdaterService {
    http: reqwest::Client,
    staging_dir: PathBuf,
    current_version: String,
    target_override: Option<String>,
    stable_url: String,
    beta_url: String,
    pubkey: Option<VerifyingKey>,
    allow_loopback_http: bool,
    cancel_flag: Arc<AtomicBool>,
    staged_path: Arc<Mutex<Option<String>>>,
}

impl UpdaterService {
    pub fn new(cache_root: PathBuf, current_version: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(format!("Raag-Updater/{}", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        let pubkey_hex = std::env::var("RAAG_UPDATER_ED25519_PUBKEY_HEX")
            .ok()
            .or_else(|| std::env::var("RESONANCE_UPDATER_ED25519_PUBKEY_HEX").ok())
            .filter(|s| !s.trim().is_empty())
            .or_else(|| EMBEDDED_UPDATER_PUBKEY_HEX.map(str::to_owned));
        let pubkey = pubkey_hex
            .as_deref()
            .and_then(|h| parse_verifying_key_hex(h).ok());

        let stable_url = std::env::var("RAAG_UPDATER_STABLE_URL")
            .ok()
            .or_else(|| std::env::var("RESONANCE_UPDATER_STABLE_URL").ok())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_STABLE_MANIFEST_URL.to_owned());
        let beta_url = std::env::var("RAAG_UPDATER_BETA_URL")
            .ok()
            .or_else(|| std::env::var("RESONANCE_UPDATER_BETA_URL").ok())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BETA_MANIFEST_URL.to_owned());

        Self {
            http,
            staging_dir: cache_root.join("updates"),
            current_version: current_version.into(),
            target_override: None,
            stable_url,
            beta_url,
            pubkey,
            allow_loopback_http: false,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            staged_path: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_test_overrides(
        mut self,
        stable_url: String,
        beta_url: String,
        pubkey: VerifyingKey,
        target: impl Into<String>,
    ) -> Self {
        self.stable_url = stable_url;
        self.beta_url = beta_url;
        self.pubkey = Some(pubkey);
        self.target_override = Some(target.into());
        self.allow_loopback_http = true;
        self
    }

    pub fn cancel_download(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    pub fn current_version(&self) -> &str {
        &self.current_version
    }

    pub fn staged_verified_path(&self) -> Option<String> {
        self.staged_path.lock().ok().and_then(|g| g.clone())
    }

    /// Checks the signed release feed for `channel` and returns the resulting [`UpdatePhase`].
    pub async fn check_for_updates(
        &self,
        channel: UpdateChannel,
        skipped_version: Option<&str>,
    ) -> UpdatePhase {
        let now = unix_secs();
        let can_install_in_place = detect_externally_managed_source().is_none();

        let Some(ref pubkey) = self.pubkey else {
            return UpdatePhase::UpToDate {
                checked_at_unix_secs: now,
            };
        };

        let manifest_url = match channel {
            UpdateChannel::Stable => &self.stable_url,
            UpdateChannel::Beta => &self.beta_url,
        };
        if let Err(reason) = validate_updater_url(manifest_url, self.allow_loopback_http) {
            return UpdatePhase::Error(reason);
        }
        let sig_url = format!("{manifest_url}.sig");

        let manifest_bytes = match self.fetch_bounded_bytes(manifest_url).await {
            Ok(b) => b,
            Err(reason) => return UpdatePhase::Error(reason),
        };
        let sig_bytes = match self.fetch_bounded_bytes(&sig_url).await {
            Ok(b) => b,
            Err(reason) => return UpdatePhase::Error(reason),
        };

        let manifest = match verify_and_parse_manifest(
            &manifest_bytes,
            &sig_bytes,
            pubkey,
            self.allow_loopback_http,
        ) {
            Ok(m) => m,
            Err(reason) => return UpdatePhase::Error(reason),
        };

        let target = self
            .target_override
            .as_deref()
            .unwrap_or(current_platform_target());

        let Some(info) = select_update_for_target(
            &manifest,
            &self.current_version,
            channel,
            target,
            can_install_in_place,
        ) else {
            return UpdatePhase::UpToDate {
                checked_at_unix_secs: now,
            };
        };

        if skipped_version.is_some_and(|s| s == info.version) {
            return UpdatePhase::UpToDate {
                checked_at_unix_secs: now,
            };
        }

        UpdatePhase::Available(info)
    }

    /// Downloads and SHA-256 verifies `info`, emitting live progress callbacks.
    pub async fn download_update(
        &self,
        info: UpdateInfo,
        on_progress: impl Fn(u64, Option<u64>) + Send + Sync,
    ) -> UpdatePhase {
        if !info.can_install_in_place {
            return UpdatePhase::Error(
                "This installation is managed by an external package manager — update via your package manager".to_owned(),
            );
        }
        self.cancel_flag.store(false, Ordering::SeqCst);
        match download_and_verify_artifact(
            &self.http,
            &self.staging_dir,
            &info,
            self.allow_loopback_http,
            Some(self.cancel_flag.clone()),
            on_progress,
        )
        .await
        {
            Ok(verified_path) => {
                if let Ok(mut guard) = self.staged_path.lock() {
                    *guard = Some(verified_path.to_string_lossy().into_owned());
                }
                UpdatePhase::ReadyToInstall {
                    info,
                    install_on_quit_scheduled: false,
                }
            }
            Err(reason) => UpdatePhase::Error(reason),
        }
    }

    pub fn stage_install_on_quit(&self) -> Result<String, String> {
        let Some(path) = self.staged_verified_path() else {
            return Err("no verified update artifact is staged yet".to_owned());
        };
        stage_install_on_quit(std::path::Path::new(&path))
    }

    pub fn restart_to_install(&self) -> Result<String, String> {
        let Some(path) = self.staged_verified_path() else {
            return Err("no verified update artifact is staged yet".to_owned());
        };
        trigger_restart_install(std::path::Path::new(&path))
    }

    async fn fetch_bounded_bytes(&self, url: &str) -> Result<Vec<u8>, String> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("update network error: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "update feed returned HTTP {}",
                resp.status().as_u16()
            ));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("failed reading update response: {e}"))?;
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err("update manifest exceeded 256 KB cap".to_owned());
        }
        Ok(bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use manifest::{ReleaseArtifactEntry, SignedUpdateManifest, is_newer_version};

    #[test]
    fn semver_ordering_handles_prereleases_correctly() {
        assert!(is_newer_version("0.1.0", "0.2.0"));
        assert!(is_newer_version("0.2.0-beta.1", "0.2.0-beta.2"));
        assert!(is_newer_version("0.2.0-beta.2", "0.2.0-rc.1"));
        assert!(is_newer_version("0.2.0-rc.1", "0.2.0"));
        assert!(!is_newer_version("0.2.0", "0.2.0-rc.1"));
        assert!(!is_newer_version("0.2.0", "0.1.9"));
    }

    #[test]
    fn ed25519_verification_rejects_tampered_manifest() {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let verifying = signing.verifying_key();
        let manifest = SignedUpdateManifest {
            schema_version: 1,
            version: "0.2.0".into(),
            channel: UpdateChannel::Stable,
            released_at_unix_secs: 1_750_000_000,
            notes_markdown: "- Faster streaming".into(),
            artifacts: vec![ReleaseArtifactEntry {
                target: "linux-x86_64".into(),
                url: "https://example.com/resonance.AppImage".into(),
                sha256: "a".repeat(64),
                size_bytes: 12_345,
            }],
        };
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let sig = signing.sign(&bytes);

        let parsed =
            verify_and_parse_manifest(&bytes, &sig.to_bytes(), &verifying, false).unwrap();
        assert_eq!(parsed.version, "0.2.0");

        let mut tampered = bytes.clone();
        tampered[10] ^= 0xFF;
        assert!(verify_and_parse_manifest(&tampered, &sig.to_bytes(), &verifying, false).is_err());
    }
}
