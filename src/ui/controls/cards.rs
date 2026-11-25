//! Card controls: square media shelf cards, feedback status cards, and diagnostics badges.
//! Zero raw color literals — semantic theme tokens only.

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use crate::ui::controls::primitives::{artwork_box, primary_button};
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    color_hero_pill_bg, color_hero_pill_fg, color_sponsor_badge, color_status_ok, glass_hairline,
    pill_glass_hover, pill_glass_surface, radius_card,
};

/// Square album / mix / release card for horizontal shelves.
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

    view()
        .w(px(card_width))
        .gap_1p5()
        .child(
            button()
                .w(px(card_width))
                .gap_2()
                .active(|s| s.opacity(0.82))
                .hover(|s| s.opacity(0.94))
                .on_press(on_open)
                .child(
                    view()
                        .w(px(art_size))
                        .h(px(art_size))
                        .rounded(radius_card)
                        .overflow_hidden()
                        .child(artwork_box(artwork_url, monogram, art_size, 10.0))
                        .when(is_active, |b| {
                            b.child(
                                view()
                                    .absolute()
                                    .bottom_2()
                                    .right_2()
                                    .w(px(34.0))
                                    .h(px(34.0))
                                    .rounded_full()
                                    .bg(color_hero_pill_bg)
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconKind::Pause, 14.0, color_hero_pill_fg)),
                            )
                        }),
                )
                .child(
                    view()
                        .w_full()
                        .gap_0p5()
                        .child(
                            text(title)
                                .number_of_lines(1)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(subtitle)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
        .when_some(on_more, |v, cb| {
            v.child(
                view().w_full().flex_row().justify_end().child(
                    button()
                        .accessibility_label("Play Next")
                        .w(px(28.0))
                        .h(px(24.0))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .hover(|s| s.bg(pill_glass_hover))
                        .on_press(cb)
                        .child(icon(IconKind::MoreVertical, 14.0, move || {
                            theme().muted_foreground
                        })),
                ),
            )
        })
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
        .bg(move || theme().card)
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
