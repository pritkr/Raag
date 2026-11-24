//! Priority Visual Surface #4: Home Feed.
//! Displays Quick Picks / Trending tracks and curated playlists with crisp
//! single-line ellipsis truncation and instant tap-to-play.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::HomeItem;

use crate::store::{self, AppTab};
use crate::ui::controls::{
    artwork_box, primary_button, secondary_button, skeleton_row, status_card, track_row_view,
};
use crate::ui::tokens::{MOBILE_MIN_TOUCH_TARGET, radius_card};

pub fn home_screen() -> impl IntoElement {
    let has_collection = move || store::selected_collection().is_some();

    view()
        .flex_1()
        .child(
            when(has_collection, collection_detail_view).otherwise(home_feed_view),
        )
}

fn collection_detail_view() -> impl IntoElement {
    let Some(playlist) = store::selected_collection() else {
        return view().flex_1();
    };
    let current_id = store::player_snapshot().current_track.map(|t| t.id);
    let tracks_for_play_all = playlist.tracks.clone();

    view().flex_1().child(
        scroll().flex_1().child(
            view()
                .p_4()
                .gap_4()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(secondary_button("← Back", store::close_collection))
                        .child(primary_button("Play All", move || {
                            if let Some(first) = tracks_for_play_all.first().cloned() {
                                store::play_track(first);
                                for extra in tracks_for_play_all.iter().skip(1).cloned() {
                                    store::play_last(extra);
                                }
                            }
                        })),
                )
                .child(
                    view()
                        .gap_1()
                        .child(
                            text(playlist.title)
                                .number_of_lines(2)
                                .text_xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                        )
                        .when_some(playlist.subtitle, |v, sub| {
                            v.child(
                                text(sub)
                                    .number_of_lines(1)
                                    .text_sm()
                                    .text_color(move || theme().muted_foreground),
                            )
                        }),
                )
                .children(playlist.tracks.into_iter().map(move |track| {
                    let is_cur = current_id.as_deref() == Some(track.id.as_str());
                    let t_play = track.clone();
                    let t_next = track.clone();
                    track_row_view(
                        track,
                        is_cur,
                        move || store::play_track(t_play.clone()),
                        move || store::play_next(t_next.clone()),
                    )
                })),
        ),
    )
}

fn home_feed_view() -> impl IntoElement {
    let sections = store::home_sections();
    let current_id = store::player_snapshot().current_track.map(|t| t.id);

    scroll().flex_1().child(
        view()
            .p_4()
            .gap_5()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap_0p5()
                            .child(
                                text("Resonance")
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(
                                text(move || {
                                    let (signed_in, acct) = store::auth_info();
                                    if signed_in {
                                        SharedString::from(
                                            acct.unwrap_or_else(|| "Signed in".into()),
                                        )
                                    } else {
                                        SharedString::from("Anonymous listening • Fast local cache")
                                    }
                                })
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(primary_button("Search", || store::set_tab(AppTab::Search))),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(secondary_button("Focus", || {
                        store::run_search("Synthwave");
                        store::set_tab(AppTab::Search);
                    }))
                    .child(secondary_button("Acoustic", || {
                        store::run_search("Acoustic");
                        store::set_tab(AppTab::Search);
                    }))
                    .child(secondary_button("Night Drive", || {
                        store::run_search("Midnight");
                        store::set_tab(AppTab::Search);
                    })),
            )
            .child(when(store::is_home_loading, || {
                view()
                    .gap_2()
                    .child(skeleton_row())
                    .child(skeleton_row())
                    .child(skeleton_row())
                    .child(skeleton_row())
            }))
            .child(when(
                || store::home_error().is_some(),
                || {
                    status_card(
                        "Unable to reach YouTube Music",
                        move || SharedString::from(store::home_error().unwrap_or_default()),
                        Some("Retry"),
                        Some(Box::new(store::refresh_home)),
                    )
                },
            ))
            .when(
                sections.is_empty() && !store::is_home_loading() && store::home_error().is_none(),
                |v| {
                    v.child(status_card(
                        "Home feed is empty",
                        "Search YouTube Music or reconnect to load recommendations.",
                        Some("Open Search"),
                        Some(Box::new(|| store::set_tab(AppTab::Search))),
                    ))
                },
            )
            .children(sections.into_iter().map(move |section| {
                let active_id = current_id.clone();
                view()
                    .gap_2()
                    .child(
                        text(section.title)
                            .number_of_lines(1)
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .children(section.items.into_iter().map(move |item| match item {
                        HomeItem::Track(track) => {
                            let is_cur = active_id.as_deref() == Some(track.id.as_str());
                            let t_play = track.clone();
                            let t_next = track.clone();
                            view().child(track_row_view(
                                track,
                                is_cur,
                                move || store::play_track(t_play.clone()),
                                move || store::play_next(t_next.clone()),
                            ))
                        }
                        HomeItem::Collection {
                            browse_id,
                            title,
                            subtitle,
                            artwork_url,
                        } => {
                            let mono = title
                                .chars()
                                .find(|c| c.is_alphanumeric())
                                .map(|c| c.to_uppercase().to_string())
                                .unwrap_or_else(|| "♫".into());
                            let c_id = browse_id.clone();
                            let c_title = title.clone();
                            let c_sub = subtitle.clone();
                            let c_art = artwork_url.clone();
                            view().child(
                                button()
                                    .w_full()
                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .p_3()
                                    .rounded(radius_card)
                                    .bg(move || theme().card)
                                    .border_1()
                                    .border_color(move || theme().border)
                                    .hover(|s| s.bg(move || theme().muted))
                                    .active(|s| s.opacity(0.8))
                                    .on_press(move || {
                                        store::open_collection(
                                            c_id.clone(),
                                            c_title.clone(),
                                            c_sub.clone(),
                                            c_art.clone(),
                                        );
                                    })
                                    .child(artwork_box(artwork_url, mono, 44.0, 8.0))
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
                                    ),
                            )
                        }
                    }))
            })),
    )
}
