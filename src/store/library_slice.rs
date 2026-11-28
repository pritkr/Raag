//! Library, History, Offline Downloads, and Cache telemetry slice (`LibrarySlice`).

use std::collections::{HashMap, HashSet};

use resonance_contract::{
    AudioCodec, CacheStats, DownloadRecord, DownloadStatus, HistoryEntry, HomeItem, HomeSection,
    LibraryCollectionRef, LibrarySnapshot, Track, TrackId,
};
use resonance_core::offline::PinnedCollection;

/// Case-insensitive substring matcher used across library search views.
pub fn matches_query(haystack: &str, query_lower: &str) -> bool {
    query_lower.is_empty() || haystack.to_lowercase().contains(query_lower)
}

/// Filters a list of tracks by title/artist/album substring and optionally sorts A–Z by title.
pub fn filter_and_sort_tracks(mut tracks: Vec<Track>, query: &str, sort_asc: bool) -> Vec<Track> {
    let ql = query.trim().to_lowercase();
    if !ql.is_empty() {
        tracks.retain(|t| {
            matches_query(&t.title, &ql)
                || matches_query(&t.artist_display(), &ql)
                || t.album
                    .as_ref()
                    .is_some_and(|a| matches_query(&a.title, &ql))
        });
    }
    if sort_asc {
        tracks.sort_by_key(|a| a.title.to_lowercase());
    }
    tracks
}

/// Deduplicates tracks across queue, liked songs, and home shelves preserving encounter order.
pub fn collect_unique_songs(
    queue_items: &[Track],
    liked_tracks: &[Track],
    home_sections: &[HomeSection],
) -> Vec<Track> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for track in queue_items
        .iter()
        .chain(liked_tracks.iter())
        .chain(home_sections.iter().flat_map(|s| {
            s.items.iter().filter_map(|item| match item {
                HomeItem::Track(t) => Some(t),
                HomeItem::Collection { .. } => None,
            })
        }))
    {
        if seen.insert(track.id.clone()) {
            out.push(track.clone());
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LibraryFilter {
    #[default]
    Playlists,
    Songs,
    LikedSongs,
    Albums,
    Artists,
    Podcasts,
    History,
    Uploads,
    Downloads,
    PinnedOffline,
    LocalFiles,
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
            Self::PinnedOffline => "Pinned Offline",
            Self::LocalFiles => "Local Files",
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

impl Default for LibrarySlice {
    fn default() -> Self {
        let fixture = super::is_fixture_mode();
        let liked_tracks = if fixture {
            resonance_core::fixture::fixture_tracks()
                .into_iter()
                .filter(|t| t.is_liked)
                .collect()
        } else {
            Vec::new()
        };
        Self::new(fixture, liked_tracks)
    }
}

impl LibrarySlice {
    pub fn new(fixture: bool, liked_tracks: Vec<Track>) -> Self {
        let playlists = if fixture {
            vec![
                LibraryCollectionRef {
                    browse_id: "VLPL_FIX_JO".into(),
                    title: "Late Night Vibes".into(),
                    subtitle: "You • 12 tracks".into(),
                    artwork_url: None,
                    playlist_id: Some("PL_FIX_JO".into()),
                    track_count: Some(12),
                },
                LibraryCollectionRef {
                    browse_id: "VLPL_FIX_MUSIC".into(),
                    title: "Daily Rotation".into(),
                    subtitle: "You • 34 tracks".into(),
                    artwork_url: None,
                    playlist_id: Some("PL_FIX_MUSIC".into()),
                    track_count: Some(34),
                },
                LibraryCollectionRef {
                    browse_id: "VLPL_FIX_SHORTS".into(),
                    title: "Sounds from Shorts".into(),
                    subtitle: "Auto playlist • 16 tracks".into(),
                    artwork_url: None,
                    playlist_id: Some("PL_FIX_SHORTS".into()),
                    track_count: Some(16),
                },
                LibraryCollectionRef {
                    browse_id: "VLPL_COMM_1".into(),
                    title: "Love and Heartbreak".into(),
                    subtitle: "YRF • 15 tracks".into(),
                    artwork_url: None,
                    playlist_id: Some("PL_COMM_1".into()),
                    track_count: Some(15),
                },
                LibraryCollectionRef {
                    browse_id: "VLPL_COMM_2".into(),
                    title: "Adrenaline Boosters".into(),
                    subtitle: "Sandeep Chahar • 20 tracks".into(),
                    artwork_url: None,
                    playlist_id: Some("PL_COMM_2".into()),
                    track_count: Some(20),
                },
                LibraryCollectionRef {
                    browse_id: "VLPL_COMM_3".into(),
                    title: "Indian Indie Essentials".into(),
                    subtitle: "Indie Radar • 32 tracks".into(),
                    artwork_url: None,
                    playlist_id: Some("PL_COMM_3".into()),
                    track_count: Some(32),
                },
            ]
        } else {
            Vec::new()
        };
        let albums = if fixture {
            vec![
                LibraryCollectionRef {
                    browse_id: "MPRE_FIX_1".into(),
                    title: "Nocturne Telemetry".into(),
                    subtitle: "Kaelen Vance • 2026".into(),
                    artwork_url: None,
                    playlist_id: None,
                    track_count: Some(10),
                },
                LibraryCollectionRef {
                    browse_id: "MPRE_FIX_2".into(),
                    title: "ネオン東京 EP 🎧".into(),
                    subtitle: "星野サクラ • 2026".into(),
                    artwork_url: None,
                    playlist_id: None,
                    track_count: Some(6),
                },
            ]
        } else {
            Vec::new()
        };
        let artists = if fixture {
            vec![
                LibraryCollectionRef {
                    browse_id: "UC_ART_1".into(),
                    title: "Kaelen Vance".into(),
                    subtitle: "1.4M subscribers".into(),
                    artwork_url: None,
                    playlist_id: None,
                    track_count: None,
                },
                LibraryCollectionRef {
                    browse_id: "UC_ART_ARIJIT".into(),
                    title: "Arijit Singh".into(),
                    subtitle: "652M monthly audience".into(),
                    artwork_url: None,
                    playlist_id: None,
                    track_count: None,
                },
                LibraryCollectionRef {
                    browse_id: "UC_ART_3".into(),
                    title: "星野サクラ 🌸".into(),
                    subtitle: "840K subscribers".into(),
                    artwork_url: None,
                    playlist_id: None,
                    track_count: None,
                },
            ]
        } else {
            Vec::new()
        };
        Self {
            liked_tracks,
            playlists,
            albums,
            artists,
            library_loading: false,
            library_hydrated_at: None,
            disliked_ids: Vec::new(),
            history: Vec::new(),
            library_filter: LibraryFilter::Playlists,
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

    pub fn tracks_for_filter(
        &self,
        filter: LibraryFilter,
        all_songs: Vec<Track>,
        query: &str,
        sort_asc: bool,
    ) -> Vec<Track> {
        let raw = match filter {
            LibraryFilter::LikedSongs => self.liked_tracks.clone(),
            LibraryFilter::History => self.history.iter().map(|e| e.track.clone()).collect(),
            LibraryFilter::Songs => all_songs,
            LibraryFilter::Downloads => self
                .downloads_list()
                .into_iter()
                .filter(|rec| rec.status == DownloadStatus::Completed)
                .map(|rec| rec.track)
                .collect(),
            LibraryFilter::Playlists
            | LibraryFilter::Albums
            | LibraryFilter::Artists
            | LibraryFilter::PinnedOffline
            | LibraryFilter::LocalFiles
            | LibraryFilter::Podcasts
            | LibraryFilter::Uploads => self.liked_tracks.clone(),
        };
        filter_and_sort_tracks(raw, query, sort_asc)
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
