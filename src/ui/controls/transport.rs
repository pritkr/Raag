//! Transport controls: seek slider, volume slider, Desktop Player Bar, and Mobile Mini-Player.
//! Zero raw color literals — semantic theme tokens only.

use nsh_base::{SliderValue, button, slider, theme};
use nsh_core::prelude::*;
use resonance_contract::format_duration_ms;

use crate::store::{self, AppTab, DesktopSidePanel};
use crate::ui::controls::ambient::{animated_like_heart, animated_play_pause_icon};
use crate::ui::controls::context_menu::{TrackMenuContext, track_context_menu};
use crate::ui::controls::primitives::{active_state_dot, svg_icon_button};
use crate::ui::controls::sleep_timer_sheet::{open_sleep_timer_sheet, sleep_timer_badge_label};
use crate::ui::controls::speed_sheet::{open_speed_sheet, speed_badge_label};
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, ambient_progress_color, color_control, color_control_fg, color_liked,
    color_on_cover, color_player_accent, cover_gradient_pair, frosted_bar_surface, glass_hairline,
    glass_specular_hairline, is_desktop_panel_visible, motion, pill_glass_hover,
    pill_glass_surface,
};

fn btn_transition_ms() -> f32 {
    motion::transition_ms(motion::PRESS_MS, store::reduced_motion())
}

fn btn_press_scale() -> f32 {
    if store::reduced_motion() {
        1.0
    } else {
        motion::PRESS_SCALE
    }
}

fn trigger_panel_layout_animation() {
    if !store::reduced_motion() {
        let _ = std::panic::catch_unwind(|| {
            animate_layout(nsh_core::LayoutAnimation::EASE_IN_EASE_OUT);
        });
    }
}

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
        .track_class("w-full h-6 justify-center")
        .indicator_class("w-full h-1.5 rounded-full bg-muted")
        .range_class("rounded-full")
        .range_style(|s| s.bg(|| ambient_progress_color(store::active_ambient_palette())))
        .thumb_class("size-3.5 rounded-full")
        .thumb_style(|s| s.bg(|| ambient_progress_color(store::active_ambient_palette())))
        .on_change(move |v: &SliderValue| {
            on_seek(v.start().max(0.0).round() as u64);
        })
}

/// Interactive `nsh_base::slider` for volume control (0.0 ..= 1.0).
pub fn volume_slider(
    volume: impl Fn() -> f64 + 'static,
    on_change: impl Fn(f32) + 'static,
) -> impl IntoElement {
    slider()
        .value(move || Some(SliderValue::Single(volume())))
        .min(0.0)
        .max(1.0)
        .step(0.02)
        .accessibility_label("Volume")
        .track_class("w-16 h-6 justify-center")
        .indicator_class("w-full h-1 rounded-full bg-muted")
        .range_class("rounded-full")
        .range_style(|s| s.bg(|| ambient_progress_color(store::active_ambient_palette())))
        .thumb_class("size-2.5 rounded-full")
        .thumb_style(|s| s.bg(|| ambient_progress_color(store::active_ambient_palette())))
        .on_change(move |v: &SliderValue| {
            on_change(v.start() as f32);
        })
}

/// Sleek frosted-glass Desktop Bottom Player Bar matching `ytm-rs`:
/// - Thin full-width progress bar flush along top edge tinted with active track's accent color
/// - Left: square art, title, artist, Like, Dislike, More
/// - Center: Shuffle, Previous, large circular Play/Pause, Next, Repeat
/// - Right: elapsed / duration, Volume, Cast/Device, Lyrics, Queue, Miniplayer, Expand Now Playing
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
    let is_lyrics_open = move || {
        store::active_tab() == AppTab::Lyrics
            || (store::desktop_side_panel_open()
                && store::desktop_side_panel() == DesktopSidePanel::Lyrics)
    };
    let is_queue_open = move || {
        store::active_tab() == AppTab::Queue
            || (store::desktop_side_panel_open()
                && store::desktop_side_panel() == DesktopSidePanel::Queue)
    };

    view()
        .test_id("desktop-player-bar")
        .w_full()
        .backdrop_blur(px(28.0))
        .backdrop_material(BackdropMaterial::Chrome)
        .bg(frosted_bar_surface)
        .border_t_1()
        .border_color(glass_hairline)
        // Thin full-width interactive progress bar flush along the top edge
        .child(
            view()
                .test_id("mini-player-progress")
                .w_full()
                .child(
                    slider()
                        .value(move || {
                            let snap = store::player_snapshot();
                            let d = snap.duration_ms as f64;
                            if d <= 0.0 {
                                Some(SliderValue::Single(0.0))
                            } else {
                                Some(SliderValue::Single((snap.position_ms as f64).clamp(0.0, d)))
                            }
                        })
                        .min(0.0)
                        .max(move || {
                            let d = store::player_snapshot().duration_ms as f64;
                            if d <= 0.0 { 1000.0 } else { d }
                        })
                        .step(500.0)
                        .accessibility_label("Desktop Playback Seek Bar")
                        .track_class("w-full h-1.5 justify-start")
                        .indicator_class("w-full h-0.5 bg-muted")
                        .range_class("h-0.5")
                        .range_style(|s| {
                            s.bg(|| ambient_progress_color(store::active_ambient_palette()))
                        })
                        .thumb_class("size-2 rounded-full")
                        .thumb_style(|s| {
                            s.bg(|| ambient_progress_color(store::active_ambient_palette()))
                        })
                        .on_change(|v: &SliderValue| {
                            store::seek_ms(v.start().max(0.0).round() as u64);
                        }),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .px_4()
                .pb_2()
                .pt_1()
                .min_h(px(64.0))
                .gap_3()
                // Left: Square artwork + Title/Artist + Like / Dislike / More
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .gap_2p5()
                        .child(
                            button()
                                .test_id("mini-player-open")
                                .accessibility_label("Open Now Playing")
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .min_w(px(0.0))
                                .max_w(px(260.0))
                                .rounded(move || theme().radius)
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.opacity(0.92))
                                .active(|s| s.scale(btn_press_scale).opacity(0.84))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(|| store::set_tab(AppTab::NowPlaying))
                                .child(
                                    view()
                                        .w(px(44.0))
                                        .h(px(44.0))
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
                                                    .size(px(44.0))
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
                                                .text_color(color_on_cover)
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
                                                        .unwrap_or_else(|| {
                                                            "Nothing playing".into()
                                                        }),
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
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(store::toggle_like_current)
                                .child(animated_like_heart(
                                    is_liked,
                                    16.0,
                                    color_liked,
                                    move || theme().muted_foreground,
                                )),
                        )
                        .child(
                            button()
                                .selected(is_disliked)
                                .accessibility_label(move || {
                                    Some(SharedString::from(if is_disliked() {
                                        "Player Bar Dislike (Active)"
                                    } else {
                                        "Player Bar Dislike"
                                    }))
                                })
                                .w(px(34.0))
                                .h(px(34.0))
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_0p5()
                                .rounded_full()
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(store::toggle_dislike_current)
                                .child(icon(IconKind::ThumbsDown, 15.0, move || {
                                    if is_disliked() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                }))
                                .child(active_state_dot(is_disliked)),
                        )
                        .child(dynamic(|| {
                            if let Some(track) = store::player_snapshot().current_track {
                                track_context_menu(track, TrackMenuContext::General).into_element()
                            } else {
                                svg_icon_button(
                                    IconKind::MoreVertical,
                                    15.0,
                                    34.0,
                                    "Track Actions",
                                    true,
                                    move || theme().muted_foreground,
                                    || {},
                                )
                                .into_element()
                            }
                        })),
                )
                // Center: Shuffle, Previous, circular Play/Pause, Next, Repeat
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            button()
                                .selected(is_shuffle)
                                .accessibility_label(move || {
                                    Some(SharedString::from(if is_shuffle() {
                                        "Player Bar Shuffle (On)"
                                    } else {
                                        "Player Bar Shuffle"
                                    }))
                                })
                                .w(px(36.0))
                                .h(px(36.0))
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_0p5()
                                .rounded_full()
                                .style_when(is_shuffle, |s| s.bg(pill_glass_surface))
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(store::toggle_shuffle)
                                .child(icon(IconKind::Shuffle, 16.0, move || {
                                    if is_shuffle() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                }))
                                .child(active_state_dot(is_shuffle)),
                        )
                        .child(svg_icon_button(
                            IconKind::SkipBack,
                            18.0,
                            36.0,
                            "Player Bar Previous Track",
                            true,
                            move || theme().foreground,
                            store::previous_track,
                        ))
                        .child(
                            button()
                                .accessibility_label("Player Bar Play or Pause")
                                .w(px(42.0))
                                .h(px(42.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(color_control)
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.opacity(0.92))
                                .active(|s| s.scale(btn_press_scale).opacity(0.86))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(store::toggle_play_pause)
                                .child(animated_play_pause_icon(
                                    is_playing,
                                    18.0,
                                    color_control_fg,
                                )),
                        )
                        .child(svg_icon_button(
                            IconKind::SkipForward,
                            18.0,
                            36.0,
                            "Player Bar Next Track",
                            true,
                            move || theme().foreground,
                            store::next_track,
                        ))
                        .child(
                            button()
                                .selected(move || {
                                    store::player_snapshot().queue.repeat
                                        != resonance_contract::RepeatMode::None
                                })
                                .accessibility_label(move || {
                                    Some(SharedString::from(
                                        match store::player_snapshot().queue.repeat {
                                            resonance_contract::RepeatMode::None => {
                                                "Player Bar Repeat Mode"
                                            }
                                            resonance_contract::RepeatMode::All => {
                                                "Player Bar Repeat Mode (All)"
                                            }
                                            resonance_contract::RepeatMode::One => {
                                                "Player Bar Repeat Mode (One)"
                                            }
                                        },
                                    ))
                                })
                                .w(px(36.0))
                                .h(px(36.0))
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_0p5()
                                .rounded_full()
                                .style_when(
                                    move || {
                                        store::player_snapshot().queue.repeat
                                            != resonance_contract::RepeatMode::None
                                    },
                                    |s| s.bg(pill_glass_surface),
                                )
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
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
                                            theme().foreground
                                        }
                                    },
                                ))
                                .child(active_state_dot(move || {
                                    store::player_snapshot().queue.repeat
                                        != resonance_contract::RepeatMode::None
                                })),
                        ),
                )
                // Right: elapsed / duration, Volume, Cast/Device, Lyrics, Queue, Miniplayer, Expand ^
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .justify_end()
                        .gap_1()
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
                            .text_color(move || theme().muted_foreground)
                            .pr_1(),
                        )
                        .child(when(
                            || speed_badge_label().is_some(),
                            || {
                                button()
                                    .accessibility_label("Adjust Playback Speed")
                                    .on_press(open_speed_sheet)
                                    .px_2()
                                    .py_1()
                                    .rounded_full()
                                    .bg(pill_glass_surface)
                                    .transition()
                                    .duration(btn_transition_ms)
                                    .ease_out()
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .active(|s| s.scale(btn_press_scale))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
                                    .child(
                                        text(move || {
                                            SharedString::from(
                                                speed_badge_label().unwrap_or_default(),
                                            )
                                        })
                                        .text_xs()
                                        .tabular_nums()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                    )
                            },
                        ))
                        .child(when(
                            || sleep_timer_badge_label().is_some(),
                            || {
                                button()
                                    .accessibility_label("Adjust Sleep Timer")
                                    .on_press(open_sleep_timer_sheet)
                                    .px_2()
                                    .py_1()
                                    .flex_row()
                                    .items_center()
                                    .gap_1()
                                    .rounded_full()
                                    .bg(pill_glass_surface)
                                    .transition()
                                    .duration(btn_transition_ms)
                                    .ease_out()
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .active(|s| s.scale(btn_press_scale))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
                                    .child(icon(IconKind::Moon, 12.0, move || theme().foreground))
                                    .child(
                                        text(move || {
                                            SharedString::from(
                                                sleep_timer_badge_label().unwrap_or_default(),
                                            )
                                        })
                                        .text_xs()
                                        .tabular_nums()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                    )
                            },
                        ))
                        .child(
                            button()
                                .accessibility_label("Player Bar Mute")
                                .w(px(32.0))
                                .h(px(32.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
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
                        // Cast / Audio Output Device button
                        .child(
                            button()
                                .test_id("player-cast-btn")
                                .accessibility_label("Cast to Device")
                                .w(px(32.0))
                                .h(px(32.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(|| store::set_cast_sheet_open(true))
                                .child(icon(IconKind::Cast, 16.0, move || {
                                    theme().muted_foreground
                                })),
                        )
                        // Lyrics microphone button (also carries `nav-lyrics` test_id)
                        .child(
                            button()
                                .test_id("nav-lyrics")
                                .selected(is_lyrics_open)
                                .accessibility_label("Lyrics")
                                .w(px(32.0))
                                .h(px(32.0))
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_0p5()
                                .rounded_full()
                                .style_when(is_lyrics_open, |s| s.bg(pill_glass_surface))
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(|| {
                                    if is_desktop_panel_visible()
                                        && store::active_tab() != AppTab::NowPlaying
                                    {
                                        trigger_panel_layout_animation();
                                        store::toggle_desktop_side_panel(DesktopSidePanel::Lyrics);
                                    } else {
                                        store::set_tab(AppTab::Lyrics);
                                    }
                                })
                                .child(icon(IconKind::Lyrics, 15.0, move || {
                                    if is_lyrics_open() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                }))
                                .child(active_state_dot(is_lyrics_open)),
                        )
                        // Queue drawer button (also carries `nav-queue` test_id)
                        .child(
                            button()
                                .test_id("nav-queue")
                                .selected(is_queue_open)
                                .accessibility_label("Queue")
                                .w(px(32.0))
                                .h(px(32.0))
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_0p5()
                                .rounded_full()
                                .style_when(is_queue_open, |s| s.bg(pill_glass_surface))
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(|| {
                                    if is_desktop_panel_visible() {
                                        trigger_panel_layout_animation();
                                        store::toggle_desktop_side_panel(DesktopSidePanel::Queue);
                                    } else {
                                        store::set_tab(AppTab::Queue);
                                    }
                                })
                                .child(icon(IconKind::Queue, 16.0, move || {
                                    if is_queue_open() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                }))
                                .child(active_state_dot(is_queue_open)),
                        )
                        // Miniplayer / Popout toggle
                        .child(
                            button()
                                .test_id("player-miniplayer-btn")
                                .selected(store::miniplayer_open)
                                .accessibility_label("Toggle Miniplayer")
                                .w(px(32.0))
                                .h(px(32.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .style_when(store::miniplayer_open, |s| s.bg(pill_glass_surface))
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(store::toggle_miniplayer)
                                .child(icon(IconKind::Miniplayer, 15.0, move || {
                                    if store::miniplayer_open() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                })),
                        )
                        // Expand / Collapse Now Playing chevron (also carries `nav-now-playing` test_id)
                        .child(
                            button()
                                .test_id("nav-now-playing")
                                .selected(is_now_playing)
                                .accessibility_label("Now Playing")
                                .w(px(32.0))
                                .h(px(32.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .transition()
                                .duration(btn_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(btn_press_scale))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
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
        .backdrop_blur(px(20.0))
        .backdrop_material(BackdropMaterial::Chrome)
        .bg(frosted_bar_surface)
        .border_t_1()
        .border_color(glass_specular_hairline)
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
                        .rounded(move || theme().radius)
                        .transition()
                        .duration(btn_transition_ms)
                        .ease_out()
                        .active(|s| s.scale(btn_press_scale).opacity(0.84))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
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
                                                .text_color(color_on_cover)
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
                        .transition()
                        .duration(btn_transition_ms)
                        .ease_out()
                        .hover(|s| s.opacity(0.92))
                        .active(|s| s.scale(btn_press_scale).opacity(0.86))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .child(animated_play_pause_icon(
                            is_playing,
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
