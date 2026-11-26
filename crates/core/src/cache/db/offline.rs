//! Durable Offline Vault repository: committed downloads + pinned albums/playlists.
//! Strictly immune to transient cache eviction (`TransientCacheRepo::clear_all_caches`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use resonance_contract::{
    AudioCodec, CoreError, CoreResult, DownloadRecord, DownloadStatus, OfflineStats,
    OfflineStorageError, OfflineStore, Track,
};
use rusqlite::{Connection, OptionalExtension, params};

pub type PinnedCollectionRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    u64,
    Vec<Track>,
);

#[derive(Clone)]
pub struct OfflineVaultRepo {
    conn: Arc<Mutex<Connection>>,
    downloads_dir: PathBuf,
}

impl OfflineVaultRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>, downloads_dir: PathBuf) -> Self {
        Self {
            conn,
            downloads_dir,
        }
    }

    /// Transitions any rows in `'downloading'` or `'tagging'` to `'paused'` on startup.
    pub fn recover_interrupted_downloads(
        &self,
        now_unix_secs: u64,
    ) -> CoreResult<Vec<DownloadRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;

        let res = (|| -> CoreResult<Vec<DownloadRecord>> {
            let mut stmt = conn
                .prepare(
                    "SELECT video_id, track_json, status, itag, codec, mime_type, relative_path,
                            bytes_downloaded, total_bytes, sha256_hex, tags_embedded,
                            created_at, updated_at, completed_at, error_message
                     FROM downloads
                     WHERE status IN ('downloading', 'tagging')",
                )
                .map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?;

            let rows = stmt.query_map([], row_to_download_record).map_err(|e| {
                CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
            })?;

            let mut recovered = Vec::new();
            for mut rec in rows.flatten() {
                rec.status = DownloadStatus::Paused;
                rec.updated_at_unix_secs = now_unix_secs;
                recovered.push(rec);
            }
            drop(stmt);

            if !recovered.is_empty() {
                conn.execute(
                    "UPDATE downloads
                     SET status = 'paused', updated_at = ?1
                     WHERE status IN ('downloading', 'tagging')",
                    params![now_unix_secs as i64],
                )
                .map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?;
            }
            Ok(recovered)
        })();

        match res {
            Ok(records) => {
                conn.execute_batch("COMMIT;").map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?;
                Ok(records)
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_pinned_collection(
        &self,
        collection_id: &str,
        kind: &str,
        title: &str,
        subtitle: Option<&str>,
        artwork_url: Option<&str>,
        tracks: &[Track],
        pinned_at_unix_secs: u64,
    ) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;

        let res = (|| -> CoreResult<()> {
            conn.execute(
                "INSERT INTO pinned_collections (collection_id, kind, title, subtitle, artwork_url, pinned_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(collection_id) DO UPDATE SET
                   kind = excluded.kind,
                   title = excluded.title,
                   subtitle = excluded.subtitle,
                   artwork_url = excluded.artwork_url,
                   pinned_at = excluded.pinned_at",
                params![
                    collection_id,
                    kind,
                    title,
                    subtitle,
                    artwork_url,
                    pinned_at_unix_secs as i64
                ],
            )
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;

            conn.execute(
                "DELETE FROM pinned_collection_tracks WHERE collection_id = ?1",
                params![collection_id],
            )
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;

            for (pos, track) in tracks.iter().enumerate() {
                let track_json = serde_json::to_string(track).map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string()))
                })?;
                conn.execute(
                    "INSERT INTO pinned_collection_tracks (collection_id, position, video_id, track_json)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![collection_id, pos as i64, track.id, track_json],
                )
                .map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?;
            }
            Ok(())
        })();

        match res {
            Ok(()) => {
                conn.execute_batch("COMMIT;").map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    pub fn list_pinned_collection_rows(&self) -> CoreResult<Vec<PinnedCollectionRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT collection_id, kind, title, subtitle, artwork_url, pinned_at
                 FROM pinned_collections
                 ORDER BY pinned_at DESC",
            )
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
        let headers: Vec<(String, String, String, Option<String>, Option<String>, u64)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get::<_, i64>(5)?.max(0) as u64,
                ))
            })
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?
            .flatten()
            .collect();
        drop(stmt);

        let mut out = Vec::with_capacity(headers.len());
        for (cid, kind, title, subtitle, artwork_url, pinned_at) in headers {
            let mut tstmt = conn
                .prepare(
                    "SELECT track_json FROM pinned_collection_tracks
                     WHERE collection_id = ?1
                     ORDER BY position ASC",
                )
                .map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?;
            let tracks: Vec<Track> = tstmt
                .query_map(params![&cid], |r| r.get::<_, String>(0))
                .map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?
                .flatten()
                .filter_map(|json| serde_json::from_str::<Track>(&json).ok())
                .collect();
            out.push((cid, kind, title, subtitle, artwork_url, pinned_at, tracks));
        }
        Ok(out)
    }

    pub fn delete_pinned_collection(&self, collection_id: &str) -> CoreResult<bool> {
        let conn = self.conn.lock().unwrap();
        let _ = conn.execute(
            "DELETE FROM pinned_collection_tracks WHERE collection_id = ?1",
            params![collection_id],
        );
        let removed = conn
            .execute(
                "DELETE FROM pinned_collections WHERE collection_id = ?1",
                params![collection_id],
            )
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
        Ok(removed > 0)
    }

    pub fn is_track_pinned(&self, video_id: &str) -> CoreResult<bool> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pinned_collection_tracks WHERE video_id = ?1",
                params![video_id],
                |r| r.get(0),
            )
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
        Ok(count > 0)
    }
}

impl OfflineStore for OfflineVaultRepo {
    fn upsert_download(&self, record: &DownloadRecord) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        let track_json = serde_json::to_string(&record.track)
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Io(e.to_string())))?;
        conn.execute(
            "INSERT INTO downloads (
                video_id, track_json, status, itag, codec, mime_type, relative_path,
                bytes_downloaded, total_bytes, sha256_hex, tags_embedded,
                created_at, updated_at, completed_at, error_message
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(video_id) DO UPDATE SET
                track_json = excluded.track_json,
                status = excluded.status,
                itag = excluded.itag,
                codec = excluded.codec,
                mime_type = excluded.mime_type,
                relative_path = excluded.relative_path,
                bytes_downloaded = excluded.bytes_downloaded,
                total_bytes = excluded.total_bytes,
                sha256_hex = excluded.sha256_hex,
                tags_embedded = excluded.tags_embedded,
                updated_at = excluded.updated_at,
                completed_at = excluded.completed_at,
                error_message = excluded.error_message",
            params![
                record.video_id,
                track_json,
                status_to_str(record.status),
                record.itag,
                codec_to_str(record.codec),
                record.mime_type,
                record.relative_path,
                record.bytes_downloaded as i64,
                record.total_bytes as i64,
                record.sha256_hex,
                if record.tags_embedded { 1i64 } else { 0i64 },
                record.created_at_unix_secs as i64,
                record.updated_at_unix_secs as i64,
                record.completed_at_unix_secs.map(|v| v as i64),
                record.error_message,
            ],
        )
        .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
        Ok(())
    }

    fn update_download_progress(
        &self,
        video_id: &str,
        status: DownloadStatus,
        bytes_downloaded: u64,
        total_bytes: u64,
        updated_at_unix_secs: u64,
    ) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE downloads
             SET status = ?1, bytes_downloaded = ?2, total_bytes = ?3, updated_at = ?4
             WHERE video_id = ?5",
            params![
                status_to_str(status),
                bytes_downloaded as i64,
                total_bytes as i64,
                updated_at_unix_secs as i64,
                video_id,
            ],
        )
        .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
        Ok(())
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
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE downloads
             SET status = 'completed',
                 relative_path = ?1,
                 bytes_downloaded = ?2,
                 total_bytes = ?2,
                 sha256_hex = ?3,
                 tags_embedded = ?4,
                 updated_at = ?5,
                 completed_at = ?5,
                 error_message = NULL
             WHERE video_id = ?6",
            params![
                relative_path,
                file_size_bytes as i64,
                sha256_hex,
                if tags_embedded { 1i64 } else { 0i64 },
                completed_at_unix_secs as i64,
                video_id,
            ],
        )
        .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
        Ok(())
    }

    fn mark_download_failed(
        &self,
        video_id: &str,
        status: DownloadStatus,
        redacted_error: Option<&str>,
        updated_at_unix_secs: u64,
    ) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE downloads
             SET status = ?1, error_message = ?2, updated_at = ?3
             WHERE video_id = ?4",
            params![
                status_to_str(status),
                redacted_error,
                updated_at_unix_secs as i64,
                video_id,
            ],
        )
        .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
        Ok(())
    }

    fn get_download(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT video_id, track_json, status, itag, codec, mime_type, relative_path,
                    bytes_downloaded, total_bytes, sha256_hex, tags_embedded,
                    created_at, updated_at, completed_at, error_message
             FROM downloads
             WHERE video_id = ?1",
            params![video_id],
            row_to_download_record,
        )
        .optional()
        .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))
    }

    fn get_completed_download(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT video_id, track_json, status, itag, codec, mime_type, relative_path,
                    bytes_downloaded, total_bytes, sha256_hex, tags_embedded,
                    created_at, updated_at, completed_at, error_message
             FROM downloads
             WHERE video_id = ?1 AND status = 'completed' AND relative_path IS NOT NULL",
            params![video_id],
            row_to_download_record,
        )
        .optional()
        .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))
    }

    fn list_downloads(&self) -> CoreResult<Vec<DownloadRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT video_id, track_json, status, itag, codec, mime_type, relative_path,
                        bytes_downloaded, total_bytes, sha256_hex, tags_embedded,
                        created_at, updated_at, completed_at, error_message
                 FROM downloads
                 ORDER BY COALESCE(completed_at, updated_at) DESC",
            )
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
        let rows = stmt
            .query_map([], row_to_download_record)
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
        let mut out = Vec::new();
        for rec in rows.flatten() {
            out.push(rec);
        }
        Ok(out)
    }

    fn delete_download_record(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN IMMEDIATE;")
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;

        let res = (|| -> CoreResult<Option<DownloadRecord>> {
            let existing = conn
                .query_row(
                    "SELECT video_id, track_json, status, itag, codec, mime_type, relative_path,
                            bytes_downloaded, total_bytes, sha256_hex, tags_embedded,
                            created_at, updated_at, completed_at, error_message
                     FROM downloads
                     WHERE video_id = ?1",
                    params![video_id],
                    row_to_download_record,
                )
                .optional()
                .map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?;

            conn.execute(
                "DELETE FROM downloads WHERE video_id = ?1",
                params![video_id],
            )
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;

            Ok(existing)
        })();

        match res {
            Ok(existing) => {
                conn.execute_batch("COMMIT;").map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?;
                Ok(existing)
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(e)
            }
        }
    }

    fn offline_stats(&self, max_bytes: u64) -> CoreResult<OfflineStats> {
        let conn = self.conn.lock().unwrap();
        let (completed_count, active_count, total_bytes): (i64, i64, i64) = conn
            .query_row(
                "SELECT
                    COALESCE(SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status IN ('queued', 'downloading', 'tagging') THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE
                        WHEN status = 'completed' THEN total_bytes
                        WHEN status IN ('queued', 'downloading', 'paused', 'tagging') THEN bytes_downloaded
                        ELSE 0
                    END), 0)
                 FROM downloads",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;

        Ok(OfflineStats {
            completed_count: completed_count.max(0) as u64,
            active_count: active_count.max(0) as u64,
            total_bytes: total_bytes.max(0) as u64,
            max_bytes,
        })
    }

    fn downloads_root(&self) -> PathBuf {
        self.downloads_dir.clone()
    }
}

fn status_to_str(status: DownloadStatus) -> &'static str {
    match status {
        DownloadStatus::Queued => "queued",
        DownloadStatus::Downloading => "downloading",
        DownloadStatus::Paused => "paused",
        DownloadStatus::Tagging => "tagging",
        DownloadStatus::Completed => "completed",
        DownloadStatus::Failed => "failed",
        DownloadStatus::Cancelled => "cancelled",
    }
}

fn str_to_status(s: &str) -> DownloadStatus {
    match s {
        "queued" => DownloadStatus::Queued,
        "downloading" => DownloadStatus::Downloading,
        "paused" => DownloadStatus::Paused,
        "tagging" => DownloadStatus::Tagging,
        "completed" => DownloadStatus::Completed,
        "failed" => DownloadStatus::Failed,
        "cancelled" => DownloadStatus::Cancelled,
        _ => DownloadStatus::Failed,
    }
}

fn codec_to_str(codec: AudioCodec) -> &'static str {
    match codec {
        AudioCodec::Opus => "opus",
        AudioCodec::Aac => "aac",
    }
}

fn str_to_codec(s: &str) -> AudioCodec {
    match s {
        "aac" => AudioCodec::Aac,
        _ => AudioCodec::Opus,
    }
}

fn row_to_download_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<DownloadRecord> {
    let video_id: String = r.get(0)?;
    let track_json: String = r.get(1)?;
    let status_str: String = r.get(2)?;
    let itag: u32 = r.get(3)?;
    let codec_str: String = r.get(4)?;
    let mime_type: String = r.get(5)?;
    let relative_path: Option<String> = r.get(6)?;
    let bytes_downloaded: i64 = r.get(7)?;
    let total_bytes: i64 = r.get(8)?;
    let sha256_hex: Option<String> = r.get(9)?;
    let tags_embedded: i64 = r.get(10)?;
    let created_at: i64 = r.get(11)?;
    let updated_at: i64 = r.get(12)?;
    let completed_at: Option<i64> = r.get(13)?;
    let error_message: Option<String> = r.get(14)?;

    let track: Track = serde_json::from_str(&track_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(DownloadRecord {
        video_id,
        track,
        status: str_to_status(&status_str),
        itag,
        codec: str_to_codec(&codec_str),
        mime_type,
        relative_path,
        bytes_downloaded: bytes_downloaded.max(0) as u64,
        total_bytes: total_bytes.max(0) as u64,
        sha256_hex,
        tags_embedded: tags_embedded != 0,
        created_at_unix_secs: created_at.max(0) as u64,
        updated_at_unix_secs: updated_at.max(0) as u64,
        completed_at_unix_secs: completed_at.map(|v| v.max(0) as u64),
        error_message,
    })
}
