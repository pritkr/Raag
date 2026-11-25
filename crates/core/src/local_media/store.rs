//! SQLite Migration v4 storage for `local_roots` and `local_tracks`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use resonance_contract::{
    CacheError, CoreError, CoreResult, LocalFolderRoot, LocalTrackRecord, Track,
};
use rusqlite::{Connection, OptionalExtension, params};

/// SQLite-backed store for registered local music folders and indexed tracks.
pub struct LocalMediaStore {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl LocalMediaStore {
    /// Opens `data_dir/resonance.db` and applies Migration v4 (`local_roots`, `local_tracks`).
    pub fn open(data_dir: &Path) -> CoreResult<Self> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        Self::open_db_path(&data_dir.join("resonance.db"))
    }

    /// Opens an explicit SQLite database path and applies Migration v4.
    pub fn open_db_path(db_path: &Path) -> CoreResult<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        }
        let conn = Connection::open(db_path)
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        migrate_v4(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: db_path.to_path_buf(),
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Registers a canonicalized local music folder root.
    pub fn add_root(&self, root_path: &Path, now_unix_secs: u64) -> CoreResult<LocalFolderRoot> {
        let canonical = std::fs::canonicalize(root_path)
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        if !canonical.is_dir() {
            return Err(CoreError::Cache(CacheError::Io(format!(
                "local root is not a directory: {}",
                canonical.display()
            ))));
        }
        let root_str = canonical.to_string_lossy().into_owned();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO local_roots (root_path, added_at, last_scanned_at)
             VALUES (?1, ?2, NULL)
             ON CONFLICT(root_path) DO NOTHING",
            params![root_str, now_unix_secs as i64],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        let (added_at, last_scanned): (i64, Option<i64>) = conn
            .query_row(
                "SELECT added_at, last_scanned_at FROM local_roots WHERE root_path = ?1",
                params![root_str],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        let track_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM local_tracks WHERE root_path = ?1",
                params![root_str],
                |r| r.get(0),
            )
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        Ok(LocalFolderRoot {
            path: root_str,
            added_at_unix_secs: added_at.max(0) as u64,
            last_scanned_unix_secs: last_scanned.map(|v| v.max(0) as u64),
            track_count: track_count.max(0) as u32,
        })
    }

    /// Removes a registered root and all indexed tracks belonging to it.
    pub fn remove_root(&self, root_path: &str) -> CoreResult<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn
            .transaction()
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        let removed_tracks = tx
            .execute(
                "DELETE FROM local_tracks WHERE root_path = ?1",
                params![root_path],
            )
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        tx.execute(
            "DELETE FROM local_roots WHERE root_path = ?1",
            params![root_path],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        tx.commit()
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(removed_tracks)
    }

    /// Lists all registered local roots with live track counts.
    pub fn list_roots(&self) -> CoreResult<Vec<LocalFolderRoot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT r.root_path, r.added_at, r.last_scanned_at, COUNT(t.track_id)
                 FROM local_roots r
                 LEFT JOIN local_tracks t ON t.root_path = r.root_path
                 GROUP BY r.root_path, r.added_at, r.last_scanned_at
                 ORDER BY r.added_at ASC",
            )
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        let rows = stmt
            .query_map([], |r| {
                let path: String = r.get(0)?;
                let added_at: i64 = r.get(1)?;
                let last_scanned: Option<i64> = r.get(2)?;
                let count: i64 = r.get(3)?;
                Ok(LocalFolderRoot {
                    path,
                    added_at_unix_secs: added_at.max(0) as u64,
                    last_scanned_unix_secs: last_scanned.map(|v| v.max(0) as u64),
                    track_count: count.max(0) as u32,
                })
            })
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?);
        }
        Ok(out)
    }

    /// Updates `last_scanned_at` for a root.
    pub fn mark_root_scanned(&self, root_path: &str, now_unix_secs: u64) -> CoreResult<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE local_roots SET last_scanned_at = ?2 WHERE root_path = ?1",
            params![root_path, now_unix_secs as i64],
        )
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(())
    }

    /// Fast-path lookup of `(file_size_bytes, mtime_unix_secs, track_id)` for incremental skip.
    pub fn get_file_fingerprint(
        &self,
        canonical_path: &str,
    ) -> CoreResult<Option<(u64, u64, String)>> {
        let conn = self.conn.lock().unwrap();
        let row: Option<(i64, i64, String)> = conn
            .query_row(
                "SELECT file_size_bytes, mtime_unix_secs, track_id
                 FROM local_tracks WHERE canonical_path = ?1",
                params![canonical_path],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        Ok(row.map(|(sz, mt, id)| (sz.max(0) as u64, mt.max(0) as u64, id)))
    }

    /// Batch upserts up to 100 tracks in a single SQLite transaction.
    pub fn upsert_tracks_batch(
        &self,
        records: &[(LocalTrackRecord, Option<String>)],
        now_unix_secs: u64,
    ) -> CoreResult<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().unwrap();
        for chunk in records.chunks(100) {
            let tx = conn
                .transaction()
                .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
            for (rec, art_hash) in chunk {
                let track_json = serde_json::to_string(&rec.track)
                    .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
                tx.execute(
                    "INSERT INTO local_tracks (
                        track_id, canonical_path, root_path, file_size_bytes,
                        mtime_unix_secs, mime_type, sample_rate_hz, bitrate_kbps,
                        artwork_hash, embedded_lyrics, track_json, indexed_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT(track_id) DO UPDATE SET
                        canonical_path = excluded.canonical_path,
                        root_path = excluded.root_path,
                        file_size_bytes = excluded.file_size_bytes,
                        mtime_unix_secs = excluded.mtime_unix_secs,
                        mime_type = excluded.mime_type,
                        sample_rate_hz = excluded.sample_rate_hz,
                        bitrate_kbps = excluded.bitrate_kbps,
                        artwork_hash = excluded.artwork_hash,
                        embedded_lyrics = excluded.embedded_lyrics,
                        track_json = excluded.track_json,
                        indexed_at = excluded.indexed_at",
                    params![
                        rec.track_id,
                        rec.canonical_path,
                        rec.root_path,
                        rec.file_size_bytes as i64,
                        rec.mtime_unix_secs as i64,
                        rec.mime_type,
                        rec.sample_rate_hz.map(i64::from),
                        rec.bitrate_kbps.map(i64::from),
                        art_hash.as_deref(),
                        rec.embedded_lyrics.as_deref(),
                        track_json,
                        now_unix_secs as i64,
                    ],
                )
                .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
            }
            tx.commit()
                .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        }
        Ok(())
    }

    /// Removes any `local_tracks` under `root_path` whose `canonical_path` was not seen.
    pub fn prune_missing_under_root(
        &self,
        root_path: &str,
        seen_canonical_paths: &HashSet<String>,
    ) -> CoreResult<u32> {
        let mut conn = self.conn.lock().unwrap();
        let existing: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT track_id, canonical_path FROM local_tracks WHERE root_path = ?1")
                .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
            let rows = stmt
                .query_map(params![root_path], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
            let mut list = Vec::new();
            for row in rows {
                list.push(row.map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?);
            }
            list
        };

        let to_delete: Vec<String> = existing
            .into_iter()
            .filter(|(_, path)| !seen_canonical_paths.contains(path))
            .map(|(id, _)| id)
            .collect();

        if to_delete.is_empty() {
            return Ok(0);
        }

        let tx = conn
            .transaction()
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        let mut removed = 0u32;
        for id in &to_delete {
            removed += tx
                .execute("DELETE FROM local_tracks WHERE track_id = ?1", params![id])
                .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?
                as u32;
        }
        tx.commit()
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(removed)
    }

    /// Lists all indexed local tracks ordered by title.
    pub fn list_tracks(&self) -> CoreResult<Vec<LocalTrackRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT track_id, canonical_path, root_path, file_size_bytes, mtime_unix_secs,
                        mime_type, sample_rate_hz, bitrate_kbps, embedded_lyrics, track_json
                 FROM local_tracks",
            )
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        let rows = stmt
            .query_map([], map_local_track_row)
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?);
        }
        out.sort_by(|a, b| a.track.title.cmp(&b.track.title));
        Ok(out)
    }

    /// Looks up a single indexed local track by `track_id` (`local:<16-hex>`).
    pub fn get_track_by_id(&self, track_id: &str) -> CoreResult<Option<LocalTrackRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT track_id, canonical_path, root_path, file_size_bytes, mtime_unix_secs,
                    mime_type, sample_rate_hz, bitrate_kbps, embedded_lyrics, track_json
             FROM local_tracks WHERE track_id = ?1",
            params![track_id],
            map_local_track_row,
        )
        .optional()
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))
    }

    /// Priority 0 lyrics lookup for `local:*` tracks.
    pub fn get_embedded_lyrics(&self, track_id: &str) -> CoreResult<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let raw: Option<Option<String>> = conn
            .query_row(
                "SELECT embedded_lyrics FROM local_tracks WHERE track_id = ?1",
                params![track_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
        Ok(raw.flatten().filter(|s| !s.trim().is_empty()))
    }

    /// Verifies that `candidate` (already canonicalized) resides inside a registered `local_roots` directory.
    pub fn is_path_confined_to_registered_root(&self, candidate: &Path) -> CoreResult<bool> {
        let roots = self.list_roots()?;
        for root in roots {
            if let Ok(canon_root) = std::fs::canonicalize(&root.path)
                && candidate.starts_with(&canon_root)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn map_local_track_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalTrackRecord> {
    let track_id: String = row.get(0)?;
    let canonical_path: String = row.get(1)?;
    let root_path: String = row.get(2)?;
    let file_size_bytes: i64 = row.get(3)?;
    let mtime_unix_secs: i64 = row.get(4)?;
    let mime_type: String = row.get(5)?;
    let sample_rate_hz: Option<i64> = row.get(6)?;
    let bitrate_kbps: Option<i64> = row.get(7)?;
    let embedded_lyrics: Option<String> = row.get(8)?;
    let track_json: String = row.get(9)?;
    let track: Track = serde_json::from_str(&track_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(LocalTrackRecord {
        track_id,
        canonical_path,
        root_path,
        file_size_bytes: file_size_bytes.max(0) as u64,
        mtime_unix_secs: mtime_unix_secs.max(0) as u64,
        mime_type,
        sample_rate_hz: sample_rate_hz.map(|v| v.max(0) as u32),
        bitrate_kbps: bitrate_kbps.map(|v| v.max(0) as u32),
        track,
        embedded_lyrics,
    })
}

fn migrate_v4(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS local_roots (
            root_path TEXT PRIMARY KEY,
            added_at INTEGER NOT NULL,
            last_scanned_at INTEGER
         );
         CREATE TABLE IF NOT EXISTS local_tracks (
            track_id TEXT PRIMARY KEY,
            canonical_path TEXT NOT NULL UNIQUE,
            root_path TEXT NOT NULL,
            file_size_bytes INTEGER NOT NULL,
            mtime_unix_secs INTEGER NOT NULL,
            mime_type TEXT NOT NULL,
            sample_rate_hz INTEGER,
            bitrate_kbps INTEGER,
            artwork_hash TEXT,
            embedded_lyrics TEXT,
            track_json TEXT NOT NULL,
            indexed_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_local_tracks_root ON local_tracks(root_path);
         CREATE INDEX IF NOT EXISTS idx_local_tracks_artist_album ON local_tracks(track_id);",
    )
    .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;

    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
    if version < 4 {
        conn.pragma_update(None, "user_version", 4)
            .map_err(|e| CoreError::Cache(CacheError::Sqlite(e.to_string())))?;
    }
    Ok(())
}
