//! Priority Visual Surface #3: Persistent Queue.
//! Virtualized list with active-track highlight, jump-to-track, Move Up / Move Down /
//! Remove actions (>= 48dp touch targets), Autoplay Radio toggle, and Clear Queue.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{RepeatMode, format_duration_ms};

use crate::store;
use crate::ui::controls::{secondary_button, setting_switch, status_card};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_card_surface, glass_hairline, radius_card,
};

pub fn queue_screen() -> impl IntoElement {
    view()
        .flex_1()
        .p_4()
        .gap_3()
        .child(
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
                                        let total =
                                            format_duration_ms(store::total_queue_duration_ms());
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
                        .child(secondary_button("Clear Queue", store::clear_queue)),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(secondary_button(
                            move || {
                                if store::player_snapshot().queue.shuffle {
                                    SharedString::from("Shuffle: On")
                                } else {
                                    SharedString::from("Shuffle: Off")
                                }
                            },
                            store::toggle_shuffle,
                        ))
                        .child(secondary_button(
                            move || {
                                SharedString::from(match store::player_snapshot().queue.repeat {
                                    RepeatMode::None => "Repeat: Off",
                                    RepeatMode::All => "Repeat: All",
                                    RepeatMode::One => "Repeat: One",
                                })
                            },
                            store::cycle_repeat,
                        )),
                ),
        )
        .child(setting_switch(
            "Autoplay Radio",
            || store::player_snapshot().queue.autoplay_radio,
            store::set_autoplay_radio,
        ))
        .child(
            virtual_list(store::queue_len, move |at| {
                let track = computed(move || store::queue_track_at(at.index()));
                let is_active = move || {
                    store::player_snapshot().queue.current_index == Some(at.index())
                };

                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .min_h(px(64.0))
                    .rounded(radius_card)
                    .style_when(is_active, |s| s.bg(move || theme().accent))
                    .child(
                        button()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .justify_center()
                            .active(|s| s.opacity(0.75))
                            .on_press(move || store::jump_queue(at.index()))
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
                                    if is_active() {
                                        theme().primary
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
                        button()
                            .accessibility_label("Move Up")
                            .w(px(MOBILE_MIN_TOUCH_TARGET))
                            .h(px(MOBILE_MIN_TOUCH_TARGET))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .hover(|s| s.bg(move || theme().muted))
                            .on_press(move || {
                                let idx = at.index();
                                if idx > 0 {
                                    store::move_queue_item(idx, idx - 1);
                                }
                            })
                            .child(
                                text("↑")
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            ),
                    )
                    .child(
                        button()
                            .accessibility_label("Move Down")
                            .w(px(MOBILE_MIN_TOUCH_TARGET))
                            .h(px(MOBILE_MIN_TOUCH_TARGET))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .hover(|s| s.bg(move || theme().muted))
                            .on_press(move || {
                                let idx = at.index();
                                if idx + 1 < store::queue_len() {
                                    store::move_queue_item(idx, idx + 1);
                                }
                            })
                            .child(
                                text("↓")
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            ),
                    )
                    .child(
                        button()
                            .accessibility_label("Remove from Queue")
                            .w(px(MOBILE_MIN_TOUCH_TARGET))
                            .h(px(MOBILE_MIN_TOUCH_TARGET))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .hover(|s| s.bg(move || theme().muted))
                            .on_press(move || store::remove_queue_item(at.index()))
                            .child(
                                text("✕")
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().destructive),
                            ),
                    )
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
        )
}
