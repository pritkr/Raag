//! 3-Zone Hysteresis Drift Controller (`W2D §3.2`).
//!
//! - Zone 1 (Deadband): `|drift_ms| < 60 ms` (or `< 20 ms` to exit slew) -> `Nominal` (`1.000x`).
//! - Zone 2 (Micro-Slew): `60 <= |drift_ms| < 450 ms` sustained for 3 consecutive ticks
//!   when native PCM slew is available -> `Slew(ratio)` clamped to `[0.985, 1.015]`.
//! - Zone 3 (Hard Seek): `|drift_ms| >= 450 ms` (or `>= 250 ms` when slew unavailable)
//!   with a 3-second post-seek cooldown -> `HardSeek(expected_position_ms)`.

use resonance_contract::AuthoritativePlaybackState;

use crate::rooms::clock_sync::DriftGuard;

pub const DEADBAND_ENTER_MS: u64 = 60;
pub const DEADBAND_EXIT_MS: u64 = 20;
pub const SLEW_MAX_DRIFT_MS: u64 = 450;
pub const HARD_SEEK_NO_SLEW_MS: u64 = 250;
pub const SLEW_CONSECUTIVE_TICKS: u8 = 3;
pub const THREE_ZONE_SEEK_COOLDOWN_MS: u64 = 3_000;
pub const ZERO_HOUR_LEAD_MS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThreeZoneDriftAction {
    Nominal,
    Slew(f64),
    HardSeek(u64),
}

#[derive(Debug, Clone, Default)]
pub struct ThreeZoneDriftController {
    last_revision: u64,
    in_slew: bool,
    consecutive_slew_ticks: u8,
    last_seek_at_ms: Option<u64>,
}

impl ThreeZoneDriftController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_slewing(&self) -> bool {
        self.in_slew
    }

    pub fn schedule_zero_hour_anchor_ms(server_now_ms: u64) -> u64 {
        server_now_ms.saturating_add(ZERO_HOUR_LEAD_MS)
    }

    pub fn evaluate(
        &mut self,
        playback: &AuthoritativePlaybackState,
        local_pos_ms: u64,
        local_now_ms: u64,
        clock_offset_ms: i64,
        slew_available: bool,
    ) -> ThreeZoneDriftAction {
        let expected = DriftGuard::expected_position_ms(playback, local_now_ms, clock_offset_ms);
        let signed_drift_ms = local_pos_ms as i64 - expected as i64;
        let abs_drift = signed_drift_ms.unsigned_abs();
        let hard_threshold = if slew_available {
            SLEW_MAX_DRIFT_MS
        } else {
            HARD_SEEK_NO_SLEW_MS
        };

        if playback.revision > self.last_revision {
            self.last_revision = playback.revision;
            self.in_slew = false;
            self.consecutive_slew_ticks = 0;
            if abs_drift >= hard_threshold {
                self.last_seek_at_ms = Some(local_now_ms);
                return ThreeZoneDriftAction::HardSeek(expected);
            }
            return ThreeZoneDriftAction::Nominal;
        }

        if abs_drift >= hard_threshold {
            if let Some(last_seek) = self.last_seek_at_ms
                && local_now_ms.saturating_sub(last_seek) < THREE_ZONE_SEEK_COOLDOWN_MS
            {
                return ThreeZoneDriftAction::Nominal;
            }
            self.in_slew = false;
            self.consecutive_slew_ticks = 0;
            self.last_seek_at_ms = Some(local_now_ms);
            return ThreeZoneDriftAction::HardSeek(expected);
        }

        if self.in_slew {
            if abs_drift < DEADBAND_EXIT_MS {
                self.in_slew = false;
                self.consecutive_slew_ticks = 0;
                return ThreeZoneDriftAction::Nominal;
            }
            return ThreeZoneDriftAction::Slew(Self::compute_slew_ratio(signed_drift_ms));
        }

        if abs_drift < DEADBAND_ENTER_MS {
            self.consecutive_slew_ticks = 0;
            return ThreeZoneDriftAction::Nominal;
        }

        if slew_available {
            self.consecutive_slew_ticks = self.consecutive_slew_ticks.saturating_add(1);
            if self.consecutive_slew_ticks >= SLEW_CONSECUTIVE_TICKS {
                self.in_slew = true;
                self.consecutive_slew_ticks = 0;
                return ThreeZoneDriftAction::Slew(Self::compute_slew_ratio(signed_drift_ms));
            }
        }

        ThreeZoneDriftAction::Nominal
    }

    fn compute_slew_ratio(signed_drift_ms: i64) -> f64 {
        (1.0 - (signed_drift_ms as f64) / 15_000.0).clamp(0.985, 1.015)
    }
}
