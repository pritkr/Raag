//! Transport controls: seek slider, volume slider, Desktop Player Bar, and Mobile Mini-Player.
//! Zero raw color literals — semantic theme tokens only.

use nsh_base::{SliderValue, button, slider, theme};
use nsh_core::prelude::*;
use resonance_contract::format_duration_ms;

use crate::store::{self, AppTab, DesktopSidePanel};
use crate::ui::controls::primitives::svg_icon_button;
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_control, color_control_fg, color_liked, color_player_accent,
    color_player_surface, cover_gradient_pair, frosted_bar_surface, glass_hairline,
    is_desktop_panel_visible, pill_glass_hover,
};

/// Interactive `nsh_base::slider` for track seeking (used in player bar & Now Playing).
pub fn seek_slider(
    position_ms: impl Fn() -> f64 + 'static,
    duration_ms: impl Fn() -> f64 + 'static,
    on_seek: impl Fn(u64) + 'static,
) -> impl IntoElement {
    let dur_rc = std::rc::Rc::new(duration_ms);
    let dur_for_val = dur_rc.clone();
    slider()
        .value(move || {
            let d = dur_for_val();
            if d <= 0.0 {
                Some(SliderValue::Single(0.0))
            } else {
                Some(SliderValue::Single(position_ms().clamp(0.0, d)))
            }
        })
        .min(0.0)
        .max(move || {
            let d = dur_rc();
            if d <= 0.0 { 1000.0 } else { d }
        })
        .step(500.0)
        .accessibility_label("Playback Position")
        .track_class("w-full h-3 justify-center")
        .indicator_class("w-full h-1 rounded-full bg-muted")
        .range_class("rounded-full")
        .range_style(|s| s.bg(color_player_accent))
        .thumb_class("size-3 rounded-full opacity-0 group-hover:opacity-100 group-active:opacity-100")
        .thumb_style(|s| s.bg(color_player_accent))
        .on_change(move |v: &SliderValue| {
            on_seek(v.start().max(0.0).round() as u64);
        })
}

/// Interactive `nsh_base::slider` for volume control (0.0 ..= 1.6).
pub fn volume_slider(
    volume: impl Fn() -> f64 + 'static,
    on_change: impl Fn(f32) + 'static,
) -> impl IntoElement {
    slider()
        .value(move || Some(SliderValue::Single(volume())))
        .min(0.0)
        .max(1.6)
        .step(0.02)
        .accessibility_label("Volume")
        .track_class("w-24 h-6 justify-center")
        .indicator_class("w-full h-1 rounded-full bg-muted")
        .range_class("rounded-full")
        .range_style(|s| s.bg(color_player_accent))
        .thumb_class("size-3 rounded-full")
        .thumb_style(|s| s.bg(color_player_accent))
        .on_change(move |v: &SliderValue| {
            on_change(v.start() as f32);
        })
}

/// Sleek full-width Desktop Bottom Player Bar.
pub fn desktop_player_bar() -> impl IntoElement {
    let is_now_playing = move || store::active_tab() == AppTab::NowPlaying;
    let is_playing = move || store::player_snapshot().state.is_playing();
    let is_liked = move || {
        store::player_snapshot()
            .current_track
            .is_some_and(|t| t.is_liked)
    };
    let is_disliked = store::is_current_disliked;
    let is_shuffle = move || store::player_snapshot().queue.shuffle;

    view()
        .w_full()
        .bg(color_player_surface)
        .border_t_1()
        .border_color(glass_hairline)
        .child(
            view()
                .w_full()
                .h(px(3.0))
                .bg(move || theme().muted)
                .overflow_hidden()
                .child(
                    view()
                        .test_id("mini-player-progress")
                        .h_full()
                        .w(move || {
                            let snap = store::player_snapshot();
                            let frac = if snap.duration_ms > 0 {
                                snap.position_ms as f32 / snap.duration_ms as f32
                            } else {
                                0.0
                            };
                            percentage(frac.clamp(0.0, 1.0))
                        })
                        .bg(color_player_accent),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .px_4()
                .py_2()
                .min_h(px(72.0))
                .gap_4()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            button()
                                .test_id("mini-player-open")
                                .accessibility_label("Open Now Playing")
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .min_w(px(0.0))
                                .max_w(px(280.0))
                                .active(|s| s.opacity(0.8))
                                .on_press(|| store::set_tab(AppTab::NowPlaying))
                                .child(
                                    view()
                                        .w(px(46.0))
                                        .h(px(46.0))
                                        .rounded(px(6.0))
                                        .bg(move || {
                                            let seed = store::player_snapshot()
                                                .current_track
                                                .map(|t| t.id)
                                                .unwrap_or_default();
                                            cover_gradient_pair(&seed).0
                                        })
                                        .border_1()
                                        .border_color(glass_hairline)
                                        .items_center()
                                        .justify_center()
                                        .overflow_hidden()
                                        .child(
                                            when(
                                                || {
                                                    store::player_snapshot()
                                                        .current_track
                                                        .and_then(|t| t.artwork_url)
                                                        .is_some()
                                                },
                                                || {
                                                    image(move || {
                                                        let raw = store::player_snapshot()
                                                            .current_track
                                                            .and_then(|t| t.artwork_url);
                                                        nsh_core::element::ImageSrc::from(
                                                            store::resolve_artwork_url(raw)
                                                                .unwrap_or_default(),
                                                        )
                                                    })
                                                    .size(px(46.0))
                                                    .object_fit(ResizeMode::Cover)
                                                    .rounded(px(6.0))
                                                },
                                            )
                                            .otherwise(|| {
                                                text(move || {
                                                    SharedString::from(
                                                        store::player_snapshot()
                                                            .current_track
                                                            .map(|t| t.fallback_monogram())
                                                            .unwrap_or_else(|| "♪".into()),
                                                    )
                                                })
                                                .test_id("mini-player-monogram")
                                                .text_base()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(move || theme().primary_foreground)
                                            }),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .gap_0p5()
                                        .child(
                                            text(move || {
                                                SharedString::from(
                                                    store::player_snapshot()
                                                        .current_track
                                                        .map(|t| t.title)
                                                        .unwrap_or_else(|| "Nothing playing".into()),
                                                )
                                            })
                                            .number_of_lines(1)
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text(move || {
                                                SharedString::from(
                                                    store::player_snapshot()
                                                        .current_track
                                                        .map(|t| t.artist_display())
                                                        .unwrap_or_else(|| "Choose a track".into()),
                                                )
                                            })
                                            .number_of_lines(1)
                                            .text_xs()
                                            .text_color(move || theme().muted_foreground),
                                        ),
                                ),
                        )
                        .child(
                            button()
                                .accessibility_label("Player Bar Like")
                                .w(px(34.0))
                                .h(px(34.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::toggle_like_current)
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
                                .accessibility_label("Player Bar Dislike")
                                .w(px(34.0))
                                .h(px(34.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::toggle_dislike_current)
                                .child(icon(IconKind::ThumbsDown, 15.0, move || {
                                    if is_disliked() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                })),
                        )
                        .child(svg_icon_button(
                            IconKind::MoreVertical,
                            15.0,
                            34.0,
                            "Open Queue",
                            true,
                            move || theme().muted_foreground,
                            || store::set_tab(AppTab::Queue),
                        )),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2p5()
                        .child(
                            button()
                                .accessibility_label("Player Bar Shuffle")
                                .w(px(36.0))
                                .h(px(36.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::toggle_shuffle)
                                .child(icon(IconKind::Shuffle, 16.0, move || {
                                    if is_shuffle() {
                                        color_player_accent()
                                    } else {
                                        theme().muted_foreground
                                    }
                                })),
                        )
                        .child(svg_icon_button(
                            IconKind::SkipBack,
                            18.0,
                            38.0,
                            "Player Bar Previous Track",
                            true,
                            move || theme().foreground,
                            store::previous_track,
                        ))
                        .child(
                            button()
                                .accessibility_label("Player Bar Play or Pause")
                                .w(px(44.0))
                                .h(px(44.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(color_control)
                                .hover(|s| s.opacity(0.92))
                                .active(|s| s.opacity(0.8))
                                .on_press(store::toggle_play_pause)
                                .child(reactive_icon(
                                    move || {
                                        if is_playing() {
                                            IconKind::Pause
                                        } else {
                                            IconKind::Play
                                        }
                                    },
                                    18.0,
                                    color_control_fg,
                                )),
                        )
                        .child(svg_icon_button(
                            IconKind::SkipForward,
                            18.0,
                            38.0,
                            "Player Bar Next Track",
                            true,
                            move || theme().foreground,
                            store::next_track,
                        ))
                        .child(
                            button()
                                .accessibility_label("Player Bar Repeat Mode")
                                .w(px(36.0))
                                .h(px(36.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::cycle_repeat)
                                .child(reactive_icon(
                                    move || match store::player_snapshot().queue.repeat {
                                        resonance_contract::RepeatMode::One => IconKind::RepeatOne,
                                        _ => IconKind::Repeat,
                                    },
                                    16.0,
                                    move || {
                                        if store::player_snapshot().queue.repeat
                                            == resonance_contract::RepeatMode::None
                                        {
                                            theme().muted_foreground
                                        } else {
                                            color_player_accent()
                                        }
                                    },
                                )),
                        ),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .justify_end()
                        .gap_2p5()
                        .child(
                            text(move || {
                                let snap = store::player_snapshot();
                                SharedString::from(format!(
                                    "{} / {}",
                                    format_duration_ms(snap.position_ms),
                                    format_duration_ms(snap.duration_ms)
                                ))
                            })
                            .text_xs()
                            .tabular_nums()
                            .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            button()
                                .accessibility_label("Player Bar Mute")
                                .w(px(34.0))
                                .h(px(34.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::toggle_mute)
                                .child(reactive_icon(
                                    move || {
                                        if store::player_snapshot().volume <= 0.01 {
                                            IconKind::VolumeMute
                                        } else {
                                            IconKind::VolumeHigh
                                        }
                                    },
                                    16.0,
                                    move || theme().foreground,
                                )),
                        )
                        .child(volume_slider(
                            || f64::from(store::player_snapshot().volume),
                            store::set_volume,
                        ))
                        .child(svg_icon_button(
                            IconKind::Lyrics,
                            16.0,
                            34.0,
                            "Toggle Lyrics",
                            true,
                            move || theme().muted_foreground,
                            || {
                                if is_desktop_panel_visible() {
                                    store::toggle_desktop_side_panel(DesktopSidePanel::Lyrics);
                                } else {
                                    store::set_tab(AppTab::Lyrics);
                                }
                            },
                        ))
                        .child(svg_icon_button(
                            IconKind::Queue,
                            16.0,
                            34.0,
                            "Toggle Queue",
                            true,
                            move || theme().muted_foreground,
                            || {
                                if is_desktop_panel_visible() {
                                    store::toggle_desktop_side_panel(DesktopSidePanel::Queue);
                                } else {
                                    store::set_tab(AppTab::Queue);
                                }
                            },
                        ))
                        .child(
                            button()
                                .accessibility_label("Toggle Fullscreen Player")
                                .w(px(34.0))
                                .h(px(34.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(move || {
                                    if is_now_playing() {
                                        store::go_back();
                                    } else {
                                        store::set_tab(AppTab::NowPlaying);
                                    }
                                })
                                .child(reactive_icon(
                                    move || {
                                        if is_now_playing() {
                                            IconKind::ChevronDown
                                        } else {
                                            IconKind::ChevronUp
                                        }
                                    },
                                    17.0,
                                    move || theme().foreground,
                                )),
                        ),
                ),
        )
}

/// Persistent Mobile Mini-Player bar docked strictly above mobile bottom navigation.
#[allow(clippy::too_many_arguments)]
pub fn mini_player_bar(
    title: impl Fn() -> SharedString + 'static,
    artist: impl Fn() -> SharedString + 'static,
    monogram: impl Fn() -> SharedString + 'static,
    state_label: impl Fn() -> SharedString + 'static,
    progress_pct: impl Fn() -> f32 + 'static,
    is_playing: impl Fn() -> bool + 'static,
    on_open_now_playing: impl Fn() + 'static,
    on_toggle_play: impl Fn() + 'static,
    on_next: impl Fn() + 'static,
) -> impl IntoElement {
    let monogram_rc = std::rc::Rc::new(monogram);
    view()
        .backdrop_blur(px(16.0))
        .backdrop_material(BackdropMaterial::Regular)
        .bg(frosted_bar_surface)
        .border_t_1()
        .border_color(glass_hairline)
        .child(
            view()
                .w_full()
                .h(px(2.0))
                .bg(move || theme().muted)
                .overflow_hidden()
                .child(
                    view()
                        .test_id("mini-player-progress")
                        .h_full()
                        .w(move || percentage((progress_pct() / 100.0).clamp(0.0, 1.0)))
                        .bg(color_player_accent),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .min_h(px(64.0))
                .child(
                    button()
                        .test_id("mini-player-open")
                        .accessibility_label("Open Now Playing")
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .on_press(on_open_now_playing)
                        .child(
                            view()
                                .w(px(42.0))
                                .h(px(42.0))
                                .rounded_md()
                                .bg(move || {
                                    let seed = store::player_snapshot()
                                        .current_track
                                        .map(|t| t.id)
                                        .unwrap_or_default();
                                    cover_gradient_pair(&seed).0
                                })
                                .border_1()
                                .border_color(glass_hairline)
                                .items_center()
                                .justify_center()
                                .overflow_hidden()
                                .child(
                                    when(
                                        || {
                                            store::player_snapshot()
                                                .current_track
                                                .and_then(|t| t.artwork_url)
                                                .is_some()
                                        },
                                        || {
                                            image(move || {
                                                let raw = store::player_snapshot()
                                                    .current_track
                                                    .and_then(|t| t.artwork_url);
                                                nsh_core::element::ImageSrc::from(
                                                    store::resolve_artwork_url(raw)
                                                        .unwrap_or_default(),
                                                )
                                            })
                                            .size(px(42.0))
                                            .object_fit(ResizeMode::Cover)
                                            .rounded_md()
                                        },
                                    )
                                    .otherwise({
                                        let m = monogram_rc.clone();
                                        move || {
                                            let m = m.clone();
                                            text(move || m())
                                                .test_id("mini-player-monogram")
                                                .text_base()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(move || theme().primary_foreground)
                                        }
                                    }),
                                ),
                        )
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
                                    text(move || format!("{} • {}", artist(), state_label()))
                                        .number_of_lines(1)
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        ),
                )
                .child(
                    button()
                        .on_press(on_toggle_play)
                        .accessibility_label("Toggle Play Pause")
                        .w(px(MOBILE_MIN_TOUCH_TARGET))
                        .h(px(MOBILE_MIN_TOUCH_TARGET))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(color_control)
                        .child(reactive_icon(
                            move || {
                                if is_playing() {
                                    IconKind::Pause
                                } else {
                                    IconKind::Play
                                }
                            },
                            18.0,
                            color_control_fg,
                        )),
                )
                .child(svg_icon_button(
                    IconKind::SkipForward,
                    18.0,
                    MOBILE_MIN_TOUCH_TARGET,
                    "Next Track",
                    false,
                    move || theme().foreground,
                    on_next,
                )),
        )
}
