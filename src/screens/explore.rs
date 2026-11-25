//! Explore & Discovery Screen:
//! - Quick category pills (`New releases`, `Charts`, `Moods & genres`)
//! - "New releases for you" paginated horizontal shelf of square artwork cards
//! - "Charts" paginated horizontal shelf (`Trending`, `Top Songs`, `Global Top 100`, `Indie Spotlight`)
//! - "Moods & genres" responsive grid of tinted editorial cards with left accent stripes

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{HomeItem, Track};

use crate::store::{self, AppTab};
use crate::ui::controls::{media_square_card, mood_chip, svg_icon_button};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_ambient_warm, cover_gradient_pair, genre_card_tint,
    glass_hairline, is_desktop_layout, radius_card,
};

const QUICK_CATEGORIES: [&str; 3] = ["New releases", "Charts", "Moods & genres"];

const MOODS_AND_GENRES: [&str; 10] = [
    "Chill",
    "Workout",
    "Focus",
    "Party",
    "Romance",
    "Bollywood",
    "Acoustic",
    "Synthwave",
    "Jazz",
    "Lo-Fi",
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
    DiscoveryMix {
        title: String,
        subtitle: String,
        monogram: String,
        query: String,
    },
}

#[derive(Clone, Copy)]
struct ChartEntry {
    title: &'static str,
    subtitle: &'static str,
    monogram: &'static str,
    query: &'static str,
}

const CHART_CARDS: [ChartEntry; 6] = [
    ChartEntry {
        title: "Trending",
        subtitle: "Rising right now • Daily",
        monogram: "TR",
        query: "Trending",
    },
    ChartEntry {
        title: "Top Songs",
        subtitle: "Most played • Global",
        monogram: "TS",
        query: "Top Songs",
    },
    ChartEntry {
        title: "Global Top 100",
        subtitle: "Worldwide weekly chart",
        monogram: "100",
        query: "Global Top 100",
    },
    ChartEntry {
        title: "Indie Spotlight",
        subtitle: "Fresh independent picks",
        monogram: "IS",
        query: "Indie",
    },
    ChartEntry {
        title: "Synthwave Pulse",
        subtitle: "Electronic & night drives",
        monogram: "SW",
        query: "Synthwave",
    },
    ChartEntry {
        title: "Acoustic Sessions",
        subtitle: "Stripped back & live",
        monogram: "AC",
        query: "Acoustic",
    },
];

fn curated_discovery_mixes() -> Vec<ReleaseCardKind> {
    vec![
        ReleaseCardKind::DiscoveryMix {
            title: "Midnight Analog".into(),
            subtitle: "Synth & warm tape • Mix".into(),
            monogram: "MA".into(),
            query: "Midnight".into(),
        },
        ReleaseCardKind::DiscoveryMix {
            title: "Tokyo Afterhours".into(),
            subtitle: "City pop & neon grooves".into(),
            monogram: "TA".into(),
            query: "Tokyo".into(),
        },
        ReleaseCardKind::DiscoveryMix {
            title: "Velvet Acoustic".into(),
            subtitle: "Warm strings & vocals".into(),
            monogram: "VA".into(),
            query: "Acoustic".into(),
        },
        ReleaseCardKind::DiscoveryMix {
            title: "Subterranean Bass".into(),
            subtitle: "Deep electronic cuts".into(),
            monogram: "SB".into(),
            query: "Pulse".into(),
        },
    ]
}

fn build_new_releases() -> Vec<ReleaseCardKind> {
    let mut items = Vec::new();
    let sections = {
        let exp = store::explore_sections();
        if exp.is_empty() {
            store::home_sections()
        } else {
            exp
        }
    };
    for section in sections {
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
    if store::is_fixture_mode() {
        items.extend(curated_discovery_mixes());
    }
    items
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
    title: &'static str,
    section_id: &'static str,
    total_items: usize,
    page_size: usize,
    desktop: bool,
) -> impl IntoElement {
    let btn_px = if desktop {
        34.0
    } else {
        MOBILE_MIN_TOUCH_TARGET
    };

    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            text(title)
                .number_of_lines(1)
                .when(desktop, |t| t.text_xl())
                .when(!desktop, |t| t.text_lg())
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
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
                    move || store::shift_section_page(section_id, -1, total_items, page_size),
                ))
                .child(svg_icon_button(
                    IconKind::ChevronRight,
                    16.0,
                    btn_px,
                    "Next page",
                    false,
                    move || theme().foreground,
                    move || store::shift_section_page(section_id, 1, total_items, page_size),
                )),
        )
}

fn new_releases_section(current_id_memo: Memo<Option<String>>, desktop: bool) -> impl IntoElement {
    const SECTION_ID: &str = "explore-new-releases";
    const PAGE_SIZE: usize = 6;
    let home_memo = computed(store::home_sections);
    let explore_memo = computed(store::explore_sections);

    view().gap_3().child(dynamic(move || {
        let _ = home_memo.get();
        let _ = explore_memo.get();
        let all_releases = build_new_releases();
        let total = all_releases.len();
        let page = store::section_page(SECTION_ID);
        let slice = paginated_slice(&all_releases, page, PAGE_SIZE);
        let active_id = current_id_memo.get();

        view()
            .gap_3()
            .child(explore_section_header(
                "New releases for you",
                SECTION_ID,
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
                        .children(slice.into_iter().map(move |entry| {
                            match entry {
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
                                        168.0,
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
                                        168.0,
                                        false,
                                        move || {
                                            store::open_collection(
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
                                ReleaseCardKind::DiscoveryMix {
                                    title,
                                    subtitle,
                                    monogram,
                                    query,
                                } => media_square_card(
                                    None,
                                    monogram,
                                    title,
                                    subtitle,
                                    168.0,
                                    false,
                                    move || {
                                        store::run_search(&query);
                                        store::set_tab(AppTab::Search);
                                    },
                                    None,
                                )
                                .into_element(),
                            }
                        })),
                ),
            )
            .into_element()
    }))
}

fn charts_section(desktop: bool) -> impl IntoElement {
    const SECTION_ID: &str = "explore-charts";
    const PAGE_SIZE: usize = 4;

    view()
        .gap_3()
        .child(explore_section_header(
            "Charts",
            SECTION_ID,
            CHART_CARDS.len(),
            PAGE_SIZE,
            desktop,
        ))
        .child(dynamic(move || {
            let page = store::section_page(SECTION_ID);
            let slice = paginated_slice(&CHART_CARDS, page, PAGE_SIZE);

            scroll()
                .horizontal()
                .w_full()
                .child(view().flex_row().items_start().gap_4().py_1().children(
                    slice.into_iter().map(|chart| {
                        media_square_card(
                            None,
                            chart.monogram.to_owned(),
                            chart.title.to_owned(),
                            chart.subtitle.to_owned(),
                            168.0,
                            false,
                            move || {
                                store::set_search_filter(store::SearchFilter::All);
                                store::run_search(chart.query);
                                store::set_tab(AppTab::Search);
                            },
                            None,
                        )
                    }),
                ))
                .into_element()
        }))
}

fn moods_and_genres_section(desktop: bool) -> impl IntoElement {
    let cols_count = if desktop { 4 } else { 2 };

    view()
        .gap_3()
        .child(
            text("Moods & genres")
                .when(desktop, |t| t.text_xl())
                .when(!desktop, |t| t.text_lg())
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            view()
                .gap_3()
                .children(MOODS_AND_GENRES.chunks(cols_count).enumerate().map(
                    move |(row_idx, row_genres)| {
                        let mut cells = Vec::with_capacity(cols_count);
                        for (col_idx, &genre) in row_genres.iter().enumerate() {
                            let idx = row_idx * cols_count + col_idx;
                            let (stripe_color, _) = cover_gradient_pair(genre);
                            cells.push(
                                button()
                                    .accessibility_label(genre)
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .min_h(px(56.0))
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .pr_3()
                                    .overflow_hidden()
                                    .rounded(radius_card)
                                    .bg(move || genre_card_tint(idx))
                                    .border_1()
                                    .border_color(glass_hairline)
                                    .hover(|s| s.opacity(0.9))
                                    .active(|s| s.opacity(0.76))
                                    .focus_visible(|s| {
                                        s.outline(px(2.0)).outline_color(move || theme().ring)
                                    })
                                    .on_press(move || {
                                        store::select_mood(genre);
                                    })
                                    .child(view().w(px(6.0)).h_full().bg(move || stripe_color))
                                    .child(
                                        text(genre)
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
                        view().flex_row().gap_3().children(cells)
                    },
                )),
        )
}

pub fn explore_screen() -> impl IntoElement {
    let selected_cat = signal(None::<&'static str>);
    let desktop_memo = computed(is_desktop_layout);
    let current_id_memo = computed(|| store::player_snapshot().current_track.map(|t| t.id));

    view()
        .flex_1()
        .bg_linear_to_b()
        .gradient_from(color_ambient_warm)
        .gradient_to(move || theme().background)
        .child(scroll().flex_1().child(dynamic(move || {
            let desktop = desktop_memo.get();
            let active_cat = selected_cat.get();

            view()
                .p_4()
                .when(desktop, |v| v.px_8().py_6())
                .gap_6()
                .child(
                    view()
                        .gap_3()
                        .child(
                            text("Explore")
                                .when(desktop, |t| t.text_3xl())
                                .when(!desktop, |t| t.text_2xl())
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(scroll().horizontal().w_full().child(
                            view().flex_row().items_center().gap_2().py_1().children(
                                QUICK_CATEGORIES.into_iter().map(move |cat| {
                                    mood_chip(
                                        cat,
                                        move || selected_cat.get() == Some(cat),
                                        move || {
                                            if selected_cat.get_untracked() == Some(cat) {
                                                selected_cat.set(None);
                                            } else {
                                                selected_cat.set(Some(cat));
                                            }
                                        },
                                    )
                                }),
                            ),
                        )),
                )
                .when(
                    active_cat.is_none() || active_cat == Some("New releases"),
                    |v| v.child(new_releases_section(current_id_memo, desktop)),
                )
                .when(active_cat.is_none() || active_cat == Some("Charts"), |v| {
                    v.child(charts_section(desktop))
                })
                .when(
                    active_cat.is_none() || active_cat == Some("Moods & genres"),
                    |v| v.child(moods_and_genres_section(desktop)),
                )
                .into_element()
        })))
}
