//! Priority Visual Surface #3: Persistent Queue.
//! Frosted header with summary & shuffle/repeat controls, Autoplay Radio switch,
//! and virtualized track list with active highlight, jump-to-track, and >= 48dp
//! Move Up / Move Down / Remove actions using crisp SVG icons (zero backdrop blur in rows).
//! Uses semantic design tokens from `theme.json` and `crate::ui::tokens`.

use nsh_base::{button, theme};
use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;
use resonance_contract::{RepeatMode, format_duration_ms};

use crate::store;
use crate::ui::controls::{artwork_box, secondary_button, setting_switch, status_card};
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_player_accent, cover_gradient_pair, frosted_bar_surface,
    frosted_card_surface, glass_hairline, pill_glass_hover, pill_glass_surface, radius_card,
};

pub fn queue_screen() -> impl IntoElement {
    view().flex_1().min_h(px(0.0)).items_center().p_3().child(
        view()
            .w_full()
            .max_w(px(860.0))
            .flex_1()
            .min_h(px(0.0))
            .gap_2()
            .child(queue_header())
            .child(now_playing_pinned_card())
            .child(setting_switch(
                "AUTOPLAY RADIO",
                || store::player_snapshot().queue.autoplay_radio,
                store::set_autoplay_radio,
            ))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        text("UP NEXT")
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().muted_foreground),
                    )
                    .child(
                        text(move || SharedString::from(format!("{} tracks", store::queue_len())))
                            .text_xs()
                            .tabular_nums()
                            .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(
                virtual_list(store::queue_len, move |at| {
                    let track = computed(move || store::queue_track_at(at.index()));
                    let is_active =
                        move || store::player_snapshot().queue.current_index == Some(at.index());

                    // Performance rule: ZERO backdrop blur inside virtual_list rows.
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .min_h(px(64.0))
                        .rounded(radius_card)
                        .style_when(is_active, |s| {
                            s.bg(pill_glass_surface)
                                .border_1()
                                .border_color(color_player_accent)
                        })
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
                                .on_press(move || store::jump_queue(at.index()))
                                .child(queue_row_artwork(track, is_active))
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
                                                        if is_active() {
                                                            color_player_accent()
                                                        } else {
                                                            theme().foreground
                                                        }
                                                    }),
                                                )
                                                .child(when(
                                                    move || {
                                                        track.with(|t| {
                                                            t.as_ref()
                                                                .is_some_and(|x| x.is_explicit)
                                                        })
                                                    },
                                                    || {
                                                        view()
                                                            .px_1()
                                                            .rounded(px(3.0))
                                                            .bg(move || theme().muted)
                                                            .child(
                                                                text("E")
                                                                    .text_xs()
                                                                    .font_weight(FontWeight::BOLD)
                                                                    .text_color(move || {
                                                                        theme().muted_foreground
                                                                    }),
                                                            )
                                                    },
                                                )),
                                        )
                                        .child(
                                            text(move || {
                                                track.with(|t| {
                                                    SharedString::from(
                                                        t.as_ref()
                                                            .map(|x| {
                                                                format!(
                                                                    "{} • {}",
                                                                    x.artist_display(),
                                                                    format_duration_ms(
                                                                        x.duration_ms
                                                                    )
                                                                )
                                                            })
                                                            .unwrap_or_default(),
                                                    )
                                                })
                                            })
                                            .number_of_lines(1)
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                        ),
                                ),
                        )
                        .child(queue_action_button(
                            IconKind::ArrowUp,
                            "Move Up",
                            move || theme().foreground,
                            move || {
                                let idx = at.index();
                                if idx > 0 {
                                    store::move_queue_item(idx, idx - 1);
                                }
                            },
                        ))
                        .child(queue_action_button(
                            IconKind::ArrowDown,
                            "Move Down",
                            move || theme().foreground,
                            move || {
                                let idx = at.index();
                                if idx + 1 < store::queue_len() {
                                    store::move_queue_item(idx, idx + 1);
                                }
                            },
                        ))
                        .child(queue_action_button(
                            IconKind::Close,
                            "Remove from Queue",
                            move || theme().destructive,
                            move || store::remove_queue_item(at.index()),
                        ))
                })
                .keys(store::queue_ids)
                .estimated_item_size(px(64.0))
                .empty(status_card(
                    "Queue is empty",
                    "Play any song from Home or Search to populate your persistent queue.",
                    None,
                    None,
                ))
                .flex_1(),
            ),
    )
}

fn queue_header() -> impl IntoElement {
    let is_shuffle = move || store::player_snapshot().queue.shuffle;

    view()
        .p_3p5()
        .gap_3()
        .rounded(radius_card)
        .backdrop_blur(px(14.0))
        .backdrop_material(BackdropMaterial::Thin)
        .bg(frosted_card_surface)
        .border_1()
        .border_color(glass_hairline)
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
                        .gap_0p5()
                        .child(
                            text("Queue")
                                .text_2xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(move || {
                                let count = store::queue_len();
                                let total = format_duration_ms(store::total_queue_duration_ms());
                                SharedString::from(format!(
                                    "{count} tracks in queue • {total} total"
                                ))
                            })
                            .test_id("queue-summary")
                            .text_xs()
                            .tabular_nums()
                            .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(secondary_button("Save as Playlist", || {
                            if let Some(first) = store::queue_track_at(0) {
                                store::open_add_to_playlist_sheet(first);
                            } else {
                                store::set_status_banner("Queue is empty");
                            }
                        }))
                        .child(secondary_button("Clear Up Next", store::clear_queue)),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    button()
                        .on_press(store::toggle_shuffle)
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .px_4()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .rounded(move || theme().radius)
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.75))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .child(icon(IconKind::Shuffle, 15.0, move || {
                            if is_shuffle() {
                                color_player_accent()
                            } else {
                                theme().muted_foreground
                            }
                        }))
                        .child(
                            text(move || {
                                if is_shuffle() {
                                    SharedString::from("Shuffle: On")
                                } else {
                                    SharedString::from("Shuffle: Off")
                                }
                            })
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || {
                                if is_shuffle() {
                                    color_player_accent()
                                } else {
                                    theme().foreground
                                }
                            }),
                        ),
                )
                .child(
                    button()
                        .on_press(store::cycle_repeat)
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .px_4()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .rounded(move || theme().radius)
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.75))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .child(reactive_icon(
                            || match store::player_snapshot().queue.repeat {
                                RepeatMode::One => IconKind::RepeatOne,
                                _ => IconKind::Repeat,
                            },
                            15.0,
                            move || {
                                if store::player_snapshot().queue.repeat == RepeatMode::None {
                                    theme().muted_foreground
                                } else {
                                    color_player_accent()
                                }
                            },
                        ))
                        .child(
                            text(move || {
                                SharedString::from(match store::player_snapshot().queue.repeat {
                                    RepeatMode::None => "Repeat: Off",
                                    RepeatMode::All => "Repeat: All",
                                    RepeatMode::One => "Repeat: One",
                                })
                            })
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || {
                                if store::player_snapshot().queue.repeat == RepeatMode::None {
                                    theme().foreground
                                } else {
                                    color_player_accent()
                                }
                            }),
                        ),
                ),
        )
}

fn now_playing_pinned_card() -> impl IntoElement {
    when(
        || store::player_snapshot().current_track.is_some(),
        || {
            let snap = store::player_snapshot();
            let Some(cur) = snap.current_track else {
                return view().into_element();
            };
            let mono = cur.fallback_monogram();
            let sub = format!(
                "{} • {}",
                cur.artist_display(),
                format_duration_ms(cur.duration_ms)
            );
            let pct = if snap.duration_ms > 0 {
                (snap.position_ms as f32 / snap.duration_ms as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };

            view()
                .test_id("queue-now-playing-card")
                .p_3()
                .gap_2()
                .rounded(radius_card)
                .bg(move || theme().card)
                .border_1()
                .border_color(glass_hairline)
                .child(
                    text("NOW PLAYING")
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color_player_accent),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(artwork_box(cur.artwork_url, mono, 48.0, 8.0))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap_0p5()
                                .child(
                                    text(cur.title)
                                        .number_of_lines(1)
                                        .text_sm()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(sub)
                                        .number_of_lines(1)
                                        .text_xs()
                                        .tabular_nums()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        ),
                )
                .child(
                    view()
                        .w_full()
                        .h(px(2.0))
                        .rounded_full()
                        .bg(move || theme().muted)
                        .overflow_hidden()
                        .child(view().h_full().w(percentage(pct)).bg(color_player_accent)),
                )
                .into_element()
        },
    )
}

fn queue_row_artwork(
    track: Memo<Option<resonance_contract::Track>>,
    is_active: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let has_art = move || {
        track.with(|t| {
            t.as_ref()
                .and_then(|x| store::resolve_artwork_url(x.artwork_url.clone()))
                .is_some()
        })
    };

    view()
        .w(px(44.0))
        .h(px(44.0))
        .rounded(px(6.0))
        .bg_linear_to_br()
        .gradient_from(move || {
            let seed = track.with(|t| {
                t.as_ref()
                    .map(|x| x.id.clone())
                    .unwrap_or_else(|| "q".into())
            });
            cover_gradient_pair(&seed).0
        })
        .gradient_to(move || {
            let seed = track.with(|t| {
                t.as_ref()
                    .map(|x| x.id.clone())
                    .unwrap_or_else(|| "q".into())
            });
            cover_gradient_pair(&seed).1
        })
        .border_1()
        .border_color(glass_hairline)
        .items_center()
        .justify_center()
        .overflow_hidden()
        .child(
            when(has_art, move || {
                image(move || {
                    let raw = track.with(|t| t.as_ref().and_then(|x| x.artwork_url.clone()));
                    ImageSrc::from(store::resolve_artwork_url(raw).unwrap_or_default())
                })
                .size(px(44.0))
                .object_fit(ResizeMode::Cover)
                .rounded(px(6.0))
            })
            .otherwise(move || {
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
                .text_color(move || theme().primary_foreground)
            }),
        )
        .child(when(is_active, || {
            view()
                .absolute()
                .inset_0()
                .bg(frosted_bar_surface)
                .opacity(0.6)
                .items_center()
                .justify_center()
                .child(icon(IconKind::Play, 15.0, move || theme().foreground))
        }))
}

fn queue_action_button(
    kind: IconKind,
    a11y_label: &'static str,
    color_fn: impl Fn() -> AdaptiveColor + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .accessibility_label(a11y_label)
        .w(px(MOBILE_MIN_TOUCH_TARGET))
        .h(px(MOBILE_MIN_TOUCH_TARGET))
        .items_center()
        .justify_center()
        .rounded_full()
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.72))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .on_press(on_press)
        .child(icon(kind, 16.0, color_fn))
}
