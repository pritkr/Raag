//! Adaptive Application Shell for Raag:
//! - Desktop (>= 820px): Collapsible left sidebar (brand, primary nav, scrollable playlists,
//!   settings & shortcut legend) + warm-ambient main canvas with top nav/search/avatar header +
//!   optional frosted right Queue/Lyrics panel + full-width bottom player bar.
//! - Android / Mobile (< 820px): Top safe-area header + main content + Mini-Player strictly
//!   stacked ABOVE the bottom navigation bar + `.pb_safe_area_plus(px(4.0))` (>= 48dp targets).

use std::rc::Rc;

use nsh_base::{avatar, button, input, tab, tabs, theme};
use nsh_core::prelude::*;

use crate::screens::{
    auth_sign_in_sheet::{AuthSignInCallbacks, auth_sign_in_sheet},
    explore, home, library, lyrics, now_playing, queue, search, settings,
    settings_updater_card::updater_top_banner,
};
use crate::store::{self, AppTab, DesktopSidePanel, LibraryFilter, SidebarPlaylist};
use crate::ui::controls::{
    add_to_playlist_sheet, artwork_box, cast_picker_sheet, desktop_player_bar, mini_player_bar,
    svg_icon_button, track_action_sheet,
};
use crate::ui::icons::{IconKind, icon, liked_music_tile_svg, yt_music_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_ambient_warm, color_sidebar_border, color_sidebar_surface,
    frosted_bar_surface, glass_hairline, is_desktop_layout, is_desktop_panel_visible,
    pill_glass_hover, pill_glass_surface, radius_card, radius_pill, sidebar_active_surface,
};

fn nav_test_id(tab: AppTab) -> &'static str {
    match tab {
        AppTab::Home => "nav-home",
        AppTab::Explore => "nav-explore",
        AppTab::Library => "nav-library",
        AppTab::ListenTogether => "nav-listen-together",
        AppTab::NowPlaying => "nav-now-playing",
        AppTab::Lyrics => "nav-lyrics",
        AppTab::Queue => "nav-queue",
        AppTab::Search => "nav-search",
        AppTab::Settings => "nav-settings",
    }
}

fn desktop_nav_item(
    tab_kind: AppTab,
    label: &'static str,
    icon_kind: IconKind,
) -> impl IntoElement {
    let selected = move || store::active_tab() == tab_kind;
    let collapsed = store::sidebar_collapsed;

    button()
        .test_id(nav_test_id(tab_kind))
        .accessibility_label(label)
        .on_press(move || store::set_tab(tab_kind))
        .w_full()
        .min_h(px(40.0))
        .flex_row()
        .items_center()
        .style_when(collapsed, |s| s.justify_center().px_2())
        .style_when(move || !collapsed(), |s| s.gap_3().px_3())
        .rounded(radius_card)
        .style_when(selected, |s| s.bg(sidebar_active_surface))
        .hover(|s| s.bg(pill_glass_surface))
        .active(|s| s.opacity(0.76))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(icon_kind, 18.0, move || {
            if selected() {
                theme().foreground
            } else {
                theme().muted_foreground
            }
        }))
        .child(when(
            move || !collapsed(),
            move || {
                text(label)
                    .flex_1()
                    .number_of_lines(1)
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .style_when(selected, |s| s.font_weight(FontWeight::SEMIBOLD))
                    .text_color(move || {
                        if selected() {
                            theme().foreground
                        } else {
                            theme().muted_foreground
                        }
                    })
            },
        ))
}

fn mobile_nav_item(tab_kind: AppTab, label: &'static str, icon_kind: IconKind) -> impl IntoElement {
    let selected = move || store::active_tab() == tab_kind;

    button()
        .test_id(nav_test_id(tab_kind))
        .accessibility_label(label)
        .on_press(move || store::set_tab(tab_kind))
        .flex_1()
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .min_w(px(MOBILE_MIN_TOUCH_TARGET))
        .items_center()
        .justify_center()
        .gap_0p5()
        .py_1()
        .rounded(radius_card)
        .style_when(selected, |s| s.bg(sidebar_active_surface))
        .hover(|s| s.bg(pill_glass_surface))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(icon_kind, 20.0, move || {
            if selected() {
                theme().primary
            } else {
                theme().muted_foreground
            }
        }))
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
        .child(when(
            move || store::active_tab() == AppTab::Home,
            home::home_screen,
        ))
        .child(when(
            move || store::active_tab() == AppTab::Explore,
            explore::explore_screen,
        ))
        .child(when(
            move || store::active_tab() == AppTab::Search,
            search::search_screen,
        ))
        .child(when(
            move || store::active_tab() == AppTab::Queue,
            queue::queue_screen,
        ))
        .child(when(
            move || store::active_tab() == AppTab::NowPlaying,
            now_playing::now_playing_screen,
        ))
        .child(when(
            move || store::active_tab() == AppTab::Lyrics,
            lyrics::lyrics_screen,
        ))
        .child(when(
            move || store::active_tab() == AppTab::Library,
            library::library_screen,
        ))
        .child(when(
            move || store::active_tab() == AppTab::ListenTogether,
            settings::listen_together_screen,
        ))
        .child(when(
            move || store::active_tab() == AppTab::Settings,
            settings::settings_screen,
        ))
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
                .py_1p5()
                .bg(move || theme().accent)
                .border_t_1()
                .border_color(move || theme().border)
                .child(
                    text(move || SharedString::from(store::status_banner().unwrap_or_default()))
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
                        .rounded(radius_card)
                        .hover(|s| s.bg(pill_glass_hover))
                        .on_press(store::dismiss_banner)
                        .child(icon(IconKind::Close, 16.0, move || {
                            theme().accent_foreground
                        })),
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

fn playlist_monogram(title: &str) -> String {
    title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "♫".into())
}

fn sidebar_playlist_row(pl: SidebarPlaylist, collapsed: bool) -> impl IntoElement {
    let is_liked = pl.is_liked_auto;
    let id = pl.id.clone();
    let title_str = pl.title.clone();
    let subtitle_str = pl.subtitle.clone();
    let art_url = pl.artwork_url.clone();
    let mono = playlist_monogram(&title_str);

    button()
        .accessibility_label(title_str.clone())
        .w_full()
        .min_h(px(48.0))
        .flex_row()
        .items_center()
        .when(collapsed, |b| b.justify_center().px_2().py_1())
        .when(!collapsed, |b| b.gap_2p5().px_2().py_1p5())
        .rounded(radius_card)
        .hover(|s| s.bg(pill_glass_surface))
        .active(|s| s.opacity(0.76))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .on_press(move || {
            if is_liked {
                store::set_library_filter(LibraryFilter::LikedSongs);
                store::set_tab(AppTab::Library);
            } else {
                store::open_collection(
                    id.clone(),
                    title_str.clone(),
                    subtitle_str.clone(),
                    art_url.clone(),
                );
            }
        })
        .child(
            view()
                .w(px(36.0))
                .h(px(36.0))
                .rounded(px(6.0))
                .overflow_hidden()
                .when(is_liked, |v| v.child(liked_music_tile_svg(36.0)))
                .when(!is_liked, |v| {
                    v.child(artwork_box(pl.artwork_url, mono, 36.0, 6.0))
                }),
        )
        .when(!collapsed, |b| {
            b.child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_0p5()
                    .child(
                        text(pl.title)
                            .number_of_lines(1)
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        text(pl.subtitle)
                            .number_of_lines(1)
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    ),
            )
        })
}

fn desktop_sidebar() -> impl IntoElement {
    let collapsed = store::sidebar_collapsed;
    let playlists_memo = computed(store::sidebar_playlists);

    view()
        .w(move || if collapsed() { px(72.0) } else { px(236.0) })
        .h_full()
        .p_3()
        .gap_1()
        .bg(color_sidebar_surface)
        .border_r_1()
        .border_color(color_sidebar_border)
        // Top header row: Menu toggle + YT Music mark + Raag / YouTube Music
        .child(
            view()
                .flex_row()
                .items_center()
                .py_2()
                .mb_2()
                .style_when(collapsed, |s| s.justify_center())
                .style_when(move || !collapsed(), |s| s.gap_2p5().px_1())
                .child(svg_icon_button(
                    IconKind::Menu,
                    18.0,
                    36.0,
                    "Toggle Sidebar",
                    true,
                    move || theme().foreground,
                    store::toggle_sidebar,
                ))
                .child(when(
                    move || !collapsed(),
                    || {
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(yt_music_icon(26.0))
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(
                                        text("Raag")
                                            .number_of_lines(1)
                                            .text_lg()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        text("YouTube Music")
                                            .number_of_lines(1)
                                            .text_xs()
                                            .text_color(move || theme().muted_foreground),
                                    ),
                            )
                    },
                )),
        )
        // Primary navigation
        .child(desktop_nav_item(AppTab::Home, "Home", IconKind::Home))
        .child(desktop_nav_item(
            AppTab::Explore,
            "Explore",
            IconKind::Explore,
        ))
        .child(desktop_nav_item(
            AppTab::Library,
            "Library",
            IconKind::Library,
        ))
        .child(desktop_nav_item(
            AppTab::ListenTogether,
            "Radio & Rooms",
            IconKind::ListenTogether,
        ))
        .child(desktop_nav_item(
            AppTab::NowPlaying,
            "Now Playing",
            IconKind::NowPlaying,
        ))
        .child(desktop_nav_item(AppTab::Lyrics, "Lyrics", IconKind::Lyrics))
        .child(desktop_nav_item(AppTab::Queue, "Queue", IconKind::Queue))
        // Divider + PLAYLISTS header
        .child(view().h(px(1.0)).w_full().bg(color_sidebar_border).my_2())
        .child(when(
            move || !collapsed(),
            || {
                view().px_3().pb_1().child(
                    text("PLAYLISTS")
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().muted_foreground),
                )
            },
        ))
        // Scrollable playlists list
        .child(scroll().flex_1().min_h(px(0.0)).child(dynamic(move || {
            let is_collapsed = collapsed();
            let list = playlists_memo.get();
            view().gap_0p5().children(
                list.into_iter()
                    .map(move |pl| sidebar_playlist_row(pl, is_collapsed)),
            )
        })))
        // Pinned footer: Settings + shortcut legend
        .child(view().h(px(1.0)).w_full().bg(color_sidebar_border).my_1())
        .child(desktop_nav_item(
            AppTab::Settings,
            "Settings",
            IconKind::Settings,
        ))
        .child(when(
            move || !collapsed(),
            || {
                view()
                    .mt_1()
                    .px_3()
                    .py_1p5()
                    .rounded(radius_card)
                    .bg(pill_glass_surface)
                    .child(
                        text("Shortcuts: Space · ←/→ · N/P · L · M")
                            .number_of_lines(1)
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    )
            },
        ))
}

fn desktop_top_header() -> impl IntoElement {
    let header_query = signal(store::search_query());
    effect(move || {
        let store_q = store::search_query();
        if header_query.get_untracked().trim() != store_q {
            header_query.set(store_q);
        }
    });

    when(
        move || store::active_tab() != AppTab::NowPlaying,
        move || {
            view()
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_4()
                .px_6()
                .py_3()
                // Left: Back / Forward / Refresh + Pill Search Bar
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .child(
                                    button()
                                        .accessibility_label("Go Back")
                                        .w(px(34.0))
                                        .h(px(34.0))
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .hover(|s| s.bg(pill_glass_hover))
                                        .active(|s| s.opacity(0.72))
                                        .on_press(store::go_back)
                                        .child(icon(IconKind::ChevronLeft, 18.0, move || {
                                            if store::can_go_back() {
                                                theme().foreground
                                            } else {
                                                theme().muted_foreground
                                            }
                                        })),
                                )
                                .child(
                                    button()
                                        .accessibility_label("Go Forward")
                                        .w(px(34.0))
                                        .h(px(34.0))
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .hover(|s| s.bg(pill_glass_hover))
                                        .active(|s| s.opacity(0.72))
                                        .on_press(store::go_forward)
                                        .child(icon(IconKind::ChevronRight, 18.0, move || {
                                            if store::can_go_forward() {
                                                theme().foreground
                                            } else {
                                                theme().muted_foreground
                                            }
                                        })),
                                )
                                .child(svg_icon_button(
                                    IconKind::Refresh,
                                    16.0,
                                    34.0,
                                    "Refresh Home",
                                    true,
                                    move || theme().muted_foreground,
                                    store::refresh_home,
                                )),
                        )
                        // Sleek pill search bar
                        .child(
                            view()
                                .flex_1()
                                .max_w(px(480.0))
                                .h(px(40.0))
                                .px_3()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .rounded(radius_pill)
                                .bg(pill_glass_surface)
                                .border_1()
                                .border_color(glass_hairline)
                                .child(
                                    button()
                                        .test_id("nav-search")
                                        .accessibility_label("Open Search")
                                        .w(px(28.0))
                                        .h(px(28.0))
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .hover(|s| s.bg(pill_glass_hover))
                                        .on_press(move || {
                                            store::run_search(&header_query.get_untracked());
                                            store::set_tab(AppTab::Search);
                                        })
                                        .child(icon(IconKind::Search, 16.0, move || {
                                            theme().muted_foreground
                                        })),
                                )
                                .child(
                                    input()
                                        .test_id("header-search-input")
                                        .value(move || header_query.get())
                                        .on_change(move |txt: &str| {
                                            header_query.set(txt.to_owned());
                                            store::run_search(txt);
                                            if !txt.trim().is_empty()
                                                && store::active_tab() != AppTab::Search
                                            {
                                                store::set_tab(AppTab::Search);
                                            }
                                        })
                                        .on_submit(move |txt: &str| {
                                            header_query.set(txt.to_owned());
                                            store::run_search(txt);
                                            store::set_tab(AppTab::Search);
                                        })
                                        .placeholder("Search songs, albums, artists, podcasts")
                                        .placeholder_color(|| color(theme().muted_foreground))
                                        .caret_color(|| color(theme().primary))
                                        .return_key_type(ReturnKeyType::Search)
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .text_sm()
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    when(
                                        move || !header_query.get().trim().is_empty(),
                                        move || {
                                            svg_icon_button(
                                                IconKind::Close,
                                                15.0,
                                                28.0,
                                                "Clear Search",
                                                true,
                                                move || theme().muted_foreground,
                                                move || {
                                                    header_query.set(String::new());
                                                    store::run_search("");
                                                },
                                            )
                                        },
                                    )
                                    .otherwise(|| {
                                        view()
                                            .w(px(28.0))
                                            .h(px(28.0))
                                            .items_center()
                                            .justify_center()
                                            .child(icon(IconKind::Headphones, 16.0, move || {
                                                theme().muted_foreground
                                            }))
                                    }),
                                ),
                        ),
                )
                // Right: Account Avatar opening Settings
                .child(
                    button()
                        .accessibility_label("Account & Settings")
                        .rounded_full()
                        .hover(|s| s.opacity(0.88))
                        .active(|s| s.opacity(0.75))
                        .focus_visible(|s| {
                            s.outline(px(2.0)).outline_color(move || theme().ring)
                        })
                        .on_press(|| store::set_tab(AppTab::Settings))
                        .child(
                            avatar()
                                .name(move || {
                                    let (signed_in, acct) = store::auth_info();
                                    if signed_in {
                                        SharedString::from(
                                            acct.unwrap_or_else(|| "Raag User".into()),
                                        )
                                    } else {
                                        SharedString::from("Raag")
                                    }
                                })
                                .accessibility_label("")
                                .fallback_class("size-9 rounded-full items-center justify-center bg-secondary text-foreground text-xs font-bold")
                                .size_9()
                                .rounded_full()
                                .overflow_hidden()
                                .border_1()
                                .border_color(glass_hairline),
                        ),
                )
        },
    )
}

fn desktop_side_panel() -> impl IntoElement {
    let show_panel = move || {
        is_desktop_panel_visible()
            && store::desktop_side_panel_open()
            && !matches!(store::active_tab(), AppTab::Queue | AppTab::Lyrics)
    };
    let is_queue = move || store::desktop_side_panel() == DesktopSidePanel::Queue;

    when(show_panel, move || {
        view()
            .test_id("desktop-side-panel")
            .w(px(308.0))
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
                    .items_center()
                    .gap_2()
                    .child(
                        tabs()
                            .flex_1()
                            .flex_row()
                            .gap_1()
                            .p_1()
                            .rounded(radius_card)
                            .bg(pill_glass_surface)
                            .indicator_class("inset-y-1 rounded-lg bg-primary")
                            .accessibility_label("Side panel sections")
                            .child(
                                tab()
                                    .test_id("side-panel-tab-queue")
                                    .selected(is_queue)
                                    .on_press(|| {
                                        store::set_desktop_side_panel(DesktopSidePanel::Queue)
                                    })
                                    .flex_1()
                                    .min_h(px(36.0))
                                    .items_center()
                                    .justify_center()
                                    .rounded(move || theme().radius)
                                    .child(
                                        text("Up Next")
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || {
                                                if is_queue() {
                                                    theme().primary_foreground
                                                } else {
                                                    theme().muted_foreground
                                                }
                                            }),
                                    ),
                            )
                            .child(
                                tab()
                                    .test_id("side-panel-tab-lyrics")
                                    .selected(move || !is_queue())
                                    .on_press(|| {
                                        store::set_desktop_side_panel(DesktopSidePanel::Lyrics)
                                    })
                                    .flex_1()
                                    .min_h(px(36.0))
                                    .items_center()
                                    .justify_center()
                                    .rounded(move || theme().radius)
                                    .child(
                                        text("Synced Lyrics")
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || {
                                                if !is_queue() {
                                                    theme().primary_foreground
                                                } else {
                                                    theme().muted_foreground
                                                }
                                            }),
                                    ),
                            ),
                    )
                    .child(svg_icon_button(
                        IconKind::Close,
                        15.0,
                        34.0,
                        "Close Side Panel",
                        true,
                        move || theme().muted_foreground,
                        move || store::toggle_desktop_side_panel(store::desktop_side_panel()),
                    )),
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
                            .rounded(radius_card)
                            .style_when(active, |s| s.bg(sidebar_active_surface))
                            .hover(|s| s.bg(pill_glass_surface))
                            .on_press(move || store::jump_queue(at.index()))
                            .child(
                                text(move || {
                                    track.with(|t| {
                                        SharedString::from(
                                            t.as_ref().map(|x| x.title.clone()).unwrap_or_default(),
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
                        || store::current_lyrics().map(|d| d.lines.len()).unwrap_or(0),
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
                                .rounded(radius_card)
                                .style_when(active, |s| s.bg(sidebar_active_surface))
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

fn mobile_top_header() -> impl IntoElement {
    when(
        move || store::active_tab() != AppTab::NowPlaying,
        || {
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .px_4()
                .py_1p5()
                .border_b_1()
                .border_color(glass_hairline)
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .flex_shrink_0()
                        .gap_2()
                        .child(yt_music_icon(22.0))
                        .child(
                            text("Raag")
                                .number_of_lines(1)
                                .min_w(px(64.0))
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(
                            button()
                                .test_id("nav-explore")
                                .accessibility_label("Explore")
                                .w(px(MOBILE_MIN_TOUCH_TARGET))
                                .h(px(MOBILE_MIN_TOUCH_TARGET))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(|| store::set_tab(AppTab::Explore))
                                .child(icon(IconKind::Explore, 20.0, move || {
                                    if store::active_tab() == AppTab::Explore {
                                        theme().primary
                                    } else {
                                        theme().foreground
                                    }
                                })),
                        )
                        .child(
                            button()
                                .test_id("nav-settings")
                                .accessibility_label("Settings")
                                .w(px(MOBILE_MIN_TOUCH_TARGET))
                                .h(px(MOBILE_MIN_TOUCH_TARGET))
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .hover(|s| s.bg(pill_glass_hover))
                                .on_press(|| store::set_tab(AppTab::Settings))
                                .child(icon(IconKind::Settings, 20.0, move || {
                                    if store::active_tab() == AppTab::Settings {
                                        theme().primary
                                    } else {
                                        theme().foreground
                                    }
                                })),
                        ),
                )
        },
    )
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
        .child(mobile_nav_item(AppTab::Home, "Home", IconKind::Home))
        .child(mobile_nav_item(AppTab::Search, "Search", IconKind::Search))
        .child(mobile_nav_item(AppTab::Lyrics, "Lyrics", IconKind::Lyrics))
        .child(mobile_nav_item(AppTab::Queue, "Queue", IconKind::Queue))
        .child(mobile_nav_item(
            AppTab::Library,
            "Library",
            IconKind::Library,
        ))
}

pub fn app_shell() -> impl IntoElement {
    let desktop = is_desktop_layout;

    view()
        .flex_1()
        .bg(move || theme().background)
        .child(
            when(desktop, || {
                view()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(
                        view()
                            .flex_1()
                            .min_h(px(0.0))
                            .flex_row()
                            .child(desktop_sidebar())
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .min_h(px(0.0))
                                    // Subtle warm top-ambient glow behind non-NowPlaying desktop views
                                    .child(when(
                                        move || store::active_tab() != AppTab::NowPlaying,
                                        || {
                                            view()
                                                .absolute()
                                                .top_0()
                                                .left_0()
                                                .right_0()
                                                .h(px(260.0))
                                                .pointer_events(PointerEvents::None)
                                                .bg_linear_to_b()
                                                .gradient_from(color_ambient_warm)
                                                .gradient_to(move || theme().background)
                                        },
                                    ))
                                    .child(desktop_top_header())
                                    .child(updater_top_banner(
                                        store::updater_available(),
                                        Rc::new(|| store::set_tab(AppTab::Settings)),
                                        Rc::new(store::dismiss_updater_banner),
                                    ))
                                    .child(active_screen())
                                    .child(status_toast()),
                            )
                            .child(desktop_side_panel()),
                    )
                    .child(desktop_player_bar())
            })
            .otherwise(|| {
                view()
                    .flex_1()
                    .pt_safe_area()
                    .child(mobile_top_header())
                    .child(active_screen())
                    .child(status_toast())
                    .child(persistent_mini_player())
                    .child(mobile_bottom_nav())
            }),
        )
        .child(track_action_sheet())
        .child(add_to_playlist_sheet())
        .child(auth_sign_in_sheet(
            signal(store::auth_sheet_open()),
            signal(store::rooms_snapshot().tv_oauth_user_code),
            signal(store::rooms_snapshot().tv_oauth_verification_url),
            signal(store::rooms_snapshot().tv_oauth_expires_in_secs),
            AuthSignInCallbacks {
                on_close: std::sync::Arc::new(store::close_auth_sheet),
                ..AuthSignInCallbacks::default()
            },
        ))
        .child(when(store::cast_sheet_open, || {
            cast_picker_sheet(
                store::cast_devices(),
                store::cast_session(),
                |_| {},
                || {},
                |_| {},
                || store::set_cast_sheet_open(false),
            )
        }))
}
