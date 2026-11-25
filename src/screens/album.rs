//! Dedicated Album Entity Screen (`src/screens/album.rs`, `H2.1` + `H2.2`).
//!
//! Zero raw color literals — 100% semantic `theme.json` & `src/ui/tokens` tokens.

use std::sync::Arc;

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{AlbumDetail, Track, format_duration_ms};
use resonance_core::downloads::CollectionOfflineProgress;
use resonance_core::palette::palette_for_track;

use crate::ui::controls::{
    artwork_box, glass_pill_button, hero_pill_button, media_square_card, secondary_button,
    track_row_view,
};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, ambient_backdrop_color, glass_hairline, is_desktop_layout,
    pill_glass_surface, radius_card, radius_pill,
};

#[derive(Clone)]
pub struct AlbumScreenCallbacks {
    pub on_back: Arc<dyn Fn()>,
    pub on_play_album: Arc<dyn Fn(Vec<Track>)>,
    pub on_shuffle_album: Arc<dyn Fn(Vec<Track>)>,
    pub on_start_radio: Arc<dyn Fn(AlbumDetail)>,
    pub on_toggle_pin_offline: Arc<dyn Fn(AlbumDetail)>,
    pub on_add_album_to_playlist: Arc<dyn Fn(AlbumDetail)>,
    pub on_open_artist: Arc<dyn Fn(String)>,
    pub on_open_album: Arc<dyn Fn(String)>,
    pub on_play_track: Arc<dyn Fn(Track)>,
    pub on_play_next: Arc<dyn Fn(Track)>,
}

impl Default for AlbumScreenCallbacks {
    fn default() -> Self {
        Self {
            on_back: Arc::new(|| crate::store::go_back()),
            on_play_album: Arc::new(|tracks| {
                if let Some(first) = tracks.first().cloned() {
                    crate::store::play_track(first);
                }
            }),
            on_shuffle_album: Arc::new(|tracks| {
                if let Some(first) = tracks.first().cloned() {
                    crate::store::play_track(first);
                }
            }),
            on_start_radio: Arc::new(|album| {
                if let Some(first) = album.tracks.first().cloned() {
                    crate::store::play_track(first);
                }
            }),
            on_toggle_pin_offline: Arc::new(|_| {}),
            on_add_album_to_playlist: Arc::new(|_| {}),
            on_open_artist: Arc::new(|_| {}),
            on_open_album: Arc::new(|_| {}),
            on_play_track: Arc::new(|t| crate::store::play_track(t)),
            on_play_next: Arc::new(|t| crate::store::play_next(t)),
        }
    }
}

fn format_album_stats(album: &AlbumDetail) -> String {
    let count = album.tracks.len();
    let total_ms = if album.total_duration_ms > 0 {
        album.total_duration_ms
    } else {
        album.tracks.iter().map(|t| t.duration_ms).sum()
    };
    let mins = (total_ms / 60_000).max(if total_ms > 0 { 1 } else { 0 });
    match album.release_year.as_deref() {
        Some(year) if !year.is_empty() => format!("{year} • {count} songs • {mins} min"),
        _ => format!("{count} songs • {mins} min"),
    }
}

fn monogram_for(title: &str) -> String {
    title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "♫".into())
}

pub fn album_screen_with_detail(
    album: AlbumDetail,
    offline_progress: CollectionOfflineProgress,
    cb: AlbumScreenCallbacks,
) -> impl IntoElement {
    let desktop = is_desktop_layout();
    let art_size = if desktop { 220.0 } else { 176.0 };
    let palette = palette_for_track(&album.browse_id, &album.title);
    let hero_bg = move || ambient_backdrop_color(palette);
    let mono = monogram_for(&album.title);
    let stats_line = format_album_stats(&album);

    let primary_artist_name = album
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let primary_artist_id = album.artists.first().and_then(|a| a.id.clone());

    let tracks_play = album.tracks.clone();
    let tracks_shuffle = album.tracks.clone();
    let album_radio = album.clone();
    let album_pin = album.clone();
    let album_playlist = album.clone();
    let tracks_list = album.tracks.clone();
    let other_versions = album.other_versions.clone();

    let cb_back = Arc::clone(&cb.on_back);
    let cb_play = Arc::clone(&cb.on_play_album);
    let cb_shuffle = Arc::clone(&cb.on_shuffle_album);
    let cb_radio = Arc::clone(&cb.on_start_radio);
    let cb_pin = Arc::clone(&cb.on_toggle_pin_offline);
    let cb_add_pl = Arc::clone(&cb.on_add_album_to_playlist);
    let cb_artist = Arc::clone(&cb.on_open_artist);
    let cb_other = Arc::clone(&cb.on_open_album);
    let cb_track = Arc::clone(&cb.on_play_track);
    let cb_next = Arc::clone(&cb.on_play_next);

    let pin_label = offline_progress.format_badge();
    let pin_icon = if offline_progress.is_pinned() {
        IconKind::Check
    } else {
        IconKind::Download
    };

    scroll().flex_1().min_h(px(0.0)).pb_safe_area().child(
        view()
            .test_id("album-entity-screen")
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
                    .gap_5()
                    .rounded(radius_card)
                    .bg(hero_bg)
                    .border_1()
                    .border_color(glass_hairline)
                    .when(desktop, |v| v.flex_row().items_end().gap_6())
                    .child(artwork_box(album.artwork_url.clone(), mono, art_size, 12.0))
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap_2()
                            .child(
                                view()
                                    .self_start()
                                    .px_2p5()
                                    .py_1()
                                    .rounded(radius_pill)
                                    .bg(pill_glass_surface)
                                    .child(
                                        text(album.release_type.clone())
                                            .text_xs()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().muted_foreground),
                                    ),
                            )
                            .child(
                                text(album.title.clone())
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .when(!primary_artist_name.is_empty(), |v| {
                                let artist_id = primary_artist_id.clone();
                                let cb_art = Arc::clone(&cb_artist);
                                v.child(
                                    button()
                                        .self_start()
                                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                        .justify_center()
                                        .on_press(move || {
                                            if let Some(cid) = artist_id.as_ref() {
                                                cb_art(cid.clone());
                                            }
                                        })
                                        .child(
                                            text(primary_artist_name.clone())
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || theme().primary),
                                        ),
                                )
                            })
                            .child(
                                text(stats_line)
                                    .text_xs()
                                    .tabular_nums()
                                    .text_color(move || theme().muted_foreground),
                            )
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
                                                "Play",
                                                Some(IconKind::Play),
                                                move || cb_play(tracks_play.clone()),
                                            )),
                                    )
                                    .child(
                                        view()
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .justify_center()
                                            .child(glass_pill_button(
                                                "Shuffle",
                                                Some(IconKind::Shuffle),
                                                move || cb_shuffle(tracks_shuffle.clone()),
                                            )),
                                    )
                                    .child(
                                        view()
                                            .test_id("album-start-radio-btn")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .justify_center()
                                            .child(glass_pill_button(
                                                "Start Radio",
                                                Some(IconKind::Radio),
                                                move || cb_radio(album_radio.clone()),
                                            )),
                                    )
                                    .child(
                                        view()
                                            .test_id("album-pin-offline-btn")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .justify_center()
                                            .child(glass_pill_button(
                                                pin_label,
                                                Some(pin_icon),
                                                move || cb_pin(album_pin.clone()),
                                            )),
                                    )
                                    .child(
                                        view()
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .justify_center()
                                            .child(glass_pill_button(
                                                "Add to Playlist",
                                                Some(IconKind::Plus),
                                                move || cb_add_pl(album_playlist.clone()),
                                            )),
                                    ),
                            ),
                    ),
            )
            .child(
                view()
                    .gap_1()
                    .children(
                        tracks_list
                            .into_iter()
                            .enumerate()
                            .map(move |(idx, track)| {
                                let t1 = track.clone();
                                let t2 = track.clone();
                                let c1 = Arc::clone(&cb_track);
                                let c2 = Arc::clone(&cb_next);
                                let dur = format_duration_ms(track.duration_ms);
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        text(format!("{}", idx + 1))
                                            .w(px(28.0))
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                    )
                                    .child(view().flex_1().min_w(px(0.0)).child(track_row_view(
                                        track,
                                        false,
                                        move || c1(t1.clone()),
                                        move || c2(t2.clone()),
                                    )))
                                    .child(
                                        text(dur)
                                            .text_xs()
                                            .tabular_nums()
                                            .text_color(move || theme().muted_foreground),
                                    )
                            }),
                    ),
            )
            .when(!other_versions.is_empty(), |v| {
                v.child(
                    view()
                        .gap_3()
                        .child(
                            text("Other Versions")
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            scroll()
                                .horizontal()
                                .child(view().flex_row().gap_4().children(
                                    other_versions.into_iter().map(move |ver| {
                                        let bid = ver.browse_id.clone();
                                        let cb_v = Arc::clone(&cb_other);
                                        let m = monogram_for(&ver.title);
                                        media_square_card(
                                            ver.artwork_url,
                                            m,
                                            ver.title,
                                            ver.subtitle,
                                            164.0,
                                            false,
                                            move || cb_v(bid.clone()),
                                            None,
                                        )
                                    }),
                                )),
                        ),
                )
            }),
    )
}
