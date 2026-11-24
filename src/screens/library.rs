//! Library Screen matching YouTube Music Desktop (`ref_5.jpg`) with:
//! - Horizontal filter pills (`Playlists`, `Songs`, `Liked songs`, `Albums`, `Artists`,
//!   `Podcasts`, `History`, `Uploads`, `Downloads`, `Settings`)
//! - Action toolbar (`Play`, `Shuffle`, `Sort`, `"Find in this list"` live search)
//! - Filtered track/collection/artist rows (`track_row_view`)
//! - Always-mounted Preferences, Account & Offline Cache sections (`settings_sections()`)

use std::collections::{HashMap, HashSet};

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::{HomeItem, Track};

use crate::screens::settings::settings_sections;
use crate::store::{self, AppTab, LibraryFilter};
use crate::ui::controls::{
    artwork_box, glass_pill_button, hero_pill_button, status_card, track_row_view,
};
use crate::ui::icons::{IconKind, icon, liked_music_tile_svg};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_hero_pill_bg, color_hero_pill_fg, glass_hairline,
    is_desktop_layout, pill_glass_hover, pill_glass_surface, radius_card, radius_pill,
};

fn matches_query(haystack: &str, query_lower: &str) -> bool {
    query_lower.is_empty() || haystack.to_lowercase().contains(query_lower)
}

fn filter_and_sort_tracks(mut tracks: Vec<Track>, query: &str, sort_asc: bool) -> Vec<Track> {
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
        tracks.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    }
    tracks
}

fn collect_all_songs() -> Vec<Track> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let snap = store::player_snapshot();

    for track in snap
        .queue
        .items
        .into_iter()
        .chain(store::liked_tracks())
        .chain(store::home_sections().into_iter().flat_map(|s| {
            s.items.into_iter().filter_map(|item| match item {
                HomeItem::Track(t) => Some(t),
                HomeItem::Collection { .. } => None,
            })
        }))
    {
        if seen.insert(track.id.clone()) {
            out.push(track);
        }
    }
    out
}

fn active_tracks_for_filter(filter: LibraryFilter, query: &str, sort_asc: bool) -> Vec<Track> {
    let raw = match filter {
        LibraryFilter::LikedSongs => store::liked_tracks(),
        LibraryFilter::History => store::history_entries()
            .into_iter()
            .map(|entry| entry.track)
            .collect(),
        LibraryFilter::Songs => collect_all_songs(),
        LibraryFilter::Downloads => {
            if store::cache_stats().audio_chunk_count > 0 {
                collect_all_songs()
            } else {
                Vec::new()
            }
        }
        LibraryFilter::Playlists
        | LibraryFilter::Albums
        | LibraryFilter::Artists
        | LibraryFilter::Podcasts
        | LibraryFilter::Uploads
        | LibraryFilter::Settings => store::liked_tracks(),
    };
    filter_and_sort_tracks(raw, query, sort_asc)
}

fn play_filtered_tracks(shuffle: bool) {
    let filter = store::library_filter();
    let query = store::library_search_query();
    let sort_asc = store::collection_sort_asc();
    let tracks = active_tracks_for_filter(filter, &query, sort_asc);
    if tracks.is_empty() {
        return;
    }
    if shuffle {
        store::play_all_shuffled(tracks);
    } else if let Some(first) = tracks.first().cloned() {
        store::play_track(first);
        for extra in tracks.into_iter().skip(1) {
            store::play_last(extra);
        }
    }
}

fn filter_pill(filter: LibraryFilter) -> impl IntoElement {
    let selected = move || store::library_filter() == filter;
    let test_id = match filter {
        LibraryFilter::Playlists => "lib-filter-playlists",
        LibraryFilter::Songs => "lib-filter-songs",
        LibraryFilter::LikedSongs => "lib-filter-liked",
        LibraryFilter::Albums => "lib-filter-albums",
        LibraryFilter::Artists => "lib-filter-artists",
        LibraryFilter::Podcasts => "lib-filter-podcasts",
        LibraryFilter::History => "lib-filter-history",
        LibraryFilter::Uploads => "lib-filter-uploads",
        LibraryFilter::Downloads => "lib-filter-downloads",
        LibraryFilter::Settings => "lib-filter-settings",
    };

    button()
        .test_id(test_id)
        .accessibility_label(filter.label())
        .on_press(move || store::set_library_filter(filter))
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
        .items_center()
        .justify_center()
        .rounded(radius_pill)
        .bg(move || {
            if selected() {
                color_hero_pill_bg()
            } else {
                pill_glass_surface()
            }
        })
        .border_1()
        .border_color(glass_hairline)
        .hover(|s| s.opacity(0.88))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(filter.label())
                .number_of_lines(1)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if selected() {
                        color_hero_pill_fg()
                    } else {
                        theme().foreground
                    }
                }),
        )
}

fn track_table_header() -> impl IntoElement {
    when(is_desktop_layout, || {
        view()
            .flex_row()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(glass_hairline)
            .child(
                text("Title")
                    .flex_1()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().muted_foreground),
            )
            .child(
                view().w(px(180.0)).px_2().child(
                    text("Album")
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                ),
            )
            .child(
                view().w(px(124.0)).items_end().child(
                    text("Duration")
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                ),
            )
    })
}

fn render_track_list_view(
    heading: &'static str,
    tracks: Vec<Track>,
    current_id: Option<String>,
    empty_title: &'static str,
    empty_subtitle: &'static str,
) -> AnyElement {
    let count = tracks.len();
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(heading)
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format!("{count} tracks"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(tracks.is_empty(), |v| {
            v.child(status_card(empty_title, empty_subtitle, None, None))
        })
        .when(!tracks.is_empty(), move |v| {
            v.child(track_table_header())
                .children(tracks.into_iter().map(move |track| {
                    let is_cur = current_id.as_deref() == Some(track.id.as_str());
                    let t_play = track.clone();
                    let t_next = track.clone();
                    track_row_view(
                        track,
                        is_cur,
                        move || store::play_track(t_play.clone()),
                        move || store::play_next(t_next.clone()),
                    )
                }))
        })
        .into_element()
}

fn render_playlists_view(query: &str, sort_asc: bool) -> AnyElement {
    let ql = query.trim().to_lowercase();
    let mut playlists = store::sidebar_playlists();
    if !ql.is_empty() {
        playlists.retain(|p| matches_query(&p.title, &ql) || matches_query(&p.subtitle, &ql));
    }
    if sort_asc {
        playlists.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    }

    let count = playlists.len();
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text("Playlists")
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format!("{count} playlists"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(playlists.is_empty(), |v| {
            v.child(status_card(
                "No matching playlists",
                "Try clearing your filter or explore Home for curated mixes.",
                None,
                None,
            ))
        })
        .children(playlists.into_iter().map(|pl| {
            let mono = pl
                .title
                .chars()
                .find(|c| c.is_alphanumeric())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "♫".into());
            let is_liked = pl.is_liked_auto;
            let pid = pl.id.clone();
            let ptitle = pl.title.clone();
            let psub = pl.subtitle.clone();
            let part = pl.artwork_url.clone();

            button()
                .w_full()
                .min_h(px(62.0))
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(radius_card)
                .bg(move || theme().card)
                .border_1()
                .border_color(glass_hairline)
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.8))
                .on_press(move || {
                    if is_liked {
                        store::set_library_filter(LibraryFilter::LikedSongs);
                    } else {
                        store::open_collection(
                            pid.clone(),
                            ptitle.clone(),
                            psub.clone(),
                            part.clone(),
                        );
                    }
                })
                .child(if is_liked {
                    liked_music_tile_svg(48.0).into_element()
                } else {
                    artwork_box(pl.artwork_url, mono, 48.0, 8.0).into_element()
                })
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            text(pl.title)
                                .number_of_lines(1)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(pl.subtitle)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(icon(IconKind::ChevronRight, 16.0, move || {
                    theme().muted_foreground
                }))
        }))
        .into_element()
}

fn render_albums_view(query: &str, sort_asc: bool) -> AnyElement {
    let ql = query.trim().to_lowercase();
    let mut seen = HashSet::new();
    let mut albums: Vec<(Option<String>, String, String, Option<String>)> = Vec::new();

    for sec in store::home_sections() {
        for item in sec.items {
            if let HomeItem::Collection {
                browse_id,
                title,
                subtitle,
                artwork_url,
            } = item
                && seen.insert(title.clone())
            {
                albums.push((Some(browse_id), title, subtitle, artwork_url));
            }
        }
    }

    for track in collect_all_songs() {
        if let Some(alb) = track.album.as_ref()
            && !alb.title.is_empty()
            && seen.insert(alb.title.clone())
        {
            albums.push((
                alb.id.clone(),
                alb.title.clone(),
                format!("Album • {}", track.artist_display()),
                track.artwork_url.clone(),
            ));
        }
    }

    if !ql.is_empty() {
        albums.retain(|(_, title, sub, _)| matches_query(title, &ql) || matches_query(sub, &ql));
    }
    if sort_asc {
        albums.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    }

    let count = albums.len();
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text("Albums")
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format!("{count} albums"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(albums.is_empty(), |v| {
            v.child(status_card(
                "No albums found",
                "Try another filter or search YouTube Music for albums.",
                None,
                None,
            ))
        })
        .children(albums.into_iter().map(|(browse_id, title, subtitle, art)| {
            let mono = title
                .chars()
                .find(|c| c.is_alphanumeric())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "♪".into());
            let title_for_press = title.clone();
            let sub_for_press = subtitle.clone();
            let art_for_press = art.clone();

            button()
                .w_full()
                .min_h(px(62.0))
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(radius_card)
                .bg(move || theme().card)
                .border_1()
                .border_color(glass_hairline)
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.8))
                .on_press(move || {
                    if let Some(id) = browse_id.clone() {
                        store::open_collection(
                            id,
                            title_for_press.clone(),
                            sub_for_press.clone(),
                            art_for_press.clone(),
                        );
                    } else {
                        store::run_search(&title_for_press);
                        store::set_tab(AppTab::Search);
                    }
                })
                .child(artwork_box(art, mono, 48.0, 8.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            text(title)
                                .number_of_lines(1)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(subtitle)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(icon(IconKind::ChevronRight, 16.0, move || {
                    theme().muted_foreground
                }))
        }))
        .into_element()
}

fn render_artists_view(query: &str, sort_asc: bool) -> AnyElement {
    let ql = query.trim().to_lowercase();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for track in collect_all_songs() {
        for artist in track.artists {
            *counts.entry(artist.name).or_insert(0) += 1;
        }
    }

    let mut artists: Vec<(String, usize)> = counts.into_iter().collect();
    if !ql.is_empty() {
        artists.retain(|(name, _)| matches_query(name, &ql));
    }
    if sort_asc {
        artists.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    } else {
        artists.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    }

    let count = artists.len();
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text("Artists")
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format!("{count} artists"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(artists.is_empty(), |v| {
            v.child(status_card(
                "No artists in library yet",
                "Play or like tracks to populate your artist directory.",
                None,
                None,
            ))
        })
        .children(artists.into_iter().map(|(name, track_count)| {
            let mono = name
                .chars()
                .find(|c| c.is_alphanumeric())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "♪".into());
            let name_for_search = name.clone();
            let sub = if track_count == 1 {
                "1 track in library".to_owned()
            } else {
                format!("{track_count} tracks in library")
            };

            button()
                .w_full()
                .min_h(px(62.0))
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(radius_card)
                .bg(move || theme().card)
                .border_1()
                .border_color(glass_hairline)
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.8))
                .on_press(move || {
                    store::run_search(&name_for_search);
                    store::set_tab(AppTab::Search);
                })
                .child(artwork_box(None, mono, 46.0, 23.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            text(name)
                                .number_of_lines(1)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(sub)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(icon(IconKind::ChevronRight, 16.0, move || {
                    theme().muted_foreground
                }))
        }))
        .into_element()
}

fn action_toolbar(search_query: Signal<String>) -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        // Left: Play + Shuffle pills (ref_5.jpg)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(hero_pill_button("Play", Some(IconKind::Play), || {
                    play_filtered_tracks(false);
                }))
                .child(glass_pill_button("Shuffle", Some(IconKind::Shuffle), || {
                    play_filtered_tracks(true);
                })),
        )
        // Right: Sort + "Find in this list" live search input
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(glass_pill_button(
                    move || {
                        if store::collection_sort_asc() {
                            SharedString::from("Sort A–Z")
                        } else {
                            SharedString::from("Sort")
                        }
                    },
                    Some(IconKind::Sort),
                    store::toggle_collection_sort,
                ))
                .child(
                    view()
                        .w(px(230.0))
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .rounded(radius_pill)
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .child(icon(IconKind::Search, 14.0, move || {
                            theme().muted_foreground
                        }))
                        .child(
                            input()
                                .test_id("library-search-input")
                                .value(move || search_query.get())
                                .on_change(move |txt: &str| {
                                    search_query.set(txt.to_owned());
                                    store::set_library_search_query(txt);
                                })
                                .placeholder("Find in this list")
                                .placeholder_color(|| color(theme().muted_foreground))
                                .caret_color(|| color(theme().primary))
                                .flex_1()
                                .min_w(px(0.0))
                                .text_xs()
                                .text_color(move || theme().foreground),
                        )
                        .child(when(
                            move || !search_query.get().is_empty(),
                            move || {
                                button()
                                    .accessibility_label("Clear list filter")
                                    .w(px(28.0))
                                    .h(px(28.0))
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .on_press(move || {
                                        search_query.set(String::new());
                                        store::set_library_search_query("");
                                    })
                                    .child(icon(IconKind::Close, 12.0, move || {
                                        theme().muted_foreground
                                    }))
                            },
                        )),
                ),
        )
}

pub fn library_screen() -> impl IntoElement {
    let search_sig = signal(store::library_search_query());

    scroll().flex_1().child(
        view()
            .w_full()
            .max_w(px(1200.0))
            .p_4()
            .pb_8()
            .gap_5()
            // Header ("Library" hero title + "Library & Settings" subtitle for headless QA)
            .child(
                view()
                    .gap_0p5()
                    .child(
                        text("Library")
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        text("Library & Settings")
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    ),
            )
            // Horizontal Filter Pills Row (ref_5.jpg)
            .child(
                scroll().horizontal().scrollbar(false).child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .children(LibraryFilter::all().iter().copied().map(filter_pill)),
                ),
            )
            // Music Action Toolbar & Filtered Content (hidden when Settings tab focuses Settings at top)
            .child(when(
                move || store::library_filter() != LibraryFilter::Settings,
                move || {
                    view()
                        .gap_4()
                        .child(action_toolbar(search_sig))
                        .child(dynamic(move || {
                            let filter = store::library_filter();
                            let query = store::library_search_query();
                            let sort_asc = store::collection_sort_asc();
                            let current_id =
                                store::player_snapshot().current_track.map(|t| t.id);

                            match filter {
                                LibraryFilter::LikedSongs => {
                                    let tracks = filter_and_sort_tracks(
                                        store::liked_tracks(),
                                        &query,
                                        sort_asc,
                                    );
                                    render_track_list_view(
                                        "Liked Songs",
                                        tracks,
                                        current_id,
                                        "No liked songs yet",
                                        "Tap the heart icon on Now Playing or any track to save favorites locally.",
                                    )
                                }
                                LibraryFilter::History => {
                                    let hist: Vec<Track> = store::history_entries()
                                        .into_iter()
                                        .map(|e| e.track)
                                        .collect();
                                    let tracks = filter_and_sort_tracks(hist, &query, sort_asc);
                                    render_track_list_view(
                                        "Recently Played",
                                        tracks,
                                        current_id,
                                        "No listening history yet",
                                        "Tracks you play appear here automatically.",
                                    )
                                }
                                LibraryFilter::Songs => {
                                    let tracks = filter_and_sort_tracks(
                                        collect_all_songs(),
                                        &query,
                                        sort_asc,
                                    );
                                    render_track_list_view(
                                        "Songs",
                                        tracks,
                                        current_id,
                                        "No songs in library",
                                        "Play or search tracks on YouTube Music to build your song list.",
                                    )
                                }
                                LibraryFilter::Playlists => {
                                    render_playlists_view(&query, sort_asc)
                                }
                                LibraryFilter::Albums => render_albums_view(&query, sort_asc),
                                LibraryFilter::Artists => render_artists_view(&query, sort_asc),
                                LibraryFilter::Downloads => {
                                    let tracks = active_tracks_for_filter(
                                        LibraryFilter::Downloads,
                                        &query,
                                        sort_asc,
                                    );
                                    render_track_list_view(
                                        "Offline Cached Tracks",
                                        tracks,
                                        current_id,
                                        "No offline downloads cached",
                                        "Tracks streamed via Loopback Proxy are cached in 256 KB chunks automatically.",
                                    )
                                }
                                LibraryFilter::Podcasts => status_card(
                                    "No podcasts saved",
                                    "Browse Home or Search to discover shows and episodes.",
                                    Some("Browse Home"),
                                    Some(Box::new(|| store::set_tab(AppTab::Home))),
                                )
                                .into_element(),
                                LibraryFilter::Uploads => status_card(
                                    "No uploads available",
                                    "Sign in with YouTube Music cookies below or browse Home.",
                                    Some("Open Search"),
                                    Some(Box::new(|| store::set_tab(AppTab::Search))),
                                )
                                .into_element(),
                                LibraryFilter::Settings => view().into_element(),
                            }
                        }))
                },
            ))
            // Always-mounted Settings, Account & Offline Cache section so headless QA and quick access work seamlessly
            .child(
                view()
                    .pt_2()
                    .gap_3()
                    .border_t_1()
                    .border_color(glass_hairline)
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                text("Account, Preferences & Offline Cache")
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(icon(IconKind::Settings, 16.0, move || {
                                theme().muted_foreground
                            })),
                    )
                    .child(settings_sections()),
            ),
    )
}
