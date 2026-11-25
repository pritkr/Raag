//! LT-2 Native NSH Auto-Updater (`nsh/updater`), channel management, and
//! audio-aware update etiquette.

pub mod channels;
pub mod service;

pub use channels::{
    is_allowed_release_url, is_newer_semver, parse_semver_tuple, validate_declared_channel,
};
pub use service::{
    AUTO_CHECK_INTERVAL_SECS, LinuxInstallKind, NativeUpdaterBackend, NshNativeUpdaterBackend,
    START_CHECK_DELAY_SECS, UpdaterService, detect_linux_install_kind,
};
