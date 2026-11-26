//! Persistent app state repository: saved queue, listening history, settings, and library snapshots.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use resonance_contract::{
    CacheError, CacheStore, CoreError, CoreResult, HistoryEntry, LIBRARY_SNAPSHOT_METADATA_KEY,
    LibrarySnapshot, QueueState, SETTINGS_METADATA_KEY, SETTINGS_TTL_SECS, SettingsV1, Track,
};
use rusqlite::{Connection, params};

use super::transient::TransientCacheRepo;
use crate::unix_secs;

#[derive(Clone)]
pub struct AppStateRepo {
    conn: Arc<Mutex<Connection>>,
    kv: TransientCacheRepo,
}

impl AppStateRepo {
    pub(crate) fn new(conn: Arc<Mutex<Connection>>, kv: TransientCacheRepo) -> Self {
        Self { conn, kv }
    }

    pub fn load_queue(&self) -> CoreResult<Option<QueueState>> {
        let Some(raw) = self.kv.get_kv("persistent_queue")? else {
            return Ok(None);
        };
        match serde_json::from_str::<QueueState>(&raw) {
            Ok(q) => Ok(Some(q)),
            Err(e) => {
                log::warn!("corrupt persistent_queue ({e}); self-healing by resetting");
                let _ = self.kv.set_kv("persistent_queue", "{}");
                Ok(None)
            }
        }
    }

    pub fn save_queue(&self, queue: &QueueState) -> CoreResult<()> {
        let json = serde_json::to_string(queue)
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        self.kv.set_kv("persistent_queue", &json)
    }

    pub fn record_history(&self, entry: &HistoryEntry) -> CoreResult<()> {
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

    pub fn load_history(&self, limit: usize) -> CoreResult<Vec<HistoryEntry>> {
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
}

/// Loads persisted user settings from SQLite metadata, self-healing to defaults if corrupt.
pub fn load_persisted_settings(cache: &dyn CacheStore) -> SettingsV1 {
    let now = unix_secs();
    match cache.get_metadata(SETTINGS_METADATA_KEY, now) {
        Ok(Some((json, _))) => match serde_json::from_str::<SettingsV1>(&json) {
            Ok(mut s) => {
                s.volume = s.volume.clamp(0.0, 1.6);
                s
            }
            Err(e) => {
                log::warn!("corrupt persisted settings: {e}; falling back to defaults");
                SettingsV1::default()
            }
        },
        _ => SettingsV1::default(),
    }
}

/// Persists user settings into SQLite metadata.
pub fn save_persisted_settings(cache: &dyn CacheStore, settings: &SettingsV1) {
    if let Ok(json) = serde_json::to_string(settings) {
        let now = unix_secs();
        if let Err(e) = cache.put_metadata(SETTINGS_METADATA_KEY, &json, SETTINGS_TTL_SECS, now) {
            log::warn!("failed to persist settings: {e}");
        }
    }
}

/// Loads the last cached account library snapshot merged with local listening history.
pub fn load_cached_library_snapshot(cache: &dyn CacheStore) -> Option<LibrarySnapshot> {
    let now = unix_secs();
    let mut snap = cache
        .get_metadata(LIBRARY_SNAPSHOT_METADATA_KEY, now)
        .ok()
        .flatten()
        .and_then(|(json, _)| serde_json::from_str::<LibrarySnapshot>(&json).ok())?;

    if let Ok(local_history) = cache.load_history(200) {
        let mut seen: HashSet<(String, u64)> = snap
            .history
            .iter()
            .map(|t| (t.id.clone(), snap.fetched_at_unix_secs / 60))
            .collect();
        for entry in local_history {
            let bucket = entry.played_at_unix_secs / 60;
            if seen.insert((entry.track.id.clone(), bucket)) {
                snap.history.push(entry.track);
            }
        }
    }
    Some(snap)
}
