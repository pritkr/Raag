//! Offline Vault quota evaluator, collection progress state machine, and
//! ref-counted orphan-safe unpin cleanup (`H2.2`).

use std::collections::HashMap;

use resonance_contract::{
    CoreError, CoreResult, DownloadRecord, DownloadStatus, OfflineStorageError, Track,
};
use serde::{Deserialize, Serialize};

use crate::downloads::vault::{OfflineVault, PinnedCollection, PinnedCollectionKind};

/// Default estimated size per 160 kbps Opus track (4.5 MB).
pub const DEFAULT_ESTIMATED_OPUS_BYTES: u64 = 4_500_000;
/// Default estimated size per 256 kbps Premium track (7.5 MB).
pub const DEFAULT_ESTIMATED_PREMIUM_BYTES: u64 = 7_500_000;

/// Standard quota presets offered in Settings & Library storage bar (bytes; `0` = Unlimited).
pub const QUOTA_PRESETS_BYTES: [(u64, &str); 6] = [
    (1_024 * 1024 * 1024, "1 GB"),
    (2 * 1024 * 1024 * 1024, "2 GB"),
    (5 * 1024 * 1024 * 1024, "5 GB"),
    (10 * 1024 * 1024 * 1024, "10 GB"),
    (25 * 1024 * 1024 * 1024, "25 GB"),
    (0, "Unlimited"),
];

/// Deterministic aggregate progress for a pinned Album or Playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectionOfflineProgress {
    Unpinned,
    Syncing {
        completed: usize,
        total: usize,
        bytes_downloaded: u64,
        bytes_total: u64,
    },
    Paused {
        completed: usize,
        total: usize,
    },
    Complete {
        total: usize,
        total_bytes: u64,
    },
    PartialFailed {
        completed: usize,
        failed: usize,
        total: usize,
    },
}

impl CollectionOfflineProgress {
    pub fn is_pinned(&self) -> bool {
        !matches!(self, Self::Unpinned)
    }

    pub fn format_badge(&self) -> String {
        match self {
            Self::Unpinned => "Pin Offline".to_owned(),
            Self::Syncing {
                completed,
                total,
                bytes_downloaded,
                ..
            } => {
                let mb = *bytes_downloaded as f64 / (1024.0 * 1024.0);
                format!("Downloading {completed} / {total} ({mb:.1} MB)")
            }
            Self::Paused { completed, total } => format!("Paused ({completed} / {total})"),
            Self::Complete { total, total_bytes } => {
                let mb = *total_bytes as f64 / (1024.0 * 1024.0);
                format!("Offline • {total} tracks ({mb:.1} MB)")
            }
            Self::PartialFailed {
                completed,
                failed,
                total,
            } => format!("{completed} / {total} • {failed} failed"),
        }
    }
}

/// Computes deterministic [`CollectionOfflineProgress`] from a [`PinnedCollection`]
/// and the current `DownloadRecord` map.
pub fn compute_collection_progress(
    pinned: Option<&PinnedCollection>,
    records: &HashMap<String, DownloadRecord>,
) -> CollectionOfflineProgress {
    let Some(collection) = pinned else {
        return CollectionOfflineProgress::Unpinned;
    };

    let total = collection.tracks.len();
    if total == 0 {
        return CollectionOfflineProgress::Complete {
            total: 0,
            total_bytes: 0,
        };
    }

    let mut completed = 0usize;
    let mut active = 0usize;
    let mut paused = 0usize;
    let mut failed = 0usize;
    let mut bytes_downloaded = 0u64;
    let mut bytes_total = 0u64;

    for track in &collection.tracks {
        match records.get(&track.id) {
            Some(rec) => {
                bytes_downloaded = bytes_downloaded.saturating_add(rec.bytes_downloaded);
                let est = if rec.total_bytes > 0 {
                    rec.total_bytes
                } else {
                    rec.bytes_downloaded.max(DEFAULT_ESTIMATED_OPUS_BYTES)
                };
                bytes_total = bytes_total.saturating_add(est);

                match rec.status {
                    DownloadStatus::Completed => completed += 1,
                    DownloadStatus::Queued
                    | DownloadStatus::Downloading
                    | DownloadStatus::Tagging => active += 1,
                    DownloadStatus::Paused => paused += 1,
                    DownloadStatus::Failed | DownloadStatus::Cancelled => failed += 1,
                }
            }
            None => {
                bytes_total = bytes_total.saturating_add(DEFAULT_ESTIMATED_OPUS_BYTES);
                active += 1;
            }
        }
    }

    if completed == total {
        CollectionOfflineProgress::Complete {
            total,
            total_bytes: bytes_downloaded,
        }
    } else if active > 0 {
        CollectionOfflineProgress::Syncing {
            completed,
            total,
            bytes_downloaded,
            bytes_total,
        }
    } else if failed > 0 {
        CollectionOfflineProgress::PartialFailed {
            completed,
            failed,
            total,
        }
    } else if paused > 0 {
        CollectionOfflineProgress::Paused { completed, total }
    } else {
        CollectionOfflineProgress::Syncing {
            completed,
            total,
            bytes_downloaded,
            bytes_total,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnpinCleanupReport {
    pub unpinned: bool,
    pub deleted_orphans: usize,
    pub preserved_shared: usize,
}

/// Pre-flight storage quota evaluator and ref-counted orphan-safe unpin coordinator.
pub struct QuotaEvaluator;

impl QuotaEvaluator {
    pub fn estimated_track_bytes(is_premium: bool) -> u64 {
        if is_premium {
            DEFAULT_ESTIMATED_PREMIUM_BYTES
        } else {
            DEFAULT_ESTIMATED_OPUS_BYTES
        }
    }

    /// Pre-flight check before enqueueing offline downloads.
    /// `quota_bytes == 0` means Unlimited.
    /// Never evicts existing pinned collections.
    pub fn check_before_enqueue(
        current_vault_bytes: u64,
        quota_bytes: u64,
        incoming_track_count: usize,
        estimated_bytes_per_track: u64,
    ) -> CoreResult<()> {
        if quota_bytes == 0 || incoming_track_count == 0 {
            return Ok(());
        }
        let incoming_bytes =
            (incoming_track_count as u64).saturating_mul(estimated_bytes_per_track);
        let projected = current_vault_bytes.saturating_add(incoming_bytes);
        if projected > quota_bytes {
            return Err(CoreError::OfflineStorage(
                OfflineStorageError::QuotaExceeded {
                    used_bytes: projected,
                    limit_bytes: quota_bytes,
                },
            ));
        }
        Ok(())
    }

    /// Checks quota for tracks not yet completed in the vault, then pins & enqueues the collection.
    #[allow(clippy::too_many_arguments)]
    pub async fn pin_collection_with_quota(
        vault: &OfflineVault,
        quota_bytes: u64,
        estimated_bytes_per_track: u64,
        id: &str,
        kind: PinnedCollectionKind,
        title: &str,
        subtitle: Option<&str>,
        artwork_url: Option<&str>,
        tracks: &[Track],
    ) -> CoreResult<PinnedCollection> {
        let stats = vault.stats()?;
        let records: HashMap<String, DownloadRecord> = vault
            .list_records()?
            .into_iter()
            .map(|r| (r.video_id.clone(), r))
            .collect();

        let pending_count = tracks
            .iter()
            .filter(|t| {
                !matches!(
                    records.get(&t.id).map(|r| r.status),
                    Some(DownloadStatus::Completed)
                )
            })
            .count();

        Self::check_before_enqueue(
            stats.total_bytes,
            quota_bytes,
            pending_count,
            estimated_bytes_per_track,
        )?;

        vault
            .pin_collection(id, kind, title, subtitle, artwork_url, tracks)
            .await
    }

    /// Unpins a collection and deletes ONLY tracks that are no longer referenced by any
    /// other pinned collection (`is_track_pinned(&video_id) == false`).
    pub async fn unpin_collection_orphan_safe(
        vault: &OfflineVault,
        collection_id: &str,
    ) -> CoreResult<UnpinCleanupReport> {
        let collections = vault.list_pinned_collections()?;
        let target_tracks: Vec<String> = collections
            .into_iter()
            .find(|c| c.id == collection_id)
            .map(|c| c.tracks.into_iter().map(|t| t.id).collect())
            .unwrap_or_default();

        let unpinned = vault.unpin_collection(collection_id)?;
        let mut deleted_orphans = 0usize;
        let mut preserved_shared = 0usize;

        for video_id in target_tracks {
            if vault.is_track_pinned(&video_id)? {
                preserved_shared += 1;
            } else {
                // Ignore NotFound if track was never downloaded yet.
                match vault.delete_track(&video_id).await {
                    Ok(()) => deleted_orphans += 1,
                    Err(CoreError::OfflineStorage(OfflineStorageError::NotFound { .. })) => {}
                    Err(e) => return Err(e),
                }
            }
        }

        Ok(UnpinCleanupReport {
            unpinned,
            deleted_orphans,
            preserved_shared,
        })
    }

    /// Enqueues only delta tracks in a pinned collection that are not yet completed or active.
    pub async fn sync_pinned_collection_delta(
        vault: &OfflineVault,
        collection: &PinnedCollection,
    ) -> CoreResult<usize> {
        let records: HashMap<String, DownloadRecord> = vault
            .list_records()?
            .into_iter()
            .map(|r| (r.video_id.clone(), r))
            .collect();

        let mut enqueued = 0usize;
        for track in &collection.tracks {
            let needs_enqueue = match records.get(&track.id).map(|r| r.status) {
                None | Some(DownloadStatus::Failed | DownloadStatus::Cancelled) => true,
                Some(
                    DownloadStatus::Queued
                    | DownloadStatus::Downloading
                    | DownloadStatus::Paused
                    | DownloadStatus::Tagging
                    | DownloadStatus::Completed,
                ) => false,
            };
            if needs_enqueue {
                vault.enqueue_track(track.clone()).await?;
                enqueued += 1;
            }
        }
        Ok(enqueued)
    }
}
