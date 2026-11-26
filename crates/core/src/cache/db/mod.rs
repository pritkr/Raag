//! SQLite WAL (`rusqlite` bundled) storage decomposed by lifecycle domain:
//! - [`migrations`] — versioned schema migrations (`v1`..`v3`)
//! - [`transient`] — `TransientCacheRepo` (SWR metadata, KV, chunk LRU, EJS memo)
//! - [`state`] — `AppStateRepo` (persistent queue, listening history, settings, library snapshot)
//! - [`offline`] — `OfflineVaultRepo` (durable downloads + pinned collections)
//!
//! [`SqliteCacheStore`] owns the shared WAL connection and exposes focused domain
//! repositories alongside `CacheStore` and `OfflineStore` trait implementations.

pub mod migrations;
pub mod offline;
pub mod state;
pub mod transient;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use resonance_contract::{
    CacheError, CacheStats, CacheStore, CoreError, CoreResult, DownloadRecord, DownloadStatus,
    HistoryEntry, OfflineStats, OfflineStorageError, OfflineStore, QueueState,
};
use rusqlite::Connection;

use crate::cache::chunks::DEFAULT_MAX_AUDIO_CACHE_BYTES;

pub use offline::{OfflineVaultRepo, PinnedCollectionRow};
pub use state::{
    AppStateRepo, load_cached_library_snapshot, load_persisted_settings, save_persisted_settings,
};
pub use transient::TransientCacheRepo;

pub struct SqliteCacheStore {
    db_path: PathBuf,
    transient: TransientCacheRepo,
    app_state: AppStateRepo,
    offline: OfflineVaultRepo,
}

impl SqliteCacheStore {
    pub fn open(data_dir: &Path) -> CoreResult<Self> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        let downloads_dir = data_dir.join("downloads");
        std::fs::create_dir_all(&downloads_dir)
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string())))?;
        let db_path = data_dir.join("resonance.db");
        if db_path.is_dir() {
            let _ = std::fs::remove_dir_all(&db_path);
        }
        let conn = Connection::open(&db_path)
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        migrations::migrate_all(&conn)?;

        let shared = Arc::new(Mutex::new(conn));
        let transient = TransientCacheRepo::new(shared.clone(), DEFAULT_MAX_AUDIO_CACHE_BYTES);
        let app_state = AppStateRepo::new(shared.clone(), transient.clone());
        let offline = OfflineVaultRepo::new(shared, downloads_dir);

        Ok(Self {
            db_path,
            transient,
            app_state,
            offline,
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn transient(&self) -> &TransientCacheRepo {
        &self.transient
    }

    pub fn app_state(&self) -> &AppStateRepo {
        &self.app_state
    }

    pub fn offline_vault(&self) -> &OfflineVaultRepo {
        &self.offline
    }
}

impl CacheStore for SqliteCacheStore {
    fn get_visitor_data(&self) -> CoreResult<Option<String>> {
        self.transient.get_kv("visitor_data")
    }

    fn set_visitor_data(&self, visitor_data: &str) -> CoreResult<()> {
        self.transient.set_kv("visitor_data", visitor_data)
    }

    fn get_metadata(&self, key: &str, now_unix_secs: u64) -> CoreResult<Option<(String, bool)>> {
        self.transient.get_metadata(key, now_unix_secs)
    }

    fn put_metadata(
        &self,
        key: &str,
        value_json: &str,
        ttl_secs: u64,
        now_unix_secs: u64,
    ) -> CoreResult<()> {
        self.transient
            .put_metadata(key, value_json, ttl_secs, now_unix_secs)
    }

    fn evict_metadata(&self, key: &str) -> CoreResult<()> {
        self.transient.evict_metadata(key)
    }

    fn load_queue(&self) -> CoreResult<Option<QueueState>> {
        self.app_state.load_queue()
    }

    fn save_queue(&self, queue: &QueueState) -> CoreResult<()> {
        self.app_state.save_queue(queue)
    }

    fn record_history(&self, entry: &HistoryEntry) -> CoreResult<()> {
        self.app_state.record_history(entry)
    }

    fn load_history(&self, limit: usize) -> CoreResult<Vec<HistoryEntry>> {
        self.app_state.load_history(limit)
    }

    fn stats(&self) -> CoreResult<CacheStats> {
        self.transient.stats()
    }

    fn clear_all_caches(&self) -> CoreResult<()> {
        self.transient.clear_all_caches()
    }
}

impl OfflineStore for SqliteCacheStore {
    fn upsert_download(&self, record: &DownloadRecord) -> CoreResult<()> {
        self.offline.upsert_download(record)
    }

    fn update_download_progress(
        &self,
        video_id: &str,
        status: DownloadStatus,
        bytes_downloaded: u64,
        total_bytes: u64,
        updated_at_unix_secs: u64,
    ) -> CoreResult<()> {
        self.offline.update_download_progress(
            video_id,
            status,
            bytes_downloaded,
            total_bytes,
            updated_at_unix_secs,
        )
    }

    fn mark_download_completed(
        &self,
        video_id: &str,
        relative_path: &str,
        file_size_bytes: u64,
        sha256_hex: &str,
        tags_embedded: bool,
        completed_at_unix_secs: u64,
    ) -> CoreResult<()> {
        self.offline.mark_download_completed(
            video_id,
            relative_path,
            file_size_bytes,
            sha256_hex,
            tags_embedded,
            completed_at_unix_secs,
        )
    }

    fn mark_download_failed(
        &self,
        video_id: &str,
        status: DownloadStatus,
        redacted_error: Option<&str>,
        updated_at_unix_secs: u64,
    ) -> CoreResult<()> {
        self.offline
            .mark_download_failed(video_id, status, redacted_error, updated_at_unix_secs)
    }

    fn get_download(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>> {
        self.offline.get_download(video_id)
    }

    fn get_completed_download(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>> {
        self.offline.get_completed_download(video_id)
    }

    fn list_downloads(&self) -> CoreResult<Vec<DownloadRecord>> {
        self.offline.list_downloads()
    }

    fn delete_download_record(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>> {
        self.offline.delete_download_record(video_id)
    }

    fn offline_stats(&self, max_bytes: u64) -> CoreResult<OfflineStats> {
        self.offline.offline_stats(max_bytes)
    }

    fn downloads_root(&self) -> PathBuf {
        self.offline.downloads_root()
    }
}
