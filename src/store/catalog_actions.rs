//! Catalog, Search, Library, Offline Vault & Optimistic Mutation actions
//! operating on `catalog_signal()`, `library_signal()`, and `mutations_signal()`.

use std::time::Duration;

use nsh_core::prelude::{SharedString, spawn, timer};
use resonance_contract::{
    AlbumDetail, ArtistDetail, ArtistRef, CacheStats, DownloadRecord, ExplorePage, HistoryEntry,
    HomeItem, HomeSection, LibraryCollectionRef, MoodCategory, Playlist, PlaylistEditOp, Track,
};
use resonance_core::fixture::{fixture_home_sections, fixture_tracks};
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

pub fn explore_loading() -> bool {
    catalog_signal().with(|c| c.explore_loading)
}

pub fn explore_error() -> Option<String> {
    catalog_signal().with(|c| c.explore_error.clone())
}

pub fn refresh_explore() {
    if is_fixture_mode() {
        catalog_signal().update(|c| {
            c.explore_loading = false;
            c.explore_error = None;
        });
        return;
    }
    catalog_signal().update(|c| {
        c.explore_loading = true;
        c.explore_error = None;
    });
    live_engine::dispatch(EngineCommand::LoadExplore);
}

pub fn mood_playlists() -> Vec<LibraryCollectionRef> {
    catalog_signal().with(|c| c.mood_playlists.clone())
}

pub fn clear_mood_playlists() {
    catalog_signal().update(|c| {
        c.selected_mood = None;
        c.mood_playlists.clear();
        c.mood_generation = c.mood_generation.wrapping_add(1);
    });
}

pub fn load_mood_playlists(mood: &str, params: &str) {
    let mut generation = 0u64;
    catalog_signal().update(|c| {
        c.selected_mood = Some(mood.to_owned());
        c.mood_generation = c.mood_generation.wrapping_add(1);
        generation = c.mood_generation;
    });
    if is_fixture_mode() {
        let collections: Vec<LibraryCollectionRef> = fixture_home_sections()
            .into_iter()
            .flat_map(|sec| sec.items)
            .filter_map(|item| match item {
                HomeItem::Collection {
                    browse_id,
                    title,
                    subtitle,
                    artwork_url,
                } => Some(LibraryCollectionRef {
                    browse_id,
                    playlist_id: None,
                    title,
                    subtitle: format!("{mood} • {subtitle}"),
                    artwork_url,
                    track_count: None,
                }),
                HomeItem::Track(_) => None,
            })
            .collect();
        catalog_signal().update(|c| {
            c.apply_mood_playlists_gen(generation, Some(mood.to_owned()), Ok(collections));
        });
    } else {
        live_engine::dispatch(EngineCommand::LoadMoodPlaylists {
            generation,
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
    let (generation, query, category, cont) = catalog_signal().with(|c| {
        (
            c.search_generation,
            c.search_query.clone(),
            c.search_filter.as_search_category(),
            c.search_continuation.clone(),
        )
    });
    if let Some(continuation) = cont {
        catalog_signal().update(|c| c.search_loading = true);
        live_engine::dispatch(EngineCommand::SearchContinuation {
            generation,
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

/// Filters or reloads the Home feed in-place for the selected YouTube Music mood chip.
/// Tapping the active mood toggles back to the default Home feed without leaving Home.
pub fn select_home_mood(mood: &str, params: Option<&str>) {
    let toggle_off = catalog_signal().with(|c| c.selected_mood.as_deref() == Some(mood));
    if toggle_off {
        clear_home_mood();
        return;
    }

    let resolved_params = params
        .filter(|p| !p.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            catalog_signal().with(|c| {
                c.home_mood_chips
                    .iter()
                    .find(|chip| chip.title.eq_ignore_ascii_case(mood))
                    .map(|chip| chip.params.clone())
                    .filter(|p| !p.trim().is_empty())
            })
        });

    if is_fixture_mode() {
        let m_low = mood.to_lowercase();
        let filtered: Vec<HomeSection> = fixture_home_sections()
            .into_iter()
            .filter_map(|mut sec| {
                let matching: Vec<HomeItem> = sec
                    .items
                    .iter()
                    .filter(|item| match item {
                        HomeItem::Track(t) => {
                            t.title.to_lowercase().contains(&m_low)
                                || t.artist_display().to_lowercase().contains(&m_low)
                                || t.album
                                    .as_ref()
                                    .is_some_and(|a| a.title.to_lowercase().contains(&m_low))
                        }
                        HomeItem::Collection {
                            title, subtitle, ..
                        } => {
                            title.to_lowercase().contains(&m_low)
                                || subtitle.to_lowercase().contains(&m_low)
                        }
                    })
                    .cloned()
                    .collect();
                if !matching.is_empty() {
                    sec.items = matching;
                    Some(sec)
                } else {
                    None
                }
            })
            .collect();
        let next_sections = if filtered.is_empty() {
            fixture_home_sections()
        } else {
            filtered
        };
        catalog_signal().update(|c| {
            c.selected_mood = Some(mood.to_owned());
            c.home_sections = next_sections;
            c.home_loading = false;
            c.home_error = None;
        });
    } else {
        catalog_signal().update(|c| {
            c.selected_mood = Some(mood.to_owned());
            c.home_loading = true;
            c.home_error = None;
        });
        live_engine::dispatch(EngineCommand::LoadHomeWithMood {
            mood: Some(mood.to_owned()),
            params: resolved_params,
        });
    }
}

pub fn clear_home_mood() {
    if is_fixture_mode() {
        catalog_signal().update(|c| {
            c.selected_mood = None;
            c.home_sections = fixture_home_sections();
            c.home_loading = false;
            c.home_error = None;
        });
    } else {
        catalog_signal().update(|c| {
            c.selected_mood = None;
            c.home_loading = true;
            c.home_error = None;
        });
        live_engine::dispatch(EngineCommand::LoadHome);
    }
}

pub fn select_mood(mood: &str) {
    select_home_mood(mood, None);
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
    let mut live_dispatch: Option<(u64, String, Option<&'static str>)> = None;
    catalog_signal().update(|c| {
        c.set_search_filter(filter);
        if !is_fixture_mode() && !c.search_query.is_empty() {
            c.search_loading = true;
            c.search_error = None;
            c.search_generation = c.search_generation.wrapping_add(1);
            live_dispatch = Some((
                c.search_generation,
                c.search_query.clone(),
                filter.params(),
            ));
        }
    });
    if let Some((generation, query, params)) = live_dispatch {
        live_engine::dispatch(EngineCommand::Search {
            generation,
            query,
            params,
        });
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
        let mut generation = 0u64;
        let mut params = None;
        let should_dispatch = !q.is_empty();
        catalog_signal().update(|c| {
            c.begin_live_search(q.clone());
            generation = c.search_generation;
            if should_dispatch {
                params = c.search_filter.params();
            }
        });
        if should_dispatch {
            live_engine::dispatch(EngineCommand::Search {
                generation,
                query: q,
                params,
            });
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
            live_engine::dispatch(EngineCommand::Search {
                generation: seq,
                query: q,
                params,
            });
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
    let mut generation = 0u64;
    catalog_signal().update(|c| {
        c.open_collection(fixture, browse_id, title, subtitle, artwork_url);
        generation = c.collection_generation;
    });
    if !fixture {
        live_engine::dispatch(EngineCommand::BrowseCollection {
            generation,
            browse_id: live_id,
        });
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

pub fn entity_loading() -> bool {
    catalog_signal().with(|c| c.entity_loading)
}

pub fn entity_error() -> Option<String> {
    catalog_signal().with(|c| c.entity_error.clone())
}

pub fn open_artist_entity(channel_id: String) {
    let fixture = is_fixture_mode();
    let mut generation = 0u64;
    catalog_signal().update(|c| {
        c.push_entity_route(EntityRoute::Artist {
            channel_id: channel_id.clone(),
        });
        c.entity_generation = c.entity_generation.wrapping_add(1);
        generation = c.entity_generation;
        c.entity_error = None;
        if fixture {
            c.entity_loading = false;
            let tracks = fixture_tracks();
            let matched: Vec<Track> = tracks
                .iter()
                .filter(|t| {
                    t.artists
                        .iter()
                        .any(|a| a.id.as_deref() == Some(channel_id.as_str()))
                })
                .cloned()
                .collect();
            let top_songs = if matched.is_empty() { tracks } else { matched };
            let name = top_songs
                .first()
                .map(|t| t.artist_display())
                .unwrap_or_else(|| "Artist".to_owned());
            let art = top_songs.first().and_then(|t| t.artwork_url.clone());
            c.active_artist = Some(ArtistDetail {
                channel_id: channel_id.clone(),
                name,
                description: Some("Fixture preview artist discography.".to_owned()),
                subscribers: Some("Fixture Artist".to_owned()),
                is_subscribed: false,
                artwork_url: art,
                radio_playlist_id: None,
                shuffle_playlist_id: None,
                top_songs,
                albums: Vec::new(),
                singles: Vec::new(),
                related_artists: Vec::new(),
            });
        } else {
            c.entity_loading = true;
            c.active_artist = None;
        }
    });
    if !fixture {
        live_engine::dispatch(EngineCommand::LoadArtistEntity {
            generation,
            channel_id,
        });
    }
}

pub fn open_album_entity(browse_id: String) {
    let fixture = is_fixture_mode();
    let mut generation = 0u64;
    catalog_signal().update(|c| {
        c.push_entity_route(EntityRoute::Album {
            browse_id: browse_id.clone(),
        });
        c.entity_generation = c.entity_generation.wrapping_add(1);
        generation = c.entity_generation;
        c.entity_error = None;
        if fixture {
            c.entity_loading = false;
            let tracks = fixture_tracks();
            let total_duration_ms = tracks.iter().map(|t| t.duration_ms).sum();
            let first = tracks.first();
            let title = first
                .and_then(|t| t.album.as_ref().map(|a| a.title.clone()))
                .unwrap_or_else(|| "Album".to_owned());
            let artists = first
                .map(|t| t.artists.clone())
                .unwrap_or_else(|| {
                    vec![ArtistRef {
                        id: None,
                        name: "Various Artists".to_owned(),
                    }]
                });
            let artwork_url = first.and_then(|t| t.artwork_url.clone());
            c.active_album = Some(AlbumDetail {
                browse_id: browse_id.clone(),
                audio_playlist_id: None,
                title,
                artists,
                release_year: Some("2025".to_owned()),
                release_type: "Album".to_owned(),
                description: Some("Fixture preview album.".to_owned()),
                artwork_url,
                total_duration_ms,
                tracks,
                other_versions: Vec::new(),
            });
        } else {
            c.entity_loading = true;
            c.active_album = None;
        }
    });
    if !fixture {
        live_engine::dispatch(EngineCommand::LoadAlbumEntity {
            generation,
            browse_id,
        });
    }
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

fn parse_playlist_privacy(label: &str) -> resonance_contract::PlaylistPrivacy {
    match label.trim().to_ascii_lowercase().as_str() {
        "public" => resonance_contract::PlaylistPrivacy::Public,
        "unlisted" => resonance_contract::PlaylistPrivacy::Unlisted,
        _ => resonance_contract::PlaylistPrivacy::Private,
    }
}

pub fn create_local_playlist_and_add(title: &str, privacy: &str, track: Track) {
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let mut created_id = String::new();
    let mut created_title = String::new();
    mutations_signal().update(|m| {
        library_signal().update(|l| {
            let created = m.create_local_playlist(title, privacy, l);
            created_id = created.browse_id.clone();
            created_title = created.title.clone();
            if let Some(pl) = l.playlists.first_mut() {
                pl.track_count = Some(1);
            }
            if live && signed_in {
                m.pending_mutations += 1;
            }
        });
    });
    let track_id = track.id.clone();
    ui_signal().update(|u| {
        u.add_to_playlist_track = None;
        u.set_status_banner(if live && signed_in {
            format!("Created \"{created_title}\" and added \"{}\"", track.title)
        } else {
            format!(
                "Created local playlist \"{created_title}\" and added \"{}\"",
                track.title
            )
        });
    });
    if live && signed_in {
        live_engine::dispatch(EngineCommand::CreatePlaylist {
            local_temp_id: created_id,
            title: created_title,
            description: String::new(),
            privacy: parse_playlist_privacy(privacy),
            initial_video_ids: vec![track_id],
        });
    }
}

pub fn create_playlist(title: &str, privacy: &str) {
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let mut created_id = String::new();
    let mut created_title = String::new();
    mutations_signal().update(|m| {
        library_signal().update(|l| {
            let created = m.create_local_playlist(title, privacy, l);
            created_id = created.browse_id.clone();
            created_title = created.title.clone();
            if live && signed_in {
                m.pending_mutations += 1;
            }
        });
    });
    ui_signal().update(|u| {
        u.set_status_banner(format!("Created playlist \"{created_title}\""));
    });
    if live && signed_in {
        live_engine::dispatch(EngineCommand::CreatePlaylist {
            local_temp_id: created_id,
            title: created_title,
            description: String::new(),
            privacy: parse_playlist_privacy(privacy),
            initial_video_ids: Vec::new(),
        });
    }
}

pub fn save_queue_as_playlist() {
    let tracks = playback_signal().with(|p| p.player.queue.items.clone());
    if tracks.is_empty() {
        set_status_banner("Queue is empty");
        return;
    }
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let title = tracks
        .first()
        .map(|t| format!("Queue Mix — {}", t.title))
        .unwrap_or_else(|| "Saved Queue".to_owned());
    let video_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();
    let track_count = video_ids.len();

    let mut created_id = String::new();
    let mut created_title = String::new();
    mutations_signal().update(|m| {
        library_signal().update(|l| {
            let created = m.create_local_playlist(&title, "private", l);
            created_id = created.browse_id.clone();
            created_title = created.title.clone();
            if let Some(pl) = l.playlists.first_mut() {
                pl.track_count = Some(track_count);
            }
            if live && signed_in {
                m.pending_mutations += 1;
            }
        });
    });

    ui_signal().update(|u| {
        u.set_status_banner(if live && signed_in {
            format!("Saved {track_count} tracks to \"{created_title}\" (syncing)")
        } else {
            format!("Saved {track_count} tracks to local playlist \"{created_title}\"")
        });
    });

    if live && signed_in {
        live_engine::dispatch(EngineCommand::CreatePlaylist {
            local_temp_id: created_id,
            title: created_title,
            description: "Saved from Raag playback queue".to_owned(),
            privacy: resonance_contract::PlaylistPrivacy::Private,
            initial_video_ids: video_ids,
        });
    }
}

pub fn rename_playlist(playlist_id: String, new_title: String) {
    let clean = new_title.trim().to_owned();
    if clean.is_empty() {
        return;
    }
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let mut renamed = false;
    mutations_signal().update(|m| {
        library_signal().update(|l| {
            renamed = m.rename_local_playlist(&playlist_id, &clean, l);
            if renamed && live && signed_in {
                m.pending_mutations += 1;
            }
        });
    });
    if renamed {
        catalog_signal().update(|c| {
            if let Some(col) = c.selected_collection.as_mut()
                && col.id == playlist_id
            {
                col.title = clean.clone();
            }
        });
        ui_signal().update(|u| u.set_status_banner(format!("Renamed playlist to \"{clean}\"")));
        if live && signed_in {
            live_engine::dispatch(EngineCommand::RenamePlaylist {
                playlist_id,
                new_title: clean,
            });
        }
    }
}

pub fn delete_playlist(playlist_id: String) {
    let live = !is_fixture_mode();
    let signed_in = settings_signal().with(|s| s.signed_in);
    let mut deleted = false;
    mutations_signal().update(|m| {
        library_signal().update(|l| {
            deleted = m.delete_local_playlist(&playlist_id, l);
            if deleted && live && signed_in {
                m.pending_mutations += 1;
            }
        });
    });
    if deleted {
        catalog_signal().update(|c| {
            if c.selected_collection
                .as_ref()
                .is_some_and(|col| col.id == playlist_id)
            {
                c.selected_collection = None;
            }
        });
        ui_signal().update(|u| u.set_status_banner("Deleted playlist"));
        if live && signed_in {
            live_engine::dispatch(EngineCommand::DeletePlaylist { playlist_id });
        }
    }
}

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
