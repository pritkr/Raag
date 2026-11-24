//! Priority Visual Surface #2: Synced Karaoke Lyrics.
//! Atmospheric artwork-driven backdrop, frosted header with sync source & step controls,
//! and virtualized list of large readable tap-to-seek lyric lines (zero backdrop blur in rows).
//! Uses semantic design tokens from `theme.json` and `crate::ui::tokens`.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::format_duration_ms;

use crate::store;
use crate::ui::controls::{secondary_button, status_card, svg_icon_button};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, ambient_backdrop_color, color_lyric_active, color_lyric_inactive,
    color_player_accent, frosted_card_surface, glass_hairline, pill_glass_hover,
    pill_glass_surface, radius_card, radius_pill,
};

pub fn lyrics_screen() -> impl IntoElement {
    let has_lyrics = move || store::current_lyrics().is_some_and(|doc| !doc.lines.is_empty());

    view()
        .flex_1()
        .min_h(px(0.0))
        .bg(move || ambient_backdrop_color(store::active_ambient_palette()))
        .items_center()
        .p_4()
        .child(
            view()
                .w_full()
                .max_w(px(860.0))
                .flex_1()
                .min_h(px(0.0))
                .gap_3()
                .child(lyrics_header(has_lyrics))
                .child(
                    when(has_lyrics, || {
                        virtual_list(
                            || {
                                store::current_lyrics()
                                    .map(|d| d.lines.len())
                                    .unwrap_or(0)
                            },
                            move |at| {
                                let line = computed(move || {
                                    store::current_lyrics()
                                        .and_then(|d| d.lines.get(at.index()).cloned())
                                });
                                let is_active = move || {
                                    let pos = store::player_snapshot().position_ms;
                                    store::current_lyrics()
                                        .and_then(|d| d.active_line_index(pos))
                                        == Some(at.index())
                                };

                                // Performance rule: ZERO backdrop blur inside virtual_list rows.
                                button()
                                    .w_full()
                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .px_4()
                                    .py_3()
                                    .rounded(radius_card)
                                    .style_when(is_active, |s| {
                                        s.bg(pill_glass_surface)
                                            .border_1()
                                            .border_color(color_player_accent)
                                    })
                                    .style_when(move || !is_active(), |s| s.opacity(0.65))
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .active(|s| s.opacity(0.85))
                                    .on_press(move || {
                                        if let Some(l) = line.get_untracked()
                                            && l.end_ms > 0
                                        {
                                            store::seek_ms(l.start_ms);
                                        }
                                    })
                                    .child(
                                        view()
                                            .flex_row()
                                            .items_center()
                                            .gap_3p5()
                                            .child(
                                                text(move || {
                                                    line.with(|l| {
                                                        SharedString::from(match l.as_ref() {
                                                            Some(x) if x.end_ms > 0 => {
                                                                format_duration_ms(x.start_ms)
                                                            }
                                                            _ => "••:••".into(),
                                                        })
                                                    })
                                                })
                                                .text_xs()
                                                .tabular_nums()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || {
                                                    if is_active() {
                                                        color_player_accent()
                                                    } else {
                                                        theme().muted_foreground
                                                    }
                                                }),
                                            )
                                            .child(
                                                text(move || {
                                                    line.with(|l| {
                                                        SharedString::from(
                                                            l.as_ref()
                                                                .map(|x| x.text.clone())
                                                                .unwrap_or_default(),
                                                        )
                                                    })
                                                })
                                                .flex_1()
                                                .text_xl()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(move || {
                                                    if is_active() {
                                                        color_lyric_active()
                                                    } else {
                                                        color_lyric_inactive()
                                                    }
                                                }),
                                            ),
                                    )
                            },
                        )
                        .keys(|| {
                            store::current_lyrics()
                                .map(|d| {
                                    (0..d.lines.len())
                                        .map(|i| SharedString::from(format!("lyric-{i}")))
                                        .collect::<Vec<_>>()
                                })
                                .unwrap_or_default()
                        })
                        .estimated_item_size(px(60.0))
                        .flex_1()
                    })
                    .otherwise(|| {
                        view().flex_1().justify_center().child(status_card(
                            "No lyrics available",
                            "Start playing a track to load synced karaoke lyrics from YTM, LRCLIB, or Better Lyrics.",
                            None,
                            None,
                        ))
                    }),
                ),
        )
}

fn lyrics_header(has_lyrics: impl Fn() -> bool + Copy + 'static) -> impl IntoElement {
    view()
        .p_3p5()
        .gap_3()
        .rounded(radius_card)
        .backdrop_blur(px(14.0))
        .backdrop_material(BackdropMaterial::Thin)
        .bg(frosted_card_surface)
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .gap_2p5()
                        .child(svg_icon_button(
                            IconKind::ChevronDown,
                            18.0,
                            MOBILE_MIN_TOUCH_TARGET,
                            "Back",
                            true,
                            move || theme().foreground,
                            store::go_back,
                        ))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap_0p5()
                                .child(
                                    text("Lyrics")
                                        .text_2xl()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(move || {
                                        SharedString::from(
                                            store::player_snapshot()
                                                .current_track
                                                .map(|t| {
                                                    format!("{} — {}", t.title, t.artist_display())
                                                })
                                                .unwrap_or_else(|| "No active track".into()),
                                        )
                                    })
                                    .number_of_lines(1)
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                                ),
                        ),
                )
                .child(
                    view()
                        .px_3()
                        .py_1()
                        .rounded(radius_pill)
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .child(
                            text(move || {
                                SharedString::from(
                                    store::current_lyrics()
                                        .map(|d| format!("Synced • {}", d.provider))
                                        .unwrap_or_else(|| "No lyrics loaded".into()),
                                )
                            })
                            .test_id("lyrics-source-badge")
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
        .child(when(has_lyrics, || {
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(move || {
                        let pos = store::player_snapshot().position_ms;
                        let line_info = store::current_lyrics()
                            .map(|d| {
                                let total = d.lines.len();
                                match d.active_line_index(pos) {
                                    Some(i) => format!("Line {} / {}", i + 1, total),
                                    None => format!("{total} timed lines"),
                                }
                            })
                            .unwrap_or_default();
                        SharedString::from(line_info)
                    })
                    .test_id("lyrics-line-counter")
                    .text_xs()
                    .tabular_nums()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().muted_foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(secondary_button("-5s", || {
                            let pos = store::player_snapshot().position_ms;
                            store::seek_ms(pos.saturating_sub(5_000));
                        }))
                        .child(secondary_button("+5s", || {
                            let pos = store::player_snapshot().position_ms;
                            store::seek_ms(pos.saturating_add(5_000));
                        })),
                )
        }))
}
