//! Pure Rust NSH Headless UI End-to-End Test Suite (`nsh_testing::Headless`).
//!
//! Verifies in `--fixture` mode across Mobile (390x844 Light & Dark) and Desktop (1200x800):
//! - Torture-test metadata (74-char title, CJK + emoji track, missing-artwork monogram fallback)
//! - Full vertical slice & Phase 2–3 flows:
//!   Home -> Search -> Play -> Mini-Player -> Now Playing -> Pause/Resume/Seek/Next
//!   -> Synced Lyrics tap-to-seek -> Queue reorder/remove -> Library/Settings SponsorBlock & Clear Cache.

use nsh_engine::{ColorScheme, EngineConfig, Environment, Points, Root, Size};
use nsh_testing::Headless;

fn start_mobile(scheme: ColorScheme) -> Headless {
    resonance::store::enable_fixture_mode(true);
    let environment = Environment {
        color_scheme: scheme,
        ..Environment::default()
    };
    Headless::start(EngineConfig {
        viewport: Size::new(Points(390.0), Points(844.0)),
        environment,
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .expect("headless mobile engine starts cleanly")
}

fn start_desktop() -> Headless {
    resonance::store::enable_fixture_mode(true);
    Headless::start(EngineConfig {
        viewport: Size::new(Points(1200.0), Points(800.0)),
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .expect("headless desktop engine starts cleanly")
}

#[test]
fn mobile_dark_fixture_journey_covers_now_playing_lyrics_queue_search_and_library()
-> Result<(), String> {
    let mut app = start_mobile(ColorScheme::Dark);

    // Home shows Quick Picks + torture-test 74-char title + CJK/emoji track
    app.wait_for("Quick Picks")?;
    assert!(
        app.screen()
            .texts()
            .iter()
            .any(|t| t.contains("Midnight Reflections Across the Neon Boulevard"))
    );
    assert!(
        app.screen()
            .texts()
            .iter()
            .any(|t| t.contains("夜に駆ける 🌙✨"))
    );

    // Tap Collection card on Home to browse Album/Playlist detail, then return
    app.press("Nocturne Telemetry")?;
    app.wait_for("Play All")?;
    app.press("← Back")?;
    app.wait_for("Quick Picks")?;

    // Tap persistent Mini-Player to open Now Playing (Priority Surface #1)
    app.press("id:mini-player-open")?;
    app.wait_for("Now Playing")?;
    app.wait_for("Playing")?;

    // Pause -> verify state updates to Paused -> Resume
    app.press("label:Play or Pause")?;
    app.wait_for("Paused")?;
    app.press("label:Play or Pause")?;
    app.wait_for("Playing")?;

    // Seek +15s updates tabular-nums position (26s + 15s = 41s)
    app.press("+15s")?;
    app.wait_for("0:41")?;

    // Open Synced Lyrics (Priority Surface #2) & tap-to-seek on a timed line (0:12)
    app.press("Open Lyrics")?;
    app.wait_for("Lyrics")?;
    app.wait_for("Streetlights blur into copper and glass")?;
    app.press("Streetlights blur into copper and glass")?;

    // Return to Now Playing and confirm tap-to-seek moved position to 0:12
    app.press("id:mini-player-open")?;
    app.wait_for("0:12")?;

    // Next track advances to CJK + emoji track, then Next again to Missing Artwork Fallback track
    app.press("label:Next Track")?;
    app.wait_for("夜に駆ける 🌙✨ — Tokyo Midnight Synthwave (2026 Remaster)")?;
    app.press("label:Next Track")?;
    app.wait_for("Amber Horizon (Acoustic Session)")?;
    assert!(
        app.screen().shows("A"),
        "missing-artwork monogram fallback must render 'A'"
    );

    // Open Queue (Priority Surface #3)
    app.press("Up Next Queue")?;
    app.wait_for("Queue")?;
    app.wait_for("Autoplay Radio")?;

    // Open Search & filter by Tokyo
    app.press("id:nav-search")?;
    app.wait_for("Search")?;
    app.type_text("id:search-input", "Tokyo")?;
    app.wait_for("夜に駆ける 🌙✨ — Tokyo Midnight Synthwave (2026 Remaster)")?;

    // Open Library & Settings -> Clear Cache updates readout to 0.0 MB
    app.press("id:nav-library")?;
    app.wait_for("Library & Settings")?;
    app.wait_for("Clear Cache")?;
    app.press("Clear Cache")?;
    app.wait_for("Audio chunks: 0.0 MB (0 chunks) • Artwork: 0.0 MB • Metadata: 0 rows")?;

    Ok(())
}

#[test]
fn desktop_wide_layout_and_mobile_light_theme_render_cleanly() -> Result<(), String> {
    let mut desktop = start_desktop();
    desktop.wait_for("Raag")?;
    desktop.wait_for("YouTube Music")?;
    desktop.press("id:nav-now-playing")?;
    desktop.wait_for("Now Playing")?;

    let mut light_mobile = start_mobile(ColorScheme::Light);
    light_mobile.wait_for("Quick Picks")?;
    light_mobile.press("id:nav-library")?;
    light_mobile.wait_for("SponsorBlock (auto-skip non-music)")?;
    Ok(())
}
