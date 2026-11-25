//! Dedicated Artist Entity Screen (`src/screens/artist.rs`, `H2.1`).
//!
//! Zero raw color literals — 100% semantic `theme.json` & `src/ui/tokens` tokens.

use std::sync::Arc;

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{ArtistDetail, LibraryCollectionRef, Track};
use resonance_core::palette::palette_for_track;

use crate::ui::controls::{
    artwork_box, glass_pill_button, hero_pill_button, media_square_card, secondary_button,
    track_row_view,
};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, ambient_backdrop_color, glass_hairline, is_desktop_layout,
    pill_glass_surface, radius_card,
};

#[derive(Clone)]
pub struct ArtistScreenCallbacks {
    pub on_back: Arc<dyn Fn()>,
    pub on_play_top_songs: Arc<dyn Fn(Vec<Track>)>,
    pub on_start_radio: Arc<dyn Fn(ArtistDetail)>,
    pub on_shuffle: Arc<dyn Fn(Vec<Track>)>,
    pub on_toggle_subscribe: Arc<dyn Fn(String, bool)>,
    pub on_play_track: Arc<dyn Fn(Track)>,
    pub on_play_next: Arc<dyn Fn(Track)>,
    pub on_open_album: Arc<dyn Fn(String)>,
    pub on_open_artist: Arc<dyn Fn(String)>,
}

impl Default for ArtistScreenCallbacks {
    fn default() -> Self {
        Self {
            on_back: Arc::new(|| crate::store::go_back()),
            on_play_top_songs: Arc::new(|tracks| {
                if let Some(first) = tracks.first().cloned() {
                    crate::store::play_track(first);
                }
            }),
            on_start_radio: Arc::new(|artist| {
                if let Some(first) = artist.top_songs.first().cloned() {
                    crate::store::play_track(first);
                }
            }),
            on_shuffle: Arc::new(|tracks| {
                if let Some(first) = tracks.first().cloned() {
                    crate::store::play_track(first);
                }
            }),
            on_toggle_subscribe: Arc::new(|_, _| {}),
            on_play_track: Arc::new(|t| crate::store::play_track(t)),
            on_play_next: Arc::new(|t| crate::store::play_next(t)),
            on_open_album: Arc::new(|_| {}),
            on_open_artist: Arc::new(|_| {}),
        }
    }
}

fn monogram_for(name: &str) -> String {
    name.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "♪".into())
}

fn shelf_section(
    heading: &'static str,
    items: Vec<LibraryCollectionRef>,
    on_open: Arc<dyn Fn(String)>,
    circular_hint: bool,
) -> impl IntoElement {
    let card_width = if circular_hint { 148.0 } else { 164.0 };
    view()
        .gap_3()
        .child(
            text(heading)
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            scroll()
                .horizontal()
                .child(
                    view()
                        .flex_row()
                        .gap_4()
                        .children(items.into_iter().map(move |item| {
                            let id = item.browse_id.clone();
                            let cb = Arc::clone(&on_open);
                            let mono = monogram_for(&item.title);
                            media_square_card(
                                item.artwork_url,
                                mono,
                                item.title,
                                item.subtitle,
                                card_width,
                                false,
                                move || cb(id.clone()),
                                None,
                            )
                        })),
                ),
        )
}

pub fn artist_screen_with_detail(
    artist: ArtistDetail,
    cb: ArtistScreenCallbacks,
) -> impl IntoElement {
    let show_all_songs = signal(false);
    let show_about = signal(false);
    let desktop = is_desktop_layout();
    let palette = palette_for_track(&artist.channel_id, &artist.name);
    let hero_bg = move || ambient_backdrop_color(palette);
    let art_size = if desktop { 172.0 } else { 128.0 };
    let mono = monogram_for(&artist.name);

    let top_for_play = artist.top_songs.clone();
    let top_for_shuffle = artist.top_songs.clone();
    let artist_for_radio = artist.clone();
    let cid_sub = artist.channel_id.clone();
    let is_sub = artist.is_subscribed;
    let has_bio = artist
        .description
        .as_ref()
        .is_some_and(|d| !d.trim().is_empty());
    let bio_text = artist.description.clone().unwrap_or_default();
    let sub_label = artist
        .subscribers
        .clone()
        .unwrap_or_else(|| "Artist".to_owned());

    let top_songs = artist.top_songs.clone();
    let albums = artist.albums.clone();
    let singles = artist.singles.clone();
    let related = artist.related_artists.clone();

    let cb_back = Arc::clone(&cb.on_back);
    let cb_play_top = Arc::clone(&cb.on_play_top_songs);
    let cb_radio = Arc::clone(&cb.on_start_radio);
    let cb_shuffle = Arc::clone(&cb.on_shuffle);
    let cb_sub = Arc::clone(&cb.on_toggle_subscribe);
    let cb_track = Arc::clone(&cb.on_play_track);
    let cb_next = Arc::clone(&cb.on_play_next);
    let cb_album = Arc::clone(&cb.on_open_album);
    let cb_singles = Arc::clone(&cb.on_open_album);
    let cb_related = Arc::clone(&cb.on_open_artist);

    scroll().flex_1().min_h(px(0.0)).pb_safe_area().child(
        view()
            .test_id("artist-entity-screen")
            .p_4()
            .when(desktop, |v| v.px_8().py_6())
            .gap_6()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(secondary_button("← Back", move || cb_back())),
            )
            .child(
                view()
                    .p_5()
                    .gap_4()
                    .rounded(radius_card)
                    .bg(hero_bg)
                    .border_1()
                    .border_color(glass_hairline)
                    .when(desktop, |v| v.flex_row().items_end().gap_6())
                    .child(
                        view()
                            .w(px(art_size))
                            .h(px(art_size))
                            .rounded_full()
                            .overflow_hidden()
                            .child(artwork_box(
                                artist.artwork_url.clone(),
                                mono,
                                art_size,
                                art_size / 2.0,
                            )),
                    )
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap_2()
                            .child(
                                text(artist.name.clone())
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(
                                text(sub_label)
                                    .text_sm()
                                    .text_color(move || theme().muted_foreground),
                            )
                            .when(has_bio, |v| {
                                v.child(
                                    button()
                                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                        .px_3()
                                        .rounded(radius_card)
                                        .bg(pill_glass_surface)
                                        .on_press(move || show_about.set(!show_about.get()))
                                        .child(
                                            text(move || {
                                                if show_about.get() {
                                                    "Hide About"
                                                } else {
                                                    "About"
                                                }
                                            })
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                        ),
                                )
                            })
                            .child(
                                view()
                                    .flex_row()
                                    .flex_wrap()
                                    .gap_2p5()
                                    .pt_1()
                                    .child(
                                        view()
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .justify_center()
                                            .child(hero_pill_button(
                                                "Play Top Songs",
                                                Some(IconKind::Play),
                                                move || cb_play_top(top_for_play.clone()),
                                            )),
                                    )
                                    .child(
                                        view()
                                            .test_id("artist-start-radio-btn")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .justify_center()
                                            .child(glass_pill_button(
                                                "Start Radio",
                                                Some(IconKind::Radio),
                                                move || cb_radio(artist_for_radio.clone()),
                                            )),
                                    )
                                    .child(
                                        view()
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .justify_center()
                                            .child(glass_pill_button(
                                                "Shuffle",
                                                Some(IconKind::Shuffle),
                                                move || cb_shuffle(top_for_shuffle.clone()),
                                            )),
                                    )
                                    .child(
                                        view()
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .justify_center()
                                            .child(glass_pill_button(
                                                if is_sub { "Subscribed" } else { "Subscribe" },
                                                Some(if is_sub {
                                                    IconKind::Check
                                                } else {
                                                    IconKind::Plus
                                                }),
                                                move || cb_sub(cid_sub.clone(), !is_sub),
                                            )),
                                    ),
                            ),
                    ),
            )
            .when(has_bio, |v| {
                v.child(when(
                    move || show_about.get(),
                    move || {
                        view()
                            .p_4()
                            .rounded(radius_card)
                            .bg(move || theme().card)
                            .border_1()
                            .border_color(glass_hairline)
                            .child(
                                text(bio_text.clone())
                                    .text_sm()
                                    .text_color(move || theme().muted_foreground),
                            )
                    },
                ))
            })
            .when(!top_songs.is_empty(), |v| {
                let songs_for_dyn = top_songs.clone();
                let total_songs = songs_for_dyn.len();
                v.child(
                    view()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    text("Top Songs")
                                        .text_lg()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .when(total_songs > 5, |h| {
                                    h.child(secondary_button(
                                        move || {
                                            if show_all_songs.get() {
                                                "Show less"
                                            } else {
                                                "Show all"
                                            }
                                        },
                                        move || show_all_songs.set(!show_all_songs.get()),
                                    ))
                                }),
                        )
                        .child(dynamic(move || {
                            let limit = if show_all_songs.get() {
                                songs_for_dyn.len()
                            } else {
                                songs_for_dyn.len().min(5)
                            };
                            view()
                                .gap_1()
                                .children(songs_for_dyn.iter().take(limit).cloned().map(|track| {
                                    let t1 = track.clone();
                                    let t2 = track.clone();
                                    let c1 = Arc::clone(&cb_track);
                                    let c2 = Arc::clone(&cb_next);
                                    track_row_view(
                                        track,
                                        false,
                                        move || c1(t1.clone()),
                                        move || c2(t2.clone()),
                                    )
                                }))
                        })),
                )
            })
            .when(!albums.is_empty(), |v| {
                v.child(shelf_section("Albums", albums, cb_album, false))
            })
            .when(!singles.is_empty(), |v| {
                v.child(shelf_section("Singles & EPs", singles, cb_singles, false))
            })
            .when(!related.is_empty(), |v| {
                v.child(shelf_section("Related Artists", related, cb_related, true))
            }),
    )
}
