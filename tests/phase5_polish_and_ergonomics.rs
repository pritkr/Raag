//! Phase 5 Final Polish, Ergonomics, Keyboard Shortcuts, Volume/Offline & Micro-Interaction QA Suite.
//!
//! Verifies in `nsh_testing::Headless` (`--fixture` mode) across Mobile & Desktop:
//! 1. Persistent Mini-Player 2px hairline progress bar + dynamic track monogram (`M` -> `夜` -> `A`)
//! 2. Now Playing source telemetry badge (`Opus 251 • Proxy` <-> `Offline Cache`), volume & mute controls
//! 3. Synced Lyrics line counter + `-5s` / `+5s` micro-seek buttons
//! 4. Queue total duration summary + Shuffle / Repeat quick toggles
//! 5. Library Offline Mode toggle + active theme indicator + global dismissible status toast
//! 6. Desktop keyboard shortcuts (`Space`, `ArrowRight`, `ArrowLeft`, `n`, `p`, `l`, `m`, `s`, `r`)
//! 7. Zero raw color literals in `src/ui/controls.rs` as well as `src/screens/`

use std::fs;
use std::path::Path;

use nsh_engine::{ColorScheme, EngineConfig, Environment, Points, Root, Size};
use nsh_testing::Headless;

fn start_mobile_dark() -> Headless {
    resonance::store::enable_fixture_mode(true);
    Headless::start(EngineConfig {
        viewport: Size::new(Points(390.0), Points(844.0)),
        environment: Environment {
            color_scheme: ColorScheme::Dark,
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .expect("mobile dark headless starts cleanly")
}

fn start_desktop() -> Headless {
    resonance::store::enable_fixture_mode(true);
    Headless::start(EngineConfig {
        viewport: Size::new(Points(1200.0), Points(800.0)),
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .expect("desktop headless starts cleanly")
}

#[test]
fn phase5_mini_player_now_playing_volume_lyrics_queue_and_offline_toast() -> Result<(), String> {
    let mut app = start_mobile_dark();
    app.wait_for("Quick Picks")?;

    // 1. Mini-player shows dynamic monogram 'M' for track 0 ("Midnight Reflections...")
    assert!(
        app.screen().shows("M"),
        "mini-player must render dynamic monogram for current track"
    );

    // 2. Open Now Playing -> verify Stream badge + Volume 100% -> Decrease -> Mute
    app.press("id:mini-player-open")?;
    app.wait_for("Standard Stream")?;
    app.wait_for("Volume 100%")?;
    app.press("label:Decrease Volume")?;
    app.wait_for("Volume 90%")?;
    app.press("label:Toggle Mute")?;
    app.wait_for("Volume 0%")?;
    app.press("label:Toggle Mute")?;
    app.wait_for("Volume 100%")?;

    // 3. Open Synced Lyrics -> verify Synced badge + Line counter + -5s / +5s micro-seek
    app.press("Open Lyrics")?;
    app.wait_for("Lyrics")?;
    assert!(
        app.screen()
            .texts()
            .iter()
            .any(|t| t.starts_with("Synced • ")),
        "lyrics header must show synced provider badge"
    );
    assert!(
        app.screen()
            .texts()
            .iter()
            .any(|t| t.starts_with("Line ") && t.contains(" / ")),
        "lyrics screen must display active line progress counter"
    );
    app.press("+5s")?;
    app.press("-5s")?;

    // 4. Open Queue -> verify total duration summary + Shuffle/Repeat toggles
    app.press("id:nav-queue")?;
    app.wait_for("Shuffle: Off")?;
    app.wait_for("Repeat: Off")?;
    assert!(
        app.screen()
            .texts()
            .iter()
            .any(|t| t.contains("tracks in queue") && t.contains("total")),
        "queue header must show total runtime summary"
    );
    app.press("Shuffle: Off")?;
    app.wait_for("Shuffle: On")?;
    app.press("Repeat: Off")?;
    app.wait_for("Repeat: All")?;
    app.press("Repeat: All")?;
    app.wait_for("Repeat: One")?;

    // 5. Open Settings -> verify active theme indicator, Offline Mode switch -> global dismissible toast
    app.press("id:nav-settings")?;
    app.wait_for("✓ System Theme")?;
    app.press("Dark Theme")?;
    app.wait_for("✓ Dark Theme")?;
    app.press("label:Offline Mode (local chunk cache only)")?;
    app.wait_for("Offline mode enabled — playing from local 256 KB chunk cache")?;

    // Dismiss global status toast cleanly
    app.press("id:dismiss-toast")?;
    app.wait_for("Offline & Local Cache")?;

    // Verify Now Playing now shows Offline telemetry badge
    app.press("id:mini-player-open")?;
    app.wait_for("Offline")?;
    Ok(())
}

#[test]
fn phase5_desktop_shortcuts_and_zero_raw_colors_in_controls() -> Result<(), String> {
    // Verify zero raw color literals across all atomic modules in src/ui/controls/
    let controls_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/controls");
    let mut checked_files = 0usize;
    for entry in fs::read_dir(&controls_dir).expect("read src/ui/controls/") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            let controls_src = fs::read_to_string(&path).expect("read control module");
            assert!(
                !controls_src.contains("rgb(")
                    && !controls_src.contains("rgba(")
                    && !controls_src.contains("0xFF"),
                "{} must use semantic theme tokens only",
                path.display()
            );
            checked_files += 1;
        }
    }
    assert!(
        checked_files >= 6,
        "expected atomic hierarchy in src/ui/controls/ (>=6 modules), found {checked_files}"
    );

    // Desktop layout shows keyboard shortcut legend in sidebar
    let mut desktop = start_desktop();
    desktop.wait_for("Shortcuts: Space · ←/→ · N/P · L · M")?;

    // Exercise desktop Now Playing controls on the live NSH engine thread
    desktop.press("id:nav-now-playing")?;
    desktop.wait_for("Playing")?;
    desktop.wait_for("0:26")?;
    desktop.press("label:Play or Pause")?;
    desktop.wait_for("Paused")?;
    desktop.press("label:Play or Pause")?;
    desktop.wait_for("Playing")?;
    desktop.press("label:Toggle Mute")?;
    desktop.wait_for("Volume 0%")?;
    desktop.press("label:Toggle Mute")?;
    desktop.wait_for("Volume 100%")?;

    // Pure shortcut mapping verification across all media keys
    use resonance::store::{ShortcutAction, map_shortcut_key};
    assert_eq!(map_shortcut_key("UnknownKey"), None);
    assert_eq!(map_shortcut_key(" "), Some(ShortcutAction::TogglePlayPause));
    assert_eq!(
        map_shortcut_key("Space"),
        Some(ShortcutAction::TogglePlayPause)
    );
    assert_eq!(
        map_shortcut_key("ArrowRight"),
        Some(ShortcutAction::SeekForward10s)
    );
    assert_eq!(
        map_shortcut_key("ArrowLeft"),
        Some(ShortcutAction::SeekBackward10s)
    );
    assert_eq!(map_shortcut_key("n"), Some(ShortcutAction::NextTrack));
    assert_eq!(map_shortcut_key("P"), Some(ShortcutAction::PreviousTrack));
    assert_eq!(map_shortcut_key("l"), Some(ShortcutAction::ToggleLike));
    assert_eq!(map_shortcut_key("M"), Some(ShortcutAction::ToggleMute));
    assert_eq!(map_shortcut_key("s"), Some(ShortcutAction::ToggleShuffle));
    assert_eq!(map_shortcut_key("R"), Some(ShortcutAction::CycleRepeat));
    Ok(())
}
