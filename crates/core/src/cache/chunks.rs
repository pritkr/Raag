//! Range-aware 256 KB fixed-size atomic audio chunk cache.
//!
//! Writes every chunk atomically (`.tmp` -> `fsync` -> `rename`), verifies SHA-256 on read
//! to self-heal corrupt files, and enforces a size-bounded LRU budget (default 1 GB).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use resonance_contract::{CacheError, CoreError, CoreResult};
use sha2::{Digest, Sha256};

use crate::cache::db::SqliteCacheStore;
use crate::unix_secs;

pub const CHUNK_SIZE: u64 = 256 * 1024;
pub const DEFAULT_MAX_AUDIO_CACHE_BYTES: u64 = 1024 * 1024 * 1024; // 1 GB

pub struct ChunkCache {
    root: PathBuf,
    db: Arc<SqliteCacheStore>,
    max_bytes: u64,
}

impl ChunkCache {
    pub fn new(data_dir: &Path, db: Arc<SqliteCacheStore>, max_bytes: u64) -> CoreResult<Self> {
        let root = data_dir.join("audio_chunks");
        fs::create_dir_all(&root).map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        Ok(Self {
            root,
            db,
            max_bytes,
        })
    }

    pub fn chunk_path(&self, video_id: &str, itag: u32, chunk_index: u64) -> PathBuf {
        let safe_id: String = video_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        self.root
            .join(format!("{safe_id}_{itag}"))
            .join(format!("chunk_{chunk_index:06}.bin"))
    }

    pub fn known_total_len(&self, video_id: &str, itag: u32) -> CoreResult<Option<u64>> {
        self.db.known_total_len(video_id, itag)
    }

    /// Reads a cached 256 KB chunk if present and intact.
    /// If the file on disk is truncated or fails SHA-256 verification, it self-heals by
    /// removing the corrupt file + DB row and returning `Ok(None)` so the proxy refetches it.
    pub fn read_chunk(
        &self,
        video_id: &str,
        itag: u32,
        chunk_index: u64,
    ) -> CoreResult<Option<(Vec<u8>, u64)>> {
        let now = unix_secs();
        let Some((expected_len, total_len, expected_sha)) =
            self.db.get_chunk_meta(video_id, itag, chunk_index, now)?
        else {
            return Ok(None);
        };

        let path = self.chunk_path(video_id, itag, chunk_index);
        let Ok(bytes) = fs::read(&path) else {
            let _ = self.db.delete_chunk_meta(video_id, itag, chunk_index);
            return Ok(None);
        };

        let actual_sha = hex::encode(Sha256::digest(&bytes));
        if (bytes.len() as u64) != expected_len || actual_sha != expected_sha {
            log::warn!(
                "corrupt chunk detected ({video_id} itag={itag} idx={chunk_index}); self-healing"
            );
            let _ = fs::remove_file(&path);
            let _ = self.db.delete_chunk_meta(video_id, itag, chunk_index);
            return Ok(None);
        }

        Ok(Some((bytes, total_len)))
    }

    /// Atomically persists a chunk via `.tmp` -> `fsync` -> `rename` and enforces LRU eviction.
    pub fn write_chunk(
        &self,
        video_id: &str,
        itag: u32,
        chunk_index: u64,
        data: &[u8],
        total_len: u64,
    ) -> CoreResult<()> {
        let path = self.chunk_path(video_id, itag, chunk_index);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        }

        let tmp = path.with_extension("bin.tmp");
        let mut f =
            fs::File::create(&tmp).map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        f.write_all(data)
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        f.sync_all()
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        fs::rename(&tmp, &path).map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;

        let sha_hex = hex::encode(Sha256::digest(data));
        self.db.record_chunk_meta(
            video_id,
            itag,
            chunk_index,
            data.len() as u64,
            total_len,
            &sha_hex,
            unix_secs(),
        )?;

        self.evict_if_needed()?;
        Ok(())
    }

    pub fn evict_if_needed(&self) -> CoreResult<usize> {
        let victims = self.db.lru_chunks_over_limit(self.max_bytes)?;
        let count = victims.len();
        for (vid, itag, idx) in victims {
            let path = self.chunk_path(&vid, itag, idx);
            let _ = fs::remove_file(&path);
            let _ = self.db.delete_chunk_meta(&vid, itag, idx);
        }
        Ok(count)
    }

    pub fn clear_all(&self) -> CoreResult<()> {
        if self.root.exists() {
            let _ = fs::remove_dir_all(&self.root);
            fs::create_dir_all(&self.root)
                .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        }
        self.db.clear_audio_chunk_rows()
    }
}
