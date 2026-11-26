//! UI Navigation, Settings, Auth, DSP, Rooms, Local Media, Updater & Cast actions
//! operating on their respective isolated domain signals.

use resonance_contract::{
    AccountProfile, AuthBackendStatus, CastDevice, CastSessionState, LocalFolderRoot,
    LocalScanProgress, LocalTrackRecord, PoTokenDiagnostics, RemoteConfigStatus, Track,
    UpdateChannel, UpdateInfo, UpdatePhase,
};
use resonance_core::audio_dsp::DspConfig;

use crate::live_engine::{self, EngineCommand};

use super::rooms_slice::RoomsSlice;
use super::settings_slice::{AudioDspCapability, SettingsSlice, ThemeChoice};
use super::ui_slice::{AppTab, DesktopSidePanel};
use super::updater_slice::UpdaterSlice;
use super::{
    apply_nsh_appearance, cast_signal, catalog_signal, is_fixture_mode, local_media_signal,
    persist_settings_now, rooms_signal, settings_signal, ui_signal, updater_signal,
};

// ── UI & Navigation ──────────────────────────────────────────────────────────

pub fn active_tab() -> AppTab {
    ui_signal().with(|u| u.active_tab)
}

pub fn set_tab(tab: AppTab) {
    ui_signal().update(|u| u.set_tab(tab));
}

pub fn can_go_back() -> bool {
    let has_col = catalog_signal().with(|c| c.selected_collection.is_some());
    ui_signal().with(|u| u.can_go_back(has_col))
}

pub fn can_go_forward() -> bool {
    ui_signal().with(|u| u.can_go_forward())
}

pub fn go_back() {
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

pub fn set_loudness_normalization(enabled: bool) {
    settings_signal().update(|s| s.set_loudness_normalization(enabled));
    ui_signal().update(|u| {
        u.set_status_banner(if enabled {
            "ReplayGain / EBU R128 loudness normalization enabled"
        } else {
            "ReplayGain / EBU R128 loudness normalization disabled"
        });
    });
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
    persist_settings_now(true);
}

pub fn cycle_eq_preset() {
    let mut label = "";
    settings_signal().update(|s| {
        label = s.cycle_eq_preset();
    });
    ui_signal().update(|u| u.set_status_banner(format!("10-band EQ preset: {label}")));
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
    persist_settings_now(true);
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

pub fn local_media_roots() -> Vec<LocalFolderRoot> {
    local_media_signal().with(|m| m.roots.clone())
}

pub fn local_media_tracks() -> Vec<LocalTrackRecord> {
    local_media_signal().with(|m| m.tracks.clone())
}

pub fn local_media_progress() -> LocalScanProgress {
    local_media_signal().with(|m| m.scan_progress.clone())
}

pub fn updater_state() -> UpdaterSlice {
    updater_signal().with(|u| u.clone())
}

pub fn updater_channel() -> UpdateChannel {
    updater_signal().with(|u| u.channel)
}

pub fn set_updater_channel(channel: UpdateChannel) {
    updater_signal().update(|u| u.channel = channel);
}

pub fn updater_auto_check() -> bool {
    updater_signal().with(|u| u.auto_check)
}

pub fn set_updater_auto_check(enabled: bool) {
    updater_signal().update(|u| u.auto_check = enabled);
}

pub fn updater_phase() -> UpdatePhase {
    updater_signal().with(|u| u.phase.clone())
}

pub fn updater_available() -> Option<UpdateInfo> {
    updater_signal().with(|u| match &u.phase {
        UpdatePhase::Available(info) if !u.banner_dismissed => Some(info.clone()),
        _ => None,
    })
}

pub fn dismiss_updater_banner() {
    updater_signal().update(|u| u.banner_dismissed = true);
}

pub fn skip_updater_version(version: String) {
    updater_signal().update(|u| {
        u.skipped_version = Some(version);
        u.phase = UpdatePhase::Idle;
    });
}

pub fn cast_sheet_open() -> bool {
    cast_signal().with(|c| c.sheet_open)
}

pub fn set_cast_sheet_open(open: bool) {
    cast_signal().update(|c| c.sheet_open = open);
}

pub fn cast_devices() -> Vec<CastDevice> {
    cast_signal().with(|c| c.devices.clone())
}

pub fn cast_session() -> CastSessionState {
    cast_signal().with(|c| c.session.clone())
}
