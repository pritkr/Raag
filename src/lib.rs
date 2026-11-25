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
use nsh_core::keymap::When;
use nsh_core::prelude::*;

#[native_module("resonance/native")]
fn native(_registry: &mut Registry) {}

/// Root Pure Rust NSH component referenced by `nsh.json` (`"root": "App"`).
#[component]
pub fn App() -> impl IntoElement {
    platform::nsh_audio::mount_nsh_audio_driver();
    store::mount_live_engine_bridge();

    let key_opts = KeyOptions {
        when: When::NotEditing,
        ..Default::default()
    };
    on_key_with("space", key_opts, |_| {
        store::handle_shortcut("Space");
    });
    on_key_with("right", key_opts, |_| {
        store::handle_shortcut("ArrowRight");
    });
    on_key_with("left", key_opts, |_| {
        store::handle_shortcut("ArrowLeft");
    });

    view()
        .flex_1()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(screens::shell::app_shell())
}
