//! Playback Speed & Pitch Preservation control sheet (`0.50×` .. `2.00×` in `0.05×` steps).
//! Zero raw color literals — semantic theme tokens only.

use std::cell::Cell;
use std::sync::{Mutex, OnceLock};

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::PlaybackSpeedConfig;

use crate::store;
use crate::ui::controls::primitives::setting_switch;
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_bar_surface, frosted_card_surface, glass_hairline,
    pill_glass_hover, pill_glass_surface, radius_card, radius_panel, radius_pill,
};

static SPEED_CONFIG: OnceLock<Mutex<PlaybackSpeedConfig>> = OnceLock::new();
static SPEED_SHEET_OPEN: OnceLock<Mutex<bool>> = OnceLock::new();

thread_local! {
    static SPEED_VERSION: Cell<Option<Signal<u64>>> = const { Cell::new(None) };
}

fn speed_lock() -> &'static Mutex<PlaybackSpeedConfig> {
    SPEED_CONFIG.get_or_init(|| Mutex::new(PlaybackSpeedConfig::default()))
}

fn sheet_open_lock() -> &'static Mutex<bool> {
    SPEED_SHEET_OPEN.get_or_init(|| Mutex::new(false))
}

fn bump_version() {
    SPEED_VERSION.with(|cell| {
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
    SPEED_VERSION.with(|cell| {
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

pub fn quantize_speed(speed: f32) -> f32 {
    let clamped = speed.clamp(0.50, 2.00);
    (clamped * 20.0).round() / 20.0
}

pub fn open_speed_sheet() {
    *sheet_open_lock().lock().unwrap() = true;
    bump_version();
}

pub fn close_speed_sheet() {
    *sheet_open_lock().lock().unwrap() = false;
    bump_version();
}

pub fn is_speed_sheet_open() -> bool {
    track_version();
    *sheet_open_lock().lock().unwrap()
}

pub fn current_speed_config() -> PlaybackSpeedConfig {
    track_version();
    *speed_lock().lock().unwrap()
}

pub fn set_playback_speed(speed: f32) {
    let q = quantize_speed(speed);
    {
        let mut cfg = speed_lock().lock().unwrap();
        cfg.speed = q;
    }
    store::set_status_banner(format!("Playback speed: {q:.2}×"));
    bump_version();
}

pub fn step_playback_speed(delta: f32) {
    let next = {
        let cfg = speed_lock().lock().unwrap();
        quantize_speed(cfg.speed + delta)
    };
    set_playback_speed(next);
}

pub fn set_preserve_pitch(preserve: bool) {
    {
        let mut cfg = speed_lock().lock().unwrap();
        cfg.preserve_pitch = preserve;
    }
    bump_version();
}

pub fn reset_playback_speed() {
    {
        let mut cfg = speed_lock().lock().unwrap();
        cfg.speed = 1.0;
    }
    store::set_status_banner("Playback speed reset to 1.0×");
    bump_version();
}

pub fn speed_badge_label() -> Option<String> {
    let cfg = current_speed_config();
    if (cfg.speed - 1.0).abs() > 0.01 {
        Some(format!("{:.2}×", cfg.speed).replace(".00×", ".0×"))
    } else {
        None
    }
}

fn speed_chip(val: f32, label: &'static str) -> impl IntoElement {
    let is_selected = move || (current_speed_config().speed - val).abs() < 0.025;
    button()
        .on_press(move || set_playback_speed(val))
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_3p5()
        .items_center()
        .justify_center()
        .rounded(radius_pill)
        .bg(move || {
            if is_selected() {
                theme().primary
            } else {
                pill_glass_surface()
            }
        })
        .border_1()
        .border_color(glass_hairline)
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.78))
        .child(
            text(label)
                .text_xs()
                .tabular_nums()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if is_selected() {
                        theme().primary_foreground
                    } else {
                        theme().foreground
                    }
                }),
        )
}

/// Modal Playback Speed & Pitch sheet (`test_id("speed-sheet")`).
pub fn speed_sheet() -> impl IntoElement {
    when(is_speed_sheet_open, || {
        view()
            .test_id("speed-sheet")
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
                    .gap_4()
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
                                    .child(icon(IconKind::Gauge, 18.0, move || theme().primary))
                                    .child(
                                        text("Playback Speed")
                                            .text_base()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                    ),
                            )
                            .child(
                                button()
                                    .accessibility_label("Close Playback Speed")
                                    .w(px(MOBILE_MIN_TOUCH_TARGET))
                                    .h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .on_press(close_speed_sheet)
                                    .child(icon(IconKind::Close, 16.0, move || theme().foreground)),
                            ),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .p_3()
                            .rounded(radius_card)
                            .bg(move || theme().card)
                            .border_1()
                            .border_color(glass_hairline)
                            .child(
                                button()
                                    .accessibility_label("Decrease Speed")
                                    .w(px(MOBILE_MIN_TOUCH_TARGET))
                                    .h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(pill_glass_surface)
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .on_press(|| step_playback_speed(-0.05))
                                    .child(
                                        text("−0.05")
                                            .text_xs()
                                            .tabular_nums()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                    ),
                            )
                            .child(
                                text(move || {
                                    SharedString::from(format!(
                                        "{:.2}×",
                                        current_speed_config().speed
                                    ))
                                })
                                .test_id("speed-readout")
                                .text_2xl()
                                .tabular_nums()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                            )
                            .child(
                                button()
                                    .accessibility_label("Increase Speed")
                                    .w(px(MOBILE_MIN_TOUCH_TARGET))
                                    .h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .bg(pill_glass_surface)
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .on_press(|| step_playback_speed(0.05))
                                    .child(
                                        text("+0.05")
                                            .text_xs()
                                            .tabular_nums()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                    ),
                            ),
                    )
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .child(speed_chip(0.75, "0.75×"))
                            .child(speed_chip(1.00, "1.0×"))
                            .child(speed_chip(1.25, "1.25×"))
                            .child(speed_chip(1.50, "1.5×"))
                            .child(speed_chip(2.00, "2.0×")),
                    )
                    .child(setting_switch(
                        "Preserve Pitch",
                        || current_speed_config().preserve_pitch,
                        set_preserve_pitch,
                    ))
                    .child(
                        button()
                            .on_press(reset_playback_speed)
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
                                text("Reset to 1.0×")
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || theme().foreground),
                            ),
                    ),
            )
    })
}
