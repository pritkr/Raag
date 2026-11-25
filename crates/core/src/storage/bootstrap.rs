//! ST-1 service bootstrapper wiring `QuickJsSolver`, `PlayerScriptFetcher`, and
//! `RemoteConfigSyncService` under `AppPaths::cache_dir`.

use std::fs;
use std::sync::Arc;

use resonance_contract::{AppPaths, CacheError, CoreError, CoreResult};

use crate::cache::SqliteCacheStore;
use crate::innertube::RemoteConfigSyncService;
use crate::solver::{PlayerScriptFetcher, QuickJsSolver};

/// Wired ST-1 services rooted under `AppPaths::cache_dir`:
/// - `cache_dir/js_solver/` (raw & preprocessed YouTube `base.js` bundles)
/// - `cache_dir/ejs/` (hot-updated `yt-dlp-ejs` bundles & manifest)
/// - `cache_dir/remote_config/` (Ed25519-verified `client_matrix.json` & `solver_manifest.json`)
pub struct St1Services {
    pub solver: Arc<QuickJsSolver>,
    pub player_fetcher: Arc<PlayerScriptFetcher>,
    pub remote_config: Arc<RemoteConfigSyncService>,
}

/// Initializes ST-1 solver, player fetcher, and signed remote-config services
/// strictly inside `paths.cache_dir` (never polluting `paths.data_dir`).
pub fn bootstrap_st1_services(
    paths: &AppPaths,
    cache: Arc<SqliteCacheStore>,
    config_base_url: Option<String>,
) -> CoreResult<St1Services> {
    let js_solver_dir = paths.cache_dir.join("js_solver");
    let ejs_dir = paths.cache_dir.join("ejs");
    let remote_config_dir = paths.cache_dir.join("remote_config");

    for dir in [&js_solver_dir, &ejs_dir, &remote_config_dir] {
        fs::create_dir_all(dir)
            .map_err(|e| CoreError::Cache(CacheError::Io(format!("create ST-1 cache dir: {e}"))))?;
    }

    let solver = Arc::new(QuickJsSolver::new(&paths.cache_dir).with_sqlite_cache(cache.clone()));
    let player_fetcher = Arc::new(PlayerScriptFetcher::new(js_solver_dir));
    let remote_config = Arc::new(RemoteConfigSyncService::new(
        remote_config_dir,
        cache,
        solver.clone(),
        config_base_url,
    )?);

    // Synchronous 0 ms startup load of last verified cached matrix (or embedded fallback).
    let _ = remote_config.load_cached_matrix_sync()?;

    Ok(St1Services {
        solver,
        player_fetcher,
        remote_config,
    })
}
