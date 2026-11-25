//! Reliable 3-Tier Atomic Cache:
//! 1. Range-aware 256 KB fixed audio chunks (`.tmp -> fsync -> rename`) + LRU eviction (1 GB default)
//! 2. SQLite WAL metadata SWR cache (`user_version` schema) + persistent queue/history/visitorData
//! 3. Two-level Artwork cache (Memory LRU + atomic disk LRU, 150 MB cap, keyed by SHA256(url + size))

pub mod artwork;
pub mod chunks;
pub mod db;

pub use artwork::{ArtworkCache, DEFAULT_ARTWORK_DISK_CAP};
pub use chunks::{CHUNK_SIZE, ChunkCache, DEFAULT_MAX_AUDIO_CACHE_BYTES};
pub use db::SqliteCacheStore;
