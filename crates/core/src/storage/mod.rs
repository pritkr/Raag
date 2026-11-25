//! Canonical OS-compliant storage paths and platform directory initialization.

pub mod paths;

pub use paths::{
    migrate_legacy_temp_dir_from, migrate_legacy_temp_dir_if_needed, resolve_app_paths,
    set_android_app_paths,
};
