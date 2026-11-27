//! Priority Visual Surface #2: Synced Karaoke Lyrics.
//! Atmospheric artwork-driven backdrop, frosted header with sync source & step controls,
//! and virtualized list of large readable tap-to-seek lyric lines (zero backdrop blur in rows).
//! Uses semantic design tokens from `theme.json` and `crate::ui::tokens`.

use std::cell::Cell;
use std::rc::Rc;

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{LyricSyncKind, format_duration_ms};

use crate::store;
use crate::ui::controls::{
    ambient_shader_backdrop, secondary_button, status_card, svg_icon_button,
};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_lyric_active, color_lyric_inactive, color_player_accent,
    frosted_card_surface, glass_hairline, pill_glass_hover, pill_glass_surface, radius_card,
    radius_pill,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsScrollMode {
    /// Automatically centers active line at 38% viewport height on line change.
    AutoFollow,
    /// User manually scrolled; suppress auto-scroll until `resume_at_unix_ms` or tap "Sync".
    UserBrowsing { resume_at_unix_ms: u64 },
}

impl Default for LyricsScrollMode {
    fn default() -> Self {
        Self::AutoFollow
    }
}

pub const PROGRAMMATIC_SCROLL_GUARD_MS: u64 = 350;
pub const USER_BROWSING_HYSTERESIS_MS: u64 = 5_000;

/// Pure state-machine transition for synced lyrics auto-scroll hysteresis.
pub fn update_lyrics_scroll_mode(
    current: LyricsScrollMode,
    now_ms: u64,
    last_programmatic_scroll_ms: u64,
    user_scrolled: bool,
    sync_tapped: bool,
) -> LyricsScrollMode {
    if sync_tapped {
        return LyricsScrollMode::AutoFollow;
    }
    if user_scrolled {
        if now_ms.saturating_sub(last_programmatic_scroll_ms) > PROGRAMMATIC_SCROLL_GUARD_MS {
            return LyricsScrollMode::UserBrowsing {
                resume_at_unix_ms: now_ms.saturating_add(USER_BROWSING_HYSTERESIS_MS),
            };
        }
        return current;
    }
    match current {
        LyricsScrollMode::UserBrowsing { resume_at_unix_ms } if now_ms >= resume_at_unix_ms => {
            LyricsScrollMode::AutoFollow
        }
        other => other,
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn lyrics_screen() -> impl IntoElement {
    let has_lyrics = move || store::current_lyrics().is_some_and(|doc| !doc.lines.is_empty());
    let list_ref = element_ref();
    let scroll_mode = signal(LyricsScrollMode::AutoFollow);
    let last_scrolled_line = Rc::new(Cell::new(None::<usize>));
    let last_prog_scroll_ms = Rc::new(Cell::new(0u64));

    let list_ref_eff = list_ref.clone();
    let last_line_eff = last_scrolled_line.clone();
    let last_prog_eff = last_prog_scroll_ms.clone();
    effect(move || {
        let pos = store::player_snapshot().position_ms;
        let active_idx = store::current_lyrics().and_then(|d| d.active_line_index(pos));
        let now = now_unix_ms();
        let mode =
            update_lyrics_scroll_mode(scroll_mode.get(), now, last_prog_eff.get(), false, false);
        if mode != scroll_mode.get_untracked() {
            scroll_mode.set(mode);
        }
        if mode == LyricsScrollMode::AutoFollow
            && let Some(idx) = active_idx
            && last_line_eff.get() != Some(idx)
        {
            last_line_eff.set(Some(idx));
            last_prog_eff.set(now);
            list_ref_eff.scroll_to_index(
                idx as u32,
                percentage(0.38),
                px(0.0),
                !store::reduced_motion(),
            );
        }
    });

    view()
        .relative()
        .flex_1()
        .min_h(px(0.0))
        .overflow_hidden()
        .items_center()
        .p_4()
        .child(ambient_shader_backdrop())
        .child(
            view()
                .relative()
                .w_full()
                .max_w(px(860.0))
                .flex_1()
                .min_h(px(0.0))
                .gap_3()
                .child(lyrics_header(has_lyrics))
                .child(
                    when(has_lyrics, {
                        let last_prog_scroll_ms = last_prog_scroll_ms.clone();
                        move || {
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
                                let is_past = move || {
                                    let pos = store::player_snapshot().position_ms;
                                    store::current_lyrics()
                                        .and_then(|d| d.active_line_index(pos))
                                        .is_some_and(|active| at.index() < active)
                                };

                                // Performance rule: ZERO blur inside virtual_list rows.
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
                                    .style_when(is_past, |s| s.opacity(0.55))
                                    .style_when(
                                        move || !is_active() && !is_past(),
                                        |s| s.opacity(0.72),
                                    )
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .active(|s| s.opacity(0.85))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
                                    .on_press(move || {
                                        if let Some(l) = line.get_untracked()
                                            && l.end_ms > 0
                                        {
                                            scroll_mode.set(LyricsScrollMode::AutoFollow);
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
                                                dynamic(move || {
                                                    let l_opt = line.get();
                                                    let Some(l) = l_opt else {
                                                        return view().flex_1().into_element();
                                                    };
                                                    let is_word = store::current_lyrics()
                                                        .is_some_and(|d| {
                                                            d.sync_kind == LyricSyncKind::Word
                                                        })
                                                        && !l.words.is_empty();
                                                    if is_word {
                                                        let pos = store::player_snapshot()
                                                            .position_ms;
                                                        view()
                                                            .flex_1()
                                                            .flex_row()
                                                            .flex_wrap()
                                                            .gap_1p5()
                                                            .children(l.words.into_iter().map(
                                                                move |w| {
                                                                    let lit = pos >= w.start_ms;
                                                                    text(w.text)
                                                                        .text_xl()
                                                                        .font_weight(FontWeight::BOLD)
                                                                        .text_color(move || {
                                                                            if lit {
                                                                                color_lyric_active()
                                                                            } else {
                                                                                color_lyric_inactive()
                                                                            }
                                                                        })
                                                                },
                                                            ))
                                                            .into_element()
                                                    } else {
                                                        text(l.text)
                                                            .flex_1()
                                                            .text_xl()
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(move || {
                                                                if is_active() {
                                                                    color_lyric_active()
                                                                } else {
                                                                    color_lyric_inactive()
                                                                }
                                                            })
                                                            .into_element()
                                                    }
                                                }),
                                            ),
                                    )
                            },
                        )
                        .element_ref(&list_ref)
                        .on_scroll({
                            let last_prog = last_prog_scroll_ms.clone();
                            move |_: &ScrollEvent| {
                                let now = now_unix_ms();
                                let next = update_lyrics_scroll_mode(
                                    scroll_mode.get_untracked(),
                                    now,
                                    last_prog.get(),
                                    true,
                                    false,
                                );
                                if next != scroll_mode.get_untracked() {
                                    scroll_mode.set(next);
                                }
                            }
                        })
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
                        }
                    })
                    .otherwise(|| {
                        view().flex_1().justify_center().child(status_card(
                            "No lyrics available",
                            "Start playing a track to load synced karaoke lyrics from YTM, LRCLIB, or Better Lyrics.",
                            None,
                            None,
                        ))
                    }),
                )
                .child(when(
                    move || {
                        has_lyrics()
                            && matches!(scroll_mode.get(), LyricsScrollMode::UserBrowsing { .. })
                    },
                    move || {
                        let list_sync = list_ref.clone();
                        let last_prog_sync = last_prog_scroll_ms.clone();
                        view()
                            .w_full()
                            .items_center()
                            .pt_1()
                            .child(
                                button()
                                    .test_id("lyrics-sync-pill")
                                    .on_press(move || {
                                        scroll_mode.set(LyricsScrollMode::AutoFollow);
                                        let pos = store::player_snapshot().position_ms;
                                        if let Some(idx) = store::current_lyrics()
                                            .and_then(|d| d.active_line_index(pos))
                                        {
                                            last_prog_sync.set(now_unix_ms());
                                            list_sync.scroll_to_index(
                                                idx as u32,
                                                percentage(0.38),
                                                px(0.0),
                                                !store::reduced_motion(),
                                            );
                                        }
                                    })
                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .px_4()
                                    .items_center()
                                    .justify_center()
                                    .rounded(radius_pill)
                                    .bg(move || theme().primary)
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
                                    .child(
                                        text("Sync to current line")
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().primary_foreground),
                                    ),
                            )
                    },
                )),
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
                                        .map(|d| {
                                            if d.sync_kind == LyricSyncKind::Plain {
                                                format!("Lyrics • {}", d.provider)
                                            } else {
                                                format!("Synced • {}", d.provider)
                                            }
                                        })
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
                                    None if d.sync_kind == LyricSyncKind::Plain => {
                                        format!("{total} lines")
                                    }
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
                        .items_center()
                        .gap_2()
                        .child(
                            text("Seek")
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().muted_foreground),
                        )
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
