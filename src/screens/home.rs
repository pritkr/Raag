//! Priority Visual Surface #4: Home Feed & Playlist / Collection Detail.
//! Warm dark YouTube Music Desktop aesthetic with mood filter chips, "Listen again"
//! square cards, 3-column "Quick Picks" grid on desktop, paginated shelves, and
//! a rich Playlist / Collection hero detail view with search & sort.

use nsh_base::{input, theme};
use nsh_core::prelude::*;
use resonance_contract::{HomeItem, HomeSection, Track};

use crate::store::{self, AppTab};
use crate::ui::controls::{
    artwork_box, compact_mix_row, glass_pill_button, hero_pill_button, media_square_card,
    mood_chip, secondary_button, skeleton_row, status_card, svg_icon_button, track_row_view,
};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_ambient_warm, glass_hairline, is_desktop_layout,
    pill_glass_surface, radius_card, radius_pill,
};

const HOME_MOODS: [&str; 11] = [
    "Podcasts",
    "Feel good",
    "Romance",
    "Energize",
    "Relax",
    "Workout",
    "Commute",
    "Party",
    "Sleep",
    "Sad",
    "Focus",
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

    view().flex_1().min_h(px(0.0)).child(
        scroll().flex_1().min_h(px(0.0)).child(
            dynamic(move || {
                let Some(playlist) = collection_memo.get() else {
                    return view().flex_1().into_element();
                };
                let desktop = desktop_memo.get();
                let art_size = if desktop { 168.0 } else { 120.0 };
                let mono = playlist
                    .title
                    .chars()
                    .find(|c| c.is_alphanumeric())
                    .map(|c| c.to_uppercase().to_string())
                    .unwrap_or_else(|| "♫".into());
                let song_count = playlist.tracks.len();
                let total_mins = format_total_minutes(&playlist.tracks);
                let stats_line = match playlist.subtitle.as_deref() {
                    Some(sub) if !sub.is_empty() => {
                        format!("{sub} • {song_count} songs • {total_mins} min")
                    }
                    _ => format!("{song_count} songs • {total_mins} min"),
                };

                let tracks_for_play_all = playlist.tracks.clone();
                let tracks_for_shuffle = playlist.tracks.clone();
                let tracks_for_radio = playlist.tracks.clone();
                let radio_title = playlist.title.clone();

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
                            .child(secondary_button("← Back", store::close_collection)),
                    )
                    .child(
                        view()
                            .when(desktop, |v| v.flex_row().items_end().gap_6())
                            .when(!desktop, |v| v.gap_4())
                            .p_4()
                            .rounded(radius_card)
                            .bg(move || theme().card)
                            .border_1()
                            .border_color(glass_hairline)
                            .child(artwork_box(
                                playlist.artwork_url.clone(),
                                mono,
                                art_size,
                                12.0,
                            ))
                            .child(
                                view()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .gap_2()
                                    .child(
                                        text("PLAYLIST")
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().muted_foreground),
                                    )
                                    .child(
                                        text(playlist.title.clone())
                                            .number_of_lines(2)
                                            .text_2xl()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        text(stats_line)
                                            .number_of_lines(1)
                                            .text_sm()
                                            .text_color(move || theme().muted_foreground),
                                    )
                                    .child(
                                        view()
                                            .pt_2()
                                            .flex_row()
                                            .flex_wrap()
                                            .items_center()
                                            .gap_2()
                                            .child(hero_pill_button(
                                                "Play All",
                                                Some(IconKind::Play),
                                                move || {
                                                    if let Some(first) =
                                                        tracks_for_play_all.first().cloned()
                                                    {
                                                        store::play_track(first);
                                                        for extra in
                                                            tracks_for_play_all.iter().skip(1).cloned()
                                                        {
                                                            store::play_last(extra);
                                                        }
                                                    }
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
                                                    if let Some(first) =
                                                        tracks_for_radio.first().cloned()
                                                    {
                                                        store::play_track(first);
                                                        for extra in
                                                            tracks_for_radio.iter().skip(1).cloned()
                                                        {
                                                            store::play_last(extra);
                                                        }
                                                    }
                                                    store::set_status_banner(format!(
                                                        "Radio started from \"{}\"",
                                                        radio_title
                                                    ));
                                                },
                                            )),
                                    ),
                            ),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_3()
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
                                input()
                                    .test_id("collection-filter-input")
                                    .value(store::collection_search_query)
                                    .on_change(store::set_collection_search_query)
                                    .placeholder("Find in this list")
                                    .placeholder_color(|| color(theme().muted_foreground))
                                    .caret_color(|| color(theme().primary))
                                    .flex_1()
                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .px_3p5()
                                    .text_sm()
                                    .text_color(move || theme().foreground)
                                    .bg(pill_glass_surface)
                                    .rounded(radius_pill)
                                    .border_1()
                                    .border_color(glass_hairline)
                                    .focus(|s| s.border_color(move || theme().ring)),
                            ),
                    )
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
                                    || t.album
                                        .as_ref()
                                        .is_some_and(|a| a.title.to_lowercase().contains(&query))
                            })
                            .cloned()
                            .collect();

                        if sort_asc {
                            filtered.sort_by(|a, b| {
                                a.title.to_lowercase().cmp(&b.title.to_lowercase())
                            });
                        }

                        if filtered.is_empty() {
                            return status_card(
                                if query.is_empty() {
                                    "Playlist has no tracks"
                                } else {
                                    "No matching tracks"
                                },
                                if query.is_empty() {
                                    "Tracks will appear here once loaded."
                                } else {
                                    "Try another song title or artist name."
                                },
                                None,
                                None,
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
            }),
        ),
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
    let btn_px = if desktop { 34.0 } else { MOBILE_MIN_TOUCH_TARGET };
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
                .when_some(eyebrow, |v, sub| {
                    v.child(
                        text(sub)
                            .number_of_lines(1)
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().muted_foreground),
                    )
                })
                .child(
                    text(title)
                        .number_of_lines(1)
                        .when(desktop, |t| t.text_xl())
                        .when(!desktop, |t| t.text_lg())
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(svg_icon_button(
                    IconKind::ChevronLeft,
                    16.0,
                    btn_px,
                    "Previous page",
                    false,
                    move || theme().foreground,
                    move || store::shift_section_page(&sec_prev, -1, total_items, page_size),
                ))
                .child(svg_icon_button(
                    IconKind::ChevronRight,
                    16.0,
                    btn_px,
                    "Next page",
                    false,
                    move || theme().foreground,
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
            Some("YOUR RECENT ROTATION"),
            "Listen again".into(),
            header_id,
            total,
            page_size,
            desktop,
        ))
        .child(dynamic(move || {
            let page = store::section_page(&shelf_id);
            let slice = paginated_slice(&tracks, page, page_size);
            let active_id = current_id_memo.get();

            view()
                .w_full()
                .when(desktop, |v| v.flex_row().flex_wrap().items_start().gap_4())
                .when(!desktop, |v| v.gap_2())
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
                        164.0,
                        is_active,
                        move || store::play_track(t_open.clone()),
                        Some(Box::new(move || store::play_next(t_more.clone()))),
                    )
                }))
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
    let page_size = if desktop { 9 } else { 6 };

    view()
        .gap_3()
        .child(section_header_with_pager(
            Some("START RADIO FROM A SONG"),
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
                view()
                    .gap_1()
                    .children(slice.chunks(3).map(|row_tracks| {
                        let mut cols = Vec::with_capacity(3);
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
                        while cols.len() < 3 {
                            cols.push(view().flex_1().min_w(px(0.0)).into_element());
                        }
                        view().flex_row().gap_3().children(cols)
                    }))
                    .into_element()
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

    view()
        .gap_3()
        .child(section_header_with_pager(
            Some("MIXES & ALBUMS"),
            title,
            header_id,
            total,
            page_size,
            desktop,
        ))
        .child(
            view()
                .w_full()
                .when(desktop, |v| v.flex_row().flex_wrap().items_start().gap_4())
                .when(!desktop, |v| v.gap_2())
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

                            let card_w = 168.0;
                            if desktop {
                                media_square_card(
                                    col.artwork_url,
                                    mono,
                                    col.title,
                                    col.subtitle,
                                    card_w,
                                    false,
                                    move || {
                                        store::open_collection(
                                            id_btn.clone(),
                                            title_btn.clone(),
                                            sub_btn.clone(),
                                            art_btn.clone(),
                                        );
                                    },
                                    None,
                                )
                                .into_element()
                            } else {
                                nsh_base::button()
                                    .w_full()
                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .p_3()
                                    .rounded(radius_card)
                                    .bg(move || theme().card)
                                    .border_1()
                                    .border_color(glass_hairline)
                                    .hover(|s| s.bg(move || theme().muted))
                                    .active(|s| s.opacity(0.8))
                                    .on_press(move || {
                                        store::open_collection(
                                            id_btn.clone(),
                                            title_btn.clone(),
                                            sub_btn.clone(),
                                            art_btn.clone(),
                                        );
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
                                    .into_element()
                            }
                        })),
        )
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
        .bg_linear_to_b()
        .gradient_from(color_ambient_warm)
        .gradient_to(move || theme().background)
        .child(
            view()
                .p_3()
                .gap_3()
                .when(is_desktop_layout(), |v| v.px_8().py_5().gap_6())
                // Mood / category filter chips row
                .child(
                    scroll().horizontal().w_full().h(px(36.0)).child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .py_1()
                            .children(HOME_MOODS.into_iter().map(|mood| {
                                mood_chip(
                                    mood,
                                    move || store::selected_mood().as_deref() == Some(mood),
                                    move || store::select_mood(mood),
                                )
                            })),
                    ),
                )
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
                            move || SharedString::from(store::home_error().unwrap_or_default()),
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
                    let mut rendered_listen_again = false;

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
                            if desktop && !rendered_listen_again {
                                children.push(
                                    listen_again_shelf(
                                        section.id.clone(),
                                        tracks.clone(),
                                        current_id_memo,
                                        desktop,
                                    )
                                    .into_element(),
                                );
                                rendered_listen_again = true;
                            }
                            children.push(
                                track_section_view(section, tracks, current_id_memo, desktop)
                                    .into_element(),
                            );
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

                    view().gap_6().children(children).into_element()
                })),
        )
}
