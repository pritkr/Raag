//! Canonical OS-compliant persistent directory resolution (`AppPaths`),
//! Unix `0o700` hardening, Android `.nomedia` sentinels, and one-time `/tmp` migration.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use directories::ProjectDirs;
use resonance_contract::{AppPaths, CacheError, CoreError, CoreResult};

static ANDROID_APP_PATHS: OnceLock<AppPaths> = OnceLock::new();

/// Called from Android JNI (`nativeInitPaths`) to register `Context.filesDir` and `cacheDir`.
pub fn set_android_app_paths(files_dir: PathBuf, cache_dir: PathBuf) {
    let paths = AppPaths {
        downloads_dir: files_dir.join("downloads"),
        config_dir: files_dir.join("config"),
        data_dir: files_dir,
        cache_dir,
    };
    let _ = ANDROID_APP_PATHS.set(paths);
}

fn cache_err(err: impl std::fmt::Display) -> CoreError {
    CoreError::Cache(CacheError::Io(err.to_string()))
}

fn ensure_dir_0700(dir: &Path) -> CoreResult<()> {
    fs::create_dir_all(dir).map_err(cache_err)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(cache_err)?;
    }
    Ok(())
}

fn ensure_nomedia_sentinel(dir: &Path) -> CoreResult<()> {
    ensure_dir_0700(dir)?;
    let sentinel = dir.join(".nomedia");
    if !sentinel.exists() {
        fs::write(&sentinel, b"").map_err(cache_err)?;
    }
    Ok(())
}

fn resolve_base_paths() -> AppPaths {
    if let Some(override_dir) = std::env::var_os("RESONANCE_DATA_DIR")
        && !override_dir.is_empty()
    {
        let data_dir = PathBuf::from(override_dir);
        return AppPaths {
            cache_dir: data_dir.join("cache"),
            config_dir: data_dir.join("config"),
            downloads_dir: data_dir.join("downloads"),
            data_dir,
        };
    }

    if let Some(android_paths) = ANDROID_APP_PATHS.get() {
        return android_paths.clone();
    }

    if let Some(proj) = ProjectDirs::from("dev", "resonance", "Raag") {
        let data_dir = proj.data_local_dir().to_path_buf();
        return AppPaths {
            cache_dir: proj.cache_dir().to_path_buf(),
            config_dir: proj.config_dir().to_path_buf(),
            downloads_dir: data_dir.join("downloads"),
            data_dir,
        };
    }

    let base = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".raag"))
        .unwrap_or_else(|| std::env::temp_dir().join("raag"));

    AppPaths {
        cache_dir: base.join("cache"),
        config_dir: base.join("config"),
        downloads_dir: base.join("downloads"),
        data_dir: base,
    }
}

/// Resolves canonical OS-compliant `AppPaths`, creates directories with `0o700` on Unix,
/// places `.nomedia` sentinels, and performs idempotent one-time legacy `/tmp` migration.
pub fn resolve_app_paths() -> CoreResult<AppPaths> {
    let paths = resolve_base_paths();

    ensure_dir_0700(&paths.data_dir)?;
    ensure_dir_0700(&paths.cache_dir)?;
    ensure_dir_0700(&paths.config_dir)?;
    ensure_nomedia_sentinel(&paths.downloads_dir)?;
    ensure_nomedia_sentinel(&paths.cache_dir.join("chunks"))?;
    ensure_nomedia_sentinel(&paths.cache_dir.join("artwork"))?;

    migrate_legacy_temp_dir_if_needed(&paths)?;

    Ok(paths)
}

/// Copies a single file atomically via a `.tmp` sibling without overwriting an existing destination.
fn copy_file_if_absent(src: &Path, dst: &Path) -> CoreResult<bool> {
    if !src.is_file() || dst.exists() {
        return Ok(false);
    }
    if let Some(parent) = dst.parent() {
        ensure_dir_0700(parent)?;
    }
    let tmp = dst.with_extension("migrating.tmp");
    fs::copy(src, &tmp).map_err(cache_err)?;
    fs::rename(&tmp, dst).map_err(cache_err)?;
    Ok(true)
}

fn copy_dir_if_absent(src_dir: &Path, dst_dir: &Path) -> CoreResult<usize> {
    if !src_dir.is_dir() {
        return Ok(0);
    }
    ensure_dir_0700(dst_dir)?;
    let mut copied = 0;
    for entry in fs::read_dir(src_dir).map_err(cache_err)? {
        let entry = entry.map_err(cache_err)?;
        let src_path = entry.path();
        let dst_path = dst_dir.join(entry.file_name());
        if src_path.is_dir() {
            copied += copy_dir_if_absent(&src_path, &dst_path)?;
        } else if copy_file_if_absent(&src_path, &dst_path)? {
            copied += 1;
        }
    }
    Ok(copied)
}

/// Migrates legacy `temp_dir()/resonance-ytmusic-live` data once if `paths.data_dir/resonance.db`
/// does not already exist.
pub fn migrate_legacy_temp_dir_if_needed(paths: &AppPaths) -> CoreResult<bool> {
    let legacy_dir = std::env::temp_dir().join("resonance-ytmusic-live");
    migrate_legacy_temp_dir_from(&legacy_dir, paths)
}

/// Testable helper: migrates from an explicit `legacy_dir` into `paths` if `resonance.db`
/// is absent in `paths.data_dir` and present in `legacy_dir`.
pub fn migrate_legacy_temp_dir_from(legacy_dir: &Path, paths: &AppPaths) -> CoreResult<bool> {
    let target_db = paths.data_dir.join("resonance.db");
    let marker = paths.data_dir.join(".migrated_from_temp_v1");
    if target_db.exists() || marker.exists() {
        return Ok(false);
    }

    let legacy_db = legacy_dir.join("resonance.db");
    if !legacy_db.is_file() {
        return Ok(false);
    }

    let mut migrated_any = false;
    migrated_any |= copy_file_if_absent(&legacy_db, &target_db)?;
    migrated_any |= copy_file_if_absent(
        &legacy_dir.join("auth_session.json"),
        &paths.data_dir.join("auth_session.json"),
    )?;
    if copy_dir_if_absent(&legacy_dir.join("downloads"), &paths.downloads_dir)? > 0 {
        migrated_any = true;
    }

    if migrated_any {
        fs::write(&marker, b"migrated_from_temp_v1\n").map_err(cache_err)?;
        log::info!(
            "Migrated legacy Resonance data from {} to {}",
            legacy_dir.display(),
            paths.data_dir.display()
        );
    }
    Ok(migrated_any)
}
