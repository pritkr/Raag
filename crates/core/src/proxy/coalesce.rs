//! Range-coalescing & single-flight chunk fetch deduplication for Proxy v2.
//!
//! Ensures overlapping or concurrent byte-range requests targeting the same
//! aligned 256 KB chunk `(video_id, itag, chunk_index)` collapse into a single
//! upstream HTTP fetch, avoiding redundant bandwidth and googlevideo throttling.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use resonance_contract::CoreResult;
use tokio::sync::{Mutex, broadcast};

use crate::cache::chunks::CHUNK_SIZE;

/// Aligned chunk key for single-flight deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChunkKey {
    pub video_id: String,
    pub itag: u32,
    pub chunk_index: u64,
}

/// Coalesced upstream fetch window covering one or more contiguous aligned chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlignedWindow {
    pub first_chunk: u64,
    pub last_chunk: u64,
    pub byte_start: u64,
    pub byte_end: u64,
}

/// Aligns an arbitrary inclusive byte range `[req_start, req_end]` to `CHUNK_SIZE`
/// boundaries and caps window span for predictable memory usage.
pub fn coalesce_byte_range(
    req_start: u64,
    req_end: u64,
    total_len: u64,
    max_chunks_per_window: u64,
) -> Vec<AlignedWindow> {
    if total_len == 0 || req_start > req_end || req_start >= total_len {
        return Vec::new();
    }
    let clamped_end = req_end.min(total_len.saturating_sub(1));
    let first_chunk = req_start / CHUNK_SIZE;
    let last_chunk = clamped_end / CHUNK_SIZE;
    let max_span = max_chunks_per_window.max(1);

    let mut windows = Vec::new();
    let mut cursor = first_chunk;
    while cursor <= last_chunk {
        let win_end = (cursor + max_span - 1).min(last_chunk);
        let byte_start = cursor * CHUNK_SIZE;
        let byte_end = ((win_end + 1) * CHUNK_SIZE - 1).min(total_len.saturating_sub(1));
        windows.push(AlignedWindow {
            first_chunk: cursor,
            last_chunk: win_end,
            byte_start,
            byte_end,
        });
        cursor = win_end + 1;
    }
    windows
}

type SharedChunkResult = Result<(Arc<Vec<u8>>, u64), String>;

/// Single-flight in-flight chunk fetch deduplicator.
pub struct ChunkCoalescer {
    in_flight: Mutex<HashMap<ChunkKey, broadcast::Sender<SharedChunkResult>>>,
    coalesced_hits: AtomicU64,
}

impl Default for ChunkCoalescer {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkCoalescer {
    pub fn new() -> Self {
        Self {
            in_flight: Mutex::new(HashMap::new()),
            coalesced_hits: AtomicU64::new(0),
        }
    }

    pub fn coalesced_hits(&self) -> u64 {
        self.coalesced_hits.load(Ordering::SeqCst)
    }

    /// Runs `fetcher` at most once for concurrent callers requesting the same `ChunkKey`.
    /// Secondary callers subscribe to the in-flight broadcast channel and receive the
    /// exact fetched chunk bytes when the leader finishes.
    pub async fn fetch_or_join<F, Fut>(
        &self,
        key: ChunkKey,
        fetcher: F,
    ) -> CoreResult<(Vec<u8>, u64)>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = CoreResult<(Vec<u8>, u64)>>,
    {
        let maybe_rx = {
            let mut guard = self.in_flight.lock().await;
            if let Some(tx) = guard.get(&key) {
                self.coalesced_hits.fetch_add(1, Ordering::SeqCst);
                Some(tx.subscribe())
            } else {
                let (tx, _rx) = broadcast::channel(4);
                guard.insert(key.clone(), tx);
                None
            }
        };

        if let Some(mut rx) = maybe_rx {
            match rx.recv().await {
                Ok(Ok((bytes, total))) => return Ok((bytes.as_ref().clone(), total)),
                Ok(Err(msg)) => {
                    return Err(resonance_contract::CoreError::Offline(msg));
                }
                Err(_) => {
                    // Leader dropped before sending; fall through and execute directly.
                }
            }
        }

        let res = fetcher().await;
        let tx = {
            let mut guard = self.in_flight.lock().await;
            guard.remove(&key)
        };
        if let Some(tx) = tx {
            match &res {
                Ok((bytes, total)) => {
                    let _ = tx.send(Ok((Arc::new(bytes.clone()), *total)));
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                }
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn coalesces_byte_ranges_into_aligned_chunk_windows() {
        let total = CHUNK_SIZE * 5 + 1000;
        let windows = coalesce_byte_range(100, CHUNK_SIZE * 4 + 50, total, 2);
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].first_chunk, 0);
        assert_eq!(windows[0].last_chunk, 1);
        assert_eq!(windows[0].byte_start, 0);
        assert_eq!(windows[0].byte_end, CHUNK_SIZE * 2 - 1);
        assert_eq!(windows[2].first_chunk, 4);
        assert_eq!(windows[2].last_chunk, 4);
    }

    #[tokio::test]
    async fn single_flight_deduplicates_concurrent_chunk_fetches() {
        let coalescer = Arc::new(ChunkCoalescer::new());
        let upstream_calls = Arc::new(AtomicUsize::new(0));
        let key = ChunkKey {
            video_id: "vid_coal".into(),
            itag: 251,
            chunk_index: 0,
        };

        let mut tasks = Vec::new();
        for _ in 0..6 {
            let c = coalescer.clone();
            let k = key.clone();
            let calls = upstream_calls.clone();
            tasks.push(tokio::spawn(async move {
                c.fetch_or_join(k, || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                    Ok((vec![42u8; 1024], 4096))
                })
                .await
                .unwrap()
            }));
        }

        for t in tasks {
            let (bytes, total) = t.await.unwrap();
            assert_eq!(bytes.len(), 1024);
            assert_eq!(total, 4096);
        }
        assert_eq!(upstream_calls.load(Ordering::SeqCst), 1);
        assert_eq!(coalescer.coalesced_hits(), 5);
    }
}
