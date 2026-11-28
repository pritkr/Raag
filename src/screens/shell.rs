//! Adaptive Application Shell for Raag:
//! - Desktop (>= 820px): Collapsible left sidebar (brand, primary nav, scrollable playlists,
//!   settings & shortcut legend) + warm-ambient main canvas with top nav/search/avatar header +
//!   optional frosted right Queue/Lyrics panel + full-width bottom player bar.
//! - Android / Mobile (< 820px): Top safe-area header + main content + Mini-Player strictly
//!   stacked ABOVE the bottom navigation bar + `.pb_safe_area_plus(px(4.0))` (>= 48dp targets).

use std::rc::Rc;
use std::sync::Arc;

use nsh_base::{button, input, popover, tab, tabs, theme};
use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;
use resonance_core::offline::PinnedCollectionKind;

use crate::screens::{
    album::{AlbumScreenCallbacks, album_screen_with_detail},
    artist::{ArtistScreenCallbacks, artist_screen_with_detail},
    auth_sign_in_sheet::{AuthSignInCallbacks, auth_sign_in_sheet},
    explore, home, library,
    listen_together::{ListenTogetherCallbacks, listen_together_screen},
    lyrics, now_playing, queue, search, settings,
    settings_updater_card::updater_top_banner,
};
use crate::store::{
    self, AppTab, DesktopSidePanel, EntityRoute, LibraryFilter, SidebarPlaylist,
};
use crate::ui::controls::{
    active_state_dot, add_to_playlist_sheet, ambient_shell_backdrop, artwork_box, back_button,
    cast_picker_sheet, desktop_player_bar, live_equalizer_indicator, mini_player_bar,
    setting_switch, sleep_timer_sheet, speed_sheet, svg_icon_button, track_action_sheet,
};
use crate::ui::icons::{IconKind, icon, liked_music_tile_svg, raag_brand_icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_lyric_inactive, color_on_cover, cover_gradient_pair,
    eyebrow_label, frosted_bar_surface, frosted_card_surface, frosted_drawer_surface,
    frosted_header_surface, frosted_sidebar_surface, glass_hairline, glass_specular_hairline,
    is_desktop_layout, is_desktop_panel_visible, motion, overlay_scrim, pill_glass_hover,
    pill_glass_surface, radius_card, radius_panel, radius_pill, radius_tab_inner,
    sidebar_active_surface,
};

fn shell_transition_ms() -> f32 {
    motion::transition_ms(motion::PRESS_MS, store::reduced_motion())
}

fn shell_press_scale() -> f32 {
    if store::reduced_motion() {
        1.0
    } else {
        motion::PRESS_SCALE
    }
}

fn trigger_shell_layout_animation() {
    if !store::reduced_motion() {
        let _ = std::panic::catch_unwind(|| {
            animate_layout(nsh_core::LayoutAnimation::EASE_IN_EASE_OUT);
        });
    }
}

fn primary_nav_index(tab: AppTab) -> Option<usize> {
    match tab {
        AppTab::Home => Some(0),
        AppTab::Explore => Some(1),
        AppTab::Library => Some(2),
        AppTab::ListenTogether => Some(3),
        AppTab::NowPlaying
        | AppTab::Lyrics
        | AppTab::Queue
        | AppTab::Search
        | AppTab::Settings => None,
    }
}

fn side_panel_queue_artwork(
    track: Memo<Option<resonance_contract::Track>>,
    is_active: impl Fn() -> bool + Copy + 'static,
) -> impl IntoElement {
    let has_art = move || {
        track.with(|t| {
            t.as_ref()
                .and_then(|x| store::resolve_artwork_url(x.artwork_url.clone()))
                .is_some()
        })
    };

    view()
        .relative()
        .w(px(38.0))
        .h(px(38.0))
        .rounded(px(6.0))
        .bg(move || {
            let seed = track.with(|t| {
                t.as_ref()
                    .map(|x| x.id.clone())
                    .unwrap_or_else(|| "q".into())
            });
            cover_gradient_pair(&seed).0
        })
        .bg_linear_to_br()
        .gradient_from(move || {
            let seed = track.with(|t| {
                t.as_ref()
                    .map(|x| x.id.clone())
                    .unwrap_or_else(|| "q".into())
            });
            cover_gradient_pair(&seed).0
        })
        .gradient_to(move || {
            let seed = track.with(|t| {
                t.as_ref()
                    .map(|x| x.id.clone())
                    .unwrap_or_else(|| "q".into())
            });
            cover_gradient_pair(&seed).1
        })
        .border_1()
        .border_color(glass_hairline)
        .items_center()
        .justify_center()
        .overflow_hidden()
        .child(
            when(has_art, move || {
                image(move || {
                    let raw = track.with(|t| t.as_ref().and_then(|x| x.artwork_url.clone()));
                    ImageSrc::from(store::resolve_artwork_url(raw).unwrap_or_default())
                })
                .size(px(38.0))
                .object_fit(ResizeMode::Cover)
                .rounded(px(6.0))
            })
            .otherwise(move || {
                text(move || {
                    track.with(|t| {
                        SharedString::from(
                            t.as_ref()
                                .map(|x| x.fallback_monogram())
                                .unwrap_or_else(|| "♪".into()),
                        )
                    })
                })
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(color_on_cover)
            }),
        )
        .child(when(is_active, || {
            view()
                .absolute()
                .inset_0()
                .bg(frosted_bar_surface)
                .opacity(0.76)
                .items_center()
                .justify_center()
                .child(live_equalizer_indicator(
                    || store::player_snapshot().state.is_playing(),
                    16.0,
                ))
        }))
}

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
    show_static_bg: bool,
) -> impl IntoElement {
    let selected = move || store::active_tab() == tab_kind;
    let collapsed = store::sidebar_collapsed;

    button()
        .test_id(nav_test_id(tab_kind))
        .accessibility_label(label)
        .on_press(move || store::set_tab(tab_kind))
        .w_full()
        .h(px(40.0))
        .flex_row()
        .items_center()
        .style_when(collapsed, |s| s.justify_center().px_2())
        .style_when(move || !collapsed(), |s| s.gap_3().px_3())
        .rounded(radius_card)
        .style_when(
            move || show_static_bg && selected(),
            |s| s.bg(sidebar_active_surface),
        )
        .transition()
        .duration(shell_transition_ms)
        .ease_out()
        .hover(|s| s.bg(pill_glass_surface))
        .active(|s| s.scale(shell_press_scale).opacity(0.84))
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

fn has_active_entity_screen() -> bool {
    store::current_entity_route().is_some()
}

fn entity_loading_or_error_view(kind_label: &'static str, id: String, is_album: bool) -> AnyElement {
    let err = store::entity_error();
    let id_retry = id.clone();
    view()
        .flex_1()
        .p_6()
        .gap_4()
        .child(view().self_start().child(back_button(store::go_back)))
        .child(if let Some(err_msg) = err {
            view()
                .p_5()
                .gap_3()
                .rounded(radius_card)
                .bg(move || theme().card)
                .border_1()
                .border_color(glass_hairline)
                .child(
                    text(format!("Could not load {kind_label}"))
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(err_msg)
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    button()
                        .self_start()
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .px_5()
                        .rounded_full()
                        .bg(move || theme().primary)
                        .transition()
                        .duration(shell_transition_ms)
                        .ease_out()
                        .hover(|s| s.opacity(0.92))
                        .active(|s| s.scale(shell_press_scale))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(move || {
                            if is_album {
                                store::open_album_entity(id_retry.clone());
                            } else {
                                store::open_artist_entity(id_retry.clone());
                            }
                        })
                        .child(
                            text("Retry")
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().primary_foreground),
                        ),
                )
                .into_element()
        } else {
            view()
                .gap_3()
                .child(
                    text(format!("Loading {kind_label}…"))
                        .text_base()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
                .child(crate::ui::controls::skeleton_row())
                .child(crate::ui::controls::skeleton_row())
                .child(crate::ui::controls::skeleton_row())
                .into_element()
        })
        .into_element()
}

fn render_active_entity_screen() -> AnyElement {
    match store::current_entity_route() {
        Some(EntityRoute::Album { browse_id }) => {
            if let Some((album, progress)) = store::active_album_detail()
                && !store::entity_loading()
            {
                return album_screen_with_detail(
                    album,
                    progress,
                    AlbumScreenCallbacks {
                        on_back: Arc::new(store::go_back),
                        on_play_album: Arc::new(|tracks| {
                            store::play_queue(tracks, 0);
                        }),
                        on_shuffle_album: Arc::new(store::play_all_shuffled),
                        on_start_radio: Arc::new(|album| {
                            store::set_autoplay_radio(true);
                            store::play_queue(album.tracks, 0);
                        }),
                        on_toggle_pin_offline: Arc::new(|album| {
                            let is_pinned = store::pinned_collections()
                                .iter()
                                .any(|c| c.id == album.browse_id);
                            if is_pinned {
                                store::unpin_collection(&album.browse_id);
                            } else {
                                store::pin_collection(
                                    album.browse_id,
                                    PinnedCollectionKind::Album,
                                    album.title,
                                    album.release_year,
                                    album.artwork_url,
                                    album.tracks,
                                );
                            }
                        }),
                        on_add_album_to_playlist: Arc::new(|album| {
                            if let Some(first) = album.tracks.first().cloned() {
                                store::open_add_to_playlist_sheet(first);
                            }
                        }),
                        on_open_artist: Arc::new(store::open_artist_entity),
                        on_open_album: Arc::new(store::open_album_entity),
                        on_play_track: Arc::new(store::play_track),
                        on_play_next: Arc::new(store::play_next),
                    },
                )
                .into_element();
            }
            return entity_loading_or_error_view("album", browse_id, true);
        }
        Some(EntityRoute::Artist { channel_id }) => {
            if let Some(artist) = store::active_artist_detail()
                && !store::entity_loading()
            {
                return artist_screen_with_detail(
                    artist,
                    ArtistScreenCallbacks {
                        on_back: Arc::new(store::go_back),
                        on_play_top_songs: Arc::new(|tracks| {
                            store::play_queue(tracks, 0);
                        }),
                        on_start_radio: Arc::new(|artist| {
                            store::set_autoplay_radio(true);
                            store::play_queue(artist.top_songs, 0);
                        }),
                        on_shuffle: Arc::new(store::play_all_shuffled),
                        on_toggle_subscribe: Arc::new(store::toggle_subscribe_artist),
                        on_play_track: Arc::new(store::play_track),
                        on_play_next: Arc::new(store::play_next),
                        on_open_album: Arc::new(store::open_album_entity),
                        on_open_artist: Arc::new(store::open_artist_entity),
                    },
                )
                .into_element();
            }
            return entity_loading_or_error_view("artist", channel_id, false);
        }
        None => {}
    }
    view().into_element()
}

fn wired_listen_together_screen() -> impl IntoElement {
    listen_together_screen(
        store::synced_signal(store::active_room_snapshot),
        store::synced_signal(store::rooms_local_track_warning),
        ListenTogetherCallbacks {
            on_create_room: Arc::new(store::create_listen_room),
            on_join_room: Arc::new(store::join_listen_room),
            on_leave_room: Arc::new(store::leave_listen_room),
            on_start_local_radio: Arc::new(store::start_local_radio_mix),
            on_vote_skip: Arc::new(store::vote_skip_room),
            on_send_reaction: Arc::new(store::send_room_reaction),
            on_kick_participant: Arc::new(store::kick_room_participant),
            on_transfer_host: Arc::new(store::transfer_room_host),
            on_copy_invite: Arc::new(|code| {
                store::copy_text_to_clipboard("room code", &code);
            }),
        },
    )
}

fn active_screen() -> impl IntoElement {
    view().flex_1().min_h(px(0.0)).child(
        when(has_active_entity_screen, || {
            dynamic(render_active_entity_screen)
        })
        .otherwise(|| {
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
                    wired_listen_together_screen,
                ))
                .child(when(
                    move || store::active_tab() == AppTab::Settings,
                    settings::settings_screen,
                ))
        }),
    )
}

fn status_toast() -> impl IntoElement {
    view().test_id("global-status-region").w_full().child(when(
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
                        .number_of_lines(3)
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
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(store::dismiss_banner)
                        .child(icon(IconKind::Close, 16.0, move || {
                            theme().accent_foreground
                        })),
                )
        },
    ))
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

fn shortcut_legend_row(keys: &'static str, action: &'static str) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            text(action)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .px_2()
                .py_0p5()
                .rounded(px(6.0))
                .bg(pill_glass_surface)
                .border_1()
                .border_color(glass_hairline)
                .child(
                    text(keys)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .tabular_nums()
                        .text_color(move || theme().foreground),
                ),
        )
}

fn keyboard_shortcuts_popover() -> impl IntoElement {
    popover()
        .placement(Placement::Bottom)
        .align(Align::End)
        .offset(px(8.0))
        .margin(px(12.0))
        .popup_style(|s| {
            s.rounded(radius_panel)
                .backdrop_blur(px(28.0))
                .bg(frosted_card_surface)
                .border_1()
                .border_color(glass_hairline)
                .border_t_color(glass_specular_hairline)
                .shadow_2xl()
        })
        .trigger(
            button()
                .test_id("shortcuts-help-btn")
                .accessibility_label("Keyboard Shortcuts")
                .w(px(34.0))
                .h(px(34.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(pill_glass_surface)
                .border_1()
                .border_color(glass_hairline)
                .transition()
                .duration(shell_transition_ms)
                .ease_out()
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.scale(shell_press_scale))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .child(icon(IconKind::HelpCircle, 16.0, move || theme().foreground)),
        )
        .child(
            view()
                .test_id("shortcuts-popover")
                .w(px(264.0))
                .p_3p5()
                .gap_2p5()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(icon(IconKind::HelpCircle, 15.0, move || theme().primary))
                        .child(
                            text("Keyboard Shortcuts")
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        ),
                )
                .child(view().h(px(1.0)).w_full().bg(glass_hairline))
                .child(shortcut_legend_row("Space", "Play / Pause"))
                .child(shortcut_legend_row("← / →", "Seek ±5 seconds"))
                .child(shortcut_legend_row("N / P", "Next / Previous track"))
                .child(shortcut_legend_row("L", "Like current track"))
                .child(shortcut_legend_row("M", "Mute / Unmute")),
        )
}

fn desktop_sidebar() -> impl IntoElement {
    let collapsed = store::sidebar_collapsed;
    let playlists_memo = computed(store::sidebar_playlists);

    // Smooth sliding active pill behind the 4 primary nav items (40px item + 4px gap = 44px pitch).
    const NAV_PITCH: f32 = 44.0;
    let init_idx = primary_nav_index(store::active_tab());
    let nav_y = animated_value(init_idx.unwrap_or(0) as f32 * NAV_PITCH);
    let nav_visible = animated_value(if init_idx.is_some() { 1.0 } else { 0.0 });

    let nav_y_eff = nav_y.clone();
    let nav_vis_eff = nav_visible.clone();
    effect(move || {
        let idx_opt = primary_nav_index(store::active_tab());
        let reduced = store::reduced_motion();
        match idx_opt {
            Some(idx) => {
                let target = idx as f32 * NAV_PITCH;
                if reduced {
                    nav_y_eff.set(target);
                    nav_vis_eff.set(1.0);
                } else {
                    nav_y_eff.timing(target, motion::LAYOUT_MS, Easing::EaseInOut);
                    nav_vis_eff.timing(1.0, motion::PRESS_MS, Easing::EaseOut);
                }
            }
            None => {
                if reduced {
                    nav_vis_eff.set(0.0);
                } else {
                    nav_vis_eff.timing(0.0, motion::PRESS_MS, Easing::EaseOut);
                }
            }
        }
    });

    view()
        .test_id("desktop-sidebar")
        .w(move || if collapsed() { px(72.0) } else { px(236.0) })
        .h_full()
        .p_3()
        .gap_1()
        .backdrop_blur(px(20.0))
        .backdrop_material(BackdropMaterial::Thin)
        .bg(frosted_sidebar_surface)
        .border_r_1()
        .border_color(glass_hairline)
        // Top header row: Hamburger menu toggle + crimson Music emblem + "Music" wordmark
        .child(
            view()
                .flex_row()
                .items_center()
                .py_1p5()
                .mb_2()
                .style_when(collapsed, |s| s.justify_center())
                .style_when(move || !collapsed(), |s| s.gap_2p5().px_1())
                .child(
                    button()
                        .test_id("sidebar-toggle")
                        .accessibility_label("Toggle Sidebar")
                        .w(px(36.0))
                        .h(px(36.0))
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .transition()
                        .duration(shell_transition_ms)
                        .ease_out()
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.scale(shell_press_scale))
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .on_press(|| {
                            trigger_shell_layout_animation();
                            store::toggle_sidebar();
                        })
                        .child(icon(IconKind::Menu, 18.0, move || theme().foreground)),
                )
                .child(when(
                    move || !collapsed(),
                    || {
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(raag_brand_icon(24.0))
                            .child(
                                text("Music")
                                    .number_of_lines(1)
                                    .text_lg()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            )
                    },
                )),
        )
        // Collapsed emblem icon (matches 06c_sidebar_collapsed.png)
        .child(when(collapsed, || {
            view()
                .w_full()
                .pb_2()
                .items_center()
                .justify_center()
                .child(raag_brand_icon(24.0))
        }))
        // Primary navigation in exact ytm-rs order: Home, Explore, Library, Listen together
        .child(
            view()
                .relative()
                .gap_1()
                .child(
                    view()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(40.0))
                        .rounded(radius_card)
                        .bg(sidebar_active_surface)
                        .border_1()
                        .border_color(glass_hairline)
                        .flex_row()
                        .items_center()
                        .pointer_events(PointerEvents::None)
                        .accessibility_elements_hidden(true)
                        .bind_translate_y(&nav_y)
                        .bind_opacity(&nav_visible),
                )
                .child(desktop_nav_item(AppTab::Home, "Home", IconKind::Home, false))
                .child(desktop_nav_item(
                    AppTab::Explore,
                    "Explore",
                    IconKind::Explore,
                    false,
                ))
                .child(desktop_nav_item(
                    AppTab::Library,
                    "Library",
                    IconKind::Library,
                    false,
                ))
                .child(desktop_nav_item(
                    AppTab::ListenTogether,
                    "Listen together",
                    IconKind::ListenTogether,
                    false,
                )),
        )
        // Divider + PLAYLISTS header
        .child(view().h(px(1.0)).w_full().bg(glass_hairline).my_2())
        .child(when(
            move || !collapsed(),
            || view().px_3().pb_1().child(eyebrow_label("Playlists")),
        ))
        // Independently scrollable playlists list
        .child(scroll().flex_1().min_h(px(0.0)).child(dynamic(move || {
            let is_collapsed = collapsed();
            let list = playlists_memo.get();
            view().gap_0p5().children(
                list.into_iter()
                    .map(move |pl| sidebar_playlist_row(pl, is_collapsed)),
            )
        })))
        // Bottom-pinned Settings item
        .child(view().h(px(1.0)).w_full().bg(glass_hairline).my_1())
        .child(desktop_nav_item(
            AppTab::Settings,
            "Settings",
            IconKind::Settings,
            true,
        ))
}

fn search_suggestions_dropdown(header_query: Signal<String>) -> impl IntoElement {
    let show_dropdown = move || store::search_dropdown_open();

    when(show_dropdown, move || {
        view()
            .test_id("search-suggestions-dropdown")
            .absolute()
            .top(px(46.0))
            .left_0()
            .right_0()
            .z_index(40)
            .p_2()
            .gap_1()
            .rounded(radius_panel)
            .backdrop_blur(px(28.0))
            .backdrop_material(BackdropMaterial::Regular)
            .bg(frosted_drawer_surface)
            .border_1()
            .border_color(glass_hairline)
            .border_t_color(glass_specular_hairline)
            .shadow_2xl()
            .child(dynamic(move || {
                let q = header_query.get();
                let trimmed = q.trim();
                let live_suggs = store::search_suggestions();

                let mut items: Vec<String> = if !live_suggs.is_empty() {
                    live_suggs
                } else if trimmed.is_empty() {
                    vec![
                        "Midnight Raga".to_owned(),
                        "A.R. Rahman".to_owned(),
                        "Monsoon Synth".to_owned(),
                        "Lo-Fi Chill Beats".to_owned(),
                        "Indian Indie Acoustics".to_owned(),
                    ]
                } else {
                    let ql = trimmed.to_lowercase();
                    let mut matches: Vec<String> = store::all_library_songs()
                        .into_iter()
                        .filter_map(|t| {
                            if t.title.to_lowercase().contains(&ql) {
                                Some(t.title)
                            } else if t.artist_display().to_lowercase().contains(&ql) {
                                Some(t.artist_display())
                            } else {
                                None
                            }
                        })
                        .collect();
                    matches.sort();
                    matches.dedup();
                    if matches.is_empty() {
                        matches.push(trimmed.to_owned());
                    }
                    matches.into_iter().take(6).collect()
                };

                if items.is_empty() {
                    items.push("Midnight Raga".to_owned());
                }

                view()
                    .gap_0p5()
                    .children(items.into_iter().take(6).map(move |suggestion| {
                        let s_click = suggestion.clone();
                        let s_label = suggestion.clone();
                        button()
                            .accessibility_label(s_label)
                            .w_full()
                            .min_h(px(38.0))
                            .px_3()
                            .flex_row()
                            .items_center()
                            .gap_2p5()
                            .rounded(radius_card)
                            .hover(|s| s.bg(pill_glass_hover))
                            .active(|s| s.opacity(0.8))
                            .focus_visible(|s| {
                                s.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .on_press(move || {
                                header_query.set(s_click.clone());
                                store::set_search_dropdown_open(false);
                                store::run_search(&s_click);
                                store::set_tab(AppTab::Search);
                            })
                            .child(icon(IconKind::Search, 14.0, move || {
                                theme().muted_foreground
                            }))
                            .child(
                                text(suggestion)
                                    .flex_1()
                                    .number_of_lines(1)
                                    .text_sm()
                                    .text_color(move || theme().foreground),
                            )
                    }))
                    .into_element()
            }))
    })
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
                .sticky()
                .top_0()
                .z_index(20)
                .w_full()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_4()
                .px_6()
                .py_3()
                .backdrop_blur(px(16.0))
                .backdrop_material(BackdropMaterial::Thin)
                .bg(frosted_header_surface)
                // Left: Back / Forward / Refresh circular ghost buttons
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
                                .transition()
                                .duration(shell_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(shell_press_scale).opacity(0.8))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
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
                                .transition()
                                .duration(shell_transition_ms)
                                .ease_out()
                                .hover(|s| s.bg(pill_glass_hover))
                                .active(|s| s.scale(shell_press_scale).opacity(0.8))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
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
                // Center: Pill-shaped search bar + floating live suggestions dropdown
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .items_center()
                        .child(
                            view()
                                .relative()
                                .w_full()
                                .max_w(px(520.0))
                                .child(
                                    view()
                                        .w_full()
                                        .h(px(40.0))
                                        .px_3()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .rounded(radius_pill)
                                        .bg(pill_glass_surface)
                                        .border_1()
                                        .border_color(move || theme().input)
                                        .child(
                                            button()
                                                .test_id("nav-search")
                                                .accessibility_label("Open Search")
                                                .w(px(28.0))
                                                .h(px(28.0))
                                                .items_center()
                                                .justify_center()
                                                .rounded_full()
                                                .transition()
                                                .duration(shell_transition_ms)
                                                .ease_out()
                                                .hover(|s| s.bg(pill_glass_hover))
                                                .active(|s| s.scale(shell_press_scale))
                                                .focus_visible(|s| {
                                                    s.outline(px(2.0))
                                                        .outline_color(move || theme().ring)
                                                })
                                                .on_press(move || {
                                                    store::set_search_dropdown_open(false);
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
                                                .on_focus(|| {
                                                    crate::ui::controls::set_text_input_focused(
                                                        true,
                                                    );
                                                    store::set_search_dropdown_open(true);
                                                })
                                                .on_blur(|| {
                                                    crate::ui::controls::set_text_input_focused(
                                                        false,
                                                    );
                                                })
                                                .on_change(move |txt: &str| {
                                                    header_query.set(txt.to_owned());
                                                    store::fetch_search_suggestions(txt);
                                                    store::set_search_dropdown_open(true);
                                                    store::run_search(txt);
                                                    if !txt.trim().is_empty()
                                                        && store::active_tab() != AppTab::Search
                                                    {
                                                        store::set_tab(AppTab::Search);
                                                    }
                                                })
                                                .on_submit(move |txt: &str| {
                                                    header_query.set(txt.to_owned());
                                                    store::set_search_dropdown_open(false);
                                                    store::run_search(txt);
                                                    store::set_tab(AppTab::Search);
                                                })
                                                .placeholder(
                                                    "Search songs, albums, artists, podcasts",
                                                )
                                                .placeholder_color(|| {
                                                    color(theme().muted_foreground)
                                                })
                                                .caret_color(|| color(theme().primary))
                                                .return_key_type(ReturnKeyType::Search)
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .text_sm()
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(when(
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
                                                        store::set_search_dropdown_open(false);
                                                        store::run_search("");
                                                    },
                                                )
                                            },
                                        ))
                                        .child(
                                            button()
                                                .accessibility_label("Audio Search")
                                                .w(px(28.0))
                                                .h(px(28.0))
                                                .items_center()
                                                .justify_center()
                                                .rounded_full()
                                                .hover(|s| s.bg(pill_glass_hover))
                                                .focus_visible(|s| {
                                                    s.outline(px(2.0))
                                                        .outline_color(move || theme().ring)
                                                })
                                                .on_press(|| {
                                                    store::set_search_dropdown_open(false);
                                                    store::set_tab(AppTab::Search);
                                                })
                                                .child(icon(IconKind::Headphones, 16.0, move || {
                                                    theme().muted_foreground
                                                })),
                                        ),
                                )
                                .child(search_suggestions_dropdown(header_query)),
                        ),
                )
                // Right: Keyboard Shortcuts `?` Popover + Circular User Profile Avatar -> Settings > Account
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(keyboard_shortcuts_popover())
                        .child(
                            button()
                                .test_id("nav-account-avatar")
                                .accessibility_label("Account & Settings")
                                .rounded_full()
                                .transition()
                                .duration(shell_transition_ms)
                                .ease_out()
                                .hover(|s| s.opacity(0.88))
                                .active(|s| s.scale(shell_press_scale).opacity(0.8))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(store::open_settings_account)
                                .child(
                                    view()
                                        .w(px(34.0))
                                        .h(px(34.0))
                                        .rounded_full()
                                        .bg(move || theme().primary)
                                        .border_1()
                                        .border_color(glass_hairline)
                                        .items_center()
                                        .justify_center()
                                        .overflow_hidden()
                                        .child(
                                            text(move || {
                                                let (signed_in, acct) = store::auth_info();
                                                let source = if signed_in {
                                                    acct.unwrap_or_else(|| "Raag User".into())
                                                } else {
                                                    "Raag".into()
                                                };
                                                let mut chars = source
                                                    .chars()
                                                    .filter(|c| c.is_alphanumeric());
                                                let initials: String = chars
                                                    .by_ref()
                                                    .take(1)
                                                    .flat_map(|c| c.to_uppercase())
                                                    .collect();
                                                SharedString::from(if initials.is_empty() {
                                                    "R".to_owned()
                                                } else {
                                                    initials
                                                })
                                            })
                                            .number_of_lines(1)
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(color_on_cover),
                                        ),
                                ),
                        ),
                )
        },
    )
}

fn desktop_side_panel() -> impl IntoElement {
    // Allow right-hand `Up next` drawer to open on Home/Explore/Library/NowPlaying (matching 07_now_playing_02_queue.png!)
    let show_panel = move || {
        is_desktop_panel_visible()
            && store::desktop_side_panel_open()
            && !matches!(store::active_tab(), AppTab::Queue | AppTab::Lyrics)
    };
    let is_queue = move || store::desktop_side_panel() == DesktopSidePanel::Queue;
    let is_lyrics = move || store::desktop_side_panel() == DesktopSidePanel::Lyrics;
    let is_related = move || store::desktop_side_panel() == DesktopSidePanel::Related;

    when(show_panel, move || {
        view()
            .test_id("desktop-side-panel")
            .w(px(344.0))
            .h_full()
            .p_3()
            .gap_3()
            .backdrop_blur(px(28.0))
            .backdrop_material(BackdropMaterial::Regular)
            .bg(frosted_drawer_surface)
            .border_l_1()
            .border_color(glass_hairline)
            // Top bar: "Up next" title + Save to Playlist, Clear Queue, Close buttons
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .px_1()
                    .gap_2()
                    .child(
                        text(move || match store::desktop_side_panel() {
                            DesktopSidePanel::Queue => SharedString::from("Up next"),
                            DesktopSidePanel::Lyrics => SharedString::from("Lyrics"),
                            DesktopSidePanel::Related => SharedString::from("Related"),
                        })
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().foreground),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .child(svg_icon_button(
                                IconKind::ListPlus,
                                15.0,
                                32.0,
                                "Save queue to playlist",
                                true,
                                move || theme().muted_foreground,
                                || {
                                    if let Some(track) = store::player_snapshot().current_track {
                                        store::open_add_to_playlist_sheet(track);
                                    }
                                },
                            ))
                            .child(svg_icon_button(
                                IconKind::Trash,
                                15.0,
                                32.0,
                                "Clear Up Next",
                                true,
                                move || theme().muted_foreground,
                                store::clear_queue,
                            ))
                            .child(svg_icon_button(
                                IconKind::Close,
                                15.0,
                                32.0,
                                "Close Side Panel",
                                true,
                                move || theme().muted_foreground,
                                move || {
                                    trigger_shell_layout_animation();
                                    store::toggle_desktop_side_panel(store::desktop_side_panel());
                                },
                            )),
                    ),
            )
            // Compact segmented switcher (`Up Next` / `Synced Lyrics` / `Related`)
            .child(
                tabs()
                    .w_full()
                    .flex_row()
                    .gap_1()
                    .p_1()
                    .rounded(radius_card)
                    .bg(pill_glass_surface)
                    .indicator_style(|s| {
                        s.top_1()
                            .bottom_1()
                            .rounded(radius_tab_inner)
                            .bg(sidebar_active_surface)
                    })
                    .accessibility_label("Side panel sections")
                    .child(
                        tab()
                            .test_id("side-panel-tab-queue")
                            .selected(is_queue)
                            .on_press(|| store::set_desktop_side_panel(DesktopSidePanel::Queue))
                            .flex_1()
                            .min_h(px(32.0))
                            .px_1()
                            .items_center()
                            .justify_center()
                            .rounded(radius_tab_inner)
                            .focus_visible(|s| {
                                s.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .child(
                                text("Up Next")
                                    .number_of_lines(1)
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || {
                                        if is_queue() {
                                            theme().foreground
                                        } else {
                                            theme().muted_foreground
                                        }
                                    }),
                            ),
                    )
                    .child(
                        tab()
                            .test_id("side-panel-tab-lyrics")
                            .selected(is_lyrics)
                            .on_press(|| store::set_desktop_side_panel(DesktopSidePanel::Lyrics))
                            .flex_grow(1.35)
                            .min_h(px(32.0))
                            .px_1()
                            .items_center()
                            .justify_center()
                            .rounded(radius_tab_inner)
                            .focus_visible(|s| {
                                s.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .child(
                                text("Synced Lyrics")
                                    .number_of_lines(1)
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || {
                                        if is_lyrics() {
                                            theme().foreground
                                        } else {
                                            theme().muted_foreground
                                        }
                                    }),
                            ),
                    )
                    .child(
                        tab()
                            .test_id("side-panel-tab-related")
                            .selected(is_related)
                            .on_press(|| store::set_desktop_side_panel(DesktopSidePanel::Related))
                            .flex_1()
                            .min_h(px(32.0))
                            .px_1()
                            .items_center()
                            .justify_center()
                            .rounded(radius_tab_inner)
                            .focus_visible(|s| {
                                s.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .child(
                                text("Related")
                                    .number_of_lines(1)
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || {
                                        if is_related() {
                                            theme().foreground
                                        } else {
                                            theme().muted_foreground
                                        }
                                    }),
                            ),
                    ),
            )
            // Body: Queue (with Autoplay similar songs toggle + duration column), Synced Lyrics, or Related
            .child(
                when(is_queue, || {
                    view()
                        .flex_1()
                        .min_h(px(0.0))
                        .gap_2()
                        .child(setting_switch(
                            "Autoplay similar songs",
                            || store::player_snapshot().queue.autoplay_radio,
                            store::set_autoplay_radio,
                        ))
                        .child(
                            virtual_list(store::queue_len, move |at| {
                                let track = computed(move || store::queue_track_at(at.index()));
                                let active = move || {
                                    store::player_snapshot().queue.current_index == Some(at.index())
                                };
                                button()
                                    .w_full()
                                    .min_h(px(50.0))
                                    .px_2()
                                    .py_1p5()
                                    .rounded(radius_card)
                                    .style_when(active, |s| s.bg(sidebar_active_surface))
                                    .hover(|s| s.bg(pill_glass_surface))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
                                    .on_press(move || store::jump_queue(at.index()))
                                    .child(
                                        view()
                                            .flex_row()
                                            .items_center()
                                            .gap_2p5()
                                            .w_full()
                                            .min_w(px(0.0))
                                            .child(side_panel_queue_artwork(track, active))
                                            .child(
                                                view()
                                                    .flex_1()
                                                    .min_w(px(0.0))
                                                    .gap_0p5()
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
                                                        .text_color(move || {
                                                            theme().muted_foreground
                                                        }),
                                                    ),
                                            )
                                            .child(
                                                text(move || {
                                                    track.with(|t| {
                                                        SharedString::from(
                                                            t.as_ref()
                                                                .map(|x| {
                                                                    resonance_contract::format_duration_ms(
                                                                        x.duration_ms,
                                                                    )
                                                                })
                                                                .unwrap_or_default(),
                                                        )
                                                    })
                                                })
                                                .text_xs()
                                                .tabular_nums()
                                                .text_color(move || theme().muted_foreground),
                                            ),
                                    )
                            })
                            .keys(store::queue_ids)
                            .estimated_item_size(px(54.0))
                            .flex_1(),
                        )
                })
                .otherwise(move || {
                    when(is_lyrics, || {
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
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
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
                                                color_lyric_inactive()
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
                    })
                    .otherwise(|| {
                        scroll().flex_1().min_h(px(0.0)).child(dynamic(|| {
                            let items = store::related_tracks();
                            if items.is_empty() {
                                return view()
                                    .p_4()
                                    .gap_2()
                                    .child(
                                        text("You might also like")
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        text("Play a track to see similar songs and mixes.")
                                            .text_xs()
                                            .text_color(move || theme().muted_foreground),
                                    )
                                    .into_element();
                            }
                            view()
                                .gap_1()
                                .child(
                                    text("You might also like")
                                        .px_2()
                                        .pb_1()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().muted_foreground),
                                )
                                .children(items.into_iter().map(|t| {
                                    let t_play = t.clone();
                                    let t_next = t.clone();
                                    let mono = t.fallback_monogram();
                                    let art_url = t.artwork_url.clone();
                                    let title = t.title.clone();
                                    let artist = t.artist_display();
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .px_2()
                                        .py_1p5()
                                        .rounded(radius_card)
                                        .hover(|s| s.bg(pill_glass_surface))
                                        .child(
                                            button()
                                                .flex_1()
                                                .min_w(px(0.0))
                                                .min_h(px(44.0))
                                                .flex_row()
                                                .items_center()
                                                .gap_2p5()
                                                .rounded(move || theme().radius)
                                                .focus_visible(|s| {
                                                    s.outline(px(2.0))
                                                        .outline_color(move || theme().ring)
                                                })
                                                .on_press(move || store::play_track(t_play.clone()))
                                                .child(artwork_box(art_url, mono, 38.0, 6.0))
                                                .child(
                                                    view()
                                                        .flex_1()
                                                        .min_w(px(0.0))
                                                        .gap_0p5()
                                                        .child(
                                                            text(title)
                                                                .number_of_lines(1)
                                                                .text_xs()
                                                                .font_weight(FontWeight::SEMIBOLD)
                                                                .text_color(move || {
                                                                    theme().foreground
                                                                }),
                                                        )
                                                        .child(
                                                            text(artist)
                                                                .number_of_lines(1)
                                                                .text_xs()
                                                                .text_color(move || {
                                                                    theme().muted_foreground
                                                                }),
                                                        ),
                                                ),
                                        )
                                        .child(svg_icon_button(
                                            IconKind::Plus,
                                            14.0,
                                            32.0,
                                            "Play Next",
                                            true,
                                            move || theme().muted_foreground,
                                            move || store::play_next(t_next.clone()),
                                        ))
                                }))
                                .into_element()
                        }))
                    })
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
                .backdrop_blur(px(16.0))
                .bg(frosted_header_surface)
                .border_b_1()
                .border_color(glass_hairline)
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .flex_shrink_0()
                        .gap_2()
                        .child(raag_brand_icon(22.0))
                        .child(
                            text("Music")
                                .number_of_lines(1)
                                .min_w(px(56.0))
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
                                .test_id("nav-rooms")
                                .selected(move || store::active_tab() == AppTab::ListenTogether)
                                .accessibility_label("Listen together")
                                .w(px(MOBILE_MIN_TOUCH_TARGET))
                                .h(px(MOBILE_MIN_TOUCH_TARGET))
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_0p5()
                                .rounded_full()
                                .style_when(
                                    move || store::active_tab() == AppTab::ListenTogether,
                                    |s| s.bg(sidebar_active_surface),
                                )
                                .hover(|s| s.bg(pill_glass_hover))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(|| store::set_tab(AppTab::ListenTogether))
                                .child(icon(IconKind::ListenTogether, 20.0, move || {
                                    if store::active_tab() == AppTab::ListenTogether {
                                        theme().primary
                                    } else {
                                        theme().foreground
                                    }
                                }))
                                .child(active_state_dot(move || {
                                    store::active_tab() == AppTab::ListenTogether
                                })),
                        )
                        .child(
                            button()
                                .test_id("nav-queue")
                                .selected(move || store::active_tab() == AppTab::Queue)
                                .accessibility_label("Queue")
                                .w(px(MOBILE_MIN_TOUCH_TARGET))
                                .h(px(MOBILE_MIN_TOUCH_TARGET))
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_0p5()
                                .rounded_full()
                                .style_when(
                                    move || store::active_tab() == AppTab::Queue,
                                    |s| s.bg(sidebar_active_surface),
                                )
                                .hover(|s| s.bg(pill_glass_hover))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(|| store::set_tab(AppTab::Queue))
                                .child(icon(IconKind::Queue, 20.0, move || {
                                    if store::active_tab() == AppTab::Queue {
                                        theme().primary
                                    } else {
                                        theme().foreground
                                    }
                                }))
                                .child(active_state_dot(move || {
                                    store::active_tab() == AppTab::Queue
                                })),
                        )
                        .child(
                            button()
                                .test_id("nav-settings")
                                .selected(move || store::active_tab() == AppTab::Settings)
                                .accessibility_label("Settings")
                                .w(px(MOBILE_MIN_TOUCH_TARGET))
                                .h(px(MOBILE_MIN_TOUCH_TARGET))
                                .flex_col()
                                .items_center()
                                .justify_center()
                                .gap_0p5()
                                .rounded_full()
                                .style_when(
                                    move || store::active_tab() == AppTab::Settings,
                                    |s| s.bg(sidebar_active_surface),
                                )
                                .hover(|s| s.bg(pill_glass_hover))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(|| store::set_tab(AppTab::Settings))
                                .child(icon(IconKind::Settings, 20.0, move || {
                                    if store::active_tab() == AppTab::Settings {
                                        theme().primary
                                    } else {
                                        theme().foreground
                                    }
                                }))
                                .child(active_state_dot(move || {
                                    store::active_tab() == AppTab::Settings
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
        .child(mobile_nav_item(
            AppTab::Explore,
            "Explore",
            IconKind::Explore,
        ))
        .child(mobile_nav_item(AppTab::Search, "Search", IconKind::Search))
        .child(mobile_nav_item(
            AppTab::Library,
            "Library",
            IconKind::Library,
        ))
}

pub fn app_shell() -> impl IntoElement {
    let desktop = is_desktop_layout;

    view()
        .relative()
        .flex_1()
        .overflow_hidden()
        .bg(move || theme().background)
        // Full-window dynamic moving blurred-glass ambient backdrop (outside all virtual lists)
        .child(ambient_shell_backdrop())
        .child(
            view()
                .relative()
                .flex_1()
                .min_h(px(0.0))
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
                                            .relative()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .min_h(px(0.0))
                                            .child(desktop_top_header())
                                            .child(dynamic(move || {
                                                updater_top_banner(
                                                    store::updater_available(),
                                                    Rc::new(|| store::set_tab(AppTab::Settings)),
                                                    Rc::new(store::dismiss_updater_banner),
                                                )
                                                .into_element()
                                            }))
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
                ),
        )
        .child(track_action_sheet())
        .child(add_to_playlist_sheet())
        .child(sleep_timer_sheet())
        .child(speed_sheet())
        .child(auth_sign_in_sheet(
            store::synced_signal(store::auth_sheet_open),
            store::synced_signal(|| store::settings_snapshot().tv_oauth_user_code),
            store::synced_signal(|| store::settings_snapshot().tv_oauth_verification_url),
            store::synced_signal(|| store::settings_snapshot().tv_oauth_expires_in_secs),
            AuthSignInCallbacks {
                on_close: Arc::new(store::close_auth_sheet),
                on_start_tv_oauth: Arc::new(store::start_tv_oauth),
                on_cancel_tv_oauth: Arc::new(store::cancel_tv_oauth),
                on_submit_cookies: Arc::new(|raw| {
                    store::sign_in_with_cookies(&raw);
                    store::close_auth_sheet();
                }),
            },
        ))
        .child(when(store::cast_sheet_open, || {
            view()
                .test_id("cast-picker-overlay")
                .absolute()
                .inset_0()
                .bg(overlay_scrim)
                .items_center()
                .justify_center()
                .p_4()
                .child(
                    view().w_full().max_w(px(420.0)).child(dynamic(|| {
                        cast_picker_sheet(
                            store::cast_devices(),
                            store::cast_session(),
                            store::select_cast_device,
                            store::disconnect_cast,
                            store::set_cast_volume,
                            || store::set_cast_sheet_open(false),
                        )
                        .into_element()
                    })),
                )
        }))
}
