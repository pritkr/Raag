//! Library, History, Offline Downloads, and Cache telemetry slice (`LibrarySlice`).

use std::collections::HashMap;

use resonance_contract::{
    AudioCodec, CacheStats, DownloadRecord, DownloadStatus, HistoryEntry, LibraryCollectionRef,
    LibrarySnapshot, Track, TrackId,
};
use resonance_core::downloads::PinnedCollection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryFilter {
    Playlists,
    Songs,
    #[default]
    LikedSongs,
    Albums,
    Artists,
    Podcasts,
    History,
    Uploads,
    Downloads,
    Settings,
}

impl LibraryFilter {
    pub fn all() -> &'static [Self] {
        &[
            Self::Playlists,
            Self::Songs,
            Self::LikedSongs,
            Self::Albums,
            Self::Artists,
            Self::Podcasts,
            Self::History,
            Self::Uploads,
            Self::Downloads,
            Self::Settings,
        ]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Playlists => "Playlists",
            Self::Songs => "Songs",
            Self::LikedSongs => "Liked songs",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Podcasts => "Podcasts",
            Self::History => "History",
            Self::Uploads => "Uploads",
            Self::Downloads => "Downloads",
            Self::Settings => "Settings",
        }
    }
}

#[derive(Clone)]
pub struct LibrarySlice {
    pub liked_tracks: Vec<Track>,
    pub playlists: Vec<LibraryCollectionRef>,
    pub albums: Vec<LibraryCollectionRef>,
    pub artists: Vec<LibraryCollectionRef>,
    pub library_loading: bool,
    pub library_hydrated_at: Option<u64>,
    pub disliked_ids: Vec<String>,
    pub history: Vec<HistoryEntry>,
    pub library_filter: LibraryFilter,
    pub library_search_query: String,
    pub cache_stats: CacheStats,
    pub downloads: HashMap<TrackId, DownloadRecord>,
    pub pinned_collections: Vec<PinnedCollection>,
}

impl LibrarySlice {
    pub fn new(fixture: bool, liked_tracks: Vec<Track>) -> Self {
        Self {
            liked_tracks,
            playlists: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            library_loading: false,
            library_hydrated_at: None,
            disliked_ids: Vec::new(),
            history: Vec::new(),
            library_filter: LibraryFilter::LikedSongs,
            library_search_query: String::new(),
            cache_stats: CacheStats {
                audio_chunk_bytes: if fixture { 42 * 1024 * 1024 } else { 0 },
                audio_chunk_count: if fixture { 168 } else { 0 },
                artwork_bytes: if fixture { 6 * 1024 * 1024 } else { 0 },
                metadata_entries: if fixture { 24 } else { 0 },
                max_audio_bytes: 1024 * 1024 * 1024,
            },
            downloads: HashMap::new(),
            pinned_collections: Vec::new(),
        }
    }

    pub fn apply_snapshot(&mut self, snap: LibrarySnapshot, current_track: Option<&mut Track>) {
        self.liked_tracks = snap.liked_songs;
        self.playlists = snap.playlists;
        self.albums = snap.albums;
        self.artists = snap.artists;
        if self.history.is_empty() && !snap.history.is_empty() {
            let ts = snap.fetched_at_unix_secs;
            self.history = snap
                .history
                .into_iter()
                .map(|track| HistoryEntry {
                    track,
                    played_at_unix_secs: ts,
                })
                .collect();
        }
        self.library_loading = false;
        self.library_hydrated_at = Some(snap.fetched_at_unix_secs);
        if let Some(cur) = current_track {
            cur.is_liked = self.liked_tracks.iter().any(|t| t.id == cur.id);
        }
    }

    pub fn record_play(&mut self, track: Track) {
        self.history.insert(
            0,
            HistoryEntry {
                track,
                played_at_unix_secs: resonance_core::unix_secs(),
            },
        );
    }

    pub fn is_track_liked(&self, track_id: &str) -> bool {
        self.liked_tracks.iter().any(|t| t.id == track_id)
    }

    pub fn clear_transient_caches(&mut self) {
        self.cache_stats.audio_chunk_bytes = 0;
        self.cache_stats.audio_chunk_count = 0;
        self.cache_stats.artwork_bytes = 0;
        self.cache_stats.metadata_entries = 0;
    }

    pub fn downloads_list(&self) -> Vec<DownloadRecord> {
        let mut list: Vec<DownloadRecord> = self.downloads.values().cloned().collect();
        list.sort_by(|a, b| {
            let ta = a.completed_at_unix_secs.unwrap_or(a.updated_at_unix_secs);
            let tb = b.completed_at_unix_secs.unwrap_or(b.updated_at_unix_secs);
            tb.cmp(&ta).then_with(|| a.track.title.cmp(&b.track.title))
        });
        list
    }

    pub fn enqueue_fixture_download(&mut self, track: Track) {
        let vid = track.id.clone();
        self.downloads.insert(
            vid.clone(),
            DownloadRecord {
                video_id: vid,
                track,
                status: DownloadStatus::Completed,
                itag: 140,
                codec: AudioCodec::Aac,
                mime_type: "audio/mp4".to_owned(),
                relative_path: None,
                bytes_downloaded: 512 * 1024,
                total_bytes: 512 * 1024,
                sha256_hex: None,
                tags_embedded: true,
                created_at_unix_secs: 0,
                updated_at_unix_secs: 0,
                completed_at_unix_secs: Some(0),
                error_message: None,
            },
        );
    }

    pub fn set_downloads(&mut self, records: Vec<DownloadRecord>) {
        self.downloads = records
            .into_iter()
            .map(|r| (r.video_id.clone(), r))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_core::fixture::fixture_tracks;

    #[test]
    fn clear_transient_caches_preserves_offline_downloads() {
        let tracks = fixture_tracks();
        let mut slice = LibrarySlice::new(true, Vec::new());
        slice.enqueue_fixture_download(tracks[0].clone());
        assert_eq!(slice.downloads_list().len(), 1);

        slice.clear_transient_caches();
        assert_eq!(slice.cache_stats.audio_chunk_bytes, 0);
        assert_eq!(slice.downloads_list().len(), 1);
    }
}
