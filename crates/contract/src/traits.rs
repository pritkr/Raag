//! Frozen Phase 0 trait contracts isolating core, protocol, storage, and platform media bridges.

use async_trait::async_trait;

use crate::{
    AuthError, AuthSession, CacheStats, CoreResult, DownloadRecord, DownloadStatus, HistoryEntry,
    HomeSection, LyricsDocument, MediaCommand, OfflineStats, Playlist, QueueState, SearchResults,
    SponsorSegment, StreamInfo, Track, TrackId,
};

#[async_trait]
pub trait MusicProvider: Send + Sync {
    async fn search(&self, query: &str) -> CoreResult<SearchResults>;
    async fn home(&self) -> CoreResult<Vec<HomeSection>>;
    async fn browse_collection(&self, browse_id: &str) -> CoreResult<Playlist>;
    async fn liked_songs(&self) -> CoreResult<Vec<Track>>;
    async fn autoplay_next(&self, video_id: &str) -> CoreResult<Vec<Track>>;
    async fn resolve_stream(&self, video_id: &str) -> CoreResult<StreamInfo>;
    async fn ytm_lyrics(&self, video_id: &str) -> CoreResult<Option<LyricsDocument>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowPlayingMetadata {
    pub track_id: TrackId,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub artwork_url: Option<String>,
    pub duration_ms: u64,
}

#[async_trait]
pub trait AudioBackend: Send + Sync {
    async fn load_and_play(&self, proxy_url: &str, meta: NowPlayingMetadata) -> CoreResult<()>;
    async fn play(&self) -> CoreResult<()>;
    async fn pause(&self) -> CoreResult<()>;
    async fn stop(&self) -> CoreResult<()>;
    async fn seek_ms(&self, position_ms: u64) -> CoreResult<()>;
    async fn set_volume(&self, volume: f32) -> CoreResult<()>;
    async fn position_ms(&self) -> u64;
    async fn update_now_playing(&self, meta: NowPlayingMetadata, playing: bool, position_ms: u64);
    fn take_command_receiver(&self) -> Option<tokio::sync::mpsc::UnboundedReceiver<MediaCommand>>;
}

pub trait AuthStore: Send + Sync {
    fn load_session(&self) -> Result<Option<AuthSession>, AuthError>;
    fn save_session(&self, session: &AuthSession) -> Result<(), AuthError>;
    fn clear_session(&self) -> Result<(), AuthError>;
}

pub trait CacheStore: Send + Sync {
    fn get_visitor_data(&self) -> CoreResult<Option<String>>;
    fn set_visitor_data(&self, visitor_data: &str) -> CoreResult<()>;

    fn get_metadata(&self, key: &str, now_unix_secs: u64) -> CoreResult<Option<(String, bool)>>;
    fn put_metadata(
        &self,
        key: &str,
        value_json: &str,
        ttl_secs: u64,
        now_unix_secs: u64,
    ) -> CoreResult<()>;
    fn evict_metadata(&self, key: &str) -> CoreResult<()>;

    fn load_queue(&self) -> CoreResult<Option<QueueState>>;
    fn save_queue(&self, queue: &QueueState) -> CoreResult<()>;

    fn record_history(&self, entry: &HistoryEntry) -> CoreResult<()>;
    fn load_history(&self, limit: usize) -> CoreResult<Vec<HistoryEntry>>;

    fn stats(&self) -> CoreResult<CacheStats>;
    fn clear_all_caches(&self) -> CoreResult<()>;
}

#[async_trait]
pub trait LyricsProvider: Send + Sync {
    async fn fetch_lyrics(&self, track: &Track) -> CoreResult<Option<LyricsDocument>>;
}

#[async_trait]
pub trait SponsorBlockProvider: Send + Sync {
    async fn fetch_segments(&self, video_id: &str) -> CoreResult<Vec<SponsorSegment>>;
}

pub trait OfflineStore: Send + Sync {
    fn upsert_download(&self, record: &DownloadRecord) -> CoreResult<()>;
    fn update_download_progress(
        &self,
        video_id: &str,
        status: DownloadStatus,
        bytes_downloaded: u64,
        total_bytes: u64,
        updated_at_unix_secs: u64,
    ) -> CoreResult<()>;
    fn mark_download_completed(
        &self,
        video_id: &str,
        relative_path: &str,
        file_size_bytes: u64,
        sha256_hex: &str,
        tags_embedded: bool,
        completed_at_unix_secs: u64,
    ) -> CoreResult<()>;
    fn mark_download_failed(
        &self,
        video_id: &str,
        status: DownloadStatus,
        redacted_error: Option<&str>,
        updated_at_unix_secs: u64,
    ) -> CoreResult<()>;
    fn get_download(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>>;
    fn get_completed_download(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>>;
    fn list_downloads(&self) -> CoreResult<Vec<DownloadRecord>>;
    fn delete_download_record(&self, video_id: &str) -> CoreResult<Option<DownloadRecord>>;
    fn offline_stats(&self, max_bytes: u64) -> CoreResult<OfflineStats>;
    /// Absolute canonical directory holding finalized offline media files.
    fn downloads_root(&self) -> std::path::PathBuf;
}
