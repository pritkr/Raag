//! Dedicated Artist Entity Screen (`src/screens/artist.rs`, `H2.1`) matching `ytm-rs` (`09_artist_detail_1.png`–`_3.png`):
//! - Full-bleed rounded artist hero banner with gradient/artwork scrim + overlaid bold Artist name & subscriber count
//! - Clean pill action bar below banner (`Play`, `Shuffle`, `Radio`, `Subscribe`)
//! - `Top songs` (`Show all` pill + compact rows)
//! - Horizontal carousels for `Albums`, `Singles & EPs`, and `Fans might also like` (circular artist cards)
//! Zero raw color literals — 100% semantic `theme.json` & `src/ui/tokens` tokens.

use std::sync::Arc;

use nsh_base::{button, theme};
use nsh_core::element::ImageSrc;
use nsh_core::prelude::*;
use resonance_contract::{ArtistDetail, LibraryCollectionRef, Track};
use resonance_core::palette::palette_for_track;

use crate::store;
use crate::ui::controls::{
    artwork_box, back_button, glass_pill_button, hero_pill_button, media_square_card,
    track_row_view,
};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, PROSE_MAX_WIDTH, ambient_accent_color, ambient_backdrop_color,
    color_on_cover, frosted_settings_card, glass_hairline, is_desktop_layout, overlay_scrim,
    radius_card,
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
            on_back: Arc::new(crate::store::go_back),
            on_play_top_songs: Arc::new(|tracks| {
                crate::store::play_queue(tracks, 0);
            }),
            on_start_radio: Arc::new(|artist| {
                crate::store::set_autoplay_radio(true);
                crate::store::play_queue(artist.top_songs, 0);
            }),
            on_shuffle: Arc::new(crate::store::play_all_shuffled),
            on_toggle_subscribe: Arc::new(crate::store::toggle_subscribe_artist),
            on_play_track: Arc::new(crate::store::play_track),
            on_play_next: Arc::new(crate::store::play_next),
            on_open_album: Arc::new(crate::store::open_album_entity),
            on_open_artist: Arc::new(crate::store::open_artist_entity),
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
    let card_width = if circular_hint { 156.0 } else { 172.0 };
    view()
        .gap_3()
        .child(
            text(heading)
                .text_xl()
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            scroll().horizontal().w_full().child(
                view()
                    .flex_row()
                    .items_start()
                    .gap_4()
                    .py_1()
                    .children(items.into_iter().map(move |item| {
                        let id = item.browse_id.clone();
                        let cb = Arc::clone(&on_open);
                        let mono = monogram_for(&item.title);
                        if circular_hint {
                            button()
                                .accessibility_label(item.title.clone())
                                .w(px(card_width))
                                .items_center()
                                .gap_2()
                                .rounded(radius_card)
                                .hover(|s| s.opacity(0.9))
                                .active(|s| s.opacity(0.78))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(move || cb(id.clone()))
                                .child(artwork_box(
                                    item.artwork_url,
                                    mono,
                                    144.0,
                                    72.0,
                                ))
                                .child(
                                    view()
                                        .w_full()
                                        .items_center()
                                        .gap_0p5()
                                        .child(
                                            text(item.title)
                                                .number_of_lines(1)
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text(item.subtitle)
                                                .number_of_lines(1)
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                        ),
                                )
                                .into_element()
                        } else {
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
                            .into_element()
                        }
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
    let palette = palette_for_track(&artist.channel_id, &artist.name);
    let hero_from = move || ambient_accent_color(palette);
    let hero_to = move || ambient_backdrop_color(palette);
    let desktop = is_desktop_layout();
    let banner_h = if desktop { 260.0 } else { 196.0 };

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
    let banner_art = store::resolve_artwork_url(artist.artwork_url.clone());

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
            .style_when(is_desktop_layout, |s| s.px_8().py_6())
            .gap_6()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(back_button(move || cb_back())),
            )
            // Full-bleed rounded Artist Hero Banner matching 09_artist_detail_1.png
            .child(
                view()
                    .relative()
                    .w_full()
                    .h(px(banner_h))
                    .rounded(radius_card)
                    .overflow_hidden()
                    .bg_linear_to_br()
                    .gradient_from(hero_from)
                    .gradient_to(hero_to)
                    .border_1()
                    .border_color(glass_hairline)
                    .when_some(banner_art, |v, url| {
                        v.child(
                            image(ImageSrc::from(url))
                                .absolute()
                                .inset_0()
                                .size_full()
                                .object_fit(ResizeMode::Cover),
                        )
                    })
                    // Subtle scrim for guaranteed white-text readability over hero imagery
                    .child(
                        view()
                            .absolute()
                            .inset_0()
                            .bg(overlay_scrim)
                            .opacity(0.45)
                            .pointer_events(PointerEvents::None),
                    )
                    .child(
                        view()
                            .relative()
                            .flex_1()
                            .justify_end()
                            .p_6()
                            .gap_1()
                            .child(
                                text(artist.name.clone())
                                    .when(desktop, |t| t.text_4xl())
                                    .when(!desktop, |t| t.text_2xl())
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(color_on_cover),
                            )
                            .child(
                                text(sub_label)
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(color_on_cover)
                                    .opacity(0.84),
                            ),
                    ),
            )
            // Action pills cleanly below banner matching 09_artist_detail_1.png
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2p5()
                    .child(
                        view()
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .justify_center()
                            .child(hero_pill_button(
                                "Play",
                                Some(IconKind::Play),
                                move || cb_play_top(top_for_play.clone()),
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
                            .test_id("artist-start-radio-btn")
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .justify_center()
                            .child(glass_pill_button(
                                "Radio",
                                Some(IconKind::Radio),
                                move || cb_radio(artist_for_radio.clone()),
                            )),
                    )
                    .child(
                        view()
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .justify_center()
                            .child(glass_pill_button(
                                if is_sub { "Subscribed" } else { "Subscribe" },
                                if is_sub { Some(IconKind::Check) } else { None },
                                move || cb_sub(cid_sub.clone(), !is_sub),
                            )),
                    )
                    .when(has_bio, |v| {
                        v.child(glass_pill_button(
                            move || {
                                if show_about.get() {
                                    SharedString::from("Hide about")
                                } else {
                                    SharedString::from("About")
                                }
                            },
                            None,
                            move || show_about.set(!show_about.get()),
                        ))
                    }),
            )
            .when(has_bio, |v| {
                v.child(when(
                    move || show_about.get(),
                    move || {
                        view()
                            .p_4()
                            .rounded(radius_card)
                            .bg(frosted_settings_card)
                            .border_1()
                            .border_color(glass_hairline)
                            .child(
                                text(bio_text.clone())
                                    .max_w(px(PROSE_MAX_WIDTH))
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
                        .gap_3()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    text("Top songs")
                                        .text_xl()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .when(total_songs > 4, |h| {
                                    h.child(glass_pill_button(
                                        move || {
                                            if show_all_songs.get() {
                                                SharedString::from("Show less")
                                            } else {
                                                SharedString::from("Show all")
                                            }
                                        },
                                        None,
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
                v.child(shelf_section("Fans might also like", related, cb_related, true))
            }),
    )
}
