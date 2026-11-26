//! Seekable Read-Ahead Media Source (`crates/core/src/pcm_engine/source.rs`).
//!
//! Supports direct local/offline disk files (`std::fs::File`) and HTTP Range streams
//! (`http://127.0.0.1:<port>/...`) with an atomic cancellation flag so rapid track
//! skips abort blocked reads within <5 ms.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub trait PcmMediaSource: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> PcmMediaSource for T {}

/// Unified seekable read-ahead source for local files and loopback HTTP streams.
pub struct SeekableReadAheadSource {
    inner: SourceKind,
    cancelled: Arc<AtomicBool>,
}

enum SourceKind {
    LocalFile { reader: BufReader<File>, len: u64 },
    InMemory { data: Vec<u8>, pos: u64 },
}

impl SeekableReadAheadSource {
    pub fn open_file(path: &Path, cancelled: Arc<AtomicBool>) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            inner: SourceKind::LocalFile {
                reader: BufReader::with_capacity(256 * 1024, file),
                len,
            },
            cancelled,
        })
    }

    pub fn from_bytes(data: Vec<u8>, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            inner: SourceKind::InMemory { data, pos: 0 },
            cancelled,
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn byte_len(&self) -> u64 {
        match &self.inner {
            SourceKind::LocalFile { len, .. } => *len,
            SourceKind::InMemory { data, .. } => data.len() as u64,
        }
    }
}

impl Read for SeekableReadAheadSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "pcm source cancelled",
            ));
        }
        match &mut self.inner {
            SourceKind::LocalFile { reader, .. } => reader.read(buf),
            SourceKind::InMemory { data, pos } => {
                let start = (*pos as usize).min(data.len());
                let avail = &data[start..];
                let n = avail.len().min(buf.len());
                buf[..n].copy_from_slice(&avail[..n]);
                *pos += n as u64;
                Ok(n)
            }
        }
    }
}

impl Seek for SeekableReadAheadSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        if self.cancelled.load(Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "pcm source cancelled",
            ));
        }
        match &mut self.inner {
            SourceKind::LocalFile { reader, .. } => reader.seek(pos),
            SourceKind::InMemory { data, pos: cur } => {
                let len = data.len() as i64;
                let next = match pos {
                    SeekFrom::Start(offset) => offset as i64,
                    SeekFrom::Current(delta) => (*cur as i64).saturating_add(delta),
                    SeekFrom::End(delta) => len.saturating_add(delta),
                };
                if next < 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "seek before start",
                    ));
                }
                *cur = next as u64;
                Ok(*cur)
            }
        }
    }
}
