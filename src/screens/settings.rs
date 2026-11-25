//! Dedicated Settings, Account & Listen Together (Shared Radio Sessions) screens.
//!
//! `settings_sections()` is shared between `settings_screen()` (`AppTab::Settings`)
//! and `library_screen()` (`AppTab::Library`) so preferences, SAPISIDHASH account auth,
//! and offline chunk cache controls stay unified with zero duplication.

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::{CredentialBackendKind, HomeItem, Track};

use crate::store::{self, AppTab, ThemeChoice};
use crate::ui::controls::{
    artwork_box, glass_pill_button, hero_pill_button, secondary_button, setting_switch,
    status_card, track_row_view,
};
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_ambient_warm, color_player_accent, color_status_ok,
    frosted_card_surface, glass_hairline, is_desktop_layout, pill_glass_surface, radius_card,
    radius_panel,
};

fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn collect_session_tracks() -> Vec<Track> {
    let queue_items = store::player_snapshot().queue.items;
    if !queue_items.is_empty() {
        return queue_items;
    }
    let mut fallback = Vec::new();
    for section in store::home_sections() {
        for item in section.items {
            if let HomeItem::Track(t) = item
                && !fallback.iter().any(|existing: &Track| existing.id == t.id)
            {
                fallback.push(t);
            }
        }
    }
    fallback
}

fn backend_badge_label(kind: CredentialBackendKind) -> &'static str {
    match kind {
        CredentialBackendKind::OsKeyring => "OS Keychain",
        CredentialBackendKind::EncryptedFallbackFile => "Local 0600 File",
        CredentialBackendKind::MemoryOnly => "Memory Only",
    }
}

fn account_card(cookie_draft: Signal<String>) -> impl IntoElement {
    view()
        .p_4()
        .gap_3()
        .rounded(radius_card)
        .bg(move || theme().card)
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text("YouTube Music Account")
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            view()
                                .px_2p5()
                                .py_1()
                                .rounded_full()
                                .bg(pill_glass_surface)
                                .child(
                                    text(move || {
                                        SharedString::from(backend_badge_label(
                                            store::auth_backend_status().backend,
                                        ))
                                    })
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(
                            view()
                                .px_2p5()
                                .py_1()
                                .rounded_full()
                                .bg(pill_glass_surface)
                                .child(
                                    text(move || {
                                        if store::auth_info().0 {
                                            SharedString::from("Connected")
                                        } else {
                                            SharedString::from("Anonymous")
                                        }
                                    })
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || {
                                        if store::auth_info().0 {
                                            color_status_ok()
                                        } else {
                                            theme().muted_foreground
                                        }
                                    }),
                                ),
                        ),
                ),
        )
        .child(
            text(move || {
                let (signed_in, acct) = store::auth_info();
                let status = store::auth_backend_status();
                if signed_in {
                    SharedString::from(format!(
                        "Connected: {} ({})",
                        acct.unwrap_or_else(|| "Google Account".into()),
                        backend_badge_label(status.backend)
                    ))
                } else {
                    SharedString::from(
                        "Anonymous mode active. On Desktop, open music.youtube.com in your browser, copy the Request Cookie header (including SAPISID / __Secure-3PAPISID) from DevTools Network, and paste it below.",
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
                                .test_id("desktop-cookie-input")
                                .value(move || cookie_draft.get())
                                .on_change(move |v: &str| cookie_draft.set(v.to_owned()))
                                .placeholder("Paste Cookie header (SAPISID=...; __Secure-3PAPISID=...)")
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
                        .child(
                            button()
                                .test_id("desktop-signin-btn")
                                .on_press(move || {
                                    let raw = cookie_draft.get_untracked();
                                    cookie_draft.set(String::new());
                                    store::sign_in_with_cookies(&raw);
                                })
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_5()
                                .items_center()
                                .justify_center()
                                .rounded(move || theme().radius)
                                .bg(move || theme().primary)
                                .hover(|s| s.opacity(0.92))
                                .active(|s| s.opacity(0.8))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .child(
                                    text("Sign In")
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().primary_foreground),
                                ),
                        )
                },
            )
            .otherwise(move || {
                button()
                    .test_id("desktop-signout-btn")
                    .on_press(move || {
                        cookie_draft.set(String::new());
                        store::sign_out();
                    })
                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                    .px_4()
                    .items_center()
                    .justify_center()
                    .rounded(move || theme().radius)
                    .bg(pill_glass_surface)
                    .border_1()
                    .border_color(glass_hairline)
                    .hover(|s| s.opacity(0.92))
                    .active(|s| s.opacity(0.8))
                    .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                    .child(
                        text("Sign Out")
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().foreground),
                    )
            }),
        )
}

fn preferences_card() -> impl IntoElement {
    view()
        .p_4()
        .gap_3()
        .rounded(radius_card)
        .bg(move || theme().card)
        .border_1()
        .border_color(glass_hairline)
        .child(
            view()
                .gap_0p5()
                .child(
                    text("Preferences")
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text("Playback, SponsorBlock auto-skip, offline chunking & appearance")
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
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
                .gap_2()
                .pt_1()
                .child(
                    text("Theme")
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().muted_foreground),
                )
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
}

fn cache_card() -> impl IntoElement {
    view()
        .p_4()
        .gap_3()
        .rounded(radius_card)
        .bg(move || theme().card)
        .border_1()
        .border_color(glass_hairline)
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
                                (s.audio_chunk_bytes as f32 / s.max_audio_bytes as f32) * 100.0
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
        .child(secondary_button("Clear Cache", store::clear_caches))
}

/// Reusable Settings, Account, Preferences & Offline Cache sections shared with `library_screen`.
pub fn settings_sections() -> impl IntoElement {
    let cookie_draft = signal(String::new());

    when(is_desktop_layout, move || {
        view()
            .w_full()
            .flex_row()
            .items_start()
            .gap_4()
            .child(view().flex_1().min_w(px(0.0)).gap_4().child(preferences_card()))
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_4()
                    .child(account_card(cookie_draft))
                    .child(cache_card()),
            )
    })
    .otherwise(move || {
        view()
            .w_full()
            .gap_4()
            .child(account_card(cookie_draft))
            .child(preferences_card())
            .child(cache_card())
    })
}

/// Dedicated Settings screen for `AppTab::Settings`.
pub fn settings_screen() -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .w_full()
            .max_w(px(1160.0))
            .p_4()
            .pb_8()
            .gap_5()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        view()
                            .w(px(40.0))
                            .h(px(40.0))
                            .rounded_full()
                            .bg(pill_glass_surface)
                            .border_1()
                            .border_color(glass_hairline)
                            .items_center()
                            .justify_center()
                            .child(icon(IconKind::Settings, 18.0, move || theme().foreground)),
                    )
                    .child(
                        view()
                            .flex_1()
                            .min_w(px(0.0))
                            .gap_0p5()
                            .child(
                                text("Settings")
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(
                                text("Account sign-in, playback preferences, theme & offline storage")
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                            ),
                    ),
            )
            .child(settings_sections()),
    )
}

/// Atmospheric Shared Radio Sessions screen for `AppTab::ListenTogether`.
pub fn listen_together_screen() -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .w_full()
            .max_w(px(1160.0))
            .p_4()
            .pb_8()
            .gap_5()
            // Atmospheric Shared Radio Hero Card
            .child(
                view()
                    .p_5()
                    .gap_4()
                    .rounded(radius_panel)
                    .bg_linear_to_br()
                    .gradient_from(color_ambient_warm)
                    .gradient_to(frosted_card_surface)
                    .border_1()
                    .border_color(glass_hairline)
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .flex_wrap()
                            .gap_3()
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        view()
                                            .w(px(44.0))
                                            .h(px(44.0))
                                            .rounded_full()
                                            .bg(pill_glass_surface)
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .items_center()
                                            .justify_center()
                                            .child(icon(IconKind::ListenTogether, 20.0, move || {
                                                theme().foreground
                                            })),
                                    )
                                    .child(
                                        view()
                                            .gap_0p5()
                                            .child(
                                                text("Listen Together")
                                                    .text_2xl()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(move || theme().foreground),
                                            )
                                            .child(
                                                text("Shared Radio Sessions • Continuous mixes seeded from your active track")
                                                    .text_xs()
                                                    .text_color(move || theme().muted_foreground),
                                            ),
                                    ),
                            )
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .px_3()
                                    .py_1p5()
                                    .rounded_full()
                                    .bg(pill_glass_surface)
                                    .border_1()
                                    .border_color(glass_hairline)
                                    .child(icon(IconKind::Radio, 14.0, move || {
                                        if store::player_snapshot().queue.autoplay_radio {
                                            color_status_ok()
                                        } else {
                                            theme().muted_foreground
                                        }
                                    }))
                                    .child(
                                        text(move || {
                                            if store::player_snapshot().queue.autoplay_radio {
                                                SharedString::from("Radio Live")
                                            } else {
                                                SharedString::from("Radio Paused")
                                            }
                                        })
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || theme().foreground),
                                    ),
                            ),
                    )
                    // Current Session Seed Track Preview
                    .child(dynamic(move || {
                        let snap = store::player_snapshot();
                        if let Some(track) = snap.current_track {
                            let mono = track.fallback_monogram();
                            let art = track.artwork_url.clone();
                            let title = track.title.clone();
                            let artist = track.artist_display();
                            let album = track
                                .album
                                .as_ref()
                                .map(|a| a.title.clone())
                                .unwrap_or_else(|| "Active Queue".into());

                            view()
                                .p_3()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .rounded(radius_card)
                                .bg(pill_glass_surface)
                                .border_1()
                                .border_color(glass_hairline)
                                .child(artwork_box(art, mono, 56.0, 8.0))
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .gap_0p5()
                                        .child(
                                            text("CURRENT RADIO SEED")
                                                .text_xs()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(color_player_accent),
                                        )
                                        .child(
                                            text(title)
                                                .number_of_lines(1)
                                                .text_base()
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text(format!("{artist} • {album}"))
                                                .number_of_lines(1)
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                        ),
                                )
                                .into_element()
                        } else {
                            view()
                                .p_3()
                                .rounded(radius_card)
                                .bg(pill_glass_surface)
                                .border_1()
                                .border_color(glass_hairline)
                                .child(
                                    text("Start a mix from any song in your Library, Home, or Queue.")
                                        .text_sm()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .into_element()
                        }
                    }))
                    // Session Action Pills
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .items_center()
                            .gap_2p5()
                            .child(hero_pill_button(
                                "Start Shared Mix",
                                Some(IconKind::Radio),
                                || {
                                    store::set_autoplay_radio(true);
                                    let tracks = collect_session_tracks();
                                    if let Some(current) = store::player_snapshot().current_track {
                                        if !store::player_snapshot().state.is_playing() {
                                            store::toggle_play_pause();
                                        }
                                        store::set_status_banner(format!(
                                            "Shared Radio Mix seeded from \"{}\"",
                                            current.title
                                        ));
                                    } else if let Some(first) = tracks.first().cloned() {
                                        store::play_track(first.clone());
                                        for extra in tracks.into_iter().skip(1) {
                                            store::play_last(extra);
                                        }
                                        store::set_status_banner(format!(
                                            "Started Shared Radio Mix with \"{}\"",
                                            first.title
                                        ));
                                    } else {
                                        store::set_tab(AppTab::Home);
                                    }
                                },
                            ))
                            .child(glass_pill_button(
                                "Shuffle Session",
                                Some(IconKind::Shuffle),
                                || {
                                    store::set_autoplay_radio(true);
                                    let tracks = collect_session_tracks();
                                    if !tracks.is_empty() {
                                        store::play_all_shuffled(tracks);
                                        store::set_status_banner(
                                            "Shuffled Shared Radio Mix session",
                                        );
                                    }
                                },
                            ))
                            .child(glass_pill_button(
                                "Open Queue",
                                Some(IconKind::Queue),
                                || store::set_tab(AppTab::Queue),
                            )),
                    ),
            )
            // Autoplay Radio switch
            .child(setting_switch(
                "Autoplay Radio",
                || store::player_snapshot().queue.autoplay_radio,
                store::set_autoplay_radio,
            ))
            // Session Mix Tracks
            .child(
                view()
                    .gap_2p5()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                text("Up Next in Session")
                                    .text_lg()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(
                                text(move || {
                                    let count = collect_session_tracks().len();
                                    SharedString::from(format!("{count} tracks"))
                                })
                                .text_xs()
                                .tabular_nums()
                                .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(dynamic(move || {
                        let snap = store::player_snapshot();
                        let current_id = snap.current_track.map(|t| t.id);
                        let tracks = collect_session_tracks();
                        if tracks.is_empty() {
                            return status_card(
                                "No tracks in session yet",
                                "Browse Home or Search to seed a Shared Radio Mix.",
                                Some("Browse Home"),
                                Some(Box::new(|| store::set_tab(AppTab::Home))),
                            )
                            .into_element();
                        }

                        view()
                            .gap_1()
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
                            .into_element()
                    })),
            ),
    )
}
