//! Updatable `yt-dlp-ejs` bundle manifest with SHA-256 verification and version pinning.

use resonance_contract::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const EMBEDDED_EJS_MANIFEST_JSON: &str =
    include_str!("../../../../assets/ejs/solver_manifest.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EjsBundleManifest {
    pub bundle_name: String,
    pub bundle_version: String,
    #[serde(default)]
    pub pinned_version: String,
    #[serde(default)]
    pub sha256_hex: String,
    pub min_quickjs_ng_version: String,
    pub ast_parser: String,
    pub updatable_remote_url: String,
    pub entrypoint_fn: String,
    #[serde(default)]
    pub notes: String,
}

impl EjsBundleManifest {
    pub fn embedded_default() -> CoreResult<Self> {
        serde_json::from_str(EMBEDDED_EJS_MANIFEST_JSON)
            .map_err(|e| CoreError::Solver(format!("invalid solver_manifest.json: {e}")))
    }

    /// Computes lowercase hex SHA-256 of a JS bundle payload.
    pub fn sha256_of(bundle_js: &str) -> String {
        hex::encode(Sha256::digest(bundle_js.as_bytes()))
    }

    /// Verifies both version pinning (if `pinned_version` is non-empty) and SHA-256
    /// integrity against `expected_sha256_hex` (or `self.sha256_hex` if non-empty).
    pub fn verify_bundle_integrity(
        &self,
        bundle_js: &str,
        expected_sha256_hex: &str,
    ) -> CoreResult<String> {
        if !self.pinned_version.is_empty() && self.bundle_version != self.pinned_version {
            return Err(CoreError::Solver(format!(
                "EJS bundle version pin violation: pinned {}, got {}",
                self.pinned_version, self.bundle_version
            )));
        }

        let actual_hex = Self::sha256_of(bundle_js);
        let expected = if !expected_sha256_hex.is_empty() {
            expected_sha256_hex
        } else {
            self.sha256_hex.as_str()
        };

        if !expected.is_empty() && actual_hex != expected.to_lowercase() {
            return Err(CoreError::Solver(format!(
                "EJS bundle SHA-256 mismatch: expected {expected}, got {actual_hex}"
            )));
        }
        Ok(actual_hex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_version_pinning_and_sha256_verification() {
        let mut manifest = EjsBundleManifest::embedded_default().unwrap();
        let js = "function solveN(n){return n;}";
        let hash = EjsBundleManifest::sha256_of(js);
        assert!(manifest.verify_bundle_integrity(js, &hash).is_ok());
        assert!(manifest.verify_bundle_integrity(js, "deadbeef").is_err());

        manifest.pinned_version = "0.9.9".into();
        assert!(manifest.verify_bundle_integrity(js, &hash).is_err());
    }
}
