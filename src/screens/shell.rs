//! Adaptive Application Shell:
//! - Desktop (>= 820px): persistent left sidebar + main content + bottom mini-player bar
//! - Android / Mobile (< 820px): top safe-area header + main content + Mini-Player strictly
//!   stacked ABOVE the bottom navigation bar + `.pb_safe_area()` (zero overlap with gesture bar,
//!   notch, or bottom nav per Binary Visual Rubric #2).

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use crate::screens::{home, library, lyrics, now_playing, queue, search};
use crate::store::{self, AppTab, DesktopSidePanel};
use crate::ui::controls::mini_player_bar;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_bar_surface, glass_hairline, is_desktop_layout,
    is_desktop_panel_visible,
};

fn nav_button(tab: AppTab, label: &'static str, icon: &'static str, compact: bool) -> impl IntoElement {
    let selected = move || store::active_tab() == tab;
    button()
        .test_id(match tab {
            AppTab::Home => "nav-home",
            AppTab::Search => "nav-search",
            AppTab::Queue => "nav-queue",
            AppTab::NowPlaying => "nav-now-playing",
            AppTab::Lyrics => "nav-lyrics",
            AppTab::Library => "nav-library",
        })
        .accessibility_label(label)
        .on_press(move || store::set_tab(tab))
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .min_w(px(MOBILE_MIN_TOUCH_TARGET))
        .when(compact, |b| b.flex_1().items_center().justify_center().gap_0p5().py_1())
        .when(!compact, |b| {
            b.flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .rounded(move || theme().radius)
        })
        .style_when(selected, |s| s.bg(move || theme().accent))
        .hover(|s| s.bg(move || theme().muted))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(icon)
                .text_base()
                .font_weight(FontWeight::BOLD)
                .text_color(move || {
                    if selected() {
                        theme().primary
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
        .child(
            text(label)
                .number_of_lines(1)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if selected() {
                        theme().foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}

fn active_screen() -> impl IntoElement {
    view()
        .flex_1()
        .min_h(px(0.0))
        .child(when(move || store::active_tab() == AppTab::Home, home::home_screen))
        .child(when(move || store::active_tab() == AppTab::Search, search::search_screen))
        .child(when(move || store::active_tab() == AppTab::Queue, queue::queue_screen))
        .child(when(
            move || store::active_tab() == AppTab::NowPlaying,
            now_playing::now_playing_screen,
        ))
        .child(when(move || store::active_tab() == AppTab::Lyrics, lyrics::lyrics_screen))
        .child(when(move || store::active_tab() == AppTab::Library, library::library_screen))
}

fn status_toast() -> impl IntoElement {
    when(
        || store::status_banner().is_some(),
        || {
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .px_4()
                .py_2()
                .bg(move || theme().accent)
                .border_t_1()
                .border_color(move || theme().border)
                .child(
                    text(move || {
                        SharedString::from(store::status_banner().unwrap_or_default())
                    })
                    .test_id("global-status-toast")
                    .flex_1()
                    .number_of_lines(1)
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().accent_foreground),
                )
                .child(
                    button()
                        .test_id("dismiss-toast")
                        .accessibility_label("Dismiss notification")
                        .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .items_center()
                        .justify_center()
                        .rounded(move || theme().radius)
                        .on_press(store::dismiss_banner)
                        .child(
                            text("✕")
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().accent_foreground),
                        ),
                )
        },
    )
}

fn persistent_mini_player() -> impl IntoElement {
    let show = move || {
        let tab = store::active_tab();
        tab != AppTab::NowPlaying && store::player_snapshot().current_track.is_some()
    };

    when(show, || {
        mini_player_bar(
            || {
                SharedString::from(
                    store::player_snapshot()
                        .current_track
                        .map(|t| t.title)
                        .unwrap_or_else(|| "Nothing playing".into()),
                )
            },
            || {
                SharedString::from(
                    store::player_snapshot()
                        .current_track
                        .map(|t| t.artist_display())
                        .unwrap_or_default(),
                )
            },
            || {
                SharedString::from(
                    store::player_snapshot()
                        .current_track
                        .map(|t| t.fallback_monogram())
                        .unwrap_or_else(|| "♪".into()),
                )
            },
            || SharedString::from(store::player_snapshot().state.name()),
            || {
                let snap = store::player_snapshot();
                if snap.duration_ms > 0 {
                    (snap.position_ms as f32 / snap.duration_ms as f32) * 100.0
                } else {
                    0.0
                }
            },
            || store::player_snapshot().state.is_playing(),
            || store::set_tab(AppTab::NowPlaying),
            store::toggle_play_pause,
            store::next_track,
        )
    })
}

fn desktop_sidebar() -> impl IntoElement {
    view()
        .w(px(232.0))
        .h_full()
        .p_3()
        .gap_1()
        .bg(move || theme().card)
        .border_r_1()
        .border_color(move || theme().border)
        .child(
            view()
                .px_3()
                .py_3()
                .mb_2()
                .child(
                    text("Resonance")
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text("YouTube Music")
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(nav_button(AppTab::Home, "Home", "⌂", false))
        .child(nav_button(AppTab::Search, "Search", "⌕", false))
        .child(nav_button(AppTab::NowPlaying, "Now Playing", "♪", false))
        .child(nav_button(AppTab::Lyrics, "Lyrics", "≡", false))
        .child(nav_button(AppTab::Queue, "Queue", "☰", false))
        .child(nav_button(AppTab::Library, "Library", "★", false))
        .child(view().flex_1())
        .child(
            view()
                .px_3()
                .py_2()
                .rounded(move || theme().radius)
                .bg(move || theme().secondary)
                .child(
                    text("Shortcuts: Space · ←/→ · N/P · L · M")
                        .number_of_lines(1)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

fn desktop_side_panel() -> impl IntoElement {
    let show_panel = move || {
        is_desktop_panel_visible()
            && !matches!(store::active_tab(), AppTab::Queue | AppTab::Lyrics)
    };
    let is_queue = move || store::desktop_side_panel() == DesktopSidePanel::Queue;

    when(show_panel, move || {
        view()
            .test_id("desktop-side-panel")
            .w(px(300.0))
            .h_full()
            .p_3()
            .gap_3()
            .backdrop_blur(px(16.0))
            .backdrop_material(BackdropMaterial::Thin)
            .bg(frosted_bar_surface)
            .border_l_1()
            .border_color(glass_hairline)
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .child(
                        button()
                            .test_id("side-panel-tab-queue")
                            .flex_1()
                            .min_h(px(40.0))
                            .items_center()
                            .justify_center()
                            .rounded(move || theme().radius)
                            .bg(move || {
                                if is_queue() {
                                    theme().primary
                                } else {
                                    theme().secondary
                                }
                            })
                            .on_press(|| store::set_desktop_side_panel(DesktopSidePanel::Queue))
                            .child(
                                text("Up Next")
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || {
                                        if is_queue() {
                                            theme().primary_foreground
                                        } else {
                                            theme().secondary_foreground
                                        }
                                    }),
                            ),
                    )
                    .child(
                        button()
                            .test_id("side-panel-tab-lyrics")
                            .flex_1()
                            .min_h(px(40.0))
                            .items_center()
                            .justify_center()
                            .rounded(move || theme().radius)
                            .bg(move || {
                                if !is_queue() {
                                    theme().primary
                                } else {
                                    theme().secondary
                                }
                            })
                            .on_press(|| store::set_desktop_side_panel(DesktopSidePanel::Lyrics))
                            .child(
                                text("Synced Lyrics")
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || {
                                        if !is_queue() {
                                            theme().primary_foreground
                                        } else {
                                            theme().secondary_foreground
                                        }
                                    }),
                            ),
                    ),
            )
            .child(
                when(is_queue, || {
                    virtual_list(store::queue_len, move |at| {
                        let track = computed(move || store::queue_track_at(at.index()));
                        let active = move || {
                            store::player_snapshot().queue.current_index == Some(at.index())
                        };
                        button()
                            .w_full()
                            .min_h(px(48.0))
                            .px_3()
                            .py_2()
                            .rounded(move || theme().radius)
                            .style_when(active, |s| s.bg(move || theme().accent))
                            .hover(|s| s.bg(move || theme().muted))
                            .on_press(move || store::jump_queue(at.index()))
                            .child(
                                text(move || {
                                    track.with(|t| {
                                        SharedString::from(
                                            t.as_ref()
                                                .map(|x| x.title.clone())
                                                .unwrap_or_default(),
                                        )
                                    })
                                })
                                .number_of_lines(1)
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || {
                                    if active() {
                                        theme().primary
                                    } else {
                                        theme().foreground
                                    }
                                }),
                            )
                            .child(
                                text(move || {
                                    track.with(|t| {
                                        SharedString::from(
                                            t.as_ref()
                                                .map(|x| x.artist_display())
                                                .unwrap_or_default(),
                                        )
                                    })
                                })
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                            )
                    })
                    .keys(store::queue_ids)
                    .estimated_item_size(px(52.0))
                    .flex_1()
                })
                .otherwise(|| {
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
                            let active = move || {
                                let pos = store::player_snapshot().position_ms;
                                store::current_lyrics().and_then(|d| d.active_line_index(pos))
                                    == Some(at.index())
                            };
                            button()
                                .w_full()
                                .min_h(px(40.0))
                                .px_3()
                                .py_1p5()
                                .rounded(move || theme().radius)
                                .style_when(active, |s| s.bg(move || theme().accent))
                                .style_when(move || !active(), |s| s.opacity(0.68))
                                .on_press(move || {
                                    if let Some(l) = line.get_untracked()
                                        && l.end_ms > 0
                                    {
                                        store::seek_ms(l.start_ms);
                                    }
                                })
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
                        },
                    )
                    .keys(|| {
                        store::current_lyrics()
                            .map(|d| {
                                (0..d.lines.len())
                                    .map(|i| SharedString::from(format!("side-lyric-{i}")))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    })
                    .estimated_item_size(px(44.0))
                    .flex_1()
                }),
            )
    })
}

fn mobile_bottom_nav() -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_around()
        .px_2()
        .pt_1()
        .pb_safe_area_plus(px(4.0))
        .backdrop_blur(px(18.0))
        .backdrop_material(BackdropMaterial::Chrome)
        .bg(frosted_bar_surface)
        .border_t_1()
        .border_color(glass_hairline)
        .child(nav_button(AppTab::Home, "Home", "⌂", true))
        .child(nav_button(AppTab::Search, "Search", "⌕", true))
        .child(nav_button(AppTab::Lyrics, "Lyrics", "≡", true))
        .child(nav_button(AppTab::Queue, "Queue", "☰", true))
        .child(nav_button(AppTab::Library, "Library", "★", true))
}

pub fn app_shell() -> impl IntoElement {
    let desktop = move || is_desktop_layout();

    view()
        .flex_1()
        .bg(move || theme().background)
        .child(
            when(desktop, || {
                view()
                    .flex_1()
                    .flex_row()
                    .child(desktop_sidebar())
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(active_screen())
                            .child(status_toast())
                            .child(persistent_mini_player()),
                    )
                    .child(desktop_side_panel())
            })
            .otherwise(|| {
                view()
                    .flex_1()
                    .pt_safe_area()
                    .child(active_screen())
                    .child(status_toast())
                    .child(persistent_mini_player())
                    .child(mobile_bottom_nav())
            }),
        )
}
