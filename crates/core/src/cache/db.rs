//! SQLite WAL (`rusqlite` bundled) storage for SWR metadata, `visitorData`,
//! persistent Queue + Resume Position, Listening History, and chunk LRU accounting.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use resonance_contract::{
    CacheError, CacheStats, CacheStore, CoreError, CoreResult, HistoryEntry, QueueState, Track,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::cache::chunks::DEFAULT_MAX_AUDIO_CACHE_BYTES;

const SCHEMA_VERSION: i32 = 1;

pub struct SqliteCacheStore {
    conn: Mutex<Connection>,
    db_path: PathBuf,
    max_audio_bytes: u64,
}

impl SqliteCacheStore {
    pub fn open(data_dir: &Path) -> CoreResult<Self> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        let db_path = data_dir.join("resonance.db");
        let conn = Connection::open(&db_path)
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        if version < SCHEMA_VERSION {
            migrate_v1(&conn)?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
            max_audio_bytes: DEFAULT_MAX_AUDIO_CACHE_BYTES,
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

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
