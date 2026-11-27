//! Reusable sheet & modal panel surfaces with semantic theme tokens and >= 48dp targets.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::DownloadStatus;

use crate::store::{self, AppTab};
use crate::ui::controls::context_menu::{track_sheet_header_preview, track_youtube_music_url};
use crate::ui::controls::primitives::{confirm_destructive, labeled_input};
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_card_surface, glass_hairline, glass_specular_hairline,
    is_desktop_layout, motion, overlay_scrim, pill_glass_hover, pill_glass_surface, radius_card,
    radius_panel, radius_pill,
};

fn sheet_transition_ms() -> f32 {
    motion::transition_ms(motion::PRESS_MS, store::reduced_motion())
}

fn sheet_press_scale() -> f32 {
    if store::reduced_motion() {
        1.0
    } else {
        motion::PRESS_SCALE
    }
}

/// Frosted glassmorphism panel container (`BackdropMaterial::Thick`) for modals & sheets.
pub fn modal_panel_container(
    title: impl IntoReactive<SharedString>,
    subtitle: impl IntoReactive<SharedString>,
    content: impl IntoElement,
) -> impl IntoElement {
    view()
        .w_full()
        .max_w(px(440.0))
        .p_4()
        .gap_3()
        .rounded(radius_panel)
        .backdrop_blur(px(28.0))
        .backdrop_material(BackdropMaterial::Thick)
        .bg(frosted_card_surface)
        .border_1()
        .border_color(glass_hairline)
        .border_t_color(glass_specular_hairline)
        .shadow_2xl()
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
        .child(scroll().max_h(px(520.0)).child(content))
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
        .transition()
        .duration(sheet_transition_ms)
        .ease_out()
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.scale(sheet_press_scale).opacity(0.84))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(icon(leading, 16.0, move || theme().foreground))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
}

/// Reusable Track Action Sheet opened from track row `⋯` menus.
pub fn track_action_sheet() -> impl IntoElement {
    let active = computed(store::active_track_sheet);

    when(
        move || active.get().is_some(),
        move || {
            let Some((track, playlist_id)) = active.get() else {
                return view().into_element();
            };
            let t_next = track.clone();
            let t_queue = track.clone();
            let t_radio = track.clone();
            let t_like = track.clone();
            let t_playlist = track.clone();
            let t_dl = track.clone();
            let vid_dl = track.id.clone();
            let vid_link = track.id.clone();
            let is_liked = store::is_track_liked(&track.id);
            let dl_rec = store::download_record(&track.id);
            let dl_label = match dl_rec.as_ref().map(|r| r.status) {
                None => "Download for Offline",
                Some(
                    DownloadStatus::Queued | DownloadStatus::Downloading | DownloadStatus::Tagging,
                ) => "Pause Download",
                Some(DownloadStatus::Paused) => "Resume Download",
                Some(DownloadStatus::Completed) => "Remove Download",
                Some(DownloadStatus::Failed | DownloadStatus::Cancelled) => "Retry Download",
            };
            let dl_status = dl_rec.as_ref().map(|r| r.status);
            let album_ref = track.album.clone();
            let artist_ref = track.artists.first().cloned();
            let vid_remove = track.id.clone();

            view()
                .test_id("track-action-sheet")
                .absolute()
                .inset_0()
                .bg(overlay_scrim)
                .items_center()
                .justify_center()
                .p_4()
                .child(modal_panel_container(
                    track.title.clone(),
                    track.artist_display(),
                    view()
                        .gap_1()
                        .child(track_sheet_header_preview(&track))
                        .child(action_sheet_row(IconKind::Play, "Play Next", move || {
                            store::play_next(t_next.clone());
                            store::close_track_sheet();
                        }))
                        .child(action_sheet_row(
                            IconKind::Queue,
                            "Add to Queue",
                            move || {
                                store::play_last(t_queue.clone());
                                store::close_track_sheet();
                            },
                        ))
                        .child(action_sheet_row(
                            IconKind::Radio,
                            "Start Radio",
                            move || {
                                store::set_autoplay_radio(true);
                                store::play_track(t_radio.clone());
                                store::close_track_sheet();
                            },
                        ))
                        .child(action_sheet_row(
                            if is_liked {
                                IconKind::HeartFilled
                            } else {
                                IconKind::Heart
                            },
                            if is_liked { "Remove Like" } else { "Like Song" },
                            move || {
                                store::toggle_like_track(t_like.clone());
                                store::close_track_sheet();
                            },
                        ))
                        .child(action_sheet_row(
                            IconKind::Plus,
                            "Add to Playlist…",
                            move || {
                                store::open_add_to_playlist_sheet(t_playlist.clone());
                            },
                        ))
                        .child(action_sheet_row(IconKind::Download, dl_label, move || {
                            match dl_status {
                                None | Some(DownloadStatus::Failed | DownloadStatus::Cancelled) => {
                                    store::enqueue_download(t_dl.clone())
                                }
                                Some(
                                    DownloadStatus::Queued
                                    | DownloadStatus::Downloading
                                    | DownloadStatus::Tagging,
                                ) => store::pause_download(&vid_dl),
                                Some(DownloadStatus::Paused) => store::resume_download(&vid_dl),
                                Some(DownloadStatus::Completed) => {
                                    let id = vid_dl.clone();
                                    let title = t_dl.title.clone();
                                    confirm_destructive(
                                        "Remove offline download?",
                                        format!(
                                            "Remove “{title}” from offline downloads on this device?"
                                        ),
                                        "Remove Download",
                                        move || store::delete_download(&id),
                                    );
                                }
                            }
                            store::close_track_sheet();
                        }))
                        .when_some(album_ref, |v, alb| {
                            let alb_title = alb.title.clone();
                            let alb_id = alb.id.clone();
                            v.child(action_sheet_row(
                                IconKind::Library,
                                "Go to Album",
                                move || {
                                    if let Some(id) = alb_id.clone() {
                                        store::open_album_entity(id);
                                    } else {
                                        store::open_collection(
                                            alb_title.clone(),
                                            alb_title.clone(),
                                            "Album".to_owned(),
                                            None,
                                        );
                                    }
                                    store::close_track_sheet();
                                },
                            ))
                        })
                        .when_some(artist_ref, |v, art| {
                            let art_name = art.name.clone();
                            let art_id = art.id.clone();
                            v.child(action_sheet_row(
                                IconKind::Headphones,
                                "Go to Artist",
                                move || {
                                    if let Some(id) = art_id.clone() {
                                        store::open_artist_entity(id);
                                    } else {
                                        store::run_search(&art_name);
                                        store::set_tab(AppTab::Search);
                                    }
                                    store::close_track_sheet();
                                },
                            ))
                        })
                        .child(action_sheet_row(IconKind::Lyrics, "View Lyrics", || {
                            store::set_tab(AppTab::Lyrics);
                            store::close_track_sheet();
                        }))
                        .child(action_sheet_row(
                            IconKind::Link,
                            "Share Track Link",
                            move || {
                                let url = track_youtube_music_url(&vid_link);
                                store::copy_text_to_clipboard("track link", &url);
                                store::close_track_sheet();
                            },
                        ))
                        .when_some(playlist_id, |v, pid| {
                            let vid = vid_remove.clone();
                            let track_title = track.title.clone();
                            v.child(action_sheet_row(
                                IconKind::Trash,
                                "Remove from Playlist",
                                move || {
                                    let pid_c = pid.clone();
                                    let vid_c = vid.clone();
                                    confirm_destructive(
                                        "Remove from playlist?",
                                        format!("Remove “{track_title}” from this playlist?"),
                                        "Remove",
                                        move || {
                                            store::remove_track_from_playlist(pid_c, vid_c);
                                        },
                                    );
                                    store::close_track_sheet();
                                },
                            ))
                        })
                        .child(action_sheet_row(IconKind::Close, "Close", || {
                            store::close_track_sheet();
                        })),
                ))
                .into_element()
        },
    )
}

/// Add-to-Playlist modal sheet (`test_id("add-to-playlist-sheet")`) with "+ New Playlist" creator.
pub fn add_to_playlist_sheet() -> impl IntoElement {
    let target_track = computed(store::add_to_playlist_track);
    let new_title = signal(String::new());
    let privacy = signal("Private");

    when(
        move || target_track.get().is_some(),
        move || {
            let Some(track) = target_track.get() else {
                return view().into_element();
            };
            let (signed_in, _) = store::auth_info();
            let playlists = store::library_playlists();
            let t_create = track.clone();

            let privacy_btn = move |lbl: &'static str| {
                let is_sel = move || privacy.get() == lbl;
                button()
                    .selected(is_sel)
                    .on_press(move || privacy.set(lbl))
                    .min_h(px(36.0))
                    .style_when(
                        || !is_desktop_layout(),
                        |s| s.min_h(px(MOBILE_MIN_TOUCH_TARGET)),
                    )
                    .px_3()
                    .items_center()
                    .justify_center()
                    .rounded(radius_pill)
                    .bg(move || {
                        if is_sel() {
                            theme().primary
                        } else {
                            pill_glass_surface()
                        }
                    })
                    .border_1()
                    .border_color(glass_hairline)
                    .transition()
                    .duration(sheet_transition_ms)
                    .ease_out()
                    .hover(|s| s.opacity(0.92))
                    .active(|s| s.scale(sheet_press_scale))
                    .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                    .child(
                        text(lbl)
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || {
                                if is_sel() {
                                    theme().primary_foreground
                                } else {
                                    theme().foreground
                                }
                            }),
                    )
            };

            view()
                .test_id("add-to-playlist-sheet")
                .absolute()
                .inset_0()
                .bg(overlay_scrim)
                .items_center()
                .justify_center()
                .p_4()
                .child(modal_panel_container(
                    "Add to Playlist",
                    track.title.clone(),
                    view()
                        .gap_2()
                        .child(
                            view()
                                .p_3()
                                .gap_2()
                                .rounded(radius_card)
                                .bg(move || theme().card)
                                .border_1()
                                .border_color(glass_hairline)
                                .child(
                                    text("+ New Playlist")
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(labeled_input(
                                    "Playlist name",
                                    "new-playlist-title-input",
                                    "e.g. Late Night Raag",
                                    move || new_title.get(),
                                    move |s| new_title.set(s.to_owned()),
                                ))
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_1p5()
                                        .child(privacy_btn("Private"))
                                        .child(privacy_btn("Unlisted"))
                                        .child(privacy_btn("Public")),
                                )
                                .child(
                                    button()
                                        .on_press(move || {
                                            let title = new_title.get_untracked();
                                            let clean = title.trim();
                                            if !clean.is_empty() {
                                                store::create_local_playlist_and_add(
                                                    clean,
                                                    privacy.get_untracked(),
                                                    t_create.clone(),
                                                );
                                                new_title.set(String::new());
                                            }
                                        })
                                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                        .px_4()
                                        .items_center()
                                        .justify_center()
                                        .rounded(radius_pill)
                                        .bg(move || theme().primary)
                                        .transition()
                                        .duration(sheet_transition_ms)
                                        .ease_out()
                                        .hover(|s| s.opacity(0.92))
                                        .active(|s| s.scale(sheet_press_scale))
                                        .focus_visible(|s| {
                                            s.outline(px(2.0)).outline_color(move || theme().ring)
                                        })
                                        .child(
                                            text("Create & Add")
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || theme().primary_foreground),
                                        ),
                                ),
                        )
                        .when(!signed_in, |v| {
                            v.child(
                                text("Sign in to sync playlists with your YouTube Music account")
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                            )
                            .child(action_sheet_row(
                                IconKind::Settings,
                                "Open Account Settings",
                                || {
                                    store::close_add_to_playlist_sheet();
                                    store::set_tab(AppTab::Settings);
                                },
                            ))
                        })
                        .when(signed_in, |v| {
                            v.children(playlists.into_iter().map(move |pl| {
                                let t = track.clone();
                                let pid = pl.playlist_id.unwrap_or(pl.browse_id);
                                let title = pl.title.clone();
                                let label = match pl.track_count {
                                    Some(c) => format!("{} ({c})", pl.title),
                                    None => pl.title,
                                };
                                action_sheet_row(IconKind::Library, label, move || {
                                    store::add_track_to_playlist(
                                        pid.clone(),
                                        title.clone(),
                                        t.clone(),
                                    );
                                })
                            }))
                        })
                        .child(action_sheet_row(IconKind::Close, "Cancel", || {
                            store::close_add_to_playlist_sheet();
                        })),
                ))
                .into_element()
        },
    )
}
