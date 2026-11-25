//! Search Screen: controlled NSH `input()`, category filter pills
//! (`All`, `Songs`, `Videos`, `Albums`, `Playlists`, `Artists`), and unified full-page scrolling.

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::{Track, format_duration_ms};

use crate::store::{self, SearchFilter};
use crate::ui::controls::{artwork_box, primary_button, skeleton_row, status_card};
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_hero_pill_bg, color_hero_pill_fg, color_liked,
    color_player_accent, glass_hairline, is_desktop_layout, pill_glass_hover, pill_glass_surface,
    radius_card, radius_pill,
};

fn filter_tab(filter: SearchFilter, test_id: &'static str) -> impl IntoElement {
    let active = move || store::search_filter() == filter;
    button()
        .test_id(test_id)
        .accessibility_label(filter.label())
        .on_press(move || store::set_search_filter(filter))
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
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
) -> impl IntoElement {
    let tid = track.id.clone();
    let tid_like = track.id.clone();
    let is_current = move || current_id_memo.get().as_deref() == Some(tid.as_str());
    let is_liked = move || store::is_track_liked(&tid_like);
    let plays_text = if desktop {
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
    let album_title = track.album.as_ref().map(|a| a.title.clone());
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
        .px_3()
        .py_1p5()
        .min_h(px(64.0))
        .rounded(radius_card)
        .style_when(is_current.clone(), |s| s.bg(pill_glass_surface))
        .hover(|s| s.bg(pill_glass_surface))
        .child(
            button()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                .flex_row()
                .items_center()
                .gap_3()
                .active(|s| s.opacity(0.75))
                .on_press(move || store::play_track(t_play.clone()))
                .child(artwork_box(art_url, mono, 44.0, 8.0))
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
        .child(
            button()
                .accessibility_label("Toggle Like Track")
                .w(px(MOBILE_MIN_TOUCH_TARGET))
                .h(px(MOBILE_MIN_TOUCH_TARGET))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.75))
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
                    16.0,
                    move || {
                        if is_liked() {
                            color_liked()
                        } else {
                            theme().muted_foreground
                        }
                    },
                )),
        )
        .child(
            button()
                .accessibility_label("Play Next")
                .w(px(MOBILE_MIN_TOUCH_TARGET))
                .h(px(MOBILE_MIN_TOUCH_TARGET))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.75))
                .on_press(move || store::play_next(t_next.clone()))
                .child(icon(IconKind::Plus, 16.0, move || theme().muted_foreground)),
        )
}

pub fn search_screen() -> impl IntoElement {
    let query_sig = signal(store::search_query());
    let current_id_memo = computed(|| store::player_snapshot().current_track.map(|t| t.id));
    let desktop_memo = computed(is_desktop_layout);
    let tracks_memo = computed(store::search_tracks);
    let albums_memo = computed(store::search_albums);
    let filter_memo = computed(store::search_filter);

    scroll().flex_1().min_h(px(0.0)).child(
        view()
            .p_4()
            .when(is_desktop_layout(), |v| v.px_8().py_5())
            .gap_4()
            .child(
                view()
                    .gap_1()
                    .child(
                        text("Search")
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        text(move || {
                            let track_count = store::search_results_len();
                            let album_count = store::search_albums().len();
                            let filter = store::search_filter();
                            let count = if filter.is_collection_only() {
                                album_count
                            } else if filter == SearchFilter::All {
                                track_count + album_count
                            } else {
                                track_count
                            };
                            let q = store::search_query();
                            if q.is_empty() {
                                SharedString::from(
                                    "Find songs, videos, albums, playlists, and artists",
                                )
                            } else {
                                SharedString::from(format!("{count} results for \"{q}\""))
                            }
                        })
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
                            .border_color(glass_hairline)
                            .child(icon(IconKind::Search, 16.0, move || {
                                theme().muted_foreground
                            }))
                            .child(
                                input()
                                    .test_id("search-input")
                                    .value(move || query_sig.get())
                                    .on_change(move |txt: &str| {
                                        query_sig.set(txt.to_owned());
                                        store::run_search_debounced(txt);
                                    })
                                    .on_submit(move |txt: &str| {
                                        query_sig.set(txt.to_owned());
                                        store::run_search(txt);
                                    })
                                    .placeholder("Search songs, artists, albums...")
                                    .placeholder_color(|| color(theme().muted_foreground))
                                    .caret_color(|| color(theme().primary))
                                    .return_key_type(ReturnKeyType::Search)
                                    .flex_1()
                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .text_sm()
                                    .text_color(move || theme().foreground),
                            )
                            .child(when(
                                move || !query_sig.with(String::is_empty),
                                move || {
                                    button()
                                        .accessibility_label("Clear search")
                                        .w(px(MOBILE_MIN_TOUCH_TARGET))
                                        .h(px(MOBILE_MIN_TOUCH_TARGET))
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .hover(|s| s.bg(pill_glass_hover))
                                        .on_press(move || {
                                            query_sig.set(String::new());
                                            store::run_search("");
                                        })
                                        .child(icon(IconKind::Close, 15.0, move || {
                                            theme().muted_foreground
                                        }))
                                },
                            )),
                    )
                    .child(primary_button("Go", move || {
                        store::run_search(&query_sig.get_untracked());
                    })),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(filter_tab(SearchFilter::All, "search-filter-all"))
                    .child(filter_tab(SearchFilter::Songs, "search-filter-songs"))
                    .child(filter_tab(SearchFilter::Videos, "search-filter-videos"))
                    .child(filter_tab(SearchFilter::Albums, "search-filter-albums"))
                    .child(filter_tab(
                        SearchFilter::Playlists,
                        "search-filter-playlists",
                    ))
                    .child(filter_tab(SearchFilter::Artists, "search-filter-artists")),
            )
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
                        move || SharedString::from(store::search_error().unwrap_or_default()),
                        Some("Retry"),
                        Some(Box::new(move || {
                            store::run_search(&q_retry.get_untracked());
                        })),
                    )
                },
            ))
            .child(dynamic(move || {
                let filter = filter_memo.get();
                let tracks = tracks_memo.get();
                let albums = albums_memo.get();
                let desktop = desktop_memo.get();
                let collection_only = filter.is_collection_only();
                let show_albums =
                    !albums.is_empty() && (filter == SearchFilter::All || collection_only);

                if tracks.is_empty() && (!show_albums || albums.is_empty()) {
                    return status_card(
                        "No matching results",
                        "Try searching for another song, artist, album, or playlist.",
                        None,
                        None,
                    )
                    .into_element();
                }

                let mut out = view().gap_4();

                if show_albums {
                    let section_label = match filter {
                        SearchFilter::Playlists => "Playlists",
                        SearchFilter::Artists => "Artists",
                        _ => "Albums & Collections",
                    };
                    out = out.child(
                        view()
                            .gap_2()
                            .child(
                                text(section_label)
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(
                                scroll()
                                    .when(!collection_only, |s| s.horizontal())
                                    .w_full()
                                    .child(
                                        view()
                                            .when(!collection_only, |v| v.flex_row().gap_3().py_1())
                                            .when(collection_only, |v| {
                                                v.flex_row().flex_wrap().gap_4().py_1()
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
                                                button()
                                                    .w(px(136.0))
                                                    .gap_1p5()
                                                    .active(|s| s.opacity(0.8))
                                                    .on_press(move || {
                                                        store::open_collection(
                                                            id_btn.clone(),
                                                            title_btn.clone(),
                                                            sub_btn.clone(),
                                                            art_btn.clone(),
                                                        );
                                                    })
                                                    .child(artwork_box(
                                                        alb.artwork_url,
                                                        mono,
                                                        136.0,
                                                        10.0,
                                                    ))
                                                    .child(
                                                        text(alb.title)
                                                            .number_of_lines(1)
                                                            .text_xs()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(move || theme().foreground),
                                                    )
                                                    .child(
                                                        text(alb.subtitle.unwrap_or_default())
                                                            .number_of_lines(1)
                                                            .text_xs()
                                                            .text_color(move || {
                                                                theme().muted_foreground
                                                            }),
                                                    )
                                            })),
                                    ),
                            ),
                    );
                }

                if !collection_only && !tracks.is_empty() {
                    out = out.child(
                        view().gap_1().children(
                            tracks
                                .into_iter()
                                .map(|t| search_track_row(t, current_id_memo, desktop)),
                        ),
                    );
                }

                out.into_element()
            })),
    )
}
