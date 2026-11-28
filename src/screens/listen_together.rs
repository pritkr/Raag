//! Listen together Screen matching `ytm-rs` (`05_listen_together_1.png`):
//! - Bold `Listen together` heading + friendly sync explanation
//! - `Start a jam` translucent glass card: `Your name` row + hairline divider + `Host a jam` row with white `Start` pill
//! - `Join a jam` translucent glass card: uppercase `ABC234` code input + white `Join` pill
//! - Plus optional `Radio Mix` quick action and full active-room controls when hosting/joined
//! Zero raw color literals — 100% semantic `theme.json` & `src/ui/tokens` tokens.

use std::sync::Arc;

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;
use resonance_contract::RoomRole;
use resonance_core::rooms::RoomStateSnapshot;

use crate::ui::controls::{
    confirm_destructive, glass_pill_button, hero_pill_button, secondary_button,
    set_text_input_focused,
};
use crate::ui::icons::IconKind;
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, PROSE_MAX_WIDTH, color_destructive_foreground, frosted_settings_card,
    glass_hairline, is_desktop_layout, pill_glass_surface, radius_card, radius_pill,
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
    let default_relay = std::env::var("RAAG_ROOM_RELAY_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "https://relay.raag.audio".to_owned());
    let relay_url_draft = signal(default_relay);
    let display_name_draft = signal(String::from("Listener"));
    let join_code_draft = signal(String::new());
    let show_relay_field = signal(false);
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
                .w_full()
                .max_w(px(760.0))
                .p_4()
                .when(desktop, |v| v.px_8().py_6())
                .gap_6()
                // Header matching 05_listen_together_1.png
                .child(
                    view()
                        .gap_1p5()
                        .child(
                            text("Listen together")
                                .when(desktop, |t| t.text_3xl())
                                .when(!desktop, |t| t.text_2xl())
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text("Start a jam and friends hear what you play, at the same moment. Everyone listens on their own Raag, and anyone can add songs.")
                                .max_w(px(PROSE_MAX_WIDTH))
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(dynamic(move || {
                    if let Some(snap) = room_snapshot.get() {
                        let cb_l_inner = Arc::clone(&cb_leave);
                        let cb_vs_inner = Arc::clone(&cb_vote_skip);
                        let cb_rx_inner = Arc::clone(&cb_reaction);
                        let cb_k_inner = Arc::clone(&cb_kick);
                        let cb_tr_inner = Arc::clone(&cb_transfer);
                        let cb_cp_inner = Arc::clone(&cb_copy);
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
                            .bg(frosted_settings_card)
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
                                                text(format!("Jam {}", snap.room_code))
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
                                            .child(glass_pill_button("Copy invite", None, move || {
                                                cb_cp_inner(room_code_copy.clone())
                                            }))
                                            .child(glass_pill_button("Leave jam", None, move || {
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
                                        text(format!("Now syncing: {track_label}"))
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
                                                        .bg(pill_glass_surface)
                                                        .border_1()
                                                        .border_color(glass_hairline)
                                                        .focus_visible(|s| {
                                                            s.outline(px(2.0)).outline_color(
                                                                move || theme().ring,
                                                            )
                                                        })
                                                        .on_press(move || cb_e(label.clone()))
                                                        .child(text(emoji).text_sm().text_color(
                                                            move || theme().foreground,
                                                        ))
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
                                                .text_color(move || theme().muted_foreground)
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
                                                                let name = display_name_tr.clone();
                                                                confirm_destructive(
                                                                    "Transfer host control?",
                                                                    format!(
                                                                        "Make {name} the jam host? You will become a guest."
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
                                                                let id = peer_id_kick.clone();
                                                                let name =
                                                                    display_name_kick.clone();
                                                                confirm_destructive(
                                                                    "Remove participant?",
                                                                    format!(
                                                                        "Remove {name} from this Listen together jam?"
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
                            .into_element()
                    } else {
                        let cb_c_inner = Arc::clone(&cb_create);
                        let cb_j_inner = Arc::clone(&cb_join);
                        let cb_r_inner = Arc::clone(&cb_radio);

                        view()
                            .gap_6()
                            // Section 1: Start a jam (matches 05_listen_together_1.png)
                            .child(
                                view()
                                    .gap_3()
                                    .child(
                                        text("Start a jam")
                                            .text_base()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        view()
                                            .p_5()
                                            .gap_4()
                                            .rounded(radius_card)
                                            .bg(frosted_settings_card)
                                            .border_1()
                                            .border_color(glass_hairline)
                                            // Row 1: Your name
                                            .child(
                                                view()
                                                    .when(desktop, |v| {
                                                        v.flex_row()
                                                            .items_center()
                                                            .justify_between()
                                                            .gap_4()
                                                    })
                                                    .when(!desktop, |v| v.gap_2p5())
                                                    .child(
                                                        view()
                                                            .flex_1()
                                                            .min_w(px(0.0))
                                                            .gap_0p5()
                                                            .child(
                                                                text("Your name")
                                                                    .text_sm()
                                                                    .font_weight(
                                                                        FontWeight::SEMIBOLD,
                                                                    )
                                                                    .text_color(move || {
                                                                        theme().foreground
                                                                    }),
                                                            )
                                                            .child(
                                                                text("What others in the jam see.")
                                                                    .text_xs()
                                                                    .text_color(move || {
                                                                        theme().muted_foreground
                                                                    }),
                                                            ),
                                                    )
                                                    .child(
                                                        input()
                                                            .test_id("room-display-name-input")
                                                            .value(move || display_name_draft.get())
                                                            .on_change(move |v: &str| {
                                                                display_name_draft
                                                                    .set(v.to_owned())
                                                            })
                                                            .on_focus(|| {
                                                                set_text_input_focused(true)
                                                            })
                                                            .on_blur(|| {
                                                                set_text_input_focused(false)
                                                            })
                                                            .placeholder("Your name")
                                                            .placeholder_color(|| {
                                                                color(theme().muted_foreground)
                                                            })
                                                            .caret_color(|| color(theme().primary))
                                                            .when(desktop, |i| i.w(px(210.0)))
                                                            .when(!desktop, |i| i.w_full())
                                                            .min_h(px(if desktop {
                                                                38.0
                                                            } else {
                                                                MOBILE_MIN_TOUCH_TARGET
                                                            }))
                                                            .px_3p5()
                                                            .text_sm()
                                                            .text_color(move || theme().foreground)
                                                            .bg(pill_glass_surface)
                                                            .rounded(move || theme().radius)
                                                            .border_1()
                                                            .border_color(move || theme().input)
                                                            .focus(|s| {
                                                                s.border_color(move || theme().ring)
                                                            }),
                                                    ),
                                            )
                                            // Hairline divider
                                            .child(view().w_full().h(px(1.0)).bg(glass_hairline))
                                            // Row 2: Host a jam + Start pill
                                            .child(
                                                view()
                                                    .flex_row()
                                                    .items_center()
                                                    .justify_between()
                                                    .gap_4()
                                                    .child(
                                                        view()
                                                            .flex_1()
                                                            .min_w(px(0.0))
                                                            .gap_0p5()
                                                            .child(
                                                                text("Host a jam")
                                                                    .text_sm()
                                                                    .font_weight(
                                                                        FontWeight::SEMIBOLD,
                                                                    )
                                                                    .text_color(move || {
                                                                        theme().foreground
                                                                    }),
                                                            )
                                                            .child(
                                                                text("You get a code and a link to share. You control playback.")
                                                                    .text_xs()
                                                                    .text_color(move || {
                                                                        theme().muted_foreground
                                                                    }),
                                                            ),
                                                    )
                                                    .child(hero_pill_button(
                                                        "Start",
                                                        Some(IconKind::ListenTogether),
                                                        move || {
                                                            cb_c_inner(
                                                                relay_url_draft.get(),
                                                                display_name_draft.get(),
                                                            );
                                                        },
                                                    )),
                                            )
                                            // Optional collapsible Relay server URL
                                            .child(when(
                                                move || show_relay_field.get(),
                                                move || {
                                                    view()
                                                        .pt_2()
                                                        .border_t_1()
                                                        .border_color(glass_hairline)
                                                        .gap_1p5()
                                                        .child(
                                                            text("Relay server URL")
                                                                .text_xs()
                                                                .font_weight(FontWeight::MEDIUM)
                                                                .text_color(move || {
                                                                    theme().muted_foreground
                                                                }),
                                                        )
                                                        .child(
                                                            input()
                                                                .test_id("room-relay-url-input")
                                                                .value(move || {
                                                                    relay_url_draft.get()
                                                                })
                                                                .on_change(move |v: &str| {
                                                                    relay_url_draft
                                                                        .set(v.to_owned())
                                                                })
                                                                .on_focus(|| {
                                                                    set_text_input_focused(true)
                                                                })
                                                                .on_blur(|| {
                                                                    set_text_input_focused(false)
                                                                })
                                                                .placeholder(
                                                                    "https://relay.raag.audio",
                                                                )
                                                                .placeholder_color(|| {
                                                                    color(
                                                                        theme().muted_foreground,
                                                                    )
                                                                })
                                                                .caret_color(|| {
                                                                    color(theme().primary)
                                                                })
                                                                .w_full()
                                                                .min_h(px(MOBILE_MIN_TOUCH_TARGET))
                                                                .px_3()
                                                                .text_xs()
                                                                .text_color(move || {
                                                                    theme().foreground
                                                                })
                                                                .bg(pill_glass_surface)
                                                                .rounded(move || theme().radius)
                                                                .border_1()
                                                                .border_color(move || theme().input)
                                                                .focus(|s| {
                                                                    s.border_color(move || {
                                                                        theme().ring
                                                                    })
                                                                }),
                                                        )
                                                },
                                            )),
                                    ),
                            )
                            // Section 2: Join a jam (matches 05_listen_together_1.png)
                            .child(
                                view()
                                    .gap_3()
                                    .child(
                                        text("Join a jam")
                                            .text_base()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(move || theme().foreground),
                                    )
                                    .child(
                                        view()
                                            .p_5()
                                            .gap_3()
                                            .rounded(radius_card)
                                            .bg(frosted_settings_card)
                                            .border_1()
                                            .border_color(glass_hairline)
                                            .child(
                                                text("Type the code from the host, or open the link they shared.")
                                                    .text_xs()
                                                    .text_color(move || theme().muted_foreground),
                                            )
                                            .child(
                                                view()
                                                    .flex_row()
                                                    .items_center()
                                                    .gap_3()
                                                    .child(
                                                        input()
                                                            .test_id("room-join-code-input")
                                                            .value(move || join_code_draft.get())
                                                            .on_change(move |v: &str| {
                                                                join_code_draft
                                                                    .set(v.to_uppercase())
                                                            })
                                                            .on_focus(|| {
                                                                set_text_input_focused(true)
                                                            })
                                                            .on_blur(|| {
                                                                set_text_input_focused(false)
                                                            })
                                                            .placeholder("ABC234")
                                                            .placeholder_color(|| {
                                                                color(theme().muted_foreground)
                                                            })
                                                            .caret_color(|| color(theme().primary))
                                                            .flex_1()
                                                            .min_w(px(0.0))
                                                            .min_h(px(if desktop {
                                                                40.0
                                                            } else {
                                                                MOBILE_MIN_TOUCH_TARGET
                                                            }))
                                                            .px_3p5()
                                                            .text_sm()
                                                            .text_color(move || theme().foreground)
                                                            .bg(pill_glass_surface)
                                                            .rounded(move || theme().radius)
                                                            .border_1()
                                                            .border_color(move || theme().input)
                                                            .focus(|s| {
                                                                s.border_color(move || theme().ring)
                                                            }),
                                                    )
                                                    .child(hero_pill_button("Join", None, move || {
                                                        cb_j_inner(
                                                            relay_url_draft.get(),
                                                            join_code_draft.get(),
                                                            display_name_draft.get(),
                                                        );
                                                    })),
                                            ),
                                    ),
                            )
                            // Subtle footer utility row: Instant Radio Mix + Relay config toggle
                            .child(
                                view()
                                    .flex_row()
                                    .flex_wrap()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(glass_pill_button(
                                        "Start Radio Mix",
                                        Some(IconKind::Radio),
                                        move || cb_r_inner(),
                                    ))
                                    .child(glass_pill_button(
                                        move || {
                                            if show_relay_field.get() {
                                                SharedString::from("Hide relay server")
                                            } else {
                                                SharedString::from("Custom relay server")
                                            }
                                        },
                                        None,
                                        move || {
                                            show_relay_field
                                                .set(!show_relay_field.get_untracked());
                                        },
                                    )),
                            )
                            .into_element()
                    }
                })),
        )
}
