//! Resonance — 100% Pure Rust YouTube Music client built on Native Shell (`nsh.dev`).
//!
//! Targets Android (API 26+) and Desktop with zero TSX in the app.

#[path = "../platform/mod.rs"]
pub mod platform;
pub mod live_engine;
pub mod screens;
pub mod store;
pub mod ui;

use nsh_base::theme;
use nsh_core::prelude::*;

#[native_module("resonance/native")]
fn native(_registry: &mut Registry) {}

/// Root Pure Rust NSH component referenced by `nsh.json` (`"root": "App"`).
#[component]
pub fn App() -> impl IntoElement {
    platform::nsh_audio::mount_nsh_audio_driver();
    store::mount_live_engine_bridge();
    view()
        .flex_1()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(screens::shell::app_shell())
}
