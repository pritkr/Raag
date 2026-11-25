//! Local audio folder indexing and track metadata contracts.

use serde::{Deserialize, Serialize};

use crate::Track;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalFolderRoot {
    pub path: String,
    pub added_at_unix_secs: u64,
    pub last_scanned_unix_secs: Option<u64>,
    pub track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalTrackRecord {
    pub track_id: String,
    pub canonical_path: String,
    pub root_path: String,
    pub file_size_bytes: u64,
    pub mtime_unix_secs: u64,
    pub mime_type: String,
    pub sample_rate_hz: Option<u32>,
    pub bitrate_kbps: Option<u32>,
    pub track: Track,
    pub embedded_lyrics: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LocalScanProgress {
    pub scanning: bool,
    pub files_seen: u32,
    pub files_indexed: u32,
    pub files_skipped_unchanged: u32,
    pub files_removed_missing: u32,
    pub errors: u32,
}
