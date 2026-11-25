//! Pure deterministic playback finite-state machine (`PlaybackFsm`).
//!
//! Zero async runtime, zero network, zero disk I/O — 100% unit-testable in isolation.

use std::collections::HashSet;

use resonance_contract::{
    CodecBadgeInfo, CoreError, CoreResult, PlaybackError, PlayerSnapshot, PlayerState, QueueState,
    SleepTimerState, SponsorSegment, Track,
};

use crate::playback::prefetch::{PrefetchDecision, PrefetchThresholds};
use crate::playback::queue::QueueManager;

/// Pure effects emitted by [`PlaybackFsm::tick_position`] for the async controller to execute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickEffects {
    /// Optional SponsorBlock skip target `(video_id, target_ms)`.
    pub sponsor_skip: Option<(String, u64)>,
    /// Prefetch / Proxy v2 dual-buffer warm-up instruction.
    pub prefetch: PrefetchDecision,
    /// True when playback position has exceeded track duration past the safety grace window
    /// without an `Ended` event from the native audio backend.
    pub reached_end: bool,
}

/// Pure effects emitted by [`PlaybackFsm::tick_sleep_timer`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SleepTimerTickEffect {
    None,
    /// Smoothly attenuate volume during the final 3 seconds before auto-pause.
    AttenuateVolume(f32),
    /// Countdown reached zero: pause playback and restore `restore_volume`.
    ExpireAndPause {
        restore_volume: f32,
    },
}

/// Pure deterministic state machine behind [`super::PlaybackController`].
#[derive(Debug, Clone)]
pub struct PlaybackFsm {
    pub snapshot: PlayerSnapshot,
    pub sponsor_segments: Vec<SponsorSegment>,
    pub skipped_segments: HashSet<usize>,
    pub paused_by_focus: bool,
    /// Video ID already pre-resolved at >= 50% progress.
    pub preresolved_video_id: Option<String>,
    /// Video ID already pre-warmed in Proxy v2 `next_track` slot at >= 75% progress.
    pub prewarmed_video_id: Option<String>,
    /// Active sleep timer state (countdown or end-of-current-track).
    pub sleep_timer: SleepTimerState,
    /// Saved user volume prior to the 3-second sleep-timer fade-out.
    pub saved_volume_before_fade: Option<f32>,
    /// Truthful codec/source metadata for the active stream.
    pub codec_badge: Option<CodecBadgeInfo>,
    /// Guards against firing the end-of-track safety watchdog more than once per loaded track.
    pub ended_triggered: bool,
}

impl PlaybackFsm {
    pub fn from_restored_queue(restored_queue: QueueState) -> Self {
        let current_track = restored_queue.current_track().cloned();
        let resume_pos = restored_queue.resume_position_ms;
        let duration_ms = current_track.as_ref().map_or(0, |t| t.duration_ms);
        let initial_state = if current_track.is_some() {
            PlayerState::Paused
        } else {
            PlayerState::Idle
        };

        Self {
            snapshot: PlayerSnapshot {
                state: initial_state,
                current_track,
                position_ms: resume_pos,
                duration_ms,
                buffered_ms: 0,
                volume: 1.0,
                queue: restored_queue,
                active_proxy_url: None,
                sponsorblock_enabled: true,
                last_skipped_segment: None,
                offline_mode: false,
            },
            sponsor_segments: Vec::new(),
            skipped_segments: HashSet::new(),
            paused_by_focus: false,
            preresolved_video_id: None,
            prewarmed_video_id: None,
            sleep_timer: SleepTimerState::Off,
            saved_volume_before_fade: None,
            codec_badge: None,
            ended_triggered: false,
        }
    }

    /// Transitions into `Loading` for the current queue track, resetting per-track state.
    pub fn begin_loading(&mut self, resume_ms: u64) -> CoreResult<Track> {
        let Some(track) = self.snapshot.queue.current_track().cloned() else {
            return Err(CoreError::Playback(PlaybackError::EmptyQueue));
        };
        self.snapshot.current_track = Some(track.clone());
        self.snapshot.position_ms = resume_ms;
        self.snapshot.duration_ms = track.duration_ms;
        self.snapshot.last_skipped_segment = None;
        self.sponsor_segments.clear();
        self.skipped_segments.clear();
        self.preresolved_video_id = None;
        self.prewarmed_video_id = None;
        self.codec_badge = None;
        self.ended_triggered = false;
        let _ = self.snapshot.state.transition(PlayerState::Loading);
        Ok(track)
    }

    pub fn enter_buffering(&mut self, proxy_url: String, duration_ms: u64, offline_used: bool) {
        self.snapshot.active_proxy_url = Some(proxy_url);
        self.snapshot.duration_ms = duration_ms;
        self.snapshot.offline_mode = offline_used;
        let _ = self.snapshot.state.transition(PlayerState::Buffering);
    }

    pub fn enter_playing(&mut self) {
        self.paused_by_focus = false;
        let _ = self.snapshot.state.transition(PlayerState::Playing);
    }

    pub fn enter_paused(&mut self) {
        self.paused_by_focus = false;
        let _ = self.snapshot.state.transition(PlayerState::Paused);
        self.snapshot.queue.resume_position_ms = self.snapshot.position_ms;
    }

    pub fn enter_error(&mut self, message: impl Into<String>) {
        let _ = self
            .snapshot
            .state
            .transition(PlayerState::Error(message.into()));
    }

    pub fn clear_to_idle(&mut self) {
        QueueManager::clear_preserving_modes(&mut self.snapshot.queue);
        self.snapshot.current_track = None;
        self.snapshot.position_ms = 0;
        self.snapshot.duration_ms = 0;
        self.snapshot.active_proxy_url = None;
        self.preresolved_video_id = None;
        self.prewarmed_video_id = None;
        self.sponsor_segments.clear();
        self.skipped_segments.clear();
        self.codec_badge = None;
        let _ = self.snapshot.state.transition(PlayerState::Idle);
    }

    pub fn set_sleep_timer(&mut self, timer: SleepTimerState) {
        self.sleep_timer = timer;
        if matches!(timer, SleepTimerState::Off) {
            self.saved_volume_before_fade = None;
        }
    }

    /// Advances an active `SleepTimerState::Countdown` by `elapsed_secs`, attenuating
    /// volume over the final 3 seconds and returning `ExpireAndPause` at 0.
    pub fn tick_sleep_timer(&mut self, elapsed_secs: u64) -> SleepTimerTickEffect {
        let SleepTimerState::Countdown { remaining_secs } = self.sleep_timer else {
            return SleepTimerTickEffect::None;
        };
        let next_secs = remaining_secs.saturating_sub(elapsed_secs);
        if next_secs == 0 {
            let restore_volume = self
                .saved_volume_before_fade
                .take()
                .unwrap_or(self.snapshot.volume);
            self.sleep_timer = SleepTimerState::Off;
            return SleepTimerTickEffect::ExpireAndPause { restore_volume };
        }
        self.sleep_timer = SleepTimerState::Countdown {
            remaining_secs: next_secs,
        };
        if next_secs <= 3 {
            let base = *self
                .saved_volume_before_fade
                .get_or_insert(self.snapshot.volume);
            let attenuated = (base * (next_secs as f32 / 3.0)).clamp(0.0, 1.0);
            return SleepTimerTickEffect::AttenuateVolume(attenuated);
        }
        SleepTimerTickEffect::None
    }

    pub fn clamp_seek(&self, position_ms: u64) -> u64 {
        if self.snapshot.duration_ms > 0 {
            position_ms.min(self.snapshot.duration_ms)
        } else {
            position_ms
        }
    }

    pub fn apply_seek(&mut self, clamped_ms: u64) {
        self.snapshot.position_ms = clamped_ms;
        self.snapshot.queue.resume_position_ms = clamped_ms;
    }

    /// Evaluates a playback position tick: updates resume position, checks SponsorBlock segments,
    /// and computes both 50% stream pre-resolution and 75% Proxy v2 dual-buffer warm-up decisions.
    pub fn tick_position(&mut self, position_ms: u64) -> TickEffects {
        let dur_ms = self.snapshot.duration_ms;
        let clamped_pos = if dur_ms > 0 {
            position_ms.min(dur_ms)
        } else {
            position_ms
        };
        self.snapshot.position_ms = clamped_pos;
        self.snapshot.queue.resume_position_ms = clamped_pos;

        let reached_end = if dur_ms > 0
            && self.snapshot.state == PlayerState::Playing
            && !self.ended_triggered
            && position_ms >= dur_ms.saturating_add(1_000)
        {
            self.ended_triggered = true;
            true
        } else {
            false
        };

        let dur_secs = dur_ms as f64 / 1000.0;
        let sponsor_skip = if self.snapshot.sponsorblock_enabled {
            self.sponsor_segments
                .iter()
                .enumerate()
                .find(|(idx, seg)| {
                    !self.skipped_segments.contains(idx)
                        && seg.fits_duration(dur_secs)
                        && seg.contains_ms(clamped_pos)
                })
                .map(|(idx, seg)| (idx, (seg.end_secs * 1000.0) as u64))
        } else {
            None
        }
        .map(|(idx, target_ms)| {
            self.skipped_segments.insert(idx);
            (
                self.snapshot
                    .current_track
                    .as_ref()
                    .map(|t| t.id.clone())
                    .unwrap_or_default(),
                target_ms,
            )
        });

        let upcoming = QueueManager::peek_upcoming_track_id(&self.snapshot.queue);
        let prefetch = PrefetchThresholds::evaluate(
            clamped_pos,
            dur_ms,
            upcoming.as_deref(),
            self.preresolved_video_id.as_deref(),
            self.prewarmed_video_id.as_deref(),
        );

        match &prefetch {
            PrefetchDecision::None => {}
            PrefetchDecision::ResolveOnly { video_id } => {
                self.preresolved_video_id = Some(video_id.clone());
            }
            PrefetchDecision::WarmProxySlot { video_id } => {
                self.preresolved_video_id = Some(video_id.clone());
                self.prewarmed_video_id = Some(video_id.clone());
            }
        }

        TickEffects {
            sponsor_skip,
            prefetch,
            reached_end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_contract::SponsorSegment;

    fn sample_track(id: &str, duration_ms: u64) -> Track {
        Track {
            id: id.to_owned(),
            title: format!("Track {id}"),
            artists: Vec::new(),
            album: None,
            duration_ms,
            artwork_url: None,
            is_explicit: false,
            is_liked: false,
            is_video: false,
        }
    }

    #[test]
    fn deterministic_state_transitions_and_prefetch_thresholds() {
        let mut q = QueueState::default();
        QueueManager::replace(
            &mut q,
            vec![sample_track("t1", 200_000), sample_track("t2", 180_000)],
            0,
        )
        .unwrap();
        let mut fsm = PlaybackFsm::from_restored_queue(q);
        assert_eq!(fsm.snapshot.state, PlayerState::Paused);

        fsm.begin_loading(0).unwrap();
        assert_eq!(fsm.snapshot.state, PlayerState::Loading);

        fsm.enter_buffering("http://127.0.0.1:9999/stream/t1".into(), 200_000, false);
        assert_eq!(fsm.snapshot.state, PlayerState::Buffering);

        fsm.enter_playing();
        assert_eq!(fsm.snapshot.state, PlayerState::Playing);

        // 25% -> no prefetch
        let fx25 = fsm.tick_position(50_000);
        assert_eq!(fx25.prefetch, PrefetchDecision::None);

        // 50% -> resolve next track stream metadata
        let fx50 = fsm.tick_position(100_000);
        assert_eq!(
            fx50.prefetch,
            PrefetchDecision::ResolveOnly {
                video_id: "t2".into()
            }
        );

        // 60% -> already resolved, below 75% warm threshold
        let fx60 = fsm.tick_position(120_000);
        assert_eq!(fx60.prefetch, PrefetchDecision::None);

        // 75% -> pre-warm Proxy v2 next_track dual-buffer slot
        let fx75 = fsm.tick_position(150_000);
        assert_eq!(
            fx75.prefetch,
            PrefetchDecision::WarmProxySlot {
                video_id: "t2".into()
            }
        );

        // 85% -> already pre-warmed
        let fx85 = fsm.tick_position(170_000);
        assert_eq!(fx85.prefetch, PrefetchDecision::None);
    }

    #[test]
    fn detects_sponsorblock_skip_window() {
        let mut q = QueueState::default();
        QueueManager::replace(&mut q, vec![sample_track("t1", 200_000)], 0).unwrap();
        let mut fsm = PlaybackFsm::from_restored_queue(q);
        fsm.begin_loading(0).unwrap();
        fsm.sponsor_segments.push(SponsorSegment {
            start_secs: 10.0,
            end_secs: 25.0,
            video_duration_secs: 200.0,
        });

        let fx = fsm.tick_position(12_000);
        assert_eq!(fx.sponsor_skip, Some(("t1".into(), 25_000)));
    }

    #[test]
    fn clamps_position_to_duration_and_triggers_reached_end_once() {
        let mut q = QueueState::default();
        QueueManager::replace(&mut q, vec![sample_track("t1", 100_000)], 0).unwrap();
        let mut fsm = PlaybackFsm::from_restored_queue(q);
        fsm.begin_loading(0).unwrap();
        fsm.enter_buffering("http://127.0.0.1:9999/stream/t1".into(), 100_000, false);
        fsm.enter_playing();

        // Slightly past duration (< 1s grace): clamped, does not force end yet.
        let fx_grace = fsm.tick_position(100_500);
        assert_eq!(fsm.snapshot.position_ms, 100_000);
        assert!(!fx_grace.reached_end);

        // Past 1s grace without Ended telemetry: triggers reached_end once.
        let fx_over = fsm.tick_position(101_000);
        assert_eq!(fsm.snapshot.position_ms, 100_000);
        assert!(fx_over.reached_end);

        // Subsequent ticks do not re-trigger reached_end for the same track.
        let fx_again = fsm.tick_position(102_000);
        assert!(!fx_again.reached_end);
    }
}
