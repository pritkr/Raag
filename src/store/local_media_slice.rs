//! Reactive store slice for H3.2 Local Media Folders & Indexed Tracks (`LocalMediaSlice`).

use std::collections::BTreeMap;

use resonance_contract::{LocalFolderRoot, LocalScanProgress, LocalTrackRecord, Track};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalLibrarySubTab {
    #[default]
    Tracks,
    Albums,
    Artists,
}

impl LocalLibrarySubTab {
    pub fn all() -> &'static [Self] {
        &[Self::Tracks, Self::Albums, Self::Artists]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Tracks => "Tracks",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAlbumGroup {
    pub title: String,
    pub artist: String,
    pub artwork_url: Option<String>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalArtistGroup {
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Default)]
pub struct LocalMediaSlice {
    pub roots: Vec<LocalFolderRoot>,
    pub tracks: Vec<LocalTrackRecord>,
    pub scan_progress: LocalScanProgress,
    pub sub_tab: LocalLibrarySubTab,
    pub search_query: String,
    pub last_error: Option<String>,
}

impl LocalMediaSlice {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_roots(&mut self, roots: Vec<LocalFolderRoot>) {
        self.roots = roots;
    }

    pub fn set_tracks(&mut self, tracks: Vec<LocalTrackRecord>) {
        self.tracks = tracks;
    }

    pub fn set_progress(&mut self, progress: LocalScanProgress) {
        self.scan_progress = progress;
    }

    pub fn filtered_tracks(&self) -> Vec<Track> {
        let q = self.search_query.trim().to_lowercase();
        self.tracks
            .iter()
            .filter(|rec| {
                if q.is_empty() {
                    return true;
                }
                rec.track.title.to_lowercase().contains(&q)
                    || rec.track.artist_display().to_lowercase().contains(&q)
                    || rec
                        .track
                        .album
                        .as_ref()
                        .is_some_and(|a| a.title.to_lowercase().contains(&q))
            })
            .map(|rec| rec.track.clone())
            .collect()
    }

    /// Deterministically groups indexed local tracks by `(album_title, primary_artist)`.
    pub fn album_groups(&self) -> Vec<LocalAlbumGroup> {
        let mut map: BTreeMap<(String, String), LocalAlbumGroup> = BTreeMap::new();
        for rec in &self.tracks {
            let album_title = rec
                .track
                .album
                .as_ref()
                .map(|a| a.title.clone())
                .unwrap_or_else(|| "Unknown Album".to_owned());
            let artist = rec.track.artist_display();
            let key = (album_title.clone(), artist.clone());
            let entry = map.entry(key).or_insert_with(|| LocalAlbumGroup {
                title: album_title,
                artist,
                artwork_url: rec.track.artwork_url.clone(),
                tracks: Vec::new(),
            });
            if entry.artwork_url.is_none() {
                entry.artwork_url = rec.track.artwork_url.clone();
            }
            entry.tracks.push(rec.track.clone());
        }
        map.into_values().collect()
    }

    /// Deterministically groups indexed local tracks by artist display name.
    pub fn artist_groups(&self) -> Vec<LocalArtistGroup> {
        let mut map: BTreeMap<String, LocalArtistGroup> = BTreeMap::new();
        for rec in &self.tracks {
            let name = rec.track.artist_display();
            let entry = map.entry(name.clone()).or_insert_with(|| LocalArtistGroup {
                name,
                tracks: Vec::new(),
            });
            entry.tracks.push(rec.track.clone());
        }
        map.into_values().collect()
    }
}
