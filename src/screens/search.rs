//! Search Screen: controlled NSH `input()` + `virtual_list` of songs.
//! Recycled list cells always bind reactively via `at.index()` and `computed`,
//! with zero `.backdrop_blur(...)` inside virtualized rows.

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::format_duration_ms;

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

pub fn search_screen() -> impl IntoElement {
    let query_sig = signal(store::search_query());
    let current_id_memo = computed(|| store::player_snapshot().current_track.map(|t| t.id));
    let desktop_memo = computed(is_desktop_layout);

    view()
        .flex_1()
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
                        let count = store::search_results_len();
                        let q = store::search_query();
                        if q.is_empty() {
                            SharedString::from("Find songs, albums, and artists across YouTube Music")
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
                        .child(icon(IconKind::Search, 16.0, move || theme().muted_foreground))
                        .child(
                            input()
                                .test_id("search-input")
                                .value(move || query_sig.get())
                                .on_change(move |txt: &str| {
                                    query_sig.set(txt.to_owned());
                                    store::run_search(txt);
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
                .child(filter_tab(SearchFilter::Albums, "search-filter-albums")),
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
        .child(
            virtual_list(store::search_results_len, move |at| {
                let track = computed(move || store::search_result_at(at.index()));
                let is_current = move || {
                    let cur_id = current_id_memo.get();
                    track.with(|t| t.as_ref().map(|x| x.id.as_str()) == cur_id.as_deref())
                };
                let is_liked = move || {
                    track.with(|t| t.as_ref().is_some_and(|x| store::is_track_liked(&x.id)))
                };
                let is_explicit = move || track.with(|t| t.as_ref().is_some_and(|x| x.is_explicit));
                let has_album = move || {
                    desktop_memo.get()
                        && track.with(|t| t.as_ref().and_then(|x| x.album.as_ref()).is_some())
                };

                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1p5()
                    .min_h(px(64.0))
                    .rounded(radius_card)
                    .style_when(is_current, |s| s.bg(pill_glass_surface))
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
                            .on_press(move || {
                                if let Some(t) = track.get_untracked() {
                                    store::play_track(t);
                                }
                            })
                            .child(view().w(px(44.0)).h(px(44.0)).child(dynamic(move || {
                                match track.get() {
                                    Some(t) => artwork_box(
                                        t.artwork_url.clone(),
                                        t.fallback_monogram(),
                                        44.0,
                                        8.0,
                                    )
                                    .into_element(),
                                    None => view().w(px(44.0)).h(px(44.0)).into_element(),
                                }
                            })))
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
                                                text(move || {
                                                    track.with(|t| {
                                                        SharedString::from(
                                                            t.as_ref()
                                                                .map(|x| x.title.clone())
                                                                .unwrap_or_default(),
                                                        )
                                                    })
                                                })
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
                                            .child(when(is_explicit, || {
                                                view()
                                                    .px_1()
                                                    .rounded(px(4.0))
                                                    .bg(move || theme().muted)
                                                    .child(
                                                        text("E")
                                                            .text_xs()
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(move || {
                                                                theme().muted_foreground
                                                            }),
                                                    )
                                            })),
                                    )
                                    .child(
                                        text(move || {
                                            track.with(|t| {
                                                SharedString::from(
                                                    t.as_ref()
                                                        .map(|x| x.artist_display())
                                                        .unwrap_or_default(),
                                                )
                                            })
                                        })
                                        .number_of_lines(1)
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                    ),
                            )
                            .child(when(has_album, move || {
                                view().w(px(180.0)).px_2().child(
                                    text(move || {
                                        track.with(|t| {
                                            SharedString::from(
                                                t.as_ref()
                                                    .and_then(|x| {
                                                        x.album.as_ref().map(|a| a.title.clone())
                                                    })
                                                    .unwrap_or_default(),
                                            )
                                        })
                                    })
                                    .number_of_lines(1)
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                                )
                            }))
                            .child(
                                text(move || {
                                    track.with(|t| {
                                        SharedString::from(
                                            t.as_ref()
                                                .map(|x| format_duration_ms(x.duration_ms))
                                                .unwrap_or_default(),
                                        )
                                    })
                                })
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
                            .on_press(move || {
                                if let Some(t) = track.get_untracked() {
                                    store::toggle_like_track(t);
                                }
                            })
                            .child(reactive_icon(
                                move || {
                                    if is_liked() {
                                        IconKind::HeartFilled
                                    } else {
                                        IconKind::Heart
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
                            .on_press(move || {
                                if let Some(t) = track.get_untracked() {
                                    store::play_next(t);
                                }
                            })
                            .child(icon(IconKind::Plus, 16.0, move || {
                                theme().muted_foreground
                            })),
                    )
            })
            .keys(store::search_result_ids)
            .estimated_item_size(px(64.0))
            .empty(status_card(
                "No matching tracks",
                "Try searching for another song title or artist.",
                None,
                None,
            ))
            .flex_1(),
        )
}
