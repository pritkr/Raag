//! Explore & Discovery Screen matching `ytm-rs` (`02_explore_01.png` – `02_explore_02.png`):
//! - Transparent over root moving blurred-glass ambient backdrop
//! - Bold `Explore` header
//! - `New releases for you` horizontal square-card carousel with `< >` arrows
//! - `Charts` horizontal square-card carousel with `< >` arrows
//! - `Moods & genres` 4-column translucent rounded tiles with vibrant 5px left vertical color stripes

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{HomeItem, Track};

use crate::store;
use crate::ui::controls::{
    compact_mix_row, media_square_card, secondary_button, skeleton_row, status_card,
    svg_icon_button, track_row_view,
};
use crate::ui::icons::IconKind;
use crate::ui::text_format::format_recoverable_error;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_settings_card, genre_stripe_color, glass_hairline,
    is_desktop_layout, is_narrow_viewport, pill_glass_hover, radius_card,
};

const FALLBACK_MOODS_AND_GENRES: [&str; 16] = [
    "Chill",
    "Workout",
    "Focus",
    "Party",
    "Romance",
    "Feel good",
    "Indian Pop",
    "Bollywood",
    "Acoustic",
    "Synthwave",
    "Jazz",
    "Lo-Fi",
    "Commute",
    "Energize",
    "Sad",
    "Sleep",
];

#[derive(Clone)]
enum ReleaseCardKind {
    Track(Track),
    Collection {
        browse_id: String,
        title: String,
        subtitle: String,
        artwork_url: Option<String>,
    },
}

fn build_new_releases() -> Vec<ReleaseCardKind> {
    let mut items = Vec::new();
    if let Some(page) = store::explore_page() {
        for alb in page.new_releases {
            items.push(ReleaseCardKind::Collection {
                browse_id: alb.id,
                title: alb.title,
                subtitle: alb.subtitle.unwrap_or_default(),
                artwork_url: alb.artwork_url,
            });
        }
    }
    if items.is_empty() {
        for section in store::explore_sections() {
            for item in section.items {
                match item {
                    HomeItem::Collection {
                        browse_id,
                        title,
                        subtitle,
                        artwork_url,
                    } => items.push(ReleaseCardKind::Collection {
                        browse_id,
                        title,
                        subtitle,
                        artwork_url,
                    }),
                    HomeItem::Track(track) => items.push(ReleaseCardKind::Track(track)),
                }
            }
        }
    }
    if items.is_empty() {
        // Fixture fallback from Home collections/tracks so Explore looks rich immediately
        for section in store::home_sections() {
            for item in section.items {
                if let HomeItem::Collection {
                    browse_id,
                    title,
                    subtitle,
                    artwork_url,
                } = item
                {
                    items.push(ReleaseCardKind::Collection {
                        browse_id,
                        title,
                        subtitle,
                        artwork_url,
                    });
                }
            }
        }
    }
    items
}

fn fixture_chart_cards() -> Vec<(String, String, String, Option<String>)> {
    vec![
        (
            "VLPLchart_trending20".into(),
            "Trending 20 India".into(),
            "Chart • Updated daily".into(),
            None,
        ),
        (
            "VLPLchart_top100".into(),
            "Top 100 Songs Global".into(),
            "Chart • Weekly top tracks".into(),
            None,
        ),
        (
            "VLPLchart_musicvideos".into(),
            "Daily Top Music Videos".into(),
            "Chart • Most watched today".into(),
            None,
        ),
        (
            "VLPLchart_regional".into(),
            "Top Weekly Songs".into(),
            "Chart • Regional hits".into(),
            None,
        ),
        (
            "VLPLchart_indie".into(),
            "Rising Indie Spotlight".into(),
            "Chart • Fresh voices".into(),
            None,
        ),
    ]
}

fn paginated_slice<T: Clone>(items: &[T], page: usize, page_size: usize) -> Vec<T> {
    if items.is_empty() || page_size == 0 {
        return Vec::new();
    }
    let start = page.saturating_mul(page_size);
    if start >= items.len() {
        let last_page = (items.len() - 1) / page_size;
        return items[(last_page * page_size)..].to_vec();
    }
    let end = (start + page_size).min(items.len());
    items[start..end].to_vec()
}

fn explore_section_header(
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
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text(title)
                .flex_1()
                .min_w(px(0.0))
                .number_of_lines(1)
                .when(desktop, |t| t.text_2xl())
                .when(!desktop, |t| t.text_lg())
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
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

fn open_explore_collection(
    browse_id: String,
    title: String,
    subtitle: String,
    artwork_url: Option<String>,
) {
    if browse_id.starts_with("MPRE") {
        store::open_album_entity(browse_id);
    } else {
        store::open_collection(browse_id, title, subtitle, artwork_url);
    }
}

fn new_releases_section(current_id_memo: Memo<Option<String>>, desktop: bool) -> impl IntoElement {
    const SECTION_ID: &str = "explore-new-releases";
    const PAGE_SIZE: usize = 8;
    let page_memo = computed(store::explore_page);
    let explore_memo = computed(store::explore_sections);

    view().gap_3().child(dynamic(move || {
        let _ = page_memo.get();
        let _ = explore_memo.get();
        let all_releases = build_new_releases();
        if all_releases.is_empty() {
            return view().into_element();
        }
        let total = all_releases.len();
        let page = store::section_page(SECTION_ID);
        let slice = paginated_slice(&all_releases, page, PAGE_SIZE);
        let active_id = current_id_memo.get();
        let card_w = if is_narrow_viewport() { 148.0 } else { 172.0 };

        view()
            .gap_3()
            .child(explore_section_header(
                "New releases for you".to_owned(),
                SECTION_ID.to_owned(),
                total,
                PAGE_SIZE,
                desktop,
            ))
            .child(
                scroll().horizontal().w_full().child(
                    view()
                        .flex_row()
                        .items_start()
                        .gap_4()
                        .py_1()
                        .children(slice.into_iter().map(move |entry| match entry {
                            ReleaseCardKind::Track(track) => {
                                let is_active = active_id.as_deref() == Some(track.id.as_str());
                                let t_open = track.clone();
                                let t_next = track.clone();
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
                                    Some(Box::new(move || store::play_next(t_next.clone()))),
                                )
                                .into_element()
                            }
                            ReleaseCardKind::Collection {
                                browse_id,
                                title,
                                subtitle,
                                artwork_url,
                            } => {
                                let mono = title
                                    .chars()
                                    .find(|c| c.is_alphanumeric())
                                    .map(|c| c.to_uppercase().to_string())
                                    .unwrap_or_else(|| "♫".into());
                                let b_id = browse_id.clone();
                                let b_title = title.clone();
                                let b_sub = subtitle.clone();
                                let b_art = artwork_url.clone();
                                media_square_card(
                                    artwork_url,
                                    mono,
                                    title,
                                    subtitle,
                                    card_w,
                                    false,
                                    move || {
                                        open_explore_collection(
                                            b_id.clone(),
                                            b_title.clone(),
                                            b_sub.clone(),
                                            b_art.clone(),
                                        );
                                    },
                                    None,
                                )
                                .into_element()
                            }
                        })),
                ),
            )
            .into_element()
    }))
}

fn charts_section(current_id_memo: Memo<Option<String>>, desktop: bool) -> impl IntoElement {
    let page_memo = computed(store::explore_page);
    let fallback_memo = computed(store::explore_sections);

    view().gap_5().child(dynamic(move || {
        let live_charts = page_memo
            .get()
            .map(|p| p.charts)
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| fallback_memo.get());

        if live_charts.is_empty() {
            // Render rich fixture Charts carousel matching 02_explore_01.png
            let charts = fixture_chart_cards();
            let card_w = if is_narrow_viewport() { 148.0 } else { 172.0 };
            let total = charts.len();
            return view()
                .gap_3()
                .child(explore_section_header(
                    "Charts".to_owned(),
                    "explore-charts-fixture".to_owned(),
                    total,
                    8,
                    desktop,
                ))
                .child(scroll().horizontal().w_full().child(
                    view().flex_row().items_start().gap_4().py_1().children(
                        charts.into_iter().map(move |(id, title, sub, art)| {
                            let mono = title
                                .chars()
                                .find(|c| c.is_alphanumeric())
                                .map(|c| c.to_uppercase().to_string())
                                .unwrap_or_else(|| "♫".into());
                            let id_c = id.clone();
                            let title_c = title.clone();
                            let sub_c = sub.clone();
                            let art_c = art.clone();
                            media_square_card(
                                art,
                                mono,
                                title,
                                sub,
                                card_w,
                                false,
                                move || {
                                    open_explore_collection(
                                        id_c.clone(),
                                        title_c.clone(),
                                        sub_c.clone(),
                                        art_c.clone(),
                                    );
                                },
                                None,
                            )
                        }),
                    ),
                ))
                .into_element();
        }

        let active_id = current_id_memo.get();
        let mut rendered = Vec::new();

        for (sec_idx, section) in live_charts.into_iter().enumerate() {
            let mut tracks = Vec::new();
            let mut collections = Vec::new();
            for item in section.items {
                match item {
                    HomeItem::Track(t) => tracks.push(t),
                    HomeItem::Collection {
                        browse_id,
                        title,
                        subtitle,
                        artwork_url,
                    } => collections.push((browse_id, title, subtitle, artwork_url)),
                }
            }

            let sec_key = format!("explore-chart-{sec_idx}-{}", section.id);
            if !collections.is_empty() && collections.len() >= tracks.len() {
                let page_size = 8;
                let page = store::section_page(&sec_key);
                let slice = paginated_slice(&collections, page, page_size);
                let card_w = if is_narrow_viewport() { 148.0 } else { 172.0 };
                rendered.push(
                    view()
                        .gap_3()
                        .child(explore_section_header(
                            if sec_idx == 0 {
                                "Charts".to_owned()
                            } else {
                                section.title
                            },
                            sec_key,
                            collections.len(),
                            page_size,
                            desktop,
                        ))
                        .child(scroll().horizontal().w_full().child(
                            view().flex_row().items_start().gap_4().py_1().children(
                                slice.into_iter().map(move |(id, title, sub, art)| {
                                    let mono = title
                                        .chars()
                                        .find(|c| c.is_alphanumeric())
                                        .map(|c| c.to_uppercase().to_string())
                                        .unwrap_or_else(|| "♫".into());
                                    let id_c = id.clone();
                                    let title_c = title.clone();
                                    let sub_c = sub.clone();
                                    let art_c = art.clone();
                                    media_square_card(
                                        art,
                                        mono,
                                        title,
                                        sub,
                                        card_w,
                                        false,
                                        move || {
                                            open_explore_collection(
                                                id_c.clone(),
                                                title_c.clone(),
                                                sub_c.clone(),
                                                art_c.clone(),
                                            );
                                        },
                                        None,
                                    )
                                }),
                            ),
                        ))
                        .into_element(),
                );
            } else if !tracks.is_empty() {
                let page_size = if desktop { 9 } else { 6 };
                let page = store::section_page(&sec_key);
                let slice = paginated_slice(&tracks, page, page_size);
                let active_id_sec = active_id.clone();

                let body = if desktop {
                    view()
                        .gap_1()
                        .children(slice.chunks(3).map(|row_tracks| {
                            let mut cols = Vec::with_capacity(3);
                            for track in row_tracks {
                                let is_cur = active_id_sec.as_deref() == Some(track.id.as_str());
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
                            let is_cur = active_id_sec.as_deref() == Some(track.id.as_str());
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
                };

                rendered.push(
                    view()
                        .gap_3()
                        .child(explore_section_header(
                            if sec_idx == 0 {
                                "Charts".to_owned()
                            } else {
                                section.title
                            },
                            sec_key,
                            tracks.len(),
                            page_size,
                            desktop,
                        ))
                        .child(body)
                        .into_element(),
                );
            }
        }

        view().gap_7().children(rendered).into_element()
    }))
}

fn moods_and_genres_section(desktop: bool) -> impl IntoElement {
    let cols_count = if desktop { 4 } else { 2 };
    let explore_memo = computed(store::explore_page);
    let mood_playlists_memo = computed(store::mood_playlists);

    view().gap_3().child(dynamic(move || {
        let page = explore_memo.get();
        let live_moods: Vec<(String, Option<String>)> = page
            .as_ref()
            .map(|p| {
                p.moods_and_genres
                    .iter()
                    .map(|m| (m.title.clone(), Some(m.params.clone())))
                    .collect()
            })
            .unwrap_or_default();

        let cards: Vec<(String, Option<String>)> = if !live_moods.is_empty() {
            live_moods
        } else {
            FALLBACK_MOODS_AND_GENRES
                .iter()
                .map(|g| ((*g).to_owned(), None))
                .collect()
        };

        let playlists = mood_playlists_memo.get();
        let active_mood = store::selected_mood();

        let active_mood_cards = active_mood.clone();
        let mut root = view()
            .gap_3()
            .child(
                text("Moods & genres")
                    .when(desktop, |t| t.text_2xl())
                    .when(!desktop, |t| t.text_lg())
                    .font_weight(FontWeight::BOLD)
                    .text_color(move || theme().foreground),
            )
            .child(
                view()
                    .gap_2p5()
                    .children(cards.chunks(cols_count).enumerate().map(
                        move |(row_idx, row_genres)| {
                            let mut cells = Vec::with_capacity(cols_count);
                            for (col_idx, (genre, params)) in row_genres.iter().enumerate() {
                                let idx = row_idx * cols_count + col_idx;
                                let genre_label = genre.clone();
                                let genre_click = genre.clone();
                                let params_click = params.clone();
                                let is_selected =
                                    active_mood_cards.as_deref() == Some(genre.as_str());
                                cells.push(
                                    button()
                                        .accessibility_label(genre_label.clone())
                                        .selected(move || is_selected)
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .min_h(px(52.0))
                                        .flex_row()
                                        .items_center()
                                        .gap_3()
                                        .pr_3()
                                        .overflow_hidden()
                                        .rounded(radius_card)
                                        .bg(frosted_settings_card)
                                        .border_1()
                                        .border_color(move || {
                                            if is_selected {
                                                theme().primary
                                            } else {
                                                glass_hairline()
                                            }
                                        })
                                        .hover(|s| s.bg(pill_glass_hover))
                                        .active(|s| s.opacity(0.76))
                                        .focus_visible(|s| {
                                            s.outline(px(2.0)).outline_color(move || theme().ring)
                                        })
                                        .on_press(move || {
                                            if let Some(ref p) = params_click {
                                                store::load_mood_playlists(&genre_click, p);
                                            } else {
                                                store::load_mood_playlists(&genre_click, "");
                                            }
                                        })
                                        .child(
                                            view()
                                                .w(px(5.0))
                                                .h_full()
                                                .bg(move || genre_stripe_color(idx)),
                                        )
                                        .child(
                                            text(genre_label)
                                                .flex_1()
                                                .number_of_lines(1)
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || theme().foreground),
                                        )
                                        .into_element(),
                                );
                            }
                            while cells.len() < cols_count {
                                cells.push(view().flex_1().min_w(px(0.0)).into_element());
                            }
                            view().flex_row().gap_2p5().children(cells)
                        },
                    )),
            );

        if !playlists.is_empty() {
            let heading = active_mood
                .map(|m| format!("{m} playlists"))
                .unwrap_or_else(|| "Mood playlists".to_owned());
            root = root.child(
                view()
                    .gap_2()
                    .pt_2()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                text(heading)
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(secondary_button("Clear", store::clear_mood_playlists)),
                    )
                    .child(scroll().horizontal().w_full().child({
                        let card_w = if is_narrow_viewport() { 148.0 } else { 172.0 };
                        view().flex_row().items_start().gap_4().py_1().children(
                            playlists.into_iter().map(move |pl| {
                                let mono = pl
                                    .title
                                    .chars()
                                    .find(|c| c.is_alphanumeric())
                                    .map(|c| c.to_uppercase().to_string())
                                    .unwrap_or_else(|| "♫".into());
                                let id = pl.browse_id.clone();
                                let title = pl.title.clone();
                                let sub = pl.subtitle.clone();
                                let art = pl.artwork_url.clone();
                                media_square_card(
                                    pl.artwork_url,
                                    mono,
                                    pl.title,
                                    pl.subtitle,
                                    card_w,
                                    false,
                                    move || {
                                        open_explore_collection(
                                            id.clone(),
                                            title.clone(),
                                            sub.clone(),
                                            art.clone(),
                                        );
                                    },
                                    None,
                                )
                            }),
                        )
                    })),
            );
        }

        root.into_element()
    }))
}

pub fn explore_screen() -> impl IntoElement {
    let desktop_memo = computed(is_desktop_layout);
    let current_id_memo = computed(|| store::player_snapshot().current_track.map(|t| t.id));

    view().flex_1().min_h(px(0.0)).child(
        scroll()
            .flex_1()
            .min_h(px(0.0))
            .pb_safe_area()
            .child(dynamic(move || {
                let desktop = desktop_memo.get();

                view()
                    .p_4()
                    .when(desktop, |v| v.px_8().py_6())
                    .gap_7()
                    // Bold Explore Header (matches 02_explore_01.png)
                    .child(
                        text("Explore")
                            .when(desktop, |t| t.text_3xl())
                            .when(!desktop, |t| t.text_2xl())
                            .font_weight(FontWeight::BOLD)
                            .text_color(move || theme().foreground),
                    )
                    // Loading skeletons
                    .child(when(store::explore_loading, || {
                        view()
                            .gap_2()
                            .child(skeleton_row())
                            .child(skeleton_row())
                            .child(skeleton_row())
                    }))
                    // Error card
                    .child(when(
                        || store::explore_error().is_some(),
                        || {
                            status_card(
                                "Unable to load Explore",
                                move || {
                                    SharedString::from(format_recoverable_error(
                                        "Could not load Explore feed",
                                        &store::explore_error().unwrap_or_default(),
                                        "Check your internet connection and select Retry.",
                                    ))
                                },
                                Some("Retry"),
                                Some(Box::new(store::refresh_explore)),
                            )
                        },
                    ))
                    .child(new_releases_section(current_id_memo, desktop))
                    .child(charts_section(current_id_memo, desktop))
                    .child(moods_and_genres_section(desktop))
                    .into_element()
            })),
    )
}
