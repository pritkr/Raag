//! Radio & Listen Together Screen (`src/screens/listen_together.rs`, `H2.3`).
//!
//! Two honest sub-modes:
//! 1. **Radio Mix** (local instant seed/mood radio)
//! 2. **Listen Together** (host-authoritative synced rooms with join codes)
//!
//! Zero raw color literals — 100% semantic `theme.json` & `src/ui/tokens` tokens.

use std::sync::Arc;

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::RoomRole;
use resonance_core::rooms::RoomStateSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListenTogetherSubMode {
    #[default]
    RadioMix,
    Rooms,
}
use crate::ui::controls::{
    glass_pill_button, hero_pill_button, primary_button, secondary_button, status_card,
};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, glass_hairline, is_desktop_layout, pill_glass_surface, radius_card,
    radius_pill,
};

#[derive(Clone)]
pub struct ListenTogetherCallbacks {
    pub on_create_room: Arc<dyn Fn(String, String)>,
    pub on_join_room: Arc<dyn Fn(String, String, String)>,
    pub on_leave_room: Arc<dyn Fn()>,
    pub on_start_local_radio: Arc<dyn Fn()>,
}

impl Default for ListenTogetherCallbacks {
    fn default() -> Self {
        Self {
            on_create_room: Arc::new(|_, _| {}),
            on_join_room: Arc::new(|_, _, _| {}),
            on_leave_room: Arc::new(|| {}),
            on_start_local_radio: Arc::new(|| {}),
        }
    }
}

pub fn listen_together_screen(
    room_snapshot: Signal<Option<RoomStateSnapshot>>,
    local_track_warning: Signal<Option<String>>,
    cb: ListenTogetherCallbacks,
) -> impl IntoElement {
    let sub_mode = signal(ListenTogetherSubMode::Rooms);
    let relay_url_draft = signal(String::from("https://relay.resonance.dev"));
    let display_name_draft = signal(String::from("Listener"));
    let join_code_draft = signal(String::new());
    let desktop = is_desktop_layout();

    let cb_create = Arc::clone(&cb.on_create_room);
    let cb_join = Arc::clone(&cb.on_join_room);
    let cb_leave = Arc::clone(&cb.on_leave_room);
    let cb_radio = Arc::clone(&cb.on_start_local_radio);

    scroll()
        .flex_1()
        .min_h(px(0.0))
        .pb_safe_area()
        .child(
            view()
                .test_id("listen-together-screen")
                .p_4()
                .when(desktop, |v| v.px_8().py_6())
                .gap_5()
                .child(
                    view()
                        .gap_1()
                        .child(
                            text("Radio & Listen Together")
                                .text_2xl()
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text("Host-authoritative synced rooms — zero audio proxying, each peer streams independently.")
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(
                            button()
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_4()
                                .rounded(radius_pill)
                                .bg(move || {
                                    if sub_mode.get() == ListenTogetherSubMode::RadioMix {
                                        theme().primary
                                    } else {
                                        pill_glass_surface()
                                    }
                                })
                                .on_press(move || sub_mode.set(ListenTogetherSubMode::RadioMix))
                                .child(
                                    text("Radio Mix")
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || {
                                            if sub_mode.get() == ListenTogetherSubMode::RadioMix {
                                                theme().primary_foreground
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                ),
                        )
                        .child(
                            button()
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_4()
                                .rounded(radius_pill)
                                .bg(move || {
                                    if sub_mode.get() == ListenTogetherSubMode::Rooms {
                                        theme().primary
                                    } else {
                                        pill_glass_surface()
                                    }
                                })
                                .on_press(move || sub_mode.set(ListenTogetherSubMode::Rooms))
                                .child(
                                    text("Listen Together")
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || {
                                            if sub_mode.get() == ListenTogetherSubMode::Rooms {
                                                theme().primary_foreground
                                            } else {
                                                theme().foreground
                                            }
                                        }),
                                ),
                        ),
                )
                .child(when(
                    move || sub_mode.get() == ListenTogetherSubMode::RadioMix,
                    move || {
                        let cb_r = Arc::clone(&cb_radio);
                        status_card(
                            "Instant Radio Mix",
                            "Generate a continuous queue seeded from your current track or mood.",
                            Some("Start Radio Mix"),
                            Some(Box::new(move || cb_r())),
                        )
                    },
                ))
                .child(when(
                    move || sub_mode.get() == ListenTogetherSubMode::Rooms,
                    move || {
                        let cb_c = Arc::clone(&cb_create);
                        let cb_j = Arc::clone(&cb_join);
                        let cb_l = Arc::clone(&cb_leave);

                        dynamic(move || {
                            if let Some(snap) = room_snapshot.get() {
                                let cb_l_inner = Arc::clone(&cb_l);
                                let role_label = match snap.role {
                                    RoomRole::Host => "Host",
                                    RoomRole::Guest => "Guest",
                                };
                                let track_label = snap
                                    .playback
                                    .current_track
                                    .as_ref()
                                    .map(|t| format!("{} — {}", t.title, t.artist))
                                    .unwrap_or_else(|| "No track playing".to_owned());
                                let warning = local_track_warning.get();

                                view()
                                    .p_5()
                                    .gap_4()
                                    .rounded(radius_card)
                                    .bg(move || theme().card)
                                    .border_1()
                                    .border_color(glass_hairline)
                                    .child(
                                        view()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .child(
                                                view()
                                                    .gap_1()
                                                    .child(
                                                        text(format!("Room {}", snap.room_code))
                                                            .text_xl()
                                                            .font_weight(FontWeight::BOLD)
                                                            .text_color(move || theme().foreground),
                                                    )
                                                    .child(
                                                        text(format!(
                                                            "{role_label} • {} • Offset {} ms",
                                                            snap.transport.label(),
                                                            snap.clock_offset_ms
                                                        ))
                                                        .text_xs()
                                                        .text_color(move || theme().muted_foreground),
                                                    ),
                                            )
                                            .child(secondary_button("Leave Room", move || {
                                                cb_l_inner()
                                            })),
                                    )
                                    .when_some(warning, |v, msg| {
                                        v.child(
                                            view()
                                                .p_3()
                                                .rounded(radius_card)
                                                .bg(move || theme().destructive)
                                                .child(
                                                    text(msg)
                                                        .text_xs()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(move || {
                                                            theme().primary_foreground
                                                        }),
                                                ),
                                        )
                                    })
                                    .child(
                                        view()
                                            .p_3()
                                            .rounded(radius_card)
                                            .bg(pill_glass_surface)
                                            .child(
                                                text(format!("Now Syncing: {track_label}"))
                                                    .text_sm()
                                                    .text_color(move || theme().foreground),
                                            ),
                                    )
                                    .child(
                                        view()
                                            .gap_2()
                                            .child(
                                                text(format!(
                                                    "Participants ({})",
                                                    snap.participants.len()
                                                ))
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || theme().foreground),
                                            )
                                            .children(snap.participants.iter().map(|p| {
                                                let badge = match p.role {
                                                    RoomRole::Host => "Host",
                                                    RoomRole::Guest => "Guest",
                                                };
                                                view()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .py_1()
                                                    .child(
                                                        text(p.display_name.clone())
                                                            .text_sm()
                                                            .text_color(move || theme().foreground),
                                                    )
                                                    .child(
                                                        text(badge)
                                                            .text_xs()
                                                            .text_color(move || {
                                                                theme().muted_foreground
                                                            }),
                                                    )
                                            })),
                                    )
                            } else {
                                let cb_c_inner = Arc::clone(&cb_c);
                                let cb_j_inner = Arc::clone(&cb_j);
                                let cb_c_connect = Arc::clone(&cb_c);
                                let cb_j_connect = Arc::clone(&cb_j);
                                view()
                                    .p_5()
                                    .gap_4()
                                    .rounded(radius_card)
                                    .bg(move || theme().card)
                                    .border_1()
                                    .border_color(glass_hairline)
                                    .child(
                                        text("Start or Join a Room")
                                            .text_base()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        input()
                                            .value(move || display_name_draft.get())
                                            .on_change(move |v: &str| display_name_draft.set(v.to_owned()))
                                            .placeholder("Display name")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET)),
                                    )
                                    .child(
                                        input()
                                            .value(move || relay_url_draft.get())
                                            .on_change(move |v: &str| relay_url_draft.set(v.to_owned()))
                                            .placeholder("Relay URL (https://...)")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET)),
                                    )
                                    .child(
                                        view()
                                            .flex_row()
                                            .flex_wrap()
                                            .gap_3()
                                            .child(
                                                view()
                                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                    .justify_center()
                                                    .child(hero_pill_button(
                                                        "Create Room",
                                                        Some(IconKind::ListenTogether),
                                                        move || {
                                                            cb_c_inner(
                                                                relay_url_draft.get(),
                                                                display_name_draft.get(),
                                                            );
                                                        },
                                                    )),
                                            )
                                            .child(
                                                view()
                                                    .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                    .justify_center()
                                                    .child(glass_pill_button(
                                                        "Join Room",
                                                        Some(IconKind::Play),
                                                        move || {
                                                            cb_j_inner(
                                                                relay_url_draft.get(),
                                                                join_code_draft.get(),
                                                                display_name_draft.get(),
                                                            );
                                                        },
                                                    )),
                                            ),
                                    )
                                    .child(
                                        input()
                                            .value(move || join_code_draft.get())
                                            .on_change(move |v: &str| join_code_draft.set(v.to_owned()))
                                            .placeholder("Room code (e.g. RAAG-7K4M)")
                                            .min_h(px(MOBILE_MIN_TOUCH_TARGET)),
                                    )
                                    .child(
                                        primary_button("Connect", move || {
                                            let code = join_code_draft.get();
                                            if code.trim().is_empty() {
                                                cb_c_connect(
                                                    relay_url_draft.get(),
                                                    display_name_draft.get(),
                                                );
                                            } else {
                                                cb_j_connect(
                                                    relay_url_draft.get(),
                                                    code,
                                                    display_name_draft.get(),
                                                );
                                            }
                                        }),
                                    )
                            }
                        })
                    },
                )),
        )
}
