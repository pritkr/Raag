//! SQLite WAL (`rusqlite` bundled) storage for SWR metadata, `visitorData`,
//! persistent Queue + Resume Position, Listening History, and chunk LRU accounting.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use resonance_contract::{
    AudioCodec, CacheError, CacheStats, CacheStore, CoreError, CoreResult, DownloadRecord,
    DownloadStatus, HistoryEntry, OfflineStats, OfflineStorageError, OfflineStore, QueueState,
    Track,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::cache::chunks::DEFAULT_MAX_AUDIO_CACHE_BYTES;

#[allow(dead_code)]
const SCHEMA_VERSION: i32 = 3;

pub struct SqliteCacheStore {
    conn: Mutex<Connection>,
    db_path: PathBuf,
    downloads_dir: PathBuf,
    max_audio_bytes: u64,
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

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        if version < 1 {
            migrate_v1(&conn)?;
        }
        if version < 2 {
            migrate_v2(&conn)?;
        }
        if version < 3 {
            migrate_v3(&conn)?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
            downloads_dir,
            max_audio_bytes: DEFAULT_MAX_AUDIO_CACHE_BYTES,
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_chunk_meta(
        &self,
        video_id: &str,
        itag: u32,
        chunk_index: u64,
        byte_len: u64,
        total_len: u64,
        sha256_hex: &str,
        now_unix_secs: u64,
    ) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO audio_chunks (video_id, itag, chunk_index, byte_len, total_len, sha256_hex, last_accessed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(video_id, itag, chunk_index) DO UPDATE SET
               byte_len = excluded.byte_len,
               total_len = excluded.total_len,
               sha256_hex = excluded.sha256_hex,
               last_accessed = excluded.last_accessed",
            params![
                video_id,
                itag,
                chunk_index as i64,
                byte_len as i64,
                total_len as i64,
                sha256_hex,
                now_unix_secs as i64
            ],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    pub fn get_chunk_meta(
        &self,
        video_id: &str,
        itag: u32,
        chunk_index: u64,
        now_unix_secs: u64,
    ) -> CoreResult<Option<(u64, u64, String)>> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(i64, i64, String)> = conn
            .query_row(
                "SELECT byte_len, total_len, sha256_hex FROM audio_chunks
                 WHERE video_id = ?1 AND itag = ?2 AND chunk_index = ?3",
                params![video_id, itag, chunk_index as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        if let Some((byte_len, total_len, hash)) = row {
            let _ = conn.execute(
                "UPDATE audio_chunks SET last_accessed = ?1 WHERE video_id = ?2 AND itag = ?3 AND chunk_index = ?4",
                params![now_unix_secs as i64, video_id, itag, chunk_index as i64],
            );
            return Ok(Some((byte_len as u64, total_len as u64, hash)));
        }
        Ok(None)
    }

    pub fn known_total_len(&self, video_id: &str, itag: u32) -> CoreResult<Option<u64>> {
        let conn = self.conn.lock().unwrap();
        let total: Option<i64> = conn
            .query_row(
                "SELECT total_len FROM audio_chunks WHERE video_id = ?1 AND itag = ?2 AND total_len > 0 LIMIT 1",
                params![video_id, itag],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(total.map(|t| t as u64))
    }

    pub fn delete_chunk_meta(&self, video_id: &str, itag: u32, chunk_index: u64) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM audio_chunks WHERE video_id = ?1 AND itag = ?2 AND chunk_index = ?3",
            params![video_id, itag, chunk_index as i64],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    /// Returns oldest chunks `(video_id, itag, chunk_index, byte_len)` to evict until under `max_bytes`.
    pub fn lru_chunks_over_limit(&self, max_bytes: u64) -> CoreResult<Vec<(String, u32, u64)>> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(byte_len), 0) FROM audio_chunks",
                [],
                |r| r.get(0),
            )
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        if (total as u64) <= max_bytes {
            return Ok(Vec::new());
        }

        let mut remaining = total as u64;
        let mut stmt = conn
            .prepare(
                "SELECT video_id, itag, chunk_index, byte_len FROM audio_chunks ORDER BY last_accessed ASC",
            )
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, i64>(2)? as u64,
                    r.get::<_, i64>(3)? as u64,
                ))
            })
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        let mut evict = Vec::new();
        for row in rows.flatten() {
            if remaining <= max_bytes {
                break;
            }
            remaining = remaining.saturating_sub(row.3);
            evict.push((row.0, row.1, row.2));
        }
        Ok(evict)
    }

    pub fn clear_audio_chunk_rows(&self) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM audio_chunks", [])
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    /// Saves a single arbitrary KV setting (e.g. `sponsorblock_enabled`, `theme_mode`).
    pub fn set_kv(&self, key: &str, value: &str) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO kv_store (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    pub fn get_kv(&self, key: &str) -> CoreResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT value FROM kv_store WHERE key = ?1", params![key], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))
    }

    /// Transitions any rows in `'downloading'` or `'tagging'` to `'paused'` on startup.
    pub fn recover_interrupted_downloads(&self, now_unix_secs: u64) -> CoreResult<Vec<DownloadRecord>> {
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
                .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;

            let rows = stmt
                .query_map([], row_to_download_record)
                .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;

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

    // ------------------------------------------------------------------------
    // SQLite v3: EJS Player-JS AST / n-Challenge Memoization Cache (< 2 ms)
    // ------------------------------------------------------------------------

    pub fn get_ejs_memo(&self, player_hash: &str, raw_n: &str) -> CoreResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT solved_n FROM ejs_memo_cache WHERE player_hash = ?1 AND raw_n = ?2",
            params![player_hash, raw_n],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))
    }

    pub fn put_ejs_memo(
        &self,
        player_hash: &str,
        raw_n: &str,
        solved_n: &str,
        ast_meta_json: Option<&str>,
        now_unix_secs: u64,
    ) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO ejs_memo_cache (player_hash, raw_n, solved_n, ast_meta_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(player_hash, raw_n) DO UPDATE SET
               solved_n = excluded.solved_n,
               ast_meta_json = COALESCE(excluded.ast_meta_json, ejs_memo_cache.ast_meta_json),
               updated_at = excluded.updated_at",
            params![
                player_hash,
                raw_n,
                solved_n,
                ast_meta_json,
                now_unix_secs as i64
            ],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    pub fn clear_ejs_memo(&self) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM ejs_memo_cache", [])
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    // ------------------------------------------------------------------------
    // SQLite v3: Offline Vault Pinned Albums & Playlists (Immune to LRU Clear)
    // ------------------------------------------------------------------------

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

    pub fn list_pinned_collection_rows(
        &self,
    ) -> CoreResult<Vec<(String, String, String, Option<String>, Option<String>, u64, Vec<Track>)>> {
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

fn migrate_v1(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS kv_store (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS metadata_cache (
            cache_key TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            ttl_secs INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS listening_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            track_id TEXT NOT NULL,
            track_json TEXT NOT NULL,
            played_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_history_played_at ON listening_history(played_at DESC);

        CREATE TABLE IF NOT EXISTS audio_chunks (
            video_id TEXT NOT NULL,
            itag INTEGER NOT NULL,
            chunk_index INTEGER NOT NULL,
            byte_len INTEGER NOT NULL,
            total_len INTEGER NOT NULL,
            sha256_hex TEXT NOT NULL,
            last_accessed INTEGER NOT NULL,
            PRIMARY KEY (video_id, itag, chunk_index)
        );

        CREATE INDEX IF NOT EXISTS idx_audio_chunks_lru ON audio_chunks(last_accessed ASC);

        PRAGMA user_version = 1;
        ",
    )
    .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
    Ok(())
}

fn migrate_v2(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(
        "
        BEGIN IMMEDIATE;
        CREATE TABLE IF NOT EXISTS downloads (
            video_id TEXT PRIMARY KEY,
            track_json TEXT NOT NULL,
            status TEXT NOT NULL,
            itag INTEGER NOT NULL,
            codec TEXT NOT NULL,
            mime_type TEXT NOT NULL,
            relative_path TEXT,
            bytes_downloaded INTEGER NOT NULL DEFAULT 0,
            total_bytes INTEGER NOT NULL DEFAULT 0,
            sha256_hex TEXT,
            tags_embedded INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            completed_at INTEGER,
            error_message TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_downloads_status ON downloads(status, updated_at DESC);
        PRAGMA user_version = 2;
        COMMIT;
        ",
    )
    .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
    Ok(())
}

fn migrate_v3(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(
        "
        BEGIN IMMEDIATE;
        CREATE TABLE IF NOT EXISTS ejs_memo_cache (
            player_hash TEXT NOT NULL,
            raw_n TEXT NOT NULL,
            solved_n TEXT NOT NULL,
            ast_meta_json TEXT,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (player_hash, raw_n)
        );
        CREATE INDEX IF NOT EXISTS idx_ejs_memo_updated ON ejs_memo_cache(updated_at DESC);

        CREATE TABLE IF NOT EXISTS pinned_collections (
            collection_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            title TEXT NOT NULL,
            subtitle TEXT,
            artwork_url TEXT,
            pinned_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS pinned_collection_tracks (
            collection_id TEXT NOT NULL,
            position INTEGER NOT NULL,
            video_id TEXT NOT NULL,
            track_json TEXT NOT NULL,
            PRIMARY KEY (collection_id, video_id)
        );
        CREATE INDEX IF NOT EXISTS idx_pinned_tracks_vid ON pinned_collection_tracks(video_id);

        PRAGMA user_version = 3;
        COMMIT;
        ",
    )
    .map_err(|e| CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string())))?;
    Ok(())
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

impl CacheStore for SqliteCacheStore {
    fn get_visitor_data(&self) -> CoreResult<Option<String>> {
        self.get_kv("visitor_data")
    }

    fn set_visitor_data(&self, visitor_data: &str) -> CoreResult<()> {
        self.set_kv("visitor_data", visitor_data)
    }

    fn get_metadata(&self, key: &str, now_unix_secs: u64) -> CoreResult<Option<(String, bool)>> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(String, i64, i64)> = conn
            .query_row(
                "SELECT payload_json, updated_at, ttl_secs FROM metadata_cache WHERE cache_key = ?1",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        Ok(row.map(|(json, updated, ttl)| {
            let age = now_unix_secs.saturating_sub(updated as u64);
            let is_stale = age > (ttl as u64);
            (json, is_stale)
        }))
    }

    fn put_metadata(
        &self,
        key: &str,
        value_json: &str,
        ttl_secs: u64,
        now_unix_secs: u64,
    ) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO metadata_cache (cache_key, payload_json, updated_at, ttl_secs)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(cache_key) DO UPDATE SET
               payload_json = excluded.payload_json,
               updated_at = excluded.updated_at,
               ttl_secs = excluded.ttl_secs",
            params![key, value_json, now_unix_secs as i64, ttl_secs as i64],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    fn evict_metadata(&self, key: &str) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM metadata_cache WHERE cache_key = ?1", params![key])
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    fn load_queue(&self) -> CoreResult<Option<QueueState>> {
        let Some(raw) = self.get_kv("persistent_queue")? else {
            return Ok(None);
        };
        match serde_json::from_str::<QueueState>(&raw) {
            Ok(q) => Ok(Some(q)),
            Err(e) => {
                log::warn!("corrupt persistent_queue ({e}); self-healing by resetting");
                let _ = self.set_kv("persistent_queue", "{}");
                Ok(None)
            }
        }
    }

    fn save_queue(&self, queue: &QueueState) -> CoreResult<()> {
        let json = serde_json::to_string(queue)
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        self.set_kv("persistent_queue", &json)
    }

    fn record_history(&self, entry: &HistoryEntry) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        let json = serde_json::to_string(&entry.track)
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        conn.execute(
            "INSERT INTO listening_history (track_id, track_json, played_at) VALUES (?1, ?2, ?3)",
            params![entry.track.id, json, entry.played_at_unix_secs as i64],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    fn load_history(&self, limit: usize) -> CoreResult<Vec<HistoryEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT track_json, played_at FROM listening_history ORDER BY played_at DESC LIMIT ?1",
            )
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
            })
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        let mut out = Vec::new();
        for row in rows.flatten() {
            if let Ok(track) = serde_json::from_str::<Track>(&row.0) {
                out.push(HistoryEntry {
                    track,
                    played_at_unix_secs: row.1,
                });
            }
        }
        Ok(out)
    }

    fn stats(&self) -> CoreResult<CacheStats> {
        let conn = self.conn.lock().unwrap();
        let (audio_chunk_bytes, audio_chunk_count): (i64, i64) = conn
            .query_row(
                "SELECT COALESCE(SUM(byte_len), 0), COUNT(*) FROM audio_chunks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        let metadata_entries: i64 = conn
            .query_row("SELECT COUNT(*) FROM metadata_cache", [], |r| r.get(0))
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        Ok(CacheStats {
            audio_chunk_bytes: audio_chunk_bytes as u64,
            audio_chunk_count: audio_chunk_count as u64,
            artwork_bytes: 0,
            metadata_entries: metadata_entries as u64,
            max_audio_bytes: self.max_audio_bytes,
        })
    }

    fn clear_all_caches(&self) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("DELETE FROM metadata_cache; DELETE FROM audio_chunks;")
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }
}

impl OfflineStore for SqliteCacheStore {
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

            conn.execute("DELETE FROM downloads WHERE video_id = ?1", params![video_id])
                .map_err(|e| {
                    CoreError::OfflineStorage(OfflineStorageError::Sqlite(e.to_string()))
                })?;

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
