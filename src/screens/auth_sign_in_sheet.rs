//! Single Unified Sign-In Modal Sheet (`src/screens/auth_sign_in_sheet.rs`, `H3.1`).
//!
//! 1. Primary (Desktop): Embedded NSH WebView Sign-In (`accounts.google.com`)
//! 2. Universal Fallback: RFC 8628 YouTube TV OAuth Device-Code Flow
//! 3. Advanced: Collapsible Manual Cookie Header Paste
//!
//! Zero raw color literals — 100% semantic `theme.json` & `src/ui/tokens` tokens.

use std::sync::Arc;

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use crate::ui::controls::{
    labeled_input, modal_panel_container, primary_button, secondary_button,
};
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, glass_hairline, overlay_scrim, pill_glass_hover, pill_glass_surface,
    radius_card, radius_pill,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthSheetTab {
    #[default]
    WebView,
    TvOAuth,
}

#[derive(Clone)]
pub struct AuthSignInCallbacks {
    pub on_close: Arc<dyn Fn()>,
    pub on_start_tv_oauth: Arc<dyn Fn()>,
    pub on_cancel_tv_oauth: Arc<dyn Fn()>,
    pub on_submit_cookies: Arc<dyn Fn(String)>,
}

impl Default for AuthSignInCallbacks {
    fn default() -> Self {
        Self {
            on_close: Arc::new(|| {}),
            on_start_tv_oauth: Arc::new(|| {}),
            on_cancel_tv_oauth: Arc::new(|| {}),
            on_submit_cookies: Arc::new(|raw| crate::store::sign_in_with_cookies(&raw)),
        }
    }
}

pub fn auth_sign_in_sheet(
    open: Signal<bool>,
    tv_user_code: Signal<Option<String>>,
    tv_verification_url: Signal<Option<String>>,
    tv_countdown_secs: Signal<u64>,
    cb: AuthSignInCallbacks,
) -> impl IntoElement {
    #[cfg(target_os = "android")]
    let initial_tab = AuthSheetTab::WebView;
    #[cfg(not(target_os = "android"))]
    let initial_tab = AuthSheetTab::TvOAuth;

    let active_tab = signal(initial_tab);
    let show_advanced_cookies = signal(false);
    let cookie_draft = signal(String::new());

    when(
        move || open.get(),
        move || {
            let cb_close = Arc::clone(&cb.on_close);
            let cb_start_tv = Arc::clone(&cb.on_start_tv_oauth);
            let cb_start_tv_2 = Arc::clone(&cb.on_start_tv_oauth);
            let cb_cancel_tv = Arc::clone(&cb.on_cancel_tv_oauth);
            let cb_cookies = Arc::clone(&cb.on_submit_cookies);

            view()
                .test_id("auth-sign-in-sheet")
                .absolute()
                .inset_0()
                .bg(overlay_scrim)
                .items_center()
                .justify_center()
                .p_4()
                .child(modal_panel_container(
                    "Sign in to YouTube Music",
                    "Credentials are stored only in OS Keychain / Keystore.",
                    view()
                        .w_full()
                        .gap_3()
                        .child(
                            view()
                                .flex_row()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    button()
                                        .selected(move || active_tab.get() == AuthSheetTab::TvOAuth)
                                        .on_press(move || active_tab.set(AuthSheetTab::TvOAuth))
                                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                        .px_4()
                                        .flex_row()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .rounded(radius_pill)
                                        .bg(move || {
                                            if active_tab.get() == AuthSheetTab::TvOAuth {
                                                theme().primary
                                            } else {
                                                pill_glass_surface()
                                            }
                                        })
                                        .border_1()
                                        .border_color(glass_hairline)
                                        .hover(|s| s.opacity(0.92))
                                        .focus_visible(|s| {
                                            s.outline(px(2.0)).outline_color(move || theme().ring)
                                        })
                                        .child(icon(IconKind::Radio, 15.0, move || {
                                            if active_tab.get() == AuthSheetTab::TvOAuth {
                                                theme().primary_foreground
                                            } else {
                                                theme().foreground
                                            }
                                        }))
                                        .child(
                                            text("TV Code (Recommended)")
                                                .text_sm()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || {
                                                    if active_tab.get() == AuthSheetTab::TvOAuth {
                                                        theme().primary_foreground
                                                    } else {
                                                        theme().foreground
                                                    }
                                                }),
                                        ),
                                )
                                .child(
                                    button()
                                        .selected(move || active_tab.get() == AuthSheetTab::WebView)
                                        .on_press(move || active_tab.set(AuthSheetTab::WebView))
                                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                        .px_4()
                                        .flex_row()
                                        .items_center()
                                        .justify_center()
                                        .gap_2()
                                        .rounded(radius_pill)
                                        .bg(move || {
                                            if active_tab.get() == AuthSheetTab::WebView {
                                                theme().primary
                                            } else {
                                                pill_glass_surface()
                                            }
                                        })
                                        .border_1()
                                        .border_color(glass_hairline)
                                        .hover(|s| s.opacity(0.92))
                                        .focus_visible(|s| {
                                            s.outline(px(2.0)).outline_color(move || theme().ring)
                                        })
                                        .child(icon(IconKind::Home, 15.0, move || {
                                            if active_tab.get() == AuthSheetTab::WebView {
                                                theme().primary_foreground
                                            } else {
                                                theme().foreground
                                            }
                                        }))
                                        .child(
                                            text("Android WebView")
                                                .text_sm()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || {
                                                    if active_tab.get() == AuthSheetTab::WebView {
                                                        theme().primary_foreground
                                                    } else {
                                                        theme().foreground
                                                    }
                                                }),
                                        ),
                                ),
                        )
                        .child(when(
                            move || active_tab.get() == AuthSheetTab::WebView,
                            move || {
                                let cb_tv_switch = Arc::clone(&cb_start_tv);
                                #[cfg(target_os = "android")]
                                {
                                    view()
                                        .p_4()
                                        .gap_3()
                                        .rounded(radius_card)
                                        .bg(move || theme().card)
                                        .border_1()
                                        .border_color(glass_hairline)
                                        .child(
                                            text("Android YouTube Music WebView")
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text("Opens the native Android WebView to sign in on music.youtube.com and securely capture HttpOnly session cookies into Android Keystore.")
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                        )
                                        .child(
                                            view()
                                                .flex_row()
                                                .flex_wrap()
                                                .gap_2()
                                                .child(primary_button(
                                                    "Launch YouTube Music Sign-In",
                                                    crate::store::launch_android_webview_sign_in,
                                                ))
                                                .child(secondary_button(
                                                    "Use TV OAuth Code Instead",
                                                    move || {
                                                        active_tab.set(AuthSheetTab::TvOAuth);
                                                        cb_tv_switch();
                                                    },
                                                )),
                                        )
                                }
                                #[cfg(not(target_os = "android"))]
                                {
                                    view()
                                        .p_4()
                                        .gap_3()
                                        .rounded(radius_card)
                                        .bg(move || theme().card)
                                        .border_1()
                                        .border_color(glass_hairline)
                                        .child(
                                            text("Native WebView Cookie Capture (Android Only)")
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text("Desktop builds cannot read HttpOnly cookies from embedded WebViews. Use YouTube's official TV OAuth Device Code below, or paste a browser Cookie header under Advanced.")
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                        )
                                        .child(primary_button("Continue with TV Code", move || {
                                            active_tab.set(AuthSheetTab::TvOAuth);
                                            cb_tv_switch();
                                        }))
                                }
                            },
                        ))
                        .child(when(
                            move || active_tab.get() == AuthSheetTab::TvOAuth,
                            move || {
                                let cb_gen = Arc::clone(&cb_start_tv_2);
                                let cb_can = Arc::clone(&cb_cancel_tv);
                                dynamic(move || {
                                    let cb_gen_inner = Arc::clone(&cb_gen);
                                    let cb_can_inner = Arc::clone(&cb_can);
                                    let status_msg =
                                        crate::store::settings_snapshot().tv_oauth_status;
                                    if let Some(code) = tv_user_code.get() {
                                        let url = tv_verification_url
                                            .get()
                                            .unwrap_or_else(|| "https://www.google.com/device".to_owned());
                                        let secs = tv_countdown_secs.get();
                                        view()
                                            .p_4()
                                            .gap_3()
                                            .rounded(radius_card)
                                            .bg(move || theme().card)
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .child(
                                                text(format!("Enter code at {url}"))
                                                    .text_xs()
                                                    .text_color(move || theme().muted_foreground),
                                            )
                                            .child(
                                                text(code)
                                                    .text_2xl()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(move || theme().primary),
                                            )
                                            .child(
                                                text(format!("Expires in {secs}s"))
                                                    .text_xs()
                                                    .tabular_nums()
                                                    .text_color(move || theme().muted_foreground),
                                            )
                                            .when_some(status_msg, |v, msg| {
                                                v.child(
                                                    text(msg)
                                                        .text_xs()
                                                        .text_color(move || {
                                                            theme().muted_foreground
                                                        }),
                                                )
                                            })
                                            .child(
                                                view()
                                                    .flex_row()
                                                    .flex_wrap()
                                                    .gap_2()
                                                    .child(primary_button("Open Sign-In Page", {
                                                        let url = url.clone();
                                                        move || {
                                                            let _ = open_url(&url);
                                                        }
                                                    }))
                                                    .child(secondary_button(
                                                        "Generate New Code",
                                                        move || cb_gen_inner(),
                                                    ))
                                                    .child(secondary_button("Cancel", move || {
                                                        cb_can_inner()
                                                    })),
                                            )
                                    } else {
                                        view()
                                            .p_4()
                                            .gap_3()
                                            .rounded(radius_card)
                                            .bg(move || theme().card)
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .child(
                                                text("Sign in using YouTube's official RFC 8628 TV device code flow — no DevTools cookies required.")
                                                    .text_xs()
                                                    .text_color(move || theme().muted_foreground),
                                            )
                                            .when_some(status_msg, |v, msg| {
                                                v.child(
                                                    text(msg)
                                                        .text_xs()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(move || theme().primary),
                                                )
                                            })
                                            .child(primary_button("Generate TV Sign-In Code", move || {
                                                cb_gen_inner()
                                            }))
                                    }
                                })
                            },
                        ))
                        .child(
                            button()
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_3()
                                .rounded(radius_card)
                                .bg(pill_glass_surface)
                                .border_1()
                                .border_color(glass_hairline)
                                .hover(|s| s.bg(pill_glass_hover))
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
                                })
                                .on_press(move || {
                                    show_advanced_cookies.set(!show_advanced_cookies.get())
                                })
                                .child(
                                    text("Advanced: Paste Cookie Header")
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(when(
                            move || show_advanced_cookies.get(),
                            move || {
                                let cb_sub = Arc::clone(&cb_cookies);
                                view()
                                    .gap_2()
                                    .child(labeled_input(
                                        "Cookie header",
                                        "auth-cookie-input",
                                        "SAPISID=...; __Secure-3PAPISID=...; SID=...",
                                        move || cookie_draft.get(),
                                        move |v| cookie_draft.set(v.to_owned()),
                                    ))
                                    .child(primary_button("Sign In with Cookies", move || {
                                        cb_sub(cookie_draft.get());
                                    }))
                            },
                        ))
                        .child(
                            view().flex_row().justify_end().child(secondary_button(
                                "Close",
                                move || cb_close(),
                            )),
                        ),
                ))
        },
    )
}
