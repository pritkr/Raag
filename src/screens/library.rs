//! Library Screen matching YouTube Music Desktop (`ref_5.jpg`) with:
//! - Horizontal filter pills (`Playlists`, `Songs`, `Liked songs`, `Albums`, `Artists`,
//!   `Pinned Offline`, `Downloads`, `Local Files`, `Podcasts`, `History`, `Uploads`)
//! - Action toolbar (`Play`, `Shuffle`, `Sort`, `"Find in this list"` live search)
//! - Filtered track/collection/artist rows (`track_row_view`)

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::{DownloadRecord, DownloadStatus, HomeItem, Track};

use crate::screens::local_files::{LocalFilesProps, local_files_screen};
use crate::store::library_slice::matches_query;
use crate::store::{self, AppTab, LibraryFilter};
use crate::ui::controls::{
    artwork_box, confirm_destructive, glass_pill_button, hero_pill_button, set_text_input_focused,
    status_card, track_row_view,
};
use crate::ui::icons::{IconKind, icon, liked_music_tile_svg};
use crate::ui::text_format::{format_recoverable_error, pluralize};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_hero_pill_bg, color_hero_pill_fg, glass_hairline,
    is_desktop_layout, pill_glass_hover, pill_glass_surface, radius_card, radius_pill,
};

fn play_filtered_tracks(shuffle: bool) {
    let filter = store::library_filter();
    let query = store::library_search_query();
    let sort_asc = store::collection_sort_asc();
    let tracks = store::filtered_library_tracks(filter, &query, sort_asc);
    if tracks.is_empty() {
        return;
    }
    if shuffle {
        store::play_all_shuffled(tracks);
    } else {
        store::play_queue(tracks, 0);
    }
}

fn filter_pill(filter: LibraryFilter) -> impl IntoElement {
    let selected = move || store::library_filter() == filter;
    let test_id = match filter {
        LibraryFilter::Playlists => "lib-filter-playlists",
        LibraryFilter::Songs => "lib-filter-songs",
        LibraryFilter::LikedSongs => "lib-filter-liked",
        LibraryFilter::Albums => "lib-filter-albums",
        LibraryFilter::Artists => "lib-filter-artists",
        LibraryFilter::PinnedOffline => "lib-filter-pinned",
        LibraryFilter::Downloads => "lib-filter-downloads",
        LibraryFilter::LocalFiles => "lib-filter-local",
        LibraryFilter::Podcasts => "lib-filter-podcasts",
        LibraryFilter::History => "lib-filter-history",
        LibraryFilter::Uploads => "lib-filter-uploads",
    };

    button()
        .test_id(test_id)
        .accessibility_label(filter.label())
        .selected(selected)
        .on_press(move || store::set_library_filter(filter))
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_4()
        .items_center()
        .justify_center()
        .rounded(radius_pill)
        .bg(move || {
            if selected() {
                color_hero_pill_bg()
            } else {
                pill_glass_surface()
            }
        })
        .border_1()
        .border_color(glass_hairline)
        .hover(|s| s.opacity(0.88))
        .active(|s| s.opacity(0.75))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .child(
            text(filter.label())
                .number_of_lines(1)
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || {
                    if selected() {
                        color_hero_pill_fg()
                    } else {
                        theme().foreground
                    }
                }),
        )
}

fn track_table_header() -> impl IntoElement {
    when(is_desktop_layout, || {
        view()
            .flex_row()
            .items_center()
            .gap_2()
            .px_3()
            .py_1p5()
            .border_b_1()
            .border_color(glass_hairline)
            .child(
                text("Title")
                    .flex_1()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().muted_foreground),
            )
            .child(
                view().w(px(180.0)).px_2().child(
                    text("Album")
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                ),
            )
            .child(
                view().w(px(124.0)).items_end().child(
                    text("Duration")
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                ),
            )
    })
}

fn render_track_list_view(
    heading: &'static str,
    tracks: Vec<Track>,
    current_id: Option<String>,
    empty_title: &'static str,
    empty_subtitle: &'static str,
) -> AnyElement {
    let count = tracks.len();
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text(heading)
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(pluralize(count, "track", "tracks"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(tracks.is_empty(), |v| {
            v.child(status_card(
                empty_title,
                empty_subtitle,
                Some("Open Search"),
                Some(Box::new(|| store::set_tab(AppTab::Search))),
            ))
        })
        .when(!tracks.is_empty(), move |v| {
            v.child(track_table_header())
                .children(tracks.into_iter().map(move |track| {
                    let is_cur = current_id.as_deref() == Some(track.id.as_str());
                    let t_play = track.clone();
                    let t_next = track.clone();
                    track_row_view(
                        track,
                        is_cur,
                        move || store::play_track(t_play.clone()),
                        move || store::play_next(t_next.clone()),
                    )
                }))
        })
        .into_element()
}

fn download_status_label(rec: &DownloadRecord) -> String {
    match rec.status {
        DownloadStatus::Queued => "Queued".to_owned(),
        DownloadStatus::Downloading => {
            if let Some(pct) = rec
                .bytes_downloaded
                .saturating_mul(100)
                .checked_div(rec.total_bytes)
            {
                format!("Downloading {}%", pct.min(100))
            } else {
                "Downloading…".to_owned()
            }
        }
        DownloadStatus::Paused => "Paused".to_owned(),
        DownloadStatus::Tagging => "Tagging…".to_owned(),
        DownloadStatus::Completed => {
            let mb = rec.total_bytes as f64 / (1024.0 * 1024.0);
            format!("Offline • {mb:.1} MB")
        }
        DownloadStatus::Failed => rec
            .error_message
            .as_deref()
            .map(|e| {
                format_recoverable_error(
                    "Download failed",
                    e,
                    "Check your connection and select Retry.",
                )
            })
            .unwrap_or_else(|| {
                "Download failed. Check your connection and select Retry.".to_owned()
            }),
        DownloadStatus::Cancelled => "Canceled".to_owned(),
    }
}

fn render_downloads_view(query: &str, sort_asc: bool, current_id: Option<String>) -> AnyElement {
    let ql = query.trim().to_lowercase();
    let mut records: Vec<DownloadRecord> = store::downloads_list()
        .into_iter()
        .filter(|rec| rec.status != DownloadStatus::Cancelled)
        .filter(|rec| {
            ql.is_empty()
                || matches_query(&rec.track.title, &ql)
                || matches_query(&rec.track.artist_display(), &ql)
        })
        .collect();

    if sort_asc {
        records.sort_by(|a, b| {
            a.track
                .title
                .to_lowercase()
                .cmp(&b.track.title.to_lowercase())
        });
    }

    let pinned = store::pinned_collections();
    let count = records.len();
    view()
        .gap_3()
        .when(!pinned.is_empty(), move |v| {
            v.child(
                view()
                    .test_id("pinned-collections-section")
                    .gap_2()
                    .child(
                        text("Pinned Collections")
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .children(pinned.into_iter().map(|col| {
                        let total = col.tracks.len();
                        let completed = col
                            .tracks
                            .iter()
                            .filter(|t| {
                                store::download_record(&t.id)
                                    .is_some_and(|r| r.status == DownloadStatus::Completed)
                            })
                            .count();
                        let col_id = col.id.clone();
                        let col_tracks = col.tracks.clone();
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .px_3()
                            .py_2()
                            .rounded(radius_card)
                            .bg(pill_glass_surface)
                            .border_1()
                            .border_color(glass_hairline)
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .gap_0p5()
                                    .child(
                                        text(col.title)
                                            .number_of_lines(1)
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        text(format!("{completed} / {total} tracks offline"))
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                    ),
                            )
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        button()
                                            .accessibility_label("Play Offline")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .px_3()
                                            .items_center()
                                            .justify_center()
                                            .rounded(radius_pill)
                                            .bg(pill_glass_surface)
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .hover(|s| s.bg(pill_glass_hover))
                                            .focus_visible(|s| {
                                                s.outline(px(2.0))
                                                    .outline_color(move || theme().ring)
                                            })
                                            .on_press(move || {
                                                if !col_tracks.is_empty() {
                                                    store::play_queue(col_tracks.clone(), 0);
                                                }
                                            })
                                            .child(
                                                text("Play Offline")
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || theme().foreground),
                                            ),
                                    )
                                    .child(
                                        button()
                                            .accessibility_label("Unpin collection")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .px_3()
                                            .items_center()
                                            .justify_center()
                                            .rounded(radius_pill)
                                            .bg(pill_glass_surface)
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .hover(|s| s.bg(pill_glass_hover))
                                            .focus_visible(|s| {
                                                s.outline(px(2.0))
                                                    .outline_color(move || theme().ring)
                                            })
                                            .on_press(move || store::unpin_collection(&col_id))
                                            .child(
                                                text("Unpin")
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || theme().muted_foreground),
                                            ),
                                    ),
                            )
                    })),
            )
        })
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text("Offline Downloads")
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(pluralize(count, "item", "items"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(records.is_empty(), |v| {
            v.child(status_card(
                "No offline downloads yet",
                "Download tracks to play them locally from disk with zero network usage.",
                Some("Open Search"),
                Some(Box::new(|| store::set_tab(AppTab::Search))),
            ))
        })
        .when(!records.is_empty(), move |v| {
            v.children(records.into_iter().map(move |rec| {
                let is_cur = current_id.as_deref() == Some(rec.video_id.as_str());
                let status_label = download_status_label(&rec);
                let vid_pause = rec.video_id.clone();
                let vid_resume = rec.video_id.clone();
                let vid_delete = rec.video_id.clone();
                let title_delete = rec.track.title.clone();
                let track_play = rec.track.clone();
                let track_next = rec.track.clone();
                let is_completed = rec.status == DownloadStatus::Completed;
                let is_failed = rec.status == DownloadStatus::Failed;
                let is_active = matches!(
                    rec.status,
                    DownloadStatus::Queued | DownloadStatus::Downloading
                );
                let is_resumable =
                    matches!(rec.status, DownloadStatus::Paused | DownloadStatus::Failed);

                view()
                    .gap_1()
                    .when(is_completed, |wrap| {
                        wrap.child(track_row_view(
                            rec.track.clone(),
                            is_cur,
                            move || store::play_track(track_play.clone()),
                            move || store::play_next(track_next.clone()),
                        ))
                    })
                    .when(!is_completed, |wrap| {
                        wrap.child(
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .px_3()
                                .py_2()
                                .rounded(radius_card)
                                .bg(pill_glass_surface)
                                .border_1()
                                .border_color(glass_hairline)
                                .child(
                                    view()
                                        .flex_1()
                                        .gap_0p5()
                                        .child(
                                            text(rec.track.title.clone())
                                                .number_of_lines(1)
                                                .text_sm()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text(format!(
                                                "{} • {status_label}",
                                                rec.track.artist_display()
                                            ))
                                            .number_of_lines(if is_failed { 3 } else { 1 })
                                            .text_xs()
                                            .text_color(move || {
                                                if is_failed {
                                                    theme().destructive
                                                } else {
                                                    theme().muted_foreground
                                                }
                                            }),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .when(is_active, |row| {
                                            row.child(
                                                button()
                                                    .accessibility_label("Pause download")
                                                    .on_press(move || {
                                                        store::pause_download(&vid_pause)
                                                    })
                                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                    .px_3()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded(radius_pill)
                                                    .bg(pill_glass_surface)
                                                    .border_1()
                                                    .border_color(glass_hairline)
                                                    .hover(|s| s.bg(pill_glass_hover))
                                                    .focus_visible(|s| {
                                                        s.outline(px(2.0))
                                                            .outline_color(move || theme().ring)
                                                    })
                                                    .child(
                                                        text("Pause")
                                                            .text_xs()
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(move || theme().foreground),
                                                    ),
                                            )
                                        })
                                        .when(is_resumable, |row| {
                                            let action_label =
                                                if is_failed { "Retry" } else { "Resume" };
                                            let a11y_label = if is_failed {
                                                "Retry download"
                                            } else {
                                                "Resume download"
                                            };
                                            row.child(
                                                button()
                                                    .accessibility_label(a11y_label)
                                                    .on_press(move || {
                                                        store::resume_download(&vid_resume)
                                                    })
                                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                    .px_3()
                                                    .items_center()
                                                    .justify_center()
                                                    .rounded(radius_pill)
                                                    .bg(pill_glass_surface)
                                                    .border_1()
                                                    .border_color(glass_hairline)
                                                    .hover(|s| s.bg(pill_glass_hover))
                                                    .focus_visible(|s| {
                                                        s.outline(px(2.0))
                                                            .outline_color(move || theme().ring)
                                                    })
                                                    .child(
                                                        text(action_label)
                                                            .text_xs()
                                                            .font_weight(FontWeight::MEDIUM)
                                                            .text_color(move || theme().foreground),
                                                    ),
                                            )
                                        })
                                        .child(
                                            button()
                                                .accessibility_label("Delete download")
                                                .on_press(move || {
                                                    let vid = vid_delete.clone();
                                                    let title = title_delete.clone();
                                                    confirm_destructive(
                                                        "Delete offline download?",
                                                        format!(
                                                            "Remove \"{title}\" from offline storage?"
                                                        ),
                                                        "Remove",
                                                        move || store::delete_download(&vid),
                                                    );
                                                })
                                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                .px_3()
                                                .items_center()
                                                .justify_center()
                                                .rounded(radius_pill)
                                                .bg(pill_glass_surface)
                                                .border_1()
                                                .border_color(glass_hairline)
                                                .hover(|s| s.bg(pill_glass_hover))
                                                .focus_visible(|s| {
                                                    s.outline(px(2.0))
                                                        .outline_color(move || theme().ring)
                                                })
                                                .child(
                                                    text("Remove")
                                                        .text_xs()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(move || theme().destructive),
                                                ),
                                        ),
                                ),
                        )
                    })
            }))
        })
        .into_element()
}

fn render_playlists_view(query: &str, sort_asc: bool) -> AnyElement {
    let ql = query.trim().to_lowercase();
    let mut playlists = store::sidebar_playlists();
    if !ql.is_empty() {
        playlists.retain(|p| matches_query(&p.title, &ql) || matches_query(&p.subtitle, &ql));
    }
    if sort_asc {
        playlists.sort_by_key(|a| a.title.to_lowercase());
    }

    let count = playlists.len();
    let new_pl_title = signal(String::new());

    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text("Playlists")
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(pluralize(count, "playlist", "playlists"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    input()
                        .test_id("library-new-playlist-input")
                        .flex_1()
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .px_3()
                        .rounded(radius_card)
                        .bg(move || theme().card)
                        .border_1()
                        .border_color(move || theme().input)
                        .focus(|s| s.border_color(move || theme().ring))
                        .text_sm()
                        .text_color(move || theme().foreground)
                        .placeholder("New playlist title…")
                        .value(move || new_pl_title.get())
                        .on_focus(|| set_text_input_focused(true))
                        .on_blur(|| set_text_input_focused(false))
                        .on_change(move |v: &str| new_pl_title.set(v.to_owned())),
                )
                .child(hero_pill_button("+ New Playlist", None, move || {
                    let draft = new_pl_title.get();
                    if !draft.trim().is_empty() {
                        store::create_playlist(&draft, "Private");
                        new_pl_title.set(String::new());
                    }
                })),
        )
        .when(playlists.is_empty(), |v| {
            v.child(status_card(
                "No matching playlists",
                "Try clearing your filter or create a new playlist above.",
                None,
                None,
            ))
        })
        .children(playlists.into_iter().map(|pl| {
            let mono = pl
                .title
                .chars()
                .find(|c| c.is_alphanumeric())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "♫".into());
            let is_liked = pl.is_liked_auto;
            let pid = pl.id.clone();
            let pid_delete = pl.id.clone();
            let ptitle = pl.title.clone();
            let ptitle_del = pl.title.clone();
            let psub = pl.subtitle.clone();
            let part = pl.artwork_url.clone();

            view()
                .w_full()
                .min_h(px(62.0))
                .flex_row()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded(radius_card)
                .bg(move || theme().card)
                .border_1()
                .border_color(glass_hairline)
                .child(
                    button()
                        .accessibility_label(ptitle.clone())
                        .flex_1()
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .rounded(radius_card)
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.8))
                        .focus_visible(|s| {
                            s.outline(px(2.0)).outline_color(move || theme().ring)
                        })
                        .on_press(move || {
                            if is_liked {
                                store::set_library_filter(LibraryFilter::LikedSongs);
                            } else {
                                store::open_collection(
                                    pid.clone(),
                                    ptitle.clone(),
                                    psub.clone(),
                                    part.clone(),
                                );
                            }
                        })
                        .child(if is_liked {
                            liked_music_tile_svg(48.0).into_element()
                        } else {
                            artwork_box(pl.artwork_url, mono, 48.0, 8.0).into_element()
                        })
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.0))
                                .gap_0p5()
                                .child(
                                    text(pl.title)
                                        .number_of_lines(1)
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(pl.subtitle)
                                        .number_of_lines(1)
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(icon(IconKind::ChevronRight, 16.0, move || {
                            theme().muted_foreground
                        })),
                )
                .when(!is_liked, move |row| {
                    let pid_del = pid_delete.clone();
                    let ptitle_a11y = ptitle_del.clone();
                    let ptitle_dialog = ptitle_del.clone();
                    row.child(
                        button()
                            .accessibility_label(format!("Delete playlist {ptitle_a11y}"))
                            .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .items_center()
                            .justify_center()
                            .rounded(radius_pill)
                            .hover(|s| s.bg(pill_glass_hover))
                            .active(|s| s.opacity(0.75))
                            .focus_visible(|s| {
                                s.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .on_press(move || {
                                let id = pid_del.clone();
                                let title = ptitle_dialog.clone();
                                confirm_destructive(
                                    "Delete playlist?",
                                    format!(
                                        "Permanently delete \"{title}\"? This cannot be undone."
                                    ),
                                    "Delete",
                                    move || store::delete_playlist(id),
                                );
                            })
                            .child(icon(IconKind::Trash, 16.0, move || theme().destructive)),
                    )
                })
        }))
        .into_element()
}

fn render_albums_view(query: &str, sort_asc: bool) -> AnyElement {
    let ql = query.trim().to_lowercase();
    let mut seen = HashSet::new();
    let mut albums: Vec<(Option<String>, String, String, Option<String>)> = Vec::new();

    for sec in store::home_sections() {
        for item in sec.items {
            if let HomeItem::Collection {
                browse_id,
                title,
                subtitle,
                artwork_url,
            } = item
                && seen.insert(title.clone())
            {
                albums.push((Some(browse_id), title, subtitle, artwork_url));
            }
        }
    }

    for track in store::all_library_songs() {
        if let Some(alb) = track.album.as_ref()
            && !alb.title.is_empty()
            && seen.insert(alb.title.clone())
        {
            albums.push((
                alb.id.clone(),
                alb.title.clone(),
                format!("Album • {}", track.artist_display()),
                track.artwork_url.clone(),
            ));
        }
    }

    if !ql.is_empty() {
        albums.retain(|(_, title, sub, _)| matches_query(title, &ql) || matches_query(sub, &ql));
    }
    if sort_asc {
        albums.sort_by_key(|a| a.1.to_lowercase());
    }

    let count = albums.len();
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text("Albums")
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(pluralize(count, "album", "albums"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(albums.is_empty(), |v| {
            v.child(status_card(
                "No albums found",
                "Try another filter or search YouTube Music for albums.",
                Some("Open Search"),
                Some(Box::new(|| store::set_tab(AppTab::Search))),
            ))
        })
        .children(albums.into_iter().map(|(browse_id, title, subtitle, art)| {
            let mono = title
                .chars()
                .find(|c| c.is_alphanumeric())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "♪".into());
            let title_for_press = title.clone();
            let sub_for_press = subtitle.clone();
            let art_for_press = art.clone();

            button()
                .accessibility_label(title.clone())
                .w_full()
                .min_h(px(62.0))
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(radius_card)
                .bg(move || theme().card)
                .border_1()
                .border_color(glass_hairline)
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.8))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(move || {
                    if let Some(id) = browse_id.clone() {
                        if id.starts_with("MPRE") {
                            store::open_album_entity(id);
                        } else if id.starts_with("UC") {
                            store::open_artist_entity(id);
                        } else {
                            store::open_collection(
                                id,
                                title_for_press.clone(),
                                sub_for_press.clone(),
                                art_for_press.clone(),
                            );
                        }
                    } else {
                        store::run_search(&title_for_press);
                        store::set_tab(AppTab::Search);
                    }
                })
                .child(artwork_box(art, mono, 48.0, 8.0))
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
                            text(subtitle)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(icon(IconKind::ChevronRight, 16.0, move || {
                    theme().muted_foreground
                }))
        }))
        .into_element()
}

fn render_artists_view(query: &str, sort_asc: bool) -> AnyElement {
    let ql = query.trim().to_lowercase();
    let mut counts: HashMap<String, (usize, Option<String>)> = HashMap::new();
    for track in store::all_library_songs() {
        for artist in track.artists {
            let entry = counts.entry(artist.name).or_insert((0, None));
            entry.0 += 1;
            if entry.1.is_none() && artist.id.as_deref().is_some_and(|id| !id.is_empty()) {
                entry.1 = artist.id;
            }
        }
    }

    let mut artists: Vec<(String, usize, Option<String>)> = counts
        .into_iter()
        .map(|(name, (count, id))| (name, count, id))
        .collect();
    if !ql.is_empty() {
        artists.retain(|(name, _, _)| matches_query(name, &ql));
    }
    if sort_asc {
        artists.sort_by_key(|a| a.0.to_lowercase());
    } else {
        artists.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    }

    let count = artists.len();
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text("Artists")
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(pluralize(count, "artist", "artists"))
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .when(artists.is_empty(), |v| {
            v.child(status_card(
                "No artists in library yet",
                "Play or like tracks to populate your artist directory.",
                Some("Open Search"),
                Some(Box::new(|| store::set_tab(AppTab::Search))),
            ))
        })
        .children(artists.into_iter().map(|(name, track_count, artist_id)| {
            let mono = name
                .chars()
                .find(|c| c.is_alphanumeric())
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "♪".into());
            let name_for_search = name.clone();
            let sub = format!("{} in library", pluralize(track_count, "track", "tracks"));

            button()
                .accessibility_label(name.clone())
                .w_full()
                .min_h(px(62.0))
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(radius_card)
                .bg(move || theme().card)
                .border_1()
                .border_color(glass_hairline)
                .hover(|s| s.bg(pill_glass_hover))
                .active(|s| s.opacity(0.8))
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(move || {
                    if let Some(cid) = artist_id.clone() {
                        store::open_artist_entity(cid);
                    } else {
                        store::run_search(&name_for_search);
                        store::set_tab(AppTab::Search);
                    }
                })
                .child(artwork_box(None, mono, 46.0, 23.0))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            text(name)
                                .number_of_lines(1)
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(sub)
                                .number_of_lines(1)
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(icon(IconKind::ChevronRight, 16.0, move || {
                    theme().muted_foreground
                }))
        }))
        .into_element()
}

fn action_toolbar(search_query: Signal<String>) -> impl IntoElement {
    view()
        .w_full()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_3()
        // Left: Play + Shuffle pills (ref_5.jpg)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(hero_pill_button("Play", Some(IconKind::Play), || {
                    play_filtered_tracks(false);
                }))
                .child(glass_pill_button(
                    "Shuffle",
                    Some(IconKind::Shuffle),
                    || {
                        play_filtered_tracks(true);
                    },
                )),
        )
        // Right: Sort + "Find in this list" live search input
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(glass_pill_button(
                    move || {
                        if store::collection_sort_asc() {
                            SharedString::from("Sort A–Z")
                        } else {
                            SharedString::from("Sort")
                        }
                    },
                    Some(IconKind::Sort),
                    store::toggle_collection_sort,
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w(px(180.0))
                        .max_w(px(280.0))
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .rounded(radius_pill)
                        .bg(pill_glass_surface)
                        .border_1()
                        .border_color(move || theme().input)
                        .child(icon(IconKind::Search, 14.0, move || {
                            theme().muted_foreground
                        }))
                        .child(
                            input()
                                .test_id("library-search-input")
                                .value(move || search_query.get())
                                .on_change(move |txt: &str| {
                                    search_query.set(txt.to_owned());
                                    store::set_library_search_query(txt);
                                })
                                .on_focus(|| set_text_input_focused(true))
                                .on_blur(|| set_text_input_focused(false))
                                .placeholder("Filter this list…")
                                .placeholder_color(|| color(theme().muted_foreground))
                                .caret_color(|| color(theme().primary))
                                .flex_1()
                                .min_w(px(0.0))
                                .text_xs()
                                .text_color(move || theme().foreground),
                        )
                        .child(when(
                            move || !search_query.get().is_empty(),
                            move || {
                                button()
                                    .accessibility_label("Clear list filter")
                                    .w(px(MOBILE_MIN_TOUCH_TARGET))
                                    .h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .items_center()
                                    .justify_center()
                                    .rounded_full()
                                    .hover(|s| s.bg(pill_glass_hover))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
                                    .on_press(move || {
                                        search_query.set(String::new());
                                        store::set_library_search_query("");
                                    })
                                    .child(icon(IconKind::Close, 14.0, move || {
                                        theme().muted_foreground
                                    }))
                            },
                        )),
                ),
        )
}

pub fn library_screen() -> impl IntoElement {
    let search_sig = signal(store::library_search_query());

    scroll().flex_1().child(
        view()
            .w_full()
            .max_w(px(1200.0))
            .p_4()
            .pb_8()
            .gap_5()
            // Header ("Library" hero title + Settings shortcut gear)
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
                            .gap_0p5()
                            .child(
                                text("Library")
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(
                                text("Your saved music, playlists, and offline downloads")
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(
                        button()
                            .test_id("nav-settings-shortcut")
                            .accessibility_label("Open Settings")
                            .w(px(MOBILE_MIN_TOUCH_TARGET))
                            .h(px(MOBILE_MIN_TOUCH_TARGET))
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(pill_glass_surface)
                            .border_1()
                            .border_color(glass_hairline)
                            .hover(|s| s.bg(pill_glass_hover))
                            .active(|s| s.opacity(0.75))
                            .focus_visible(|s| {
                                s.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .on_press(|| store::set_tab(AppTab::Settings))
                            .child(icon(IconKind::Settings, 18.0, move || theme().foreground)),
                    ),
            )
            // Wrapping Filter Pills Row (all categories visible without hidden scrollbar clipping)
            .child(
                view()
                    .w_full()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .children(LibraryFilter::all().iter().copied().map(filter_pill)),
            )
            // Music Action Toolbar & Filtered Content (or full Local Library Manager)
            .child(
                when(
                    move || store::library_filter() == LibraryFilter::LocalFiles,
                    || {
                        local_files_screen(LocalFilesProps {
                            roots: store::synced_signal(store::local_media_roots),
                            tracks: store::synced_signal(store::local_media_tracks),
                            progress: store::synced_signal(store::local_media_progress),
                            reduced_motion: store::synced_signal(store::reduced_motion),
                            on_add_folder: Rc::new(store::add_local_media_folder),
                            on_remove_folder: Rc::new(store::remove_local_media_folder),
                            on_rescan: Rc::new(store::rescan_local_media),
                            on_play_track: Rc::new(store::play_local_track_queue),
                        })
                    },
                )
                .otherwise(move || {
                    view()
                        .gap_4()
                        .child(action_toolbar(search_sig))
                        .child(dynamic(move || {
                            let filter = store::library_filter();
                            let query = store::library_search_query();
                            let sort_asc = store::collection_sort_asc();
                            let current_id =
                                store::player_snapshot().current_track.map(|t| t.id);

                            match filter {
                                LibraryFilter::LikedSongs => {
                                    let tracks = store::filtered_library_tracks(
                                        LibraryFilter::LikedSongs,
                                        &query,
                                        sort_asc,
                                    );
                                    render_track_list_view(
                                        "Liked Songs",
                                        tracks,
                                        current_id,
                                        "No liked songs yet",
                                        "Tap the heart icon on Now Playing or any track to save favorites locally.",
                                    )
                                }
                                LibraryFilter::History => {
                                    let tracks = store::filtered_library_tracks(
                                        LibraryFilter::History,
                                        &query,
                                        sort_asc,
                                    );
                                    render_track_list_view(
                                        "Recently Played",
                                        tracks,
                                        current_id,
                                        "No listening history yet",
                                        "Tracks you play appear here automatically.",
                                    )
                                }
                                LibraryFilter::Songs => {
                                    let tracks = store::filtered_library_tracks(
                                        LibraryFilter::Songs,
                                        &query,
                                        sort_asc,
                                    );
                                    render_track_list_view(
                                        "Songs",
                                        tracks,
                                        current_id,
                                        "No songs in library",
                                        "Play or search tracks on YouTube Music to build your song list.",
                                    )
                                }
                                LibraryFilter::Playlists => {
                                    render_playlists_view(&query, sort_asc)
                                }
                                LibraryFilter::Albums => render_albums_view(&query, sort_asc),
                                LibraryFilter::Artists => render_artists_view(&query, sort_asc),
                                LibraryFilter::PinnedOffline => {
                                    let pinned = store::pinned_collections();
                                    if pinned.is_empty() {
                                        status_card(
                                            "No pinned collections",
                                            "Pin an album or playlist to keep it available offline.",
                                            None,
                                            None,
                                        )
                                        .into_element()
                                    } else {
                                        view()
                                            .gap_2()
                                            .children(pinned.into_iter().map(|p| {
                                                let pid = p.id.clone();
                                                let title = p.title.clone();
                                                let sub =
                                                    pluralize(p.tracks.len(), "track", "tracks");
                                                button()
                                                    .accessibility_label(title.clone())
                                                    .w_full()
                                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                    .px_3()
                                                    .py_2()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .rounded(radius_card)
                                                    .bg(move || theme().card)
                                                    .border_1()
                                                    .border_color(glass_hairline)
                                                    .focus_visible(|s| {
                                                        s.outline(px(2.0))
                                                            .outline_color(move || theme().ring)
                                                    })
                                                    .on_press(move || {
                                                        store::open_collection(
                                                            pid.clone(),
                                                            title.clone(),
                                                            "Pinned Offline".to_owned(),
                                                            None,
                                                        );
                                                    })
                                                    .child(
                                                        text(p.title)
                                                            .text_sm()
                                                            .font_weight(FontWeight::SEMIBOLD)
                                                            .text_color(move || theme().foreground),
                                                    )
                                                    .child(
                                                        text(sub)
                                                            .text_xs()
                                                            .tabular_nums()
                                                            .text_color(move || {
                                                                theme().muted_foreground
                                                            }),
                                                    )
                                            }))
                                            .into_element()
                                    }
                                }
                                LibraryFilter::Downloads => {
                                    render_downloads_view(&query, sort_asc, current_id)
                                }
                                LibraryFilter::LocalFiles => view().into_element(),
                                LibraryFilter::Podcasts => status_card(
                                    "No podcasts saved",
                                    "Browse Home or Search to discover shows and episodes.",
                                    Some("Browse Home"),
                                    Some(Box::new(|| store::set_tab(AppTab::Home))),
                                )
                                .into_element(),
                                LibraryFilter::Uploads => status_card(
                                    "No uploads available",
                                    "Sign in with your YouTube Music account in Settings or browse Home.",
                                    Some("Open Search"),
                                    Some(Box::new(|| store::set_tab(AppTab::Search))),
                                )
                                .into_element(),
                            }
                        }))
                }),
            ),
    )
}
