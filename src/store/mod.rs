//! Domain-scoped reactive NSH store decomposed into independent `global::<Slice>()`
//! signals so high-frequency playback ticks (250ms) never invalidate unrelated screens.
//!
//! Reactive signal topology:
//! - `global::<PlaybackSlice>()` — player state, queue, lyrics, shortcuts, native audio
//! - `global::<CatalogSlice>()` — home shelves, explore, moods, search, collections, entity nav
//! - `global::<LibrarySlice>()` — liked tracks, playlists, history, cache stats, offline vault
//! - `global::<MutationSlice>()` — optimistic mutation state & rollback
//! - `global::<SettingsSlice>()` — theme, auth & TV OAuth sheet, DSP, remote config, PO-token
//! - `global::<UiSlice>()` — tab navigation history, sidebars, action sheets, status toast
//! - `global::<RoomsSlice>()` — Listen Together room state
//! - `global::<LocalMediaSlice>()` — local folder roots, tracks, scan progress
//! - `global::<UpdaterSlice>()` — release channel & update lifecycle
//! - `global::<CastSlice>()` — Cast picker sheet, discovered devices, active cast session

mod app_actions;
mod catalog_actions;
pub mod catalog_slice;
pub mod library_slice;
pub mod local_media_slice;
pub mod mutation_slice;
mod playback_actions;
pub mod playback_slice;
pub mod rooms_slice;
pub mod settings_slice;
pub mod ui_slice;
pub mod updater_slice;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use nsh_core::prelude::*;
use resonance_contract::{CastDevice, CastSessionState, LocalFolderRoot, UpdateChannel};

use crate::live_engine::{self, EngineCommand, LiveUpdate};

pub use app_actions::*;
pub use catalog_actions::*;
pub use catalog_slice::{CatalogSlice, EntityRoute, SearchFilter, SidebarPlaylist};
pub use library_slice::{LibraryFilter, LibrarySlice};
pub use local_media_slice::{
    LocalAlbumGroup, LocalArtistGroup, LocalLibrarySubTab, LocalMediaSlice,
};
pub use mutation_slice::MutationSlice;
pub use playback_actions::*;
pub use playback_slice::{AudioDriverSnapshot, PlaybackSlice, ShortcutAction, map_shortcut_key};
pub use rooms_slice::{ListenTogetherSubMode, RoomsSlice};
pub use settings_slice::{AudioDspCapability, SettingsSlice, ThemeChoice};
pub use ui_slice::{AppTab, DesktopSidePanel, UiSlice};
pub use updater_slice::UpdaterSlice;

/// Isolated Cast discovery and active-session state slice.
#[derive(Debug, Clone, Default)]
pub struct CastSlice {
    pub sheet_open: bool,
    pub devices: Vec<CastDevice>,
    pub session: CastSessionState,
}

/// Creates a component-scoped `Signal<T>` that stays reactively synced to a store projection.
pub fn synced_signal<T: Clone + PartialEq + 'static>(read: impl Fn() -> T + 'static) -> Signal<T> {
    let sig = signal(read());
    effect(move || {
        let next = read();
        if sig.get_untracked() != next {
            sig.set(next);
        }
    });
    sig
}

static FIXTURE_MODE_FLAG: AtomicBool = AtomicBool::new(false);
static PROXY_BASE_URL: OnceLock<RwLock<String>> = OnceLock::new();

fn proxy_base_lock() -> &'static RwLock<String> {
    PROXY_BASE_URL.get_or_init(|| RwLock::new("http://127.0.0.1:0".to_owned()))
}

pub fn set_proxy_base_url(base_url: impl Into<String>) {
    if let Ok(mut guard) = proxy_base_lock().write() {
        *guard = base_url.into().trim_end_matches('/').to_owned();
    }
}

pub fn proxy_stream_url(video_id: &str) -> String {
    let base = proxy_base_lock()
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "http://127.0.0.1:0".to_owned());
    let itag = if is_fixture_mode() {
        251
    } else {
        #[cfg(target_os = "android")]
        {
            251
        }
        #[cfg(not(target_os = "android"))]
        {
            140
        }
    };
    format!("{base}/stream/{video_id}?itag={itag}")
}

/// Normalizes a remote artwork URL and, when the live `LoopbackProxy` is running,
/// routes it through `/artwork?url=...` for L1/L2 caching, connection pooling, and retries.
pub fn resolve_artwork_url(raw: Option<String>) -> Option<String> {
    let url = raw?;
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = if let Some(rest) = trimmed.strip_prefix("//") {
        format!("https://{rest}")
    } else if let Some(rest) = trimmed.strip_prefix("http://")
        && !rest.starts_with("127.0.0.1")
        && !rest.starts_with("localhost")
    {
        format!("https://{rest}")
    } else {
        trimmed.to_owned()
    };

    if is_fixture_mode() || !normalized.starts_with("http") {
        return Some(normalized);
    }
    if normalized.starts_with("http://127.0.0.1") || normalized.starts_with("http://localhost") {
        return Some(normalized);
    }

    let base = proxy_base_lock()
        .read()
        .map(|g| g.clone())
        .unwrap_or_default();
    if base.starts_with("http://127.0.0.1:") && !base.ends_with(":0") {
        let mut encoded = String::with_capacity(normalized.len() * 2);
        for b in normalized.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    encoded.push(*b as char);
                }
                _ => {
                    use std::fmt::Write;
                    let _ = write!(encoded, "%{b:02X}");
                }
            }
        }
        return Some(format!("{base}/artwork?url={encoded}"));
    }
    Some(normalized)
}

pub fn enable_fixture_mode(enabled: bool) {
    FIXTURE_MODE_FLAG.store(enabled, Ordering::SeqCst);
}

pub fn is_fixture_mode() -> bool {
    FIXTURE_MODE_FLAG.load(Ordering::SeqCst)
}

// ── Independent Domain-Scoped Reactive Signals ───────────────────────────────

pub(crate) fn ui_signal() -> Signal<UiSlice> {
    global::<UiSlice>()
}

pub(crate) fn playback_signal() -> Signal<PlaybackSlice> {
    global::<PlaybackSlice>()
}

pub(crate) fn catalog_signal() -> Signal<CatalogSlice> {
    global::<CatalogSlice>()
}

pub(crate) fn library_signal() -> Signal<LibrarySlice> {
    global::<LibrarySlice>()
}

pub(crate) fn mutations_signal() -> Signal<MutationSlice> {
    global::<MutationSlice>()
}

pub(crate) fn settings_signal() -> Signal<SettingsSlice> {
    global::<SettingsSlice>()
}

pub(crate) fn rooms_signal() -> Signal<RoomsSlice> {
    global::<RoomsSlice>()
}

pub(crate) fn local_media_signal() -> Signal<LocalMediaSlice> {
    global::<LocalMediaSlice>()
}

pub(crate) fn updater_signal() -> Signal<UpdaterSlice> {
    global::<UpdaterSlice>()
}

pub(crate) fn cast_signal() -> Signal<CastSlice> {
    global::<CastSlice>()
}

pub(crate) fn persist_settings_now(immediate: bool) {
    if is_fixture_mode() {
        return;
    }
    let playback_snap = playback_signal().with(|p| p.clone());
    let persisted = settings_signal().with(|s| s.to_persisted_settings(&playback_snap));
    live_engine::dispatch(EngineCommand::PersistSettings {
        settings: persisted,
        immediate,
    });
}

pub(crate) fn apply_nsh_appearance(choice: ThemeChoice) {
    let appearance = match choice {
        ThemeChoice::System => Appearance::System,
        ThemeChoice::Light => Appearance::Light,
        ThemeChoice::Dark => Appearance::Dark,
    };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        set_color_scheme(appearance);
    }));
}

/// Mounts the non-blocking background live-engine update pump when NOT running in `--fixture` mode.
pub fn mount_live_engine_bridge() {
    if is_fixture_mode() {
        return;
    }
    live_engine::dispatch(EngineCommand::LoadHome);
    live_engine::dispatch(EngineCommand::LoadExplore);
    spawn(async {
        loop {
            for update in live_engine::try_drain_updates() {
                apply_live_update(update);
            }
            timer(Duration::from_millis(80)).await;
        }
    });
}

fn apply_live_update(update: LiveUpdate) {
    match update {
        LiveUpdate::Ready {
            proxy_base_url,
            snapshot,
            cache_stats,
            signed_in,
        } => {
            set_proxy_base_url(proxy_base_url);
            playback_signal().update(|p| {
                if p.player.current_track.is_none() && snapshot.current_track.is_some() {
                    p.player = snapshot;
                }
            });
            library_signal().update(|l| l.cache_stats = cache_stats);
            settings_signal().update(|s| {
                s.signed_in = signed_in;
                if signed_in && s.account_name.is_none() {
                    s.account_name = Some("Google Account (SAPISIDHASH)".into());
                }
            });
        }
        LiveUpdate::HomeLoaded(res) => {
            catalog_signal().update(|c| c.apply_home_loaded(res));
        }
        LiveUpdate::ExploreLoaded(res) => {
            catalog_signal().update(|c| c.apply_explore_loaded(res));
        }
        LiveUpdate::ExplorePageLoaded(res) => {
            catalog_signal().update(|c| c.apply_explore_page(res));
        }
        LiveUpdate::MoodPlaylistsLoaded { mood, result } => {
            catalog_signal().update(|c| c.apply_mood_playlists(Some(mood), result));
        }
        LiveUpdate::SearchLoaded {
            query,
            result,
            plays,
        } => {
            catalog_signal().update(|c| c.apply_search_loaded(&query, result, plays));
        }
        LiveUpdate::SearchSuggestionsLoaded {
            prefix,
            suggestions,
        } => {
            catalog_signal().update(|c| c.apply_search_suggestions(&prefix, suggestions));
        }
        LiveUpdate::SearchContinuationLoaded { query, result } => {
            catalog_signal().update(|c| c.append_search_page(&query, result));
        }
        LiveUpdate::CollectionLoaded(res) => match res {
            Ok(playlist) => catalog_signal().update(|c| c.selected_collection = Some(playlist)),
            Err(err) => {
                ui_signal().update(|u| u.set_status_banner(format!("Collection load failed: {err}")));
            }
        },
        LiveUpdate::PlaybackUpdated(mut snapshot) => {
            // `PlaybackController` is authoritative in live mode; only preserve UI-owned offline_mode.
            playback_signal().update(|p| {
                snapshot.offline_mode = p.player.offline_mode;
                p.player = snapshot;
            });
        }
        LiveUpdate::LyricsLoaded(doc) => {
            playback_signal().update(|p| p.lyrics = doc);
        }
        LiveUpdate::CacheStatsUpdated(stats) => {
            library_signal().update(|l| l.cache_stats = stats);
        }
        LiveUpdate::AuthUpdated { signed_in, banner } => {
            settings_signal().update(|s| s.apply_auth_updated(signed_in));
            ui_signal().update(|u| u.set_status_banner(banner));
        }
        LiveUpdate::AuthStatus(status) => {
            settings_signal().update(|s| s.apply_auth_status(status));
        }
        LiveUpdate::AccountProfileUpdated(profile) => {
            settings_signal().update(|s| s.apply_account_profile(profile));
        }
        LiveUpdate::RemoteConfigUpdated(status) => {
            settings_signal().update(|s| s.apply_remote_config_status(status));
        }
        LiveUpdate::PoTokenDiagnostics(diag) => {
            settings_signal().update(|s| s.po_token_diagnostics = diag);
        }
        LiveUpdate::MutationFailed { command, reason } => {
            mutations_signal().update(|m| {
                playback_signal().update(|p| {
                    library_signal().update(|l| {
                        catalog_signal().update(|c| {
                            m.rollback_mutation(command, p, l, c);
                        });
                    });
                });
            });
            ui_signal().update(|u| u.set_status_banner(format!("Action failed: {reason}")));
        }
        LiveUpdate::DownloadsLoaded(records) => {
            library_signal().update(|l| l.set_downloads(records));
        }
        LiveUpdate::DownloadUpdated(record) => {
            library_signal().update(|l| {
                l.downloads.insert(record.video_id.clone(), record);
            });
        }
        LiveUpdate::DownloadRemoved(video_id) => {
            library_signal().update(|l| {
                l.downloads.remove(&video_id);
            });
        }
        LiveUpdate::PinnedCollectionsLoaded(list) => {
            library_signal().update(|l| l.pinned_collections = list);
        }
        LiveUpdate::LibraryLoaded(snapshot) => {
            playback_signal().update(|p| {
                library_signal().update(|l| {
                    l.apply_snapshot(snapshot, p.player.current_track.as_mut());
                });
            });
        }
        LiveUpdate::SettingsLoaded(settings) => {
            let mut loaded_theme = ThemeChoice::System;
            let mut loaded_volume = 1.0_f32;
            settings_signal().update(|s| {
                playback_signal().update(|p| {
                    s.apply_persisted_settings(&settings, p);
                    loaded_volume = p.player.volume;
                });
                loaded_theme = s.theme_choice;
            });
            let channel = if settings.update_channel.eq_ignore_ascii_case("beta") {
                UpdateChannel::Beta
            } else {
                UpdateChannel::Stable
            };
            updater_signal().update(|u| u.set_channel(channel));
            if !settings.local_music_dirs.is_empty() {
                local_media_signal().update(|m| {
                    for dir in &settings.local_music_dirs {
                        if !dir.trim().is_empty() && !m.roots.iter().any(|r| &r.path == dir) {
                            m.roots.push(LocalFolderRoot {
                                path: dir.clone(),
                                added_at_unix_secs: 0,
                                last_scanned_unix_secs: None,
                                track_count: 0,
                            });
                        }
                    }
                });
            }
            apply_nsh_appearance(loaded_theme);
            if !is_fixture_mode() {
                live_engine::dispatch(EngineCommand::SetVolume(loaded_volume));
            }
        }
        LiveUpdate::StatusBanner(msg) => {
            ui_signal().update(|u| u.set_status_banner(msg));
        }
    }
}
