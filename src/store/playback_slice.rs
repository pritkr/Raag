//! Playback & Queue state slice (`PlaybackSlice`), keyboard shortcut mapping,
//! and native `nsh/audio` driver snapshot/event handlers.

use nsh_core::prelude::SharedString;
use resonance_contract::{LyricsDocument, PlayerSnapshot, PlayerState, QueueState, RepeatMode, Track};
use resonance_core::fixture::fixture_lyrics;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    TogglePlayPause,
    SeekForward10s,
    SeekBackward10s,
    NextTrack,
    PreviousTrack,
    ToggleLike,
    ToggleMute,
    ToggleShuffle,
    CycleRepeat,
}

/// Pure, thread-independent keyboard shortcut mapper.
pub fn map_shortcut_key(key: &str) -> Option<ShortcutAction> {
    match key {
        " " | "Space" => Some(ShortcutAction::TogglePlayPause),
        "ArrowRight" => Some(ShortcutAction::SeekForward10s),
        "ArrowLeft" => Some(ShortcutAction::SeekBackward10s),
        "n" | "N" => Some(ShortcutAction::NextTrack),
        "p" | "P" => Some(ShortcutAction::PreviousTrack),
        "l" | "L" => Some(ShortcutAction::ToggleLike),
        "m" | "M" => Some(ShortcutAction::ToggleMute),
        "s" | "S" => Some(ShortcutAction::ToggleShuffle),
        "r" | "R" => Some(ShortcutAction::CycleRepeat),
        _ => None,
    }
}

/// Snapshot observed by `platform::nsh_audio::mount_nsh_audio_driver()`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioDriverSnapshot {
    pub active_proxy_url: Option<String>,
    pub desired_playing: bool,
    pub position_ms: u64,
    pub seek_generation: u64,
    pub volume: f32,
    pub looped: bool,
}

#[derive(Clone)]
pub struct PlaybackSlice {
    pub player: PlayerSnapshot,
    pub lyrics: Option<LyricsDocument>,
    pub seek_generation: u64,
    pub nsh_audio_available: bool,
}

impl PlaybackSlice {
    pub fn new(fixture: bool, tracks: Vec<Track>, proxy_url_for: impl Fn(&str) -> String) -> Self {
        let current_track = tracks.first().cloned();
        let active_proxy_url = current_track.as_ref().map(|t| proxy_url_for(&t.id));
        let lyrics = if fixture {
            current_track.as_ref().map(|t| fixture_lyrics(&t.id))
        } else {
            None
        };
        let duration_ms = current_track.as_ref().map_or(0, |t| t.duration_ms);

        Self {
            player: PlayerSnapshot {
                state: if current_track.is_some() {
                    PlayerState::Playing
                } else {
                    PlayerState::Idle
                },
                current_track,
                position_ms: if fixture { 26_000 } else { 0 },
                duration_ms,
                buffered_ms: duration_ms,
                volume: 1.0,
                queue: QueueState {
                    items: tracks,
                    current_index: if fixture { Some(0) } else { None },
                    shuffle: false,
                    repeat: RepeatMode::None,
                    autoplay_radio: true,
                    resume_position_ms: if fixture { 26_000 } else { 0 },
                },
                active_proxy_url,
                sponsorblock_enabled: true,
                last_skipped_segment: None,
                offline_mode: false,
            },
            lyrics,
            seek_generation: 0,
            nsh_audio_available: false,
        }
    }

    pub fn queue_ids(&self) -> Vec<SharedString> {
        self.player
            .queue
            .items
            .iter()
            .enumerate()
            .map(|(i, t)| SharedString::from(format!("{}-{i}", t.id)))
            .collect()
    }

    pub fn play_track(&mut self, track: Track, live: bool, proxy_url: String) {
        let pos = self
            .player
            .queue
            .items
            .iter()
            .position(|t| t.id == track.id)
            .unwrap_or_else(|| {
                self.player.queue.items.insert(0, track.clone());
                0
            });
        self.player.queue.current_index = Some(pos);
        self.player.current_track = Some(track.clone());
        self.player.duration_ms = track.duration_ms;
        self.player.position_ms = 0;
        self.player.state = if live {
            PlayerState::Loading
        } else {
            PlayerState::Playing
        };
        self.player.active_proxy_url = Some(proxy_url);
        self.lyrics = if live {
            None
        } else {
            Some(fixture_lyrics(&track.id))
        };
    }

    pub fn play_next(&mut self, track: Track) -> String {
        let idx = self
            .player
            .queue
            .current_index
            .map(|i| (i + 1).min(self.player.queue.items.len()))
            .unwrap_or(0);
        let title = track.title.clone();
        self.player.queue.items.insert(idx, track);
        format!("Added \"{title}\" to Play Next")
    }

    pub fn play_last(&mut self, track: Track) -> String {
        let title = track.title.clone();
        self.player.queue.items.push(track);
        format!("Appended \"{title}\" to end of queue")
    }

    pub fn toggle_play_pause(&mut self) {
        if self.player.current_track.is_none() {
            return;
        }
        self.player.state = if self.player.state.is_playing() {
            PlayerState::Paused
        } else {
            PlayerState::Playing
        };
    }

    pub fn seek_ms(&mut self, target_ms: u64) {
        let clamped = if self.player.duration_ms > 0 {
            target_ms.min(self.player.duration_ms)
        } else {
            target_ms
        };
        self.player.position_ms = clamped;
        self.player.queue.resume_position_ms = clamped;
        self.seek_generation = self.seek_generation.wrapping_add(1);
    }

    pub fn next_track(
        &mut self,
        live: bool,
        proxy_url_for: impl Fn(&str) -> String,
    ) -> Option<Track> {
        let len = self.player.queue.items.len();
        if len == 0 {
            return None;
        }
        let cur = self.player.queue.current_index.unwrap_or(0);
        let next = match self.player.queue.repeat {
            RepeatMode::One => cur,
            RepeatMode::All => (cur + 1) % len,
            RepeatMode::None => (cur + 1).min(len - 1),
        };
        self.activate_queue_index(next, live, proxy_url_for)
    }

    pub fn previous_track(
        &mut self,
        live: bool,
        proxy_url_for: impl Fn(&str) -> String,
    ) -> Option<Track> {
        if self.player.position_ms > 3_000 {
            self.player.position_ms = 0;
            self.player.queue.resume_position_ms = 0;
            self.seek_generation = self.seek_generation.wrapping_add(1);
            return None;
        }
        let len = self.player.queue.items.len();
        if len == 0 {
            return None;
        }
        let cur = self.player.queue.current_index.unwrap_or(0);
        let prev = cur.saturating_sub(1);
        self.activate_queue_index(prev, live, proxy_url_for)
    }

    pub fn jump_queue(
        &mut self,
        index: usize,
        live: bool,
        proxy_url_for: impl Fn(&str) -> String,
    ) -> Option<Track> {
        self.activate_queue_index(index, live, proxy_url_for)
    }

    fn activate_queue_index(
        &mut self,
        index: usize,
        live: bool,
        proxy_url_for: impl Fn(&str) -> String,
    ) -> Option<Track> {
        let t = self.player.queue.items.get(index).cloned()?;
        self.player.queue.current_index = Some(index);
        self.player.duration_ms = t.duration_ms;
        self.player.position_ms = 0;
        self.player.queue.resume_position_ms = 0;
        self.player.state = if live {
            PlayerState::Loading
        } else {
            PlayerState::Playing
        };
        self.player.active_proxy_url = Some(proxy_url_for(&t.id));
        self.lyrics = if live {
            None
        } else {
            Some(fixture_lyrics(&t.id))
        };
        self.player.current_track = Some(t.clone());
        Some(t)
    }

    pub fn move_queue_item(&mut self, from: usize, to: usize) {
        let len = self.player.queue.items.len();
        if from >= len || to >= len || from == to {
            return;
        }
        let item = self.player.queue.items.remove(from);
        self.player.queue.items.insert(to, item);
        if let Some(cur) = self.player.queue.current_index {
            self.player.queue.current_index = Some(if cur == from {
                to
            } else if from < cur && cur <= to {
                cur - 1
            } else if to <= cur && cur < from {
                cur + 1
            } else {
                cur
            });
        }
    }

    pub fn remove_queue_item(&mut self, index: usize, proxy_url_for: impl Fn(&str) -> String) {
        if index >= self.player.queue.items.len() {
            return;
        }
        self.player.queue.items.remove(index);
        if self.player.queue.items.is_empty() {
            self.player.queue.current_index = None;
            self.player.current_track = None;
            self.player.active_proxy_url = None;
            self.player.state = PlayerState::Idle;
        } else if let Some(cur) = self.player.queue.current_index {
            if index < cur {
                self.player.queue.current_index = Some(cur - 1);
            } else if index == cur {
                let next = cur.min(self.player.queue.items.len() - 1);
                self.player.queue.current_index = Some(next);
                let next_track = self.player.queue.items.get(next).cloned();
                self.player.active_proxy_url = next_track.as_ref().map(|t| proxy_url_for(&t.id));
                self.player.current_track = next_track;
            }
        }
    }

    pub fn clear_queue(&mut self) {
        self.player.queue.items.clear();
        self.player.queue.current_index = None;
        self.player.current_track = None;
        self.player.active_proxy_url = None;
        self.player.state = PlayerState::Idle;
    }

    pub fn toggle_shuffle(&mut self) {
        self.player.queue.shuffle = !self.player.queue.shuffle;
    }

    pub fn cycle_repeat(&mut self) {
        self.player.queue.repeat = self.player.queue.repeat.cycle();
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.player.volume = volume.clamp(0.0, 1.0);
    }

    pub fn adjust_volume(&mut self, delta: f32) {
        self.player.volume = (self.player.volume + delta).clamp(0.0, 1.0);
    }

    pub fn toggle_mute(&mut self) {
        self.player.volume = if self.player.volume > 0.01 { 0.0 } else { 1.0 };
    }

    pub fn total_queue_duration_ms(&self) -> u64 {
        self.player.queue.items.iter().map(|t| t.duration_ms).sum()
    }

    pub fn audio_driver_snapshot(&self) -> AudioDriverSnapshot {
        AudioDriverSnapshot {
            active_proxy_url: self.player.active_proxy_url.clone(),
            desired_playing: matches!(
                self.player.state,
                PlayerState::Playing | PlayerState::Buffering
            ),
            position_ms: self.player.position_ms,
            seek_generation: self.seek_generation,
            volume: self.player.volume,
            looped: self.player.queue.repeat == RepeatMode::One,
        }
    }

    pub fn on_native_audio_duration(&mut self, duration_ms: u64) {
        if duration_ms > 0 {
            self.player.duration_ms = duration_ms;
        }
    }

    pub fn on_native_audio_state(&mut self, state: PlayerState) {
        if self.player.current_track.is_some() {
            self.player.state = state;
        }
    }

    pub fn on_native_audio_buffering(&mut self, buffering: bool) {
        if self.player.current_track.is_none() {
            return;
        }
        if buffering {
            self.player.state = PlayerState::Buffering;
        } else if matches!(self.player.state, PlayerState::Buffering) {
            self.player.state = PlayerState::Playing;
        }
    }

    /// Updates playback position from native `nsh/audio` progress events WITHOUT
    /// incrementing `seek_generation` (preventing seek echo loops).
    pub fn on_native_audio_progress(&mut self, position_ms: u64, duration_ms: Option<u64>) {
        if self.player.current_track.is_none() {
            return;
        }
        self.player.position_ms = position_ms;
        self.player.queue.resume_position_ms = position_ms;
        if let Some(dur) = duration_ms.filter(|d| *d > 0) {
            self.player.duration_ms = dur;
        }
    }

    pub fn on_native_audio_ended(
        &mut self,
        live: bool,
        proxy_url_for: impl Fn(&str) -> String,
    ) -> Option<Track> {
        let len = self.player.queue.items.len();
        if len == 0 {
            self.player.state = PlayerState::Idle;
            self.player.active_proxy_url = None;
            return None;
        }
        let cur = self.player.queue.current_index.unwrap_or(0);
        let next = match self.player.queue.repeat {
            RepeatMode::One => Some(cur),
            RepeatMode::All => Some((cur + 1) % len),
            RepeatMode::None => (cur + 1 < len).then_some(cur + 1),
        };
        if let Some(next_idx) = next {
            self.activate_queue_index(next_idx, live, proxy_url_for)
        } else {
            self.player.state = PlayerState::Paused;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_core::fixture::fixture_tracks;

    fn mock_proxy(id: &str) -> String {
        format!("http://127.0.0.1:9999/stream/{id}?itag=251")
    }

    #[test]
    fn seek_increments_generation_but_native_progress_does_not() {
        let mut slice = PlaybackSlice::new(true, fixture_tracks(), mock_proxy);
        let gen0 = slice.seek_generation;
        slice.on_native_audio_progress(45_000, None);
        assert_eq!(slice.player.position_ms, 45_000);
        assert_eq!(slice.seek_generation, gen0);

        slice.seek_ms(12_000);
        assert_eq!(slice.player.position_ms, 12_000);
        assert_eq!(slice.seek_generation, gen0 + 1);
    }

    #[test]
    fn move_queue_item_preserves_active_index() {
        let mut slice = PlaybackSlice::new(true, fixture_tracks(), mock_proxy);
        assert_eq!(slice.player.queue.current_index, Some(0));
        slice.move_queue_item(0, 2);
        assert_eq!(slice.player.queue.current_index, Some(2));
    }

    #[test]
    fn shortcut_mapper_covers_media_keys() {
        assert_eq!(map_shortcut_key("Space"), Some(ShortcutAction::TogglePlayPause));
        assert_eq!(map_shortcut_key("ArrowRight"), Some(ShortcutAction::SeekForward10s));
        assert_eq!(map_shortcut_key("m"), Some(ShortcutAction::ToggleMute));
        assert_eq!(map_shortcut_key("z"), None);
    }
}
