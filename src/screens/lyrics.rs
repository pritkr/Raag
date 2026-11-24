//! Priority Visual Surface #2: Synced Karaoke Lyrics.
//! Virtualized list of timed lines with high-contrast active-line highlight
//! and instant tap-to-seek (`store::seek_ms(line.start_ms)`).

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::format_duration_ms;

use crate::store;
use crate::ui::controls::{secondary_button, status_card};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, ambient_backdrop_color, color_lyric_active, color_lyric_inactive,
    frosted_card_surface, glass_hairline, radius_card,
};

pub fn lyrics_screen() -> impl IntoElement {
    let has_lyrics = move || store::current_lyrics().is_some_and(|doc| !doc.lines.is_empty());

    view()
        .flex_1()
        .bg(move || ambient_backdrop_color(store::active_ambient_palette()))
        .p_4()
        .gap_3()
        .child(
            view()
                .p_3p5()
                .gap_2p5()
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
                        .gap_2()
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
                        )
                        .child(
                            text(move || {
                                SharedString::from(
                                    store::current_lyrics()
                                        .map(|d| format!("Synced • {}", d.provider))
                                        .unwrap_or_default(),
                                )
                            })
                            .test_id("lyrics-source-badge")
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
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
                })),
        )
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
                            store::current_lyrics().and_then(|d| d.lines.get(at.index()).cloned())
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
                                s.bg(move || theme().accent)
                                    .border_1()
                                    .border_color(move || theme().primary)
                            })
                            .style_when(move || !is_active(), |s| s.opacity(0.68))
                            .hover(|s| s.bg(move || theme().muted))
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
                                    .gap_3()
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
                                        .text_color(move || theme().muted_foreground),
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
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
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
                .estimated_item_size(px(56.0))
                .flex_1()
            })
            .otherwise(|| {
                status_card(
                    "No lyrics available",
                    "Start playing a track to load synced karaoke lyrics from YTM, LRCLIB, or Better Lyrics.",
                    None,
                    None,
                )
            }),
        )
}
