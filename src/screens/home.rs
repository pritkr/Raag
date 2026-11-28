//! Priority Visual Surface #4: Home Feed & Playlist / Collection Detail.
//! Matches `ytm-rs` reference (`01_home_01.png`–`01_home_03.png` & `04_playlist_detail_1.png`–`_4.png`):
//! - Wrapping mood/activity filter pills over the moving blurred-glass ambient backdrop
//! - Clean section headings with `< >` carousel arrows (no redundant eyebrow clutter)
//! - Horizontal single-row square-card carousels (`Listen again`, `Your daily discover`, etc.)
//!   and 3-column compact `Song mixes` grid on desktop
//! - Rich Playlist / Collection Detail hero (`PLAYLIST`, 220px artwork, Play/Shuffle/Radio/Select,
//!   Download, Sort + Find in this list)

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::{HomeItem, HomeSection, Track};
use resonance_core::offline::PinnedCollectionKind;

use crate::store::{self, AppTab};
use crate::ui::controls::{
    artwork_box, back_button, compact_mix_row, glass_pill_button, hero_pill_button,
    media_square_card, mood_chip, mood_chip_with_icon, set_text_input_focused, skeleton_row,
    status_card, svg_icon_button, track_row_view,
};
use crate::ui::icons::{IconKind, icon};
use crate::ui::text_format::{format_recoverable_error, pluralize, quote_term};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, eyebrow_label, frosted_settings_card, glass_hairline,
    is_desktop_layout, is_narrow_viewport, pill_glass_hover, pill_glass_surface, radius_card,
    radius_pill,
};

const HOME_MOODS: [&str; 11] = [
    "Podcasts",
    "Relax",
    "Workout",
    "Energize",
    "Romance",
    "Commute",
    "Feel good",
    "Party",
    "Sad",
    "Focus",
    "Sleep",
];

pub fn home_screen() -> impl IntoElement {
    let has_collection = move || store::selected_collection().is_some();

    view()
        .flex_1()
        .min_h(px(0.0))
        .child(when(has_collection, collection_detail_view).otherwise(home_feed_view))
}

fn format_total_minutes(tracks: &[Track]) -> u64 {
    let total_ms: u64 = tracks.iter().map(|t| t.duration_ms).sum();
    (total_ms / 60_000).max(if total_ms > 0 { 1 } else { 0 })
}

fn collection_detail_view() -> impl IntoElement {
    let collection_memo = computed(store::selected_collection);
    let current_id_memo = computed(|| store::player_snapshot().current_track.map(|t| t.id));
    let desktop_memo = computed(is_desktop_layout);
    let select_mode = signal(false);

    view()
        .flex_1()
        .min_h(px(0.0))
        .child(
            scroll()
                .flex_1()
                .min_h(px(0.0))
                .pb_safe_area()
                .child(dynamic(move || {
                    let Some(playlist) = collection_memo.get() else {
                        return view().flex_1().into_element();
                    };
                    let desktop = desktop_memo.get();
                    let art_size = if desktop { 220.0 } else { 136.0 };
                    let mono = playlist
                        .title
                        .chars()
                        .find(|c| c.is_alphanumeric())
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_else(|| "♫".into());
                    let song_count = playlist.tracks.len();
                    let songs_label = pluralize(song_count, "song", "songs");
                    let total_mins = format_total_minutes(&playlist.tracks);
                    let owner_line = playlist
                        .subtitle
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(|sub| format!("{sub} • {songs_label}"))
                        .unwrap_or_else(|| songs_label.clone());
                    let stats_line = format!("{songs_label} • {total_mins} min");

                    let tracks_for_play_all = playlist.tracks.clone();
                    let tracks_for_shuffle = playlist.tracks.clone();
                    let tracks_for_radio = playlist.tracks.clone();
                    let tracks_for_download = playlist.tracks.clone();
                    let radio_title = playlist.title.clone();
                    let dl_id = playlist.id.clone();
                    let dl_title = playlist.title.clone();
                    let dl_sub = playlist.subtitle.clone();
                    let dl_art = playlist.artwork_url.clone();
                    let is_pinned = store::pinned_collections()
                        .iter()
                        .any(|p| p.id == playlist.id);
                    let pin_id = playlist.id.clone();

                    view()
                        .p_4()
                        .when(desktop, |v| v.px_8().py_6())
                        .gap_5()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap_3()
                                .child(back_button(store::close_collection)),
                        )
                        // Playlist Hero Header matching 04_playlist_detail_1.png
                        .child(
                            view()
                                .when(desktop, |v| v.flex_row().items_end().gap_7())
                                .when(!desktop, |v| v.gap_4())
                                .child(artwork_box(
                                    playlist.artwork_url.clone(),
                                    mono,
                                    art_size,
                                    14.0,
                                ))
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .gap_1p5()
                                        .child(eyebrow_label("Playlist"))
                                        .child(
                                            text(playlist.title.clone())
                                                .number_of_lines(2)
                                                .when(desktop, |t| t.text_3xl())
                                                .when(!desktop, |t| t.text_2xl())
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text(owner_line)
                                                .number_of_lines(1)
                                                .text_sm()
                                                .text_color(move || theme().muted_foreground),
                                        )
                                        .child(
                                            text(stats_line)
                                                .number_of_lines(1)
                                                .tabular_nums()
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                        )
                                        // Primary action pills row
                                        .child(
                                            view()
                                                .pt_2()
                                                .flex_row()
                                                .flex_wrap()
                                                .items_center()
                                                .gap_2()
                                                .child(hero_pill_button(
                                                    "Play",
                                                    Some(IconKind::Play),
                                                    move || {
                                                        store::play_queue(
                                                            tracks_for_play_all.clone(),
                                                            0,
                                                        );
                                                    },
                                                ))
                                                .child(glass_pill_button(
                                                    "Shuffle",
                                                    Some(IconKind::Shuffle),
                                                    move || {
                                                        store::play_all_shuffled(
                                                            tracks_for_shuffle.clone(),
                                                        );
                                                    },
                                                ))
                                                .child(glass_pill_button(
                                                    "Radio",
                                                    Some(IconKind::Radio),
                                                    move || {
                                                        store::set_autoplay_radio(true);
                                                        store::play_queue(
                                                            tracks_for_radio.clone(),
                                                            0,
                                                        );
                                                        store::set_status_banner(format!(
                                                            "Radio started from {}",
                                                            quote_term(&radio_title)
                                                        ));
                                                    },
                                                ))
                                                .child(glass_pill_button(
                                                    move || {
                                                        if select_mode.get() {
                                                            SharedString::from("Done")
                                                        } else {
                                                            SharedString::from("Select")
                                                        }
                                                    },
                                                    Some(IconKind::Check),
                                                    move || {
                                                        select_mode
                                                            .set(!select_mode.get_untracked());
                                                    },
                                                )),
                                        )
                                        // Secondary library/download pills row
                                        .child(
                                            view()
                                                .pt_1()
                                                .flex_row()
                                                .flex_wrap()
                                                .items_center()
                                                .gap_2()
                                                .child(glass_pill_button(
                                                    if is_pinned {
                                                        "Remove from library"
                                                    } else {
                                                        "Save to library"
                                                    },
                                                    None,
                                                    move || {
                                                        if is_pinned {
                                                            store::unpin_collection(&pin_id);
                                                            store::set_status_banner(
                                                                "Removed playlist from offline library",
                                                            );
                                                        } else {
                                                            store::pin_collection(
                                                                dl_id.clone(),
                                                                PinnedCollectionKind::Playlist,
                                                                dl_title.clone(),
                                                                dl_sub.clone(),
                                                                dl_art.clone(),
                                                                tracks_for_download.clone(),
                                                            );
                                                            store::set_status_banner(
                                                                "Saved playlist to library",
                                                            );
                                                        }
                                                    },
                                                ))
                                                .child(glass_pill_button(
                                                    "Download",
                                                    Some(IconKind::Download),
                                                    {
                                                        let tracks_dl = playlist.tracks.clone();
                                                        move || {
                                                            for t in tracks_dl.iter().cloned() {
                                                                store::enqueue_download(t);
                                                            }
                                                            store::set_status_banner(
                                                                "Queued playlist tracks for offline download",
                                                            );
                                                        }
                                                    },
                                                ))
                                                .child(svg_icon_button(
                                                    IconKind::MoreVertical,
                                                    16.0,
                                                    if desktop {
                                                        38.0
                                                    } else {
                                                        MOBILE_MIN_TOUCH_TARGET
                                                    },
                                                    "Playlist options",
                                                    true,
                                                    move || theme().foreground,
                                                    {
                                                        let title_copy = playlist.title.clone();
                                                        move || {
                                                            store::copy_text_to_clipboard(
                                                                "playlist",
                                                                &title_copy,
                                                            );
                                                        }
                                                    },
                                                )),
                                        )
                                        // Sort + Find in this list row
                                        .child(
                                            view()
                                                .pt_2()
                                                .flex_row()
                                                .flex_wrap()
                                                .items_center()
                                                .gap_2()
                                                .child(glass_pill_button(
                                                    move || {
                                                        if store::collection_sort_asc() {
                                                            SharedString::from("Sort (A–Z)")
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
                                                        .max_w(px(300.0))
                                                        .min_h(px(if desktop {
                                                            38.0
                                                        } else {
                                                            MOBILE_MIN_TOUCH_TARGET
                                                        }))
                                                        .flex_row()
                                                        .items_center()
                                                        .gap_2()
                                                        .px_3p5()
                                                        .rounded(radius_pill)
                                                        .bg(pill_glass_surface)
                                                        .border_1()
                                                        .border_color(glass_hairline)
                                                        .child(icon(
                                                            IconKind::Search,
                                                            14.0,
                                                            move || theme().muted_foreground,
                                                        ))
                                                        .child(
                                                            input()
                                                                .test_id("collection-filter-input")
                                                                .value(
                                                                    store::collection_search_query,
                                                                )
                                                                .on_change(
                                                                    store::set_collection_search_query,
                                                                )
                                                                .on_focus(|| {
                                                                    set_text_input_focused(true)
                                                                })
                                                                .on_blur(|| {
                                                                    set_text_input_focused(false)
                                                                })
                                                                .placeholder("Find in this list")
                                                                .placeholder_color(|| {
                                                                    color(
                                                                        theme().muted_foreground,
                                                                    )
                                                                })
                                                                .caret_color(|| {
                                                                    color(theme().primary)
                                                                })
                                                                .flex_1()
                                                                .min_w(px(0.0))
                                                                .text_xs()
                                                                .text_color(move || {
                                                                    theme().foreground
                                                                })
                                                                .border_color(move || {
                                                                    theme().input
                                                                }),
                                                        ),
                                                ),
                                        ),
                                ),
                        )
                        // Track list
                        .child(dynamic(move || {
                            let query = store::collection_search_query().trim().to_lowercase();
                            let sort_asc = store::collection_sort_asc();
                            let active_id = current_id_memo.get();
                            let mut filtered: Vec<Track> = playlist
                                .tracks
                                .iter()
                                .filter(|t| {
                                    query.is_empty()
                                        || t.title.to_lowercase().contains(&query)
                                        || t.artist_display().to_lowercase().contains(&query)
                                        || t.album.as_ref().is_some_and(|a| {
                                            a.title.to_lowercase().contains(&query)
                                        })
                                })
                                .cloned()
                                .collect();

                            if sort_asc {
                                filtered.sort_by(|a, b| {
                                    a.title.to_lowercase().cmp(&b.title.to_lowercase())
                                });
                            }

                            if filtered.is_empty() {
                                if query.is_empty() {
                                    return status_card(
                                        "Playlist has no tracks",
                                        "Tracks will appear here once loaded.",
                                        Some("Back to Home"),
                                        Some(Box::new(store::close_collection)),
                                    )
                                    .into_element();
                                }
                                return status_card(
                                    SharedString::from(format!(
                                        "No tracks match {}",
                                        quote_term(&query)
                                    )),
                                    "Try another song title or artist name, or clear the filter.",
                                    Some("Clear filter"),
                                    Some(Box::new(|| store::set_collection_search_query(""))),
                                )
                                .into_element();
                            }

                            view()
                                .gap_1()
                                .children(filtered.into_iter().map(move |track| {
                                    let is_cur = active_id.as_deref() == Some(track.id.as_str());
                                    let t_play = track.clone();
                                    let t_next = track.clone();
                                    track_row_view(
                                        track,
                                        is_cur,
                                        move || store::play_track(t_play.clone()),
                                        move || store::play_next(t_next.clone()),
                                    )
                                }))
                                .into_element()
                        }))
                        .into_element()
                })),
        )
}

fn section_header_with_pager(
    eyebrow: Option<&'static str>,
    title: String,
    section_id: String,
    total_items: usize,
    page_size: usize,
    desktop: bool,
) -> impl IntoElement {
    let btn_px = if desktop {
        32.0
    } else {
        MOBILE_MIN_TOUCH_TARGET
    };
    let sec_prev = section_id.clone();
    let sec_next = section_id;

    view()
        .flex_row()
        .items_end()
        .justify_between()
        .gap_3()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .when_some(eyebrow, |v, sub| v.child(eyebrow_label(sub)))
                .child(
                    text(title)
                        .number_of_lines(1)
                        .when(desktop, |t| t.text_2xl())
                        .when(!desktop, |t| t.text_lg())
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(svg_icon_button(
                    IconKind::ChevronLeft,
                    15.0,
                    btn_px,
                    "Previous page",
                    false,
                    move || theme().muted_foreground,
                    move || store::shift_section_page(&sec_prev, -1, total_items, page_size),
                ))
                .child(svg_icon_button(
                    IconKind::ChevronRight,
                    15.0,
                    btn_px,
                    "Next page",
                    false,
                    move || theme().muted_foreground,
                    move || store::shift_section_page(&sec_next, 1, total_items, page_size),
                )),
        )
}

fn paginated_slice<T: Clone>(items: &[T], page: usize, page_size: usize) -> Vec<T> {
    if items.is_empty() || page_size == 0 {
        return Vec::new();
    }
    let start = page.saturating_mul(page_size);
    if start >= items.len() {
        let last_page = (items.len() - 1) / page_size;
        let fallback_start = last_page * page_size;
        return items[fallback_start..].to_vec();
    }
    let end = (start + page_size).min(items.len());
    items[start..end].to_vec()
}

fn listen_again_shelf(
    section_id: String,
    title: String,
    tracks: Vec<Track>,
    current_id_memo: Memo<Option<String>>,
    desktop: bool,
) -> impl IntoElement {
    let shelf_id = format!("{section_id}-listen-again");
    let total = tracks.len();
    let page_size = 8;
    let header_id = shelf_id.clone();

    view()
        .gap_3()
        .child(section_header_with_pager(
            None, title, header_id, total, page_size, desktop,
        ))
        .child(dynamic(move || {
            let page = store::section_page(&shelf_id);
            let slice = paginated_slice(&tracks, page, page_size);
            let active_id = current_id_memo.get();
            let card_w = if is_narrow_viewport() { 148.0 } else { 172.0 };

            scroll()
                .horizontal()
                .w_full()
                .child(
                    view()
                        .flex_row()
                        .items_start()
                        .gap_4()
                        .py_1()
                        .children(slice.into_iter().map(move |track| {
                            let is_active = active_id.as_deref() == Some(track.id.as_str());
                            let t_open = track.clone();
                            let t_more = track.clone();
                            let mono = track.fallback_monogram();
                            let art = track.artwork_url.clone();
                            let title = track.title.clone();
                            let sub = track.artist_display();

                            media_square_card(
                                art,
                                mono,
                                title,
                                sub,
                                card_w,
                                is_active,
                                move || store::play_track(t_open.clone()),
                                Some(Box::new(move || store::play_next(t_more.clone()))),
                            )
                        })),
                )
                .into_element()
        }))
}

fn track_section_view(
    section: HomeSection,
    tracks: Vec<Track>,
    current_id_memo: Memo<Option<String>>,
    desktop: bool,
) -> impl IntoElement {
    let sec_id = section.id.clone();
    let total = tracks.len();
    let page_size = if desktop { 12 } else { 6 };

    view()
        .gap_3()
        .child(section_header_with_pager(
            None,
            section.title,
            sec_id.clone(),
            total,
            page_size,
            desktop,
        ))
        .child(dynamic(move || {
            let page = store::section_page(&sec_id);
            let slice = paginated_slice(&tracks, page, page_size);
            let active_id = current_id_memo.get();

            if desktop {
                let win_w = window().width.0;
                let sidebar_w = if store::sidebar_collapsed() {
                    76.0
                } else {
                    240.0
                };
                let panel_w = if store::desktop_side_panel_open() && win_w >= 1120.0 {
                    336.0
                } else {
                    0.0
                };
                let center_w = (win_w - sidebar_w - panel_w - 64.0).max(0.0);
                let col_count: usize = if center_w >= 860.0 {
                    3
                } else if center_w >= 580.0 {
                    2
                } else {
                    1
                };

                if col_count == 1 {
                    view()
                        .gap_1()
                        .children(slice.into_iter().map(move |track| {
                            let is_cur = active_id.as_deref() == Some(track.id.as_str());
                            let t_play = track.clone();
                            let t_next = track.clone();
                            compact_mix_row(
                                track,
                                is_cur,
                                move || store::play_track(t_play.clone()),
                                move || store::play_next(t_next.clone()),
                            )
                        }))
                        .into_element()
                } else {
                    view()
                        .gap_1()
                        .children(slice.chunks(col_count).map(|row_tracks| {
                            let mut cols = Vec::with_capacity(col_count);
                            for track in row_tracks {
                                let is_cur = active_id.as_deref() == Some(track.id.as_str());
                                let t_play = track.clone();
                                let t_next = track.clone();
                                cols.push(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .child(compact_mix_row(
                                            track.clone(),
                                            is_cur,
                                            move || store::play_track(t_play.clone()),
                                            move || store::play_next(t_next.clone()),
                                        ))
                                        .into_element(),
                                );
                            }
                            while cols.len() < col_count {
                                cols.push(view().flex_1().min_w(px(0.0)).into_element());
                            }
                            view().flex_row().gap_4().children(cols)
                        }))
                        .into_element()
                }
            } else {
                view()
                    .gap_1()
                    .children(slice.into_iter().map(move |track| {
                        let is_cur = active_id.as_deref() == Some(track.id.as_str());
                        let t_play = track.clone();
                        let t_next = track.clone();
                        track_row_view(
                            track,
                            is_cur,
                            move || store::play_track(t_play.clone()),
                            move || store::play_next(t_next.clone()),
                        )
                    }))
                    .into_element()
            }
        }))
}

#[derive(Clone)]
struct CollectionCardItem {
    browse_id: String,
    title: String,
    subtitle: String,
    artwork_url: Option<String>,
}

fn collection_section_view(
    section_id: String,
    title: String,
    collections: Vec<CollectionCardItem>,
    desktop: bool,
) -> impl IntoElement {
    let total = collections.len();
    let page_size = 8;
    let header_id = section_id.clone();

    let page_memo = computed({
        let id = section_id.clone();
        move || store::section_page(&id)
    });
    let page = page_memo.get();
    let slice = paginated_slice(&collections, page, page_size);
    let card_w = if is_narrow_viewport() { 148.0 } else { 172.0 };

    view()
        .gap_3()
        .child(section_header_with_pager(
            None, title, header_id, total, page_size, desktop,
        ))
        .child(if desktop {
            scroll()
                .horizontal()
                .w_full()
                .child(
                    view()
                        .flex_row()
                        .items_start()
                        .gap_4()
                        .py_1()
                        .children(slice.into_iter().map(move |col| {
                            let mono = col
                                .title
                                .chars()
                                .find(|c| c.is_alphanumeric())
                                .map(|c| c.to_uppercase().to_string())
                                .unwrap_or_else(|| "♫".into());
                            let id_btn = col.browse_id.clone();
                            let title_btn = col.title.clone();
                            let sub_btn = col.subtitle.clone();
                            let art_btn = col.artwork_url.clone();

                            media_square_card(
                                col.artwork_url,
                                mono,
                                col.title,
                                col.subtitle,
                                card_w,
                                false,
                                move || {
                                    if id_btn.starts_with("MPRE") {
                                        store::open_album_entity(id_btn.clone());
                                    } else {
                                        store::open_collection(
                                            id_btn.clone(),
                                            title_btn.clone(),
                                            sub_btn.clone(),
                                            art_btn.clone(),
                                        );
                                    }
                                },
                                None,
                            )
                        })),
                )
                .into_element()
        } else {
            view()
                .w_full()
                .gap_2()
                .py_1()
                .children(slice.into_iter().map(|col| {
                    let mono = col
                        .title
                        .chars()
                        .find(|c| c.is_alphanumeric())
                        .map(|c| c.to_uppercase().to_string())
                        .unwrap_or_else(|| "♫".into());
                    let id_btn = col.browse_id.clone();
                    let title_btn = col.title.clone();
                    let sub_btn = col.subtitle.clone();
                    let art_btn = col.artwork_url.clone();

                    button()
                        .w_full()
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .p_3()
                        .rounded(radius_card)
                        .bg(frosted_settings_card)
                        .border_1()
                        .border_color(glass_hairline)
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| s.opacity(0.8))
                        .focus_visible(|s| {
                            s.outline(px(2.0)).outline_color(move || theme().ring)
                        })
                        .on_press(move || {
                            if id_btn.starts_with("MPRE") {
                                store::open_album_entity(id_btn.clone());
                            } else {
                                store::open_collection(
                                    id_btn.clone(),
                                    title_btn.clone(),
                                    sub_btn.clone(),
                                    art_btn.clone(),
                                );
                            }
                        })
                        .child(artwork_box(col.artwork_url, mono, 44.0, 8.0))
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
                                    text(col.subtitle)
                                        .number_of_lines(1)
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                }))
                .into_element()
        })
}

fn home_feed_view() -> impl IntoElement {
    let sections_memo = computed(store::home_sections);
    let desktop_memo = computed(is_desktop_layout);
    let current_id_memo = computed(|| store::player_snapshot().current_track.map(|t| t.id));
    let loading_memo = computed(store::is_home_loading);
    let error_memo = computed(store::home_error);

    scroll()
        .flex_1()
        .min_h(px(0.0))
        .pb_safe_area()
        .child(
            view()
                .p_3()
                .gap_4()
                .style_when(is_desktop_layout, |s| s.px_8().py_5().gap_7())
                // Wrapping mood/activity filter pills row matching 01_home_01.png
                .child(dynamic(|| {
                    let live_chips = store::home_mood_chips();
                    let active = store::selected_mood();
                    let chips: Vec<(String, Option<String>)> = if !live_chips.is_empty() {
                        live_chips
                            .into_iter()
                            .map(|c| (c.title, Some(c.params)))
                            .collect()
                    } else {
                        HOME_MOODS
                            .into_iter()
                            .map(|m| (m.to_owned(), None))
                            .collect()
                    };

                    let mut row = view()
                        .w_full()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .py_1();
                    if active.is_some() {
                        row = row.child(mood_chip_with_icon(
                            Some(IconKind::Close),
                            "Clear",
                            || false,
                            store::clear_home_mood,
                        ));
                    }
                    row.children(chips.into_iter().map(|(label, params)| {
                        let is_selected = active.as_deref() == Some(label.as_str());
                        let label_click = label.clone();
                        mood_chip(
                            SharedString::from(label),
                            move || is_selected,
                            move || {
                                store::select_home_mood(&label_click, params.as_deref());
                            },
                        )
                    }))
                    .into_element()
                }))
                // Loading skeletons
                .child(when(store::is_home_loading, || {
                    view()
                        .gap_2()
                        .child(skeleton_row())
                        .child(skeleton_row())
                        .child(skeleton_row())
                        .child(skeleton_row())
                }))
                // Error card
                .child(when(
                    || store::home_error().is_some(),
                    || {
                        status_card(
                            "Unable to reach YouTube Music",
                            move || {
                                SharedString::from(format_recoverable_error(
                                    "Could not load YouTube Music home feed",
                                    &store::home_error().unwrap_or_default(),
                                    "Check your internet connection and select Retry.",
                                ))
                            },
                            Some("Retry"),
                            Some(Box::new(store::refresh_home)),
                        )
                    },
                ))
                // Reactive Home sections
                .child(dynamic(move || {
                    let sections = sections_memo.get();
                    let desktop = desktop_memo.get();
                    let loading = loading_memo.get();
                    let has_error = error_memo.get().is_some();

                    if sections.is_empty() && !loading && !has_error {
                        return status_card(
                            "Home feed is empty",
                            "Search YouTube Music or reconnect to load recommendations.",
                            Some("Open Search"),
                            Some(Box::new(|| store::set_tab(AppTab::Search))),
                        )
                        .into_element();
                    }

                    let mut children = Vec::new();

                    for section in sections {
                        let mut tracks = Vec::new();
                        let mut collections = Vec::new();

                        for item in &section.items {
                            match item {
                                HomeItem::Track(t) => tracks.push(t.clone()),
                                HomeItem::Collection {
                                    browse_id,
                                    title,
                                    subtitle,
                                    artwork_url,
                                } => collections.push(CollectionCardItem {
                                    browse_id: browse_id.clone(),
                                    title: title.clone(),
                                    subtitle: subtitle.clone(),
                                    artwork_url: artwork_url.clone(),
                                }),
                            }
                        }

                        if !tracks.is_empty() && tracks.len() >= collections.len() {
                            let lower = section.title.to_lowercase();
                            if lower.contains("listen again") || lower.contains("recent") {
                                children.push(
                                    listen_again_shelf(
                                        section.id,
                                        section.title,
                                        tracks,
                                        current_id_memo,
                                        desktop,
                                    )
                                    .into_element(),
                                );
                            } else {
                                children.push(
                                    track_section_view(section, tracks, current_id_memo, desktop)
                                        .into_element(),
                                );
                            }
                        } else if !collections.is_empty() {
                            children.push(
                                collection_section_view(
                                    section.id,
                                    section.title,
                                    collections,
                                    desktop,
                                )
                                .into_element(),
                            );
                        }
                    }

                    view().gap_7().children(children).into_element()
                })),
        )
}
