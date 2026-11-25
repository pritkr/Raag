//! Two-level Artwork Cache:
//! - L1: In-memory LRU (`lru::LruCache`)
//! - L2: Atomic disk LRU (150 MB cap) keyed by `SHA256(url + size)`

use std::fs;
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use lru::LruCache;
use resonance_contract::{CacheError, CoreError, CoreResult};
use sha2::{Digest, Sha256};

pub const DEFAULT_ARTWORK_DISK_CAP: u64 = 150 * 1024 * 1024; // 150 MB

pub struct ArtworkCache {
    dir: PathBuf,
    mem: Mutex<LruCache<String, Vec<u8>>>,
    max_disk_bytes: u64,
}

impl ArtworkCache {
    pub fn new(data_dir: &Path, mem_entries: usize, max_disk_bytes: u64) -> CoreResult<Self> {
        let dir = data_dir.join("artwork");
        fs::create_dir_all(&dir).map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        let cap = NonZeroUsize::new(mem_entries.max(16)).unwrap();
        Ok(Self {
            dir,
            mem: Mutex::new(LruCache::new(cap)),
            max_disk_bytes,
        })
    }

    pub fn cache_key(url: &str, size: u32) -> String {
        let mut h = Sha256::new();
        h.update(url.as_bytes());
        h.update(b"::");
        h.update(size.to_string().as_bytes());
        hex::encode(h.finalize())
    }

    pub fn get(&self, url: &str, size: u32) -> Option<Vec<u8>> {
        let key = Self::cache_key(url, size);
        if let Some(bytes) = self.mem.lock().unwrap().get(&key).cloned() {
            return Some(bytes);
        }
        let path = self.dir.join(format!("{key}.img"));
        if let Ok(bytes) = fs::read(&path) {
            if !bytes.is_empty() {
                self.mem.lock().unwrap().put(key, bytes.clone());
                return Some(bytes);
            }
            // Corrupt empty file -> self-heal.
            let _ = fs::remove_file(&path);
        }
        None
    }

    pub fn put(&self, url: &str, size: u32, bytes: Vec<u8>) -> CoreResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let key = Self::cache_key(url, size);
        let path = self.dir.join(format!("{key}.img"));
        let tmp = self.dir.join(format!("{key}.img.tmp"));

        let mut f =
            fs::File::create(&tmp).map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        f.write_all(&bytes)
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        f.sync_all()
            .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        fs::rename(&tmp, &path).map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;

        self.mem.lock().unwrap().put(key, bytes);
        self.evict_disk_if_needed();
        Ok(())
    }

    pub fn disk_usage_bytes(&self) -> u64 {
        fs::read_dir(&self.dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.metadata().ok().map(|m| m.len()))
            .sum()
    }

    pub fn clear(&self) -> CoreResult<()> {
        self.mem.lock().unwrap().clear();
        if self.dir.exists() {
            let _ = fs::remove_dir_all(&self.dir);
            fs::create_dir_all(&self.dir)
                .map_err(|e| CoreError::Cache(CacheError::Io(e.to_string())))?;
        }
        Ok(())
    }

    fn evict_disk_if_needed(&self) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = entries
            .flatten()
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                let modified = meta.modified().ok()?;
                Some((e.path(), meta.len(), modified))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        if total <= self.max_disk_bytes {
            return;
        }
        files.sort_by_key(|f| f.2);
        for (path, len, _) in files {
            if total <= self.max_disk_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }
}
