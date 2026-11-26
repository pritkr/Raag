//! Sleep Timer sheet & reactive controller (`Off`, `15m`, `30m`, `45m`, `60m`, `End of track`, `+5m`).
//! Zero raw color literals — semantic theme tokens only.

use std::cell::Cell;
use std::sync::{Mutex, OnceLock};

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::SleepTimerState;

use crate::store;
use crate::store::ui_slice::{SleepTimerController, SleepTimerMode, SleepTimerTickAction};
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_bar_surface, frosted_card_surface, glass_hairline,
    pill_glass_hover, pill_glass_surface, radius_card, radius_panel, radius_pill,
};

static SLEEP_CONTROLLER: OnceLock<Mutex<SleepTimerController>> = OnceLock::new();
static SLEEP_SHEET_OPEN: OnceLock<Mutex<bool>> = OnceLock::new();

thread_local! {
    static SLEEP_VERSION: Cell<Option<Signal<u64>>> = const { Cell::new(None) };
}

fn controller() -> &'static Mutex<SleepTimerController> {
    SLEEP_CONTROLLER.get_or_init(|| Mutex::new(SleepTimerController::default()))
}

fn sheet_open_lock() -> &'static Mutex<bool> {
    SLEEP_SHEET_OPEN.get_or_init(|| Mutex::new(false))
}

fn bump_version() {
    SLEEP_VERSION.with(|cell| {
        if let Some(sig) = cell.get() {
            if sig.try_get().is_some() {
                sig.update(|v| *v = v.wrapping_add(1));
            } else {
                cell.set(None);
            }
        }
    });
}

fn track_version() {
    SLEEP_VERSION.with(|cell| {
        let sig = match cell.get() {
            Some(s) if s.try_get().is_some() => s,
            _ => {
                let s = signal(0u64);
                cell.set(Some(s));
                s
            }
        };
        let _ = sig.get();
    });
}

pub fn open_sleep_timer_sheet() {
    *sheet_open_lock().lock().unwrap() = true;
    bump_version();
}

pub fn close_sleep_timer_sheet() {
    *sheet_open_lock().lock().unwrap() = false;
    bump_version();
}

pub fn is_sleep_timer_sheet_open() -> bool {
    track_version();
    *sheet_open_lock().lock().unwrap()
}

pub fn set_sleep_timer_minutes(minutes: u64) {
    let vol = store::player_snapshot().volume;
    let now_ms = resonance_core::unix_secs().saturating_mul(1000);
    controller()
        .lock()
        .unwrap()
        .set_countdown(minutes.saturating_mul(60), now_ms, vol);
    if minutes == 0 {
        store::set_status_banner("Sleep timer turned off");
    } else {
        store::set_status_banner(format!("Sleep timer set for {minutes} min"));
    }
    close_sleep_timer_sheet();
}

pub fn set_sleep_timer_end_of_track() {
    let vol = store::player_snapshot().volume;
    controller().lock().unwrap().set_end_of_track(vol);
    store::set_status_banner("Sleep timer: pause at end of track");
    close_sleep_timer_sheet();
}

pub fn extend_sleep_timer_5m() {
    let now_ms = resonance_core::unix_secs().saturating_mul(1000);
    controller().lock().unwrap().extend_minutes(300, now_ms);
    store::set_status_banner("Added 5 minutes to sleep timer");
    bump_version();
}

pub fn cancel_sleep_timer() {
    let restore = controller().lock().unwrap().cancel();
    if let Some(vol) = restore {
        store::set_volume(vol);
    }
    store::set_status_banner("Sleep timer cancelled");
    close_sleep_timer_sheet();
}

pub fn sleep_timer_state() -> SleepTimerState {
    track_version();
    let now_ms = resonance_core::unix_secs().saturating_mul(1000);
    controller().lock().unwrap().state(now_ms)
}

pub fn sleep_timer_badge_label() -> Option<String> {
    track_version();
    let now_ms = resonance_core::unix_secs().saturating_mul(1000);
    controller().lock().unwrap().badge_label(now_ms)
}

pub fn tick_sleep_timer(now_unix_ms: u64) {
    let action = controller().lock().unwrap().tick(now_unix_ms);
    match action {
        SleepTimerTickAction::None => {}
        SleepTimerTickAction::FadeVolume(v) => store::set_volume(v),
        SleepTimerTickAction::PauseAndRestoreVolume(saved) => {
            if store::player_snapshot().state.is_playing() {
                store::toggle_play_pause();
            }
            store::set_volume(saved);
            store::set_status_banner("Sleep timer paused playback");
            bump_version();
        }
    }
}

fn preset_row(
    label: &'static str,
    active: bool,
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
            if active {
                pill_glass_surface()
            } else {
                theme().card
            }
        })
        .border_1()
        .border_color(move || {
            if active {
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
        .when(active, |v| {
            v.child(icon(IconKind::Check, 16.0, move || theme().primary))
        })
}

/// Modal Sleep Timer presets sheet (`test_id("sleep-timer-sheet")`).
pub fn sleep_timer_sheet() -> impl IntoElement {
    when(is_sleep_timer_sheet_open, || {
        let mode = controller().lock().unwrap().mode;
        let is_off = mode == SleepTimerMode::Off;
        let is_eot = mode == SleepTimerMode::EndOfTrack;
        let is_countdown = matches!(mode, SleepTimerMode::Countdown { .. });

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
                            .child(preset_row("Off", is_off, cancel_sleep_timer))
                            .child(preset_row("15 minutes", false, || {
                                set_sleep_timer_minutes(15)
                            }))
                            .child(preset_row("30 minutes", false, || {
                                set_sleep_timer_minutes(30)
                            }))
                            .child(preset_row("45 minutes", false, || {
                                set_sleep_timer_minutes(45)
                            }))
                            .child(preset_row("60 minutes", false, || {
                                set_sleep_timer_minutes(60)
                            }))
                            .child(preset_row(
                                "End of current track",
                                is_eot,
                                set_sleep_timer_end_of_track,
                            )),
                    )
                    .when(is_countdown, |v| {
                        v.child(
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
                                ),
                        )
                    }),
            )
    })
}
