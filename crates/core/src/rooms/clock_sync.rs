//! NTP-style median clock synchronization filter and 400ms hysteresis drift guard (`H2.3`).

use std::collections::VecDeque;

use resonance_contract::AuthoritativePlaybackState;

const WINDOW_SIZE: usize = 8;
pub const DRIFT_DEADBAND_MS: u64 = 400;
pub const SEEK_COOLDOWN_MS: u64 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClockSample {
    rtt_ms: u64,
    offset_ms: i64,
}

/// Rolling-window NTP-style median filter rejecting high-RTT outlier spikes.
#[derive(Debug, Clone, Default)]
pub struct ClockSyncFilter {
    samples: VecDeque<ClockSample>,
    current_offset_ms: i64,
}

impl ClockSyncFilter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a `ClockPong` measurement and returns the updated median offset (ms)
    /// if the sample was accepted (not an RTT outlier).
    pub fn record_pong(
        &mut self,
        client_send_ms: u64,
        server_recv_ms: u64,
        server_send_ms: u64,
        client_recv_ms: u64,
    ) -> Option<i64> {
        let total_elapsed = client_recv_ms.saturating_sub(client_send_ms);
        let server_proc = server_send_ms.saturating_sub(server_recv_ms);
        let rtt_ms = total_elapsed.saturating_sub(server_proc);

        if let Some(min_rtt) = self.min_rtt_ms()
            && self.samples.len() >= 3
            && rtt_ms > min_rtt.saturating_mul(2).max(20)
        {
            return None;
        }

        let offset_ms = ((server_recv_ms as i64 - client_send_ms as i64)
            + (server_send_ms as i64 - client_recv_ms as i64))
            / 2;

        if self.samples.len() >= WINDOW_SIZE {
            self.samples.pop_front();
        }
        self.samples.push_back(ClockSample { rtt_ms, offset_ms });

        let mut by_rtt: Vec<ClockSample> = self.samples.iter().copied().collect();
        by_rtt.sort_unstable_by_key(|s| s.rtt_ms);
        let mut offsets: Vec<i64> = by_rtt.into_iter().take(5).map(|s| s.offset_ms).collect();
        offsets.sort_unstable();
        self.current_offset_ms = offsets[offsets.len() / 2];
        Some(self.current_offset_ms)
    }

    pub fn offset_ms(&self) -> i64 {
        self.current_offset_ms
    }

    pub fn min_rtt_ms(&self) -> Option<u64> {
        self.samples.iter().map(|s| s.rtt_ms).min()
    }

    pub fn median_rtt_ms(&self) -> Option<u64> {
        if self.samples.is_empty() {
            return None;
        }
        let mut rtts: Vec<u64> = self.samples.iter().map(|s| s.rtt_ms).collect();
        rtts.sort_unstable();
        Some(rtts[rtts.len() / 2])
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftAction {
    Ignore,
    SeekTo(u64),
}

/// Hysteresis drift guard:
/// - `< 400 ms` drift: ignored (deadband)
/// - `>= 400 ms` sustained across 2 consecutive ticks (or immediate on revision bump):
///   issues a single `SeekTo(expected_pos_ms)` with a 3-second post-seek cooldown.
#[derive(Debug, Clone, Default)]
pub struct DriftGuard {
    last_revision: u64,
    consecutive_high_drift: u8,
    last_seek_at_ms: Option<u64>,
}

impl DriftGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Computes expected guest playback position in milliseconds.
    pub fn expected_position_ms(
        playback: &AuthoritativePlaybackState,
        local_now_ms: u64,
        clock_offset_ms: i64,
    ) -> u64 {
        if !playback.is_playing {
            return playback.anchor_position_ms;
        }
        let estimated_server_now = (local_now_ms as i64).saturating_add(clock_offset_ms);
        let elapsed = estimated_server_now
            .saturating_sub(playback.anchor_server_ms as i64)
            .max(0) as u64;
        playback.anchor_position_ms.saturating_add(elapsed)
    }

    pub fn evaluate(
        &mut self,
        playback: &AuthoritativePlaybackState,
        local_pos_ms: u64,
        local_now_ms: u64,
        clock_offset_ms: i64,
    ) -> DriftAction {
        let expected = Self::expected_position_ms(playback, local_now_ms, clock_offset_ms);
        let drift = (local_pos_ms as i64 - expected as i64).unsigned_abs();

        let revision_bumped = playback.revision > self.last_revision;
        if revision_bumped {
            self.last_revision = playback.revision;
            self.consecutive_high_drift = 0;
            if drift >= DRIFT_DEADBAND_MS {
                self.last_seek_at_ms = Some(local_now_ms);
                return DriftAction::SeekTo(expected);
            }
            return DriftAction::Ignore;
        }

        if let Some(last_seek) = self.last_seek_at_ms
            && local_now_ms.saturating_sub(last_seek) < SEEK_COOLDOWN_MS
        {
            return DriftAction::Ignore;
        }

        if drift < DRIFT_DEADBAND_MS {
            self.consecutive_high_drift = 0;
            return DriftAction::Ignore;
        }

        self.consecutive_high_drift = self.consecutive_high_drift.saturating_add(1);
        if self.consecutive_high_drift >= 2 {
            self.consecutive_high_drift = 0;
            self.last_seek_at_ms = Some(local_now_ms);
            DriftAction::SeekTo(expected)
        } else {
            DriftAction::Ignore
        }
    }
}
