//! Two-tier player-JS AST / `n`-challenge memoization cache (in-memory + SQLite WAL).
//!
//! Ensures repeat `n`-challenge evaluations complete in < 2 ms even across app restarts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::cache::SqliteCacheStore;
use crate::unix_secs;

pub struct EjsMemoCache {
    mem: Mutex<HashMap<(String, String), String>>,
    sqlite: Option<Arc<SqliteCacheStore>>,
}

impl Default for EjsMemoCache {
    fn default() -> Self {
        Self::new(None)
    }
}

impl EjsMemoCache {
    pub fn new(sqlite: Option<Arc<SqliteCacheStore>>) -> Self {
        Self {
            mem: Mutex::new(HashMap::new()),
            sqlite,
        }
    }

    pub fn with_sqlite(mut self, sqlite: Arc<SqliteCacheStore>) -> Self {
        self.sqlite = Some(sqlite);
        self
    }

    /// Looks up a memoized `n`-challenge result from L1 RAM, falling back to L2 SQLite
    /// and promoting hits back into L1.
    pub fn get(&self, player_hash: &str, raw_n: &str) -> Option<String> {
        let key = (player_hash.to_owned(), raw_n.to_owned());
        if let Some(hit) = self.mem.lock().unwrap().get(&key).cloned() {
            return Some(hit);
        }
        if let Some(db) = &self.sqlite
            && let Ok(Some(hit)) = db.get_ejs_memo(player_hash, raw_n)
        {
            self.mem.lock().unwrap().insert(key, hit.clone());
            return Some(hit);
        }
        None
    }

    /// Persists a solved `n`-challenge result (and optional AST function metadata) in both L1 and L2.
    pub fn insert(
        &self,
        player_hash: &str,
        raw_n: &str,
        solved_n: &str,
        ast_meta_json: Option<&str>,
    ) {
        self.mem.lock().unwrap().insert(
            (player_hash.to_owned(), raw_n.to_owned()),
            solved_n.to_owned(),
        );
        if let Some(db) = &self.sqlite {
            let _ = db.put_ejs_memo(player_hash, raw_n, solved_n, ast_meta_json, unix_secs());
        }
    }

    pub fn clear(&self) {
        self.mem.lock().unwrap().clear();
        if let Some(db) = &self.sqlite {
            let _ = db.clear_ejs_memo();
        }
    }
}
