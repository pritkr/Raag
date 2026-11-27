//! Playback, Queue, Shortcuts & Native Audio actions operating on `playback_signal()`.

use nsh_core::prelude::SharedString;
use resonance_contract::{
    AudioTelemetry, LyricsDocument, PlayerSnapshot, RepeatMode, Track,
};
use resonance_core::palette::{AmbientPalette, palette_for_track};

use crate::live_engine::{self, EngineCommand};

use super::playback_slice::{AudioDriverSnapshot, ShortcutAction, map_shortcut_key};
use super::{
    is_fixture_mode, library_signal, persist_settings_now, playback_signal, proxy_stream_url,
    toggle_like_current, ui_signal,
};

/// Returns the cached artwork-driven ambient palette for the currently playing track.
pub fn active_ambient_palette() -> AmbientPalette {
    playback_signal().with(|p| match p.player.current_track.as_ref() {
        Some(t) => palette_for_track(&t.id, &t.title),
        None => AmbientPalette::default(),
    })
}

pub fn player_snapshot() -> PlayerSnapshot {
    playback_signal().with(|p| p.player.clone())
}

pub fn queue_len() -> usize {
    playback_signal().with(|p| p.player.queue.items.len())
}

pub fn queue_track_at(index: usize) -> Option<Track> {
    playback_signal().with(|p| p.player.queue.items.get(index).cloned())
}

pub fn queue_ids() -> Vec<SharedString> {
    playback_signal().with(|p| p.queue_ids())
}

pub fn current_lyrics() -> Option<LyricsDocument> {
    playback_signal().with(|p| p.lyrics.clone())
}

pub fn related_tracks() -> Vec<Track> {
    playback_signal().with(|p| p.related_tracks.clone())
}

pub fn set_related_tracks(video_id: String, tracks: Vec<Track>) {
    playback_signal().update(|p| {
        if p.player
            .current_track
            .as_ref()
            .is_some_and(|t| t.id == video_id)
            || p.related_seed_id.as_deref() == Some(video_id.as_str())
        {
            p.related_seed_id = Some(video_id);
            p.related_tracks = tracks;
        }
    });
}

pub fn play_track(track: Track) {
    let live = !is_fixture_mode();
    let track_for_live = track.clone();
    let url = proxy_stream_url(&track.id);
    playback_signal().update(|p| {
        p.play_track(track.clone(), live, url);
        p.related_seed_id = Some(track.id.clone());
        if !live {
            p.related_tracks = resonance_core::fixture::fixture_tracks()
                .into_iter()
                .filter(|t| t.id != track.id)
                .collect();
        }
    });
    library_signal().update(|l| l.record_play(track));
    if live {
        live_engine::dispatch(EngineCommand::PlayTrack(track_for_live));
    }
}

/// Atomically replaces the active queue with `tracks` and begins playback at `start_index`.
pub fn play_queue(tracks: Vec<Track>, start_index: usize) {
    if tracks.is_empty() {
        return;
    }
    let live = !is_fixture_mode();
    let clamped = start_index.min(tracks.len().saturating_sub(1));
    let tracks_for_live = tracks.clone();
    let mut started = None;
    playback_signal().update(|p| {
        started = p.play_queue(tracks, clamped, live, proxy_stream_url);
        if let Some(ref active) = started {
            p.related_seed_id = Some(active.id.clone());
            if !live {
                p.related_tracks = resonance_core::fixture::fixture_tracks()
                    .into_iter()
                    .filter(|t| t.id != active.id)
                    .collect();
            }
        }
    });
    if let Some(ref t) = started {
        library_signal().update(|l| l.record_play(t.clone()));
    }
    if live && started.is_some() {
        live_engine::dispatch(EngineCommand::PlayQueue {
            tracks: tracks_for_live,
            start_index: clamped,
            shuffle: false,
        });
    }
}

pub fn play_all_shuffled(tracks: Vec<Track>) {
    if tracks.is_empty() {
        return;
    }
    let live = !is_fixture_mode();
    let tracks_for_live = tracks.clone();
    let mut started = None;
    let mut start_index = 0usize;
    playback_signal().update(|p| {
        started = p.play_all_shuffled(tracks, live, proxy_stream_url);
        start_index = p.player.queue.current_index.unwrap_or(0);
    });
    if let Some(ref t) = started {
        library_signal().update(|l| l.record_play(t.clone()));
    }
    if live && started.is_some() {
        live_engine::dispatch(EngineCommand::PlayQueue {
            tracks: tracks_for_live,
            start_index,
            shuffle: true,
        });
    }
}

pub fn play_next(track: Track) {
    let track_for_live = track.clone();
    let mut banner = String::new();
    playback_signal().update(|p| {
        banner = p.play_next(track);
    });
    ui_signal().update(|u| u.set_status_banner(banner));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::PlayNext(track_for_live));
    }
}

pub fn play_last(track: Track) {
    let track_for_live = track.clone();
    let mut banner = String::new();
    playback_signal().update(|p| {
        banner = p.play_last(track);
    });
    ui_signal().update(|u| u.set_status_banner(banner));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::PlayLast(track_for_live));
    }
}

pub fn toggle_play_pause() {
    let mut has_track = false;
    playback_signal().update(|p| {
        has_track = p.player.current_track.is_some();
        p.toggle_play_pause();
    });
    if !is_fixture_mode() && has_track {
        live_engine::dispatch(EngineCommand::TogglePlayPause);
    }
}

pub fn seek_ms(target_ms: u64) {
    let mut clamped = target_ms;
    let mut has_track = false;
    playback_signal().update(|p| {
        has_track = p.player.current_track.is_some();
        p.seek_ms(target_ms);
        clamped = p.player.position_ms;
    });
    if !is_fixture_mode() && has_track {
        live_engine::dispatch(EngineCommand::SeekMs(clamped));
    }
}

pub fn next_track() {
    let live = !is_fixture_mode();
    playback_signal().update(|p| {
        let _ = p.next_track(live, proxy_stream_url);
    });
    if live {
        live_engine::dispatch(EngineCommand::NextTrack);
    }
}

pub fn previous_track() {
    let live = !is_fixture_mode();
    let mut moved = None;
    let mut sought_to_start = false;
    playback_signal().update(|p| {
        let was_past_threshold = p.player.position_ms > 3_000;
        moved = p.previous_track(live, proxy_stream_url);
        sought_to_start =
            moved.is_none() && was_past_threshold && p.player.current_track.is_some();
    });
    if live {
        if moved.is_some() {
            live_engine::dispatch(EngineCommand::PreviousTrack);
        } else if sought_to_start {
            live_engine::dispatch(EngineCommand::SeekMs(0));
        }
    }
}

pub fn jump_queue(index: usize) {
    let live = !is_fixture_mode();
    let mut jumped = None;
    playback_signal().update(|p| {
        jumped = p.jump_queue(index, live, proxy_stream_url);
    });
    if live && jumped.is_some() {
        live_engine::dispatch(EngineCommand::JumpQueue(index));
    }
}

pub fn move_queue_item(from: usize, to: usize) {
    playback_signal().update(|p| p.move_queue_item(from, to));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::MoveQueueItem { from, to });
    }
}

pub fn remove_queue_item(index: usize) {
    playback_signal().update(|p| p.remove_queue_item(index, proxy_stream_url));
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::RemoveQueueItem(index));
    }
}

pub fn clear_up_next() {
    let mut removed = 0usize;
    playback_signal().update(|p| {
        removed = p.clear_up_next();
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::ClearUpNext);
    }
    ui_signal().update(|u| {
        u.set_status_banner(if removed == 0 {
            "Up Next is already empty".to_owned()
        } else {
            format!("Cleared {removed} upcoming track(s)")
        });
    });
}

pub fn clear_queue() {
    playback_signal().update(|p| p.clear_queue());
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::ClearQueue);
    }
}

pub fn toggle_shuffle() {
    let mut enabled = false;
    playback_signal().update(|p| {
        p.toggle_shuffle();
        enabled = p.player.queue.shuffle;
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetShuffle(enabled));
    }
}

pub fn cycle_repeat() {
    let mut mode = RepeatMode::None;
    playback_signal().update(|p| {
        p.cycle_repeat();
        mode = p.player.queue.repeat;
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetRepeatMode(mode));
    }
}

pub fn set_autoplay_radio(enabled: bool) {
    playback_signal().update(|p| p.player.queue.autoplay_radio = enabled);
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetAutoplayRadio(enabled));
    }
}

pub fn set_sponsorblock_enabled(enabled: bool) {
    playback_signal().update(|p| p.player.sponsorblock_enabled = enabled);
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetSponsorBlock(enabled));
    }
    persist_settings_now(true);
}

pub fn set_offline_mode(enabled: bool) {
    playback_signal().update(|p| p.player.offline_mode = enabled);
    ui_signal().update(|u| {
        u.set_status_banner(if enabled {
            "Offline mode enabled — playing from local 256 KB chunk cache"
        } else {
            "Online mode restored — streaming via Loopback Proxy"
        });
    });
    persist_settings_now(true);
}

pub fn set_volume(volume: f32) {
    let mut applied = volume;
    playback_signal().update(|p| {
        p.set_volume(volume);
        applied = p.player.volume;
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetVolume(applied));
    }
    persist_settings_now(false);
}

pub fn adjust_volume(delta: f32) {
    let mut applied = 1.0_f32;
    playback_signal().update(|p| {
        p.adjust_volume(delta);
        applied = p.player.volume;
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetVolume(applied));
    }
    persist_settings_now(false);
}

pub fn toggle_mute() {
    let mut applied = 1.0_f32;
    playback_signal().update(|p| {
        p.toggle_mute();
        applied = p.player.volume;
    });
    if !is_fixture_mode() {
        live_engine::dispatch(EngineCommand::SetVolume(applied));
    }
}

pub fn total_queue_duration_ms() -> u64 {
    playback_signal().with(|p| p.total_queue_duration_ms())
}

/// Handles desktop keyboard media shortcuts on the NSH engine thread without blocking the UI.
pub fn handle_shortcut(key: &str) -> bool {
    let Some(action) = map_shortcut_key(key) else {
        return false;
    };
    match action {
        ShortcutAction::TogglePlayPause => toggle_play_pause(),
        ShortcutAction::SeekForward10s => {
            let pos = player_snapshot().position_ms;
            seek_ms(pos.saturating_add(10_000));
        }
        ShortcutAction::SeekBackward10s => {
            let pos = player_snapshot().position_ms;
            seek_ms(pos.saturating_sub(10_000));
        }
        ShortcutAction::NextTrack => next_track(),
        ShortcutAction::PreviousTrack => previous_track(),
        ShortcutAction::ToggleLike => toggle_like_current(),
        ShortcutAction::ToggleMute => toggle_mute(),
        ShortcutAction::ToggleShuffle => toggle_shuffle(),
        ShortcutAction::CycleRepeat => cycle_repeat(),
    }
    true
}

pub fn audio_driver_snapshot() -> AudioDriverSnapshot {
    playback_signal().with(|p| p.audio_driver_snapshot())
}

pub fn set_nsh_audio_available(available: bool) {
    playback_signal().update(|p| p.nsh_audio_available = available);
}

pub fn nsh_audio_available() -> bool {
    playback_signal().with(|p| p.nsh_audio_available)
}

/// Applies native `nsh/audio` telemetry directly to `PlaybackSlice` in headless fixture mode
/// (where no background `AppEngine` Tokio runtime is running).
pub fn apply_fixture_audio_telemetry(event: AudioTelemetry) {
    match event {
        AudioTelemetry::Loaded { duration_ms, .. } => {
            playback_signal().update(|p| p.on_native_audio_duration(duration_ms));
        }
        AudioTelemetry::StateChanged { state, .. } => {
            playback_signal().update(|p| p.on_native_audio_state(state));
        }
        AudioTelemetry::Buffering { buffering, .. } => {
            playback_signal().update(|p| p.on_native_audio_buffering(buffering));
        }
        AudioTelemetry::Progress {
            position_ms,
            duration_ms,
            ..
        } => {
            playback_signal().update(|p| p.on_native_audio_progress(position_ms, duration_ms));
        }
        AudioTelemetry::Ended { .. } => {
            let mut end_of_track_restore = None;
            ui_signal().update(|u| {
                if let super::ui_slice::SleepTimerTickAction::PauseAndRestoreVolume(saved) =
                    u.sleep_timer.on_track_ended()
                {
                    end_of_track_restore = Some(saved);
                }
            });
            if let Some(saved) = end_of_track_restore {
                playback_signal().update(|p| {
                    p.player.state = resonance_contract::PlayerState::Paused;
                    p.set_volume(saved);
                });
                ui_signal().update(|u| u.set_status_banner("Sleep timer paused at end of track"));
            } else {
                playback_signal().update(|p| {
                    let _ = p.on_native_audio_ended(false, proxy_stream_url);
                });
            }
        }
        AudioTelemetry::Failed { message, .. } => {
            ui_signal().update(|u| u.set_status_banner(message));
        }
    }
}
