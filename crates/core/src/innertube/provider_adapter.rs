//! Clean `MusicProvider` adapter traits separating Browse/Search/Library delegation
//! (e.g. wrapping MIT-licensed `ytmapi-rs`) from Resonance's owned Playback Resolution path
//! (`ClientMatrix` + `EjsSolver` + `LoopbackProxy`).
//!
//! Composed centrally by [`crate::providers::HybridMusicProvider`].
//!
//! License & Provenance Policy:
//! - Resonance is `GPL-3.0-or-later`.
//! - `ytmapi-rs` (MIT) is compatible and recommended behind `BrowseDelegate`.
//! - Any study repositories under `_refs/` are strictly study-only and gitignored.

use async_trait::async_trait;
use resonance_contract::{
    CoreResult, HomeSection, LyricsDocument, Playlist, SearchResults, StreamInfo, Track,
};

/// Browse/metadata delegate trait (e.g. an MIT `ytmapi-rs` wrapper or InnerTube browse client).
#[async_trait]
pub trait BrowseDelegate: Send + Sync {
    async fn search(&self, query: &str) -> CoreResult<SearchResults>;
    async fn home(&self) -> CoreResult<Vec<HomeSection>>;
    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist>;
    async fn liked_songs(&self) -> CoreResult<Vec<Track>>;
    async fn autoplay_next(&self, video_id: &str) -> CoreResult<Vec<Track>>;
    async fn ytm_lyrics(&self, video_id: &str) -> CoreResult<Option<LyricsDocument>>;
}

/// Owned playback-resolution trait (data-driven `ClientMatrix` + `yt-dlp-ejs` in `rquickjs`
/// + decoder-driven format selection).
#[async_trait]
pub trait PlaybackResolver: Send + Sync {
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo>;
}
