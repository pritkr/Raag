//! Reusable navigation primitives, mobile bottom nav bar, and global keyboard shortcuts.
//! Zero raw color literals — semantic theme tokens only.

use std::sync::atomic::{AtomicBool, Ordering};

use nsh_base::{button, theme};
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::store::{self, AppTab};
use crate::ui::controls::sleep_timer_sheet::close_sleep_timer_sheet;
use crate::ui::controls::speed_sheet::close_speed_sheet;
use crate::ui::icons::{IconKind, icon};
use crate::ui::tokens::{
    MOBILE_MIN_TOUCH_TARGET, frosted_bar_surface, glass_hairline, pill_glass_hover,
    sidebar_active_surface,
};

static TEXT_INPUT_FOCUSED: AtomicBool = AtomicBool::new(false);

pub fn set_text_input_focused(focused: bool) {
    TEXT_INPUT_FOCUSED.store(focused, Ordering::SeqCst);
}

pub fn is_text_input_focused() -> bool {
    TEXT_INPUT_FOCUSED.load(Ordering::SeqCst)
}

/// Accessible navigation pill button (>= 48dp touch height).
pub fn nav_pill_button(
    kind: IconKind,
    label: &'static str,
    active: impl Fn() -> bool + Copy + 'static,
    on_press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .on_press(on_press)
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .px_3()
        .flex_row()
        .items_center()
        .gap_3()
        .rounded(move || theme().radius)
        .bg(move || {
            if active() {
                sidebar_active_surface()
            } else {
                theme().background
            }
        })
        .hover(|s| s.bg(pill_glass_hover))
        .active(|s| s.opacity(0.82))
        .child(icon(kind, 18.0, move || {
            if active() {
                theme().foreground
            } else {
                theme().muted_foreground
            }
        }))
        .child(
            text(label)
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

/// Consistent shelf / section header row with title and optional trailing subtitle.
pub fn section_header_row(
    title: impl IntoReactive<SharedString>,
    trailing: impl IntoReactive<SharedString>,
) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            text(title)
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(trailing)
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

/// Reusable Mobile Bottom Navigation Bar with `.flex_shrink_0()`, `.min_h(px(62.0))`,
/// `.pb_safe_area_plus(px(6.0))`, and `>= 48dp` targets so labels are never clipped.
pub fn mobile_bottom_nav_bar<I>(items: I) -> impl IntoElement
where
    I: IntoIterator,
    I::Item: IntoElement,
{
    view()
        .test_id("mobile-bottom-nav")
        .w_full()
        .flex_shrink_0()
        .min_h(px(62.0))
        .pt_1p5()
        .pb_safe_area_plus(px(6.0))
        .px_2()
        .flex_row()
        .items_center()
        .justify_around()
        .bg(frosted_bar_surface)
        .border_t_1()
        .border_color(glass_hairline)
        .children(items)
}

/// Responsive shell layout helper notes for Wave 1.5 integration into `shell.rs`.
pub fn desktop_responsive_shell_helpers() -> (bool, u32) {
    (
        crate::ui::tokens::breakpoint_is_desktop(),
        crate::ui::tokens::grid_columns(),
    )
}

/// Returns `true` if `key` is a recognized global shortcut and is allowed under `input_focused`.
pub fn is_shortcut_allowed(key: &str, input_focused: bool) -> bool {
    if input_focused && !matches!(key, "mod+k" | "escape") {
        return false;
    }
    matches!(
        key,
        "mod+k"
            | "escape"
            | "space"
            | "left"
            | "right"
            | "shift+left"
            | "shift+right"
            | "n"
            | "p"
            | "l"
            | "m"
            | "s"
            | "r"
            | "up"
            | "down"
            | "/"
    )
}

/// Pure, testable global keyboard shortcut dispatcher.
/// Single-letter and arrow shortcuts are suppressed when `input_focused == true`;
/// modifier shortcuts (`mod+k`, `escape`) always execute.
pub fn dispatch_global_shortcut(key: &str, input_focused: bool) -> bool {
    match key {
        "mod+k" => {
            store::set_tab(AppTab::Search);
            true
        }
        "escape" => {
            if store::add_to_playlist_track().is_some() {
                store::close_add_to_playlist_sheet();
                return true;
            }
            if store::active_track_sheet().is_some() {
                store::close_track_sheet();
                return true;
            }
            close_sleep_timer_sheet();
            close_speed_sheet();
            if store::active_tab() == AppTab::NowPlaying {
                store::go_back();
            }
            true
        }
        _ if input_focused => false,
        "space" => {
            store::toggle_play_pause();
            true
        }
        "left" => {
            let pos = store::player_snapshot().position_ms;
            store::seek_ms(pos.saturating_sub(5_000));
            true
        }
        "right" => {
            let pos = store::player_snapshot().position_ms;
            store::seek_ms(pos.saturating_add(5_000));
            true
        }
        "shift+left" => {
            let pos = store::player_snapshot().position_ms;
            store::seek_ms(pos.saturating_sub(15_000));
            true
        }
        "shift+right" => {
            let pos = store::player_snapshot().position_ms;
            store::seek_ms(pos.saturating_add(15_000));
            true
        }
        "n" => {
            store::next_track();
            true
        }
        "p" => {
            store::previous_track();
            true
        }
        "l" => {
            store::toggle_like_current();
            true
        }
        "m" => {
            store::toggle_mute();
            true
        }
        "s" => {
            store::toggle_shuffle();
            true
        }
        "r" => {
            store::cycle_repeat();
            true
        }
        "up" => {
            store::adjust_volume(0.05);
            true
        }
        "down" => {
            store::adjust_volume(-0.05);
            true
        }
        "/" => {
            store::set_tab(AppTab::Search);
            true
        }
        _ => false,
    }
}

/// Registers the full global keyboard shortcut map on the NSH window (stitched into `src/lib.rs` in Wave 1.5).
pub fn register_global_shortcuts() {
    let not_editing = KeyOptions {
        when: When::NotEditing,
        ..Default::default()
    };
    for key in [
        "space",
        "left",
        "right",
        "shift+left",
        "shift+right",
        "n",
        "p",
        "l",
        "m",
        "s",
        "r",
        "up",
        "down",
        "/",
    ] {
        on_key_with(key, not_editing, move |_| {
            dispatch_global_shortcut(key, is_text_input_focused());
        });
    }
    on_key("mod+k", |_| {
        dispatch_global_shortcut("mod+k", is_text_input_focused());
    });
    on_key("escape", |_| {
        dispatch_global_shortcut("escape", is_text_input_focused());
    });
}
