//! Atomic UI primitives: buttons, chips, artwork tiles, and switches.
//! Zero raw color literals — semantic theme tokens only.

use nsh_base::{alert_dialog, button, input, switch, theme};
use nsh_core::prelude::*;

use crate::store;
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_control, color_control_fg, color_destructive_foreground,
    color_hero_pill_bg, color_hero_pill_fg, color_on_cover, color_player_accent,
    cover_gradient_pair, glass_hairline, is_desktop_layout, motion, pill_glass_hover,
    pill_glass_surface, radius_card, radius_pill,
};

fn press_duration_ms() -> f32 {
    motion::transition_ms(motion::PRESS_MS, store::reduced_motion())
}

fn active_press_scale() -> f32 {
    if store::reduced_motion() {
        1.0
    } else {
        motion::PRESS_SCALE
    }
}

/// Primary filled action button with >=48dp height on mobile and tactile press feedback.
pub fn primary_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_5()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(move || theme().primary)
        .transition()
        .duration(press_duration_ms)
        .ease_out()
        .hover(|s| s.opacity(0.92))
        .active(|s| s.scale(active_press_scale).opacity(0.86))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().primary_foreground),
        )
}

/// High-contrast solid pill button (`Play` style).
pub fn hero_pill_button(
    label: impl IntoReactive<SharedString>,
    leading_icon: Option<IconKind>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(40.0))
        .style_when(
            || !is_desktop_layout(),
            |s| s.min_h(px(MOBILE_MIN_TOUCH_TARGET)),
        )
        .px_5()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(radius_pill)
        .bg(color_hero_pill_bg)
        .transition()
        .duration(press_duration_ms)
        .ease_out()
        .hover(|s| s.opacity(0.92))
        .active(|s| s.scale(active_press_scale).opacity(0.84))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .when_some(leading_icon, |b, kind| {
            b.child(icon(kind, 15.0, color_hero_pill_fg))
        })
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color_hero_pill_fg),
        )
}

/// Translucent glass pill button (`Shuffle`, `Radio`, `Sort`, etc.).
pub fn glass_pill_button(
    label: impl IntoReactive<SharedString>,
    leading_icon: Option<IconKind>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(40.0))
        .style_when(
            || !is_desktop_layout(),
            |s| s.min_h(px(MOBILE_MIN_TOUCH_TARGET)),
        )
        .px_4()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(radius_pill)
        .bg(pill_glass_surface)
        .border_1()
        .border_color(glass_hairline)
        .transition()
        .duration(press_duration_ms)
        .ease_out()
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.scale(active_press_scale).opacity(0.84))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .when_some(leading_icon, |b, kind| {
            b.child(icon(kind, 15.0, move || theme().foreground))
        })
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
}

/// Secondary subtle action button (>=48dp touch target for mobile accessibility).
pub fn secondary_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(pill_glass_surface)
        .border_1()
        .border_color(glass_hairline)
        .transition()
        .duration(press_duration_ms)
        .ease_out()
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.scale(active_press_scale).opacity(0.84))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
}

/// High-contrast destructive action button (>=48dp touch target, WCAG AA destructive-foreground).
pub fn destructive_button(
    label: impl IntoReactive<SharedString>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(move || theme().destructive)
        .transition()
        .duration(press_duration_ms)
        .ease_out()
        .hover(|s| s.opacity(0.92))
        .active(|s| s.scale(active_press_scale).opacity(0.84))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color_destructive_foreground),
        )
}

/// Non-blocking native confirmation dialog (`nsh_base::alert_dialog()`) for destructive actions.
/// Executes `on_confirm` ONLY if the user explicitly confirms; Cancel/dismiss is a safe no-op.
pub fn confirm_destructive(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    confirm_label: impl Into<SharedString>,
    on_confirm: impl FnOnce() + 'static,
) {
    let title: SharedString = title.into();
    let description: SharedString = description.into();
    let confirm_label: SharedString = confirm_label.into();
    let answer = alert_dialog()
        .title(title)
        .description(description)
        .cancel("cancel", "Cancel")
        .destructive("confirm", confirm_label)
        .show();
    spawn(async move {
        if answer
            .await
            .is_ok_and(|res| res.action.as_deref() == Some("confirm"))
        {
            on_confirm();
        }
    });
}

/// Non-color-only active state indicator dot (4px accent pip paired with icon/border changes).
pub fn active_state_dot(active: impl Fn() -> bool + 'static) -> impl IntoElement {
    when(active, || {
        view()
            .w(px(4.0))
            .h(px(4.0))
            .rounded_full()
            .bg(color_player_accent)
            .accessibility_elements_hidden(true)
    })
}

/// Accessible labeled text input with WCAG 1.4.11 `theme().input` border (`>= 3:1`),
/// focus ring, >=48dp touch height, and automatic global shortcut suppression on focus/blur.
pub fn labeled_input(
    label: &'static str,
    test_id: &'static str,
    placeholder: &'static str,
    value: impl Fn() -> String + 'static,
    on_change: impl Fn(&str) + 'static,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .gap_1p5()
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            input()
                .test_id(test_id)
                .value(value)
                .on_change(on_change)
                .on_focus(|| crate::ui::controls::set_text_input_focused(true))
                .on_blur(|| crate::ui::controls::set_text_input_focused(false))
                .placeholder(placeholder)
                .placeholder_color(|| color(theme().muted_foreground))
                .caret_color(|| color(theme().primary))
                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                .w_full()
                .px_3p5()
                .text_sm()
                .text_color(move || theme().foreground)
                .bg(pill_glass_surface)
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().input)
                .focus(|s| s.border_color(move || theme().ring)),
        )
}

/// Accessible labeled text input supporting an `on_submit` handler and `ReturnKeyType`.
pub fn labeled_input_with_submit(
    label: &'static str,
    test_id: &'static str,
    placeholder: &'static str,
    return_key: ReturnKeyType,
    value: impl Fn() -> String + 'static,
    on_change: impl Fn(&str) + 'static,
    on_submit: impl Fn(&str) + 'static,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(0.0))
        .gap_1p5()
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            input()
                .test_id(test_id)
                .value(value)
                .on_change(on_change)
                .on_submit(on_submit)
                .on_focus(|| crate::ui::controls::set_text_input_focused(true))
                .on_blur(|| crate::ui::controls::set_text_input_focused(false))
                .placeholder(placeholder)
                .placeholder_color(|| color(theme().muted_foreground))
                .caret_color(|| color(theme().primary))
                .return_key_type(return_key)
                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                .w_full()
                .px_3p5()
                .text_sm()
                .text_color(move || theme().foreground)
                .bg(pill_glass_surface)
                .rounded(move || theme().radius)
                .border_1()
                .border_color(move || theme().input)
                .focus(|s| s.border_color(move || theme().ring)),
        )
}

/// Icon-backed Back navigation button (`IconKind::ChevronLeft` + `"Back"`, >=48dp touch target).
pub fn back_button(on_press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .on_press(on_press)
        .accessibility_label("Back")
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_3p5()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(radius_pill)
        .bg(pill_glass_surface)
        .border_1()
        .border_color(glass_hairline)
        .transition()
        .duration(press_duration_ms)
        .ease_out()
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.scale(active_press_scale).opacity(0.84))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(IconKind::ChevronLeft, 16.0, move || theme().foreground))
        .child(
            text("Back")
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
}

/// Mood / category filter chip (>= 48dp touch height on mobile, 36dp on desktop, zero truncation).
pub fn mood_chip(
    label: impl IntoReactive<SharedString>,
    selected: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    mood_chip_with_icon(None, label, selected, on_press)
}

/// Icon-backed mood / filter chip (e.g. `IconKind::Close` + `"Clear"`).
pub fn mood_chip_with_icon(
    leading_icon: Option<IconKind>,
    label: impl IntoReactive<SharedString>,
    selected: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(36.0))
        .style_when(
            || !is_desktop_layout(),
            |s| s.min_h(px(MOBILE_MIN_TOUCH_TARGET)),
        )
        .px_4()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap_1p5()
        .rounded(move || theme().radius)
        .bg(move || {
            if selected() {
                color_hero_pill_bg()
            } else {
                pill_glass_surface()
            }
        })
        .border_1()
        .border_color(glass_hairline)
        .transition()
        .duration(press_duration_ms)
        .ease_out()
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.scale(active_press_scale).opacity(0.86))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .when_some(leading_icon, |b, kind| {
            b.child(icon(kind, 13.0, move || {
                if selected() {
                    color_hero_pill_fg()
                } else {
                    theme().foreground
                }
            }))
        })
        .child(
            text(label)
                .number_of_lines(1)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if selected() {
                        color_hero_pill_fg()
                    } else {
                        theme().foreground
                    }
                }),
        )
}

/// Accessible icon/glyph button strictly >= 48dp x 48dp.
pub fn icon_button(
    glyph: impl IntoReactive<SharedString>,
    a11y_label: &'static str,
    emphasized: bool,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .accessibility_label(a11y_label)
        .w(px(MOBILE_MIN_TOUCH_TARGET))
        .h(px(MOBILE_MIN_TOUCH_TARGET))
        .items_center()
        .justify_center()
        .rounded_full()
        .when(emphasized, |b| b.bg(color_control))
        .when(!emphasized, |b| b.bg(pill_glass_surface))
        .transition()
        .duration(press_duration_ms)
        .ease_out()
        .hover(|s| s.opacity(0.9))
        .active(|s| s.scale(active_press_scale).opacity(0.8))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(glyph)
                .text_base()
                .font_weight(FontWeight::BOLD)
                .text_color(move || {
                    if emphasized {
                        color_control_fg()
                    } else {
                        theme().foreground
                    }
                }),
        )
}

/// Crisp SVG icon button with optional subtle ghost background and tactile press feedback.
pub fn svg_icon_button(
    kind: IconKind,
    icon_px: f32,
    box_px: f32,
    a11y_label: &'static str,
    ghost: bool,
    color_fn: impl Fn() -> AdaptiveColor + Copy + 'static,
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
        .when(!ghost, |b| b.bg(pill_glass_surface))
        .transition()
        .duration(press_duration_ms)
        .ease_out()
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.scale(active_press_scale).opacity(0.8))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(kind, icon_px, color_fn))
}

/// Artwork tile with real image loading + rich editorial gradient & crisp monogram fallback.
pub fn artwork_box(
    artwork_url: Option<String>,
    monogram: String,
    size_px: f32,
    radius_px: f32,
) -> impl IntoElement {
    let resolved_url = crate::store::resolve_artwork_url(artwork_url);
    let has_url = resolved_url.is_some();
    let seed = format!("{monogram}-{}", resolved_url.as_deref().unwrap_or("mono"));
    let (grad_from, grad_to) = cover_gradient_pair(&seed);
    let large_mono = size_px >= 120.0;

    view()
        .w(px(size_px))
        .h(px(size_px))
        .rounded(px(radius_px))
        .bg(move || grad_from)
        .bg_linear_to_br()
        .gradient_from(move || grad_from)
        .gradient_to(move || grad_to)
        .border_1()
        .border_color(glass_hairline)
        .items_center()
        .justify_center()
        .overflow_hidden()
        .when(!has_url && !monogram.is_empty(), move |v| {
            v.child(
                text(monogram)
                    .when(large_mono, |t| t.text_4xl())
                    .when(!large_mono && size_px >= 64.0, |t| t.text_xl())
                    .when(size_px < 64.0, |t| t.text_sm())
                    .font_weight(FontWeight::BOLD)
                    .text_color(color_on_cover),
            )
        })
        .when_some(resolved_url, move |v, url| {
            v.child(
                image(url)
                    .size(px(size_px))
                    .object_fit(ResizeMode::Cover)
                    .rounded(px(radius_px)),
            )
        })
}

/// Styled toggle switch with fully styled track & thumb parts.
pub fn setting_switch(
    label: &'static str,
    checked: impl Fn() -> bool + 'static,
    on_change: impl Fn(bool) + 'static,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
        .py_2()
        .rounded(radius_card)
        .bg(move || theme().card)
        .border_1()
        .border_color(glass_hairline)
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
        .child(
            switch()
                .checked(checked)
                .on_change(move |v: &bool| on_change(*v))
                .accessibility_label(label)
                .track_class("w-11 h-6 rounded-full bg-muted p-0.5 checked:bg-primary")
                .thumb_class("size-5 rounded-full bg-background checked:translate-x-5"),
        )
}
