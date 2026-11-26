//! LT-2 Native NSH Auto-Updater (`nsh/updater`), channel management, and
//! audio-aware update etiquette.

pub mod channels;
pub mod manager;
pub mod manifest;
pub mod platform_env;
pub mod service;
pub mod verify;

pub use channels::{
    is_allowed_release_url, is_newer_semver, parse_semver_tuple, validate_declared_channel,
};
pub use manager::{
    UpdateManagerState, apply_staged_update_on_quit, cleanup_stale_staged_updates,
    fetch_and_verify_manifest, resolve_install_on_quit_default, stage_verified_artifact,
};
pub use manifest::{
    PlatformArtifact, UpdateManifest, channel_manifest_filename, current_platform_key,
};
pub use platform_env::{InstallMode, detect_current_install_mode, detect_install_mode};
pub use service::{
    AUTO_CHECK_INTERVAL_SECS, LinuxInstallKind, NativeUpdaterBackend, NshNativeUpdaterBackend,
    START_CHECK_DELAY_SECS, UpdaterService, detect_linux_install_kind,
};
pub use verify::{verify_manifest_ed25519, verify_sha256_bytes, verify_sha256_file};
