//! Automatic ATR (`/api/stats/atr`) watchtime reporter.
//!
//! Subscribes to `EventBus` (`AppEvent::PlayerUpdated`, `AppEvent::AuthStateChanged`)
//! and emits `WatchtimeReport` commands via `MutationWorker` at 30s played, on pause,
//! and on track end/switch — strictly when `signed_in == true`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rand::Rng;
use resonance_contract::{
    AppEvent, EventBus, MutationCommand, PlayerSnapshot, PlayerState, WatchtimeReport,
};
use tokio::sync::broadcast;

use crate::innertube::mutations::MutationWorker;

const ATR_MILESTONE_SECS: u64 = 30;
const BASE64URL_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Generates a 16-character base64url Client Playback Nonce (`cpn`).
pub fn generate_cpn() -> String {
    let mut rng = rand::rng();
    (0..16)
        .map(|_| {
            let idx = rng.random_range(0..64);
            BASE64URL_ALPHABET[idx] as char
        })
        .collect()
}

/// Deterministic state machine tracking playback milestones and generating
/// `WatchtimeReport`s when signed in.
#[derive(Debug, Clone)]
pub struct AtrTracker {
    signed_in: bool,
    atr_enabled: bool,
    active_video_id: Option<String>,
    active_cpn: String,
    active_duration_secs: u64,
    last_reported_secs: u64,
    max_seen_secs: u64,
    milestone_30s_sent: bool,
    was_playing: bool,
    final_sent_for_active: bool,
}

impl AtrTracker {
    pub fn new(signed_in: bool) -> Self {
        Self {
            signed_in,
            atr_enabled: true,
            active_video_id: None,
            active_cpn: generate_cpn(),
            active_duration_secs: 0,
            last_reported_secs: 0,
            max_seen_secs: 0,
            milestone_30s_sent: false,
            was_playing: false,
            final_sent_for_active: false,
        }
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.atr_enabled = enabled;
        self
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.atr_enabled = enabled;
        if !enabled {
            self.reset_session();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.atr_enabled
    }

    pub fn is_signed_in(&self) -> bool {
        self.signed_in
    }

    fn reset_session(&mut self) {
        self.active_video_id = None;
        self.active_cpn = generate_cpn();
        self.active_duration_secs = 0;
        self.last_reported_secs = 0;
        self.max_seen_secs = 0;
        self.milestone_30s_sent = false;
        self.was_playing = false;
        self.final_sent_for_active = false;
    }

    fn flush_final_if_needed(&mut self, out: &mut Vec<WatchtimeReport>) {
        if !self.signed_in || !self.atr_enabled || self.final_sent_for_active {
            return;
        }
        let Some(video_id) = self.active_video_id.clone() else {
            return;
        };
        if video_id.starts_with("local:") {
            return;
        }
        if self.max_seen_secs > 0 || self.was_playing || self.milestone_30s_sent {
            let end_secs = self.max_seen_secs.max(self.last_reported_secs);
            out.push(WatchtimeReport {
                video_id,
                cpn: self.active_cpn.clone(),
                start_secs: self.last_reported_secs,
                end_secs,
                duration_secs: self.active_duration_secs,
                is_final: true,
            });
            self.last_reported_secs = end_secs;
            self.final_sent_for_active = true;
        }
    }

    /// Processes a single `AppEvent` and returns any `WatchtimeReport`s to enqueue.
    pub fn process_event(&mut self, event: &AppEvent) -> Vec<WatchtimeReport> {
        let mut out = Vec::new();
        match event {
            AppEvent::AuthStateChanged { signed_in, .. } => {
                self.signed_in = *signed_in;
                if !self.signed_in {
                    self.reset_session();
                }
            }
            AppEvent::PlayerUpdated(snap) => {
                self.process_player_snapshot(snap, &mut out);
            }
            _ => {}
        }
        out
    }

    fn process_player_snapshot(&mut self, snap: &PlayerSnapshot, out: &mut Vec<WatchtimeReport>) {
        if !self.signed_in || !self.atr_enabled {
            self.reset_session();
            return;
        }

        let next_id = snap
            .current_track
            .as_ref()
            .filter(|t| !t.id.starts_with("local:"))
            .map(|t| t.id.as_str());
        if self.active_video_id.as_deref() != next_id {
            self.flush_final_if_needed(out);
            self.reset_session();
            if let Some(track) = &snap.current_track {
                self.active_video_id = Some(track.id.clone());
                self.active_duration_secs = (snap.duration_ms.max(track.duration_ms)) / 1000;
            }
        }

        let Some(video_id) = self.active_video_id.clone() else {
            return;
        };

        if let Some(track) = &snap.current_track {
            let dur = snap.duration_ms.max(track.duration_ms) / 1000;
            if dur > 0 {
                self.active_duration_secs = dur;
            }
        }

        let pos_secs = snap.position_ms / 1000;
        self.max_seen_secs = self.max_seen_secs.max(pos_secs);
        let is_playing = matches!(snap.state, PlayerState::Playing);

        if is_playing && pos_secs >= ATR_MILESTONE_SECS && !self.milestone_30s_sent {
            self.milestone_30s_sent = true;
            let end_secs = pos_secs.max(self.last_reported_secs);
            out.push(WatchtimeReport {
                video_id: video_id.clone(),
                cpn: self.active_cpn.clone(),
                start_secs: self.last_reported_secs,
                end_secs,
                duration_secs: self.active_duration_secs,
                is_final: false,
            });
            self.last_reported_secs = end_secs;
        }

        if self.was_playing && matches!(snap.state, PlayerState::Paused) {
            let end_secs = pos_secs.max(self.last_reported_secs);
            if end_secs > self.last_reported_secs || !self.milestone_30s_sent {
                out.push(WatchtimeReport {
                    video_id: video_id.clone(),
                    cpn: self.active_cpn.clone(),
                    start_secs: self.last_reported_secs,
                    end_secs,
                    duration_secs: self.active_duration_secs,
                    is_final: false,
                });
                self.last_reported_secs = end_secs;
            }
        }

        if self.was_playing && matches!(snap.state, PlayerState::Idle) {
            self.flush_final_if_needed(out);
        }

        self.was_playing = is_playing;
    }
}

/// Background task subscribing to `EventBus` and enqueueing `MutationCommand::ReportAtr`
/// onto `MutationWorker`.
pub struct AtrReporter;

impl AtrReporter {
    pub fn spawn(
        bus: EventBus,
        worker: MutationWorker,
        initial_signed_in: bool,
    ) -> tokio::task::JoinHandle<()> {
        Self::spawn_with_enabled(
            bus,
            worker,
            initial_signed_in,
            Arc::new(AtomicBool::new(true)),
        )
    }

    pub fn spawn_with_enabled(
        bus: EventBus,
        worker: MutationWorker,
        initial_signed_in: bool,
        enabled: Arc<AtomicBool>,
    ) -> tokio::task::JoinHandle<()> {
        let mut rx = bus.subscribe();
        tokio::spawn(async move {
            let mut tracker =
                AtrTracker::new(initial_signed_in).with_enabled(enabled.load(Ordering::SeqCst));
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let desired_enabled = enabled.load(Ordering::SeqCst);
                        if tracker.is_enabled() != desired_enabled {
                            tracker.set_enabled(desired_enabled);
                        }
                        for report in tracker.process_event(&event) {
                            let _ = worker.enqueue(MutationCommand::ReportAtr(report));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
    }
}
