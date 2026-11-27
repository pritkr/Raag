//! Universal Track Context Menu (`...`) for Desktop (`menu`) and Mobile (`sheet`).
//! Zero raw color literals — semantic theme tokens only.

use nsh_base::{button, menu, menu_item, menu_separator, theme};
use nsh_core::prelude::*;
use resonance_contract::{DownloadStatus, Track, format_duration_ms};

use crate::store::{self, AppTab};
use crate::ui::controls::primitives::artwork_box;
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_liked, glass_hairline, pill_glass_hover, radius_card,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackMenuContext {
    General,
    Playlist {
        playlist_id: String,
        set_video_id: Option<String>,
        index: usize,
    },
    Queue {
        queue_index: usize,
    },
    Downloaded,
}

pub fn track_youtube_music_url(track_id: &str) -> String {
    format!("https://music.youtube.com/watch?v={track_id}")
}

fn desktop_menu_row(
    label: impl IntoReactive<SharedString>,
    destructive: bool,
    on_select: impl Fn() + 'static,
) -> impl IntoElement {
    menu_item()
        .on_select(on_select)
        .px_3()
        .py_2()
        .rounded_md()
        .hover(|s| s.bg(pill_glass_hover))
        .child(
            text(label)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if destructive {
                        theme().destructive
                    } else {
                        theme().foreground
                    }
                }),
        )
}

/// Universal Track Context Menu (`...` trigger).
/// On desktop, opens an anchored `nsh_base::menu()`; on mobile, opens the bottom `track_action_sheet`.
pub fn track_context_menu(track: Track, context: TrackMenuContext) -> impl IntoElement {
    let t_sheet = track.clone();
    let playlist_id_for_sheet = match &context {
        TrackMenuContext::Playlist { playlist_id, .. } => Some(playlist_id.clone()),
        _ => None,
    };

    let t_next = track.clone();
    let t_queue = track.clone();
    let t_radio = track.clone();
    let t_like = track.clone();
    let t_playlist = track.clone();
    let t_dl = track.clone();
    let vid_dl = track.id.clone();
    let vid_like = track.id.clone();
    let vid_link = track.id.clone();

    let dl_rec = store::download_record(&track.id);
    let dl_status = dl_rec.as_ref().map(|r| r.status);
    let dl_label = match dl_rec.as_ref() {
        None => "Download for Offline".to_owned(),
        Some(r) => match r.status {
            DownloadStatus::Queued => "Pause Download (Queued)".to_owned(),
            DownloadStatus::Downloading | DownloadStatus::Tagging => {
                let pct = r
                    .bytes_downloaded
                    .saturating_mul(100)
                    .checked_div(r.total_bytes.max(1))
                    .unwrap_or(0)
                    .min(100);
                format!("Pause Download ({pct}%)")
            }
            DownloadStatus::Paused | DownloadStatus::Failed | DownloadStatus::Cancelled => {
                "Resume Download".to_owned()
            }
            DownloadStatus::Completed => "Remove Download".to_owned(),
        },
    };
    let dl_destructive = matches!(dl_status, Some(DownloadStatus::Completed));

    let artist_ref = track.artists.first().cloned();
    let album_ref = track.album.clone();
    let is_liked = store::is_track_liked(&vid_like);
    let like_label = if is_liked { "Remove Like" } else { "Like Song" };

    let trigger_btn = button()
        .test_id(&format!("track-menu-btn-{}", track.id))
        .accessibility_label("Track Actions")
        .w(px(MOBILE_MIN_TOUCH_TARGET))
        .h(px(MOBILE_MIN_TOUCH_TARGET))
        .items_center()
        .justify_center()
        .rounded_full()
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.7))
        .child(icon(IconKind::MoreVertical, 16.0, move || {
            theme().muted_foreground
        }));

    if !crate::ui::tokens::is_desktop_layout() {
        return trigger_btn
            .on_press(move || {
                store::open_track_sheet(t_sheet.clone(), playlist_id_for_sheet.clone())
            })
            .into_element();
    }

    let sep = || {
        menu_separator()
            .h(px(1.0))
            .my_1()
            .bg(move || theme().border)
    };

    let mut m = menu()
        .offset(px(4.0))
        .popup_style(|s| {
            s.p_1()
                .min_w(px(220.0))
                .rounded(radius_card)
                .border_1()
                .border_color(glass_hairline)
                .bg(move || theme().popover)
                .shadow_md()
        })
        .trigger(trigger_btn)
        .child(desktop_menu_row("Play Next", false, move || {
            store::play_next(t_next.clone());
        }))
        .child(desktop_menu_row("Add to Queue", false, move || {
            store::play_last(t_queue.clone());
        }))
        .child(desktop_menu_row("Start Radio", false, move || {
            store::set_autoplay_radio(true);
            store::play_track(t_radio.clone());
        }))
        .child(sep())
        .child(desktop_menu_row(like_label, false, move || {
            store::toggle_like_track(t_like.clone());
        }))
        .child(desktop_menu_row("Add to Playlist...", false, move || {
            store::open_add_to_playlist_sheet(t_playlist.clone());
        }));

    if let TrackMenuContext::Playlist { playlist_id, .. } = &context {
        let pid = playlist_id.clone();
        let vid = track.id.clone();
        m = m.child(desktop_menu_row("Remove from Playlist", true, move || {
            store::remove_track_from_playlist(pid.clone(), vid.clone());
        }));
    }

    if let TrackMenuContext::Queue { queue_index } = &context {
        let idx = *queue_index;
        m = m.child(desktop_menu_row("Remove from Queue", true, move || {
            store::remove_queue_item(idx);
        }));
    }

    m = m.child(sep()).child(desktop_menu_row(
        dl_label,
        dl_destructive,
        move || match dl_status {
            None | Some(DownloadStatus::Failed | DownloadStatus::Cancelled) => {
                store::enqueue_download(t_dl.clone())
            }
            Some(
                DownloadStatus::Queued | DownloadStatus::Downloading | DownloadStatus::Tagging,
            ) => store::pause_download(&vid_dl),
            Some(DownloadStatus::Paused) => store::resume_download(&vid_dl),
            Some(DownloadStatus::Completed) => store::delete_download(&vid_dl),
        },
    ));

    if let Some(art) = artist_ref {
        let art_name = art.name.clone();
        let art_id = art.id.clone();
        m = m.child(desktop_menu_row("Go to Artist", false, move || {
            if let Some(id) = art_id.clone() {
                store::open_artist_entity(id);
            } else {
                store::run_search(&art_name);
                store::set_tab(AppTab::Search);
            }
        }));
    }

    if let Some(alb) = album_ref {
        let alb_title = alb.title.clone();
        let alb_id = alb.id.clone();
        m = m.child(desktop_menu_row("Go to Album", false, move || {
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
        }));
    }

    m = m
        .child(desktop_menu_row("View Lyrics", false, || {
            store::set_tab(AppTab::Lyrics);
        }))
        .child(sep())
        .child(desktop_menu_row("Share Track Link", false, move || {
            let url = track_youtube_music_url(&vid_link);
            store::copy_text_to_clipboard("track link", &url);
        }));

    m.into_element()
}

/// Compact header preview for mobile track action sheet (`48px` artwork + Title + Artist + Duration).
pub fn track_sheet_header_preview(track: &Track) -> impl IntoElement {
    let mono = track.fallback_monogram();
    let sub = format!(
        "{} • {}",
        track.artist_display(),
        format_duration_ms(track.duration_ms)
    );
    let liked = store::is_track_liked(&track.id);

    view()
        .flex_row()
        .items_center()
        .gap_3()
        .pb_2()
        .border_b_1()
        .border_color(glass_hairline)
        .child(artwork_box(track.artwork_url.clone(), mono, 48.0, 8.0))
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text(track.title.clone())
                        .number_of_lines(1)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(sub)
                        .number_of_lines(1)
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(icon(
            if liked {
                IconKind::HeartFilled
            } else {
                IconKind::Heart
            },
            16.0,
            move || {
                if liked {
                    color_liked()
                } else {
                    theme().muted_foreground
                }
            },
        ))
}
