//! Reusable navigation primitives (nav pills and section headers).
//! Zero raw color literals — semantic theme tokens only.

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{MOBILE_MIN_TOUCH_TARGET, pill_glass_hover, sidebar_active_surface};

/// Accessible navigation pill button (>= 48dp touch height).
pub fn nav_pill_button(
    kind: IconKind,
    label: &'static str,
    active: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_3()
        .flex_row()
        .items_center()
        .gap_3()
        .rounded(move || theme().radius)
        .bg(move || {
            if active() {
                sidebar_active_surface()
            } else {
                theme().background
            }
        })
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.82))
        .child(icon(kind, 18.0, move || {
            if active() {
                theme().foreground
            } else {
                theme().muted_foreground
            }
        }))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active() {
                        theme().foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}

/// Consistent shelf / section header row with title and optional trailing subtitle.
pub fn section_header_row(
    title: impl IntoReactive<SharedString>,
    trailing: impl IntoReactive<SharedString>,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            text(title)
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(trailing)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}
