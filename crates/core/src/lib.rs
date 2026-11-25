//! Resonance Pure Rust Core:
//! - Centralized InnerTube client (`WEB_REMIX`, `VISIONOS`, `ANDROID_VR`, `TV_SAMSUNG`)
//! - Native QuickJS-NG (`rquickjs >= 0.12.0`) `n`-param / `sig` solver
//! - 3-Tier Atomic Cache (256 KB audio chunks, SQLite WAL SWR metadata, 2-level artwork LRU)
//! - In-process Loopback HTTP Streaming Proxy (`127.0.0.1`) with transparent `403` re-resolution
//! - Single Source-of-Truth `PlaybackController` + persistent queue + history + resume
//! - Synced Karaoke Lyrics cascade (YTM -> LRCLIB -> Better Lyrics)
//! - SponsorBlock (`music_offtopic`) auto-skip
//! - Deterministic `--fixture` QA dataset

pub mod audio_dsp;
pub mod cache;
pub mod downloads;
pub mod fixture;
pub mod innertube;
pub mod lyrics;
pub mod offline;
pub mod palette;
pub mod playback;
pub mod potoken;
pub mod proxy;
pub mod solver;
pub mod sponsorblock;
pub mod walking_skeleton;

use std::time::{SystemTime, UNIX_EPOCH};

pub fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
