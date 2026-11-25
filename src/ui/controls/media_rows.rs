//! Media list rows: compact mix tiles, full track rows, and loading skeletons.
//! Zero blur inside list rows; semantic theme tokens only.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{Track, format_duration_ms};

use crate::store;
use crate::ui::controls::primitives::artwork_box;
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_liked, color_player_accent, frosted_bar_surface, glass_hairline,
    is_desktop_layout, pill_glass_hover, pill_glass_surface, radius_card,
};

/// Compact track tile used in the 3-column "Song mixes" / "Quick Picks" grid.
pub fn compact_mix_row(
    track: Track,
    is_current: bool,
    on_play: impl Fn() + 'static,
    on_play_next: impl Fn() + 'static,
) -> impl IntoElement {
    let title = SharedString::from(track.title.clone());
    let subtitle = SharedString::from(format!(
        "{} • {}",
        track.artist_display(),
        format_duration_ms(track.duration_ms)
    ));
    let monogram = track.fallback_monogram();
    let art_url = track.artwork_url.clone();
    let track_id = track.id.clone();
    let track_for_like = track.clone();

    view()
        .flex_row()
        .items_center()
        .gap_2p5()
        .px_2()
        .py_1p5()
        .min_h(px(58.0))
        .rounded(radius_card)
        .when(is_current, |v| v.bg(pill_glass_surface))
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
                .on_press(on_play)
                .child(
                    view()
                        .w(px(46.0))
                        .h(px(46.0))
                        .child(artwork_box(art_url, monogram, 46.0, 6.0))
                        .when(is_current, |v| {
                            v.child(
                                view()
                                    .absolute()
                                    .inset_0()
                                    .rounded(px(6.0))
                                    .bg(frosted_bar_surface)
                                    .opacity(0.55)
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconKind::Play, 16.0, move || theme().foreground)),
                            )
                        }),
                )
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
                                        .font_weight(if is_current {
                                            FontWeight::BOLD
                                        } else {
                                            FontWeight::MEDIUM
                                        })
                                        .text_color(move || {
                                            if is_current {
                                                color_player_accent()
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                )
                                .when(track.is_explicit, |v| {
                                    v.child(
                                        view()
                                            .px_1()
                                            .rounded(px(3.0))
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
                            text(subtitle)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
        .child(
            button()
                .accessibility_label("Toggle Like Track")
                .w(px(34.0))
                .h(px(34.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .on_press(move || store::toggle_like_track(track_for_like.clone()))
                .child(reactive_icon(
                    move || {
                        if store::is_track_liked(&track_id) {
                            IconKind::HeartFilled
                        } else {
                            IconKind::Heart
                        }
                    },
                    14.0,
                    move || theme().muted_foreground,
                )),
        )
        .child(
            button()
                .accessibility_label("Play Next")
                .w(px(34.0))
                .h(px(34.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .on_press(on_play_next)
                .child(icon(IconKind::MoreVertical, 15.0, move || {
                    theme().muted_foreground
                })),
        )
}

/// Full track list row with optional album column, heart, duration, and Play Next action.
pub fn track_row_view(
    track: Track,
    is_current: bool,
    on_play: impl Fn() + 'static,
    on_play_next: impl Fn() + 'static,
) -> impl IntoElement {
    let title = SharedString::from(track.title.clone());
    let artist = SharedString::from(track.artist_display());
    let album_title = track
        .album
        .as_ref()
        .map(|a| SharedString::from(a.title.clone()));
    let duration = SharedString::from(format_duration_ms(track.duration_ms));
    let monogram = track.fallback_monogram();
    let art_url = track.artwork_url.clone();
    let track_id = track.id.clone();
    let track_for_like = track.clone();

    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py_1p5()
        .min_h(px(58.0))
        .rounded(radius_card)
        .when(is_current, |v| v.bg(pill_glass_surface))
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
                .on_press(on_play)
                .child(artwork_box(art_url, monogram, 44.0, 6.0))
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
                                        .font_weight(if is_current {
                                            FontWeight::BOLD
                                        } else {
                                            FontWeight::MEDIUM
                                        })
                                        .text_color(move || {
                                            if is_current {
                                                color_player_accent()
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                )
                                .when(track.is_explicit, |v| {
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
                .when_some(album_title, |b, alb| {
                    b.child(when(is_desktop_layout, move || {
                        let alb_text = alb.clone();
                        view().w(px(180.0)).px_2().child(
                            text(alb_text)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                    }))
                })
                .child(
                    text(duration)
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            button()
                .accessibility_label("Toggle Like Track")
                .w(px(36.0))
                .h(px(36.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .on_press(move || store::toggle_like_track(track_for_like.clone()))
                .child(reactive_icon(
                    move || {
                        if store::is_track_liked(&track_id) {
                            IconKind::HeartFilled
                        } else {
                            IconKind::Heart
                        }
                    },
                    15.0,
                    move || {
                        if is_current {
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
                .active(|s| s.opacity(0.7))
                .on_press(on_play_next)
                .child(icon(IconKind::Plus, 16.0, move || {
                    theme().muted_foreground
                })),
        )
}

/// Subtle loading skeleton placeholder row (zero layout shift while Home or Search loads).
pub fn skeleton_row() -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .min_h(px(60.0))
        .rounded(radius_card)
        .bg(move || theme().card)
        .border_1()
        .border_color(glass_hairline)
        .opacity(0.65)
        .child(
            view()
                .w(px(44.0))
                .h(px(44.0))
                .rounded_md()
                .bg(move || theme().muted),
        )
        .child(
            view()
                .flex_1()
                .gap_2()
                .child(
                    view()
                        .w(percentage(68.0))
                        .h(px(12.0))
                        .rounded_full()
                        .bg(move || theme().muted),
                )
                .child(
                    view()
                        .w(percentage(42.0))
                        .h(px(10.0))
                        .rounded_full()
                        .bg(move || theme().muted),
                ),
        )
}
