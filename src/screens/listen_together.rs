//! Radio & Listen Together Screen (`src/screens/listen_together.rs`, `H2.3`).
//!
//! Two honest sub-modes:
//! 1. **Radio Mix** (local instant seed/mood radio)
//! 2. **Listen Together** (host-authoritative synced rooms with join codes)
//!
//! Zero raw color literals — 100% semantic `theme.json` & `src/ui/tokens` tokens.

use std::sync::Arc;

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use resonance_contract::RoomRole;
use resonance_core::rooms::RoomStateSnapshot;

use crate::store::ListenTogetherSubMode;
use crate::ui::controls::{
    confirm_destructive, glass_pill_button, hero_pill_button, labeled_input, secondary_button,
    status_card,
};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, color_destructive_foreground, glass_hairline, is_desktop_layout,
    pill_glass_surface, radius_card, radius_pill,
};

#[derive(Clone)]
pub struct ListenTogetherCallbacks {
    pub on_create_room: Arc<dyn Fn(String, String)>,
    pub on_join_room: Arc<dyn Fn(String, String, String)>,
    pub on_leave_room: Arc<dyn Fn()>,
    pub on_start_local_radio: Arc<dyn Fn()>,
    pub on_vote_skip: Arc<dyn Fn()>,
    pub on_send_reaction: Arc<dyn Fn(String)>,
    pub on_kick_participant: Arc<dyn Fn(String)>,
    pub on_transfer_host: Arc<dyn Fn(String)>,
    pub on_copy_invite: Arc<dyn Fn(String)>,
}

impl Default for ListenTogetherCallbacks {
    fn default() -> Self {
        Self {
            on_create_room: Arc::new(|_, _| {}),
            on_join_room: Arc::new(|_, _, _| {}),
            on_leave_room: Arc::new(|| {}),
            on_start_local_radio: Arc::new(|| {}),
            on_vote_skip: Arc::new(|| {}),
            on_send_reaction: Arc::new(|_| {}),
            on_kick_participant: Arc::new(|_| {}),
            on_transfer_host: Arc::new(|_| {}),
            on_copy_invite: Arc::new(|_| {}),
        }
    }
}

pub fn listen_together_screen(
    room_snapshot: Signal<Option<RoomStateSnapshot>>,
    local_track_warning: Signal<Option<String>>,
    cb: ListenTogetherCallbacks,
) -> impl IntoElement {
    let sub_mode = signal(ListenTogetherSubMode::Rooms);
    let default_relay = std::env::var("RAAG_ROOM_RELAY_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://relay.raag.audio".to_owned());
    let relay_url_draft = signal(default_relay);
    let display_name_draft = signal(String::from("Listener"));
    let join_code_draft = signal(String::new());
    let desktop = is_desktop_layout();

    let cb_create = Arc::clone(&cb.on_create_room);
    let cb_join = Arc::clone(&cb.on_join_room);
    let cb_leave = Arc::clone(&cb.on_leave_room);
    let cb_radio = Arc::clone(&cb.on_start_local_radio);
    let cb_vote_skip = Arc::clone(&cb.on_vote_skip);
    let cb_reaction = Arc::clone(&cb.on_send_reaction);
    let cb_kick = Arc::clone(&cb.on_kick_participant);
    let cb_transfer = Arc::clone(&cb.on_transfer_host);
    let cb_copy = Arc::clone(&cb.on_copy_invite);

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
                        .flex_wrap()
                        .gap_2()
                        .child(
                            button()
                                .accessibility_label("Radio Mix")
                                .selected(move || sub_mode.get() == ListenTogetherSubMode::RadioMix)
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_4()
                                .items_center()
                                .justify_center()
                                .rounded(radius_pill)
                                .bg(move || {
                                    if sub_mode.get() == ListenTogetherSubMode::RadioMix {
                                        theme().primary
                                    } else {
                                        pill_glass_surface()
                                    }
                                })
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
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
                                .accessibility_label("Listen Together")
                                .selected(move || sub_mode.get() == ListenTogetherSubMode::Rooms)
                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                .px_4()
                                .items_center()
                                .justify_center()
                                .rounded(radius_pill)
                                .bg(move || {
                                    if sub_mode.get() == ListenTogetherSubMode::Rooms {
                                        theme().primary
                                    } else {
                                        pill_glass_surface()
                                    }
                                })
                                .focus_visible(|s| {
                                    s.outline(px(2.0)).outline_color(move || theme().ring)
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
                        let cb_vs = Arc::clone(&cb_vote_skip);
                        let cb_rx = Arc::clone(&cb_reaction);
                        let cb_k = Arc::clone(&cb_kick);
                        let cb_tr = Arc::clone(&cb_transfer);
                        let cb_cp = Arc::clone(&cb_copy);

                        dynamic(move || {
                            if let Some(snap) = room_snapshot.get() {
                                let cb_l_inner = Arc::clone(&cb_l);
                                let cb_vs_inner = Arc::clone(&cb_vs);
                                let cb_rx_inner = Arc::clone(&cb_rx);
                                let cb_k_inner = Arc::clone(&cb_k);
                                let cb_tr_inner = Arc::clone(&cb_tr);
                                let cb_cp_inner = Arc::clone(&cb_cp);
                                let room_code_copy = snap.room_code.clone();
                                let is_host = snap.role == RoomRole::Host;
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
                                            .flex_wrap()
                                            .items_center()
                                            .justify_between()
                                            .gap_2()
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
                                                            "{role_label} • {} • Synced ±{} ms",
                                                            snap.transport.label(),
                                                            snap.clock_offset_ms.abs()
                                                        ))
                                                        .text_xs()
                                                        .tabular_nums()
                                                        .text_color(move || theme().muted_foreground),
                                                    ),
                                            )
                                            .child(
                                                view()
                                                    .flex_row()
                                                    .gap_2()
                                                    .child(secondary_button("Copy Invite", move || {
                                                        cb_cp_inner(room_code_copy.clone())
                                                    }))
                                                    .child(secondary_button("Leave Room", move || {
                                                        cb_l_inner()
                                                    })),
                                            ),
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
                                                        .text_color(color_destructive_foreground),
                                                ),
                                        )
                                    })
                                    .child(
                                        view()
                                            .p_3()
                                            .gap_3()
                                            .rounded(radius_card)
                                            .bg(pill_glass_surface)
                                            .child(
                                                text(format!("Now Syncing: {track_label}"))
                                                    .text_sm()
                                                    .text_color(move || theme().foreground),
                                            )
                                            .child(
                                                view()
                                                    .flex_row()
                                                    .flex_wrap()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(secondary_button("Vote Skip", move || {
                                                        cb_vs_inner()
                                                    }))
                                                    .children(
                                                        [
                                                            ("🔥", "Send fire reaction"),
                                                            ("❤️", "Send heart reaction"),
                                                            ("🎧", "Send headphones reaction"),
                                                            ("✨", "Send sparkle reaction"),
                                                        ]
                                                        .into_iter()
                                                        .map(|(emoji, a11y)| {
                                                            let cb_e = Arc::clone(&cb_rx_inner);
                                                            let label = emoji.to_owned();
                                                            button()
                                                                .accessibility_label(a11y)
                                                                .min_w(px(MOBILE_MIN_TOUCH_TARGET))
                                                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                                .px_3()
                                                                .items_center()
                                                                .justify_center()
                                                                .rounded(radius_pill)
                                                                .bg(move || theme().secondary)
                                                                .focus_visible(|s| {
                                                                    s.outline(px(2.0))
                                                                        .outline_color(move || {
                                                                            theme().ring
                                                                        })
                                                                })
                                                                .on_press(move || {
                                                                    cb_e(label.clone())
                                                                })
                                                                .child(
                                                                    text(emoji)
                                                                        .text_sm()
                                                                        .text_color(move || {
                                                                            theme().foreground
                                                                        }),
                                                                )
                                                        }),
                                                    ),
                                            ),
                                    )
                                    .when(!snap.playback.queue.is_empty(), |v| {
                                        v.child(
                                            view()
                                                .gap_1()
                                                .child(
                                                    text(format!(
                                                        "Up Next ({})",
                                                        snap.playback.queue.len()
                                                    ))
                                                    .text_sm()
                                                    .tabular_nums()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(move || theme().foreground),
                                                )
                                                .children(snap.playback.queue.iter().map(|q| {
                                                    text(format!("• {} — {}", q.title, q.artist))
                                                        .text_xs()
                                                        .text_color(move || {
                                                            theme().muted_foreground
                                                        })
                                                })),
                                        )
                                    })
                                    .child(
                                        view()
                                            .gap_2()
                                            .child(
                                                text(format!(
                                                    "Participants ({})",
                                                    snap.participants.len()
                                                ))
                                                .text_sm()
                                                .tabular_nums()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(move || theme().foreground),
                                            )
                                            .children(snap.participants.iter().map(|p| {
                                                let badge = match p.role {
                                                    RoomRole::Host => "Host",
                                                    RoomRole::Guest => "Guest",
                                                };
                                                let peer_id = p.peer_id.clone();
                                                let peer_id_kick = p.peer_id.clone();
                                                let display_name_tr = p.display_name.clone();
                                                let display_name_kick = p.display_name.clone();
                                                let can_manage = is_host && p.role == RoomRole::Guest;
                                                let cb_tr_p = Arc::clone(&cb_tr_inner);
                                                let cb_k_p = Arc::clone(&cb_k_inner);
                                                view()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .py_1()
                                                    .child(
                                                        text(format!("{} ({badge})", p.display_name))
                                                            .text_sm()
                                                            .text_color(move || theme().foreground),
                                                    )
                                                    .when(can_manage, |row| {
                                                        row.child(
                                                            view()
                                                                .flex_row()
                                                                .gap_2()
                                                                .child(secondary_button(
                                                                    "Make Host",
                                                                    move || {
                                                                        let cb = Arc::clone(&cb_tr_p);
                                                                        let id = peer_id.clone();
                                                                        let name = display_name_tr
                                                                            .clone();
                                                                        confirm_destructive(
                                                                            "Transfer host control?",
                                                                            format!(
                                                                                "Make {name} the room host? You will become a guest."
                                                                            ),
                                                                            "Transfer Host",
                                                                            move || cb(id),
                                                                        );
                                                                    },
                                                                ))
                                                                .child(secondary_button(
                                                                    "Kick",
                                                                    move || {
                                                                        let cb = Arc::clone(&cb_k_p);
                                                                        let id =
                                                                            peer_id_kick.clone();
                                                                        let name =
                                                                            display_name_kick
                                                                                .clone();
                                                                        confirm_destructive(
                                                                            "Remove participant?",
                                                                            format!(
                                                                                "Remove {name} from this Listen Together room?"
                                                                            ),
                                                                            "Remove",
                                                                            move || cb(id),
                                                                        );
                                                                    },
                                                                )),
                                                        )
                                                    })
                                            })),
                                    )
                            } else {
                                let cb_c_inner = Arc::clone(&cb_c);
                                let cb_j_inner = Arc::clone(&cb_j);
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
                                    .child(labeled_input(
                                        "Display name",
                                        "room-display-name-input",
                                        "Listener",
                                        move || display_name_draft.get(),
                                        move |v: &str| display_name_draft.set(v.to_owned()),
                                    ))
                                    .child(labeled_input(
                                        "Relay URL",
                                        "room-relay-url-input",
                                        "https://relay.raag.audio",
                                        move || relay_url_draft.get(),
                                        move |v: &str| relay_url_draft.set(v.to_owned()),
                                    ))
                                    .child(labeled_input(
                                        "Room code (required to join an existing room)",
                                        "room-join-code-input",
                                        "e.g. RAAG-7K4M",
                                        move || join_code_draft.get(),
                                        move |v: &str| join_code_draft.set(v.to_owned()),
                                    ))
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
                            }
                        })
                    },
                )),
        )
}
