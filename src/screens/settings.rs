//! Dedicated Settings & Account screens (`src/screens/settings.rs`).

use std::rc::Rc;

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{CredentialBackendKind, PoTokenHealthStatus};

use crate::screens::settings_updater_card::{UpdaterCardProps, settings_updater_card};
use crate::store::{self, ThemeChoice};
use crate::ui::controls::{
    confirm_destructive, destructive_button, diagnostics_badge, labeled_input, secondary_button,
    setting_switch,
};
use crate::ui::icons::{IconKind, icon};
use crate::ui::text_format::pluralize;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, PROSE_MAX_WIDTH, color_status_ok, glass_hairline, is_desktop_layout,
    pill_glass_surface, radius_card,
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

fn account_card(cookie_draft: Signal<String>) -> impl IntoElement {
    let show_cookie_fallback = signal(false);
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
                let profile = store::account_profile();
                if signed_in {
                    let tier = if profile.as_ref().is_some_and(|p| p.is_premium) {
                        "YouTube Music Premium"
                    } else {
                        "Free Tier"
                    };
                    let handle = profile
                        .and_then(|p| p.channel_handle)
                        .map(|h| format!(" ({h})"))
                        .unwrap_or_default();
                    SharedString::from(format!(
                        "Connected: {}{} • {} • {}",
                        acct.unwrap_or_else(|| "Google Account".into()),
                        handle,
                        tier,
                        backend_badge_label(status.backend)
                    ))
                } else {
                    SharedString::from(
                        "Anonymous mode active. Sign in with YouTube TV Device Code (recommended on Desktop) or Android WebView to sync playlists, likes, and Premium audio.",
                    )
                }
            })
            .max_w(px(PROSE_MAX_WIDTH))
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
        .child(
            when(
                || !store::auth_info().0,
                move || {
                    view()
                        .gap_2p5()
                        .child(
                            button()
                                .test_id("open-auth-sheet-btn")
                                .on_press(store::open_auth_sheet)
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
                                    text("Sign In (TV Code / Browser)")
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().primary_foreground),
                                ),
                        )
                        .child(secondary_button(
                            move || {
                                if show_cookie_fallback.get() {
                                    SharedString::from("Hide Advanced Cookie Paste")
                                } else {
                                    SharedString::from("Advanced: Paste Browser Cookie")
                                }
                            },
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
                                            .rounded(move || theme().radius)
                                            .bg(pill_glass_surface)
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .hover(|s| s.opacity(0.92))
                                            .active(|s| s.opacity(0.8))
                                            .focus_visible(|s| {
                                                s.outline(px(2.0))
                                                    .outline_color(move || theme().ring)
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
            )
            .otherwise(move || {
                button()
                    .test_id("desktop-signout-btn")
                    .on_press(move || {
                        confirm_destructive(
                            "Sign out of YouTube Music?",
                            "Your saved credentials will be removed from this device and you will return to anonymous mode.",
                            "Sign Out",
                            move || {
                                cookie_draft.set(String::new());
                                store::sign_out();
                            },
                        );
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

fn stream_diagnostics_card() -> impl IntoElement {
    let expanded = signal(false);

    view()
        .p_4()
        .gap_2p5()
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
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap_0p5()
                        .child(
                            text("Advanced & Diagnostics")
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text("Stream resolver status, attestation & signed remote config")
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(secondary_button(
                    move || {
                        if expanded.get() {
                            SharedString::from("Hide")
                        } else {
                            SharedString::from("Show")
                        }
                    },
                    move || expanded.set(!expanded.get_untracked()),
                )),
        )
        .child(when(
            move || expanded.get(),
            move || {
                view()
                    .pt_2()
                    .gap_2p5()
                    .border_t_1()
                    .border_color(glass_hairline)
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                text("Stream Acquisition & PO Token")
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(move || theme().foreground),
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
                                || {
                                    store::po_token_diagnostics().status
                                        == PoTokenHealthStatus::Healthy
                                },
                                || {
                                    store::po_token_diagnostics().status
                                        == PoTokenHealthStatus::DegradedColdStart
                                },
                            )),
                    )
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

/// Reusable Settings, Account, Preferences, Audio DSP & Offline Cache sections shared with `library_screen`.
pub fn settings_sections() -> impl IntoElement {
    let cookie_draft = signal(String::new());

    when(is_desktop_layout, move || {
        view()
            .w_full()
            .flex_row()
            .items_start()
            .gap_4()
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_4()
                    .child(preferences_card())
                    .child(crate::screens::settings_dsp_card::dsp_settings_section())
                    .child(updater_card()),
            )
            .child(
                view()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap_4()
                    .child(account_card(cookie_draft))
                    .child(stream_diagnostics_card())
                    .child(cache_card()),
            )
    })
    .otherwise(move || {
        view()
            .w_full()
            .gap_4()
            .child(account_card(cookie_draft))
            .child(stream_diagnostics_card())
            .child(preferences_card())
            .child(crate::screens::settings_dsp_card::dsp_settings_section())
            .child(updater_card())
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
