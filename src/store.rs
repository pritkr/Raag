//! App-level reactive NSH store (`global::<AppState>`) bridging `crates/core`
//! to Pure Rust NSH UI signals without ever blocking the UI thread.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use nsh_core::prelude::*;
use resonance_contract::{
    AudioCodec, CacheStats, DownloadRecord, DownloadStatus, HistoryEntry, HomeSection,
    LyricsDocument, PlayerSnapshot, PlayerState, Playlist, QueueState, RepeatMode, Track, TrackId,
};
use resonance_core::fixture::{fixture_home_sections, fixture_lyrics, fixture_tracks};
use resonance_core::palette::{AmbientPalette, palette_for_track};

use crate::live_engine::{self, LiveUpdate};

static FIXTURE_MODE_FLAG: AtomicBool = AtomicBool::new(false);
static PROXY_BASE_URL: OnceLock<RwLock<String>> = OnceLock::new();

fn proxy_base_lock() -> &'static RwLock<String> {
    PROXY_BASE_URL.get_or_init(|| RwLock::new("http://127.0.0.1:0".to_owned()))
}

pub fn set_proxy_base_url(base_url: impl Into<String>) {
    if let Ok(mut guard) = proxy_base_lock().write() {
        *guard = base_url.into().trim_end_matches('/').to_owned();
    }
}

pub fn proxy_stream_url(video_id: &str) -> String {
    let base = proxy_base_lock()
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "http://127.0.0.1:0".to_owned());
    let itag = if is_fixture_mode() {
        251
    } else {
        #[cfg(target_os = "android")]
        {
            251
        }
        #[cfg(not(target_os = "android"))]
        {
            140
        }
    };
    format!("{base}/stream/{video_id}?itag={itag}")
}

pub fn enable_fixture_mode(enabled: bool) {
    FIXTURE_MODE_FLAG.store(enabled, Ordering::SeqCst);
}

pub fn is_fixture_mode() -> bool {
    FIXTURE_MODE_FLAG.load(Ordering::SeqCst)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppTab {
    #[default]
    Home,
    Explore,
    Search,
    Queue,
    NowPlaying,
    Lyrics,
    Library,
    ListenTogether,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchFilter {
    #[default]
    All,
    Songs,
    Albums,
}

impl SearchFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Songs => "Songs",
            Self::Albums => "Albums",
        }
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DesktopSidePanel {
    #[default]
    Queue,
    Lyrics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarPlaylist {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub artwork_url: Option<String>,
    pub is_liked_auto: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub active_tab: AppTab,
    pub nav_history: Vec<AppTab>,
    pub nav_pos: usize,
    pub sidebar_collapsed: bool,
    pub desktop_side_panel_open: bool,
    pub theme_choice: ThemeChoice,
    pub reduced_motion: bool,
    pub player: PlayerSnapshot,
    pub home_sections: Vec<HomeSection>,
    pub home_loading: bool,
    pub home_error: Option<String>,
    pub selected_mood: Option<String>,
    pub section_pages: HashMap<String, usize>,
    pub search_query: String,
    pub search_filter: SearchFilter,
    pub search_all_tracks: Vec<Track>,
    pub search_results: Vec<Track>,
    pub search_loading: bool,
    pub search_error: Option<String>,
    pub desktop_side_panel: DesktopSidePanel,
    pub lyrics: Option<LyricsDocument>,
    pub liked_tracks: Vec<Track>,
    pub disliked_ids: Vec<String>,
    pub history: Vec<HistoryEntry>,
    pub library_filter: LibraryFilter,
    pub library_search_query: String,
    pub collection_search_query: String,
    pub collection_sort_asc: bool,
    pub cache_stats: CacheStats,
    pub downloads: HashMap<TrackId, DownloadRecord>,
    pub signed_in: bool,
    pub account_name: Option<String>,
    pub status_banner: Option<String>,
    pub selected_collection: Option<Playlist>,
    pub seek_generation: u64,
    pub nsh_audio_available: bool,
}

fn filter_tracks_for_mode(tracks: &[Track], filter: SearchFilter) -> Vec<Track> {
    match filter {
        SearchFilter::All => tracks.to_vec(),
        SearchFilter::Songs => tracks
            .iter()
            .filter(|t| t.duration_ms > 0)
            .cloned()
            .collect(),
        SearchFilter::Albums => tracks
            .iter()
            .filter(|t| t.album.as_ref().is_some_and(|a| !a.title.is_empty()))
            .cloned()
            .collect(),
    }
}

impl Default for AppState {
    fn default() -> Self {
        let fixture = is_fixture_mode();
        let tracks = if fixture {
            fixture_tracks()
        } else {
            Vec::new()
        };
        let home_sections = if fixture {
            fixture_home_sections()
        } else {
            Vec::new()
        };
        let current_track = tracks.first().cloned();
        let active_proxy_url = current_track.as_ref().map(|t| proxy_stream_url(&t.id));
        let lyrics = if fixture {
            current_track.as_ref().map(|t| fixture_lyrics(&t.id))
        } else {
            None
        };
        let liked_tracks = tracks.iter().filter(|t| t.is_liked).cloned().collect();
        let duration_ms = current_track.as_ref().map_or(0, |t| t.duration_ms);
        let initial_search = if fixture { fixture_tracks() } else { Vec::new() };

        Self {
            active_tab: AppTab::Home,
            nav_history: vec![AppTab::Home],
            nav_pos: 0,
            sidebar_collapsed: false,
            desktop_side_panel_open: true,
            theme_choice: ThemeChoice::System,
            reduced_motion: false,
            player: PlayerSnapshot {
                state: if current_track.is_some() {
                    PlayerState::Playing
                } else {
                    PlayerState::Idle
                },
                current_track,
                position_ms: if fixture { 26_000 } else { 0 },
                duration_ms,
                buffered_ms: duration_ms,
                volume: 1.0,
                queue: QueueState {
                    items: tracks,
                    current_index: if fixture { Some(0) } else { None },
                    shuffle: false,
                    repeat: RepeatMode::None,
                    autoplay_radio: true,
                    resume_position_ms: if fixture { 26_000 } else { 0 },
                },
                active_proxy_url,
                sponsorblock_enabled: true,
                last_skipped_segment: None,
                offline_mode: false,
            },
            home_sections,
            home_loading: !fixture,
            home_error: None,
            selected_mood: None,
            section_pages: HashMap::new(),
            search_query: String::new(),
            search_filter: SearchFilter::All,
            search_all_tracks: initial_search.clone(),
            search_results: initial_search,
            search_loading: false,
            search_error: None,
            desktop_side_panel: DesktopSidePanel::Queue,
            lyrics,
            liked_tracks,
            disliked_ids: Vec::new(),
            history: Vec::new(),
            library_filter: LibraryFilter::LikedSongs,
            library_search_query: String::new(),
            collection_search_query: String::new(),
            collection_sort_asc: false,
            cache_stats: CacheStats {
                audio_chunk_bytes: if fixture { 42 * 1024 * 1024 } else { 0 },
                audio_chunk_count: if fixture { 168 } else { 0 },
                artwork_bytes: if fixture { 6 * 1024 * 1024 } else { 0 },
                metadata_entries: if fixture { 24 } else { 0 },
                max_audio_bytes: 1024 * 1024 * 1024,
            },
            downloads: HashMap::new(),
            signed_in: false,
            account_name: None,
            status_banner: None,
            selected_collection: None,
            seek_generation: 0,
            nsh_audio_available: false,
        }
    }
}

fn state_signal() -> Signal<AppState> {
    global::<AppState>()
}

/// Mounts the non-blocking background live-engine update pump when NOT running in `--fixture` mode.
pub fn mount_live_engine_bridge() {
    if is_fixture_mode() {
        return;
    }
    live_engine::dispatch_load_home();
    spawn(async {
        loop {
            let updates = live_engine::try_drain_updates();
            for update in updates {
                apply_live_update(update);
            }
            timer(Duration::from_millis(80)).await;
        }
    });
}

fn apply_live_update(update: LiveUpdate) {
    match update {
        LiveUpdate::Ready {
            proxy_base_url,
            snapshot,
            cache_stats,
            signed_in,
        } => {
            set_proxy_base_url(proxy_base_url);
            state_signal().update(|s| {
                if s.player.current_track.is_none() && snapshot.current_track.is_some() {
                    s.player = snapshot;
                }
                s.cache_stats = cache_stats;
                s.signed_in = signed_in;
                if signed_in && s.account_name.is_none() {
                    s.account_name = Some("Google Account (SAPISIDHASH)".into());
                }
            });
        }
        LiveUpdate::HomeLoaded(res) => {
            state_signal().update(|s| {
                s.home_loading = false;
                match res {
                    Ok(sections) => {
                        s.home_sections = sections;
                        s.home_error = None;
                    }
                    Err(err) => {
                        s.home_error = Some(err);
                    }
                }
            });
        }
        LiveUpdate::SearchLoaded { query, result } => {
            state_signal().update(|s| {
                if s.search_query != query {
                    return;
                }
                s.search_loading = false;
                match result {
                    Ok(res) => {
                        s.search_all_tracks = res.tracks;
                        s.search_results =
                            filter_tracks_for_mode(&s.search_all_tracks, s.search_filter);
                        s.search_error = None;
                    }
                    Err(err) => {
                        s.search_error = Some(err);
                    }
                }
            });
        }
        LiveUpdate::CollectionLoaded(res) => {
            state_signal().update(|s| match res {
                Ok(playlist) => {
                    s.selected_collection = Some(playlist);
                }
                Err(err) => {
                    s.status_banner = Some(format!("Collection load failed: {err}"));
                }
            });
        }
        LiveUpdate::PlaybackUpdated(snapshot) => {
            state_signal().update(|s| {
                s.player = snapshot;
            });
        }
        LiveUpdate::LyricsLoaded(doc) => {
            state_signal().update(|s| {
                s.lyrics = doc;
            });
        }
        LiveUpdate::CacheStatsUpdated(stats) => {
            state_signal().update(|s| {
                s.cache_stats = stats;
            });
        }
        LiveUpdate::AuthUpdated { signed_in, banner } => {
            state_signal().update(|s| {
                s.signed_in = signed_in;
                s.account_name = if signed_in {
                    Some("Google Account (SAPISIDHASH)".into())
                } else {
                    None
                };
                s.status_banner = Some(banner);
            });
        }
        LiveUpdate::DownloadsLoaded(records) => {
            state_signal().update(|s| {
                s.downloads = records
                    .into_iter()
                    .map(|r| (r.video_id.clone(), r))
                    .collect();
            });
        }
        LiveUpdate::DownloadUpdated(record) => {
            state_signal().update(|s| {
                s.downloads.insert(record.video_id.clone(), record);
            });
        }
        LiveUpdate::DownloadRemoved(video_id) => {
            state_signal().update(|s| {
                s.downloads.remove(&video_id);
            });
        }
        LiveUpdate::StatusBanner(msg) => {
            state_signal().update(|s| {
                s.status_banner = Some(msg);
            });
        }
    }
}

/// Returns the cached artwork-driven ambient palette for the currently playing track.
pub fn active_ambient_palette() -> AmbientPalette {
    state_signal().with(|s| match s.player.current_track.as_ref() {
        Some(t) => palette_for_track(&t.id, &t.title),
        None => AmbientPalette::default(),
    })
}

pub fn active_tab() -> AppTab {
    state_signal().with(|s| s.active_tab)
}

pub fn set_tab(tab: AppTab) {
    state_signal().update(|s| {
        if s.active_tab != tab {
            s.nav_history.truncate(s.nav_pos + 1);
            s.nav_history.push(tab);
            s.nav_pos = s.nav_history.len().saturating_sub(1);
            s.active_tab = tab;
        }
        if tab != AppTab::Home {
            // Keep selected collection only when staying on Home or returning to it.
        }
    });
}

pub fn can_go_back() -> bool {
    state_signal().with(|s| s.selected_collection.is_some() || s.nav_pos > 0)
}

pub fn can_go_forward() -> bool {
    state_signal().with(|s| s.nav_pos + 1 < s.nav_history.len())
}

pub fn go_back() {
    state_signal().update(|s| {
        if s.active_tab == AppTab::Home && s.selected_collection.is_some() {
            s.selected_collection = None;
            return;
        }
        if s.nav_pos > 0 {
            s.nav_pos -= 1;
            s.active_tab = s.nav_history[s.nav_pos];
        }
    });
}

pub fn go_forward() {
    state_signal().update(|s| {
        if s.nav_pos + 1 < s.nav_history.len() {
            s.nav_pos += 1;
            s.active_tab = s.nav_history[s.nav_pos];
        }
    });
}

pub fn sidebar_collapsed() -> bool {
    state_signal().with(|s| s.sidebar_collapsed)
}

pub fn toggle_sidebar() {
    state_signal().update(|s| s.sidebar_collapsed = !s.sidebar_collapsed);
}

pub fn desktop_side_panel_open() -> bool {
    state_signal().with(|s| s.desktop_side_panel_open)
}

pub fn toggle_desktop_side_panel(panel: DesktopSidePanel) {
    state_signal().update(|s| {
        if s.desktop_side_panel_open && s.desktop_side_panel == panel {
            s.desktop_side_panel_open = false;
        } else {
            s.desktop_side_panel = panel;
            s.desktop_side_panel_open = true;
        }
    });
}

pub fn theme_choice() -> ThemeChoice {
    state_signal().with(|s| s.theme_choice)
}

pub fn set_theme_choice(choice: ThemeChoice) {
    state_signal().update(|s| s.theme_choice = choice);
}

pub fn reduced_motion() -> bool {
    state_signal().with(|s| s.reduced_motion)
}

pub fn set_reduced_motion(enabled: bool) {
    state_signal().update(|s| s.reduced_motion = enabled);
}

pub fn desktop_side_panel() -> DesktopSidePanel {
    state_signal().with(|s| s.desktop_side_panel)
}

pub fn set_desktop_side_panel(panel: DesktopSidePanel) {
    state_signal().update(|s| {
        s.desktop_side_panel = panel;
        s.desktop_side_panel_open = true;
    });
}

pub fn player_snapshot() -> PlayerSnapshot {
    state_signal().with(|s| s.player.clone())
}

pub fn home_sections() -> Vec<HomeSection> {
    state_signal().with(|s| s.home_sections.clone())
}

pub fn is_home_loading() -> bool {
    state_signal().with(|s| s.home_loading)
}

pub fn home_error() -> Option<String> {
    state_signal().with(|s| s.home_error.clone())
}

pub fn selected_mood() -> Option<String> {
    state_signal().with(|s| s.selected_mood.clone())
}

pub fn select_mood(mood: &str) {
    let toggle_off = state_signal().with(|s| s.selected_mood.as_deref() == Some(mood));
    state_signal().update(|s| {
        s.selected_mood = if toggle_off {
            None
        } else {
            Some(mood.to_owned())
        };
    });
    if !toggle_off {
        run_search(mood);
        set_tab(AppTab::Search);
    }
}

pub fn section_page(section_id: &str) -> usize {
    state_signal().with(|s| s.section_pages.get(section_id).copied().unwrap_or(0))
}

pub fn shift_section_page(section_id: &str, delta: isize, total_items: usize, page_size: usize) {
    if total_items == 0 || page_size == 0 {
        return;
    }
    let max_page = total_items.saturating_sub(1) / page_size;
    state_signal().update(|s| {
        let cur = s.section_pages.get(section_id).copied().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, max_page as isize) as usize;
        s.section_pages.insert(section_id.to_owned(), next);
    });
}

pub fn sidebar_playlists() -> Vec<SidebarPlaylist> {
    state_signal().with(|s| {
        let mut out = vec![SidebarPlaylist {
            id: "FEmusic_liked_videos".into(),
            title: "Liked Music".into(),
            subtitle: format!("Auto playlist • {} tracks", s.liked_tracks.len()),
            artwork_url: None,
            is_liked_auto: true,
        }];
        for sec in &s.home_sections {
            for item in &sec.items {
                if let resonance_contract::HomeItem::Collection {
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
    })
}

pub fn refresh_home() {
    if is_fixture_mode() {
        state_signal().update(|s| {
            s.home_sections = fixture_home_sections();
            s.home_loading = false;
            s.home_error = None;
        });
    } else {
        state_signal().update(|s| {
            s.home_loading = true;
            s.home_error = None;
        });
        live_engine::dispatch_load_home();
    }
}

pub fn search_query() -> String {
    state_signal().with(|s| s.search_query.clone())
}

pub fn search_filter() -> SearchFilter {
    state_signal().with(|s| s.search_filter)
}

pub fn set_search_filter(filter: SearchFilter) {
    state_signal().update(|s| {
        s.search_filter = filter;
        s.search_results = filter_tracks_for_mode(&s.search_all_tracks, filter);
    });
}

pub fn search_results_len() -> usize {
    state_signal().with(|s| s.search_results.len())
}

pub fn search_result_at(index: usize) -> Option<Track> {
    state_signal().with(|s| s.search_results.get(index).cloned())
}

pub fn search_result_ids() -> Vec<SharedString> {
    state_signal().with(|s| {
        s.search_results
            .iter()
            .map(|t| SharedString::from(t.id.clone()))
            .collect()
    })
}

pub fn is_search_loading() -> bool {
    state_signal().with(|s| s.search_loading)
}

pub fn search_error() -> Option<String> {
    state_signal().with(|s| s.search_error.clone())
}

pub fn run_search(query: &str) {
    let q = query.trim().to_owned();
    if is_fixture_mode() {
        state_signal().update(|s| {
            s.search_query = q.clone();
            s.search_loading = false;
            s.search_error = None;
            let ql = q.to_lowercase();
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
            s.search_all_tracks = matched;
            s.search_results = filter_tracks_for_mode(&s.search_all_tracks, s.search_filter);
        });
    } else {
        state_signal().update(|s| {
            s.search_query = q.clone();
            s.search_error = None;
            if q.is_empty() {
                s.search_loading = false;
                s.search_all_tracks.clear();
                s.search_results.clear();
            } else {
                s.search_loading = true;
            }
        });
        if !q.is_empty() {
            live_engine::dispatch_search(q);
        }
    }
}

pub fn queue_len() -> usize {
    state_signal().with(|s| s.player.queue.items.len())
}

pub fn queue_track_at(index: usize) -> Option<Track> {
    state_signal().with(|s| s.player.queue.items.get(index).cloned())
}

pub fn queue_ids() -> Vec<SharedString> {
    state_signal().with(|s| {
        s.player
            .queue
            .items
            .iter()
            .enumerate()
            .map(|(i, t)| SharedString::from(format!("{}-{i}", t.id)))
            .collect()
    })
}

pub fn current_lyrics() -> Option<LyricsDocument> {
    state_signal().with(|s| s.lyrics.clone())
}

pub fn liked_tracks() -> Vec<Track> {
    state_signal().with(|s| s.liked_tracks.clone())
}

pub fn is_track_liked(track_id: &str) -> bool {
    state_signal().with(|s| s.liked_tracks.iter().any(|t| t.id == track_id))
}

pub fn is_current_disliked() -> bool {
    state_signal().with(|s| {
        s.player
            .current_track
            .as_ref()
            .is_some_and(|t| s.disliked_ids.contains(&t.id))
    })
}

pub fn toggle_dislike_current() {
    state_signal().update(|s| {
        if let Some(cur) = s.player.current_track.as_mut() {
            if let Some(pos) = s.disliked_ids.iter().position(|id| id == &cur.id) {
                s.disliked_ids.remove(pos);
            } else {
                s.disliked_ids.push(cur.id.clone());
                cur.is_liked = false;
                s.liked_tracks.retain(|t| t.id != cur.id);
                s.status_banner = Some(format!("Less like \"{}\"", cur.title));
            }
        }
    });
}

pub fn library_filter() -> LibraryFilter {
    state_signal().with(|s| s.library_filter)
}

pub fn set_library_filter(filter: LibraryFilter) {
    state_signal().update(|s| s.library_filter = filter);
}

pub fn library_search_query() -> String {
    state_signal().with(|s| s.library_search_query.clone())
}

pub fn set_library_search_query(q: &str) {
    state_signal().update(|s| s.library_search_query = q.to_owned());
}

pub fn collection_search_query() -> String {
    state_signal().with(|s| s.collection_search_query.clone())
}

pub fn set_collection_search_query(q: &str) {
    state_signal().update(|s| s.collection_search_query = q.to_owned());
}

pub fn collection_sort_asc() -> bool {
    state_signal().with(|s| s.collection_sort_asc)
}

pub fn toggle_collection_sort() {
    state_signal().update(|s| s.collection_sort_asc = !s.collection_sort_asc);
}

pub fn history_entries() -> Vec<HistoryEntry> {
    state_signal().with(|s| s.history.clone())
}

pub fn cache_stats() -> CacheStats {
    state_signal().with(|s| s.cache_stats)
}

pub fn auth_info() -> (bool, Option<String>) {
    state_signal().with(|s| (s.signed_in, s.account_name.clone()))
}

pub fn status_banner() -> Option<String> {
    state_signal().with(|s| s.status_banner.clone())
}

pub fn set_status_banner(msg: impl Into<String>) {
    let text = msg.into();
    state_signal().update(|s| {
        s.status_banner = Some(text);
    });
}

pub fn play_track(track: Track) {
    let live = !is_fixture_mode();
    let track_for_live = track.clone();
    state_signal().update(|s| {
        let pos = s
            .player
            .queue
            .items
            .iter()
            .position(|t| t.id == track.id)
            .unwrap_or_else(|| {
                s.player.queue.items.insert(0, track.clone());
                0
            });
        s.player.queue.current_index = Some(pos);
        s.player.current_track = Some(track.clone());
        s.player.duration_ms = track.duration_ms;
        s.player.position_ms = 0;
        s.player.state = if live {
            PlayerState::Loading
        } else {
            PlayerState::Playing
        };
        s.player.active_proxy_url = Some(proxy_stream_url(&track.id));
        s.lyrics = if live {
            None
        } else {
            Some(fixture_lyrics(&track.id))
        };
        s.history.insert(
            0,
            HistoryEntry {
                track,
                played_at_unix_secs: resonance_core::unix_secs(),
            },
        );
    });
    if live {
        live_engine::dispatch_play_track(track_for_live);
    }
}

pub fn play_all_shuffled(mut tracks: Vec<Track>) {
    if tracks.is_empty() {
        return;
    }
    tracks.reverse();
    let first = tracks.remove(0);
    state_signal().update(|s| {
        s.player.queue.shuffle = true;
    });
    play_track(first);
    for extra in tracks {
        play_last(extra);
    }
}

pub fn play_next(track: Track) {
    state_signal().update(|s| {
        let idx = s
            .player
            .queue
            .current_index
            .map(|i| (i + 1).min(s.player.queue.items.len()))
            .unwrap_or(0);
        s.player.queue.items.insert(idx, track.clone());
        s.status_banner = Some(format!("Added \"{}\" to Play Next", track.title));
    });
}

pub fn play_last(track: Track) {
    state_signal().update(|s| {
        s.player.queue.items.push(track.clone());
        s.status_banner = Some(format!("Appended \"{}\" to end of queue", track.title));
    });
}

pub fn selected_collection() -> Option<Playlist> {
    state_signal().with(|s| s.selected_collection.clone())
}

pub fn open_collection(
    browse_id: String,
    title: String,
    subtitle: String,
    artwork_url: Option<String>,
) {
    set_tab(AppTab::Home);
    state_signal().update(|s| {
        s.collection_search_query.clear();
        s.collection_sort_asc = false;
    });
    if is_fixture_mode() {
        state_signal().update(|s| {
            s.selected_collection = Some(Playlist {
                id: browse_id,
                title,
                subtitle: Some(subtitle),
                artwork_url,
                tracks: fixture_tracks(),
            });
        });
    } else {
        state_signal().update(|s| {
            s.selected_collection = Some(Playlist {
                id: browse_id.clone(),
                title,
                subtitle: Some(subtitle),
                artwork_url,
                tracks: Vec::new(),
            });
        });
        live_engine::dispatch_browse_collection(browse_id);
    }
}

pub fn close_collection() {
    state_signal().update(|s| {
        s.selected_collection = None;
        s.collection_search_query.clear();
    });
}

pub fn toggle_play_pause() {
    state_signal().update(|s| {
        if s.player.current_track.is_none() {
            return;
        }
        s.player.state = if s.player.state.is_playing() {
            PlayerState::Paused
        } else {
            PlayerState::Playing
        };
    });
}

pub fn seek_ms(target_ms: u64) {
    state_signal().update(|s| {
        let clamped = if s.player.duration_ms > 0 {
            target_ms.min(s.player.duration_ms)
        } else {
            target_ms
        };
        s.player.position_ms = clamped;
        s.player.queue.resume_position_ms = clamped;
        s.seek_generation = s.seek_generation.wrapping_add(1);
    });
}

pub fn next_track() {
    let live = !is_fixture_mode();
    let mut dispatch: Option<Track> = None;
    state_signal().update(|s| {
        let len = s.player.queue.items.len();
        if len == 0 {
            return;
        }
        let cur = s.player.queue.current_index.unwrap_or(0);
        let next = match s.player.queue.repeat {
            RepeatMode::One => cur,
            RepeatMode::All => (cur + 1) % len,
            RepeatMode::None => (cur + 1).min(len - 1),
        };
        s.player.queue.current_index = Some(next);
        if let Some(t) = s.player.queue.items.get(next).cloned() {
            s.player.duration_ms = t.duration_ms;
            s.player.position_ms = 0;
            s.player.state = if live {
                PlayerState::Loading
            } else {
                PlayerState::Playing
            };
            s.player.active_proxy_url = Some(proxy_stream_url(&t.id));
            s.lyrics = if live {
                None
            } else {
                Some(fixture_lyrics(&t.id))
            };
            dispatch = Some(t.clone());
            s.player.current_track = Some(t);
        }
    });
    if live && let Some(t) = dispatch {
        live_engine::dispatch_play_track(t);
    }
}

pub fn previous_track() {
    let live = !is_fixture_mode();
    let mut dispatch: Option<Track> = None;
    state_signal().update(|s| {
        if s.player.position_ms > 3_000 {
            s.player.position_ms = 0;
            s.player.queue.resume_position_ms = 0;
            s.seek_generation = s.seek_generation.wrapping_add(1);
            return;
        }
        let len = s.player.queue.items.len();
        if len == 0 {
            return;
        }
        let cur = s.player.queue.current_index.unwrap_or(0);
        let prev = if cur > 0 { cur - 1 } else { 0 };
        s.player.queue.current_index = Some(prev);
        if let Some(t) = s.player.queue.items.get(prev).cloned() {
            s.player.duration_ms = t.duration_ms;
            s.player.position_ms = 0;
            s.player.state = if live {
                PlayerState::Loading
            } else {
                PlayerState::Playing
            };
            s.player.active_proxy_url = Some(proxy_stream_url(&t.id));
            s.lyrics = if live {
                None
            } else {
                Some(fixture_lyrics(&t.id))
            };
            dispatch = Some(t.clone());
            s.player.current_track = Some(t);
        }
    });
    if live && let Some(t) = dispatch {
        live_engine::dispatch_play_track(t);
    }
}

pub fn jump_queue(index: usize) {
    let live = !is_fixture_mode();
    let mut dispatch: Option<Track> = None;
    state_signal().update(|s| {
        if let Some(t) = s.player.queue.items.get(index).cloned() {
            s.player.queue.current_index = Some(index);
            s.player.duration_ms = t.duration_ms;
            s.player.position_ms = 0;
            s.player.state = if live {
                PlayerState::Loading
            } else {
                PlayerState::Playing
            };
            s.player.active_proxy_url = Some(proxy_stream_url(&t.id));
            s.lyrics = if live {
                None
            } else {
                Some(fixture_lyrics(&t.id))
            };
            dispatch = Some(t.clone());
            s.player.current_track = Some(t);
        }
    });
    if live && let Some(t) = dispatch {
        live_engine::dispatch_play_track(t);
    }
}

pub fn move_queue_item(from: usize, to: usize) {
    state_signal().update(|s| {
        let len = s.player.queue.items.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let item = s.player.queue.items.remove(from);
        s.player.queue.items.insert(to, item);
        if let Some(cur) = s.player.queue.current_index {
            s.player.queue.current_index = Some(if cur == from {
                to
            } else if from < cur && cur <= to {
                cur - 1
            } else if to <= cur && cur < from {
                cur + 1
            } else {
                cur
            });
        }
    });
}

pub fn remove_queue_item(index: usize) {
    state_signal().update(|s| {
        if index >= s.player.queue.items.len() {
            return;
        }
        s.player.queue.items.remove(index);
        if s.player.queue.items.is_empty() {
            s.player.queue.current_index = None;
            s.player.current_track = None;
            s.player.active_proxy_url = None;
            s.player.state = PlayerState::Idle;
        } else if let Some(cur) = s.player.queue.current_index {
            if index < cur {
                s.player.queue.current_index = Some(cur - 1);
            } else if index == cur {
                let next = cur.min(s.player.queue.items.len() - 1);
                s.player.queue.current_index = Some(next);
                let next_track = s.player.queue.items.get(next).cloned();
                s.player.active_proxy_url = next_track.as_ref().map(|t| proxy_stream_url(&t.id));
                s.player.current_track = next_track;
            }
        }
    });
}

pub fn clear_queue() {
    state_signal().update(|s| {
        s.player.queue.items.clear();
        s.player.queue.current_index = None;
        s.player.current_track = None;
        s.player.active_proxy_url = None;
        s.player.state = PlayerState::Idle;
    });
}

pub fn toggle_shuffle() {
    state_signal().update(|s| {
        s.player.queue.shuffle = !s.player.queue.shuffle;
    });
}

pub fn cycle_repeat() {
    state_signal().update(|s| {
        s.player.queue.repeat = s.player.queue.repeat.cycle();
    });
}

pub fn toggle_like_current() {
    state_signal().update(|s| {
        if let Some(cur) = s.player.current_track.as_mut() {
            cur.is_liked = !cur.is_liked;
            if cur.is_liked {
                s.disliked_ids.retain(|id| id != &cur.id);
                if !s.liked_tracks.iter().any(|t| t.id == cur.id) {
                    s.liked_tracks.insert(0, cur.clone());
                }
            } else {
                s.liked_tracks.retain(|t| t.id != cur.id);
            }
        }
    });
}

pub fn toggle_like_track(mut track: Track) {
    state_signal().update(|s| {
        let already = s.liked_tracks.iter().any(|t| t.id == track.id);
        let next_liked = !already;
        if next_liked {
            track.is_liked = true;
            s.disliked_ids.retain(|id| id != &track.id);
            s.liked_tracks.insert(0, track.clone());
        } else {
            s.liked_tracks.retain(|t| t.id != track.id);
        }
        if let Some(cur) = s.player.current_track.as_mut()
            && cur.id == track.id
        {
            cur.is_liked = next_liked;
        }
    });
}

pub fn set_autoplay_radio(enabled: bool) {
    state_signal().update(|s| s.player.queue.autoplay_radio = enabled);
}

pub fn set_sponsorblock_enabled(enabled: bool) {
    state_signal().update(|s| s.player.sponsorblock_enabled = enabled);
}

pub fn clear_caches() {
    state_signal().update(|s| {
        s.cache_stats.audio_chunk_bytes = 0;
        s.cache_stats.audio_chunk_count = 0;
        s.cache_stats.artwork_bytes = 0;
        s.cache_stats.metadata_entries = 0;
        s.status_banner = Some("Cleared audio, artwork, and metadata caches".into());
    });
    if !is_fixture_mode() {
        live_engine::dispatch_clear_caches();
    }
}

pub fn downloads_list() -> Vec<DownloadRecord> {
    let mut list: Vec<DownloadRecord> = state_signal().with(|s| s.downloads.values().cloned().collect());
    list.sort_by(|a, b| {
        let ta = a.completed_at_unix_secs.unwrap_or(a.updated_at_unix_secs);
        let tb = b.completed_at_unix_secs.unwrap_or(b.updated_at_unix_secs);
        tb.cmp(&ta).then_with(|| a.track.title.cmp(&b.track.title))
    });
    list
}

pub fn download_record(video_id: &str) -> Option<DownloadRecord> {
    state_signal().with(|s| s.downloads.get(video_id).cloned())
}

pub fn enqueue_download(track: Track) {
    if is_fixture_mode() {
        let vid = track.id.clone();
        state_signal().update(|s| {
            s.downloads.insert(
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
        });
        return;
    }
    live_engine::dispatch_enqueue_download(track);
}

pub fn pause_download(video_id: &str) {
    if !is_fixture_mode() {
        live_engine::dispatch_pause_download(video_id.to_owned());
    }
}

pub fn resume_download(video_id: &str) {
    if !is_fixture_mode() {
        live_engine::dispatch_resume_download(video_id.to_owned());
    }
}

pub fn cancel_download(video_id: &str) {
    if !is_fixture_mode() {
        live_engine::dispatch_cancel_download(video_id.to_owned());
    }
}

pub fn delete_download(video_id: &str) {
    if is_fixture_mode() {
        state_signal().update(|s| {
            s.downloads.remove(video_id);
        });
        return;
    }
    live_engine::dispatch_delete_download(video_id.to_owned());
}

pub fn sign_in_with_cookies(cookie_input: &str) {
    let trimmed = cookie_input.trim();
    if trimmed.contains("SAPISID=") || trimmed.contains("__Secure-3PAPISID=") {
        state_signal().update(|s| {
            s.signed_in = true;
            s.account_name = Some("Google Account (SAPISIDHASH)".into());
            s.status_banner = Some("Signed in securely (stored in OS Keystore)".into());
        });
        if !is_fixture_mode() {
            live_engine::dispatch_sign_in(trimmed.to_owned());
        }
    } else {
        state_signal().update(|s| {
            s.status_banner = Some("Missing SAPISID / __Secure-3PAPISID cookie".into());
        });
    }
}

pub fn sign_out() {
    state_signal().update(|s| {
        s.signed_in = false;
        s.account_name = None;
        s.status_banner = Some("Signed out — running in anonymous mode".into());
    });
    if !is_fixture_mode() {
        live_engine::dispatch_sign_out();
    }
}

pub fn dismiss_banner() {
    state_signal().update(|s| {
        s.status_banner = None;
    });
}

pub fn set_offline_mode(enabled: bool) {
    state_signal().update(|s| {
        s.player.offline_mode = enabled;
        s.status_banner = Some(if enabled {
            "Offline mode enabled — playing from local 256 KB chunk cache".into()
        } else {
            "Online mode restored — streaming via Loopback Proxy".into()
        });
    });
}

pub fn set_volume(volume: f32) {
    let clamped = volume.clamp(0.0, 1.0);
    state_signal().update(|s| {
        s.player.volume = clamped;
    });
}

pub fn adjust_volume(delta: f32) {
    state_signal().update(|s| {
        s.player.volume = (s.player.volume + delta).clamp(0.0, 1.0);
    });
}

pub fn toggle_mute() {
    state_signal().update(|s| {
        s.player.volume = if s.player.volume > 0.01 { 0.0 } else { 1.0 };
    });
}

pub fn total_queue_duration_ms() -> u64 {
    state_signal().with(|s| s.player.queue.items.iter().map(|t| t.duration_ms).sum())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    TogglePlayPause,
    SeekForward10s,
    SeekBackward10s,
    NextTrack,
    PreviousTrack,
    ToggleLike,
    ToggleMute,
    ToggleShuffle,
    CycleRepeat,
}

/// Pure, thread-independent keyboard shortcut mapper.
pub fn map_shortcut_key(key: &str) -> Option<ShortcutAction> {
    match key {
        " " | "Space" => Some(ShortcutAction::TogglePlayPause),
        "ArrowRight" => Some(ShortcutAction::SeekForward10s),
        "ArrowLeft" => Some(ShortcutAction::SeekBackward10s),
        "n" | "N" => Some(ShortcutAction::NextTrack),
        "p" | "P" => Some(ShortcutAction::PreviousTrack),
        "l" | "L" => Some(ShortcutAction::ToggleLike),
        "m" | "M" => Some(ShortcutAction::ToggleMute),
        "s" | "S" => Some(ShortcutAction::ToggleShuffle),
        "r" | "R" => Some(ShortcutAction::CycleRepeat),
        _ => None,
    }
}

/// Handles desktop keyboard media shortcuts on the NSH engine thread without blocking the UI.
/// Returns `true` when the key was handled.
pub fn handle_shortcut(key: &str) -> bool {
    let Some(action) = map_shortcut_key(key) else {
        return false;
    };
    match action {
        ShortcutAction::TogglePlayPause => toggle_play_pause(),
        ShortcutAction::SeekForward10s => {
            let pos = player_snapshot().position_ms;
            seek_ms(pos.saturating_add(10_000));
        }
        ShortcutAction::SeekBackward10s => {
            let pos = player_snapshot().position_ms;
            seek_ms(pos.saturating_sub(10_000));
        }
        ShortcutAction::NextTrack => next_track(),
        ShortcutAction::PreviousTrack => previous_track(),
        ShortcutAction::ToggleLike => toggle_like_current(),
        ShortcutAction::ToggleMute => toggle_mute(),
        ShortcutAction::ToggleShuffle => toggle_shuffle(),
        ShortcutAction::CycleRepeat => cycle_repeat(),
    }
    true
}

/// Snapshot observed by `platform::nsh_audio::mount_nsh_audio_driver()`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioDriverSnapshot {
    pub active_proxy_url: Option<String>,
    pub desired_playing: bool,
    pub position_ms: u64,
    pub seek_generation: u64,
    pub volume: f32,
    pub looped: bool,
}

pub fn audio_driver_snapshot() -> AudioDriverSnapshot {
    state_signal().with(|s| AudioDriverSnapshot {
        active_proxy_url: s.player.active_proxy_url.clone(),
        desired_playing: matches!(
            s.player.state,
            PlayerState::Playing | PlayerState::Buffering
        ),
        position_ms: s.player.position_ms,
        seek_generation: s.seek_generation,
        volume: s.player.volume,
        looped: s.player.queue.repeat == RepeatMode::One,
    })
}

pub fn set_nsh_audio_available(available: bool) {
    state_signal().update(|s| {
        s.nsh_audio_available = available;
    });
}

pub fn nsh_audio_available() -> bool {
    state_signal().with(|s| s.nsh_audio_available)
}

pub fn on_native_audio_duration(duration_ms: u64) {
    if duration_ms == 0 {
        return;
    }
    state_signal().update(|s| {
        s.player.duration_ms = duration_ms;
    });
}

pub fn on_native_audio_state(state: PlayerState) {
    state_signal().update(|s| {
        if s.player.current_track.is_some() {
            s.player.state = state;
        }
    });
}

pub fn on_native_audio_buffering(buffering: bool) {
    state_signal().update(|s| {
        if s.player.current_track.is_none() {
            return;
        }
        if buffering {
            s.player.state = PlayerState::Buffering;
        } else if matches!(s.player.state, PlayerState::Buffering) {
            s.player.state = PlayerState::Playing;
        }
    });
}

/// Updates playback position from native `nsh/audio` progress events WITHOUT
/// incrementing `seek_generation` (preventing seek echo loops).
pub fn on_native_audio_progress(position_ms: u64, duration_ms: Option<u64>) {
    state_signal().update(|s| {
        if s.player.current_track.is_none() {
            return;
        }
        s.player.position_ms = position_ms;
        s.player.queue.resume_position_ms = position_ms;
        if let Some(dur) = duration_ms.filter(|d| *d > 0) {
            s.player.duration_ms = dur;
        }
    });
}

pub fn on_native_audio_ended() {
    let live = !is_fixture_mode();
    let mut dispatch: Option<Track> = None;
    state_signal().update(|s| {
        let len = s.player.queue.items.len();
        if len == 0 {
            s.player.state = PlayerState::Idle;
            s.player.active_proxy_url = None;
            return;
        }
        let cur = s.player.queue.current_index.unwrap_or(0);
        let next = match s.player.queue.repeat {
            RepeatMode::One => Some(cur),
            RepeatMode::All => Some((cur + 1) % len),
            RepeatMode::None => {
                if cur + 1 < len {
                    Some(cur + 1)
                } else {
                    None
                }
            }
        };
        if let Some(next_idx) = next {
            s.player.queue.current_index = Some(next_idx);
            if let Some(t) = s.player.queue.items.get(next_idx).cloned() {
                s.player.duration_ms = t.duration_ms;
                s.player.position_ms = 0;
                s.player.queue.resume_position_ms = 0;
                s.player.state = if live {
                    PlayerState::Loading
                } else {
                    PlayerState::Playing
                };
                s.player.active_proxy_url = Some(proxy_stream_url(&t.id));
                s.lyrics = if live {
                    None
                } else {
                    Some(fixture_lyrics(&t.id))
                };
                dispatch = Some(t.clone());
                s.player.current_track = Some(t);
            }
        } else {
            s.player.state = PlayerState::Paused;
        }
    });
    if live && let Some(t) = dispatch {
        live_engine::dispatch_play_track(t);
    }
}

pub fn on_native_audio_error(message: String) {
    state_signal().update(|s| {
        s.status_banner = Some(message);
    });
}
