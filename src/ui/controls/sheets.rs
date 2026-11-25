//! Reusable sheet & modal panel surfaces with semantic theme tokens and >= 48dp targets.

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_card_surface, glass_hairline, pill_glass_hover, radius_panel,
};

/// Frosted panel container for side drawers, queue/DSP sheets, and modal actions.
pub fn modal_panel_container(
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    content: impl IntoElement,
) -> impl IntoElement {
    view()
        .p_4()
        .gap_3()
        .rounded(radius_panel)
        .bg(frosted_card_surface)
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .gap_0p5()
                .child(
                    text(title)
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(subtitle)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(content)
}

/// Accessible >= 48dp action sheet row with leading icon and label.
pub fn action_sheet_row(
    leading: IconKind,
    label: impl IntoReactive<SharedString>,
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
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.8))
        .child(icon(leading, 16.0, move || theme().foreground))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
}
