//! Canonical OS-compliant storage layout and remote-config/account contracts.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Canonical OS-compliant directory layout for Resonance (`Raag`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppPaths {
    /// Persistent user state: `resonance.db`, fallback `auth_session.json`, `downloads/`.
    pub data_dir: PathBuf,
    /// Evictable caches: `chunks/`, `artwork/`, `js_solver/`, `ejs/`, `remote_config/`, `rustypipe/`.
    pub cache_dir: PathBuf,
    /// User-editable configuration overrides (optional `client_matrix.override.json`).
    pub config_dir: PathBuf,
    /// Dedicated offline media root (`data_dir/downloads`, `0o700` on Unix, `.nomedia` on Android).
    pub downloads_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProfile {
    pub account_name: String,
    pub channel_handle: Option<String>,
    pub avatar_url: Option<String>,
    pub is_premium: bool,
    pub detected_at_unix_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConfigStatus {
    pub matrix_revision: u64,
    pub matrix_source: String,
    pub solver_bundle_version: String,
    pub signature_verified: bool,
    pub last_sync_unix_secs: Option<u64>,
}
