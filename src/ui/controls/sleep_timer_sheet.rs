//! Sleep Timer sheet & reactive controller (`Off`, `15m`, `30m`, `45m`, `60m`, `End of track`, `+5m`).
//! Zero raw color literals — semantic theme tokens only.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::SleepTimerState;

use crate::store;
use crate::store::ui_slice::{SleepTimerMode, SleepTimerTickAction};
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_bar_surface, frosted_card_surface, glass_hairline,
    pill_glass_hover, pill_glass_surface, radius_card, radius_panel, radius_pill,
};

pub fn open_sleep_timer_sheet() {
    store::open_sleep_timer_sheet();
}

pub fn close_sleep_timer_sheet() {
    store::close_sleep_timer_sheet();
}

pub fn is_sleep_timer_sheet_open() -> bool {
    store::is_sleep_timer_sheet_open()
}

pub fn set_sleep_timer_minutes(minutes: u64) {
    let now_ms = resonance_core::unix_secs().saturating_mul(1000);
    store::set_sleep_timer_countdown_secs(minutes.saturating_mul(60), now_ms);
    if minutes == 0 {
        store::set_status_banner("Sleep timer turned off");
    } else {
        store::set_status_banner(format!("Sleep timer set for {minutes} min"));
    }
}

pub fn set_sleep_timer_end_of_track() {
    store::set_sleep_timer_end_of_track_mode();
    store::set_status_banner("Sleep timer: pause at end of track");
}

pub fn extend_sleep_timer_5m() {
    let now_ms = resonance_core::unix_secs().saturating_mul(1000);
    store::extend_sleep_timer_secs(300, now_ms);
    store::set_status_banner("Added 5 minutes to sleep timer");
}

pub fn cancel_sleep_timer() {
    if let Some(vol) = store::cancel_sleep_timer_mode() {
        store::set_volume(vol);
    }
    store::set_status_banner("Sleep timer cancelled");
}

pub fn sleep_timer_state() -> SleepTimerState {
    let now_ms = resonance_core::unix_secs().saturating_mul(1000);
    store::sleep_timer_state_at(now_ms)
}

pub fn sleep_timer_badge_label() -> Option<String> {
    let now_ms = resonance_core::unix_secs().saturating_mul(1000);
    store::sleep_timer_badge_at(now_ms)
}

pub fn tick_sleep_timer(now_unix_ms: u64) {
    match store::tick_sleep_timer_action(now_unix_ms) {
        SleepTimerTickAction::None => {}
        SleepTimerTickAction::FadeVolume(v) => store::set_volume(v),
        SleepTimerTickAction::PauseAndRestoreVolume(saved) => {
            if store::player_snapshot().state.is_playing() {
                store::toggle_play_pause();
            }
            store::set_volume(saved);
            store::set_status_banner("Sleep timer paused playback");
        }
    }
}

fn preset_row(
    label: &'static str,
    active: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_3p5()
        .flex_row()
        .items_center()
        .justify_between()
        .rounded(radius_card)
        .bg(move || {
            if active() {
                pill_glass_surface()
            } else {
                theme().card
            }
        })
        .border_1()
        .border_color(move || {
            if active() {
                theme().primary
            } else {
                glass_hairline()
            }
        })
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.8))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
        .child(when(active, || {
            icon(IconKind::Check, 16.0, move || theme().primary)
        }))
}

fn is_countdown_preset(mins: u64) -> bool {
    matches!(
        store::sleep_timer_mode(),
        SleepTimerMode::Countdown { duration_secs, .. } if duration_secs == mins * 60
    )
}

/// Modal Sleep Timer presets sheet (`test_id("sleep-timer-sheet")`).
pub fn sleep_timer_sheet() -> impl IntoElement {
    when(is_sleep_timer_sheet_open, || {
        view()
            .test_id("sleep-timer-sheet")
            .absolute()
            .inset_0()
            .bg(frosted_bar_surface)
            .items_center()
            .justify_center()
            .p_4()
            .child(
                view()
                    .w_full()
                    .max_w(px(380.0))
                    .p_4()
                    .gap_3()
                    .rounded(radius_panel)
                    .bg(frosted_card_surface)
                    .border_1()
                    .border_color(glass_hairline)
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(icon(IconKind::Moon, 18.0, move || theme().primary))
                                    .child(
                                        text("Sleep Timer")
                                            .text_base()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                    ),
                            )
                            .child(
                                button()
                                    .accessibility_label("Close Sleep Timer")
                                    .w(px(MOBILE_MIN_TOUCH_TARGET))
                                    .h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .on_press(close_sleep_timer_sheet)
                                    .child(icon(IconKind::Close, 16.0, move || theme().foreground)),
                            ),
                    )
                    .child(
                        view()
                            .gap_1p5()
                            .child(preset_row(
                                "Off",
                                || store::sleep_timer_mode() == SleepTimerMode::Off,
                                cancel_sleep_timer,
                            ))
                            .child(preset_row(
                                "15 minutes",
                                || is_countdown_preset(15),
                                || set_sleep_timer_minutes(15),
                            ))
                            .child(preset_row(
                                "30 minutes",
                                || is_countdown_preset(30),
                                || set_sleep_timer_minutes(30),
                            ))
                            .child(preset_row(
                                "45 minutes",
                                || is_countdown_preset(45),
                                || set_sleep_timer_minutes(45),
                            ))
                            .child(preset_row(
                                "60 minutes",
                                || is_countdown_preset(60),
                                || set_sleep_timer_minutes(60),
                            ))
                            .child(preset_row(
                                "End of current track",
                                || store::sleep_timer_mode() == SleepTimerMode::EndOfTrack,
                                set_sleep_timer_end_of_track,
                            )),
                    )
                    .child(when(
                        || matches!(store::sleep_timer_mode(), SleepTimerMode::Countdown { .. }),
                        || {
                            button()
                                .on_press(extend_sleep_timer_5m)
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_4()
                                .items_center()
                                .justify_center()
                                .rounded(radius_pill)
                                .bg(pill_glass_surface)
                                .border_1()
                                .border_color(glass_hairline)
                                .hover(|s| s.bg(pill_glass_hover))
                                .child(
                                    text("+5 minutes")
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().primary),
                                )
                        },
                    )),
            )
    })
}
