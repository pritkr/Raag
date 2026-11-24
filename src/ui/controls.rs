//! Styled reusable NSH UI controls enforcing:
//! - >= 48dp interactive touch targets on Android/mobile
//! - Proper single-line / two-line ellipsis (`min_w(0)` + `number_of_lines`)
//! - Crisp deterministic artwork fallback monogram (never a blank box)
//! - Semantic theme tokens only (no raw hex/RGB)

use nsh_base::{button, switch, theme};
use nsh_core::prelude::*;
use resonance_contract::{Track, format_duration_ms};

use crate::store;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, ambient_accent_color, color_control, color_control_fg,
    frosted_bar_surface, glass_hairline, radius_card,
};

/// Primary filled action button with >=48dp height on mobile and native press state.
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
        .hover(|s| s.opacity(0.92))
        .active(|s| s.opacity(0.8))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .disabled_style(|s| s.opacity(0.45))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().primary_foreground),
        )
}

/// Secondary subtle action button (>=48dp touch target).
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
        .bg(move || theme().secondary)
        .border_1()
        .border_color(move || theme().border)
        .hover(|s| s.bg(move || theme().muted))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().secondary_foreground),
        )
}

/// Accessible icon/glyph button strictly >= 48dp x 48dp to satisfy Mobile Rubric #1.
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
        .when(!emphasized, |b| b.bg(move || theme().secondary))
        .hover(|s| s.opacity(0.88))
        .active(|s| s.opacity(0.72))
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

/// Artwork tile with crisp monogram fallback so missing artwork NEVER renders as a blank box.
pub fn artwork_box(
    artwork_url: Option<String>,
    monogram: String,
    size_px: f32,
    radius_px: f32,
) -> impl IntoElement {
    let has_url = artwork_url.is_some();
    view()
        .w(px(size_px))
        .h(px(size_px))
        .rounded(px(radius_px))
        .bg(move || theme().accent)
        .border_1()
        .border_color(move || theme().border)
        .items_center()
        .justify_center()
        .overflow_hidden()
        .when_some(artwork_url, move |v, url| {
            v.child(
                image(url)
                    .size(px(size_px))
                    .object_fit(ResizeMode::Cover)
                    .rounded(px(radius_px)),
            )
        })
        .when(!has_url && !monogram.is_empty(), move |v| {
            v.child(
                text(monogram)
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .text_color(move || theme().accent_foreground),
            )
        })
}

/// Track list row with >=56dp height, ellipsis truncation, explicit badge, and queue action.
pub fn track_row_view(
    track: Track,
    is_current: bool,
    on_play: impl Fn() + 'static,
    on_play_next: impl Fn() + 'static,
) -> impl IntoElement {
    let title = SharedString::from(track.title.clone());
    let artist = SharedString::from(track.artist_display());
    let duration = SharedString::from(format_duration_ms(track.duration_ms));
    let monogram = track.fallback_monogram();
    let art_url = track.artwork_url.clone();

    view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .min_h(px(60.0))
        .rounded(radius_card)
        .when(is_current, |v| v.bg(move || theme().accent))
        .child(
            button()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                .flex_row()
                .items_center()
                .gap_3()
                .active(|s| s.opacity(0.75))
                .on_press(on_play)
                .child(artwork_box(art_url, monogram, 44.0, 8.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .min_w(px(0.0))
                                .child(
                                    text(title)
                                        .flex_1()
                                        .number_of_lines(1)
                                        .text_sm()
                                        .font_weight(if is_current {
                                            FontWeight::BOLD
                                        } else {
                                            FontWeight::MEDIUM
                                        })
                                        .text_color(move || {
                                            if is_current {
                                                theme().primary
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                )
                                .when(track.is_explicit, |v| {
                                    v.child(
                                        view()
                                            .px_1()
                                            .rounded(px(4.0))
                                            .bg(move || theme().muted)
                                            .child(
                                                text("E")
                                                    .text_xs()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(move || theme().muted_foreground),
                                            ),
                                    )
                                }),
                        )
                        .child(
                            text(artist)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    text(duration)
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            button()
                .accessibility_label("Play Next")
                .w(px(MOBILE_MIN_TOUCH_TARGET))
                .h(px(MOBILE_MIN_TOUCH_TARGET))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(move || theme().muted))
                .active(|s| s.opacity(0.7))
                .on_press(on_play_next)
                .child(
                    text("+")
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

/// Styled toggle switch with fully styled track & thumb parts (required by NSH rust-ui rules).
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
        .border_color(move || theme().border)
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
        .border_color(move || theme().border)
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
            v.child(primary_button(lbl, move || cb()))
        })
}

/// Subtle loading skeleton placeholder row (zero layout shift while Home or Search loads).
pub fn skeleton_row() -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .min_h(px(60.0))
        .rounded(radius_card)
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .opacity(0.65)
        .child(
            view()
                .w(px(44.0))
                .h(px(44.0))
                .rounded_md()
                .bg(move || theme().muted),
        )
        .child(
            view()
                .flex_1()
                .gap_2()
                .child(
                    view()
                        .w(percentage(68.0))
                        .h(px(12.0))
                        .rounded_full()
                        .bg(move || theme().muted),
                )
                .child(
                    view()
                        .w(percentage(42.0))
                        .h(px(10.0))
                        .rounded_full()
                        .bg(move || theme().muted),
                ),
        )
}

/// Persistent Mini-Player bar docked strictly above mobile bottom navigation (zero overlap).
/// Features native NSH frosted blur + calibrated translucent fallback, a 2px reactive hairline
/// progress bar, and an artwork-ambient monogram tile.
#[allow(clippy::too_many_arguments)]
pub fn mini_player_bar(
    title: impl Fn() -> SharedString + 'static,
    artist: impl Fn() -> SharedString + 'static,
    monogram: impl Fn() -> SharedString + 'static,
    state_label: impl Fn() -> SharedString + 'static,
    progress_pct: impl Fn() -> f32 + 'static,
    is_playing: impl Fn() -> bool + 'static,
    on_open_now_playing: impl Fn() + 'static,
    on_toggle_play: impl Fn() + 'static,
    on_next: impl Fn() + 'static,
) -> impl IntoElement {
    view()
        .backdrop_blur(px(16.0))
        .backdrop_material(BackdropMaterial::Regular)
        .bg(frosted_bar_surface)
        .border_t_1()
        .border_color(glass_hairline)
        // 2px hairline reactive playback progress track
        .child(
            view()
                .w_full()
                .h(px(2.0))
                .bg(move || theme().muted)
                .overflow_hidden()
                .child(
                    view()
                        .test_id("mini-player-progress")
                        .h_full()
                        .w(move || percentage(progress_pct().clamp(0.0, 100.0)))
                        .bg(move || theme().primary),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .min_h(px(64.0))
                .child(
                    button()
                        .test_id("mini-player-open")
                        .accessibility_label("Open Now Playing")
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .on_press(on_open_now_playing)
                        .child(
                            view()
                                .w(px(40.0))
                                .h(px(40.0))
                                .rounded_md()
                                .bg(move || ambient_accent_color(store::active_ambient_palette()))
                                .border_1()
                                .border_color(glass_hairline)
                                .items_center()
                                .justify_center()
                                .child(
                                    text(monogram)
                                        .test_id("mini-player-monogram")
                                        .text_base()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || theme().foreground),
                                ),
                        )
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap_0p5()
                                .child(
                                    text(title)
                                        .number_of_lines(1)
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(move || format!("{} • {}", artist(), state_label()))
                                        .number_of_lines(1)
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        ),
                )
                .child(icon_button(
                    move || SharedString::from(if is_playing() { "❚❚" } else { "▶" }),
                    "Toggle Play Pause",
                    true,
                    on_toggle_play,
                ))
                .child(icon_button("⏭", "Next Track", false, on_next)),
        )
}
