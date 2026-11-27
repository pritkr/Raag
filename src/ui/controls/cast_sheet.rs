//! LAN Cast Device Picker Sheet & Cast Status Button (`LT-3`).
//!
//! Zero raw color literals; all colors come from
//! `nsh_base::theme()` and `crate::ui::tokens`. All interactive targets >= 48dp.

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::{CastDevice, CastProtocol, CastSessionState};

use crate::store;
use crate::ui::controls::primitives::{secondary_button, svg_icon_button};
use crate::ui::controls::sheets::modal_panel_container;
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, motion, pill_glass_hover, pill_glass_surface, radius_card, radius_pill,
};

fn protocol_badge_label(protocol: CastProtocol) -> &'static str {
    match protocol {
        CastProtocol::Chromecast => "Chromecast",
        CastProtocol::DlnaUpnp => "DLNA / UPnP",
        CastProtocol::AirPlayNative => "AirPlay",
    }
}

/// Accessible >= 48dp Cast button for Now Playing & Mini-Player bars.
/// Lights up with `theme().primary` when `connected` is true.
pub fn cast_button(connected: bool, on_press: impl Fn() + 'static) -> impl IntoElement {
    svg_icon_button(
        IconKind::Radio,
        20.0,
        MOBILE_MIN_TOUCH_TARGET,
        "Cast",
        false,
        move || {
            if connected {
                theme().primary
            } else {
                theme().foreground
            }
        },
        on_press,
    )
}

/// Cast Device Picker Sheet (bottom sheet on mobile, popover dialog on desktop).
pub fn cast_picker_sheet(
    devices: Vec<CastDevice>,
    session: CastSessionState,
    on_select_device: impl Fn(CastDevice) + Clone + 'static,
    on_disconnect: impl Fn() + 'static,
    on_volume_change: impl Fn(f32) + Clone + 'static,
    on_close: impl Fn() + 'static,
) -> impl IntoElement {
    let status_subtitle: SharedString = match &session {
        CastSessionState::Disconnected => "Select a speaker or TV on your local network".into(),
        CastSessionState::Connecting { device_name } => {
            format!("Connecting to {device_name}…").into()
        }
        CastSessionState::Connected { device, .. } => {
            format!("Connected to {}", device.name).into()
        }
        CastSessionState::Error(err) => format!("Cast error: {err}").into(),
    };

    let active_device_id = match &session {
        CastSessionState::Connected { device, .. } => Some(device.id.clone()),
        _ => None,
    };

    let body = view()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text("Available Devices")
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().muted_foreground),
                )
                .child(svg_icon_button(
                    IconKind::Close,
                    16.0,
                    MOBILE_MIN_TOUCH_TARGET,
                    "Close Cast Picker",
                    true,
                    move || theme().muted_foreground,
                    on_close,
                )),
        )
        .children(if devices.is_empty() {
            vec![
                view()
                    .p_4()
                    .rounded(radius_card)
                    .bg(pill_glass_surface)
                    .child(
                        text("Searching for Chromecast & DLNA devices on Wi‑Fi…")
                            .text_sm()
                            .text_color(move || theme().muted_foreground),
                    )
                    .into_element(),
            ]
        } else {
            devices
                .into_iter()
                .map(|dev| {
                    let is_active = active_device_id.as_deref() == Some(dev.id.as_str());
                    let badge = protocol_badge_label(dev.protocol);
                    let dev_name: SharedString = dev.name.clone().into();
                    let dev_model: SharedString =
                        dev.model.clone().unwrap_or_else(|| badge.to_owned()).into();
                    let cb = on_select_device.clone();
                    let dev_clone = dev.clone();

                    button()
                        .on_press(move || cb(dev_clone.clone()))
                        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                        .px_3()
                        .py_2()
                        .rounded(radius_card)
                        .bg(pill_glass_surface)
                        .transition()
                        .duration(|| motion::transition_ms(motion::PRESS_MS, store::reduced_motion()))
                        .ease_out()
                        .hover(|s| s.bg(pill_glass_hover))
                        .active(|s| {
                            s.scale(|| {
                                if store::reduced_motion() {
                                    1.0
                                } else {
                                    motion::PRESS_SCALE
                                }
                            })
                        })
                        .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .child(icon(IconKind::Headphones, 18.0, move || {
                                    if is_active {
                                        theme().primary
                                    } else {
                                        theme().foreground
                                    }
                                }))
                                .child(
                                    view()
                                        .gap_0p5()
                                        .child(
                                            text(dev_name)
                                                .text_sm()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(move || theme().foreground),
                                        )
                                        .child(
                                            text(dev_model)
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                        ),
                                ),
                        )
                        .child(
                            view()
                                .px_2()
                                .py_1()
                                .rounded(radius_pill)
                                .bg(move || {
                                    if is_active {
                                        theme().primary
                                    } else {
                                        theme().secondary
                                    }
                                })
                                .child(
                                    text(if is_active { "Connected" } else { badge })
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || {
                                            if is_active {
                                                theme().primary_foreground
                                            } else {
                                                theme().secondary_foreground
                                            }
                                        }),
                                ),
                        )
                        .into_element()
                })
                .collect()
        })
        .when(
            matches!(session, CastSessionState::Connected { .. }),
            |el| {
                let vol_pct = match &session {
                    CastSessionState::Connected { volume_pct, .. } => *volume_pct,
                    _ => 100,
                };
                let dec_cb = on_volume_change.clone();
                let inc_cb = on_volume_change.clone();
                let vol_down = ((vol_pct as f32 / 100.0) - 0.1).clamp(0.0, 1.0);
                let vol_up = ((vol_pct as f32 / 100.0) + 0.1).clamp(0.0, 1.0);

                el.child(
                    view()
                        .pt_2()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    text(format!("Receiver Volume: {vol_pct}%"))
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .gap_2()
                                        .child(secondary_button("Vol -", move || dec_cb(vol_down)))
                                        .child(secondary_button("Vol +", move || inc_cb(vol_up))),
                                ),
                        )
                        .child(secondary_button("Disconnect", on_disconnect)),
                )
            },
        );

    modal_panel_container("Cast to Device", status_subtitle, body)
}

/// Active Cast Banner rendered above the Mini-Player / Desktop Player Bar while casting.
pub fn active_cast_banner(
    device_name: &str,
    on_open_picker: impl Fn() + 'static,
    on_disconnect: impl Fn() + 'static,
) -> impl IntoElement {
    let label: SharedString = format!("Casting to {device_name}").into();
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .px_3()
        .py_1()
        .rounded(radius_card)
        .bg(pill_glass_surface)
        .child(
            button()
                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                .flex_row()
                .items_center()
                .gap_2()
                .rounded(move || theme().radius)
                .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
                .on_press(on_open_picker)
                .child(icon(IconKind::Radio, 16.0, move || theme().primary))
                .child(
                    text(label)
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(secondary_button("Disconnect", on_disconnect))
}
