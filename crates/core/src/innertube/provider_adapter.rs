//! Clean `MusicProvider` adapter seam separating Browse/Search/Library delegation
//! (e.g. wrapping MIT-licensed `ytmapi-rs`) from Resonance's owned Playback Resolution path
//! (`ClientMatrix` + `EjsSolver` + `LoopbackProxy`).
//!
//! License & Provenance Policy:
//! - Resonance is `Apache-2.0`.
//! - `ytmapi-rs` (MIT) is license-compatible and recommended behind `BrowseDelegate`.
//! - GPL-licensed libraries (e.g. RustyPipe) are strictly excluded from Cargo dependencies;
//!   any study repositories under `_refs/` are strictly study-only and gitignored.

use std::sync::Arc;

use async_trait::async_trait;
use resonance_contract::{
    CoreResult, HomeSection, LyricsDocument, MusicProvider, Playlist, SearchResults, StreamInfo,
    Track,
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

/// Composes any wrapped browse provider (such as MIT `ytmapi-rs`) with Resonance's owned
/// playback-resolution engine behind the frozen `MusicProvider` contract.
pub struct CompositeMusicProvider {
    browse: Arc<dyn BrowseDelegate>,
    playback: Arc<dyn PlaybackResolver>,
}

impl CompositeMusicProvider {
    pub fn new(browse: Arc<dyn BrowseDelegate>, playback: Arc<dyn PlaybackResolver>) -> Self {
        Self { browse, playback }
    }
}

#[async_trait]
impl MusicProvider for CompositeMusicProvider {
    async fn search(&self, query: &str) -> CoreResult<SearchResults> {
        self.browse.search(query).await
    }

    async fn home(&self) -> CoreResult<Vec<HomeSection>> {
        self.browse.home().await
    }

    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist> {
        self.browse.browse_collection(browse_id).await
    }

    async fn liked_songs(&self) -> CoreResult<Vec<Track>> {
        self.browse.liked_songs().await
    }

    async fn autoplay_next(&self, video_id: &str) -> CoreResult<Vec<Track>> {
        self.browse.autoplay_next(video_id).await
    }

    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo> {
        self.playback.resolve_stream(video_id).await
    }

    async fn ytm_lyrics(&self, video_id: &str) -> CoreResult<Option<LyricsDocument>> {
        self.browse.ytm_lyrics(video_id).await
    }
}
