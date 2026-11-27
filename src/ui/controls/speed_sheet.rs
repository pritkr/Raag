//! Playback Speed & Pitch Preservation control sheet (`0.50×` .. `2.00×` in `0.05×` steps).
//! Zero raw color literals — semantic theme tokens only.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::PlaybackSpeedConfig;

use crate::store;
use crate::ui::controls::primitives::setting_switch;
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_card_surface, glass_hairline, glass_specular_hairline, motion,
    overlay_scrim, pill_glass_hover, pill_glass_surface, radius_card, radius_panel, radius_pill,
};

fn speed_transition_ms() -> f32 {
    motion::transition_ms(motion::PRESS_MS, store::reduced_motion())
}

fn speed_press_scale() -> f32 {
    if store::reduced_motion() {
        1.0
    } else {
        motion::PRESS_SCALE
    }
}

pub fn quantize_speed(speed: f32) -> f32 {
    let clamped = speed.clamp(0.50, 2.00);
    (clamped * 20.0).round() / 20.0
}

pub fn open_speed_sheet() {
    store::open_speed_sheet();
}

pub fn close_speed_sheet() {
    store::close_speed_sheet();
}

pub fn is_speed_sheet_open() -> bool {
    store::is_speed_sheet_open()
}

pub fn current_speed_config() -> PlaybackSpeedConfig {
    store::playback_speed_config()
}

pub fn set_playback_speed(speed: f32) {
    let q = quantize_speed(speed);
    store::set_playback_speed(q);
    store::set_status_banner(format!("Playback speed: {q:.2}×"));
}

pub fn step_playback_speed(delta: f32) {
    let next = quantize_speed(current_speed_config().speed + delta);
    set_playback_speed(next);
}

pub fn set_preserve_pitch(preserve: bool) {
    store::set_preserve_pitch(preserve);
}

pub fn reset_playback_speed() {
    store::set_playback_speed(1.0);
    store::set_status_banner("Playback speed reset to 1.0×");
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
        .transition()
        .duration(speed_transition_ms)
        .ease_out()
        .hover(|s| s.opacity(0.92))
        .active(|s| s.scale(speed_press_scale).opacity(0.84))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
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
            .bg(overlay_scrim)
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
                    .backdrop_blur(px(28.0))
                    .backdrop_material(BackdropMaterial::Thick)
                    .bg(frosted_card_surface)
                    .border_1()
                    .border_color(glass_hairline)
                    .border_t_color(glass_specular_hairline)
                    .shadow_2xl()
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
                                    .transition()
                                    .duration(speed_transition_ms)
                                    .ease_out()
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .active(|s| s.scale(speed_press_scale))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
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
                                    .transition()
                                    .duration(speed_transition_ms)
                                    .ease_out()
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .active(|s| s.scale(speed_press_scale))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
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
                                    .transition()
                                    .duration(speed_transition_ms)
                                    .ease_out()
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .active(|s| s.scale(speed_press_scale))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
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
                            .transition()
                            .duration(speed_transition_ms)
                            .ease_out()
                            .hover(|s| s.bg(pill_glass_hover))
                            .active(|s| s.scale(speed_press_scale))
                            .focus_visible(|s| {
                                s.outline(px(2.0)).outline_color(move || theme().ring)
                            })
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
