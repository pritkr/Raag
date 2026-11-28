//! Card controls: square media shelf cards, feedback status cards, and diagnostics badges.
//! Zero raw color literals — semantic theme tokens only.

use std::rc::Rc;

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use crate::store;
use crate::ui::controls::primitives::{artwork_box, primary_button};
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_hero_pill_bg, color_hero_pill_fg, color_sponsor_badge,
    color_status_ok, frosted_bar_surface, frosted_settings_card, glass_hairline, is_desktop_layout,
    motion, muted_caption_label, pill_glass_hover, pill_glass_surface, radius_card,
};

fn card_transition_ms() -> f32 {
    motion::transition_ms(motion::PRESS_MS, store::reduced_motion())
}

fn card_press_scale() -> f32 {
    if store::reduced_motion() {
        1.0
    } else {
        motion::PRESS_SCALE
    }
}

/// Square album / mix / release card for horizontal shelves.
/// Overlays top-right `⋯` menu and bottom-right floating Play pill on the artwork (hover/focus
/// revealed on desktop, 48dp touch targets on mobile).
#[allow(clippy::too_many_arguments)]
pub fn media_square_card(
    artwork_url: Option<String>,
    monogram: String,
    title: String,
    subtitle: String,
    card_width: f32,
    is_active: bool,
    on_open: impl Fn() + 'static,
    on_more: Option<Box<dyn Fn() + 'static>>,
) -> impl IntoElement {
    let art_size = card_width;
    let on_open_rc = Rc::new(on_open);
    let on_open_art = on_open_rc.clone();
    let on_open_pill = on_open_rc.clone();
    let on_open_meta = on_open_rc;

    view()
        .group()
        .w(px(card_width))
        .gap_2()
        .child(
            view()
                .relative()
                .w(px(art_size))
                .h(px(art_size))
                .rounded(radius_card)
                .overflow_hidden()
                .child(
                    button()
                        .accessibility_label(format!("Open {title}"))
                        .w(px(art_size))
                        .h(px(art_size))
                        .rounded(radius_card)
                        .overflow_hidden()
                        .transition()
                        .duration(card_transition_ms)
                        .ease_out()
                        .hover(|s| s.opacity(0.94))
                        .active(|s| s.scale(card_press_scale).opacity(0.88))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(move || on_open_art())
                        .child(artwork_box(artwork_url, monogram, art_size, 10.0)),
                )
                .when_some(on_more, |v, cb| {
                    v.child(
                        button()
                            .accessibility_label("Play Next")
                            .absolute()
                            .top_2()
                            .right_2()
                            .w(move || {
                                px(if is_desktop_layout() {
                                    36.0
                                } else {
                                    MOBILE_MIN_TOUCH_TARGET
                                })
                            })
                            .h(move || {
                                px(if is_desktop_layout() {
                                    36.0
                                } else {
                                    MOBILE_MIN_TOUCH_TARGET
                                })
                            })
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(frosted_bar_surface)
                            .border_1()
                            .border_color(glass_hairline)
                            .shadow_md()
                            .opacity(move || if is_desktop_layout() { 0.0_f32 } else { 0.94 })
                            .transition()
                            .duration(card_transition_ms)
                            .ease_out()
                            .group_hover(|s| s.opacity(1.0))
                            .group_focus_within(|s| s.opacity(1.0))
                            .hover(|s| s.bg(pill_glass_hover))
                            .active(|s| s.scale(card_press_scale))
                            .focus_visible(|s| {
                                s.opacity(1.0)
                                    .outline(px(2.0))
                                    .outline_color(move || theme().ring)
                            })
                            .on_press(cb)
                            .child(icon(IconKind::MoreVertical, 15.0, move || {
                                theme().foreground
                            })),
                    )
                })
                .child(
                    button()
                        .accessibility_label(if is_active {
                            "Pause Track"
                        } else {
                            "Play Track"
                        })
                        .absolute()
                        .bottom_2()
                        .right_2()
                        .w(move || {
                            px(if is_desktop_layout() {
                                38.0
                            } else {
                                MOBILE_MIN_TOUCH_TARGET
                            })
                        })
                        .h(move || {
                            px(if is_desktop_layout() {
                                38.0
                            } else {
                                MOBILE_MIN_TOUCH_TARGET
                            })
                        })
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(color_hero_pill_bg)
                        .shadow_lg()
                        .opacity(move || {
                            if is_active || !is_desktop_layout() {
                                1.0_f32
                            } else {
                                0.0_f32
                            }
                        })
                        .transition()
                        .duration(card_transition_ms)
                        .ease_out()
                        .group_hover(|s| s.opacity(1.0))
                        .group_focus_within(|s| s.opacity(1.0))
                        .hover(|s| s.opacity(0.94))
                        .active(|s| s.scale(card_press_scale))
                        .focus_visible(|s| {
                            s.opacity(1.0)
                                .outline(px(2.0))
                                .outline_color(move || theme().ring)
                        })
                        .on_press(move || on_open_pill())
                        .child(icon(
                            if is_active {
                                IconKind::Pause
                            } else {
                                IconKind::Play
                            },
                            15.0,
                            color_hero_pill_fg,
                        )),
                ),
        )
        .child(
            button()
                .w_full()
                .gap_0p5()
                .rounded(move || theme().radius)
                .transition()
                .duration(card_transition_ms)
                .ease_out()
                .hover(|s| s.opacity(0.9))
                .active(|s| s.opacity(0.78))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(move || on_open_meta())
                .child(
                    text(title)
                        .number_of_lines(2)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(muted_caption_label(subtitle)),
        )
}

/// Reusable empty/offline/error feedback panel.
pub fn status_card(
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    action_label: Option<&'static str>,
    on_action: Option<Box<dyn Fn() + 'static>>,
) -> impl IntoElement {
    view()
        .p_6()
        .gap_2()
        .items_center()
        .justify_center()
        .rounded(radius_card)
        .bg(frosted_settings_card)
        .border_1()
        .border_color(glass_hairline)
        .child(
            text(title)
                .text_base()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(subtitle)
                .text_sm()
                .text_color(move || theme().muted_foreground),
        )
        .when_some(action_label.zip(on_action), |v, (lbl, cb)| {
            v.child(primary_button(lbl, cb))
        })
}

/// Non-blocking diagnostics badge surfaced in Settings (PO Token health / degraded fallback).
pub fn diagnostics_badge(
    label: impl IntoReactive<SharedString>,
    healthy: impl Fn() -> bool + Copy + 'static,
    degraded: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    view()
        .px_2p5()
        .py_1()
        .rounded_full()
        .bg(pill_glass_surface)
        .border_1()
        .border_color(glass_hairline)
        .child(
            text(label)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if healthy() {
                        color_status_ok()
                    } else if degraded() {
                        color_sponsor_badge()
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}
