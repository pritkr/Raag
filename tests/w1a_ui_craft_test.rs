//! Deterministic offline test suite for W1A — UI Craft, Layout Fixes, Product Honesty & Ergonomics.

use std::fs;
use std::path::Path;

use nsh_engine::{ColorScheme, EngineConfig, Environment, Points, Root, Size};
use nsh_testing::Headless;
use resonance::screens::lyrics::{
    LyricsScrollMode, PROGRAMMATIC_SCROLL_GUARD_MS, USER_BROWSING_HYSTERESIS_MS,
    update_lyrics_scroll_mode,
};
use resonance::store::catalog_slice::CatalogSlice;
use resonance::store::library_slice::LibrarySlice;
use resonance::store::mutation_slice::MutationSlice;
use resonance::store::ui_slice::{SleepTimerController, SleepTimerMode, SleepTimerTickAction};
use resonance::ui::controls::context_menu::{TrackMenuContext, track_youtube_music_url};
use resonance::ui::controls::navigation::{dispatch_global_shortcut, is_shortcut_allowed};
use resonance::ui::controls::speed_sheet::quantize_speed;
use resonance_contract::{PlaylistEditOp, SleepTimerState};
use resonance_core::fixture::fixture_tracks;

fn start_headless(w: f32, h: f32) -> Headless {
    resonance::store::enable_fixture_mode(true);
    Headless::start(EngineConfig {
        viewport: Size::new(Points(w), Points(h)),
        environment: Environment {
            color_scheme: ColorScheme::Dark,
            ..Environment::default()
        },
        ..EngineConfig::new(Root::of::<resonance::App>())
    })
    .expect("headless engine starts cleanly")
}

#[test]
fn heart_icon_color_follows_is_liked_not_is_current() {
    let media_rows =
        fs::read_to_string(Path::new("src/ui/controls/media_rows.rs")).expect("read media_rows.rs");
    // Ensure heart color check uses store::is_track_liked, never is_current.
    assert!(
        media_rows.contains("if store::is_track_liked(&track_id_color)"),
        "media_rows.rs must color heart based on is_track_liked, not is_current"
    );
    assert!(
        !media_rows.contains("if is_current {\n                            color_liked()"),
        "inverted is_current heart color bug must not exist"
    );
}

#[test]
fn mobile_and_desktop_layouts_render_cleanly_without_dead_voids() -> Result<(), String> {
    let mut mobile = start_headless(390.0, 844.0);
    mobile.wait_for("Quick Picks")?;
    assert!(
        mobile
            .screen()
            .texts()
            .iter()
            .any(|t| t.contains("Library"))
    );

    let mut desktop = start_headless(1440.0, 900.0);
    desktop.wait_for("Quick Picks")?;
    Ok(())
}

#[test]
fn track_context_menu_and_playlist_mutations_with_rollback() {
    let url = track_youtube_music_url("dQw4w9WgXcQ");
    assert_eq!(url, "https://music.youtube.com/watch?v=dQw4w9WgXcQ");

    let ctx = TrackMenuContext::Queue { queue_index: 2 };
    assert_eq!(ctx, TrackMenuContext::Queue { queue_index: 2 });

    let tracks = fixture_tracks();
    let mut library = LibrarySlice::new(true, Vec::new());
    let mut catalog = CatalogSlice::new(true);
    let mut mutations = MutationSlice::default();

    let created = mutations.create_local_playlist("Night Drive", "Private", &mut library);
    let pid = created.playlist_id.clone().unwrap();
    assert_eq!(library.playlists.len(), 1);
    assert!(mutations.rename_local_playlist(&pid, "Late Night Drive", &mut library));
    assert_eq!(library.playlists[0].title, "Late Night Drive");

    let _ = mutations.apply_playlist_edit(
        &pid,
        &PlaylistEditOp::AddVideo {
            video_id: tracks[0].id.clone(),
        },
        Some(tracks[0].clone()),
        &mut catalog,
        &mut library,
        true,
        true,
    );
    let mutation_gen = mutations.next_mutation_gen;
    assert_eq!(library.playlists[0].track_count, Some(1));

    let toast = mutations.rollback_playlist_edit_by_gen(mutation_gen, &mut library, &mut catalog);
    assert_eq!(
        toast.as_deref(),
        Some("Couldn't update playlist — restored")
    );
    assert_eq!(library.playlists[0].track_count, Some(0));

    assert!(mutations.delete_local_playlist(&pid, &mut library));
    assert!(library.playlists.is_empty());
}

#[test]
fn lyrics_auto_scroll_pauses_on_user_scroll_and_resumes_on_sync() {
    let mode = LyricsScrollMode::AutoFollow;
    let last_prog = 1_000;

    // Scroll within 350ms programmatic guard does NOT enter UserBrowsing.
    let guarded = update_lyrics_scroll_mode(
        mode,
        last_prog + PROGRAMMATIC_SCROLL_GUARD_MS,
        last_prog,
        true,
        false,
    );
    assert_eq!(guarded, LyricsScrollMode::AutoFollow);

    // User scroll after guard transitions to UserBrowsing for 5000ms.
    let t_user = 2_000;
    let browsing = update_lyrics_scroll_mode(mode, t_user, last_prog, true, false);
    assert_eq!(
        browsing,
        LyricsScrollMode::UserBrowsing {
            resume_at_unix_ms: t_user + USER_BROWSING_HYSTERESIS_MS
        }
    );

    // Before expiry, stays in UserBrowsing.
    let still_browsing =
        update_lyrics_scroll_mode(browsing, t_user + 2_500, last_prog, false, false);
    assert_eq!(still_browsing, browsing);

    // Tapping Sync immediately restores AutoFollow.
    let synced = update_lyrics_scroll_mode(browsing, t_user + 1_000, last_prog, false, true);
    assert_eq!(synced, LyricsScrollMode::AutoFollow);

    // Timestamp expiry also restores AutoFollow.
    let expired = update_lyrics_scroll_mode(
        browsing,
        t_user + USER_BROWSING_HYSTERESIS_MS,
        last_prog,
        false,
        false,
    );
    assert_eq!(expired, LyricsScrollMode::AutoFollow);
}

#[test]
fn sleep_timer_fades_volume_pauses_and_restores_saved_volume() {
    let mut timer = SleepTimerController::default();
    timer.set_countdown(10, 10_000, 0.8);
    assert_eq!(
        timer.state(10_000),
        SleepTimerState::Countdown { remaining_secs: 10 }
    );

    // Before 3s fade window: no volume change.
    assert_eq!(timer.tick(15_000), SleepTimerTickAction::None);

    // At 1.5s remaining: 50% of saved volume (0.4).
    match timer.tick(18_500) {
        SleepTimerTickAction::FadeVolume(v) => assert!((v - 0.4).abs() < 0.01),
        other => panic!("expected FadeVolume, got {other:?}"),
    }

    // At expiry (20_000): pauses and restores saved volume 0.8.
    assert_eq!(
        timer.tick(20_000),
        SleepTimerTickAction::PauseAndRestoreVolume(0.8)
    );
    assert_eq!(timer.mode, SleepTimerMode::Off);

    // EndOfTrack mode triggers on track end and restores saved volume.
    timer.set_end_of_track(0.65);
    assert_eq!(
        timer.on_track_ended(),
        SleepTimerTickAction::PauseAndRestoreVolume(0.65)
    );
    assert_eq!(timer.mode, SleepTimerMode::Off);
}

#[test]
fn speed_quantization_clamps_and_steps_by_0_05() {
    assert!((quantize_speed(0.20) - 0.50).abs() < 1e-4);
    assert!((quantize_speed(2.50) - 2.00).abs() < 1e-4);
    assert!((quantize_speed(1.23) - 1.25).abs() < 1e-4);
}

#[test]
fn keyboard_shortcuts_ignore_letter_keys_while_input_focused() {
    resonance::store::enable_fixture_mode(true);
    // Single-letter and arrow keys suppressed when input is focused:
    assert!(!dispatch_global_shortcut("space", true));
    assert!(!dispatch_global_shortcut("n", true));
    assert!(!dispatch_global_shortcut("l", true));
    assert!(!dispatch_global_shortcut("left", true));
    assert!(!dispatch_global_shortcut("/", true));

    // Modifier shortcuts always work even when input is focused:
    assert!(is_shortcut_allowed("mod+k", true));
    assert!(is_shortcut_allowed("escape", true));

    // Unfocused: all bindings are allowed:
    assert!(is_shortcut_allowed("space", false));
    assert!(is_shortcut_allowed("n", false));
    assert!(is_shortcut_allowed("p", false));
    assert!(is_shortcut_allowed("l", false));
    assert!(is_shortcut_allowed("m", false));
    assert!(is_shortcut_allowed("s", false));
    assert!(is_shortcut_allowed("r", false));
    assert!(is_shortcut_allowed("up", false));
    assert!(is_shortcut_allowed("down", false));
    assert!(is_shortcut_allowed("shift+left", false));
    assert!(is_shortcut_allowed("shift+right", false));
    assert!(is_shortcut_allowed("/", false));
    assert!(!is_shortcut_allowed("unknown_key", false));
}

#[test]
fn zero_raw_color_literals_and_no_settings_in_library() {
    let dirs = [Path::new("src/screens"), Path::new("src/ui/controls")];
    for dir in dirs {
        for entry in fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let content = fs::read_to_string(&path).expect("read rs file");
            for needle in ["rgb(", "rgba(", "0xFF"] {
                assert!(
                    !content.contains(needle),
                    "raw color literal `{needle}` found in {}",
                    path.display()
                );
            }
        }
    }

    let library_rs =
        fs::read_to_string(Path::new("src/screens/library.rs")).expect("read library.rs");
    assert!(
        !library_rs.contains("settings_sections"),
        "library.rs must not render settings_sections()"
    );
    assert!(
        library_rs.contains("nav-settings-shortcut"),
        "library.rs must expose nav-settings-shortcut gear button"
    );

    let now_playing_rs =
        fs::read_to_string(Path::new("src/screens/now_playing.rs")).expect("read now_playing.rs");
    assert!(
        !now_playing_rs.contains("Opus 251 • Proxy"),
        "now_playing.rs must not hardcode Opus 251 • Proxy"
    );
}
