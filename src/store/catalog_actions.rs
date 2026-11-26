//! Catalog, Search, Library, Offline Vault & Optimistic Mutation actions
//! operating on `catalog_signal()`, `library_signal()`, and `mutations_signal()`.

use std::time::Duration;

use nsh_core::prelude::{SharedString, spawn, timer};
use resonance_contract::{
    AlbumDetail, ArtistDetail, CacheStats, DownloadRecord, ExplorePage, HistoryEntry, HomeSection,
    LibraryCollectionRef, MoodCategory, Playlist, PlaylistEditOp, Track,
};
use resonance_core::fixture::fixture_home_sections;
use resonance_core::offline::{CollectionOfflineProgress, PinnedCollection, PinnedCollectionKind};

use crate::live_engine::{self, EngineCommand};

use super::catalog_slice::{EntityRoute, SearchFilter, SidebarPlaylist};
use super::library_slice::LibraryFilter;
use super::ui_slice::AppTab;
use super::{
    catalog_signal, is_fixture_mode, library_signal, mutations_signal, playback_signal, set_tab,
    settings_signal, ui_signal,
};

// ── Catalog, Explore, Moods & Search ─────────────────────────────────────────

pub fn home_sections() -> Vec<HomeSection> {
    catalog_signal().with(|c| c.home_sections.clone())
}

pub fn explore_sections() -> Vec<HomeSection> {
    catalog_signal().with(|c| c.explore_sections.clone())
}

pub fn explore_page() -> Option<ExplorePage> {
    catalog_signal().with(|c| c.explore_page.clone())
}

pub fn mood_playlists() -> Vec<LibraryCollectionRef> {
    catalog_signal().with(|c| c.mood_playlists.clone())
}

pub fn load_mood_playlists(mood: &str, params: &str) {
    catalog_signal().update(|c| {
        c.selected_mood = Some(mood.to_owned());
    });
    if is_fixture_mode() {
        run_search(mood);
        set_tab(AppTab::Search);
    } else {
        live_engine::dispatch(EngineCommand::LoadMoodPlaylists {
            mood: mood.to_owned(),
            params: params.to_owned(),
        });
    }
}

pub fn search_suggestions() -> Vec<String> {
    catalog_signal().with(|c| c.search_suggestions.clone())
}

pub fn search_history() -> Vec<String> {
    catalog_signal().with(|c| c.search_history.clone())
}

pub fn record_search_history(query: &str) {
    catalog_signal().update(|c| c.record_search_history(query));
}

pub fn remove_search_history_item(query: &str) {
    catalog_signal().update(|c| c.remove_search_history_item(query));
}

pub fn clear_search_history() {
    catalog_signal().update(|c| c.clear_search_history());
}

pub fn home_mood_chips() -> Vec<MoodCategory> {
    catalog_signal().with(|c| c.home_mood_chips.clone())
}

pub fn search_continuation() -> Option<String> {
    catalog_signal().with(|c| c.search_continuation.clone())
}

pub fn load_more_search() {
    if is_fixture_mode() {
        return;
    }
    let (query, category, cont) = catalog_signal().with(|c| {
        (
            c.search_query.clone(),
            c.search_filter.as_search_category(),
            c.search_continuation.clone(),
        )
    });
    if let Some(continuation) = cont {
        catalog_signal().update(|c| c.search_loading = true);
        live_engine::dispatch(EngineCommand::SearchContinuation {
            query,
            category,
            continuation,
        });
    }
}

pub fn fetch_search_suggestions(prefix: &str) {
    if !is_fixture_mode() && !prefix.trim().is_empty() {
        live_engine::dispatch(EngineCommand::SearchSuggestions {
            prefix: prefix.to_owned(),
        });
    }
}

pub fn is_home_loading() -> bool {
    catalog_signal().with(|c| c.home_loading)
}

pub fn home_error() -> Option<String> {
    catalog_signal().with(|c| c.home_error.clone())
}

pub fn set_home_init_error(err: String) {
    catalog_signal().update(|c| {
        c.home_loading = false;
        c.home_error = Some(err);
    });
}

pub fn set_search_init_error(query: String, err: String) {
    catalog_signal().update(|c| {
        if c.search_query == query {
            c.search_loading = false;
            c.search_error = Some(err);
        }
    });
}

pub fn selected_mood() -> Option<String> {
    catalog_signal().with(|c| c.selected_mood.clone())
}

pub fn select_mood(mood: &str) {
    let toggle_off = catalog_signal().with(|c| c.selected_mood.as_deref() == Some(mood));
    catalog_signal().update(|c| {
        c.selected_mood = if toggle_off {
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
    catalog_signal().with(|c| c.section_pages.get(section_id).copied().unwrap_or(0))
}

pub fn shift_section_page(section_id: &str, delta: isize, total_items: usize, page_size: usize) {
    catalog_signal().update(|c| c.shift_section_page(section_id, delta, total_items, page_size));
}

pub fn sidebar_playlists() -> Vec<SidebarPlaylist> {
    let liked_count = library_signal().with(|l| l.liked_tracks.len());
    catalog_signal().with(|c| c.sidebar_playlists(liked_count))
}

pub fn refresh_home() {
    if is_fixture_mode() {
        catalog_signal().update(|c| {
            c.home_sections = fixture_home_sections();
            c.home_loading = false;
            c.home_error = None;
        });
    } else {
        catalog_signal().update(|c| {
            c.home_loading = true;
            c.home_error = None;
        });
        live_engine::dispatch(EngineCommand::LoadHome);
    }
}

pub fn search_query() -> String {
    catalog_signal().with(|c| c.search_query.clone())
}

pub fn search_filter() -> SearchFilter {
    catalog_signal().with(|c| c.search_filter)
}

pub fn set_search_filter(filter: SearchFilter) {
    let mut live_dispatch: Option<(String, Option<&'static str>)> = None;
    catalog_signal().update(|c| {
        c.set_search_filter(filter);
        if !is_fixture_mode() && !c.search_query.is_empty() {
            c.search_loading = true;
            c.search_error = None;
            c.search_generation = c.search_generation.wrapping_add(1);
            live_dispatch = Some((c.search_query.clone(), filter.params()));
        }
    });
    if let Some((query, params)) = live_dispatch {
        live_engine::dispatch(EngineCommand::Search { query, params });
    }
}

pub fn search_albums() -> Vec<Playlist> {
    catalog_signal().with(|c| c.search_albums.clone())
}

pub fn search_tracks() -> Vec<Track> {
    catalog_signal().with(|c| c.search_results.clone())
}

pub fn track_plays(video_id: &str) -> Option<String> {
    catalog_signal().with(|c| c.search_plays.get(video_id).cloned())
}

pub fn search_results_len() -> usize {
    catalog_signal().with(|c| c.search_results.len())
}

pub fn search_result_at(index: usize) -> Option<Track> {
    catalog_signal().with(|c| c.search_results.get(index).cloned())
}

pub fn search_result_ids() -> Vec<SharedString> {
    catalog_signal().with(|c| c.search_result_ids())
}

pub fn is_search_loading() -> bool {
    catalog_signal().with(|c| c.search_loading)
}

pub fn search_error() -> Option<String> {
    catalog_signal().with(|c| c.search_error.clone())
}

pub fn run_search(query: &str) {
    let q = query.trim().to_owned();
    if is_fixture_mode() {
        catalog_signal().update(|c| c.run_fixture_search(q));
    } else {
        let mut params = None;
        let should_dispatch = !q.is_empty();
        catalog_signal().update(|c| {
            c.begin_live_search(q.clone());
            if should_dispatch {
                params = c.search_filter.params();
            }
        });
        if should_dispatch {
            live_engine::dispatch(EngineCommand::Search { query: q, params });
        }
    }
}

/// Debounced search for interactive typing (260ms) so fast keystrokes do not spam InnerTube.
pub fn run_search_debounced(query: &str) {
    let q = query.trim().to_owned();
    if is_fixture_mode() || q.is_empty() {
        run_search(query);
        return;
    }
    let mut seq = 0u64;
    let mut params = None;
    catalog_signal().update(|c| {
        c.begin_live_search(q.clone());
        seq = c.search_generation;
        params = c.search_filter.params();
    });
    spawn(async move {
        timer(Duration::from_millis(260)).await;
        let still_current =
            catalog_signal().with(|c| c.search_generation == seq && c.search_query == q);
        if still_current {
            live_engine::dispatch(EngineCommand::Search { query: q, params });
        }
    });
}

pub fn selected_collection() -> Option<Playlist> {
    catalog_signal().with(|c| c.selected_collection.clone())
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
    catalog_signal().update(|c| {
        c.open_collection(fixture, browse_id, title, subtitle, artwork_url);
    });
    if !fixture {
        live_engine::dispatch(EngineCommand::BrowseCollection { browse_id: live_id });
    }
}

pub fn close_collection() {
    catalog_signal().update(|c| c.close_collection());
}

pub fn collection_search_query() -> String {
    catalog_signal().with(|c| c.collection_search_query.clone())
}

pub fn set_collection_search_query(q: &str) {
    catalog_signal().update(|c| c.collection_search_query = q.to_owned());
}

pub fn collection_sort_asc() -> bool {
    catalog_signal().with(|c| c.collection_sort_asc)
}

pub fn toggle_collection_sort() {
    catalog_signal().update(|c| c.collection_sort_asc = !c.collection_sort_asc);
}

pub fn current_entity_route() -> Option<EntityRoute> {
    catalog_signal().with(|c| c.current_entity_route().cloned())
}

pub fn active_artist_detail() -> Option<ArtistDetail> {
    catalog_signal().with(|c| c.active_artist.clone())
}

pub fn active_album_detail() -> Option<(AlbumDetail, CollectionOfflineProgress)> {
    catalog_signal().with(|c| {
        c.active_album
            .clone()
            .map(|album| (album, c.album_offline_progress.clone()))
    })
}

pub fn open_artist_entity(channel_id: String) {
    catalog_signal().update(|c| {
        c.push_entity_route(EntityRoute::Artist { channel_id });
    });
}

pub fn open_album_entity(browse_id: String) {
    catalog_signal().update(|c| {
        c.push_entity_route(EntityRoute::Album { browse_id });
    });
}

pub fn toggle_subscribe_artist(channel_id: String, subscribe: bool) {
    catalog_signal().update(|c| {
        if let Some(artist) = c.active_artist.as_mut()
            && artist.channel_id == channel_id
        {
            artist.is_subscribed = subscribe;
        }
    });
    ui_signal().update(|u| {
        u.set_status_banner(if subscribe {
            "Subscribed to artist"
        } else {
            "Unsubscribed from artist"
        });
    });
}

// ── Library, Cache & Offline Vault ───────────────────────────────────────────

pub fn library_playlists() -> Vec<LibraryCollectionRef> {
    library_signal().with(|l| l.playlists.clone())
}

pub fn library_albums() -> Vec<LibraryCollectionRef> {
    library_signal().with(|l| l.albums.clone())
}

pub fn library_artists() -> Vec<LibraryCollectionRef> {
    library_signal().with(|l| l.artists.clone())
}

pub fn liked_tracks() -> Vec<Track> {
    library_signal().with(|l| l.liked_tracks.clone())
}

pub fn is_track_liked(track_id: &str) -> bool {
    library_signal().with(|l| l.is_track_liked(track_id))
}

pub fn is_current_disliked() -> bool {
    let cur_id = playback_signal().with(|p| p.player.current_track.as_ref().map(|t| t.id.clone()));
    cur_id.is_some_and(|id| library_signal().with(|l| l.disliked_ids.contains(&id)))
}

pub fn library_filter() -> LibraryFilter {
    library_signal().with(|l| l.library_filter)
}

pub fn set_library_filter(filter: LibraryFilter) {
    library_signal().update(|l| l.library_filter = filter);
}

pub fn library_search_query() -> String {
    library_signal().with(|l| l.library_search_query.clone())
}

pub fn set_library_search_query(q: &str) {
    library_signal().update(|l| l.library_search_query = q.to_owned());
}

pub fn history_entries() -> Vec<HistoryEntry> {
    library_signal().with(|l| l.history.clone())
}

pub fn all_library_songs() -> Vec<Track> {
    let queue_items = playback_signal().with(|p| p.player.queue.items.clone());
    let home = catalog_signal().with(|c| c.home_sections.clone());
    library_signal().with(|l| {
        super::library_slice::collect_unique_songs(&queue_items, &l.liked_tracks, &home)
    })
}

pub fn filtered_library_tracks(filter: LibraryFilter, query: &str, sort_asc: bool) -> Vec<Track> {
    let all_songs = if filter == LibraryFilter::Songs {
        all_library_songs()
    } else {
        Vec::new()
    };
    library_signal().with(|l| l.tracks_for_filter(filter, all_songs, query, sort_asc))
}

pub fn cache_stats() -> CacheStats {
    library_signal().with(|l| l.cache_stats)
}

pub fn clear_caches() {
    library_signal().update(|l| l.clear_transient_caches());
    ui_signal().update(|u| u.set_status_banner("Cleared audio, artwork, and metadata caches"));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::ClearCaches);
    }
}

pub fn downloads_list() -> Vec<DownloadRecord> {
    library_signal().with(|l| l.downloads_list())
}

pub fn download_record(video_id: &str) -> Option<DownloadRecord> {
    library_signal().with(|l| l.downloads.get(video_id).cloned())
}

pub fn enqueue_download(track: Track) {
    if is_fixture_mode() {
        library_signal().update(|l| l.enqueue_fixture_download(track));
        return;
    }
    live_engine::dispatch(EngineCommand::EnqueueDownload(track));
}

pub fn pause_download(video_id: &str) {
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::PauseDownload(video_id.to_owned()));
    }
}

pub fn resume_download(video_id: &str) {
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::ResumeDownload(video_id.to_owned()));
    }
}

pub fn cancel_download(video_id: &str) {
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::CancelDownload(video_id.to_owned()));
    }
}

pub fn delete_download(video_id: &str) {
    if is_fixture_mode() {
        library_signal().update(|l| {
            l.downloads.remove(video_id);
        });
        return;
    }
    live_engine::dispatch(EngineCommand::DeleteDownload(video_id.to_owned()));
}

pub fn pinned_collections() -> Vec<PinnedCollection> {
    library_signal().with(|l| l.pinned_collections.clone())
}

pub fn pin_collection(
    id: String,
    kind: PinnedCollectionKind,
    title: String,
    subtitle: Option<String>,
    artwork_url: Option<String>,
    tracks: Vec<Track>,
) {
    if is_fixture_mode() {
        let now = resonance_core::unix_secs();
        library_signal().update(|l| {
            l.pinned_collections.retain(|c| c.id != id);
            l.pinned_collections.push(PinnedCollection {
                id,
                kind,
                title,
                subtitle,
                artwork_url,
                pinned_at_unix_secs: now,
                tracks,
            });
        });
        return;
    }
    live_engine::dispatch(EngineCommand::PinCollection {
        id,
        kind,
        title,
        subtitle,
        artwork_url,
        tracks,
    });
}

pub fn unpin_collection(collection_id: &str) {
    if is_fixture_mode() {
        library_signal().update(|l| {
            l.pinned_collections.retain(|c| c.id != collection_id);
        });
        return;
    }
    live_engine::dispatch(EngineCommand::UnpinCollection(collection_id.to_owned()));
}

// ── Optimistic Mutations ─────────────────────────────────────────────────────

pub fn add_track_to_playlist(playlist_id: String, playlist_title: String, track: Track) {
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let op = PlaylistEditOp::AddVideo {
        video_id: track.id.clone(),
    };
    let mut dispatch_cmd = None;
    mutations_signal().update(|m| {
        catalog_signal().update(|c| {
            library_signal().update(|l| {
                dispatch_cmd =
                    m.apply_playlist_edit(&playlist_id, &op, Some(track), c, l, live, signed_in);
            });
        });
    });
    ui_signal().update(|u| {
        u.add_to_playlist_track = None;
        u.set_status_banner(format!("Added to {playlist_title}"));
    });
    if let Some((playlist_id, op)) = dispatch_cmd {
        live_engine::dispatch(EngineCommand::EditPlaylist { playlist_id, op });
    }
}

pub fn remove_track_from_playlist(playlist_id: String, video_id: String) {
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let op = PlaylistEditOp::RemoveVideo {
        video_id: video_id.clone(),
        set_video_id: video_id,
    };
    let mut dispatch_cmd = None;
    mutations_signal().update(|m| {
        catalog_signal().update(|c| {
            library_signal().update(|l| {
                dispatch_cmd =
                    m.apply_playlist_edit(&playlist_id, &op, None, c, l, live, signed_in);
            });
        });
    });
    ui_signal().update(|u| u.set_status_banner("Removed from playlist"));
    if let Some((playlist_id, op)) = dispatch_cmd {
        live_engine::dispatch(EngineCommand::EditPlaylist { playlist_id, op });
    }
}

pub fn move_playlist_track(
    playlist_id: String,
    video_id: String,
    predecessor_video_id: Option<String>,
) {
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let op = PlaylistEditOp::MoveVideoBefore {
        set_video_id: video_id,
        predecessor_set_video_id: predecessor_video_id,
    };
    let mut dispatch_cmd = None;
    mutations_signal().update(|m| {
        catalog_signal().update(|c| {
            library_signal().update(|l| {
                dispatch_cmd =
                    m.apply_playlist_edit(&playlist_id, &op, None, c, l, live, signed_in);
            });
        });
    });
    if let Some((playlist_id, op)) = dispatch_cmd {
        live_engine::dispatch(EngineCommand::EditPlaylist { playlist_id, op });
    }
}

pub fn toggle_like_current() {
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let mut dispatch_cmd = None;
    mutations_signal().update(|m| {
        playback_signal().update(|p| {
            library_signal().update(|l| {
                dispatch_cmd = m.toggle_like_current(p, l, live, signed_in);
            });
        });
    });
    if let Some((video_id, action)) = dispatch_cmd {
        live_engine::dispatch(EngineCommand::SetLike { video_id, action });
    }
}

pub fn toggle_like_track(track: Track) {
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let mut dispatch_cmd = None;
    mutations_signal().update(|m| {
        playback_signal().update(|p| {
            library_signal().update(|l| {
                dispatch_cmd = m.toggle_like_track(track, p, l, live, signed_in);
            });
        });
    });
    if let Some((video_id, action)) = dispatch_cmd {
        live_engine::dispatch(EngineCommand::SetLike { video_id, action });
    }
}

pub fn toggle_dislike_current() {
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let mut dispatch_cmd = None;
    let mut banner = None;
    mutations_signal().update(|m| {
        playback_signal().update(|p| {
            library_signal().update(|l| {
                let (cmd, msg) = m.toggle_dislike_current(p, l, live, signed_in);
                dispatch_cmd = cmd;
                banner = msg;
            });
        });
    });
    if let Some(msg) = banner {
        ui_signal().update(|u| u.set_status_banner(msg));
    }
    if let Some((video_id, action)) = dispatch_cmd {
        live_engine::dispatch(EngineCommand::SetLike { video_id, action });
    }
}
