//! Media list rows: compact mix tiles, full track rows, and loading skeletons.
//! Zero blur inside list rows; semantic theme tokens only.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{DownloadStatus, Track, format_duration_ms};

use crate::store;
use crate::ui::controls::ambient::live_equalizer_indicator;
use crate::ui::controls::context_menu::{TrackMenuContext, track_context_menu};
use crate::ui::controls::primitives::artwork_box;
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_liked, color_player_accent, frosted_bar_surface, glass_hairline,
    is_desktop_layout, motion, pill_glass_hover, pill_glass_surface, radius_card,
};

fn row_transition_ms() -> f32 {
    motion::transition_ms(motion::PRESS_MS, store::reduced_motion())
}

fn row_press_scale() -> f32 {
    if store::reduced_motion() {
        1.0
    } else {
        motion::PRESS_SCALE
    }
}

/// Reactive per-track download state badge & quick action button (>= 48dp touch target).
pub fn download_state_badge(track: Track) -> impl IntoElement {
    let track_id = track.id.clone();
    let track_id_icon = track.id.clone();
    let track_id_label = track.id.clone();
    let track_id_color = track.id.clone();
    let track_for_press = track.clone();

    button()
        .test_id(&format!("track-download-btn-{}", track.id))
        .accessibility_label("Download Track")
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .min_w(px(MOBILE_MIN_TOUCH_TARGET))
        .px_2()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1()
        .rounded_full()
        .transition()
        .duration(row_transition_ms)
        .ease_out()
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.scale(row_press_scale).opacity(0.8))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .on_press(move || match store::download_record(&track_id) {
            None => store::enqueue_download(track_for_press.clone()),
            Some(rec) => match rec.status {
                DownloadStatus::Queued | DownloadStatus::Downloading | DownloadStatus::Tagging => {
                    store::pause_download(&rec.video_id)
                }
                DownloadStatus::Paused => store::resume_download(&rec.video_id),
                DownloadStatus::Failed | DownloadStatus::Cancelled => {
                    store::enqueue_download(track_for_press.clone())
                }
                DownloadStatus::Completed => store::delete_download(&rec.video_id),
            },
        })
        .child(reactive_icon(
            move || match store::download_record(&track_id_icon).map(|r| r.status) {
                None => IconKind::Download,
                Some(
                    DownloadStatus::Queued | DownloadStatus::Downloading | DownloadStatus::Tagging,
                ) => IconKind::Pause,
                Some(DownloadStatus::Paused) => IconKind::Download,
                Some(DownloadStatus::Completed) => IconKind::CheckCircle,
                Some(DownloadStatus::Failed | DownloadStatus::Cancelled) => IconKind::AlertCircle,
            },
            15.0,
            move || match store::download_record(&track_id_color) {
                Some(rec) if rec.status == DownloadStatus::Completed => theme().primary,
                Some(rec)
                    if matches!(
                        rec.status,
                        DownloadStatus::Failed | DownloadStatus::Cancelled
                    ) =>
                {
                    theme().destructive
                }
                _ => theme().muted_foreground,
            },
        ))
        .child(
            text(move || match store::download_record(&track_id_label) {
                Some(rec)
                    if matches!(
                        rec.status,
                        DownloadStatus::Downloading | DownloadStatus::Tagging
                    ) =>
                {
                    let pct = rec
                        .bytes_downloaded
                        .saturating_mul(100)
                        .checked_div(rec.total_bytes.max(1))
                        .unwrap_or(0)
                        .min(100);
                    SharedString::from(format!("{pct}%"))
                }
                Some(rec) if rec.status == DownloadStatus::Queued => SharedString::from("…"),
                _ => SharedString::from(""),
            })
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .tabular_nums()
            .text_color(move || theme().muted_foreground),
        )
}

/// Compact track tile used in the 3-column "Song mixes" / "Quick Picks" grid.
pub fn compact_mix_row(
    track: Track,
    is_current: bool,
    on_play: impl Fn() + 'static,
    _on_play_next: impl Fn() + 'static,
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
    let track_id_color = track.id.clone();
    let track_for_like = track.clone();
    let track_for_menu = track.clone();

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
                .rounded(move || theme().radius)
                .transition()
                .duration(row_transition_ms)
                .ease_out()
                .active(|s| s.opacity(0.78))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(on_play)
                .child(
                    view()
                        .relative()
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
                                    .opacity(0.76)
                                    .items_center()
                                    .justify_center()
                                    .child(live_equalizer_indicator(
                                        || store::player_snapshot().state.is_playing(),
                                        18.0,
                                    )),
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
                .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                .items_center()
                .justify_center()
                .rounded_full()
                .transition()
                .duration(row_transition_ms)
                .ease_out()
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.scale(row_press_scale))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
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
                    move || {
                        if store::is_track_liked(&track_id_color) {
                            color_liked()
                        } else {
                            theme().muted_foreground
                        }
                    },
                )),
        )
        .child(track_context_menu(
            track_for_menu,
            TrackMenuContext::General,
        ))
}

/// Full track list row with optional album column, live equalizer badge, heart, download badge,
/// duration, and action sheet.
pub fn track_row_view(
    track: Track,
    is_current: bool,
    on_play: impl Fn() + 'static,
    _on_play_next: impl Fn() + 'static,
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
    let track_id_color = track.id.clone();
    let track_for_like = track.clone();
    let track_for_badge = track.clone();
    let track_for_menu = track.clone();

    let track_id_offline = track.id.clone();

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
                .rounded(move || theme().radius)
                .transition()
                .duration(row_transition_ms)
                .ease_out()
                .active(|s| s.opacity(0.78))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(on_play)
                .child(
                    view()
                        .relative()
                        .w(px(44.0))
                        .h(px(44.0))
                        .child(artwork_box(art_url, monogram, 44.0, 6.0))
                        .when(is_current, |v| {
                            v.child(
                                view()
                                    .absolute()
                                    .inset_0()
                                    .rounded(px(6.0))
                                    .bg(frosted_bar_surface)
                                    .opacity(0.76)
                                    .items_center()
                                    .justify_center()
                                    .child(live_equalizer_indicator(
                                        || store::player_snapshot().state.is_playing(),
                                        18.0,
                                    )),
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
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .min_w(px(0.0))
                                .child(when(
                                    move || {
                                        !is_desktop_layout()
                                            && store::download_record(&track_id_offline)
                                                .is_some_and(|r| {
                                                    r.status == DownloadStatus::Completed
                                                })
                                    },
                                    || icon(IconKind::CheckCircle, 12.0, move || theme().primary),
                                ))
                                .child(
                                    text(artist)
                                        .flex_1()
                                        .number_of_lines(1)
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
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
        .child(when(is_desktop_layout, move || {
            let like_track = track_for_like.clone();
            let badge_track = track_for_badge.clone();
            let id_icon = track_id.clone();
            let id_color = track_id_color.clone();
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    button()
                        .accessibility_label("Toggle Like Track")
                        .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .transition()
                        .duration(row_transition_ms)
                        .ease_out()
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.scale(row_press_scale))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(move || store::toggle_like_track(like_track.clone()))
                        .child(reactive_icon(
                            move || {
                                if store::is_track_liked(&id_icon) {
                                    IconKind::HeartFilled
                                } else {
                                    IconKind::Heart
                                }
                            },
                            15.0,
                            move || {
                                if store::is_track_liked(&id_color) {
                                    color_liked()
                                } else {
                                    theme().muted_foreground
                                }
                            },
                        )),
                )
                .child(download_state_badge(badge_track))
        }))
        .child(track_context_menu(
            track_for_menu,
            TrackMenuContext::General,
        ))
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
                        .w(percentage(0.68))
                        .h(px(12.0))
                        .rounded_full()
                        .bg(move || theme().muted),
                )
                .child(
                    view()
                        .w(percentage(0.42))
                        .h(px(10.0))
                        .rounded_full()
                        .bg(move || theme().muted),
                ),
        )
}
