//! SQLite schema migrations (`user_version` v1 -> v3).

use resonance_contract::{CacheError, CoreError, CoreResult, OfflineStorageError};
use rusqlite::Connection;

pub const SCHEMA_VERSION: i32 = 3;

pub fn migrate_all(conn: &Connection) -> CoreResult<()> {
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

    if version < 1 {
        migrate_v1(conn)?;
    }
    if version < 2 {
        migrate_v2(conn)?;
    }
    if version < 3 {
        migrate_v3(conn)?;
    }
    Ok(())
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
