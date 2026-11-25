//! Extended catalog, library snapshot, artist/album detail, and explore contracts.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{ArtistRef, CoreResult, HomeSection, Playlist, SearchResults, Track};

pub const LIBRARY_SNAPSHOT_METADATA_KEY: &str = "library:snapshot:v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchCategory {
    All,
    Songs,
    Albums,
    Artists,
    Playlists,
    Podcasts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PagedResult<T> {
    pub items: Vec<T>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoodCategory {
    pub title: String,
    pub params: String,
    pub accent_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplorePage {
    pub new_releases: Vec<Playlist>,
    pub charts: Vec<HomeSection>,
    pub moods_and_genres: Vec<MoodCategory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryCollectionRef {
    pub browse_id: String,
    pub playlist_id: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub artwork_url: Option<String>,
    pub track_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LibrarySnapshot {
    pub liked_songs: Vec<Track>,
    pub playlists: Vec<LibraryCollectionRef>,
    pub albums: Vec<LibraryCollectionRef>,
    pub artists: Vec<LibraryCollectionRef>,
    pub history: Vec<Track>,
    pub podcasts: Vec<LibraryCollectionRef>,
    pub uploads: Vec<Track>,
    pub fetched_at_unix_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumDetail {
    pub browse_id: String,
    pub audio_playlist_id: Option<String>,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub release_year: Option<String>,
    pub release_type: String,
    pub description: Option<String>,
    pub artwork_url: Option<String>,
    pub total_duration_ms: u64,
    pub tracks: Vec<Track>,
    pub other_versions: Vec<LibraryCollectionRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistDetail {
    pub channel_id: String,
    pub name: String,
    pub description: Option<String>,
    pub subscribers: Option<String>,
    pub is_subscribed: bool,
    pub artwork_url: Option<String>,
    pub radio_playlist_id: Option<String>,
    pub shuffle_playlist_id: Option<String>,
    pub top_songs: Vec<Track>,
    pub albums: Vec<LibraryCollectionRef>,
    pub singles: Vec<LibraryCollectionRef>,
    pub related_artists: Vec<LibraryCollectionRef>,
}

#[async_trait]
pub trait CatalogExtProvider: Send + Sync {
    async fn explore(&self) -> CoreResult<ExplorePage>;
    async fn mood_playlists(&self, params: &str) -> CoreResult<Vec<LibraryCollectionRef>>;
    async fn search_categorized(
        &self,
        query: &str,
        category: SearchCategory,
        continuation: Option<&str>,
    ) -> CoreResult<SearchResults>;
    async fn search_suggestions(&self, prefix: &str) -> CoreResult<Vec<String>>;
    async fn fetch_library_snapshot(&self) -> CoreResult<LibrarySnapshot>;
    async fn fetch_artist(&self, channel_id: &str) -> CoreResult<ArtistDetail>;
    async fn fetch_album(&self, browse_id: &str) -> CoreResult<AlbumDetail>;
    async fn create_playlist(
        &self,
        title: &str,
        description: &str,
        video_ids: &[String],
    ) -> CoreResult<String>;
}
