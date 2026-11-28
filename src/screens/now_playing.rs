//! Priority Visual Surface #1: Expanded Now Playing Screen matching `ytm-rs` (`07_now_playing_01_expanded.png` & `_04_lyrics.png`):
//! - Full-screen artwork-driven animated ambient backdrop (`ambient_shader_backdrop`)
//! - Desktop (>= 820px): Ultra-clean top bar (`v` collapse top-left, subtle actions top-right),
//!   spacious 2-column split (340px focal artwork + track title/artist + compact actions on left,
//!   large expressive synced karaoke lyrics directly over the ambient canvas on right)
//! - Mobile (< 820px): Single-column focal stack with >= 48dp interactive touch targets
//! - 100% semantic design tokens from `theme.json` and `crate::ui::tokens`.

use nsh_base::{button, tab, tabs, theme};
use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;
use resonance_contract::{DownloadStatus, LyricSyncKind, RepeatMode, format_duration_ms};

use crate::store::{self, AppTab, NowPlayingTab};
use crate::ui::controls::{
    active_state_dot, ambient_shader_backdrop, animated_like_heart, animated_play_pause_icon,
    artwork_box, confirm_destructive, open_sleep_timer_sheet, open_speed_sheet, secondary_button,
    seek_slider, setting_switch, sleep_timer_badge_label, speed_badge_label, status_card,
    svg_icon_button, volume_slider,
};
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, ambient_accent_color, color_control, color_control_fg, color_liked,
    color_lyric_active, color_lyric_inactive, color_on_cover, color_player_accent,
    color_sponsor_badge, color_status_ok, cover_gradient_pair, frosted_card_surface,
    glass_hairline, glass_specular_hairline, is_desktop_layout, is_narrow_viewport, motion,
    pill_glass_hover, pill_glass_surface, radius_card, radius_panel, radius_pill,
    radius_tab_inner, sidebar_active_surface,
};

fn np_transition_ms() -> f32 {
    motion::transition_ms(motion::PRESS_MS, store::reduced_motion())
}

fn np_press_scale() -> f32 {
    if store::reduced_motion() {
        1.0
    } else {
        motion::PRESS_SCALE
    }
}

pub fn now_playing_screen() -> impl IntoElement {
    let has_track = move || store::player_snapshot().current_track.is_some();

    view()
        .relative()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .child(ambient_shader_backdrop())
        .child(
            view()
                .relative()
                .flex_1()
                .min_h(px(0.0))
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
                ),
        )
}

fn header_pill_button(
    label: &'static str,
    kind: IconKind,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(34.0))
        .px_3()
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
        36.0
    } else {
        MOBILE_MIN_TOUCH_TARGET
    };

    view()
        .w_full()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .when(!desktop, |v| {
            v.px_3()
                .py_2()
                .rounded(radius_card)
                .bg(frosted_card_surface)
                .border_1()
                .border_color(glass_hairline)
        })
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2p5()
                .child(svg_icon_button(
                    IconKind::ChevronDown,
                    18.0,
                    btn_box,
                    "Collapse Now Playing",
                    false,
                    move || theme().foreground,
                    store::go_back,
                ))
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
                                } else if snap.current_track.as_ref().is_some_and(|t| {
                                    t.id.starts_with("local:") || t.id.starts_with("local_")
                                }) {
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
                .child(
                    button()
                        .selected(|| sleep_timer_badge_label().is_some())
                        .accessibility_label("Sleep Timer")
                        .w(px(btn_box))
                        .h(px(btn_box))
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_0p5()
                        .rounded_full()
                        .bg(pill_glass_surface)
                        .transition()
                        .duration(np_transition_ms)
                        .ease_out()
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.scale(np_press_scale))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(open_sleep_timer_sheet)
                        .child(icon(IconKind::Moon, 15.0, move || {
                            if sleep_timer_badge_label().is_some() {
                                color_player_accent()
                            } else {
                                theme().foreground
                            }
                        }))
                        .child(active_state_dot(|| sleep_timer_badge_label().is_some())),
                )
                .child(
                    button()
                        .selected(|| speed_badge_label().is_some())
                        .accessibility_label("Playback Speed")
                        .w(px(btn_box))
                        .h(px(btn_box))
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_0p5()
                        .rounded_full()
                        .bg(pill_glass_surface)
                        .transition()
                        .duration(np_transition_ms)
                        .ease_out()
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.scale(np_press_scale))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(open_speed_sheet)
                        .child(icon(IconKind::Gauge, 15.0, move || {
                            if speed_badge_label().is_some() {
                                color_player_accent()
                            } else {
                                theme().foreground
                            }
                        }))
                        .child(active_state_dot(|| speed_badge_label().is_some())),
                )
                .child(svg_icon_button(
                    IconKind::MoreHorizontal,
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
        .relative()
        .p_1()
        .items_center()
        .justify_center()
        .child(
            view()
                .absolute()
                .inset_1()
                .rounded(radius_panel)
                .bg(move || ambient_accent_color(store::active_ambient_palette()))
                .blur(px(28.0))
                .opacity(0.48)
                .pointer_events(PointerEvents::None)
                .accessibility_elements_hidden(true),
        )
        .child(
            view()
                .relative()
                .w(px(size_px))
                .h(px(size_px))
                .rounded(radius_card)
                .shadow_xl()
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
                        .text_color(color_on_cover)
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
                        .accessibility_label(|| {
                            let Some(track) = store::player_snapshot().current_track else {
                                return Some(SharedString::from("Download Current Track"));
                            };
                            Some(SharedString::from(
                                match store::download_record(&track.id).map(|r| r.status) {
                                    None => "Download Current Track",
                                    Some(
                                        DownloadStatus::Queued
                                        | DownloadStatus::Downloading
                                        | DownloadStatus::Tagging,
                                    ) => "Pause Download",
                                    Some(DownloadStatus::Paused) => "Resume Download",
                                    Some(DownloadStatus::Completed) => "Offline (Remove Download)",
                                    Some(DownloadStatus::Failed | DownloadStatus::Cancelled) => {
                                        "Retry Download"
                                    }
                                },
                            ))
                        })
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
                                        let id = rec.video_id.clone();
                                        let title = track.title.clone();
                                        confirm_destructive(
                                            "Remove offline download?",
                                            format!(
                                                "Remove “{title}” from offline downloads on this device?"
                                            ),
                                            "Remove Download",
                                            move || store::delete_download(&id),
                                        );
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
                                    Some(DownloadStatus::Completed) => color_status_ok(),
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
                        .transition()
                        .duration(np_transition_ms)
                        .ease_out()
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.scale(np_press_scale).opacity(0.84))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(store::toggle_like_current)
                        .child(animated_like_heart(
                            is_liked,
                            18.0,
                            color_liked,
                            move || theme().foreground,
                        )),
                )
                .child(
                    button()
                        .selected(is_disliked)
                        .accessibility_label("Dislike")
                        .w(px(MOBILE_MIN_TOUCH_TARGET))
                        .h(px(MOBILE_MIN_TOUCH_TARGET))
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap_0p5()
                        .rounded_full()
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(move || {
                            if is_disliked() {
                                color_player_accent()
                            } else {
                                glass_hairline()
                            }
                        })
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.75))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(store::toggle_dislike_current)
                        .child(icon(IconKind::ThumbsDown, 17.0, move || {
                            if is_disliked() {
                                theme().foreground
                            } else {
                                theme().muted_foreground
                            }
                        }))
                        .child(active_state_dot(is_disliked)),
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
        32.0
    };

    view()
        .w_full()
        .gap_1p5()
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
                        .gap_1p5()
                        .child(step_seek_pill("\u{2212}15s", pill_min, || {
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
                .selected(is_shuffle)
                .accessibility_label("Toggle Shuffle")
                .w(px(MOBILE_MIN_TOUCH_TARGET))
                .h(px(MOBILE_MIN_TOUCH_TARGET))
                .flex_col()
                .items_center()
                .justify_center()
                .gap_0p5()
                .rounded_full()
                .style_when(is_shuffle, |s| s.bg(pill_glass_surface))
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.72))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(store::toggle_shuffle)
                .child(icon(IconKind::Shuffle, 18.0, move || {
                    if is_shuffle() {
                        color_player_accent()
                    } else {
                        theme().muted_foreground
                    }
                }))
                .child(active_state_dot(is_shuffle)),
        )
        .child(svg_icon_button(
            IconKind::SkipBack,
            21.0,
            MOBILE_MIN_TOUCH_TARGET,
            "Previous Track",
            false,
            move || theme().foreground,
            store::previous_track,
        ))
        .child(
            button()
                .accessibility_label("Play or Pause")
                .w(px(56.0))
                .h(px(56.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(color_control)
                .shadow_lg()
                .transition()
                .duration(np_transition_ms)
                .ease_out()
                .hover(|s| s.opacity(0.92))
                .active(|s| s.scale(np_press_scale).opacity(0.86))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(store::toggle_play_pause)
                .child(animated_play_pause_icon(
                    is_playing,
                    22.0,
                    color_control_fg,
                )),
        )
        .child(svg_icon_button(
            IconKind::SkipForward,
            21.0,
            MOBILE_MIN_TOUCH_TARGET,
            "Next Track",
            false,
            move || theme().foreground,
            store::next_track,
        ))
        .child(
            button()
                .selected(|| store::player_snapshot().queue.repeat != RepeatMode::None)
                .accessibility_label("Cycle Repeat Mode")
                .w(px(MOBILE_MIN_TOUCH_TARGET))
                .h(px(MOBILE_MIN_TOUCH_TARGET))
                .flex_col()
                .items_center()
                .justify_center()
                .gap_0p5()
                .rounded_full()
                .style_when(
                    || store::player_snapshot().queue.repeat != RepeatMode::None,
                    |s| s.bg(pill_glass_surface),
                )
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.72))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(store::cycle_repeat)
                .child(reactive_icon(
                    move || match store::player_snapshot().queue.repeat {
                        RepeatMode::One => IconKind::RepeatOne,
                        _ => IconKind::Repeat,
                    },
                    18.0,
                    move || {
                        if store::player_snapshot().queue.repeat == RepeatMode::None {
                            theme().muted_foreground
                        } else {
                            color_player_accent()
                        }
                    },
                ))
                .child(active_state_dot(|| {
                    store::player_snapshot().queue.repeat != RepeatMode::None
                })),
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
        32.0
    } else {
        MOBILE_MIN_TOUCH_TARGET
    };
    let is_muted = move || store::player_snapshot().volume <= 0.01;

    view()
        .w_full()
        .gap_1p5()
        .px_2p5()
        .py_1()
        .rounded(radius_card)
        .bg(pill_glass_surface)
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
                            16.0,
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
                        .gap_1()
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

/// Frosted glass control dock grouping seek, transport, and volume controls.
fn now_playing_control_dock(desktop: bool) -> impl IntoElement {
    view()
        .test_id("now-playing-control-dock")
        .w_full()
        .p_3p5()
        .gap_3()
        .rounded(radius_panel)
        .bg(frosted_card_surface)
        .border_1()
        .border_color(glass_hairline)
        .border_t_color(glass_specular_hairline)
        .child(seek_section(!desktop))
        .child(transport_bar())
        .child(volume_strip(desktop))
}

fn sponsorblock_badge() -> impl IntoElement {
    when(
        || store::player_snapshot().sponsorblock_enabled,
        || {
            view()
                .px_3()
                .py_1()
                .rounded(radius_pill)
                .bg(pill_glass_surface)
                .border_1()
                .border_color(color_sponsor_badge)
                .child(
                    text("SponsorBlock active")
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color_sponsor_badge),
                )
        },
    )
}

fn now_playing_tab_bar() -> impl IntoElement {
    let is_up_next = move || store::now_playing_tab() == NowPlayingTab::UpNext;
    let is_lyrics = move || store::now_playing_tab() == NowPlayingTab::Lyrics;
    let is_related = move || store::now_playing_tab() == NowPlayingTab::Related;

    tabs()
        .w_full()
        .flex_row()
        .gap_1()
        .p_1()
        .rounded(radius_card)
        .bg(pill_glass_surface)
        .indicator_style(|s| {
            s.top_1()
                .bottom_1()
                .rounded(radius_tab_inner)
                .bg(move || theme().primary)
        })
        .accessibility_label("Now Playing panels")
        .child(
            tab()
                .test_id("now-playing-tab-lyrics")
                .selected(is_lyrics)
                .on_press(|| store::set_now_playing_tab(NowPlayingTab::Lyrics))
                .flex_1()
                .min_h(px(36.0))
                .items_center()
                .justify_center()
                .rounded(radius_tab_inner)
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .child(
                    text("Lyrics")
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || {
                            if is_lyrics() {
                                theme().primary_foreground
                            } else {
                                theme().muted_foreground
                            }
                        }),
                ),
        )
        .child(
            tab()
                .test_id("now-playing-tab-up-next")
                .selected(is_up_next)
                .on_press(|| store::set_now_playing_tab(NowPlayingTab::UpNext))
                .flex_1()
                .min_h(px(36.0))
                .items_center()
                .justify_center()
                .rounded(radius_tab_inner)
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .child(
                    text("Up Next")
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || {
                            if is_up_next() {
                                theme().primary_foreground
                            } else {
                                theme().muted_foreground
                            }
                        }),
                ),
        )
        .child(
            tab()
                .test_id("now-playing-tab-related")
                .selected(is_related)
                .on_press(|| store::set_now_playing_tab(NowPlayingTab::Related))
                .flex_1()
                .min_h(px(36.0))
                .items_center()
                .justify_center()
                .rounded(radius_tab_inner)
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .child(
                    text("Related")
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || {
                            if is_related() {
                                theme().primary_foreground
                            } else {
                                theme().muted_foreground
                            }
                        }),
                ),
        )
}

fn now_playing_up_next_body() -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .gap_2p5()
        .child(setting_switch(
            "Autoplay similar songs",
            || store::player_snapshot().queue.autoplay_radio,
            store::set_autoplay_radio,
        ))
        .child(
            virtual_list(store::queue_len, move |at| {
                let track = computed(move || store::queue_track_at(at.index()));
                let active =
                    move || store::player_snapshot().queue.current_index == Some(at.index());
                button()
                    .w_full()
                    .min_h(px(52.0))
                    .px_2p5()
                    .py_1p5()
                    .rounded(radius_card)
                    .style_when(active, |s| s.bg(sidebar_active_surface))
                    .hover(|s| s.bg(pill_glass_hover))
                    .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                    .on_press(move || store::jump_queue(at.index()))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2p5()
                            .w_full()
                            .min_w(px(0.0))
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
                                        .text_color(move || {
                                            if active() {
                                                color_player_accent()
                                            } else {
                                                theme().foreground
                                            }
                                        }),
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
            })
            .keys(store::queue_ids)
            .estimated_item_size(px(54.0))
            .flex_1(),
        )
}

/// Large expressive synced karaoke lyrics rendered directly over the ambient canvas
/// (matching `07_now_playing_01_expanded.png` and `_04_lyrics.png` — zero heavy card border).
fn now_playing_lyrics_body() -> impl IntoElement {
    let has_lyrics = move || store::current_lyrics().is_some_and(|d| !d.lines.is_empty());

    view()
        .flex_1()
        .min_h(px(0.0))
        .gap_3()
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

                        button()
                            .w_full()
                            .min_h(px(52.0))
                            .px_2()
                            .py_3()
                            .rounded(radius_card)
                            .hover(|s| s.bg(pill_glass_surface))
                            .active(|s| s.opacity(0.85))
                            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
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
                                .text_2xl()
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
                .estimated_item_size(px(64.0))
                .flex_1()
            })
            .otherwise(|| {
                view().flex_1().items_center().justify_center().p_6().child(
                    text("Lyrics will appear here when available for the active track.")
                        .text_base()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
            }),
        )
        .child(
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .px_2()
                .child(
                    text(move || {
                        let label = match store::current_lyrics() {
                            Some(doc)
                                if !doc.lines.is_empty()
                                    && doc.sync_kind != LyricSyncKind::Plain =>
                            {
                                format!("Synced lyrics • {}", doc.provider)
                            }
                            Some(doc) if !doc.lines.is_empty() => {
                                format!("Lyrics • {}", doc.provider)
                            }
                            _ => "Lyrics unavailable".into(),
                        };
                        SharedString::from(label)
                    })
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().muted_foreground),
                )
                .child(sponsorblock_badge()),
        )
}

fn now_playing_related_body() -> impl IntoElement {
    scroll().flex_1().min_h(px(0.0)).child(dynamic(|| {
        let items = store::related_tracks();
        if items.is_empty() {
            return view()
                .p_6()
                .gap_2()
                .items_center()
                .justify_center()
                .child(
                    text("You might also like")
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text("Similar songs and recommendations appear here as tracks play.")
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .into_element();
        }
        view()
            .gap_1p5()
            .child(
                text("You might also like")
                    .px_2()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().muted_foreground),
            )
            .children(items.into_iter().map(|t| {
                let t_play = t.clone();
                let t_next = t.clone();
                let mono = t.fallback_monogram();
                let art_url = t.artwork_url.clone();
                let title = t.title.clone();
                let artist = t.artist_display();
                let dur = format_duration_ms(t.duration_ms);
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(radius_card)
                    .hover(|s| s.bg(pill_glass_hover))
                    .child(
                        button()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .flex_row()
                            .items_center()
                            .gap_2p5()
                            .rounded(move || theme().radius)
                            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                            .on_press(move || store::play_track(t_play.clone()))
                            .child(artwork_box(art_url, mono, 42.0, 6.0))
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
                                        text(format!("{artist} • {dur}"))
                                            .number_of_lines(1)
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                    ),
                            ),
                    )
                    .child(svg_icon_button(
                        IconKind::Plus,
                        15.0,
                        MOBILE_MIN_TOUCH_TARGET,
                        "Play Next",
                        true,
                        move || theme().muted_foreground,
                        move || store::play_next(t_next.clone()),
                    ))
            }))
            .into_element()
    }))
}

fn desktop_now_playing_right_panel() -> impl IntoElement {
    let is_up_next = move || store::now_playing_tab() == NowPlayingTab::UpNext;
    let is_related = move || store::now_playing_tab() == NowPlayingTab::Related;

    view()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .h_full()
        .gap_4()
        .px_4()
        .py_2()
        .child(
            view()
                .max_w(px(360.0))
                .child(now_playing_tab_bar()),
        )
        .child(
            when(is_up_next, now_playing_up_next_body).otherwise(move || {
                when(is_related, now_playing_related_body).otherwise(now_playing_lyrics_body)
            }),
        )
}

fn desktop_now_playing() -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .px_8()
        .py_5()
        .gap_5()
        .items_center()
        .child(
            view()
                .w_full()
                .max_w(px(1160.0))
                .child(top_header_bar(true)),
        )
        .child(
            view()
                .w_full()
                .max_w(px(1160.0))
                .flex_1()
                .min_h(px(0.0))
                .flex_row()
                .items_start()
                .gap_10()
                .child(
                    scroll().w(px(384.0)).h_full().child(
                        view()
                            .w_full()
                            .gap_4()
                            .items_center()
                            .pb_4()
                            .child(focal_artwork(340.0))
                            .child(track_info_and_actions())
                            .child(now_playing_control_dock(true)),
                    ),
                )
                .child(desktop_now_playing_right_panel()),
        )
}

fn mobile_now_playing_tabs_card() -> impl IntoElement {
    view()
        .w_full()
        .p_3()
        .gap_3()
        .rounded(radius_panel)
        .bg(frosted_card_surface)
        .border_1()
        .border_color(glass_hairline)
        .child(now_playing_tab_bar())
        .child(dynamic(|| match store::now_playing_tab() {
            NowPlayingTab::UpNext => {
                let snap = store::player_snapshot();
                let cur_idx = snap.queue.current_index;
                let items = snap.queue.items;
                view()
                    .gap_2()
                    .child(setting_switch(
                        "Autoplay similar songs",
                        || store::player_snapshot().queue.autoplay_radio,
                        store::set_autoplay_radio,
                    ))
                    .children(items.into_iter().enumerate().take(6).map(|(idx, t)| {
                        let active = cur_idx == Some(idx);
                        let title = t.title.clone();
                        let artist = t.artist_display();
                        let dur = format_duration_ms(t.duration_ms);
                        button()
                            .w_full()
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .px_2p5()
                            .py_1p5()
                            .rounded(radius_card)
                            .when(active, |b| b.bg(sidebar_active_surface))
                            .hover(|s| s.bg(pill_glass_hover))
                            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                            .on_press(move || store::jump_queue(idx))
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .child(
                                        view()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .child(
                                                text(title)
                                                    .number_of_lines(1)
                                                    .text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(move || {
                                                        if active {
                                                            color_player_accent()
                                                        } else {
                                                            theme().foreground
                                                        }
                                                    }),
                                            )
                                            .child(
                                                text(artist)
                                                    .number_of_lines(1)
                                                    .text_xs()
                                                    .text_color(move || theme().muted_foreground),
                                            ),
                                    )
                                    .child(
                                        text(dur)
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                    ),
                            )
                    }))
                    .into_element()
            }
            NowPlayingTab::Lyrics => {
                let preview_lines: Vec<String> = store::current_lyrics()
                    .map(|d| d.lines.into_iter().take(6).map(|l| l.text).collect())
                    .unwrap_or_default();
                let mut col = view().gap_2();
                if preview_lines.is_empty() {
                    col = col.child(
                        text("Lyrics will appear here when available for the active track.")
                            .text_sm()
                            .text_color(move || theme().muted_foreground),
                    );
                } else {
                    for line in preview_lines {
                        col = col.child(
                            text(line)
                                .text_base()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().foreground),
                        );
                    }
                }
                col.into_element()
            }
            NowPlayingTab::Related => {
                let items = store::related_tracks();
                if items.is_empty() {
                    return text("Similar songs appear here once loaded for the active track.")
                        .text_sm()
                        .text_color(move || theme().muted_foreground)
                        .into_element();
                }
                view()
                    .gap_1p5()
                    .children(items.into_iter().take(6).map(|t| {
                        let t_play = t.clone();
                        let mono = t.fallback_monogram();
                        let art_url = t.artwork_url.clone();
                        let title = t.title.clone();
                        let artist = t.artist_display();
                        button()
                            .w_full()
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .px_2()
                            .py_1p5()
                            .rounded(radius_card)
                            .hover(|s| s.bg(pill_glass_hover))
                            .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                            .on_press(move || store::play_track(t_play.clone()))
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2p5()
                                    .child(artwork_box(art_url, mono, 40.0, 6.0))
                                    .child(
                                        view()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .child(
                                                text(title)
                                                    .number_of_lines(1)
                                                    .text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(move || theme().foreground),
                                            )
                                            .child(
                                                text(artist)
                                                    .number_of_lines(1)
                                                    .text_xs()
                                                    .text_color(move || theme().muted_foreground),
                                            ),
                                    ),
                            )
                    }))
                    .into_element()
            }
        }))
}

fn mobile_now_playing() -> impl IntoElement {
    let art_size = if is_narrow_viewport() { 236.0 } else { 268.0 };
    scroll().flex_1().child(
        view()
            .w_full()
            .p_4()
            .gap_4()
            .items_center()
            .pb_safe_area_plus(px(16.0))
            .child(top_header_bar(false))
            .child(focal_artwork(art_size))
            .child(track_info_and_actions())
            .child(now_playing_control_dock(false))
            .child(sponsorblock_badge())
            .child(mobile_now_playing_tabs_card())
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
