//! App-level reactive NSH store (`global::<AppState>`) decomposed into clear
//! domain slices without ever blocking the NSH UI thread:
//!
//! - [`playback_slice`]: Player snapshot, queue, lyrics, shortcuts, native audio callbacks
//! - [`catalog_slice`]: Home shelves, moods, search, collection browsing
//! - [`library_slice`]: Liked tracks, history, cache stats, downloads
//! - [`mutation_slice`]: Optimistic like/dislike mutations & rollback
//! - [`settings_slice`]: Theme, reduced motion, SAPISIDHASH auth, Audio DSP, PO token diagnostics
//! - [`ui_slice`]: Tab navigation history, sidebars, global toast banner

pub mod catalog_slice;
pub mod library_slice;
pub mod mutation_slice;
pub mod playback_slice;
pub mod settings_slice;
pub mod ui_slice;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use nsh_core::prelude::*;
use resonance_contract::{
    AuthBackendStatus, CacheStats, DownloadRecord, HistoryEntry, HomeSection, LyricsDocument,
    PlayerSnapshot, PlayerState, Playlist, PoTokenDiagnostics, Track,
};
use resonance_core::audio_dsp::DspConfig;
use resonance_core::fixture::{fixture_home_sections, fixture_tracks};
use resonance_core::palette::{AmbientPalette, palette_for_track};

use crate::live_engine::{self, LiveUpdate};

pub use catalog_slice::{CatalogSlice, SearchFilter, SidebarPlaylist};
pub use library_slice::{LibraryFilter, LibrarySlice};
pub use mutation_slice::MutationSlice;
pub use playback_slice::{AudioDriverSnapshot, PlaybackSlice, ShortcutAction, map_shortcut_key};
pub use settings_slice::{SettingsSlice, ThemeChoice};
pub use ui_slice::{AppTab, DesktopSidePanel, UiSlice};

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

#[derive(Clone)]
pub struct AppState {
    pub ui: UiSlice,
    pub playback: PlaybackSlice,
    pub catalog: CatalogSlice,
    pub library: LibrarySlice,
    pub mutations: MutationSlice,
    pub settings: SettingsSlice,
}

impl Default for AppState {
    fn default() -> Self {
        let fixture = is_fixture_mode();
        let tracks = if fixture {
            fixture_tracks()
        } else {
            Vec::new()
        };
        let liked_tracks = tracks.iter().filter(|t| t.is_liked).cloned().collect();

        Self {
            ui: UiSlice::default(),
            playback: PlaybackSlice::new(fixture, tracks, proxy_stream_url),
            catalog: CatalogSlice::new(fixture),
            library: LibrarySlice::new(fixture, liked_tracks),
            mutations: MutationSlice::default(),
            settings: SettingsSlice::new(fixture),
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
                if s.playback.player.current_track.is_none() && snapshot.current_track.is_some() {
                    s.playback.player = snapshot;
                }
                s.library.cache_stats = cache_stats;
                s.settings.signed_in = signed_in;
                if signed_in && s.settings.account_name.is_none() {
                    s.settings.account_name = Some("Google Account (SAPISIDHASH)".into());
                }
            });
        }
        LiveUpdate::HomeLoaded(res) => {
            state_signal().update(|s| s.catalog.apply_home_loaded(res));
        }
        LiveUpdate::SearchLoaded { query, result } => {
            state_signal().update(|s| s.catalog.apply_search_loaded(&query, result));
        }
        LiveUpdate::CollectionLoaded(res) => {
            state_signal().update(|s| match res {
                Ok(playlist) => {
                    s.catalog.selected_collection = Some(playlist);
                }
                Err(err) => {
                    s.ui.set_status_banner(format!("Collection load failed: {err}"));
                }
            });
        }
        LiveUpdate::PlaybackUpdated(snapshot) => {
            state_signal().update(|s| {
                s.playback.player = snapshot;
            });
        }
        LiveUpdate::LyricsLoaded(doc) => {
            state_signal().update(|s| {
                s.playback.lyrics = doc;
            });
        }
        LiveUpdate::CacheStatsUpdated(stats) => {
            state_signal().update(|s| {
                s.library.cache_stats = stats;
            });
        }
        LiveUpdate::AuthUpdated { signed_in, banner } => {
            state_signal().update(|s| {
                s.settings.apply_auth_updated(signed_in);
                s.ui.set_status_banner(banner);
            });
        }
        LiveUpdate::AuthStatus(status) => {
            state_signal().update(|s| s.settings.apply_auth_status(status));
        }
        LiveUpdate::PoTokenDiagnostics(diag) => {
            state_signal().update(|s| {
                s.settings.po_token_diagnostics = diag;
            });
        }
        LiveUpdate::MutationFailed { command, reason } => {
            state_signal().update(|s| {
                s.mutations
                    .rollback_mutation(command, &mut s.playback, &mut s.library);
                s.ui.set_status_banner(format!("Action failed: {reason}"));
            });
        }
        LiveUpdate::DownloadsLoaded(records) => {
            state_signal().update(|s| s.library.set_downloads(records));
        }
        LiveUpdate::DownloadUpdated(record) => {
            state_signal().update(|s| {
                s.library.downloads.insert(record.video_id.clone(), record);
            });
        }
        LiveUpdate::DownloadRemoved(video_id) => {
            state_signal().update(|s| {
                s.library.downloads.remove(&video_id);
            });
        }
        LiveUpdate::StatusBanner(msg) => {
            state_signal().update(|s| s.ui.set_status_banner(msg));
        }
    }
}

/// Returns the cached artwork-driven ambient palette for the currently playing track.
pub fn active_ambient_palette() -> AmbientPalette {
    state_signal().with(|s| match s.playback.player.current_track.as_ref() {
        Some(t) => palette_for_track(&t.id, &t.title),
        None => AmbientPalette::default(),
    })
}

// ── UI / Navigation delegates ────────────────────────────────────────────────

pub fn active_tab() -> AppTab {
    state_signal().with(|s| s.ui.active_tab)
}

pub fn set_tab(tab: AppTab) {
    state_signal().update(|s| s.ui.set_tab(tab));
}

pub fn can_go_back() -> bool {
    state_signal().with(|s| s.ui.can_go_back(s.catalog.selected_collection.is_some()))
}

pub fn can_go_forward() -> bool {
    state_signal().with(|s| s.ui.can_go_forward())
}

pub fn go_back() {
    state_signal().update(|s| {
        if s.ui.active_tab == AppTab::Home && s.catalog.selected_collection.is_some() {
            s.catalog.selected_collection = None;
            return;
        }
        s.ui.go_back();
    });
}

pub fn go_forward() {
    state_signal().update(|s| s.ui.go_forward());
}

pub fn sidebar_collapsed() -> bool {
    state_signal().with(|s| s.ui.sidebar_collapsed)
}

pub fn toggle_sidebar() {
    state_signal().update(|s| s.ui.toggle_sidebar());
}

pub fn desktop_side_panel_open() -> bool {
    state_signal().with(|s| s.ui.desktop_side_panel_open)
}

pub fn toggle_desktop_side_panel(panel: DesktopSidePanel) {
    state_signal().update(|s| s.ui.toggle_desktop_side_panel(panel));
}

pub fn desktop_side_panel() -> DesktopSidePanel {
    state_signal().with(|s| s.ui.desktop_side_panel)
}

pub fn set_desktop_side_panel(panel: DesktopSidePanel) {
    state_signal().update(|s| s.ui.set_desktop_side_panel(panel));
}

pub fn status_banner() -> Option<String> {
    state_signal().with(|s| s.ui.status_banner.clone())
}

pub fn set_status_banner(msg: impl Into<String>) {
    let text = msg.into();
    state_signal().update(|s| s.ui.set_status_banner(text));
}

pub fn dismiss_banner() {
    state_signal().update(|s| s.ui.dismiss_banner());
}

// ── Settings, Auth, DSP & PO Token Diagnostics delegates ─────────────────────

pub fn theme_choice() -> ThemeChoice {
    state_signal().with(|s| s.settings.theme_choice)
}

pub fn set_theme_choice(choice: ThemeChoice) {
    state_signal().update(|s| s.settings.theme_choice = choice);
}

pub fn reduced_motion() -> bool {
    state_signal().with(|s| s.settings.reduced_motion)
}

pub fn set_reduced_motion(enabled: bool) {
    state_signal().update(|s| s.settings.reduced_motion = enabled);
}

pub fn auth_info() -> (bool, Option<String>) {
    state_signal().with(|s| (s.settings.signed_in, s.settings.account_name.clone()))
}

pub fn auth_backend_status() -> AuthBackendStatus {
    state_signal().with(|s| s.settings.auth_backend_status.clone())
}

pub fn sign_in_with_cookies(cookie_input: &str) {
    let trimmed = cookie_input.trim().to_owned();
    let mut accepted = false;
    state_signal().update(|s| {
        let (ok, banner) = s.settings.sign_in_with_cookies(&trimmed);
        accepted = ok;
        s.ui.set_status_banner(banner);
    });
    if accepted && !is_fixture_mode() {
        live_engine::dispatch_sign_in(trimmed);
    }
}

pub fn sign_out() {
    state_signal().update(|s| {
        let banner = s.settings.sign_out();
        s.ui.set_status_banner(banner);
    });
    if !is_fixture_mode() {
        live_engine::dispatch_sign_out();
    }
}

pub fn dsp_config() -> DspConfig {
    state_signal().with(|s| s.settings.dsp_config.clone())
}

pub fn set_loudness_normalization(enabled: bool) {
    state_signal().update(|s| {
        s.settings.set_loudness_normalization(enabled);
        s.ui.set_status_banner(if enabled {
            "ReplayGain / EBU R128 loudness normalization enabled"
        } else {
            "ReplayGain / EBU R128 loudness normalization disabled"
        });
    });
}

pub fn set_silence_skip(enabled: bool) {
    state_signal().update(|s| {
        s.settings.set_silence_skip(enabled);
        s.ui.set_status_banner(if enabled {
            "Silence skipping enabled"
        } else {
            "Silence skipping disabled"
        });
    });
}

pub fn cycle_eq_preset() {
    state_signal().update(|s| {
        let label = s.settings.cycle_eq_preset();
        s.ui.set_status_banner(format!("10-band EQ preset: {label}"));
    });
}

pub fn cycle_crossfade_ms() {
    state_signal().update(|s| {
        let ms = s.settings.cycle_crossfade_ms();
        let label = if ms == 0 {
            "Gapless playback (0s crossfade)".to_owned()
        } else {
            format!("Crossfade set to {}s", ms / 1000)
        };
        s.ui.set_status_banner(label);
    });
}

pub fn po_token_diagnostics() -> PoTokenDiagnostics {
    state_signal().with(|s| s.settings.po_token_diagnostics.clone())
}

pub fn set_po_token_diagnostics(diag: PoTokenDiagnostics) {
    state_signal().update(|s| s.settings.po_token_diagnostics = diag);
}

// ── Catalog / Search / Collection delegates ──────────────────────────────────

pub fn home_sections() -> Vec<HomeSection> {
    state_signal().with(|s| s.catalog.home_sections.clone())
}

pub fn is_home_loading() -> bool {
    state_signal().with(|s| s.catalog.home_loading)
}

pub fn home_error() -> Option<String> {
    state_signal().with(|s| s.catalog.home_error.clone())
}

pub fn selected_mood() -> Option<String> {
    state_signal().with(|s| s.catalog.selected_mood.clone())
}

pub fn select_mood(mood: &str) {
    let toggle_off = state_signal().with(|s| s.catalog.selected_mood.as_deref() == Some(mood));
    state_signal().update(|s| {
        s.catalog.selected_mood = if toggle_off {
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
    state_signal().with(|s| s.catalog.section_pages.get(section_id).copied().unwrap_or(0))
}

pub fn shift_section_page(section_id: &str, delta: isize, total_items: usize, page_size: usize) {
    state_signal().update(|s| {
        s.catalog
            .shift_section_page(section_id, delta, total_items, page_size);
    });
}

pub fn sidebar_playlists() -> Vec<SidebarPlaylist> {
    state_signal().with(|s| s.catalog.sidebar_playlists(s.library.liked_tracks.len()))
}

pub fn refresh_home() {
    if is_fixture_mode() {
        state_signal().update(|s| {
            s.catalog.home_sections = fixture_home_sections();
            s.catalog.home_loading = false;
            s.catalog.home_error = None;
        });
    } else {
        state_signal().update(|s| {
            s.catalog.home_loading = true;
            s.catalog.home_error = None;
        });
        live_engine::dispatch_load_home();
    }
}

pub fn search_query() -> String {
    state_signal().with(|s| s.catalog.search_query.clone())
}

pub fn search_filter() -> SearchFilter {
    state_signal().with(|s| s.catalog.search_filter)
}

pub fn set_search_filter(filter: SearchFilter) {
    state_signal().update(|s| s.catalog.set_search_filter(filter));
}

pub fn search_results_len() -> usize {
    state_signal().with(|s| s.catalog.search_results.len())
}

pub fn search_result_at(index: usize) -> Option<Track> {
    state_signal().with(|s| s.catalog.search_results.get(index).cloned())
}

pub fn search_result_ids() -> Vec<SharedString> {
    state_signal().with(|s| s.catalog.search_result_ids())
}

pub fn is_search_loading() -> bool {
    state_signal().with(|s| s.catalog.search_loading)
}

pub fn search_error() -> Option<String> {
    state_signal().with(|s| s.catalog.search_error.clone())
}

pub fn run_search(query: &str) {
    let q = query.trim().to_owned();
    if is_fixture_mode() {
        state_signal().update(|s| s.catalog.run_fixture_search(q));
    } else {
        let should_dispatch = !q.is_empty();
        state_signal().update(|s| s.catalog.begin_live_search(q.clone()));
        if should_dispatch {
            live_engine::dispatch_search(q);
        }
    }
}

pub fn selected_collection() -> Option<Playlist> {
    state_signal().with(|s| s.catalog.selected_collection.clone())
}

pub fn open_collection(
    browse_id: String,
    title: String,
    subtitle: String,
    artwork_url: Option<String>,
) {
    let fixture = is_fixture_mode();
    set_tab(AppTab::Home);
    let live_id = browse_id.clone();
    state_signal().update(|s| {
        s.catalog
            .open_collection(fixture, browse_id, title, subtitle, artwork_url);
    });
    if !fixture {
        live_engine::dispatch_browse_collection(live_id);
    }
}

pub fn close_collection() {
    state_signal().update(|s| s.catalog.close_collection());
}

pub fn collection_search_query() -> String {
    state_signal().with(|s| s.catalog.collection_search_query.clone())
}

pub fn set_collection_search_query(q: &str) {
    state_signal().update(|s| s.catalog.collection_search_query = q.to_owned());
}

pub fn collection_sort_asc() -> bool {
    state_signal().with(|s| s.catalog.collection_sort_asc)
}

pub fn toggle_collection_sort() {
    state_signal().update(|s| s.catalog.collection_sort_asc = !s.catalog.collection_sort_asc);
}

// ── Library, Cache & Downloads delegates ─────────────────────────────────────

pub fn liked_tracks() -> Vec<Track> {
    state_signal().with(|s| s.library.liked_tracks.clone())
}

pub fn is_track_liked(track_id: &str) -> bool {
    state_signal().with(|s| s.library.is_track_liked(track_id))
}

pub fn is_current_disliked() -> bool {
    state_signal().with(|s| {
        s.playback
            .player
            .current_track
            .as_ref()
            .is_some_and(|t| s.library.disliked_ids.contains(&t.id))
    })
}

pub fn library_filter() -> LibraryFilter {
    state_signal().with(|s| s.library.library_filter)
}

pub fn set_library_filter(filter: LibraryFilter) {
    state_signal().update(|s| s.library.library_filter = filter);
}

pub fn library_search_query() -> String {
    state_signal().with(|s| s.library.library_search_query.clone())
}

pub fn set_library_search_query(q: &str) {
    state_signal().update(|s| s.library.library_search_query = q.to_owned());
}

pub fn history_entries() -> Vec<HistoryEntry> {
    state_signal().with(|s| s.library.history.clone())
}

pub fn cache_stats() -> CacheStats {
    state_signal().with(|s| s.library.cache_stats)
}

pub fn clear_caches() {
    state_signal().update(|s| {
        s.library.clear_transient_caches();
        s.ui.set_status_banner("Cleared audio, artwork, and metadata caches");
    });
    if !is_fixture_mode() {
        live_engine::dispatch_clear_caches();
    }
}

pub fn downloads_list() -> Vec<DownloadRecord> {
    state_signal().with(|s| s.library.downloads_list())
}

pub fn download_record(video_id: &str) -> Option<DownloadRecord> {
    state_signal().with(|s| s.library.downloads.get(video_id).cloned())
}

pub fn enqueue_download(track: Track) {
    if is_fixture_mode() {
        state_signal().update(|s| s.library.enqueue_fixture_download(track));
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
            s.library.downloads.remove(video_id);
        });
        return;
    }
    live_engine::dispatch_delete_download(video_id.to_owned());
}

// ── Optimistic Mutation delegates ────────────────────────────────────────────

pub fn toggle_like_current() {
    let live = !is_fixture_mode();
    let mut dispatch = None;
    state_signal().update(|s| {
        dispatch = s.mutations.toggle_like_current(
            &mut s.playback,
            &mut s.library,
            live,
            s.settings.signed_in,
        );
    });
    if let Some((video_id, action)) = dispatch {
        live_engine::dispatch_set_like(video_id, action);
    }
}

pub fn toggle_like_track(track: Track) {
    let live = !is_fixture_mode();
    let mut dispatch = None;
    state_signal().update(|s| {
        dispatch = s.mutations.toggle_like_track(
            track,
            &mut s.playback,
            &mut s.library,
            live,
            s.settings.signed_in,
        );
    });
    if let Some((video_id, action)) = dispatch {
        live_engine::dispatch_set_like(video_id, action);
    }
}

pub fn toggle_dislike_current() {
    let live = !is_fixture_mode();
    let mut dispatch = None;
    state_signal().update(|s| {
        let (cmd, banner) = s.mutations.toggle_dislike_current(
            &mut s.playback,
            &mut s.library,
            live,
            s.settings.signed_in,
        );
        dispatch = cmd;
        if let Some(msg) = banner {
            s.ui.set_status_banner(msg);
        }
    });
    if let Some((video_id, action)) = dispatch {
        live_engine::dispatch_set_like(video_id, action);
    }
}

// ── Playback, Queue & Native Audio delegates ─────────────────────────────────

pub fn player_snapshot() -> PlayerSnapshot {
    state_signal().with(|s| s.playback.player.clone())
}

pub fn queue_len() -> usize {
    state_signal().with(|s| s.playback.player.queue.items.len())
}

pub fn queue_track_at(index: usize) -> Option<Track> {
    state_signal().with(|s| s.playback.player.queue.items.get(index).cloned())
}

pub fn queue_ids() -> Vec<SharedString> {
    state_signal().with(|s| s.playback.queue_ids())
}

pub fn current_lyrics() -> Option<LyricsDocument> {
    state_signal().with(|s| s.playback.lyrics.clone())
}

pub fn play_track(track: Track) {
    let live = !is_fixture_mode();
    let track_for_live = track.clone();
    let url = proxy_stream_url(&track.id);
    state_signal().update(|s| {
        s.playback.play_track(track.clone(), live, url);
        s.library.record_play(track);
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
        s.playback.player.queue.shuffle = true;
    });
    play_track(first);
    for extra in tracks {
        play_last(extra);
    }
}

pub fn play_next(track: Track) {
    state_signal().update(|s| {
        let banner = s.playback.play_next(track);
        s.ui.set_status_banner(banner);
    });
}

pub fn play_last(track: Track) {
    state_signal().update(|s| {
        let banner = s.playback.play_last(track);
        s.ui.set_status_banner(banner);
    });
}

pub fn toggle_play_pause() {
    state_signal().update(|s| s.playback.toggle_play_pause());
}

pub fn seek_ms(target_ms: u64) {
    state_signal().update(|s| s.playback.seek_ms(target_ms));
}

pub fn next_track() {
    let live = !is_fixture_mode();
    let mut dispatch = None;
    state_signal().update(|s| {
        dispatch = s.playback.next_track(live, proxy_stream_url);
    });
    if live && let Some(t) = dispatch {
        live_engine::dispatch_play_track(t);
    }
}

pub fn previous_track() {
    let live = !is_fixture_mode();
    let mut dispatch = None;
    state_signal().update(|s| {
        dispatch = s.playback.previous_track(live, proxy_stream_url);
    });
    if live && let Some(t) = dispatch {
        live_engine::dispatch_play_track(t);
    }
}

pub fn jump_queue(index: usize) {
    let live = !is_fixture_mode();
    let mut dispatch = None;
    state_signal().update(|s| {
        dispatch = s.playback.jump_queue(index, live, proxy_stream_url);
    });
    if live && let Some(t) = dispatch {
        live_engine::dispatch_play_track(t);
    }
}

pub fn move_queue_item(from: usize, to: usize) {
    state_signal().update(|s| s.playback.move_queue_item(from, to));
}

pub fn remove_queue_item(index: usize) {
    state_signal().update(|s| s.playback.remove_queue_item(index, proxy_stream_url));
}

pub fn clear_queue() {
    state_signal().update(|s| s.playback.clear_queue());
}

pub fn toggle_shuffle() {
    state_signal().update(|s| s.playback.toggle_shuffle());
}

pub fn cycle_repeat() {
    state_signal().update(|s| s.playback.cycle_repeat());
}

pub fn set_autoplay_radio(enabled: bool) {
    state_signal().update(|s| s.playback.player.queue.autoplay_radio = enabled);
}

pub fn set_sponsorblock_enabled(enabled: bool) {
    state_signal().update(|s| s.playback.player.sponsorblock_enabled = enabled);
}

pub fn set_offline_mode(enabled: bool) {
    state_signal().update(|s| {
        s.playback.player.offline_mode = enabled;
        s.ui.set_status_banner(if enabled {
            "Offline mode enabled — playing from local 256 KB chunk cache"
        } else {
            "Online mode restored — streaming via Loopback Proxy"
        });
    });
}

pub fn set_volume(volume: f32) {
    state_signal().update(|s| s.playback.set_volume(volume));
}

pub fn adjust_volume(delta: f32) {
    state_signal().update(|s| s.playback.adjust_volume(delta));
}

pub fn toggle_mute() {
    state_signal().update(|s| s.playback.toggle_mute());
}

pub fn total_queue_duration_ms() -> u64 {
    state_signal().with(|s| s.playback.total_queue_duration_ms())
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

pub fn audio_driver_snapshot() -> AudioDriverSnapshot {
    state_signal().with(|s| s.playback.audio_driver_snapshot())
}

pub fn set_nsh_audio_available(available: bool) {
    state_signal().update(|s| s.playback.nsh_audio_available = available);
}

pub fn nsh_audio_available() -> bool {
    state_signal().with(|s| s.playback.nsh_audio_available)
}

pub fn on_native_audio_duration(duration_ms: u64) {
    state_signal().update(|s| s.playback.on_native_audio_duration(duration_ms));
}

pub fn on_native_audio_state(state: PlayerState) {
    state_signal().update(|s| s.playback.on_native_audio_state(state));
}

pub fn on_native_audio_buffering(buffering: bool) {
    state_signal().update(|s| s.playback.on_native_audio_buffering(buffering));
}

pub fn on_native_audio_progress(position_ms: u64, duration_ms: Option<u64>) {
    state_signal().update(|s| s.playback.on_native_audio_progress(position_ms, duration_ms));
}

pub fn on_native_audio_ended() {
    let live = !is_fixture_mode();
    let mut dispatch = None;
    state_signal().update(|s| {
        dispatch = s.playback.on_native_audio_ended(live, proxy_stream_url);
    });
    if live && let Some(t) = dispatch {
        live_engine::dispatch_play_track(t);
    }
}

pub fn on_native_audio_error(message: String) {
    state_signal().update(|s| s.ui.set_status_banner(message));
}
