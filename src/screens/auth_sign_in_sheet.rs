//! Single Unified Sign-In Modal Sheet (`src/screens/auth_sign_in_sheet.rs`, `H3.1`).
//!
//! 1. Primary (Desktop): Embedded NSH WebView Sign-In (`accounts.google.com`)
//! 2. Universal Fallback: RFC 8628 YouTube TV OAuth Device-Code Flow
//! 3. Advanced: Collapsible Manual Cookie Header Paste
//!
//! Zero raw color literals — 100% semantic `theme.json` & `src/ui/tokens` tokens.

use std::sync::Arc;

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;

use crate::ui::controls::{
    glass_pill_button, modal_panel_container, primary_button, secondary_button,
};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_bar_surface, glass_hairline, pill_glass_surface, radius_card,
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
    let active_tab = signal(AuthSheetTab::WebView);
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
                .bg(frosted_bar_surface)
                .items_center()
                .justify_center()
                .p_4()
                .child(modal_panel_container(
                    "Sign in to YouTube Music",
                    "Credentials are stored only in OS Keychain / Keystore.",
                    view()
                        .w(px(480.0))
                        .gap_3()
                        .child(
                            view()
                                .flex_row()
                                .gap_2()
                                .child(
                                    view()
                                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                        .justify_center()
                                        .child(glass_pill_button(
                                            "Browser Sign-In",
                                            Some(IconKind::Home),
                                            move || active_tab.set(AuthSheetTab::WebView),
                                        )),
                                )
                                .child(
                                    view()
                                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                        .justify_center()
                                        .child(glass_pill_button(
                                            "TV Code (Universal)",
                                            Some(IconKind::Radio),
                                            move || active_tab.set(AuthSheetTab::TvOAuth),
                                        )),
                                ),
                        )
                        .child(when(
                            move || active_tab.get() == AuthSheetTab::WebView,
                            move || {
                                let cb_tv_switch = Arc::clone(&cb_start_tv);
                                view()
                                    .p_4()
                                    .gap_3()
                                    .rounded(radius_card)
                                    .bg(move || theme().card)
                                    .border_1()
                                    .border_color(glass_hairline)
                                    .child(
                                        text("Embedded Google Sign-In")
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        text("Sign in via accounts.google.com. If your platform lacks HttpOnly cookie access, switch to TV Code with one tap.")
                                            .text_xs()
                                            .text_color(move || theme().muted_foreground),
                                    )
                                    .child(secondary_button("Use TV OAuth Device Code", move || {
                                        active_tab.set(AuthSheetTab::TvOAuth);
                                        cb_tv_switch();
                                    }))
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
                                            .child(
                                                view()
                                                    .flex_row()
                                                    .gap_2()
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
                                    .child(
                                        input()
                                            .value(move || cookie_draft.get())
                                            .on_change(move |v: &str| cookie_draft.set(v.to_owned()))
                                            .placeholder("SAPISID=...; __Secure-3PAPISID=...; SID=...")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET)),
                                    )
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
