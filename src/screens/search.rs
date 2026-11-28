//! Search Screen matching `ytm-rs` (`08_search_01_focused.png` – `08_search_03_results_3.png`):
//! - Scope pills (`YT Music`, `Library`, `Uploads`) + category filter tabs (`All`, `Songs`, `Videos`, `Albums`, `Playlists`, `Artists`, `Podcasts`)
//! - Split Desktop top row: **Top result** frosted card on left + **Songs** quick list with `Play` pill on right
//! - Followed by **Artists** (circular cards), **Playlists & Albums** (square cards), and **Episodes** sections

use std::collections::HashSet;

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::{Track, format_duration_ms};

use crate::store::{self, AppTab, SearchFilter};
use crate::ui::controls::{
    TrackMenuContext, artwork_box, glass_pill_button, hero_pill_button, media_square_card,
    primary_button, set_text_input_focused, skeleton_row, status_card, track_context_menu,
};
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::text_format::{format_recoverable_error, pluralize, quote_term};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_hero_pill_bg, color_hero_pill_fg, color_liked,
    color_player_accent, frosted_settings_card, glass_hairline, is_desktop_layout,
    pill_glass_hover, pill_glass_surface, radius_card, radius_pill,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchScope {
    YtMusic,
    Library,
    Uploads,
}

impl SearchScope {
    fn label(self) -> &'static str {
        match self {
            Self::YtMusic => "YT Music",
            Self::Library => "Library",
            Self::Uploads => "Uploads",
        }
    }
}

fn scope_pill(scope: SearchScope, active_scope: Signal<SearchScope>) -> impl IntoElement {
    let is_active = move || active_scope.get() == scope;
    button()
        .accessibility_label(scope.label())
        .selected(is_active)
        .on_press(move || active_scope.set(scope))
        .min_h(move || {
            px(if is_desktop_layout() {
                34.0
            } else {
                MOBILE_MIN_TOUCH_TARGET
            })
        })
        .px_3p5()
        .items_center()
        .justify_center()
        .rounded(radius_pill)
        .bg(move || {
            if is_active() {
                color_hero_pill_bg()
            } else {
                pill_glass_surface()
            }
        })
        .border_1()
        .border_color(glass_hairline)
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.78))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(scope.label())
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if is_active() {
                        color_hero_pill_fg()
                    } else {
                        theme().foreground
                    }
                }),
        )
}

fn filter_tab(filter: SearchFilter, test_id: &'static str) -> impl IntoElement {
    let active = move || store::search_filter() == filter;
    button()
        .test_id(test_id)
        .accessibility_label(filter.label())
        .selected(active)
        .on_press(move || store::set_search_filter(filter))
        .min_h(move || {
            px(if is_desktop_layout() {
                34.0
            } else {
                MOBILE_MIN_TOUCH_TARGET
            })
        })
        .px_3p5()
        .items_center()
        .justify_center()
        .rounded(radius_pill)
        .bg(move || {
            if active() {
                color_hero_pill_bg()
            } else {
                pill_glass_surface()
            }
        })
        .border_1()
        .border_color(glass_hairline)
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.8))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(filter.label())
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if active() {
                        color_hero_pill_fg()
                    } else {
                        theme().foreground
                    }
                }),
        )
}

fn search_track_row(
    track: Track,
    current_id_memo: Memo<Option<String>>,
    desktop: bool,
    compact_quick: bool,
) -> impl IntoElement {
    let tid = track.id.clone();
    let tid_like = track.id.clone();
    let is_current = move || current_id_memo.get().as_deref() == Some(tid.as_str());
    let is_liked = move || store::is_track_liked(&tid_like);
    let plays_text = if desktop && !compact_quick {
        store::track_plays(&track.id)
    } else {
        None
    };

    let t_play = track.clone();
    let t_like = track.clone();
    let t_next = track.clone();
    let mono = track.fallback_monogram();
    let art_url = track.artwork_url.clone();
    let title = track.title.clone();
    let artist = track.artist_display();
    let album_title = if compact_quick {
        None
    } else {
        track.album.as_ref().map(|a| a.title.clone())
    };
    let dur_str = if track.duration_ms > 0 {
        format_duration_ms(track.duration_ms)
    } else {
        String::new()
    };
    let is_explicit = track.is_explicit;

    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_2p5()
        .py_1()
        .min_h(px(if compact_quick { 54.0 } else { 62.0 }))
        .rounded(radius_card)
        .style_when(is_current.clone(), |s| s.bg(pill_glass_surface))
        .hover(|s| s.bg(pill_glass_surface))
        .child(
            button()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(if compact_quick && desktop {
                    44.0
                } else {
                    MOBILE_MIN_TOUCH_TARGET
                }))
                .flex_row()
                .items_center()
                .gap_3()
                .active(|s| s.opacity(0.75))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(move || store::play_track(t_play.clone()))
                .child(artwork_box(
                    art_url,
                    mono,
                    if compact_quick { 40.0 } else { 44.0 },
                    6.0,
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .min_w(px(0.0))
                                .child(
                                    text(title)
                                        .flex_1()
                                        .number_of_lines(1)
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || {
                                            if is_current() {
                                                color_player_accent()
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                )
                                .when(is_explicit, |v| {
                                    v.child(
                                        view()
                                            .px_1()
                                            .rounded(px(4.0))
                                            .bg(move || theme().muted)
                                            .child(
                                                text("E")
                                                    .text_xs()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(move || theme().muted_foreground),
                                            ),
                                    )
                                }),
                        )
                        .child(
                            text(artist)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .when_some(if desktop { album_title } else { None }, |b, alb| {
                    b.child(
                        view().w(px(160.0)).px_2().child(
                            text(alb)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                    )
                })
                .when_some(plays_text, |b, plays| {
                    b.child(
                        view().w(px(88.0)).px_1().child(
                            text(plays)
                                .number_of_lines(1)
                                .text_xs()
                                .tabular_nums()
                                .text_color(move || theme().muted_foreground),
                        ),
                    )
                })
                .child(
                    text(dur_str)
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(desktop, move |row| {
            let is_liked_label = is_liked.clone();
            let is_liked_sel = is_liked.clone();
            row.child(
                button()
                    .accessibility_label(move || {
                        Some(SharedString::from(if is_liked_label() {
                            "Unlike track"
                        } else {
                            "Like track"
                        }))
                    })
                    .selected(is_liked_sel)
                    .w(px(34.0))
                    .h(px(34.0))
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .hover(|s| s.bg(pill_glass_hover))
                    .active(|s| s.opacity(0.75))
                    .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                    .on_press(move || store::toggle_like_track(t_like.clone()))
                    .child(reactive_icon(
                        {
                            let is_liked = is_liked.clone();
                            move || {
                                if is_liked() {
                                    IconKind::HeartFilled
                                } else {
                                    IconKind::Heart
                                }
                            }
                        },
                        15.0,
                        move || {
                            if is_liked() {
                                color_liked()
                            } else {
                                theme().muted_foreground
                            }
                        },
                    )),
            )
            .when(!compact_quick, |r| {
                r.child(
                    button()
                        .accessibility_label("Play Next")
                        .w(px(34.0))
                        .h(px(34.0))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.75))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(move || store::play_next(t_next.clone()))
                        .child(icon(IconKind::Plus, 15.0, move || theme().muted_foreground)),
                )
            })
        })
        .when(!compact_quick, |r| {
            r.child(track_context_menu(track, TrackMenuContext::General))
        })
}

fn top_result_card(top_track: Track) -> impl IntoElement {
    let title = top_track.title.clone();
    let artist = top_track.artist_display();
    let subtitle = format!("Song • {artist}");
    let mono = top_track.fallback_monogram();
    let art_url = top_track.artwork_url.clone();
    let t_play = top_track.clone();
    let t_radio = top_track;

    view()
        .p_5()
        .gap_4()
        .rounded(radius_card)
        .bg(frosted_settings_card)
        .border_1()
        .border_color(glass_hairline)
        .child(artwork_box(art_url, mono, 108.0, 54.0))
        .child(
            view()
                .flex_row()
                .items_end()
                .justify_between()
                .gap_3()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_1()
                        .child(
                            text(title)
                                .number_of_lines(1)
                                .text_2xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(subtitle)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(glass_pill_button("Radio", Some(IconKind::Radio), move || {
                            store::set_autoplay_radio(true);
                            store::play_track(t_radio.clone());
                        }))
                        .child(
                            button()
                                .accessibility_label("Play top result")
                                .w(px(46.0))
                                .h(px(46.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(color_hero_pill_bg)
                                .shadow_md()
                                .hover(|s| s.opacity(0.92))
                                .active(|s| s.opacity(0.8))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(move || store::play_track(t_play.clone()))
                                .child(icon(IconKind::Play, 18.0, color_hero_pill_fg)),
                        ),
                ),
        )
}

pub fn search_screen() -> impl IntoElement {
    let query_sig = signal(store::search_query());
    let active_scope = signal(SearchScope::YtMusic);
    let current_id_memo = computed(|| store::player_snapshot().current_track.map(|t| t.id));
    let desktop_memo = computed(is_desktop_layout);
    let tracks_memo = computed(store::search_tracks);
    let albums_memo = computed(store::search_albums);
    let filter_memo = computed(store::search_filter);

    // Keep local search input synced if user typed from top header search bar
    effect(move || {
        let store_q = store::search_query();
        if query_sig.get_untracked() != store_q {
            query_sig.set(store_q);
        }
    });

    scroll().flex_1().min_h(px(0.0)).pb_safe_area().child(
        view()
            .p_4()
            .style_when(is_desktop_layout, |s| s.px_8().py_5())
            .gap_5()
            // Search bar (inline on mobile/tablet or direct test access; complements top bar)
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .px_3p5()
                            .rounded(radius_pill)
                            .bg(pill_glass_surface)
                            .border_1()
                            .border_color(move || theme().input)
                            .child(icon(IconKind::Search, 16.0, move || {
                                theme().muted_foreground
                            }))
                            .child(
                                input()
                                    .test_id("search-input")
                                    .value(move || query_sig.get())
                                    .on_focus(|| set_text_input_focused(true))
                                    .on_blur(|| set_text_input_focused(false))
                                    .on_change(move |txt: &str| {
                                        query_sig.set(txt.to_owned());
                                        store::fetch_search_suggestions(txt);
                                        store::run_search_debounced(txt);
                                    })
                                    .on_submit(move |txt: &str| {
                                        query_sig.set(txt.to_owned());
                                        store::run_search(txt);
                                    })
                                    .placeholder("Search songs, albums, artists, podcasts")
                                    .placeholder_color(|| color(theme().muted_foreground))
                                    .caret_color(|| color(theme().primary))
                                    .return_key_type(ReturnKeyType::Search)
                                    .flex_1()
                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .text_sm()
                                    .text_color(move || theme().foreground)
                                    .border_color(move || theme().input),
                            )
                            .child(when(
                                move || !query_sig.with(String::is_empty),
                                move || {
                                    button()
                                        .accessibility_label("Clear search")
                                        .w(px(36.0))
                                        .h(px(36.0))
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .hover(|s| s.bg(pill_glass_hover))
                                        .focus_visible(|s| {
                                            s.outline(px(2.0)).outline_color(move || theme().ring)
                                        })
                                        .on_press(move || {
                                            query_sig.set(String::new());
                                            store::run_search("");
                                        })
                                        .child(icon(IconKind::Close, 14.0, move || {
                                            theme().muted_foreground
                                        }))
                                },
                            )),
                    )
                    .child(primary_button("Search", move || {
                        store::run_search(&query_sig.get_untracked());
                    })),
            )
            // Scope pills (YT Music, Library, Uploads) + Category filter chips
            .child(
                view()
                    .gap_2()
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(scope_pill(SearchScope::YtMusic, active_scope))
                            .child(scope_pill(SearchScope::Library, active_scope))
                            .child(scope_pill(SearchScope::Uploads, active_scope))
                            .child(view().w(px(1.0)).h(px(18.0)).bg(glass_hairline))
                            .child(filter_tab(SearchFilter::All, "search-filter-all"))
                            .child(filter_tab(SearchFilter::Songs, "search-filter-songs"))
                            .child(filter_tab(SearchFilter::Videos, "search-filter-videos"))
                            .child(filter_tab(SearchFilter::Albums, "search-filter-albums"))
                            .child(filter_tab(
                                SearchFilter::Playlists,
                                "search-filter-playlists",
                            ))
                            .child(filter_tab(SearchFilter::Artists, "search-filter-artists"))
                            .child(filter_tab(SearchFilter::Podcasts, "search-filter-podcasts")),
                    ),
            )
            // Live suggestions chips
            .child(dynamic(move || {
                let suggestions = store::search_suggestions();
                if suggestions.is_empty() {
                    return view().into_element();
                }
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .children(suggestions.into_iter().take(6).map(|sugg| {
                        let sugg_click = sugg.clone();
                        button()
                            .accessibility_label(sugg.clone())
                            .min_h(px(36.0))
                            .px_3()
                            .items_center()
                            .justify_center()
                            .rounded(radius_pill)
                            .bg(pill_glass_surface)
                            .border_1()
                            .border_color(glass_hairline)
                            .hover(|s| s.bg(pill_glass_hover))
                            .focus_visible(|s| {
                                s.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .on_press(move || {
                                query_sig.set(sugg_click.clone());
                                store::run_search(&sugg_click);
                            })
                            .child(text(sugg).text_xs().text_color(move || theme().foreground))
                    }))
                    .into_element()
            }))
            .child(when(store::is_search_loading, || {
                view()
                    .gap_2()
                    .child(skeleton_row())
                    .child(skeleton_row())
                    .child(skeleton_row())
            }))
            .child(when(
                || store::search_error().is_some(),
                move || {
                    let q_retry = query_sig;
                    status_card(
                        "Search unavailable",
                        move || {
                            SharedString::from(format_recoverable_error(
                                "Search could not complete",
                                &store::search_error().unwrap_or_default(),
                                "Check your connection and select Retry.",
                            ))
                        },
                        Some("Retry"),
                        Some(Box::new(move || {
                            store::run_search(&q_retry.get_untracked());
                        })),
                    )
                },
            ))
            // Search Results matching 08_search_03_results_1.png
            .child(dynamic(move || {
                let scope = active_scope.get();
                let filter = filter_memo.get();
                let desktop = desktop_memo.get();
                let raw_q = store::search_query();
                let trimmed = raw_q.trim();

                let tracks: Vec<Track> = match scope {
                    SearchScope::YtMusic => tracks_memo.get(),
                    SearchScope::Library => {
                        let q = trimmed.to_lowercase();
                        store::all_library_songs()
                            .into_iter()
                            .filter(|t| {
                                q.is_empty()
                                    || t.title.to_lowercase().contains(&q)
                                    || t.artist_display().to_lowercase().contains(&q)
                            })
                            .collect()
                    }
                    SearchScope::Uploads => Vec::new(),
                };
                let albums = if scope == SearchScope::YtMusic {
                    albums_memo.get()
                } else {
                    Vec::new()
                };
                let collection_only = filter.is_collection_only();
                let show_albums =
                    !albums.is_empty() && (filter == SearchFilter::All || collection_only);

                if tracks.is_empty() && (!show_albums || albums.is_empty()) {
                    if scope == SearchScope::Uploads {
                        return status_card(
                            "No uploads match",
                            "Sign in with your YouTube Music account in Settings to search uploaded tracks.",
                            Some("Switch to YT Music"),
                            Some(Box::new(move || active_scope.set(SearchScope::YtMusic))),
                        )
                        .into_element();
                    }
                    if trimmed.is_empty() {
                        return status_card(
                            "Start searching",
                            "Search YouTube Music for songs, artists, albums, playlists, or podcasts.",
                            Some("Browse Explore"),
                            Some(Box::new(|| store::set_tab(AppTab::Explore))),
                        )
                        .into_element();
                    }
                    return status_card(
                        SharedString::from(format!("No matches for {}", quote_term(trimmed))),
                        "Check your spelling, try another filter tab, or clear the search.",
                        Some("Clear search"),
                        Some(Box::new(move || {
                            query_sig.set(String::new());
                            store::run_search("");
                        })),
                    )
                    .into_element();
                }

                let mut out = view().gap_6();

                // Split Top Result + Songs quick list on Desktop when filter == All
                if filter == SearchFilter::All && !tracks.is_empty() {
                    let top_track = tracks[0].clone();
                    let quick_songs: Vec<Track> = tracks.iter().take(4).cloned().collect();
                    let play_songs = tracks.clone();

                    if desktop {
                        out = out.child(
                            view()
                                .flex_row()
                                .items_start()
                                .gap_6()
                                .child(
                                    view()
                                        .w(px(360.0))
                                        .gap_3()
                                        .child(
                                            text("Top result")
                                                .text_xl()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(top_result_card(top_track)),
                                )
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .gap_3()
                                        .child(
                                            view()
                                                .flex_row()
                                                .items_center()
                                                .gap_3()
                                                .child(
                                                    text("Songs")
                                                        .text_xl()
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(move || theme().foreground),
                                                )
                                                .child(hero_pill_button(
                                                    "Play",
                                                    Some(IconKind::Play),
                                                    move || {
                                                        store::play_queue(play_songs.clone(), 0);
                                                    },
                                                )),
                                        )
                                        .child(view().gap_1().children(quick_songs.into_iter().map(
                                            |t| search_track_row(t, current_id_memo, true, true),
                                        ))),
                                ),
                        );
                    } else {
                        out = out
                            .child(
                                view()
                                    .gap_2p5()
                                    .child(
                                        text("Top result")
                                            .text_lg()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(top_result_card(top_track)),
                            )
                            .child(
                                view()
                                    .gap_2()
                                    .child(
                                        view()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                text("Songs")
                                                    .text_lg()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(move || theme().foreground),
                                            )
                                            .child(hero_pill_button(
                                                "Play",
                                                Some(IconKind::Play),
                                                move || {
                                                    store::play_queue(play_songs.clone(), 0);
                                                },
                                            )),
                                    )
                                    .children(
                                        quick_songs
                                            .into_iter()
                                            .map(|t| search_track_row(t, current_id_memo, false, false)),
                                    ),
                            );
                    }

                    // Artists circular carousel extracted from matched tracks
                    let mut seen_artists = HashSet::new();
                    let matched_artists: Vec<(String, Option<String>, Option<String>)> = tracks
                        .iter()
                        .flat_map(|t| {
                            let art = t.artwork_url.clone();
                            t.artists.iter().map(move |a| (a.name.clone(), a.id.clone(), art.clone()))
                        })
                        .filter(|(name, _, _)| seen_artists.insert(name.clone()))
                        .take(8)
                        .collect();

                    if !matched_artists.is_empty() {
                        out = out.child(
                            view()
                                .gap_3()
                                .child(
                                    text("Artists")
                                        .text_xl()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(scroll().horizontal().w_full().child(
                                    view().flex_row().items_start().gap_5().py_1().children(
                                        matched_artists.into_iter().map(|(name, cid, art)| {
                                            let mono = name
                                                .chars()
                                                .find(|c| c.is_alphanumeric())
                                                .map(|c| c.to_uppercase().to_string())
                                                .unwrap_or_else(|| "♪".into());
                                            let target_id =
                                                cid.unwrap_or_else(|| format!("UC_{name}"));
                                            button()
                                                .accessibility_label(name.clone())
                                                .w(px(152.0))
                                                .items_center()
                                                .gap_2()
                                                .rounded(radius_card)
                                                .hover(|s| s.opacity(0.9))
                                                .active(|s| s.opacity(0.78))
                                                .focus_visible(|s| {
                                                    s.outline(px(2.0))
                                                        .outline_color(move || theme().ring)
                                                })
                                                .on_press(move || {
                                                    store::open_artist_entity(target_id.clone());
                                                })
                                                .child(artwork_box(art, mono, 140.0, 70.0))
                                                .child(
                                                    text(name)
                                                        .number_of_lines(1)
                                                        .text_sm()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(move || theme().foreground),
                                                )
                                                .child(
                                                    text("Artist")
                                                        .number_of_lines(1)
                                                        .text_xs()
                                                        .text_color(move || {
                                                            theme().muted_foreground
                                                        }),
                                                )
                                        }),
                                    ),
                                )),
                        );
                    }
                }

                // Playlists / Albums / Episodes collection shelf
                if show_albums {
                    let section_label = match filter {
                        SearchFilter::Playlists => "Playlists",
                        SearchFilter::Artists => "Artists",
                        SearchFilter::Podcasts => "Episodes",
                        _ => "Playlists & Albums",
                    };
                    let album_count_label = pluralize(albums.len(), "item", "items");
                    out = out.child(
                        view()
                            .gap_3()
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        text(section_label)
                                            .text_xl()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        text(album_count_label)
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                    ),
                            )
                            .child(
                                scroll()
                                    .when(!collection_only, |s| s.horizontal())
                                    .w_full()
                                    .child(
                                        view()
                                            .when(!collection_only, |v| {
                                                v.flex_row().items_start().gap_4().py_1()
                                            })
                                            .when(collection_only, |v| {
                                                v.flex_row().flex_wrap().items_start().gap_4().py_1()
                                            })
                                            .children(albums.into_iter().take(24).map(|alb| {
                                                let mono = alb
                                                    .title
                                                    .chars()
                                                    .find(|c| c.is_alphanumeric())
                                                    .map(|c| c.to_uppercase().to_string())
                                                    .unwrap_or_else(|| "♫".into());
                                                let id_btn = alb.id.clone();
                                                let title_btn = alb.title.clone();
                                                let sub_btn =
                                                    alb.subtitle.clone().unwrap_or_default();
                                                let art_btn = alb.artwork_url.clone();
                                                media_square_card(
                                                    alb.artwork_url,
                                                    mono,
                                                    alb.title,
                                                    alb.subtitle.unwrap_or_default(),
                                                    164.0,
                                                    false,
                                                    move || {
                                                        if id_btn.starts_with("UC")
                                                            || filter == SearchFilter::Artists
                                                        {
                                                            store::open_artist_entity(
                                                                id_btn.clone(),
                                                            );
                                                        } else if id_btn.starts_with("MPRE")
                                                            || filter == SearchFilter::Albums
                                                        {
                                                            store::open_album_entity(
                                                                id_btn.clone(),
                                                            );
                                                        } else {
                                                            store::open_collection(
                                                                id_btn.clone(),
                                                                title_btn.clone(),
                                                                sub_btn.clone(),
                                                                art_btn.clone(),
                                                            );
                                                        }
                                                    },
                                                    None,
                                                )
                                            })),
                                    ),
                            ),
                    );
                }

                // Full track results for Songs/Videos or remainder when filter != All
                if !collection_only && !tracks.is_empty() && filter != SearchFilter::All {
                    out = out.child(
                        view().gap_1().children(
                            tracks
                                .into_iter()
                                .map(|t| search_track_row(t, current_id_memo, desktop, false)),
                        ),
                    );
                }

                if store::search_continuation().is_some() {
                    out = out.child(
                        view().pt_2().items_center().child(
                            button()
                                .test_id("search-load-more")
                                .accessibility_label("Load more results")
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_5()
                                .items_center()
                                .justify_center()
                                .rounded(radius_pill)
                                .bg(pill_glass_surface)
                                .border_1()
                                .border_color(glass_hairline)
                                .hover(|s| s.bg(pill_glass_hover))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(store::load_more_search)
                                .child(
                                    text("Load more")
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                ),
                        ),
                    );
                }

                out.into_element()
            })),
    )
}
