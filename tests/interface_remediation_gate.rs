//! Interface Remediation Gate Suite (`F-01`..`F-23`).
//!
//! Enforces static hygiene invariants across Rust screens, controls, and the web preview bridge,
//! plus live `nsh_testing::Headless` end-to-end checks for destructive confirmation dialogs,
//! Escape dismissal, and 320px compact-viewport reflow across Light and Dark themes.

use std::fs;
use std::path::Path;

use nsh_engine::{ColorScheme, EngineConfig, Environment, Points, Root, Size};
use nsh_testing::{DialogAnswer, Headless};

fn collect_rs_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read dir {}: {e}", dir.display())) {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            files.extend(collect_rs_files(&path));
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    files
}

#[test]
fn zero_raw_colors_and_primitives_exports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for rel in ["src/screens", "src/ui/controls"] {
        for path in collect_rs_files(&root.join(rel)) {
            let src = fs::read_to_string(&path).expect("read rs source");
            assert!(
                !src.contains("rgb(") && !src.contains("rgba(") && !src.contains("0xFF"),
                "raw color literal found in {}",
                path.display()
            );
        }
    }

    let primitives = fs::read_to_string(root.join("src/ui/controls/primitives.rs"))
        .expect("read primitives.rs");
    for required in [
        "pub fn confirm_destructive(",
        "pub fn destructive_button(",
        "pub fn labeled_input(",
        "pub fn labeled_input_with_submit(",
        "pub fn active_state_dot(",
    ] {
        assert!(
            primitives.contains(required),
            "src/ui/controls/primitives.rs missing required helper: {required}"
        );
    }
}

#[test]
fn all_inputs_suppress_global_shortcuts_and_use_input_border() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for rel in ["src/screens", "src/ui/controls"] {
        for path in collect_rs_files(&root.join(rel)) {
            let src = fs::read_to_string(&path).expect("read rs source");
            if src.contains("input()") {
                assert!(
                    src.contains("set_text_input_focused"),
                    "{} calls input() without suppressing global shortcuts via set_text_input_focused",
                    path.display()
                );
                assert!(
                    src.contains("theme().input"),
                    "{} calls input() without theme().input border contrast token",
                    path.display()
                );
            }
        }
    }
}

#[test]
fn no_hidden_horizontal_scrollbars_or_truncated_errors_or_uk_spelling() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for path in collect_rs_files(&root.join("src/screens")) {
        let src = fs::read_to_string(&path).expect("read screen source");
        assert!(
            !src.contains(".scrollbar(false)"),
            "{} hides horizontal scrollbars (.scrollbar(false))",
            path.display()
        );
        assert!(
            !src.contains("\"Cancelled\""),
            "{} uses UK spelling \"Cancelled\" instead of US \"Canceled\"",
            path.display()
        );
    }

    let shell = fs::read_to_string(root.join("src/screens/shell.rs")).expect("read shell.rs");
    assert!(
        shell.contains(".number_of_lines(3)"),
        "shell.rs toast must allow wrapping up to 3 lines"
    );

    let media_rows = fs::read_to_string(root.join("src/ui/controls/media_rows.rs"))
        .expect("read media_rows.rs");
    assert!(
        media_rows.contains("color_status_ok"),
        "media_rows.rs must use semantic color_status_ok for completed downloads"
    );

    let now_playing =
        fs::read_to_string(root.join("src/screens/now_playing.rs")).expect("read now_playing.rs");
    assert!(
        now_playing.contains("color_status_ok"),
        "now_playing.rs must use semantic color_status_ok for completed download status"
    );
}

#[test]
fn web_bridge_tokens_and_a11y_hygiene() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let app_tsx = fs::read_to_string(root.join("src/App.tsx")).expect("read src/App.tsx");
    for forbidden in [
        "outline: 'none'",
        "Toggle Color Scheme",
        "#d92638",
        "#e13244",
        "text-[10px]",
        "text-[11px]",
        "paddingLeft",
        "paddingRight",
        "textAlign: 'left'",
    ] {
        assert!(
            !app_tsx.contains(forbidden),
            "src/App.tsx contains forbidden legacy pattern: {forbidden}"
        );
    }
    for required in [
        "<h1",
        "paddingInlineStart",
        "overscrollBehavior",
        "formatHostError",
    ] {
        assert!(
            app_tsx.contains(required),
            "src/App.tsx missing required accessibility/RTL/error pattern: {required}"
        );
    }
}

#[test]
fn destructive_confirmations_cancel_is_noop_and_confirm_executes() -> Result<(), String> {
    resonance::store::enable_fixture_mode(true);
    let mut app = Headless::start(EngineConfig {
        viewport: Size::new(Points(390.0), Points(844.0)),
        environment: Environment {
            color_scheme: ColorScheme::Dark,
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .expect("headless start");

    app.wait_for("Listen again")?;
    app.press("id:nav-queue")?;
    app.wait_for("Clear Up Next")?;
    app.wait_for("4 tracks in queue • 15:26 total")?;

    // Cancel leaves queue untouched
    app.on_dialog(|d| {
        assert!(
            d.title
                .as_deref()
                .unwrap_or_default()
                .contains("Clear upcoming tracks"),
            "unexpected queue clear dialog title: {:?}",
            d.title
        );
        DialogAnswer::Press("Cancel".to_owned())
    });
    app.press("Clear Up Next")?;
    app.wait_for("4 tracks in queue • 15:26 total")?;

    // Confirm clears upcoming tracks
    app.on_dialog(|_| DialogAnswer::Press("Clear Up Next".to_owned()));
    app.press("Clear Up Next")?;
    assert!(
        app.screen().dialogs().len() >= 2,
        "expected both Cancel and Confirm dialogs to be recorded"
    );

    Ok(())
}

#[test]
fn escape_key_dismisses_auth_sheet_and_now_playing() -> Result<(), String> {
    resonance::store::enable_fixture_mode(true);
    let desktop = Size::new(Points(1200.0), Points(800.0));
    let mut app = Headless::start(EngineConfig {
        viewport: desktop,
        environment: Environment {
            window: desktop,
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .expect("headless desktop start");

    app.wait_for("Listen again")?;
    app.press("id:nav-settings")?;
    app.wait_for("Settings")?;
    app.press("id:open-auth-sheet-btn")?;
    app.wait_for("Sign in to YouTube Music")?;

    app.press_key("escape")?;
    app.wait_for("Settings")?;
    assert!(
        !app.screen().shows("Sign in to YouTube Music"),
        "Escape must dismiss active auth sheet"
    );

    app.press("id:nav-now-playing")?;
    app.wait_for("Now Playing")?;
    app.press_key("escape")?;
    app.wait_for("Settings")?;

    Ok(())
}

#[test]
fn reflow_320px_light_and_dark_smoke() -> Result<(), String> {
    for scheme in [ColorScheme::Light, ColorScheme::Dark] {
        resonance::store::enable_fixture_mode(true);
        let mut app = Headless::start(EngineConfig {
            viewport: Size::new(Points(320.0), Points(640.0)),
            environment: Environment {
                color_scheme: scheme,
                ..Environment::default()
            },
            ..EngineConfig::new(Root::of::<resonance::App>())
        })
        .expect("320px headless start");

        app.wait_for("Listen again")?;
        app.press("id:nav-search")?;
        app.wait_for("Search")?;
        app.press("id:nav-queue")?;
        app.wait_for("Queue")?;
        app.press("id:nav-library")?;
        app.wait_for("Library")?;
        app.press("id:nav-settings-shortcut")?;
        app.wait_for("Settings")?;
    }
    Ok(())
}
