//! Library, Account Sign-In (`; wv`-stripped Android WebView / SAPISIDHASH),
//! and Settings (Theme, Reduced Motion, SponsorBlock auto-skip, Cache readout & Clear).

use nsh_base::{input, theme};
use nsh_core::prelude::*;

use crate::store::{self, ThemeChoice};
use crate::ui::controls::{
    primary_button, secondary_button, setting_switch, status_card, track_row_view,
};
use crate::ui::tokens::{MOBILE_MIN_TOUCH_TARGET, radius_card};

fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

pub fn library_screen() -> impl IntoElement {
    let cookie_draft = signal(String::new());
    let liked = store::liked_tracks();
    let history = store::history_entries();
    let current_id = store::player_snapshot().current_track.map(|t| t.id);

    scroll().flex_1().child(
        view()
            .p_4()
            .gap_5()
            .child(
                text("Library & Settings")
                    .text_2xl()
                    .font_weight(FontWeight::BOLD)
                    .text_color(move || theme().foreground),
            )
            // Account & Anonymous-First Auth Section
            .child(
                view()
                    .p_4()
                    .gap_3()
                    .rounded(radius_card)
                    .bg(move || theme().card)
                    .border_1()
                    .border_color(move || theme().border)
                    .child(
                        text("YouTube Music Account")
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        text(move || {
                            let (signed_in, acct) = store::auth_info();
                            if signed_in {
                                SharedString::from(format!(
                                    "Connected: {}",
                                    acct.unwrap_or_else(|| "Google Account".into())
                                ))
                            } else {
                                SharedString::from(
                                    "Anonymous mode active. Optional Sign-In uses Android WebView with '; wv' User-Agent stripping and SAPISIDHASH isolation.",
                                )
                            }
                        })
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                    )
                    .child(
                        when(
                            || !store::auth_info().0,
                            move || {
                                view()
                                    .gap_2()
                                    .child(
                                        input()
                                            .test_id("cookie-input")
                                            .value(move || cookie_draft.get())
                                            .on_change(move |v: &str| {
                                                cookie_draft.set(v.to_owned())
                                            })
                                            .placeholder("SAPISID=...; __Secure-3PAPISID=...")
                                            .placeholder_color(|| color(theme().muted_foreground))
                                            .caret_color(|| color(theme().primary))
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                            .px_3()
                                            .text_sm()
                                            .text_color(move || theme().foreground)
                                            .bg(move || theme().background)
                                            .rounded(move || theme().radius)
                                            .border_1()
                                            .border_color(move || theme().input),
                                    )
                                    .child(primary_button("Sign In with Cookies", move || {
                                        store::sign_in_with_cookies(&cookie_draft.get_untracked());
                                    }))
                            },
                        )
                        .otherwise(|| secondary_button("Sign Out", store::sign_out)),
                    ),
            )
            // Playback, SponsorBlock & Theme Settings
            .child(
                view()
                    .gap_2()
                    .child(
                        text("Preferences")
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(setting_switch(
                        "SponsorBlock (auto-skip non-music)",
                        || store::player_snapshot().sponsorblock_enabled,
                        store::set_sponsorblock_enabled,
                    ))
                    .child(setting_switch(
                        "Offline Mode (local chunk cache only)",
                        || store::player_snapshot().offline_mode,
                        store::set_offline_mode,
                    ))
                    .child(setting_switch(
                        "Reduce Motion",
                        store::reduced_motion,
                        store::set_reduced_motion,
                    ))
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .gap_2()
                            .child(secondary_button(
                                move || {
                                    if store::theme_choice() == ThemeChoice::System {
                                        SharedString::from("✓ System Theme")
                                    } else {
                                        SharedString::from("System Theme")
                                    }
                                },
                                || store::set_theme_choice(ThemeChoice::System),
                            ))
                            .child(secondary_button(
                                move || {
                                    if store::theme_choice() == ThemeChoice::Light {
                                        SharedString::from("✓ Light Theme")
                                    } else {
                                        SharedString::from("Light Theme")
                                    }
                                },
                                || store::set_theme_choice(ThemeChoice::Light),
                            ))
                            .child(secondary_button(
                                move || {
                                    if store::theme_choice() == ThemeChoice::Dark {
                                        SharedString::from("✓ Dark Theme")
                                    } else {
                                        SharedString::from("Dark Theme")
                                    }
                                },
                                || store::set_theme_choice(ThemeChoice::Dark),
                            )),
                    ),
            )
            // Local Storage & Offline Chunk Cache Control
            .child(
                view()
                    .p_4()
                    .gap_3()
                    .rounded(radius_card)
                    .bg(move || theme().card)
                    .border_1()
                    .border_color(move || theme().border)
                    .child(
                        text("Offline & Local Cache")
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        view()
                            .w_full()
                            .h(px(6.0))
                            .rounded_full()
                            .bg(move || theme().muted)
                            .overflow_hidden()
                            .child(
                                view()
                                    .test_id("cache-usage-bar")
                                    .h_full()
                                    .w(move || {
                                        let s = store::cache_stats();
                                        let pct = if s.max_audio_bytes > 0 {
                                            (s.audio_chunk_bytes as f32
                                                / s.max_audio_bytes as f32)
                                                * 100.0
                                        } else {
                                            0.0
                                        };
                                        percentage(pct.clamp(0.0, 100.0))
                                    })
                                    .bg(move || theme().primary),
                            ),
                    )
                    .child(
                        text(move || {
                            let s = store::cache_stats();
                            SharedString::from(format!(
                                "Audio chunks: {} ({} chunks) • Artwork: {} • Metadata: {} rows",
                                format_mb(s.audio_chunk_bytes),
                                s.audio_chunk_count,
                                format_mb(s.artwork_bytes),
                                s.metadata_entries
                            ))
                        })
                        .test_id("cache-stats-readout")
                        .text_xs()
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                    )
                    .child(secondary_button("Clear Cache", store::clear_caches)),
            )
            // Liked Songs
            .child(
                view()
                    .gap_2()
                    .child(
                        text("Liked Songs")
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .when(liked.is_empty(), |v| {
                        v.child(status_card(
                            "No liked songs yet",
                            "Tap the heart icon on Now Playing to save favorites locally.",
                            None,
                            None,
                        ))
                    })
                    .children(liked.into_iter().map(move |track| {
                        let is_cur = current_id.as_deref() == Some(track.id.as_str());
                        let t_play = track.clone();
                        let t_next = track.clone();
                        track_row_view(
                            track,
                            is_cur,
                            move || store::play_track(t_play.clone()),
                            move || store::play_next(t_next.clone()),
                        )
                    })),
            )
            // Local Listening History
            .child(
                view()
                    .gap_2()
                    .child(
                        text("Recently Played")
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .when(history.is_empty(), |v| {
                        v.child(
                            text("Tracks you play appear here automatically.")
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                    })
                    .children(history.into_iter().take(10).map(|entry| {
                        let t_play = entry.track.clone();
                        let t_next = entry.track.clone();
                        track_row_view(
                            entry.track,
                            false,
                            move || store::play_track(t_play.clone()),
                            move || store::play_next(t_next.clone()),
                        )
                    })),
            ),
    )
}
