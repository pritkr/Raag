//! First-Class Offline Vault separating intentional offline downloads & pinned
//! albums/playlists from the transient 256 KB LRU chunk cache.
//!
//! Architectural guarantee:
//! - Transient `ChunkCache` LRU eviction and `SqliteCacheStore::clear_all_caches()`
//!   NEVER delete pinned albums/playlists or committed vault media files.
//! - Resumable range downloads + embedded artwork & synced LRC lyrics are delegated
//!   to `offline::DownloadManager` and tracked through SQLite `downloads` + `pinned_collections`.

use std::sync::Arc;

use resonance_contract::{
    CoreError, CoreResult, DownloadRecord, DownloadStatus, OfflineStats, OfflineStorageError,
    OfflineStore, Track,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cache::SqliteCacheStore;
use crate::downloads::state::VaultTrackState;
use crate::offline::{DownloadManager, resolve_confined_path};
use crate::unix_secs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinnedCollectionKind {
    Album,
    Playlist,
}

impl PinnedCollectionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Playlist => "playlist",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "album" => Self::Album,
            _ => Self::Playlist,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedCollection {
    pub id: String,
    pub kind: PinnedCollectionKind,
    pub title: String,
    pub subtitle: Option<String>,
    pub artwork_url: Option<String>,
    pub pinned_at_unix_secs: u64,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VaultSummary {
    pub pinned_collections: usize,
    pub available_tracks: u64,
    pub active_downloads: u64,
    pub total_vault_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnpinReport {
    pub unpinned_id: String,
    pub deleted_tracks: usize,
    pub retained_shared_tracks: usize,
    pub reclaimed_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VaultAuditReport {
    pub verified_count: usize,
    pub corrupted_video_ids: Vec<String>,
}

/// First-class Offline Vault coordinating pinned collections, resumable downloads,
/// artwork/lyrics bundling, and SHA-256 disk integrity verification.
pub struct OfflineVault {
    db: Arc<SqliteCacheStore>,
    manager: Arc<DownloadManager>,
    max_vault_bytes: u64,
}

impl OfflineVault {
    pub fn new(db: Arc<SqliteCacheStore>, manager: Arc<DownloadManager>) -> Self {
        Self {
            db,
            manager,
            max_vault_bytes: crate::offline::DEFAULT_MAX_OFFLINE_BYTES,
        }
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_vault_bytes = max_bytes;
        self
    }

    /// Pins an album or playlist in SQLite (`pinned_collections`) and enqueues resumable
    /// offline downloads (with artwork & lyrics bundling) for every track in the collection.
    pub async fn pin_collection(
        &self,
        id: &str,
        kind: PinnedCollectionKind,
        title: &str,
        subtitle: Option<&str>,
        artwork_url: Option<&str>,
        tracks: &[Track],
    ) -> CoreResult<PinnedCollection> {
        let now = unix_secs();
        self.db.upsert_pinned_collection(
            id,
            kind.as_str(),
            title,
            subtitle,
            artwork_url,
            tracks,
            now,
        )?;

        for track in tracks {
            self.manager.enqueue(track.clone()).await?;
        }

        Ok(PinnedCollection {
            id: id.to_owned(),
            kind,
            title: title.to_owned(),
            subtitle: subtitle.map(str::to_owned),
            artwork_url: artwork_url.map(str::to_owned),
            pinned_at_unix_secs: now,
            tracks: tracks.to_vec(),
        })
    }

    /// Unpins a collection from the offline vault.
    pub fn unpin_collection(&self, collection_id: &str) -> CoreResult<bool> {
        self.db.delete_pinned_collection(collection_id)
    }

    /// Lists all pinned albums and playlists stored in the offline vault.
    pub fn list_pinned_collections(&self) -> CoreResult<Vec<PinnedCollection>> {
        let rows = self.db.list_pinned_collection_rows()?;
        Ok(rows
            .into_iter()
            .map(
                |(id, kind, title, subtitle, artwork_url, pinned_at_unix_secs, tracks)| {
                    PinnedCollection {
                        id,
                        kind: PinnedCollectionKind::from_str(&kind),
                        title,
                        subtitle,
                        artwork_url,
                        pinned_at_unix_secs,
                        tracks,
                    }
                },
            )
            .collect())
    }

    /// Returns `true` if `video_id` belongs to at least one pinned album or playlist.
    pub fn is_track_pinned(&self, video_id: &str) -> CoreResult<bool> {
        self.db.is_track_pinned(video_id)
    }

    /// Runs crash recovery on startup, pausing any interrupted in-flight downloads.
    pub async fn startup_recovery(&self) -> CoreResult<usize> {
        self.manager.startup_recovery().await
    }

    /// Returns the underlying `DownloadManager` worker when lower-level control is required.
    pub fn download_manager(&self) -> &Arc<DownloadManager> {
        &self.manager
    }

    /// Enqueues an individual track download into the Offline Vault.
    pub async fn enqueue_track(&self, track: Track) -> CoreResult<()> {
        self.manager.enqueue(track).await.map(|_| ())
    }

    pub async fn pause_track(&self, video_id: &str) -> CoreResult<()> {
        self.manager.pause(video_id).await
    }

    pub async fn resume_track(&self, video_id: &str) -> CoreResult<()> {
        self.manager.resume(video_id).await.map(|_| ())
    }

    pub async fn cancel_track(&self, video_id: &str) -> CoreResult<()> {
        self.manager.cancel(video_id).await
    }

    pub async fn delete_track(&self, video_id: &str) -> CoreResult<()> {
        self.manager.delete(video_id).await
    }

    /// Returns the explicit [`VaultTrackState`] (`Queued -> Downloading -> Verified -> Available`)
    /// for a track in SQLite.
    pub fn track_state(&self, video_id: &str) -> CoreResult<Option<VaultTrackState>> {
        let Some(rec) = self.db.get_download(video_id)? else {
            return Ok(None);
        };
        Ok(Some(VaultTrackState::from_download_status(
            rec.status,
            rec.sha256_hex.is_some(),
        )))
    }

    /// Verifies a committed vault file's SHA-256 checksum on disk (`Verified -> Available`).
    pub fn verify_track_on_disk(&self, video_id: &str) -> CoreResult<VaultTrackState> {
        let Some(rec) = self.db.get_download(video_id)? else {
            return Err(CoreError::OfflineStorage(OfflineStorageError::Io(format!(
                "track {video_id} not found in offline vault"
            ))));
        };
        let Some(rel_path) = rec.relative_path.as_deref() else {
            return Ok(VaultTrackState::from_download_status(rec.status, false));
        };
        let path = resolve_confined_path(&self.db.downloads_root(), rel_path)?;
        let bytes = std::fs::read(&path)
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string())))?;
        let actual_sha = hex::encode(Sha256::digest(&bytes));

        if let Some(expected_sha) = rec.sha256_hex.as_deref()
            && actual_sha != expected_sha
        {
            return Err(CoreError::OfflineStorage(
                OfflineStorageError::IntegrityMismatch {
                    video_id: video_id.to_owned(),
                    expected: expected_sha.to_owned(),
                    actual: actual_sha,
                },
            ));
        }

        let verified = VaultTrackState::Downloading
            .transition(VaultTrackState::Verified)
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e)))?;
        verified
            .transition(VaultTrackState::Available)
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e)))
    }

    pub fn list_records(&self) -> CoreResult<Vec<DownloadRecord>> {
        self.db.list_downloads()
    }

    pub fn stats(&self) -> CoreResult<OfflineStats> {
        self.db.offline_stats(self.max_vault_bytes)
    }

    pub fn summary(&self) -> CoreResult<VaultSummary> {
        let pinned = self.list_pinned_collections()?.len();
        let st = self.stats()?;
        Ok(VaultSummary {
            pinned_collections: pinned,
            available_tracks: st.completed_count,
            active_downloads: st.active_count,
            total_vault_bytes: st.total_bytes,
        })
    }

    /// Unpins a collection and deletes only tracks with ref-count == 0 across all remaining
    /// pinned collections, returning an [`UnpinReport`] with reclaimed bytes.
    pub async fn unpin_collection_with_cleanup(
        &self,
        collection_id: &str,
    ) -> CoreResult<UnpinReport> {
        let before_bytes = self.stats()?.total_bytes;
        let report = crate::downloads::quota::QuotaEvaluator::unpin_collection_orphan_safe(
            self,
            collection_id,
        )
        .await?;
        let after_bytes = self.stats()?.total_bytes;
        Ok(UnpinReport {
            unpinned_id: collection_id.to_owned(),
            deleted_tracks: report.deleted_orphans,
            retained_shared_tracks: report.preserved_shared,
            reclaimed_bytes: before_bytes.saturating_sub(after_bytes),
        })
    }

    /// Audits every completed download on disk via SHA-256 verification.
    /// Corrupted or missing files are marked `Failed("File corrupted — ready to re-download")`.
    pub fn audit_all_tracks(&self) -> CoreResult<VaultAuditReport> {
        let records = self.list_records()?;
        let mut verified_count = 0usize;
        let mut corrupted_video_ids = Vec::new();

        for rec in records {
            if rec.status != DownloadStatus::Completed {
                continue;
            }
            match self.verify_track_on_disk(&rec.video_id) {
                Ok(_) => verified_count += 1,
                Err(_) => {
                    let _ = self.db.mark_download_failed(
                        &rec.video_id,
                        DownloadStatus::Failed,
                        Some("File corrupted — ready to re-download"),
                        crate::unix_secs(),
                    );
                    corrupted_video_ids.push(rec.video_id);
                }
            }
        }

        Ok(VaultAuditReport {
            verified_count,
            corrupted_video_ids,
        })
    }
}
