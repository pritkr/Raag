//! UI Navigation, Settings, Auth, DSP, Rooms, Local Media, Updater & Cast actions
//! operating on their respective isolated domain signals.

use std::path::PathBuf;

use resonance_contract::{
    AccountProfile, AuthBackendStatus, AuthoritativePlaybackState, CastDevice, CastSessionState,
    LocalFolderRoot, LocalScanProgress, LocalTrackRecord, PlaybackSpeedConfig, PoTokenDiagnostics,
    RemoteConfigStatus, RoomParticipant, RoomRole, RoomTrackRef, SleepTimerState, Track,
    UpdateChannel, UpdateInfo, UpdatePhase,
};
use resonance_core::audio_dsp::DspConfig;
use resonance_core::rooms::{RoomStateSnapshot, RoomTransportKind, generate_room_code};

use crate::live_engine::{self, EngineCommand};

use super::rooms_slice::RoomsSlice;
use super::settings_slice::{AudioDspCapability, SettingsSlice, ThemeChoice};
use super::ui_slice::{AppTab, DesktopSidePanel, NowPlayingTab};
use super::updater_slice::UpdaterSlice;
use super::{
    all_library_songs, apply_nsh_appearance, cast_signal, catalog_signal, is_fixture_mode,
    local_media_signal, persist_settings_now, play_all_shuffled, play_queue, play_track,
    playback_signal, player_snapshot, rooms_signal, set_autoplay_radio, settings_signal,
    toggle_play_pause, ui_signal, updater_signal,
};

// ── UI & Navigation ──────────────────────────────────────────────────────────

pub fn active_tab() -> AppTab {
    ui_signal().with(|u| u.active_tab)
}

pub fn set_tab(tab: AppTab) {
    if catalog_signal().with(|c| c.can_go_back_entity()) {
        catalog_signal().update(|c| c.clear_entity_routes());
    }
    ui_signal().update(|u| u.set_tab(tab));
}

pub fn can_go_back() -> bool {
    let has_entity_or_col = catalog_signal()
        .with(|c| c.can_go_back_entity() || c.selected_collection.is_some());
    ui_signal().with(|u| u.can_go_back(has_entity_or_col))
}

pub fn can_go_forward() -> bool {
    ui_signal().with(|u| u.can_go_forward())
}

pub fn go_back() {
    if catalog_signal().with(|c| c.can_go_back_entity()) {
        catalog_signal().update(|c| {
            let _ = c.go_back_entity();
        });
        return;
    }
    let at_home = ui_signal().with(|u| u.active_tab == AppTab::Home);
    let has_col = catalog_signal().with(|c| c.selected_collection.is_some());
    if at_home && has_col {
        catalog_signal().update(|c| c.selected_collection = None);
        return;
    }
    ui_signal().update(|u| u.go_back());
}

pub fn go_forward() {
    ui_signal().update(|u| u.go_forward());
}

pub fn sidebar_collapsed() -> bool {
    ui_signal().with(|u| u.sidebar_collapsed)
}

pub fn toggle_sidebar() {
    ui_signal().update(|u| u.toggle_sidebar());
}

pub fn desktop_side_panel_open() -> bool {
    ui_signal().with(|u| u.desktop_side_panel_open)
}

pub fn toggle_desktop_side_panel(panel: DesktopSidePanel) {
    ui_signal().update(|u| u.toggle_desktop_side_panel(panel));
}

pub fn desktop_side_panel() -> DesktopSidePanel {
    ui_signal().with(|u| u.desktop_side_panel)
}

pub fn set_desktop_side_panel(panel: DesktopSidePanel) {
    ui_signal().update(|u| u.set_desktop_side_panel(panel));
}

pub fn now_playing_tab() -> NowPlayingTab {
    ui_signal().with(|u| u.now_playing_tab)
}

pub fn set_now_playing_tab(tab: NowPlayingTab) {
    ui_signal().update(|u| u.now_playing_tab = tab);
}

pub fn settings_section() -> super::ui_slice::SettingsSection {
    ui_signal().with(|u| u.settings_section)
}

pub fn set_settings_section(section: super::ui_slice::SettingsSection) {
    ui_signal().update(|u| u.settings_section = section);
}

pub fn open_settings_account() {
    if catalog_signal().with(|c| c.can_go_back_entity()) {
        catalog_signal().update(|c| c.clear_entity_routes());
    }
    ui_signal().update(|u| {
        u.settings_section = super::ui_slice::SettingsSection::Account;
        u.set_tab(AppTab::Settings);
    });
}

pub fn open_settings_shortcuts() {
    if catalog_signal().with(|c| c.can_go_back_entity()) {
        catalog_signal().update(|c| c.clear_entity_routes());
    }
    ui_signal().update(|u| {
        u.settings_section = super::ui_slice::SettingsSection::KeyboardShortcuts;
        u.set_tab(AppTab::Settings);
    });
}

pub fn settings_search_query() -> String {
    ui_signal().with(|u| u.settings_search_query.clone())
}

pub fn set_settings_search_query(query: String) {
    ui_signal().update(|u| u.settings_search_query = query);
}

pub fn search_dropdown_open() -> bool {
    ui_signal().with(|u| u.search_dropdown_open)
}

pub fn set_search_dropdown_open(open: bool) {
    ui_signal().update(|u| u.search_dropdown_open = open);
}

pub fn miniplayer_open() -> bool {
    ui_signal().with(|u| u.miniplayer_open)
}

pub fn toggle_miniplayer() {
    let mut now_open = false;
    ui_signal().update(|u| {
        u.miniplayer_open = !u.miniplayer_open;
        now_open = u.miniplayer_open;
    });
    if now_open {
        set_status_banner("Miniplayer mode enabled");
    } else {
        set_status_banner("Returned to full player");
    }
}

pub fn status_banner() -> Option<String> {
    ui_signal().with(|u| u.status_banner.clone())
}

pub fn set_status_banner(msg: impl Into<String>) {
    let text = msg.into();
    ui_signal().update(|u| u.set_status_banner(text));
}

pub fn dismiss_banner() {
    ui_signal().update(|u| u.dismiss_banner());
}

pub fn open_track_sheet(track: Track, playlist_id: Option<String>) {
    ui_signal().update(|u| u.active_track_sheet = Some((track, playlist_id)));
}

pub fn close_track_sheet() {
    ui_signal().update(|u| u.active_track_sheet = None);
}

pub fn active_track_sheet() -> Option<(Track, Option<String>)> {
    ui_signal().with(|u| u.active_track_sheet.clone())
}

pub fn open_add_to_playlist_sheet(track: Track) {
    ui_signal().update(|u| {
        u.active_track_sheet = None;
        u.add_to_playlist_track = Some(track);
    });
}

pub fn close_add_to_playlist_sheet() {
    ui_signal().update(|u| u.add_to_playlist_track = None);
}

pub fn add_to_playlist_track() -> Option<Track> {
    ui_signal().with(|u| u.add_to_playlist_track.clone())
}

pub fn open_sleep_timer_sheet() {
    ui_signal().update(|u| u.sleep_timer_sheet_open = true);
}

pub fn close_sleep_timer_sheet() {
    ui_signal().update(|u| u.sleep_timer_sheet_open = false);
}

pub fn is_sleep_timer_sheet_open() -> bool {
    ui_signal().with(|u| u.sleep_timer_sheet_open)
}

pub fn sleep_timer_mode() -> super::ui_slice::SleepTimerMode {
    ui_signal().with(|u| u.sleep_timer.mode)
}

pub fn sleep_timer_state_at(now_unix_ms: u64) -> SleepTimerState {
    ui_signal().with(|u| u.sleep_timer.state(now_unix_ms))
}

pub fn sleep_timer_badge_at(now_unix_ms: u64) -> Option<String> {
    ui_signal().with(|u| u.sleep_timer.badge_label(now_unix_ms))
}

pub fn set_sleep_timer_countdown_secs(duration_secs: u64, now_unix_ms: u64) {
    let vol = player_snapshot().volume;
    let mut next_state = SleepTimerState::Off;
    ui_signal().update(|u| {
        u.sleep_timer.set_countdown(duration_secs, now_unix_ms, vol);
        next_state = u.sleep_timer.state(now_unix_ms);
        u.sleep_timer_sheet_open = false;
    });
    playback_signal().update(|p| p.set_sleep_timer(next_state));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetSleepTimer(next_state));
    }
}

pub fn set_sleep_timer_end_of_track_mode() {
    let vol = player_snapshot().volume;
    ui_signal().update(|u| {
        u.sleep_timer.set_end_of_track(vol);
        u.sleep_timer_sheet_open = false;
    });
    playback_signal().update(|p| p.set_sleep_timer(SleepTimerState::EndOfTrack));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetSleepTimer(SleepTimerState::EndOfTrack));
    }
}

pub fn extend_sleep_timer_secs(add_secs: u64, now_unix_ms: u64) {
    let mut next_state = SleepTimerState::Off;
    ui_signal().update(|u| {
        u.sleep_timer.extend_minutes(add_secs, now_unix_ms);
        next_state = u.sleep_timer.state(now_unix_ms);
    });
    playback_signal().update(|p| p.set_sleep_timer(next_state));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetSleepTimer(next_state));
    }
}

pub fn cancel_sleep_timer_mode() -> Option<f32> {
    let mut restore = None;
    ui_signal().update(|u| {
        restore = u.sleep_timer.cancel();
        u.sleep_timer_sheet_open = false;
    });
    playback_signal().update(|p| p.set_sleep_timer(SleepTimerState::Off));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetSleepTimer(SleepTimerState::Off));
    }
    restore
}

pub fn tick_sleep_timer_action(now_unix_ms: u64) -> super::ui_slice::SleepTimerTickAction {
    let mut action = super::ui_slice::SleepTimerTickAction::None;
    let mut next_state = SleepTimerState::Off;
    ui_signal().update(|u| {
        action = u.sleep_timer.tick(now_unix_ms);
        next_state = u.sleep_timer.state(now_unix_ms);
    });
    playback_signal().update(|p| {
        if p.sleep_timer != next_state {
            p.set_sleep_timer(next_state);
        }
    });
    action
}

pub fn open_speed_sheet() {
    ui_signal().update(|u| u.speed_sheet_open = true);
}

pub fn close_speed_sheet() {
    ui_signal().update(|u| u.speed_sheet_open = false);
}

pub fn is_speed_sheet_open() -> bool {
    ui_signal().with(|u| u.speed_sheet_open)
}

pub fn playback_speed_config() -> PlaybackSpeedConfig {
    settings_signal().with(|s| PlaybackSpeedConfig {
        speed: s.playback_speed,
        preserve_pitch: s.preserve_pitch,
    })
}

pub fn set_playback_speed(speed: f32) {
    let clamped = (speed.clamp(0.50, 2.00) * 20.0).round() / 20.0;
    let mut preserve_pitch = true;
    settings_signal().update(|s| {
        s.playback_speed = clamped;
        preserve_pitch = s.preserve_pitch;
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetPlaybackSpeed {
            speed: clamped,
            preserve_pitch,
        });
    }
    persist_settings_now(true);
}

pub fn set_preserve_pitch(preserve: bool) {
    let mut speed = 1.0_f32;
    settings_signal().update(|s| {
        s.preserve_pitch = preserve;
        speed = s.playback_speed;
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetPlaybackSpeed {
            speed,
            preserve_pitch: preserve,
        });
    }
    persist_settings_now(true);
}

pub fn set_atr_watchtime_sync(enabled: bool) {
    settings_signal().update(|s| s.atr_watchtime_sync = enabled);
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetAtrSync(enabled));
    }
    persist_settings_now(true);
}

// ── Settings, Auth, DSP & PO-Token Diagnostics ───────────────────────────────

pub fn settings_snapshot() -> SettingsSlice {
    settings_signal().with(|s| s.clone())
}

pub fn open_auth_sheet() {
    settings_signal().update(|s| s.auth_sheet_open = true);
}

pub fn close_auth_sheet() {
    settings_signal().update(|s| s.auth_sheet_open = false);
}

pub fn auth_sheet_open() -> bool {
    settings_signal().with(|s| s.auth_sheet_open)
}

pub fn start_tv_oauth() {
    settings_signal().update(|s| {
        s.tv_oauth_user_code = None;
        s.tv_oauth_verification_url = Some("https://www.google.com/device".to_owned());
        s.tv_oauth_expires_in_secs = 0;
        s.tv_oauth_status = Some("Requesting TV device code from Google…".to_owned());
    });
    if is_fixture_mode() {
        settings_signal().update(|s| {
            s.tv_oauth_user_code = Some("RAAG-DEMO".to_owned());
            s.tv_oauth_expires_in_secs = 300;
            s.tv_oauth_status =
                Some("Fixture preview — live device-code polling runs outside --fixture mode".to_owned());
        });
        set_status_banner("Fixture preview: TV OAuth device-code UI ready");
    } else {
        live_engine::dispatch(EngineCommand::StartTvOAuth);
    }
}

pub fn cancel_tv_oauth() {
    settings_signal().update(|s| {
        s.tv_oauth_user_code = None;
        s.tv_oauth_verification_url = None;
        s.tv_oauth_expires_in_secs = 0;
        s.tv_oauth_status = None;
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::CancelTvOAuth);
    }
    set_status_banner("TV OAuth sign-in cancelled");
}

pub fn launch_android_webview_sign_in() {
    if is_fixture_mode() {
        set_status_banner(
            "Fixture preview — Android WebView sign-in launches on Android devices",
        );
    } else {
        live_engine::dispatch(EngineCommand::LaunchAndroidWebViewSignIn);
    }
}

pub fn theme_choice() -> ThemeChoice {
    settings_signal().with(|s| s.theme_choice)
}

pub fn set_theme_choice(choice: ThemeChoice) {
    settings_signal().update(|s| s.theme_choice = choice);
    apply_nsh_appearance(choice);
    persist_settings_now(true);
}

pub fn reduced_motion() -> bool {
    settings_signal().with(|s| s.reduced_motion)
}

pub fn set_reduced_motion(enabled: bool) {
    settings_signal().update(|s| s.reduced_motion = enabled);
    persist_settings_now(true);
}

pub fn auth_info() -> (bool, Option<String>) {
    settings_signal().with(|s| (s.signed_in, s.account_name.clone()))
}

pub fn auth_backend_status() -> AuthBackendStatus {
    settings_signal().with(|s| s.auth_backend_status.clone())
}

pub fn sign_in_with_cookies(cookie_input: &str) {
    let trimmed = cookie_input.trim().to_owned();
    let mut accepted = false;
    let mut banner = String::new();
    settings_signal().update(|s| {
        let (ok, msg) = s.sign_in_with_cookies(&trimmed);
        accepted = ok;
        banner = msg;
    });
    ui_signal().update(|u| u.set_status_banner(banner));
    if accepted && !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SignIn(trimmed));
    }
}

pub fn sign_out() {
    let mut banner = String::new();
    settings_signal().update(|s| {
        banner = s.sign_out();
    });
    ui_signal().update(|u| u.set_status_banner(banner));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SignOut);
    }
}

pub fn dsp_config() -> DspConfig {
    settings_signal().with(|s| s.dsp_config.clone())
}

pub fn dsp_capability() -> AudioDspCapability {
    settings_signal().with(|s| s.dsp_capability())
}

pub fn account_profile() -> Option<AccountProfile> {
    settings_signal().with(|s| s.account_profile.clone())
}

pub fn remote_config_status() -> RemoteConfigStatus {
    settings_signal().with(|s| s.remote_config_status.clone())
}

pub fn refresh_remote_config() {
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::RefreshRemoteConfig);
    }
}

fn dispatch_current_dsp() {
    if !is_fixture_mode() {
        let cfg = settings_signal().with(|s| s.dsp_config.clone());
        live_engine::dispatch(EngineCommand::UpdateDsp(cfg));
    }
}

pub fn set_loudness_normalization(enabled: bool) {
    settings_signal().update(|s| s.set_loudness_normalization(enabled));
    ui_signal().update(|u| {
        u.set_status_banner(if enabled {
            "ReplayGain / EBU R128 loudness normalization enabled"
        } else {
            "ReplayGain / EBU R128 loudness normalization disabled"
        });
    });
    dispatch_current_dsp();
    persist_settings_now(true);
}

pub fn set_silence_skip(enabled: bool) {
    settings_signal().update(|s| s.set_silence_skip(enabled));
    ui_signal().update(|u| {
        u.set_status_banner(if enabled {
            "Silence skipping enabled"
        } else {
            "Silence skipping disabled"
        });
    });
    dispatch_current_dsp();
    persist_settings_now(true);
}

pub fn cycle_eq_preset() {
    let mut label = "";
    settings_signal().update(|s| {
        label = s.cycle_eq_preset();
    });
    ui_signal().update(|u| u.set_status_banner(format!("10-band EQ preset: {label}")));
    dispatch_current_dsp();
    persist_settings_now(true);
}

pub fn set_eq_band_gain(band_idx: usize, gain_db: f32) {
    settings_signal().update(|s| s.set_eq_band_gain(band_idx, gain_db));
    dispatch_current_dsp();
    persist_settings_now(false);
}

pub fn reset_eq_flat() {
    settings_signal().update(|s| s.reset_eq_flat());
    ui_signal().update(|u| u.set_status_banner("10-band EQ reset to Flat (0 dB)"));
    dispatch_current_dsp();
    persist_settings_now(true);
}

pub fn cycle_crossfade_ms() {
    let mut ms = 0u32;
    settings_signal().update(|s| {
        ms = s.cycle_crossfade_ms();
    });
    let label = if ms == 0 {
        "Gapless playback (0s crossfade)".to_owned()
    } else {
        format!("Crossfade set to {}s", ms / 1000)
    };
    ui_signal().update(|u| u.set_status_banner(label));
    dispatch_current_dsp();
    persist_settings_now(true);
}

/// Copies `text` to the native OS clipboard and shows a truthful status toast.
pub fn copy_text_to_clipboard(label: &str, text: &str) {
    match crate::platform::copy_to_clipboard(text) {
        Ok(()) => set_status_banner(format!("Copied {label} to clipboard")),
        Err(_) => set_status_banner(format!("{label}: {text}")),
    }
}

pub fn po_token_diagnostics() -> PoTokenDiagnostics {
    settings_signal().with(|s| s.po_token_diagnostics.clone())
}

pub fn set_po_token_diagnostics(diag: PoTokenDiagnostics) {
    settings_signal().update(|s| s.po_token_diagnostics = diag);
}

// ── Rooms, Local Media, Updater & Cast ───────────────────────────────────────

pub fn rooms_snapshot() -> RoomsSlice {
    rooms_signal().with(|r| r.clone())
}

pub fn active_room_snapshot() -> Option<RoomStateSnapshot> {
    rooms_signal().with(|r| r.active_room.clone())
}

pub fn rooms_local_track_warning() -> Option<String> {
    rooms_signal().with(|r| r.local_track_warning.clone())
}

pub fn start_local_radio_mix() {
    set_autoplay_radio(true);
    let snap = player_snapshot();
    if let Some(current) = snap.current_track {
        if !snap.state.is_playing() {
            toggle_play_pause();
        }
        set_status_banner(format!("Radio Mix seeded from \"{}\"", current.title));
        return;
    }
    let tracks = all_library_songs();
    if let Some(first) = tracks.first().cloned() {
        play_queue(tracks, 0);
        set_status_banner(format!("Started Radio Mix with \"{}\"", first.title));
    } else {
        set_tab(AppTab::Home);
    }
}

pub fn create_listen_room(relay_url: String, display_name: String) {
    let name = if display_name.trim().is_empty() {
        "Host".to_owned()
    } else {
        display_name.trim().to_owned()
    };
    let queue_permission = rooms_signal().with(|r| r.queue_permission);
    if !is_fixture_mode() {
        rooms_signal().update(|r| {
            r.relay_url = relay_url.clone();
            r.joining = true;
            r.error = None;
        });
        set_status_banner("Creating Listen Together room…");
        live_engine::dispatch(EngineCommand::CreateRoom {
            relay_url,
            display_name: name,
            queue_permission,
        });
        return;
    }

    let code = generate_room_code();
    let peer_id = "host-local".to_owned();
    let snap = player_snapshot();
    let local_blocked = snap
        .current_track
        .as_ref()
        .is_some_and(|t| resonance_core::rooms::is_local_track(&t.id));
    let current_ref = snap
        .current_track
        .filter(|t| !resonance_core::rooms::is_local_track(&t.id))
        .map(|t| {
            let artist = t.artist_display();
            RoomTrackRef {
                video_id: t.id,
                title: t.title,
                artist,
                duration_ms: t.duration_ms,
                artwork_url: t.artwork_url,
            }
        });
    let room = RoomStateSnapshot {
        room_code: code.clone(),
        role: RoomRole::Host,
        peer_id: peer_id.clone(),
        host_peer_id: peer_id.clone(),
        queue_permission,
        participants: vec![RoomParticipant {
            peer_id,
            display_name: name,
            role: RoomRole::Host,
        }],
        playback: AuthoritativePlaybackState {
            revision: 1,
            current_track: current_ref,
            queue: Vec::new(),
            is_playing: snap.state.is_playing(),
            anchor_position_ms: snap.position_ms,
            anchor_server_ms: resonance_core::unix_secs() * 1000,
        },
        transport: RoomTransportKind::WebSocket,
        clock_offset_ms: 0,
        local_track_blocked: local_blocked,
        connected: true,
    };
    rooms_signal().update(|r| {
        r.relay_url = relay_url;
        r.set_room_snapshot(room);
    });
    set_status_banner(format!("Created preview room {code}"));
}

pub fn join_listen_room(relay_url: String, room_code: String, display_name: String) {
    let code = room_code.trim().to_uppercase();
    if code.is_empty() {
        set_status_banner("Enter a 6-character room code to join");
        return;
    }
    let name = if display_name.trim().is_empty() {
        "Listener".to_owned()
    } else {
        display_name.trim().to_owned()
    };
    if !is_fixture_mode() {
        rooms_signal().update(|r| {
            r.relay_url = relay_url.clone();
            r.room_code_draft = code.clone();
            r.joining = true;
            r.error = None;
        });
        set_status_banner(format!("Joining room {code}…"));
        live_engine::dispatch(EngineCommand::JoinRoom {
            relay_url,
            room_code: code,
            display_name: name,
        });
        return;
    }

    let peer_id = "guest-local".to_owned();
    let room = RoomStateSnapshot {
        room_code: code.clone(),
        role: RoomRole::Guest,
        peer_id: peer_id.clone(),
        host_peer_id: "host-remote".to_owned(),
        queue_permission: rooms_signal().with(|r| r.queue_permission),
        participants: vec![RoomParticipant {
            peer_id,
            display_name: name,
            role: RoomRole::Guest,
        }],
        playback: AuthoritativePlaybackState {
            revision: 1,
            current_track: None,
            queue: Vec::new(),
            is_playing: false,
            anchor_position_ms: 0,
            anchor_server_ms: resonance_core::unix_secs() * 1000,
        },
        transport: RoomTransportKind::WebSocket,
        clock_offset_ms: 0,
        local_track_blocked: false,
        connected: true,
    };
    rooms_signal().update(|r| {
        r.relay_url = relay_url;
        r.room_code_draft = code.clone();
        r.set_room_snapshot(room);
    });
    set_status_banner(format!("Joined preview room {code}"));
}

pub fn leave_listen_room() {
    rooms_signal().update(|r| r.leave_room());
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::LeaveRoom);
    }
    set_status_banner("Left Listen Together room");
}

pub fn vote_skip_room() {
    rooms_signal().update(|r| {
        let next = r.vote_skip_count.saturating_add(1);
        r.set_vote_skip_status(next, r.vote_skip_threshold, true);
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::VoteSkipRoom);
    }
    set_status_banner("Voted to skip track");
}

pub fn send_room_reaction(emoji: String) {
    rooms_signal().update(|r| r.add_reaction(emoji.clone()));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SendRoomReaction(emoji.clone()));
    }
    set_status_banner(format!("Sent reaction {emoji}"));
}

pub fn kick_room_participant(peer_id: String) {
    rooms_signal().update(|r| {
        if let Some(room) = r.active_room.as_mut() {
            room.participants.retain(|p| p.peer_id != peer_id);
        }
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::KickRoomParticipant(peer_id));
    }
    set_status_banner("Removed participant from room");
}

pub fn transfer_room_host(peer_id: String) {
    rooms_signal().update(|r| {
        if let Some(room) = r.active_room.as_mut() {
            room.host_peer_id = peer_id.clone();
            if room.peer_id != peer_id {
                room.role = RoomRole::Guest;
            }
            for p in &mut room.participants {
                p.role = if p.peer_id == peer_id {
                    RoomRole::Host
                } else {
                    RoomRole::Guest
                };
            }
        }
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::TransferRoomHost(peer_id));
    }
    set_status_banner("Transferred room host role");
}

pub fn local_media_roots() -> Vec<LocalFolderRoot> {
    local_media_signal().with(|m| m.roots.clone())
}

pub fn local_media_tracks() -> Vec<LocalTrackRecord> {
    local_media_signal().with(|m| m.tracks.clone())
}

pub fn local_media_progress() -> LocalScanProgress {
    local_media_signal().with(|m| m.scan_progress.clone())
}

pub fn add_local_media_folder(path: PathBuf) {
    let path_str = path.to_string_lossy().trim().to_owned();
    if path_str.is_empty() {
        return;
    }
    let now = resonance_core::unix_secs();
    local_media_signal().update(|m| {
        if !m.roots.iter().any(|r| r.path == path_str) {
            m.roots.push(LocalFolderRoot {
                path: path_str.clone(),
                added_at_unix_secs: now,
                last_scanned_unix_secs: None,
                track_count: 0,
            });
        }
    });
    settings_signal().update(|s| {
        if !s.local_music_dirs.contains(&path_str) {
            s.local_music_dirs.push(path_str.clone());
        }
    });
    persist_settings_now(true);
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::AddLocalFolder(path));
        set_status_banner(format!("Scanning local music folder: {path_str}"));
    } else {
        set_status_banner(format!("Added local music folder: {path_str}"));
    }
}

pub fn remove_local_media_folder(root_path: String) {
    local_media_signal().update(|m| {
        m.roots.retain(|r| r.path != root_path);
        m.tracks.retain(|t| t.root_path != root_path);
    });
    settings_signal().update(|s| {
        s.local_music_dirs.retain(|d| d != &root_path);
    });
    persist_settings_now(true);
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::RemoveLocalFolder(root_path.clone()));
    }
    set_status_banner(format!("Removed local music folder: {root_path}"));
}

pub fn rescan_local_media() {
    let root_count = local_media_signal().with(|m| m.roots.len());
    if root_count == 0 {
        set_status_banner("Add a local music folder before rescanning");
        return;
    }
    if !is_fixture_mode() {
        local_media_signal().update(|m| m.scan_progress.scanning = true);
        live_engine::dispatch(EngineCommand::RescanLocalMedia);
        set_status_banner(format!("Rescanning {root_count} local music folder(s)…"));
        return;
    }
    let now = resonance_core::unix_secs();
    local_media_signal().update(|m| {
        for root in &mut m.roots {
            root.last_scanned_unix_secs = Some(now);
        }
        m.scan_progress.scanning = false;
    });
    set_status_banner(format!("Rescanned {root_count} local music folder(s)"));
}

pub fn play_local_track_queue(track: Track, queue: Vec<Track>) {
    if queue.len() > 1 {
        let start_index = queue.iter().position(|t| t.id == track.id).unwrap_or(0);
        play_queue(queue, start_index);
    } else {
        play_track(track);
    }
}

pub fn updater_state() -> UpdaterSlice {
    updater_signal().with(|u| u.clone())
}

pub fn updater_channel() -> UpdateChannel {
    updater_signal().with(|u| u.channel)
}

pub fn set_updater_channel(channel: UpdateChannel) {
    updater_signal().update(|u| u.set_channel(channel));
    settings_signal().update(|s| {
        s.update_channel = match channel {
            UpdateChannel::Stable => "stable".to_owned(),
            UpdateChannel::Beta => "beta".to_owned(),
        };
    });
    persist_settings_now(true);
}

pub fn updater_auto_check() -> bool {
    updater_signal().with(|u| u.auto_check)
}

pub fn set_updater_auto_check(enabled: bool) {
    updater_signal().update(|u| u.set_auto_check(enabled));
}

pub fn updater_phase() -> UpdatePhase {
    updater_signal().with(|u| u.phase.clone())
}

pub fn updater_available() -> Option<UpdateInfo> {
    updater_signal().with(|u| u.should_show_top_banner().cloned())
}

pub fn dismiss_updater_banner() {
    updater_signal().update(|u| u.banner_dismissed = true);
}

pub fn skip_updater_version(version: String) {
    updater_signal().update(|u| {
        u.skipped_version = Some(version);
        u.banner_dismissed = true;
        u.phase = UpdatePhase::Idle;
    });
}

pub fn check_for_updates_now() {
    let mut started = false;
    let mut channel = UpdateChannel::Stable;
    let mut skipped_version = None;
    updater_signal().update(|u| {
        if u.begin_check() {
            started = true;
            channel = u.channel;
            skipped_version = u.skipped_version.clone();
        }
    });
    if !started {
        return;
    }
    if is_fixture_mode() {
        let now = resonance_core::unix_secs();
        updater_signal().update(|u| u.mark_up_to_date(now));
        set_status_banner("You are on the latest version (fixture preview)");
        return;
    }
    live_engine::dispatch(EngineCommand::CheckForUpdates {
        channel,
        skipped_version,
    });
    set_status_banner("Checking signed release feed for updates…");
}

pub fn download_update(info: UpdateInfo) {
    if is_fixture_mode() {
        updater_signal().update(|u| u.mark_ready_to_install(false));
        set_status_banner(format!("Staged preview update v{}", info.version));
        return;
    }
    let version = info.version.clone();
    updater_signal().update(|u| {
        u.phase = UpdatePhase::Downloading {
            info: info.clone(),
            received_bytes: 0,
            total_bytes: None,
        };
    });
    live_engine::dispatch(EngineCommand::DownloadUpdate(info));
    set_status_banner(format!("Downloading & verifying v{version}…"));
}

pub fn cancel_update_download() {
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::CancelUpdateDownload);
    }
    updater_signal().update(|u| u.phase = UpdatePhase::Idle);
    set_status_banner("Cancelled update download");
}

pub fn open_release_url(url: String) {
    copy_text_to_clipboard("release URL", &url);
}

pub fn install_update_on_quit() {
    updater_signal().update(|u| u.mark_ready_to_install(true));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::InstallUpdateOnQuit);
    } else {
        set_status_banner("Scheduled update install on app quit");
    }
}

pub fn restart_to_install_update() {
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::RestartToInstallUpdate);
    } else {
        set_status_banner("Restarting to apply update (fixture preview)");
    }
}

pub fn cast_sheet_open() -> bool {
    cast_signal().with(|c| c.sheet_open)
}

pub fn set_cast_sheet_open(open: bool) {
    cast_signal().update(|c| c.sheet_open = open);
    if !is_fixture_mode() {
        if open {
            live_engine::dispatch(EngineCommand::StartCastDiscovery);
        } else {
            live_engine::dispatch(EngineCommand::StopCastDiscovery);
        }
    }
}

pub fn cast_devices() -> Vec<CastDevice> {
    cast_signal().with(|c| c.devices.clone())
}

pub fn cast_session() -> CastSessionState {
    cast_signal().with(|c| c.session.clone())
}

pub fn select_cast_device(device: CastDevice) {
    let name = device.name.clone();
    if is_fixture_mode() {
        cast_signal().update(|c| {
            c.session = CastSessionState::Connected {
                device: device.clone(),
                playing: true,
                position_ms: player_snapshot().position_ms,
                duration_ms: player_snapshot().duration_ms,
                volume_pct: 100,
                muted: false,
            };
        });
        set_status_banner(format!("Casting to {name} (fixture preview)"));
        return;
    }
    cast_signal().update(|c| {
        c.session = CastSessionState::Connecting {
            device_name: name.clone(),
        };
    });
    live_engine::dispatch(EngineCommand::ConnectCastDevice(device));
    set_status_banner(format!("Connecting to {name}…"));
}

pub fn disconnect_cast() {
    cast_signal().update(|c| c.session = CastSessionState::Disconnected);
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::DisconnectCast);
    }
    set_status_banner("Disconnected Cast session");
}

pub fn set_cast_volume(volume: f32) {
    let clamped = volume.clamp(0.0, 1.0);
    let pct = (clamped * 100.0).round() as u8;
    cast_signal().update(|c| {
        if let CastSessionState::Connected { volume_pct, .. } = &mut c.session {
            *volume_pct = pct;
        }
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetCastVolume(clamped));
    }
}

#[allow(dead_code)]
pub fn shuffle_radio_mix() {
    set_autoplay_radio(true);
    let tracks = all_library_songs();
    if !tracks.is_empty() {
        play_all_shuffled(tracks);
    }
}
