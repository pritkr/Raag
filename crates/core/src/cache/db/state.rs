//! Persistent playback state repository: saved queue + listening history.

use std::sync::{Arc, Mutex};

use resonance_contract::{CacheError, CoreError, CoreResult, HistoryEntry, QueueState, Track};
use rusqlite::{Connection, params};

use super::transient::TransientCacheRepo;

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
