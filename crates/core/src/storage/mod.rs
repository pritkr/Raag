//! Canonical OS-compliant storage paths and platform directory initialization.

pub mod bootstrap;
pub mod paths;

pub use bootstrap::{St1Services, bootstrap_st1_services};
pub use paths::{
    migrate_legacy_temp_dir_from, migrate_legacy_temp_dir_if_needed, resolve_app_paths,
    set_android_app_paths,
};
