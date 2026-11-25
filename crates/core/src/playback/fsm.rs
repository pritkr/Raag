//! Pure deterministic playback finite-state machine (`PlaybackFsm`).
//!
//! Zero async runtime, zero network, zero disk I/O — 100% unit-testable in isolation.

use resonance_contract::{
    CoreError, CoreResult, PlaybackError, PlayerSnapshot, PlayerState, QueueState, SponsorSegment,
    Track,
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
}

/// Pure deterministic state machine behind [`super::PlaybackController`].
#[derive(Debug, Clone)]
pub struct PlaybackFsm {
    pub snapshot: PlayerSnapshot,
    pub sponsor_segments: Vec<SponsorSegment>,
    pub paused_by_focus: bool,
    /// Video ID already pre-resolved at >= 50% progress.
    pub preresolved_video_id: Option<String>,
    /// Video ID already pre-warmed in Proxy v2 `next_track` slot at >= 75% progress.
    pub prewarmed_video_id: Option<String>,
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
            paused_by_focus: false,
            preresolved_video_id: None,
            prewarmed_video_id: None,
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
        self.preresolved_video_id = None;
        self.prewarmed_video_id = None;
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
        let _ = self.snapshot.state.transition(PlayerState::Idle);
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
        self.snapshot.position_ms = position_ms;
        self.snapshot.queue.resume_position_ms = position_ms;

        let dur_ms = self.snapshot.duration_ms;
        let dur_secs = dur_ms as f64 / 1000.0;
        let sponsor_skip = if self.snapshot.sponsorblock_enabled {
            self.sponsor_segments
                .iter()
                .find(|seg| seg.fits_duration(dur_secs) && seg.contains_ms(position_ms))
                .map(|seg| {
                    (
                        self.snapshot
                            .current_track
                            .as_ref()
                            .map(|t| t.id.clone())
                            .unwrap_or_default(),
                        (seg.end_secs * 1000.0) as u64,
                    )
                })
        } else {
            None
        };

        let upcoming = QueueManager::peek_upcoming_track_id(&self.snapshot.queue);
        let prefetch = PrefetchThresholds::evaluate(
            position_ms,
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
}
