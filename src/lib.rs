//! Raag — 100% Pure Rust YouTube Music client built on Native Shell (`nsh.dev`).
//!
//! Targets Android (API 26+) and Desktop with zero TSX in the app.

pub mod live_engine;
#[path = "../platform/mod.rs"]
pub mod platform;
pub mod screens;
pub mod store;
pub mod ui;
pub mod updater;

use nsh_base::theme;
use nsh_core::element::desktop::window as app_window;
use nsh_core::prelude::*;

#[native_module("resonance/native")]
fn native(_registry: &mut Registry) {}

/// Root Pure Rust NSH component referenced by `nsh.json` (`"root": "App"`).
///
/// On Desktop (NSH window model v2), the host window is invisible and the app
/// declares its visible main window via `app_window()`. Off Desktop (Android/iOS),
/// `app_window()` renders its content directly in the app's single surface.
#[component]
pub fn App() -> impl IntoElement {
    platform::nsh_audio::mount_nsh_audio_driver();
    store::mount_live_engine_bridge();

    app_window()
        .id("main")
        .title("Raag")
        .default_size(size(px(1240.0), px(820.0)))
        .restore_bounds(true)
        .content(|| {
            crate::ui::controls::register_global_shortcuts();

            view()
                .flex_1()
                .bg(move || theme().background)
                .text_color(move || theme().foreground)
                .child(screens::shell::app_shell())
        })
}
