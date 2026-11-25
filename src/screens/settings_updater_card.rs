//! LT-2 Settings Updater Card & Non-Intrusive Top Banner (`src/screens/settings_updater_card.rs`).
//!
//! Zero raw color literals, `>= 48dp` mobile touch targets, `tabular_nums` for MB progress,
//! and audio-aware etiquette ("Install on Quit" primary when audio is playing).

use std::rc::Rc;

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{UpdateChannel, UpdateInfo, UpdatePhase};

use crate::ui::controls::{glass_pill_button, hero_pill_button, secondary_button, setting_switch};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_status_ok, glass_hairline, pill_glass_surface, radius_card,
};

#[derive(Clone)]
pub struct UpdaterCardProps {
    pub current_version: &'static str,
    pub channel: Signal<UpdateChannel>,
    pub auto_check: Signal<bool>,
    pub phase: Signal<UpdatePhase>,
    pub is_playing: Signal<bool>,
    pub on_check_now: Rc<dyn Fn()>,
    pub on_toggle_auto_check: Rc<dyn Fn(bool)>,
    pub on_select_channel: Rc<dyn Fn(UpdateChannel)>,
    pub on_download: Rc<dyn Fn(UpdateInfo)>,
    pub on_open_release: Rc<dyn Fn(String)>,
    pub on_skip_version: Rc<dyn Fn(String)>,
    pub on_install_on_quit: Rc<dyn Fn()>,
    pub on_restart_now: Rc<dyn Fn()>,
}

fn format_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// Settings card for app updates, channel selection, and audio-aware install actions.
pub fn settings_updater_card(props: UpdaterCardProps) -> impl IntoElement {
    let on_check = props.on_check_now.clone();
    let on_toggle_auto = props.on_toggle_auto_check.clone();
    let on_channel_stable = props.on_select_channel.clone();
    let on_channel_beta = props.on_select_channel.clone();

    view()
        .test_id("settings-updater-card")
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
                .flex_wrap()
                .child(
                    text("App Updates")
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    view()
                        .px_2p5()
                        .py_1()
                        .rounded_full()
                        .bg(pill_glass_surface)
                        .child(
                            text(move || {
                                SharedString::from(format!(
                                    "Raag v{} • {}",
                                    props.current_version,
                                    props.channel.get().label()
                                ))
                            })
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(move || theme().muted_foreground),
                        ),
                ),
        )
        .child(
            // Channel toggle: Stable / Beta
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    text("Update Channel")
                        .text_sm()
                        .text_color(move || theme().foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            button()
                                .test_id("updater-channel-stable")
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_3()
                                .rounded_full()
                                .bg(move || {
                                    if props.channel.get() == UpdateChannel::Stable {
                                        theme().primary
                                    } else {
                                        pill_glass_surface()
                                    }
                                })
                                .on_press(move || on_channel_stable(UpdateChannel::Stable))
                                .child(
                                    text("Stable")
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || {
                                            if props.channel.get() == UpdateChannel::Stable {
                                                theme().primary_foreground
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                ),
                        )
                        .child(
                            button()
                                .test_id("updater-channel-beta")
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_3()
                                .rounded_full()
                                .bg(move || {
                                    if props.channel.get() == UpdateChannel::Beta {
                                        theme().primary
                                    } else {
                                        pill_glass_surface()
                                    }
                                })
                                .on_press(move || on_channel_beta(UpdateChannel::Beta))
                                .child(
                                    text("Beta")
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(move || {
                                            if props.channel.get() == UpdateChannel::Beta {
                                                theme().primary_foreground
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                ),
                        ),
                ),
        )
        .child(setting_switch(
            "Automatic Update Checks",
            move || props.auto_check.get(),
            move |next| on_toggle_auto(next),
        ))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .gap_2()
                .child(hero_pill_button("Check for Updates", None, move || {
                    on_check();
                })),
        )
        .child({
            let on_download = props.on_download.clone();
            let on_open_release = props.on_open_release.clone();
            let on_skip = props.on_skip_version.clone();
            let on_install_quit = props.on_install_on_quit.clone();
            let on_restart = props.on_restart_now.clone();

            dynamic(move || match props.phase.get() {
                UpdatePhase::Idle => view().into_element(),
                UpdatePhase::Checking => text("Checking for updates…")
                    .text_xs()
                    .text_color(move || theme().muted_foreground)
                    .into_element(),
                UpdatePhase::UpToDate { .. } => text("You are on the latest version.")
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(color_status_ok)
                    .into_element(),
                UpdatePhase::Available(info) => {
                    let dl_cb = on_download.clone();
                    let open_cb = on_open_release.clone();
                    let skip_cb = on_skip.clone();
                    let info_dl = info.clone();
                    let url_open = info.download_url.clone();
                    let ver_skip = info.version.clone();
                    let can_install = info.can_install_in_place;

                    view()
                        .p_3()
                        .gap_2()
                        .rounded(radius_card)
                        .bg(move || theme().muted)
                        .child(
                            text(format!("Update Available: v{}", info.version))
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(info.release_notes.clone())
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .when(can_install, |v| {
                                    let dl_cb = dl_cb.clone();
                                    let info_dl = info_dl.clone();
                                    v.child(hero_pill_button(
                                        "Download & Update",
                                        None,
                                        move || dl_cb(info_dl.clone()),
                                    ))
                                })
                                .when(!can_install, |v| {
                                    let open_cb = open_cb.clone();
                                    let url_open = url_open.clone();
                                    v.child(hero_pill_button("View Release", None, move || {
                                        open_cb(url_open.clone())
                                    }))
                                })
                                .child(glass_pill_button("Skip Version", None, move || {
                                    skip_cb(ver_skip.clone())
                                })),
                        )
                        .into_element()
                }
                UpdatePhase::Downloading {
                    info,
                    received_bytes,
                    total_bytes,
                } => {
                    let label = match total_bytes {
                        Some(t) if t > 0 => format!(
                            "Downloading v{}: {} / {}",
                            info.version,
                            format_mb(received_bytes),
                            format_mb(t)
                        ),
                        _ => format!(
                            "Downloading v{}: {}",
                            info.version,
                            format_mb(received_bytes)
                        ),
                    };
                    let fraction = total_bytes
                        .filter(|&t| t > 0)
                        .map(|t| (received_bytes as f32 / t as f32).clamp(0.0, 1.0))
                        .unwrap_or(0.0);

                    view()
                        .p_3()
                        .gap_2()
                        .rounded(radius_card)
                        .bg(move || theme().muted)
                        .child(
                            text(label)
                                .text_xs()
                                .tabular_nums()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            view()
                                .h(px(6.0))
                                .w(px(240.0))
                                .rounded_full()
                                .bg(pill_glass_surface)
                                .child(
                                    view()
                                        .h_full()
                                        .w(px(240.0 * fraction))
                                        .rounded_full()
                                        .bg(move || theme().primary),
                                ),
                        )
                        .into_element()
                }
                UpdatePhase::ReadyToInstall {
                    info,
                    install_on_quit_scheduled,
                } => {
                    let playing = props.is_playing.get();
                    let quit_cb = on_install_quit.clone();
                    let restart_cb = on_restart.clone();
                    let quit_label = if install_on_quit_scheduled {
                        "Scheduled on Quit"
                    } else {
                        "Install on Quit"
                    };

                    view()
                        .p_3()
                        .gap_2()
                        .rounded(radius_card)
                        .bg(move || theme().muted)
                        .child(
                            text(format!("Ready to install v{}", info.version))
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(color_status_ok),
                        )
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .when(playing, |v| {
                                    let quit_cb = quit_cb.clone();
                                    let restart_cb = restart_cb.clone();
                                    v.child(hero_pill_button(quit_label, None, move || quit_cb()))
                                        .child(secondary_button("Restart Now", move || {
                                            restart_cb()
                                        }))
                                })
                                .when(!playing, |v| {
                                    let quit_cb = quit_cb.clone();
                                    let restart_cb = restart_cb.clone();
                                    v.child(secondary_button(quit_label, move || quit_cb()))
                                        .child(hero_pill_button("Restart Now", None, move || {
                                            restart_cb()
                                        }))
                                }),
                        )
                        .into_element()
                }
                UpdatePhase::Error(err) => text(format!("Update check failed: {err}"))
                    .text_xs()
                    .text_color(move || theme().destructive)
                    .into_element(),
            })
        })
}

/// Non-intrusive dismissible top banner shown when a background update check finds a newer release.
pub fn updater_top_banner(
    available: Option<UpdateInfo>,
    on_open_settings: Rc<dyn Fn()>,
    on_dismiss: Rc<dyn Fn()>,
) -> impl IntoElement {
    dynamic(move || {
        let Some(info) = available.clone() else {
            return view().into_element();
        };
        let open_cb = on_open_settings.clone();
        let dismiss_cb = on_dismiss.clone();
        view()
            .test_id("updater-top-banner")
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .px_4()
            .py_2()
            .rounded_full()
            .bg(pill_glass_surface)
            .border_1()
            .border_color(glass_hairline)
            .child(
                text(format!("Raag v{} is available", info.version))
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().foreground),
            )
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(glass_pill_button("Details", None, move || open_cb()))
                    .child(
                        button()
                            .test_id("updater-banner-dismiss-btn")
                            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                            .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                            .accessibility_label("Dismiss update banner")
                            .on_press(move || dismiss_cb())
                            .child(
                                text("Dismiss")
                                    .text_xs()
                                    .text_color(move || theme().muted_foreground),
                            ),
                    ),
            )
            .into_element()
    })
}
