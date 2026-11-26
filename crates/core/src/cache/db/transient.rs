//! Transient cache repository: SWR metadata, KV store, audio chunk LRU accounting,
//! and EJS n-challenge memoization. Isolated from durable offline vault tables.

use std::sync::{Arc, Mutex};

use resonance_contract::{CacheError, CacheStats, CoreError, CoreResult};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Clone)]
pub struct TransientCacheRepo {
    conn: Arc<Mutex<Connection>>,
    max_audio_bytes: u64,
}

impl TransientCacheRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>, max_audio_bytes: u64) -> Self {
        Self {
            conn,
            max_audio_bytes,
        }
    }

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
        conn.query_row(
            "SELECT value FROM kv_store WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))
    }

    pub fn get_metadata(&self, key: &str, now_unix_secs: u64) -> CoreResult<Option<(String, bool)>> {
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

    pub fn put_metadata(
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

    pub fn evict_metadata(&self, key: &str) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM metadata_cache WHERE cache_key = ?1",
            params![key],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
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

    /// Returns oldest chunks `(video_id, itag, chunk_index)` to evict until under `max_bytes`.
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

    pub fn stats(&self) -> CoreResult<CacheStats> {
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

    pub fn clear_all_caches(&self) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "DELETE FROM metadata_cache WHERE cache_key NOT LIKE 'settings:%'; DELETE FROM audio_chunks;",
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }
}
