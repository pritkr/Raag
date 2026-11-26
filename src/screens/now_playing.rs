//! Priority Visual Surface #1: Expanded Now Playing Screen.
//! - Full-screen artwork-driven atmospheric backdrop (`ambient_backdrop_color`)
//! - Desktop (>= 820px): Spacious 2-column split (340px focal artwork + controls on left,
//!   live synced karaoke lyrics beside the artwork on right)
//! - Mobile (< 820px): Single-column focal stack with >= 48dp interactive touch targets
//! - 100% semantic design tokens from `theme.json` and `crate::ui::tokens`.

use nsh_base::{button, theme};
use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;
use resonance_contract::{DownloadStatus, LyricSyncKind, RepeatMode, format_duration_ms};

use crate::store::{self, AppTab};
use crate::ui::controls::{
    open_sleep_timer_sheet, open_speed_sheet, secondary_button, seek_slider,
    sleep_timer_badge_label, sleep_timer_sheet, speed_badge_label, speed_sheet, status_card,
    svg_icon_button, volume_slider,
};
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, ambient_accent_color, ambient_backdrop_color, color_control,
    color_control_fg, color_liked, color_lyric_active, color_lyric_inactive, color_player_accent,
    color_sponsor_badge, cover_gradient_pair, frosted_card_surface, glass_hairline,
    is_desktop_layout, pill_glass_hover, pill_glass_surface, radius_card, radius_panel,
    radius_pill,
};

pub fn now_playing_screen() -> impl IntoElement {
    let has_track = move || store::player_snapshot().current_track.is_some();

    view()
        .flex_1()
        .min_h(px(0.0))
        .bg(move || ambient_backdrop_color(store::active_ambient_palette()))
        .child(
            when(has_track, || {
                when(is_desktop_layout, desktop_now_playing).otherwise(mobile_now_playing)
            })
            .otherwise(|| {
                view()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .p_5()
                    .child(status_card(
                        "Nothing is playing",
                        "Pick a song from Home or Search to begin playback.",
                        Some("Browse Home"),
                        Some(Box::new(|| store::set_tab(AppTab::Home))),
                    ))
            }),
        )
        .child(sleep_timer_sheet())
        .child(speed_sheet())
}

fn header_pill_button(
    label: &'static str,
    kind: IconKind,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(36.0))
        .px_3p5()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(radius_pill)
        .bg(pill_glass_surface)
        .border_1()
        .border_color(glass_hairline)
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.78))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(kind, 14.0, move || theme().foreground))
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
}

fn top_header_bar(desktop: bool) -> impl IntoElement {
    let btn_box = if desktop {
        40.0
    } else {
        MOBILE_MIN_TOUCH_TARGET
    };

    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .px_3()
        .py_2()
        .rounded(radius_card)
        .backdrop_blur(px(14.0))
        .backdrop_material(BackdropMaterial::Thin)
        .bg(frosted_card_surface)
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .flex_row()
                .items_center()
                .gap_2()
                .child(svg_icon_button(
                    IconKind::ChevronDown,
                    18.0,
                    btn_box,
                    "Collapse Now Playing",
                    true,
                    move || theme().foreground,
                    store::go_back,
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            text("Now Playing")
                                .number_of_lines(1)
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    text(move || {
                                        let snap = store::player_snapshot();
                                        if snap.offline_mode {
                                            SharedString::from("Offline")
                                        } else if snap
                                            .current_track
                                            .as_ref()
                                            .is_some_and(|t| t.id.starts_with("local_"))
                                        {
                                            SharedString::from("Local Audio")
                                        } else if snap.active_proxy_url.is_some() {
                                            SharedString::from("Standard Stream")
                                        } else {
                                            SharedString::from("Audio")
                                        }
                                    })
                                    .test_id("now-playing-source-badge")
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text("•")
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    text(move || {
                                        SharedString::from(store::player_snapshot().state.name())
                                    })
                                    .test_id("player-state-label")
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || theme().primary),
                                ),
                        ),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .when(desktop, |v| {
                    v.child(header_pill_button("Open Lyrics", IconKind::Lyrics, || {
                        store::set_tab(AppTab::Lyrics)
                    }))
                    .child(header_pill_button(
                        "Up Next Queue",
                        IconKind::Queue,
                        || store::set_tab(AppTab::Queue),
                    ))
                })
                .child(svg_icon_button(
                    IconKind::Moon,
                    16.0,
                    btn_box,
                    "Sleep Timer",
                    true,
                    move || {
                        if sleep_timer_badge_label().is_some() {
                            color_player_accent()
                        } else {
                            theme().foreground
                        }
                    },
                    open_sleep_timer_sheet,
                ))
                .child(svg_icon_button(
                    IconKind::Gauge,
                    16.0,
                    btn_box,
                    "Playback Speed",
                    true,
                    move || {
                        if speed_badge_label().is_some() {
                            color_player_accent()
                        } else {
                            theme().foreground
                        }
                    },
                    open_speed_sheet,
                ))
                .child(svg_icon_button(
                    IconKind::MoreVertical,
                    16.0,
                    btn_box,
                    "Track Actions",
                    true,
                    move || theme().foreground,
                    || {
                        if let Some(track) = store::player_snapshot().current_track {
                            store::open_track_sheet(track, None);
                        }
                    },
                )),
        )
}

fn focal_artwork(size_px: f32) -> impl IntoElement {
    let has_art_url = move || {
        store::player_snapshot()
            .current_track
            .and_then(|t| t.artwork_url)
            .is_some()
    };

    view()
        .p_2p5()
        .rounded(radius_panel)
        .bg(move || ambient_accent_color(store::active_ambient_palette()))
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .w(px(size_px))
                .h(px(size_px))
                .rounded(radius_card)
                .bg_linear_to_br()
                .gradient_from(move || {
                    let seed = store::player_snapshot()
                        .current_track
                        .map(|t| {
                            format!(
                                "{}-{}",
                                t.fallback_monogram(),
                                t.artwork_url.as_deref().unwrap_or("mono")
                            )
                        })
                        .unwrap_or_else(|| "mono".into());
                    cover_gradient_pair(&seed).0
                })
                .gradient_to(move || {
                    let seed = store::player_snapshot()
                        .current_track
                        .map(|t| {
                            format!(
                                "{}-{}",
                                t.fallback_monogram(),
                                t.artwork_url.as_deref().unwrap_or("mono")
                            )
                        })
                        .unwrap_or_else(|| "mono".into());
                    cover_gradient_pair(&seed).1
                })
                .border_1()
                .border_color(glass_hairline)
                .items_center()
                .justify_center()
                .overflow_hidden()
                .child(
                    when(has_art_url, move || {
                        image(move || {
                            let raw = store::player_snapshot()
                                .current_track
                                .and_then(|t| t.artwork_url);
                            let url = store::resolve_artwork_url(raw).unwrap_or_default();
                            ImageSrc::from(url)
                        })
                        .size(px(size_px))
                        .object_fit(ResizeMode::Cover)
                        .rounded(radius_card)
                    })
                    .otherwise(|| {
                        text(move || {
                            SharedString::from(
                                store::player_snapshot()
                                    .current_track
                                    .map(|t| t.fallback_monogram())
                                    .unwrap_or_else(|| "♪".into()),
                            )
                        })
                        .test_id("now-playing-artwork-monogram")
                        .text_4xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().primary_foreground)
                    }),
                ),
        )
}

fn track_info_and_actions() -> impl IntoElement {
    let is_liked = move || {
        store::player_snapshot()
            .current_track
            .is_some_and(|t| t.is_liked)
    };
    let is_disliked = store::is_current_disliked;

    view()
        .w_full()
        .flex_row()
        .items_center()
        .gap_3()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_1()
                .child(
                    text(move || {
                        SharedString::from(
                            store::player_snapshot()
                                .current_track
                                .map(|t| t.title)
                                .unwrap_or_default(),
                        )
                    })
                    .test_id("now-playing-title")
                    .number_of_lines(2)
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .text_color(move || theme().foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .min_w(px(0.0))
                        .child(when(
                            || {
                                store::player_snapshot()
                                    .current_track
                                    .is_some_and(|t| t.is_explicit)
                            },
                            || {
                                view()
                                    .px_1p5()
                                    .rounded(px(4.0))
                                    .bg(move || theme().muted)
                                    .child(
                                        text("E")
                                            .test_id("now-playing-explicit")
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(move || theme().muted_foreground),
                                    )
                            },
                        ))
                        .child(
                            text(move || {
                                SharedString::from(
                                    store::player_snapshot()
                                        .current_track
                                        .map(|t| t.artist_display())
                                        .unwrap_or_default(),
                                )
                            })
                            .flex_1()
                            .number_of_lines(1)
                            .text_sm()
                            .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(
                    button()
                        .test_id("now-playing-download-btn")
                        .accessibility_label("Download Current Track")
                        .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                        .h(px(MOBILE_MIN_TOUCH_TARGET))
                        .px_2p5()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .rounded_full()
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.75))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(|| {
                            let Some(track) = store::player_snapshot().current_track else {
                                return;
                            };
                            match store::download_record(&track.id) {
                                None => store::enqueue_download(track),
                                Some(rec) => match rec.status {
                                    DownloadStatus::Queued
                                    | DownloadStatus::Downloading
                                    | DownloadStatus::Tagging => {
                                        store::pause_download(&rec.video_id)
                                    }
                                    DownloadStatus::Paused => store::resume_download(&rec.video_id),
                                    DownloadStatus::Failed | DownloadStatus::Cancelled => {
                                        store::enqueue_download(track)
                                    }
                                    DownloadStatus::Completed => {
                                        store::delete_download(&rec.video_id)
                                    }
                                },
                            }
                        })
                        .child(reactive_icon(
                            || {
                                let Some(track) = store::player_snapshot().current_track else {
                                    return IconKind::Download;
                                };
                                match store::download_record(&track.id).map(|r| r.status) {
                                    Some(DownloadStatus::Completed) => IconKind::Check,
                                    _ => IconKind::Download,
                                }
                            },
                            16.0,
                            || {
                                let Some(track) = store::player_snapshot().current_track else {
                                    return theme().foreground;
                                };
                                match store::download_record(&track.id).map(|r| r.status) {
                                    Some(DownloadStatus::Completed) => theme().primary,
                                    Some(DownloadStatus::Failed | DownloadStatus::Cancelled) => {
                                        theme().destructive
                                    }
                                    _ => theme().foreground,
                                }
                            },
                        ))
                        .child(
                            text(|| {
                                let Some(track) = store::player_snapshot().current_track else {
                                    return SharedString::from("");
                                };
                                match store::download_record(&track.id) {
                                    None => SharedString::from(""),
                                    Some(rec) => match rec.status {
                                        DownloadStatus::Queued => SharedString::from("Queued"),
                                        DownloadStatus::Downloading | DownloadStatus::Tagging => {
                                            let pct = rec
                                                .bytes_downloaded
                                                .saturating_mul(100)
                                                .checked_div(rec.total_bytes.max(1))
                                                .unwrap_or(0)
                                                .min(100);
                                            SharedString::from(format!("{pct}%"))
                                        }
                                        DownloadStatus::Paused => SharedString::from("Paused"),
                                        DownloadStatus::Completed => SharedString::from("Offline"),
                                        DownloadStatus::Failed | DownloadStatus::Cancelled => {
                                            SharedString::from("Retry")
                                        }
                                    },
                                }
                            })
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .tabular_nums()
                            .text_color(|| theme().foreground),
                        ),
                )
                .child(
                    button()
                        .test_id("btn-like")
                        .accessibility_label("Toggle Like")
                        .w(px(MOBILE_MIN_TOUCH_TARGET))
                        .h(px(MOBILE_MIN_TOUCH_TARGET))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.75))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(store::toggle_like_current)
                        .child(reactive_icon(
                            move || {
                                if is_liked() {
                                    IconKind::HeartFilled
                                } else {
                                    IconKind::Heart
                                }
                            },
                            19.0,
                            move || {
                                if is_liked() {
                                    color_liked()
                                } else {
                                    theme().foreground
                                }
                            },
                        )),
                )
                .child(
                    button()
                        .accessibility_label("Dislike")
                        .w(px(MOBILE_MIN_TOUCH_TARGET))
                        .h(px(MOBILE_MIN_TOUCH_TARGET))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.75))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(store::toggle_dislike_current)
                        .child(icon(IconKind::ThumbsDown, 18.0, move || {
                            if is_disliked() {
                                theme().foreground
                            } else {
                                theme().muted_foreground
                            }
                        })),
                ),
        )
}

fn step_seek_pill(
    label: &'static str,
    min_size: f32,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(min_size))
        .min_w(px(min_size))
        .px_3()
        .items_center()
        .justify_center()
        .rounded(radius_pill)
        .bg(pill_glass_surface)
        .border_1()
        .border_color(glass_hairline)
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .tabular_nums()
                .text_color(move || theme().foreground),
        )
}

fn seek_section(mobile: bool) -> impl IntoElement {
    let pill_min = if mobile {
        MOBILE_MIN_TOUCH_TARGET
    } else {
        36.0
    };

    view()
        .w_full()
        .gap_2()
        .child(seek_slider(
            || store::player_snapshot().position_ms as f64,
            || store::player_snapshot().duration_ms as f64,
            store::seek_ms,
        ))
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(move || {
                        SharedString::from(format_duration_ms(store::player_snapshot().position_ms))
                    })
                    .test_id("player-position")
                    .text_xs()
                    .tabular_nums()
                    .text_color(move || theme().muted_foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(step_seek_pill("-15s", pill_min, || {
                            let pos = store::player_snapshot().position_ms;
                            store::seek_ms(pos.saturating_sub(15_000));
                        }))
                        .child(step_seek_pill("+15s", pill_min, || {
                            let pos = store::player_snapshot().position_ms;
                            store::seek_ms(pos.saturating_add(15_000));
                        })),
                )
                .child(
                    text(move || {
                        SharedString::from(format_duration_ms(store::player_snapshot().duration_ms))
                    })
                    .text_xs()
                    .tabular_nums()
                    .text_color(move || theme().muted_foreground),
                ),
        )
}

fn transport_bar() -> impl IntoElement {
    let is_playing = move || store::player_snapshot().state.is_playing();
    let is_shuffle = move || store::player_snapshot().queue.shuffle;

    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            button()
                .accessibility_label("Toggle Shuffle")
                .w(px(MOBILE_MIN_TOUCH_TARGET))
                .h(px(MOBILE_MIN_TOUCH_TARGET))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.72))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(store::toggle_shuffle)
                .child(icon(IconKind::Shuffle, 19.0, move || {
                    if is_shuffle() {
                        color_player_accent()
                    } else {
                        theme().muted_foreground
                    }
                })),
        )
        .child(svg_icon_button(
            IconKind::SkipBack,
            22.0,
            MOBILE_MIN_TOUCH_TARGET,
            "Previous Track",
            true,
            move || theme().foreground,
            store::previous_track,
        ))
        .child(
            button()
                .accessibility_label("Play or Pause")
                .w(px(62.0))
                .h(px(62.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(color_control)
                .hover(|s| s.opacity(0.92))
                .active(|s| s.opacity(0.78))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(store::toggle_play_pause)
                .child(reactive_icon(
                    move || {
                        if is_playing() {
                            IconKind::Pause
                        } else {
                            IconKind::Play
                        }
                    },
                    24.0,
                    color_control_fg,
                )),
        )
        .child(svg_icon_button(
            IconKind::SkipForward,
            22.0,
            MOBILE_MIN_TOUCH_TARGET,
            "Next Track",
            true,
            move || theme().foreground,
            store::next_track,
        ))
        .child(
            button()
                .accessibility_label("Cycle Repeat Mode")
                .w(px(MOBILE_MIN_TOUCH_TARGET))
                .h(px(MOBILE_MIN_TOUCH_TARGET))
                .items_center()
                .justify_center()
                .gap_0p5()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.72))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(store::cycle_repeat)
                .child(reactive_icon(
                    move || match store::player_snapshot().queue.repeat {
                        RepeatMode::One => IconKind::RepeatOne,
                        _ => IconKind::Repeat,
                    },
                    19.0,
                    move || {
                        if store::player_snapshot().queue.repeat == RepeatMode::None {
                            theme().muted_foreground
                        } else {
                            color_player_accent()
                        }
                    },
                ))
                .child(when(
                    || store::player_snapshot().queue.repeat == RepeatMode::One,
                    || {
                        view()
                            .w(px(4.0))
                            .h(px(4.0))
                            .rounded_full()
                            .bg(color_player_accent)
                    },
                )),
        )
}

fn volume_step_button(
    glyph: &'static str,
    a11y_label: &'static str,
    box_px: f32,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .accessibility_label(a11y_label)
        .w(px(box_px))
        .h(px(box_px))
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(pill_glass_surface)
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.72))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(glyph)
                .text_base()
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
        )
}

fn volume_strip(desktop: bool) -> impl IntoElement {
    let btn_box = if desktop {
        36.0
    } else {
        MOBILE_MIN_TOUCH_TARGET
    };
    let is_muted = move || store::player_snapshot().volume <= 0.01;

    view()
        .w_full()
        .gap_1p5()
        .px_3()
        .py_2()
        .rounded(radius_card)
        .backdrop_blur(px(14.0))
        .backdrop_material(BackdropMaterial::Thin)
        .bg(frosted_card_surface)
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    button()
                        .accessibility_label("Toggle Mute")
                        .w(px(btn_box))
                        .h(px(btn_box))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(pill_glass_surface)
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.72))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(store::toggle_mute)
                        .child(reactive_icon(
                            move || {
                                if is_muted() {
                                    IconKind::VolumeMute
                                } else {
                                    IconKind::VolumeHigh
                                }
                            },
                            17.0,
                            move || theme().foreground,
                        )),
                )
                .when(desktop, |v| {
                    v.child(volume_slider(
                        || f64::from(store::player_snapshot().volume),
                        store::set_volume,
                    ))
                })
                .child(
                    text(move || {
                        let pct = (store::player_snapshot().volume * 100.0).round() as u32;
                        SharedString::from(format!("Volume {pct}%"))
                    })
                    .test_id("volume-readout")
                    .text_xs()
                    .tabular_nums()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(volume_step_button(
                            "−",
                            "Decrease Volume",
                            btn_box,
                            || store::adjust_volume(-0.1),
                        ))
                        .child(volume_step_button("+", "Increase Volume", btn_box, || {
                            store::adjust_volume(0.1)
                        })),
                ),
        )
        .when(!desktop, |v| {
            v.child(view().w_full().items_center().child(volume_slider(
                || f64::from(store::player_snapshot().volume),
                store::set_volume,
            )))
        })
}

fn sponsorblock_badge() -> impl IntoElement {
    when(
        || store::player_snapshot().sponsorblock_enabled,
        || {
            view()
                .px_3()
                .py_1p5()
                .rounded(radius_pill)
                .bg(pill_glass_surface)
                .border_1()
                .border_color(color_sponsor_badge)
                .child(
                    text("SponsorBlock active (auto-skip non-music segments)")
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color_sponsor_badge),
                )
        },
    )
}

fn desktop_lyrics_panel() -> impl IntoElement {
    let has_lyrics = move || store::current_lyrics().is_some_and(|d| !d.lines.is_empty());

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .h_full()
        .gap_4()
        .px_4()
        .py_3()
        .rounded(radius_panel)
        .bg(pill_glass_surface)
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    text(move || {
                        let label = match store::current_lyrics() {
                            Some(doc)
                                if !doc.lines.is_empty()
                                    && doc.sync_kind != LyricSyncKind::Plain =>
                            {
                                format!("Synced • {}", doc.provider)
                            }
                            Some(doc) if !doc.lines.is_empty() => {
                                format!("These lyrics aren't synced • {}", doc.provider)
                            }
                            _ => "These lyrics aren't synced".into(),
                        };
                        SharedString::from(label)
                    })
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().muted_foreground),
                )
                .child(sponsorblock_badge()),
        )
        .child(
            when(has_lyrics, || {
                virtual_list(
                    || store::current_lyrics().map(|d| d.lines.len()).unwrap_or(0),
                    move |at| {
                        let line = computed(move || {
                            store::current_lyrics().and_then(|d| d.lines.get(at.index()).cloned())
                        });
                        let is_active = move || {
                            let pos = store::player_snapshot().position_ms;
                            store::current_lyrics().and_then(|d| d.active_line_index(pos))
                                == Some(at.index())
                        };

                        // Zero backdrop blur inside virtual_list rows.
                        button()
                            .w_full()
                            .min_h(px(44.0))
                            .px_3()
                            .py_2p5()
                            .rounded(radius_card)
                            .style_when(is_active, |s| s.bg(pill_glass_surface))
                            .style_when(move || !is_active(), |s| s.opacity(0.62))
                            .hover(|s| s.bg(pill_glass_hover))
                            .active(|s| s.opacity(0.85))
                            .on_press(move || {
                                if let Some(l) = line.get_untracked()
                                    && l.end_ms > 0
                                {
                                    store::seek_ms(l.start_ms);
                                }
                            })
                            .child(
                                text(move || {
                                    line.with(|l| {
                                        SharedString::from(
                                            l.as_ref().map(|x| x.text.clone()).unwrap_or_default(),
                                        )
                                    })
                                })
                                .text_xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || {
                                    if is_active() {
                                        color_lyric_active()
                                    } else {
                                        color_lyric_inactive()
                                    }
                                }),
                            )
                    },
                )
                .keys(|| {
                    store::current_lyrics()
                        .map(|d| {
                            (0..d.lines.len())
                                .map(|i| SharedString::from(format!("np-lyric-{i}")))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                })
                .estimated_item_size(px(56.0))
                .flex_1()
            })
            .otherwise(|| {
                view().flex_1().items_center().justify_center().p_6().child(
                    text("Lyrics will appear here when available for the active track.")
                        .text_lg()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
            }),
        )
}

fn desktop_now_playing() -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .p_5()
        .gap_5()
        .items_center()
        .child(
            view()
                .w_full()
                .max_w(px(1080.0))
                .child(top_header_bar(true)),
        )
        .child(
            view()
                .w_full()
                .max_w(px(1080.0))
                .flex_1()
                .min_h(px(0.0))
                .flex_row()
                .items_start()
                .gap_10()
                .child(
                    scroll().w(px(380.0)).h_full().child(
                        view()
                            .w_full()
                            .gap_4()
                            .items_center()
                            .pb_4()
                            .child(focal_artwork(340.0))
                            .child(track_info_and_actions())
                            .child(seek_section(false))
                            .child(transport_bar())
                            .child(volume_strip(true)),
                    ),
                )
                .child(desktop_lyrics_panel()),
        )
}

fn mobile_now_playing() -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .w_full()
            .p_4()
            .gap_4()
            .items_center()
            .pb_safe_area_plus(px(16.0))
            .child(top_header_bar(false))
            .child(focal_artwork(268.0))
            .child(track_info_and_actions())
            .child(seek_section(true))
            .child(transport_bar())
            .child(volume_strip(false))
            .child(sponsorblock_badge())
            .child(
                view()
                    .w_full()
                    .flex_row()
                    .justify_center()
                    .gap_3()
                    .child(secondary_button("Open Lyrics", || {
                        store::set_tab(AppTab::Lyrics)
                    }))
                    .child(secondary_button("Up Next Queue", || {
                        store::set_tab(AppTab::Queue)
                    })),
            ),
    )
}
