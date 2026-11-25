//! Catalog, Home shelves, Mood filters, Search, and Collection detail slice (`CatalogSlice`).

use std::collections::{HashMap, HashSet};

use nsh_core::prelude::SharedString;
use resonance_contract::{
    ExplorePage, HomeItem, HomeSection, LibraryCollectionRef, Playlist, SearchCategory,
    SearchResults, Track,
};
use resonance_core::fixture::{fixture_home_sections, fixture_tracks};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchFilter {
    #[default]
    All,
    Songs,
    Videos,
    Albums,
    Playlists,
    Artists,
    Podcasts,
}

impl SearchFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Songs => "Songs",
            Self::Videos => "Videos",
            Self::Albums => "Albums",
            Self::Playlists => "Playlists",
            Self::Artists => "Artists",
            Self::Podcasts => "Podcasts",
        }
    }

    pub fn is_collection_only(self) -> bool {
        matches!(
            self,
            Self::Albums | Self::Playlists | Self::Artists | Self::Podcasts
        )
    }

    pub fn as_search_category(self) -> SearchCategory {
        match self {
            Self::All => SearchCategory::All,
            Self::Songs | Self::Videos => SearchCategory::Songs,
            Self::Albums => SearchCategory::Albums,
            Self::Playlists => SearchCategory::Playlists,
            Self::Artists => SearchCategory::Artists,
            Self::Podcasts => SearchCategory::Podcasts,
        }
    }

    pub fn params(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Songs => Some("EgWKAQIIAWoKEAMQBBAJEAoQBQ%3D%3D"),
            Self::Videos => Some("EgWKAQIQAWoKEAMQBBAJEAoQBQ%3D%3D"),
            Self::Albums => Some("EgWKAQIYAWoKEAMQBBAJEAoQBQ%3D%3D"),
            Self::Playlists => Some("EgeKAQQoAEABagoQAxAEEAkQChAF"),
            Self::Artists => Some("EgWKAQIgAWoKEAMQBBAJEAoQBQ%3D%3D"),
            Self::Podcasts => Some("EgWKAQJQAWoKEAMQBBAJEAoQBQ%3D%3D"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarPlaylist {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub artwork_url: Option<String>,
    pub is_liked_auto: bool,
}

pub fn filter_tracks_for_mode(tracks: &[Track], filter: SearchFilter) -> Vec<Track> {
    match filter {
        SearchFilter::All | SearchFilter::Videos => tracks.to_vec(),
        SearchFilter::Songs => tracks
            .iter()
            .filter(|t| t.duration_ms > 0)
            .cloned()
            .collect(),
        SearchFilter::Albums
        | SearchFilter::Playlists
        | SearchFilter::Artists
        | SearchFilter::Podcasts => tracks
            .iter()
            .filter(|t| t.album.as_ref().is_some_and(|a| !a.title.is_empty()))
            .cloned()
            .collect(),
    }
}

#[derive(Clone)]
pub struct CatalogSlice {
    pub home_sections: Vec<HomeSection>,
    pub explore_sections: Vec<HomeSection>,
    pub explore_page: Option<ExplorePage>,
    pub mood_playlists: Vec<LibraryCollectionRef>,
    pub home_loading: bool,
    pub home_error: Option<String>,
    pub selected_mood: Option<String>,
    pub section_pages: HashMap<String, usize>,
    pub search_query: String,
    pub search_filter: SearchFilter,
    pub search_all_tracks: Vec<Track>,
    pub search_results: Vec<Track>,
    pub search_albums: Vec<Playlist>,
    pub search_suggestions: Vec<String>,
    pub search_continuation: Option<String>,
    pub search_plays: HashMap<String, String>,
    pub search_generation: u64,
    pub search_loading: bool,
    pub search_error: Option<String>,
    pub selected_collection: Option<Playlist>,
    pub collection_search_query: String,
    pub collection_sort_asc: bool,
}

impl CatalogSlice {
    pub fn new(fixture: bool) -> Self {
        let home_sections = if fixture {
            fixture_home_sections()
        } else {
            Vec::new()
        };
        let initial_search = if fixture {
            fixture_tracks()
        } else {
            Vec::new()
        };

        Self {
            home_sections,
            explore_sections: Vec::new(),
            explore_page: None,
            mood_playlists: Vec::new(),
            home_loading: !fixture,
            home_error: None,
            selected_mood: None,
            section_pages: HashMap::new(),
            search_query: String::new(),
            search_filter: SearchFilter::All,
            search_all_tracks: initial_search.clone(),
            search_results: initial_search,
            search_albums: Vec::new(),
            search_suggestions: Vec::new(),
            search_continuation: None,
            search_plays: HashMap::new(),
            search_generation: 0,
            search_loading: false,
            search_error: None,
            selected_collection: None,
            collection_search_query: String::new(),
            collection_sort_asc: false,
        }
    }

    pub fn apply_home_loaded(&mut self, res: Result<Vec<HomeSection>, String>) {
        self.home_loading = false;
        match res {
            Ok(sections) => {
                self.home_sections = sections;
                self.home_error = None;
            }
            Err(err) => {
                self.home_error = Some(err);
            }
        }
    }

    pub fn apply_explore_loaded(&mut self, res: Result<Vec<HomeSection>, String>) {
        if let Ok(sections) = res {
            self.explore_sections = sections;
        }
    }

    pub fn apply_explore_page(&mut self, res: Result<ExplorePage, String>) {
        if let Ok(page) = res {
            if !page.charts.is_empty() {
                self.explore_sections = page.charts.clone();
            }
            self.explore_page = Some(page);
        }
    }

    pub fn apply_mood_playlists(
        &mut self,
        mood: Option<String>,
        res: Result<Vec<LibraryCollectionRef>, String>,
    ) {
        self.selected_mood = mood;
        if let Ok(playlists) = res {
            self.mood_playlists = playlists;
        }
    }

    pub fn apply_search_suggestions(&mut self, prefix: &str, suggestions: Vec<String>) {
        if self.search_query == prefix {
            self.search_suggestions = suggestions;
        }
    }

    pub fn apply_search_loaded(
        &mut self,
        query: &str,
        result: Result<SearchResults, String>,
        plays: HashMap<String, String>,
    ) {
        if self.search_query != query {
            return;
        }
        self.search_loading = false;
        match result {
            Ok(res) => {
                self.search_all_tracks = res.tracks;
                self.search_albums = res.albums;
                self.search_continuation = res.continuation;
                self.search_plays = plays;
                self.search_results =
                    filter_tracks_for_mode(&self.search_all_tracks, self.search_filter);
                self.search_error = None;
            }
            Err(err) => {
                self.search_error = Some(err);
            }
        }
    }

    pub fn append_search_page(&mut self, query: &str, result: Result<SearchResults, String>) {
        if self.search_query != query {
            return;
        }
        self.search_loading = false;
        if let Ok(res) = result {
            let mut seen_tracks: HashSet<String> = self
                .search_all_tracks
                .iter()
                .map(|t| t.id.clone())
                .collect();
            for track in res.tracks {
                if !track.id.is_empty() && seen_tracks.insert(track.id.clone()) {
                    self.search_all_tracks.push(track);
                }
            }
            let mut seen_albums: HashSet<String> =
                self.search_albums.iter().map(|a| a.id.clone()).collect();
            for album in res.albums {
                if !album.id.is_empty() && seen_albums.insert(album.id.clone()) {
                    self.search_albums.push(album);
                }
            }
            self.search_continuation = res.continuation;
            self.search_results =
                filter_tracks_for_mode(&self.search_all_tracks, self.search_filter);
            self.search_error = None;
        }
    }

    pub fn set_search_filter(&mut self, filter: SearchFilter) {
        self.search_filter = filter;
        self.search_results = filter_tracks_for_mode(&self.search_all_tracks, filter);
    }

    pub fn search_result_ids(&self) -> Vec<SharedString> {
        self.search_results
            .iter()
            .map(|t| SharedString::from(t.id.clone()))
            .collect()
    }

    pub fn run_fixture_search(&mut self, query: String) {
        self.search_query = query.clone();
        self.search_loading = false;
        self.search_error = None;
        let ql = query.to_lowercase();
        let matched: Vec<Track> = fixture_tracks()
            .into_iter()
            .filter(|t| {
                ql.is_empty()
                    || t.title.to_lowercase().contains(&ql)
                    || t.artist_display().to_lowercase().contains(&ql)
                    || t.album
                        .as_ref()
                        .is_some_and(|a| a.title.to_lowercase().contains(&ql))
            })
            .collect();
        self.search_all_tracks = matched;
        self.search_albums.clear();
        self.search_results = filter_tracks_for_mode(&self.search_all_tracks, self.search_filter);
    }

    pub fn begin_live_search(&mut self, query: String) {
        self.search_query = query.clone();
        self.search_error = None;
        self.search_generation = self.search_generation.wrapping_add(1);
        if query.is_empty() {
            self.search_loading = false;
            self.search_all_tracks.clear();
            self.search_results.clear();
            self.search_albums.clear();
        } else {
            self.search_loading = true;
        }
    }

    pub fn shift_section_page(
        &mut self,
        section_id: &str,
        delta: isize,
        total_items: usize,
        page_size: usize,
    ) {
        if total_items == 0 || page_size == 0 {
            return;
        }
        let max_page = total_items.saturating_sub(1) / page_size;
        let cur = self.section_pages.get(section_id).copied().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, max_page as isize) as usize;
        self.section_pages.insert(section_id.to_owned(), next);
    }

    pub fn sidebar_playlists(&self, liked_count: usize) -> Vec<SidebarPlaylist> {
        let mut out = vec![SidebarPlaylist {
            id: "FEmusic_liked_videos".into(),
            title: "Liked Music".into(),
            subtitle: format!("Auto playlist • {liked_count} tracks"),
            artwork_url: None,
            is_liked_auto: true,
        }];
        for sec in &self.home_sections {
            for item in &sec.items {
                if let HomeItem::Collection {
                    browse_id,
                    title,
                    subtitle,
                    artwork_url,
                } = item
                    && !out.iter().any(|p| p.id == *browse_id)
                {
                    out.push(SidebarPlaylist {
                        id: browse_id.clone(),
                        title: title.clone(),
                        subtitle: subtitle.clone(),
                        artwork_url: artwork_url.clone(),
                        is_liked_auto: false,
                    });
                }
            }
        }
        out
    }

    pub fn open_collection(
        &mut self,
        fixture: bool,
        browse_id: String,
        title: String,
        subtitle: String,
        artwork_url: Option<String>,
    ) {
        self.collection_search_query.clear();
        self.collection_sort_asc = false;
        self.selected_collection = Some(Playlist {
            id: browse_id,
            title,
            subtitle: Some(subtitle),
            artwork_url,
            tracks: if fixture {
                fixture_tracks()
            } else {
                Vec::new()
            },
        });
    }

    pub fn close_collection(&mut self) {
        self.selected_collection = None;
        self.collection_search_query.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_filter_modes_narrow_fixture_results() {
        let mut slice = CatalogSlice::new(true);
        assert!(!slice.search_results.is_empty());
        slice.set_search_filter(SearchFilter::Songs);
        assert!(slice.search_results.iter().all(|t| t.duration_ms > 0));
        slice.set_search_filter(SearchFilter::Albums);
        assert!(
            slice
                .search_results
                .iter()
                .all(|t| t.album.as_ref().is_some_and(|a| !a.title.is_empty()))
        );
    }

    #[test]
    fn section_paging_clamps_to_valid_bounds() {
        let mut slice = CatalogSlice::new(true);
        slice.shift_section_page("quick", -5, 10, 4);
        assert_eq!(slice.section_pages.get("quick").copied(), Some(0));
        slice.shift_section_page("quick", 10, 10, 4);
        assert_eq!(slice.section_pages.get("quick").copied(), Some(2));
    }
}
