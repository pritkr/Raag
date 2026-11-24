//! Priority Visual Surface #1: Now Playing Screen.
//! Full-bleed focal artwork (with deterministic monogram fallback), 2-line ellipsized
//! track title, tabular-nums progress scrubber & seek controls, >= 48dp transport
//! controls, SponsorBlock indicator, and quick tabs to Lyrics & Queue.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{RepeatMode, format_duration_ms};

use crate::store::{self, AppTab};
use crate::ui::controls::{icon_button, secondary_button, status_card};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, ambient_accent_color, ambient_backdrop_color, color_liked,
    color_sponsor_badge, frosted_card_surface, glass_hairline, radius_card, radius_panel,
};

pub fn now_playing_screen() -> impl IntoElement {
    let has_track = move || store::player_snapshot().current_track.is_some();

    scroll()
        .flex_1()
        .bg(move || ambient_backdrop_color(store::active_ambient_palette()))
        .child(
            view()
                .p_5()
                .gap_5()
                .items_center()
                .child(
                    when(has_track, || {
                        view()
                            .w_full()
                            .max_w(px(560.0))
                            .gap_5()
                            .items_center()
                            // Frosted header bar with playback state + stream source badge
                            .child(
                                view()
                                    .w_full()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .px_4()
                                    .py_2p5()
                                    .rounded(radius_card)
                                    .backdrop_blur(px(14.0))
                                    .backdrop_material(BackdropMaterial::Thin)
                                    .bg(frosted_card_surface)
                                    .border_1()
                                    .border_color(glass_hairline)
                                    .child(
                                        view()
                                            .gap_0p5()
                                            .child(
                                                text("Now Playing")
                                                    .text_base()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(move || theme().foreground),
                                            )
                                            .child(
                                                text("PLAYING FROM • QUEUE")
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
                                                text(move || {
                                                    let snap = store::player_snapshot();
                                                    if snap.offline_mode {
                                                        SharedString::from("Offline Cache")
                                                    } else {
                                                        SharedString::from("Opus 251 • Proxy")
                                                    }
                                                })
                                                .test_id("now-playing-source-badge")
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                            )
                                            .child(
                                                text(move || {
                                                    SharedString::from(
                                                        store::player_snapshot().state.name(),
                                                    )
                                                })
                                                .test_id("player-state-label")
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || theme().primary),
                                            ),
                                    ),
                            )
                            // Focal Artwork Card with artwork-driven ambient halo frame
                            .child(
                                view()
                                    .p_2()
                                    .rounded(radius_panel)
                                    .bg(move || {
                                        ambient_accent_color(store::active_ambient_palette())
                                    })
                                    .border_1()
                                    .border_color(glass_hairline)
                                    .child(
                                        view()
                                            .w(px(268.0))
                                            .h(px(268.0))
                                            .rounded(radius_card)
                                            .bg(move || theme().card)
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .items_center()
                                            .justify_center()
                                            .overflow_hidden()
                                            .child(
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
                                                .text_color(move || theme().foreground),
                                            ),
                                    ),
                            )
                        // Title (max 2 lines + ellipsis), Artist + Explicit tag, Like toggle
                        .child(
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
                                                                    .text_color(move || {
                                                                        theme().muted_foreground
                                                                    }),
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
                                    button()
                                        .test_id("btn-like")
                                        .accessibility_label("Toggle Like")
                                        .w(px(MOBILE_MIN_TOUCH_TARGET))
                                        .h(px(MOBILE_MIN_TOUCH_TARGET))
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(move || theme().secondary)
                                        .active(|s| s.opacity(0.7))
                                        .on_press(store::toggle_like_current)
                                        .child(
                                            text(move || {
                                                let liked = store::player_snapshot()
                                                    .current_track
                                                    .is_some_and(|t| t.is_liked);
                                                if liked { "♥" } else { "♡" }
                                            })
                                            .text_lg()
                                            .text_color(move || {
                                                let liked = store::player_snapshot()
                                                    .current_track
                                                    .is_some_and(|t| t.is_liked);
                                                if liked {
                                                    color_liked()
                                                } else {
                                                    theme().foreground
                                                }
                                            }),
                                        ),
                                ),
                        )
                        // Progress Bar + Timestamps + Accessible Seek Steps
                        .child(
                            view()
                                .w_full()
                                .gap_2()
                                .child(
                                    view()
                                        .w_full()
                                        .h(px(6.0))
                                        .rounded_full()
                                        .bg(move || theme().muted)
                                        .overflow_hidden()
                                        .child(
                                            view()
                                                .h_full()
                                                .w(move || {
                                                    let snap = store::player_snapshot();
                                                    let ratio = if snap.duration_ms > 0 {
                                                        (snap.position_ms as f32
                                                            / snap.duration_ms as f32)
                                                            .clamp(0.0, 1.0)
                                                    } else {
                                                        0.0
                                                    };
                                                    percentage(ratio * 100.0)
                                                })
                                                .bg(move || theme().primary),
                                        ),
                                )
                                .child(
                                    view()
                                        .w_full()
                                        .flex_row()
                                        .justify_between()
                                        .child(
                                            text(move || {
                                                SharedString::from(format_duration_ms(
                                                    store::player_snapshot().position_ms,
                                                ))
                                            })
                                            .test_id("player-position")
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                        )
                                        .child(
                                            text(move || {
                                                SharedString::from(format_duration_ms(
                                                    store::player_snapshot().duration_ms,
                                                ))
                                            })
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                        ),
                                )
                                .child(
                                    view()
                                        .w_full()
                                        .flex_row()
                                        .justify_center()
                                        .gap_3()
                                        .child(secondary_button("-15s", || {
                                            let pos = store::player_snapshot().position_ms;
                                            store::seek_ms(pos.saturating_sub(15_000));
                                        }))
                                        .child(secondary_button("+15s", || {
                                            let pos = store::player_snapshot().position_ms;
                                            store::seek_ms(pos.saturating_add(15_000));
                                        })),
                                ),
                        )
                        // Transport Bar (all >= 48dp touch targets)
                        .child(
                            view()
                                .w_full()
                                .flex_row()
                                .items_center()
                                .justify_around()
                                .child(icon_button(
                                    move || {
                                        if store::player_snapshot().queue.shuffle {
                                            "⇄•"
                                        } else {
                                            "⇄"
                                        }
                                    },
                                    "Toggle Shuffle",
                                    false,
                                    store::toggle_shuffle,
                                ))
                                .child(icon_button(
                                    "⏮",
                                    "Previous Track",
                                    false,
                                    store::previous_track,
                                ))
                                .child(
                                    button()
                                        .accessibility_label("Play or Pause")
                                        .w(px(60.0))
                                        .h(px(60.0))
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(crate::ui::tokens::color_control)
                                        .hover(|s| s.opacity(0.9))
                                        .active(|s| s.opacity(0.78))
                                        .on_press(store::toggle_play_pause)
                                        .child(
                                            text(move || {
                                                if store::player_snapshot().state.is_playing() {
                                                    "❚❚"
                                                } else {
                                                    "▶"
                                                }
                                            })
                                            .text_xl()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(crate::ui::tokens::color_control_fg),
                                        ),
                                )
                                .child(icon_button("⏭", "Next Track", false, store::next_track))
                                .child(icon_button(
                                    move || match store::player_snapshot().queue.repeat {
                                        RepeatMode::None => "↻",
                                        RepeatMode::All => "↻A",
                                        RepeatMode::One => "↻1",
                                    },
                                    "Cycle Repeat Mode",
                                    false,
                                    store::cycle_repeat,
                                )),
                        )
                        // Volume & Mute Strip (frosted glass card, all >= 48dp touch targets)
                        .child(
                            view()
                                .w_full()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .px_3()
                                .py_1p5()
                                .rounded(radius_card)
                                .backdrop_blur(px(14.0))
                                .backdrop_material(BackdropMaterial::Thin)
                                .bg(frosted_card_surface)
                                .border_1()
                                .border_color(glass_hairline)
                                .child(icon_button(
                                    move || {
                                        if store::player_snapshot().volume <= 0.01 {
                                            "🔇"
                                        } else {
                                            "🔊"
                                        }
                                    },
                                    "Toggle Mute",
                                    false,
                                    store::toggle_mute,
                                ))
                                .child(
                                    text(move || {
                                        let pct =
                                            (store::player_snapshot().volume * 100.0).round() as u32;
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
                                        .gap_2()
                                        .child(icon_button(
                                            "−",
                                            "Decrease Volume",
                                            false,
                                            || store::adjust_volume(-0.1),
                                        ))
                                        .child(icon_button(
                                            "+",
                                            "Increase Volume",
                                            false,
                                            || store::adjust_volume(0.1),
                                        )),
                                ),
                        )
                        // SponsorBlock Status Pill + Quick Links to Lyrics & Queue
                        .child(when(
                            || store::player_snapshot().sponsorblock_enabled,
                            || {
                                view()
                                    .px_3()
                                    .py_1p5()
                                    .rounded(radius_card)
                                    .bg(move || theme().card)
                                    .border_1()
                                    .border_color(color_sponsor_badge)
                                    .child(
                                        text("SponsorBlock active (auto-skip non-music segments)")
                                            .text_xs()
                                            .text_color(color_sponsor_badge),
                                    )
                            },
                        ))
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
                        )
                })
                .otherwise(|| {
                    status_card(
                        "Nothing is playing",
                        "Pick a song from Home or Search to begin playback.",
                        Some("Browse Home"),
                        Some(Box::new(|| store::set_tab(AppTab::Home))),
                    )
                }),
            ),
    )
}
