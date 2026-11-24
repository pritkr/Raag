//! Search Screen: controlled NSH `input()` + `virtual_list` of songs.
//! Recycled list cells always bind reactively via `at.index()`.

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::format_duration_ms;

use crate::store::{self, SearchFilter};
use crate::ui::controls::{primary_button, skeleton_row, status_card};
use crate::ui::tokens::{MOBILE_MIN_TOUCH_TARGET, radius_card};

fn filter_tab(filter: SearchFilter, test_id: &'static str) -> impl IntoElement {
    let active = move || store::search_filter() == filter;
    button()
        .test_id(test_id)
        .on_press(move || store::set_search_filter(filter))
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(move || {
            if active() {
                theme().primary
            } else {
                theme().secondary
            }
        })
        .border_1()
        .border_color(move || {
            if active() {
                theme().primary
            } else {
                theme().border
            }
        })
        .active(|s| s.opacity(0.8))
        .child(
            text(filter.label())
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if active() {
                        theme().primary_foreground
                    } else {
                        theme().secondary_foreground
                    }
                }),
        )
}

pub fn search_screen() -> impl IntoElement {
    let query_sig = signal(store::search_query());

    view()
        .flex_1()
        .p_4()
        .gap_3()
        .child(
            text("Search")
                .text_2xl()
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
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
                        .px_3()
                        .text_sm()
                        .text_color(move || theme().foreground)
                        .bg(move || theme().card)
                        .rounded(move || theme().radius)
                        .border_1()
                        .border_color(move || theme().input)
                        .focus(|s| s.border_color(move || theme().ring)),
                )
                .child(primary_button("Go", move || {
                    store::run_search(&query_sig.get_untracked());
                })),
        )
        .child(
            view()
                .flex_row()
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
            || {
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
                    let cur_id = store::player_snapshot().current_track.map(|t| t.id);
                    track.with(|t| t.as_ref().map(|x| x.id.as_str()) == cur_id.as_deref())
                };

                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .min_h(px(64.0))
                    .rounded(radius_card)
                    .style_when(is_current, |s| s.bg(move || theme().accent))
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
                            .child(
                                view()
                                    .w(px(44.0))
                                    .h(px(44.0))
                                    .rounded_md()
                                    .bg(move || theme().accent)
                                    .border_1()
                                    .border_color(move || theme().border)
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        text(move || {
                                            track.with(|t| {
                                                SharedString::from(
                                                    t.as_ref()
                                                        .map(|x| x.fallback_monogram())
                                                        .unwrap_or_else(|| "♪".into()),
                                                )
                                            })
                                        })
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || theme().accent_foreground),
                                    ),
                            )
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .gap_0p5()
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
                                        .number_of_lines(1)
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
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
                            .accessibility_label("Play Next")
                            .w(px(MOBILE_MIN_TOUCH_TARGET))
                            .h(px(MOBILE_MIN_TOUCH_TARGET))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .hover(|s| s.bg(move || theme().muted))
                            .active(|s| s.opacity(0.75))
                            .on_press(move || {
                                if let Some(t) = track.get_untracked() {
                                    store::play_next(t);
                                }
                            })
                            .child(
                                text("+")
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().muted_foreground),
                            ),
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
