//! Dedicated Settings Screen matching `ytm-rs` (`06_settings_01_account_1.png` – `06_settings_08_about_2.png`):
//! - Left column (240px on Desktop, wrapping pill nav on Mobile):
//!   Bold `Settings` title, `Search settings` pill input, and 8 exact category tabs:
//!   `Account`, `Playback`, `Lyrics`, `Appearance`, `Connections`, `General`, `Keyboard shortcuts`, `About`
//! - Right column: Category heading + frosted translucent cards (`frosted_settings_card`)

use std::rc::Rc;

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::{CredentialBackendKind, PoTokenHealthStatus};

use crate::screens::settings_updater_card::{UpdaterCardProps, settings_updater_card};
use crate::store::{self, SettingsSection, ThemeChoice};
use crate::ui::controls::{
    confirm_destructive, destructive_button, diagnostics_badge, glass_pill_button, labeled_input,
    secondary_button, set_text_input_focused, setting_switch,
};
use crate::ui::icons::{IconKind, icon};
use crate::ui::text_format::pluralize;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, PROSE_MAX_WIDTH, color_status_ok, frosted_settings_card,
    glass_hairline, is_desktop_layout, pill_glass_hover, pill_glass_surface, radius_card,
    radius_pill, sidebar_active_surface,
};

fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn backend_badge_label(kind: CredentialBackendKind) -> &'static str {
    match kind {
        CredentialBackendKind::OsKeyring => "OS Keychain",
        CredentialBackendKind::EncryptedFallbackFile => "Encrypted Vault",
        CredentialBackendKind::MemoryOnly => "Session Only",
    }
}

fn glass_card() -> View {
    view()
        .p_5()
        .gap_3p5()
        .rounded(radius_card)
        .bg(frosted_settings_card)
        .border_1()
        .border_color(glass_hairline)
}

fn setting_info_row(title: &'static str, description: &'static str, value: &'static str) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .py_1()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap_0p5()
                .child(
                    text(title)
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(description)
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            view()
                .px_3()
                .py_1()
                .rounded(radius_pill)
                .bg(pill_glass_surface)
                .border_1()
                .border_color(glass_hairline)
                .child(
                    text(value)
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || theme().foreground),
                ),
        )
}

fn shortcut_row(action: &'static str, keys: &'static str) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_4()
        .py_1()
        .child(
            text(action)
                .flex_1()
                .text_sm()
                .text_color(move || theme().foreground),
        )
        .child(
            view()
                .px_2p5()
                .py_1()
                .rounded(move || theme().radius)
                .bg(pill_glass_surface)
                .border_1()
                .border_color(glass_hairline)
                .child(
                    text(keys)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                ),
        )
}

// ── 1. Account Section ──────────────────────────────────────────────────────

fn account_card(cookie_draft: Signal<String>) -> impl IntoElement {
    let show_cookie_fallback = signal(false);
    glass_card()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_4()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(200.0))
                        .gap_1()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    text(move || {
                                        let (signed_in, acct) = store::auth_info();
                                        if signed_in {
                                            SharedString::from(
                                                acct.unwrap_or_else(|| "Google Account".into()),
                                            )
                                        } else {
                                            SharedString::from("YouTube Music Account")
                                        }
                                    })
                                    .text_base()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || theme().foreground),
                                )
                                .child(
                                    view()
                                        .flex_shrink_0()
                                        .min_w(px(88.0))
                                        .px_2p5()
                                        .py_0p5()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(pill_glass_surface)
                                        .child(
                                            text(move || {
                                                if store::auth_info().0 {
                                                    if store::account_profile()
                                                        .is_some_and(|p| p.is_premium)
                                                    {
                                                        SharedString::from("Premium")
                                                    } else {
                                                        SharedString::from("Connected")
                                                    }
                                                } else {
                                                    SharedString::from("Anonymous")
                                                }
                                            })
                                            .number_of_lines(1)
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
                        )
                        .child(
                            text(move || {
                                let (signed_in, _) = store::auth_info();
                                let status = store::auth_backend_status();
                                if signed_in {
                                    SharedString::from(format!(
                                        "Signed in with Google. Your home feed, library, and likes come from this account. Saved in {}.",
                                        backend_badge_label(status.backend)
                                    ))
                                } else {
                                    SharedString::from(
                                        "Sign in with YouTube TV Device Code or browser cookies to sync playlists, likes, and Premium audio.",
                                    )
                                }
                            })
                            .max_w(px(PROSE_MAX_WIDTH))
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    when(
                        || !store::auth_info().0,
                        move || {
                            button()
                                .test_id("open-auth-sheet-btn")
                                .on_press(store::open_auth_sheet)
                                .min_h(px(40.0))
                                .px_4()
                                .flex_row()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .rounded(radius_pill)
                                .bg(move || theme().primary)
                                .hover(|s| s.opacity(0.92))
                                .active(|s| s.opacity(0.8))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .child(
                                    text("Sign in")
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().primary_foreground),
                                )
                        },
                    )
                    .otherwise(move || {
                        button()
                            .test_id("desktop-signout-btn")
                            .on_press(move || {
                                confirm_destructive(
                                    "Sign out of YouTube Music?",
                                    "Your saved credentials will be removed from this device and you will return to anonymous mode.",
                                    "Sign out",
                                    move || {
                                        cookie_draft.set(String::new());
                                        store::sign_out();
                                    },
                                );
                            })
                            .min_h(px(40.0))
                            .px_4()
                            .flex_row()
                            .items_center()
                            .justify_center()
                            .gap_2()
                            .rounded(radius_pill)
                            .bg(pill_glass_surface)
                            .border_1()
                            .border_color(glass_hairline)
                            .hover(|s| s.bg(pill_glass_hover))
                            .active(|s| s.opacity(0.8))
                            .focus_visible(|s| {
                                s.outline(px(2.0)).outline_color(move || theme().ring)
                            })
                            .child(icon(IconKind::LogOut, 15.0, move || theme().foreground))
                            .child(
                                text("Sign out")
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || theme().foreground),
                            )
                    }),
                ),
        )
        .child(when(
            || !store::auth_info().0,
            move || {
                view()
                    .pt_2()
                    .border_t_1()
                    .border_color(glass_hairline)
                    .gap_2p5()
                    .child(glass_pill_button(
                        move || {
                            if show_cookie_fallback.get() {
                                SharedString::from("Hide advanced cookie paste")
                            } else {
                                SharedString::from("Advanced: Paste browser cookie")
                            }
                        },
                        None,
                        move || show_cookie_fallback.set(!show_cookie_fallback.get_untracked()),
                    ))
                    .child(when(
                        move || show_cookie_fallback.get(),
                        move || {
                            view()
                                .gap_2()
                                .child(labeled_input(
                                    "Browser Cookie Header",
                                    "desktop-cookie-input",
                                    "Paste Cookie header (SAPISID=…; __Secure-3PAPISID=…)",
                                    move || cookie_draft.get(),
                                    move |v: &str| cookie_draft.set(v.to_owned()),
                                ))
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
                                        .rounded(radius_pill)
                                        .bg(pill_glass_surface)
                                        .border_1()
                                        .border_color(glass_hairline)
                                        .hover(|s| s.bg(pill_glass_hover))
                                        .active(|s| s.opacity(0.8))
                                        .focus_visible(|s| {
                                            s.outline(px(2.0)).outline_color(move || theme().ring)
                                        })
                                        .child(
                                            text("Sign In with Cookies")
                                                .text_sm()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || theme().foreground),
                                        ),
                                )
                        },
                    ))
            },
        ))
}

// ── 2. Playback Section ─────────────────────────────────────────────────────

fn playback_section() -> impl IntoElement {
    let smart_shuffle = signal(true);

    view()
        .gap_4()
        .child(
            glass_card()
                .child(
                    text("Queue & Playback")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(setting_switch(
                    "Autoplay similar songs",
                    || store::player_snapshot().queue.autoplay_radio,
                    store::set_autoplay_radio,
                ))
                .child(setting_switch(
                    "Smart shuffle (avoid repeating artists back-to-back)",
                    move || smart_shuffle.get(),
                    move |v| smart_shuffle.set(v),
                ))
                .child(setting_switch(
                    "SponsorBlock (auto-skip non-music)",
                    || store::player_snapshot().sponsorblock_enabled,
                    store::set_sponsorblock_enabled,
                ))
                .child(setting_switch(
                    "Offline Mode (local chunk cache only)",
                    || store::player_snapshot().offline_mode,
                    store::set_offline_mode,
                )),
        )
        .child(crate::screens::settings_dsp_card::dsp_settings_section())
}

// ── 3. Lyrics Section ───────────────────────────────────────────────────────

fn lyrics_section() -> impl IntoElement {
    let romanization = signal(false);
    let translation = signal(false);
    let target_lang = signal("English");
    let lyrics_style = signal("Karaoke");
    let text_size = signal(22u32);
    let line_weight = signal("Bold");

    view()
        .gap_4()
        .child(
            glass_card()
                .child(
                    text("Language & Translation")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(setting_switch(
                    "Romanization (show Latin script for non-Latin lyrics)",
                    move || romanization.get(),
                    move |v| romanization.set(v),
                ))
                .child(setting_switch(
                    "Translation (display translated lines below original)",
                    move || translation.get(),
                    move |v| translation.set(v),
                ))
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            text("Translate into")
                                .text_sm()
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .flex_wrap()
                                .gap_1p5()
                                .children(["English", "Hindi", "Spanish", "Japanese"].into_iter().map(
                                    move |lang| {
                                        let is_sel = move || target_lang.get() == lang;
                                        button()
                                            .accessibility_label(lang)
                                            .selected(is_sel)
                                            .min_h(px(34.0))
                                            .px_3()
                                            .items_center()
                                            .justify_center()
                                            .rounded(radius_pill)
                                            .bg(move || {
                                                if is_sel() {
                                                    sidebar_active_surface()
                                                } else {
                                                    pill_glass_surface()
                                                }
                                            })
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .on_press(move || target_lang.set(lang))
                                            .child(
                                                text(lang)
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || theme().foreground),
                                            )
                                    },
                                )),
                        ),
                ),
        )
        .child(
            glass_card()
                .child(
                    text("Lyrics Appearance")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            text("Lyrics style")
                                .text_sm()
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .gap_1p5()
                                .children(["Karaoke", "Compact", "Classic"].into_iter().map(
                                    move |st| {
                                        let is_sel = move || lyrics_style.get() == st;
                                        button()
                                            .accessibility_label(st)
                                            .selected(is_sel)
                                            .min_h(px(34.0))
                                            .px_3()
                                            .items_center()
                                            .justify_center()
                                            .rounded(radius_pill)
                                            .bg(move || {
                                                if is_sel() {
                                                    sidebar_active_surface()
                                                } else {
                                                    pill_glass_surface()
                                                }
                                            })
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .on_press(move || lyrics_style.set(st))
                                            .child(
                                                text(st)
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || theme().foreground),
                                            )
                                    },
                                )),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            text("Text size")
                                .text_sm()
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(glass_pill_button("−", None, move || {
                                    text_size.set(text_size.get_untracked().saturating_sub(2).max(14));
                                }))
                                .child(
                                    text(move || SharedString::from(format!("{} px", text_size.get())))
                                        .text_xs()
                                        .tabular_nums()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(glass_pill_button("+", None, move || {
                                    text_size.set((text_size.get_untracked() + 2).min(36));
                                })),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            text("Line weight")
                                .text_sm()
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .gap_1p5()
                                .children(["Regular", "Semibold", "Bold"].into_iter().map(
                                    move |w| {
                                        let is_sel = move || line_weight.get() == w;
                                        button()
                                            .accessibility_label(w)
                                            .selected(is_sel)
                                            .min_h(px(34.0))
                                            .px_3()
                                            .items_center()
                                            .justify_center()
                                            .rounded(radius_pill)
                                            .bg(move || {
                                                if is_sel() {
                                                    sidebar_active_surface()
                                                } else {
                                                    pill_glass_surface()
                                                }
                                            })
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .on_press(move || line_weight.set(w))
                                            .child(
                                                text(w)
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(move || theme().foreground),
                                            )
                                    },
                                )),
                        ),
                ),
        )
}

// ── 4. Appearance Section ───────────────────────────────────────────────────

fn appearance_section() -> impl IntoElement {
    let dynamic_bg = signal(true);

    glass_card()
        .child(
            view()
                .gap_2()
                .child(
                    text("Theme")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
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
                                if store::theme_choice() == ThemeChoice::Dark {
                                    SharedString::from("✓ Dark Theme")
                                } else {
                                    SharedString::from("Dark Theme")
                                }
                            },
                            || store::set_theme_choice(ThemeChoice::Dark),
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
                        )),
                ),
        )
        .child(setting_switch(
            "Dynamic ambient background (extract palette from active album art)",
            move || dynamic_bg.get(),
            move |v| {
                dynamic_bg.set(v);
                store::set_reduced_motion(!v);
            },
        ))
        .child(setting_switch(
            "Reduce Motion",
            store::reduced_motion,
            store::set_reduced_motion,
        ))
}

// ── 5. Connections Section ──────────────────────────────────────────────────

fn connections_section() -> impl IntoElement {
    let stream_deck = signal(false);
    let lastfm = signal(false);
    let listenbrainz = signal(false);
    let lb_token = signal(String::new());
    let discord_rpc = signal(true);

    view()
        .gap_4()
        .child(
            glass_card()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            view()
                                .flex_1()
                                .gap_0p5()
                                .child(
                                    text("Cast & Audio Devices")
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text("Stream to Chromecast, AirPlay, or DLNA speakers on your network.")
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(glass_pill_button(
                            "Cast to Device",
                            Some(IconKind::Cast),
                            || store::set_cast_sheet_open(true),
                        )),
                )
                .child(setting_switch(
                    "Stream Deck bridge (local WebSocket transport control)",
                    move || stream_deck.get(),
                    move |v| stream_deck.set(v),
                )),
        )
        .child(
            glass_card()
                .child(
                    text("Scrobbling & Rich Presence")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(setting_switch(
                    "Last.fm scrobbling",
                    move || lastfm.get(),
                    move |v| lastfm.set(v),
                ))
                .child(setting_switch(
                    "ListenBrainz scrobbling",
                    move || listenbrainz.get(),
                    move |v| listenbrainz.set(v),
                ))
                .child(when(
                    move || listenbrainz.get(),
                    move || {
                        labeled_input(
                            "ListenBrainz User Token",
                            "settings-listenbrainz-token",
                            "Paste your ListenBrainz user token",
                            move || lb_token.get(),
                            move |v: &str| lb_token.set(v.to_owned()),
                        )
                    },
                ))
                .child(setting_switch(
                    "Discord Rich Presence (show active track on your Discord profile)",
                    move || discord_rpc.get(),
                    move |v| discord_rpc.set(v),
                )),
        )
}

// ── 6. General & Cache Section ──────────────────────────────────────────────

fn cache_card() -> impl IntoElement {
    glass_card()
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
                            let frac = if s.max_audio_bytes > 0 {
                                s.audio_chunk_bytes as f32 / s.max_audio_bytes as f32
                            } else {
                                0.0
                            };
                            percentage(frac.clamp(0.0, 1.0))
                        })
                        .bg(move || theme().primary),
                ),
        )
        .child(
            text(move || {
                let s = store::cache_stats();
                SharedString::from(format!(
                    "Audio chunks: {} ({}) • Artwork: {} • Metadata: {}",
                    format_mb(s.audio_chunk_bytes),
                    pluralize(s.audio_chunk_count as usize, "chunk", "chunks"),
                    format_mb(s.artwork_bytes),
                    pluralize(s.metadata_entries as usize, "row", "rows")
                ))
            })
            .test_id("cache-stats-readout")
            .text_xs()
            .tabular_nums()
            .text_color(move || theme().muted_foreground),
        )
        .child(destructive_button("Clear Cache", || {
            confirm_destructive(
                "Clear offline cache?",
                "Remove cached audio chunks, artwork, and metadata from disk? Pinned downloads stay intact.",
                "Clear Cache",
                store::clear_caches,
            );
        }))
}

fn general_section() -> impl IntoElement {
    let open_startup = signal(false);
    let close_tray = signal(true);

    view()
        .gap_4()
        .child(
            glass_card()
                .child(
                    text("Language & Region")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(setting_info_row(
                    "App language",
                    "Interface text and date formatting.",
                    "English (US)",
                ))
                .child(setting_info_row(
                    "Song info language & country",
                    "Regional charts, localized titles, and catalog recommendations.",
                    "Auto (System)",
                )),
        )
        .child(
            glass_card()
                .child(
                    text("Desktop Behavior")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(setting_switch(
                    "Open on startup",
                    move || open_startup.get(),
                    move |v| open_startup.set(v),
                ))
                .child(setting_switch(
                    "Close to system tray (keep playing in background)",
                    move || close_tray.get(),
                    move |v| close_tray.set(v),
                )),
        )
        .child(cache_card())
}

// ── 7. Keyboard Shortcuts Section ───────────────────────────────────────────

fn shortcuts_section() -> impl IntoElement {
    let global_media_keys = signal(true);

    view()
        .gap_4()
        .child(
            glass_card()
                .child(
                    text("System-wide shortcuts")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(setting_switch(
                    "Hardware media keys (MPRIS / macOS Now Playing / SMTC)",
                    move || global_media_keys.get(),
                    move |v| global_media_keys.set(v),
                )),
        )
        .child(
            glass_card()
                .child(
                    text("In-app keyboard shortcuts")
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(shortcut_row("Play / Pause", "Space / K"))
                .child(shortcut_row("Next track", "N / Ctrl + →"))
                .child(shortcut_row("Previous track", "P / Ctrl + ←"))
                .child(shortcut_row("Seek forward 5s", "→ / L"))
                .child(shortcut_row("Seek backward 5s", "← / J"))
                .child(shortcut_row("Mute / Unmute", "M"))
                .child(shortcut_row("Volume up / down", "↑ / ↓")),
        )
}

// ── 8. About & Diagnostics Section ──────────────────────────────────────────

fn stream_diagnostics_card() -> impl IntoElement {
    let expanded = signal(true);

    glass_card()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            text("Stream Acquisition & PO Token")
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text("Stream resolver status, attestation & signed remote config")
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(diagnostics_badge(
                    move || {
                        let diag = store::po_token_diagnostics();
                        match diag.status {
                            PoTokenHealthStatus::Healthy => {
                                SharedString::from("PO Token: Healthy")
                            }
                            PoTokenHealthStatus::DegradedColdStart => {
                                SharedString::from("Degraded Fallback")
                            }
                            PoTokenHealthStatus::Unconfigured => {
                                SharedString::from("Fallback Matrix")
                            }
                        }
                    },
                    || store::po_token_diagnostics().status == PoTokenHealthStatus::Healthy,
                    || {
                        store::po_token_diagnostics().status
                            == PoTokenHealthStatus::DegradedColdStart
                    },
                )),
        )
        .child(when(
            move || expanded.get(),
            move || {
                view()
                    .pt_2()
                    .gap_2()
                    .border_t_1()
                    .border_color(glass_hairline)
                    .child(
                        text(move || {
                            let diag = store::po_token_diagnostics();
                            SharedString::from(format!(
                                "{} • {}",
                                diag.active_provider, diag.detail
                            ))
                        })
                        .test_id("po-token-diagnostics-detail")
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                    )
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                text(move || {
                                    let rc = store::remote_config_status();
                                    let sig = if rc.signature_verified {
                                        "Ed25519 verified"
                                    } else {
                                        "Embedded fallback"
                                    };
                                    SharedString::from(format!(
                                        "Remote Config rev {} ({}) • Solver {} • {}",
                                        rc.matrix_revision,
                                        rc.matrix_source,
                                        rc.solver_bundle_version,
                                        sig
                                    ))
                                })
                                .test_id("remote-config-status-detail")
                                .text_xs()
                                .tabular_nums()
                                .text_color(move || theme().muted_foreground),
                            )
                            .child(secondary_button(
                                "Sync Config",
                                store::refresh_remote_config,
                            )),
                    )
            },
        ))
}

fn updater_card() -> impl IntoElement {
    let current_version = store::updater_state().current_version;
    settings_updater_card(UpdaterCardProps {
        current_version,
        channel: store::synced_signal(store::updater_channel),
        auto_check: store::synced_signal(store::updater_auto_check),
        phase: store::synced_signal(store::updater_phase),
        is_playing: store::synced_signal(|| {
            store::player_snapshot().state == resonance_contract::PlayerState::Playing
        }),
        on_check_now: Rc::new(store::check_for_updates_now),
        on_toggle_auto_check: Rc::new(store::set_updater_auto_check),
        on_select_channel: Rc::new(store::set_updater_channel),
        on_download: Rc::new(store::download_update),
        on_open_release: Rc::new(store::open_release_url),
        on_skip_version: Rc::new(store::skip_updater_version),
        on_install_on_quit: Rc::new(store::install_update_on_quit),
        on_restart_now: Rc::new(store::restart_to_install_update),
    })
}

fn about_section() -> impl IntoElement {
    view()
        .gap_4()
        .child(updater_card())
        .child(
            glass_card()
                .child(
                    text("About Raag")
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(setting_info_row(
                    "Client",
                    "Pure Rust YouTube Music desktop & Android player built on NSH.",
                    "Raag",
                ))
                .child(setting_info_row(
                    "Lyrics providers",
                    "Synced karaoke & fallback plain lyrics pipeline.",
                    "LRCLIB • InnerTube",
                ))
                .child(setting_info_row(
                    "License",
                    "Free and open-source software.",
                    "GPL-3.0-or-later",
                )),
        )
        .child(stream_diagnostics_card())
}

fn render_section_content(section: SettingsSection, cookie_draft: Signal<String>) -> AnyElement {
    match section {
        SettingsSection::Account => account_card(cookie_draft).into_element(),
        SettingsSection::Playback => playback_section().into_element(),
        SettingsSection::Lyrics => lyrics_section().into_element(),
        SettingsSection::Appearance => appearance_section().into_element(),
        SettingsSection::Connections => connections_section().into_element(),
        SettingsSection::General => general_section().into_element(),
        SettingsSection::KeyboardShortcuts => shortcuts_section().into_element(),
        SettingsSection::About => about_section().into_element(),
    }
}

fn settings_nav_item(section: SettingsSection) -> impl IntoElement {
    let active = move || store::settings_section() == section;
    button()
        .test_id(section.test_id())
        .accessibility_label(section.label())
        .selected(active)
        .w_full()
        .min_h(px(40.0))
        .px_3p5()
        .justify_center()
        .rounded(move || theme().radius)
        .style_when(active, |s| s.bg(sidebar_active_surface))
        .hover(|s| s.bg(pill_glass_surface))
        .active(|s| s.opacity(0.8))
        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
        .on_press(move || {
            store::set_settings_search_query(String::new());
            store::set_settings_section(section);
        })
        .child(
            text(section.label())
                .number_of_lines(1)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if active() {
                        theme().foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}

fn settings_search_results(query: &str, cookie_draft: Signal<String>) -> AnyElement {
    let q = query.trim().to_lowercase();
    let sections_keywords: &[(SettingsSection, &str)] = &[
        (
            SettingsSection::Account,
            "account google sign in sign out cookie keyring premium profile",
        ),
        (
            SettingsSection::Playback,
            "playback autoplay shuffle gapless crossfade silence loudness eq equalizer dsp sponsorblock offline",
        ),
        (
            SettingsSection::Lyrics,
            "lyrics romanization translation translate karaoke text size weight",
        ),
        (
            SettingsSection::Appearance,
            "appearance theme dark light system dynamic ambient background reduce motion",
        ),
        (
            SettingsSection::Connections,
            "connections cast chromecast airplay dlna stream deck lastfm listenbrainz discord rpc",
        ),
        (
            SettingsSection::General,
            "general language country region startup tray cache storage clear",
        ),
        (
            SettingsSection::KeyboardShortcuts,
            "keyboard shortcuts hotkeys media keys mpris space play pause seek",
        ),
        (
            SettingsSection::About,
            "about updates version raag license gpl diagnostics po token remote config",
        ),
    ];

    let matched: Vec<SettingsSection> = sections_keywords
        .iter()
        .filter(|(sec, kw)| sec.label().to_lowercase().contains(&q) || kw.contains(&q))
        .map(|(sec, _)| *sec)
        .collect();

    if matched.is_empty() {
        return glass_card()
            .items_center()
            .child(
                text(format!("No settings match \"{query}\""))
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().foreground),
            )
            .child(glass_pill_button("Clear search", None, || {
                store::set_settings_search_query(String::new());
            }))
            .into_element();
    }

    view()
        .gap_6()
        .children(matched.into_iter().map(move |sec| {
            view()
                .gap_3()
                .child(
                    text(sec.label())
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().foreground),
                )
                .child(render_section_content(sec, cookie_draft))
        }))
        .into_element()
}

/// Reusable Settings sections (used by Settings screen).
pub fn settings_sections() -> impl IntoElement {
    let cookie_draft = signal(String::new());
    let desktop_memo = computed(is_desktop_layout);

    dynamic(move || {
        let desktop = desktop_memo.get();
        let search_q = store::settings_search_query();

        let search_box = view()
            .w_full()
            .min_h(px(if desktop { 38.0 } else { MOBILE_MIN_TOUCH_TARGET }))
            .flex_row()
            .items_center()
            .gap_2()
            .px_3p5()
            .rounded(radius_pill)
            .bg(pill_glass_surface)
            .border_1()
            .border_color(glass_hairline)
            .child(icon(IconKind::Search, 14.0, move || {
                theme().muted_foreground
            }))
            .child(
                input()
                    .test_id("settings-search-input")
                    .value(store::settings_search_query)
                    .on_change(|v: &str| store::set_settings_search_query(v.to_owned()))
                    .on_focus(|| set_text_input_focused(true))
                    .on_blur(|| set_text_input_focused(false))
                    .placeholder("Search settings")
                    .placeholder_color(|| color(theme().muted_foreground))
                    .caret_color(|| color(theme().primary))
                    .flex_1()
                    .min_w(px(0.0))
                    .text_xs()
                    .text_color(move || theme().foreground)
                    .border_color(move || theme().input),
            );

        if desktop {
            let active_sec = store::settings_section();
            view()
                .w_full()
                .flex_row()
                .items_start()
                .gap_8()
                // Left column: Settings title, Search settings pill, 8 category tabs
                .child(
                    view()
                        .w(px(230.0))
                        .gap_4()
                        .child(
                            text("Settings")
                                .text_3xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(search_box)
                        .child(
                            view()
                                .gap_1()
                                .children(
                                    SettingsSection::all()
                                        .iter()
                                        .copied()
                                        .map(settings_nav_item),
                                ),
                        ),
                )
                // Right column: Selected category heading + translucent glass cards
                .child(
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .max_w(px(720.0))
                        .gap_4()
                        .when(search_q.trim().is_empty(), move |col| {
                            col.child(
                                text(active_sec.label())
                                    .text_xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || theme().foreground),
                            )
                            .child(render_section_content(active_sec, cookie_draft))
                        })
                        .when(!search_q.trim().is_empty(), move |col| {
                            col.child(settings_search_results(&search_q, cookie_draft))
                        }),
                )
                .into_element()
        } else {
            let active_sec = store::settings_section();
            view()
                .w_full()
                .gap_4()
                .child(
                    text("Settings")
                        .text_2xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(move || theme().foreground),
                )
                .child(search_box)
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .children(SettingsSection::all().iter().copied().map(|sec| {
                            let is_sel = move || store::settings_section() == sec;
                            button()
                                .test_id(sec.test_id())
                                .accessibility_label(sec.label())
                                .selected(is_sel)
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_3p5()
                                .items_center()
                                .justify_center()
                                .rounded(radius_pill)
                                .bg(move || {
                                    if is_sel() {
                                        sidebar_active_surface()
                                    } else {
                                        pill_glass_surface()
                                    }
                                })
                                .border_1()
                                .border_color(glass_hairline)
                                .on_press(move || {
                                    store::set_settings_search_query(String::new());
                                    store::set_settings_section(sec);
                                })
                                .child(
                                    text(sec.label())
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                )
                        })),
                )
                .when(search_q.trim().is_empty(), move |col| {
                    col.child(
                        text(active_sec.label())
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(render_section_content(active_sec, cookie_draft))
                })
                .when(!search_q.trim().is_empty(), move |col| {
                    col.child(settings_search_results(&search_q, cookie_draft))
                })
                .into_element()
        }
    })
}

/// Dedicated Settings screen for `AppTab::Settings`.
pub fn settings_screen() -> impl IntoElement {
    scroll().flex_1().min_h(px(0.0)).pb_safe_area().child(
        view()
            .w_full()
            .p_4()
            .style_when(is_desktop_layout, |s| s.px_8().py_6())
            .pb_8()
            .child(settings_sections()),
    )
}
