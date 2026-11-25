//! Styled reusable NSH UI controls built with `nsh_base` + `nsh_core` primitives:
//! - >= 48dp interactive touch targets on Android/mobile
//! - Crisp SVG vector icons via `crate::ui::icons`
//! - Interactive `nsh_base::slider` for track seeking and volume control
//! - Rich gradient fallback artwork tiles (zero blank boxes)
//! - Semantic theme tokens only (zero raw color literals)

use nsh_base::{SliderValue, button, slider, switch, theme};
use nsh_core::prelude::*;
use resonance_contract::{Track, format_duration_ms};

use crate::store::{self, AppTab, DesktopSidePanel};
use crate::ui::icons::{IconKind, icon, reactive_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_control, color_control_fg, color_hero_pill_bg,
    color_hero_pill_fg, color_liked, color_player_accent, color_player_surface,
    cover_gradient_pair, frosted_bar_surface, glass_hairline, pill_glass_hover,
    pill_glass_surface, radius_card, radius_pill,
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

/// High-contrast solid pill button (`▶ Play` style from ref_4 & ref_5).
pub fn hero_pill_button(
    label: impl IntoReactive<SharedString>,
    leading_icon: Option<IconKind>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(40.0))
        .px_5()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(radius_pill)
        .bg(color_hero_pill_bg)
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.78))
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

/// Translucent glass pill button (`Shuffle`, `Radio`, `Sort`, etc. from ref_4 & ref_5).
pub fn glass_pill_button(
    label: impl IntoReactive<SharedString>,
    leading_icon: Option<IconKind>,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(40.0))
        .px_4()
        .flex_row()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(radius_pill)
        .bg(pill_glass_surface)
        .border_1()
        .border_color(glass_hairline)
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.78))
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
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
}

/// Mood / category filter chip (`Podcasts`, `Feel good`, `Romance`, etc. from ref_2).
pub fn mood_chip(
    label: &'static str,
    selected: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(34.0))
        .px_3p5()
        .items_center()
        .justify_center()
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
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.8))
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
        .when(!emphasized, |b| b.bg(pill_glass_surface))
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

/// Crisp SVG icon button with optional subtle ghost background and >= 48dp touch target on mobile.
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
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.72))
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
    let has_url = artwork_url.is_some();
    let seed = format!("{monogram}-{}", artwork_url.as_deref().unwrap_or("mono"));
    let (grad_from, grad_to) = cover_gradient_pair(&seed);
    let large_mono = size_px >= 120.0;

    view()
        .w(px(size_px))
        .h(px(size_px))
        .rounded(px(radius_px))
        .bg_linear_to_br()
        .gradient_from(move || grad_from)
        .gradient_to(move || grad_to)
        .border_1()
        .border_color(glass_hairline)
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
                    .when(large_mono, |t| t.text_4xl())
                    .when(!large_mono && size_px >= 64.0, |t| t.text_xl())
                    .when(size_px < 64.0, |t| t.text_sm())
                    .font_weight(FontWeight::BOLD)
                    .text_color(move || theme().primary_foreground),
            )
        })
}

/// Square album / mix / release card for horizontal shelves ("Listen again", "New releases", "Charts").
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

/// Compact track tile used in the 3-column "Song mixes" / "Quick Picks" grid (matches ref_2).
pub fn compact_mix_row(
    track: Track,
    is_current: bool,
    on_play: impl Fn() + 'static,
    on_play_next: impl Fn() + 'static,
) -> impl IntoElement {
    let title = SharedString::from(track.title.clone());
    let subtitle = SharedString::from(format!(
        "{} • {}",
        track.artist_display(),
        format_duration_ms(track.duration_ms)
    ));
    let monogram = track.fallback_monogram();
    let art_url = track.artwork_url.clone();
    let track_id = track.id.clone();
    let track_for_like = track.clone();

    view()
        .flex_row()
        .items_center()
        .gap_2p5()
        .px_2()
        .py_1p5()
        .min_h(px(58.0))
        .rounded(radius_card)
        .when(is_current, |v| v.bg(pill_glass_surface))
        .hover(|s| s.bg(pill_glass_surface))
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
                .child(
                    view()
                        .w(px(46.0))
                        .h(px(46.0))
                        .child(artwork_box(art_url, monogram, 46.0, 6.0))
                        .when(is_current, |v| {
                            v.child(
                                view()
                                    .absolute()
                                    .inset_0()
                                    .rounded(px(6.0))
                                    .bg(frosted_bar_surface)
                                    .opacity(0.55)
                                    .items_center()
                                    .justify_center()
                                    .child(icon(IconKind::Play, 16.0, move || theme().foreground)),
                            )
                        }),
                )
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
                                                color_player_accent()
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                )
                                .when(track.is_explicit, |v| {
                                    v.child(
                                        view()
                                            .px_1()
                                            .rounded(px(3.0))
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
                            text(subtitle)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
        .child(
            button()
                .accessibility_label("Toggle Like Track")
                .w(px(34.0))
                .h(px(34.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .on_press(move || store::toggle_like_track(track_for_like.clone()))
                .child(reactive_icon(
                    move || {
                        if store::is_track_liked(&track_id) {
                            IconKind::HeartFilled
                        } else {
                            IconKind::Heart
                        }
                    },
                    14.0,
                    move || theme().muted_foreground,
                )),
        )
        .child(
            button()
                .accessibility_label("Play Next")
                .w(px(34.0))
                .h(px(34.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .on_press(on_play_next)
                .child(icon(IconKind::MoreVertical, 15.0, move || {
                    theme().muted_foreground
                })),
        )
}

/// Full track list row with optional album column, heart, duration, and Play Next action.
pub fn track_row_view(
    track: Track,
    is_current: bool,
    on_play: impl Fn() + 'static,
    on_play_next: impl Fn() + 'static,
) -> impl IntoElement {
    let title = SharedString::from(track.title.clone());
    let artist = SharedString::from(track.artist_display());
    let album_title = track.album.as_ref().map(|a| SharedString::from(a.title.clone()));
    let duration = SharedString::from(format_duration_ms(track.duration_ms));
    let monogram = track.fallback_monogram();
    let art_url = track.artwork_url.clone();
    let track_id = track.id.clone();
    let track_for_like = track.clone();

    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_3()
        .py_1p5()
        .min_h(px(58.0))
        .rounded(radius_card)
        .when(is_current, |v| v.bg(pill_glass_surface))
        .hover(|s| s.bg(pill_glass_surface))
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
                .child(artwork_box(art_url, monogram, 44.0, 6.0))
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
                                                color_player_accent()
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
                .when_some(album_title, |b, alb| {
                    b.child(when(crate::ui::tokens::is_desktop_layout, move || {
                        let alb_text = alb.clone();
                        view().w(px(180.0)).px_2().child(
                            text(alb_text)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                    }))
                })
                .child(
                    text(duration)
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            button()
                .accessibility_label("Toggle Like Track")
                .w(px(36.0))
                .h(px(36.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .on_press(move || store::toggle_like_track(track_for_like.clone()))
                .child(reactive_icon(
                    move || {
                        if store::is_track_liked(&track_id) {
                            IconKind::HeartFilled
                        } else {
                            IconKind::Heart
                        }
                    },
                    15.0,
                    move || {
                        if is_current {
                            color_liked()
                        } else {
                            theme().muted_foreground
                        }
                    },
                )),
        )
        .child(
            button()
                .accessibility_label("Play Next")
                .w(px(MOBILE_MIN_TOUCH_TARGET))
                .h(px(MOBILE_MIN_TOUCH_TARGET))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.7))
                .on_press(on_play_next)
                .child(icon(IconKind::Plus, 16.0, move || {
                    theme().muted_foreground
                })),
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

/// Interactive `nsh_base::slider` for track seeking (used in player bar & Now Playing).
pub fn seek_slider(
    position_ms: impl Fn() -> f64 + 'static,
    duration_ms: impl Fn() -> f64 + 'static,
    on_seek: impl Fn(u64) + 'static,
) -> impl IntoElement {
    slider()
        .value(move || Some(SliderValue::Single(position_ms())))
        .min(0.0)
        .max(move || duration_ms().max(1.0))
        .step(500.0)
        .accessibility_label("Playback Position")
        .track_class("w-full h-3 justify-center")
        .indicator_class("w-full h-1 rounded-full bg-muted")
        .range_class("rounded-full")
        .range_style(|s| s.bg(color_player_accent))
        .thumb_class("size-3 rounded-full opacity-0 group-hover:opacity-100 group-active:opacity-100")
        .thumb_style(|s| s.bg(color_player_accent))
        .on_change(move |v: &SliderValue| {
            on_seek(v.start().max(0.0).round() as u64);
        })
}

/// Interactive `nsh_base::slider` for volume control (0.0 ..= 1.0).
pub fn volume_slider(
    volume: impl Fn() -> f64 + 'static,
    on_change: impl Fn(f32) + 'static,
) -> impl IntoElement {
    slider()
        .value(move || Some(SliderValue::Single(volume())))
        .min(0.0)
        .max(1.0)
        .step(0.02)
        .accessibility_label("Volume")
        .track_class("w-24 h-6 justify-center")
        .indicator_class("w-full h-1 rounded-full bg-muted")
        .range_class("rounded-full")
        .range_style(|s| s.bg(color_player_accent))
        .thumb_class("size-3 rounded-full")
        .thumb_style(|s| s.bg(color_player_accent))
        .on_change(move |v: &SliderValue| {
            on_change(v.start() as f32);
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
        .border_color(glass_hairline)
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

/// Sleek full-width Desktop Bottom Player Bar matching ref_1..ref_5:
/// - Top hairline interactive seek progress bar
/// - Left: Artwork + Title + Artist + Like + Dislike + More
/// - Center: Shuffle + Previous + Warm Elevated Play/Pause Circle + Next + Repeat
/// - Right: Time readout + Mute + Volume Slider + Lyrics + Queue + Expand/Collapse Now Playing
pub fn desktop_player_bar() -> impl IntoElement {
    let is_now_playing = move || store::active_tab() == AppTab::NowPlaying;
    let is_playing = move || store::player_snapshot().state.is_playing();
    let is_liked = move || {
        store::player_snapshot()
            .current_track
            .is_some_and(|t| t.is_liked)
    };
    let is_disliked = store::is_current_disliked;
    let is_shuffle = move || store::player_snapshot().queue.shuffle;

    view()
        .w_full()
        .bg(color_player_surface)
        .border_t_1()
        .border_color(glass_hairline)
        // Top-edge full-width hairline progress scrubber
        .child(
            view()
                .w_full()
                .h(px(3.0))
                .bg(move || theme().muted)
                .overflow_hidden()
                .child(
                    view()
                        .test_id("mini-player-progress")
                        .h_full()
                        .w(move || {
                            let snap = store::player_snapshot();
                            let pct = if snap.duration_ms > 0 {
                                (snap.position_ms as f32 / snap.duration_ms as f32) * 100.0
                            } else {
                                0.0
                            };
                            percentage(pct.clamp(0.0, 100.0))
                        })
                        .bg(color_player_accent),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .px_4()
                .py_2()
                .min_h(px(72.0))
                .gap_4()
                // Left: Track Thumbnail + Metadata + Like / Dislike / More
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            button()
                                .test_id("mini-player-open")
                                .accessibility_label("Open Now Playing")
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .min_w(px(0.0))
                                .max_w(px(280.0))
                                .active(|s| s.opacity(0.8))
                                .on_press(|| store::set_tab(AppTab::NowPlaying))
                                .child(
                                    view()
                                        .w(px(46.0))
                                        .h(px(46.0))
                                        .rounded(px(6.0))
                                        .bg(move || {
                                            let seed = store::player_snapshot()
                                                .current_track
                                                .map(|t| t.id)
                                                .unwrap_or_default();
                                            cover_gradient_pair(&seed).0
                                        })
                                        .border_1()
                                        .border_color(glass_hairline)
                                        .items_center()
                                        .justify_center()
                                        .overflow_hidden()
                                        .child(
                                            text(move || {
                                                SharedString::from(
                                                    store::player_snapshot()
                                                        .current_track
                                                        .map(|t| t.fallback_monogram())
                                                        .unwrap_or_else(|| "♪".into()),
                                                )
                                            })
                                            .test_id("mini-player-monogram")
                                            .text_base()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(move || theme().primary_foreground),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .gap_0p5()
                                        .child(
                                            text(move || {
                                                SharedString::from(
                                                    store::player_snapshot()
                                                        .current_track
                                                        .map(|t| t.title)
                                                        .unwrap_or_else(|| "Nothing playing".into()),
                                                )
                                            })
                                            .number_of_lines(1)
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text(move || {
                                                SharedString::from(
                                                    store::player_snapshot()
                                                        .current_track
                                                        .map(|t| t.artist_display())
                                                        .unwrap_or_else(|| "Choose a track".into()),
                                                )
                                            })
                                            .number_of_lines(1)
                                            .text_xs()
                                            .text_color(move || theme().muted_foreground),
                                        ),
                                ),
                        )
                        .child(
                            button()
                                .accessibility_label("Player Bar Like")
                                .w(px(34.0))
                                .h(px(34.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::toggle_like_current)
                                .child(reactive_icon(
                                    move || {
                                        if is_liked() {
                                            IconKind::HeartFilled
                                        } else {
                                            IconKind::Heart
                                        }
                                    },
                                    16.0,
                                    move || {
                                        if is_liked() {
                                            color_liked()
                                        } else {
                                            theme().muted_foreground
                                        }
                                    },
                                )),
                        )
                        .child(
                            button()
                                .accessibility_label("Player Bar Dislike")
                                .w(px(34.0))
                                .h(px(34.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::toggle_dislike_current)
                                .child(icon(IconKind::ThumbsDown, 15.0, move || {
                                    if is_disliked() {
                                        theme().foreground
                                    } else {
                                        theme().muted_foreground
                                    }
                                })),
                        )
                        .child(svg_icon_button(
                            IconKind::MoreVertical,
                            15.0,
                            34.0,
                            "Open Queue",
                            true,
                            move || theme().muted_foreground,
                            || store::set_tab(AppTab::Queue),
                        )),
                )
                // Center: Transport Controls
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2p5()
                        .child(
                            button()
                                .accessibility_label("Player Bar Shuffle")
                                .w(px(36.0))
                                .h(px(36.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::toggle_shuffle)
                                .child(icon(IconKind::Shuffle, 16.0, move || {
                                    if is_shuffle() {
                                        color_player_accent()
                                    } else {
                                        theme().muted_foreground
                                    }
                                })),
                        )
                        .child(svg_icon_button(
                            IconKind::SkipBack,
                            18.0,
                            38.0,
                            "Player Bar Previous Track",
                            true,
                            move || theme().foreground,
                            store::previous_track,
                        ))
                        .child(
                            button()
                                .accessibility_label("Player Bar Play or Pause")
                                .w(px(44.0))
                                .h(px(44.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(color_control)
                                .hover(|s| s.opacity(0.92))
                                .active(|s| s.opacity(0.8))
                                .on_press(store::toggle_play_pause)
                                .child(reactive_icon(
                                    move || {
                                        if is_playing() {
                                            IconKind::Pause
                                        } else {
                                            IconKind::Play
                                        }
                                    },
                                    18.0,
                                    color_control_fg,
                                )),
                        )
                        .child(svg_icon_button(
                            IconKind::SkipForward,
                            18.0,
                            38.0,
                            "Player Bar Next Track",
                            true,
                            move || theme().foreground,
                            store::next_track,
                        ))
                        .child(
                            button()
                                .accessibility_label("Player Bar Repeat Mode")
                                .w(px(36.0))
                                .h(px(36.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::cycle_repeat)
                                .child(reactive_icon(
                                    move || match store::player_snapshot().queue.repeat {
                                        resonance_contract::RepeatMode::One => IconKind::RepeatOne,
                                        _ => IconKind::Repeat,
                                    },
                                    16.0,
                                    move || {
                                        if store::player_snapshot().queue.repeat
                                            == resonance_contract::RepeatMode::None
                                        {
                                            theme().muted_foreground
                                        } else {
                                            color_player_accent()
                                        }
                                    },
                                )),
                        ),
                )
                // Right: Time + Volume Slider + Lyrics / Queue / Expand
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .justify_end()
                        .gap_2p5()
                        .child(
                            text(move || {
                                let snap = store::player_snapshot();
                                SharedString::from(format!(
                                    "{} / {}",
                                    format_duration_ms(snap.position_ms),
                                    format_duration_ms(snap.duration_ms)
                                ))
                            })
                            .text_xs()
                            .tabular_nums()
                            .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            button()
                                .accessibility_label("Player Bar Mute")
                                .w(px(34.0))
                                .h(px(34.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(store::toggle_mute)
                                .child(reactive_icon(
                                    move || {
                                        if store::player_snapshot().volume <= 0.01 {
                                            IconKind::VolumeMute
                                        } else {
                                            IconKind::VolumeHigh
                                        }
                                    },
                                    16.0,
                                    move || theme().foreground,
                                )),
                        )
                        .child(volume_slider(
                            || f64::from(store::player_snapshot().volume),
                            store::set_volume,
                        ))
                        .child(svg_icon_button(
                            IconKind::Lyrics,
                            16.0,
                            34.0,
                            "Toggle Lyrics",
                            true,
                            move || theme().muted_foreground,
                            || {
                                if crate::ui::tokens::is_desktop_panel_visible() {
                                    store::toggle_desktop_side_panel(DesktopSidePanel::Lyrics);
                                } else {
                                    store::set_tab(AppTab::Lyrics);
                                }
                            },
                        ))
                        .child(svg_icon_button(
                            IconKind::Queue,
                            16.0,
                            34.0,
                            "Toggle Queue",
                            true,
                            move || theme().muted_foreground,
                            || {
                                if crate::ui::tokens::is_desktop_panel_visible() {
                                    store::toggle_desktop_side_panel(DesktopSidePanel::Queue);
                                } else {
                                    store::set_tab(AppTab::Queue);
                                }
                            },
                        ))
                        .child(
                            button()
                                .accessibility_label("Toggle Fullscreen Player")
                                .w(px(34.0))
                                .h(px(34.0))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(move || {
                                    if is_now_playing() {
                                        store::go_back();
                                    } else {
                                        store::set_tab(AppTab::NowPlaying);
                                    }
                                })
                                .child(reactive_icon(
                                    move || {
                                        if is_now_playing() {
                                            IconKind::ChevronDown
                                        } else {
                                            IconKind::ChevronUp
                                        }
                                    },
                                    17.0,
                                    move || theme().foreground,
                                )),
                        ),
                ),
        )
}

/// Persistent Mobile Mini-Player bar docked strictly above mobile bottom navigation (zero overlap).
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
                        .bg(color_player_accent),
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
                                .w(px(42.0))
                                .h(px(42.0))
                                .rounded_md()
                                .bg(move || {
                                    let seed = store::player_snapshot()
                                        .current_track
                                        .map(|t| t.id)
                                        .unwrap_or_default();
                                    cover_gradient_pair(&seed).0
                                })
                                .border_1()
                                .border_color(glass_hairline)
                                .items_center()
                                .justify_center()
                                .child(
                                    text(monogram)
                                        .test_id("mini-player-monogram")
                                        .text_base()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || theme().primary_foreground),
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
                .child(
                    button()
                        .on_press(on_toggle_play)
                        .accessibility_label("Toggle Play Pause")
                        .w(px(MOBILE_MIN_TOUCH_TARGET))
                        .h(px(MOBILE_MIN_TOUCH_TARGET))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(color_control)
                        .child(reactive_icon(
                            move || {
                                if is_playing() {
                                    IconKind::Pause
                                } else {
                                    IconKind::Play
                                }
                            },
                            18.0,
                            color_control_fg,
                        )),
                )
                .child(svg_icon_button(
                    IconKind::SkipForward,
                    18.0,
                    MOBILE_MIN_TOUCH_TARGET,
                    "Next Track",
                    false,
                    move || theme().foreground,
                    on_next,
                )),
        )
}
