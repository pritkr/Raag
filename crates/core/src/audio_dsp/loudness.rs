//! ReplayGain / EBU R128 loudness normalization driven by YouTube's `loudnessDb` metadata.
//!
//! YouTube reports `playerConfig.audioConfig.loudnessDb` as dB relative to its -14 LUFS
//! integrated target (positive = louder than target, negative = quieter than target).
//! Applies bounded gain plus a transparent soft-knee peak limiter to prevent digital clipping.

/// Default EBU R128 / ReplayGain target offset relative to YouTube's -14 LUFS reference.
pub const DEFAULT_TARGET_OFFSET_DB: f32 = 0.0;
const MIN_GAIN_DB: f32 = -18.0;
const MAX_GAIN_DB: f32 = 12.0;
const LIMITER_KNEE: f32 = 0.92;
const LIMITER_CEILING: f32 = 0.99;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessNormalizer {
    pub enabled: bool,
    pub target_offset_db: f32,
    pub preamp_db: f32,
    linear_gain: f32,
}

impl Default for LoudnessNormalizer {
    fn default() -> Self {
        Self {
            enabled: true,
            target_offset_db: DEFAULT_TARGET_OFFSET_DB,
            preamp_db: 0.0,
            linear_gain: 1.0,
        }
    }
}

impl LoudnessNormalizer {
    pub fn new(enabled: bool, target_offset_db: f32, preamp_db: f32) -> Self {
        Self {
            enabled,
            target_offset_db,
            preamp_db,
            linear_gain: 1.0,
        }
    }

    /// Configures gain from YouTube's `StreamInfo::loudness_db` metadata.
    pub fn set_track_loudness_db(&mut self, loudness_db: Option<f32>) {
        if !self.enabled {
            self.linear_gain = 1.0;
            return;
        }
        let Some(track_db) = loudness_db else {
            self.linear_gain = db_to_linear(self.preamp_db.clamp(MIN_GAIN_DB, MAX_GAIN_DB));
            return;
        };
        let adjustment_db =
            (self.target_offset_db - track_db + self.preamp_db).clamp(MIN_GAIN_DB, MAX_GAIN_DB);
        self.linear_gain = db_to_linear(adjustment_db);
    }

    pub fn linear_gain(&self) -> f32 {
        if self.enabled { self.linear_gain } else { 1.0 }
    }

    /// Processes interleaved PCM samples in-place with zero heap allocations.
    #[inline]
    pub fn process_in_place(&self, samples: &mut [f32]) {
        if !self.enabled {
            return;
        }
        let g = self.linear_gain;
        if (g - 1.0).abs() < 1e-6 {
            return;
        }
        for s in samples.iter_mut() {
            *s = soft_limit(*s * g);
        }
    }
}

#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

#[inline]
pub(crate) fn soft_limit(x: f32) -> f32 {
    let abs = x.abs();
    if abs <= LIMITER_KNEE {
        x
    } else {
        let excess = abs - LIMITER_KNEE;
        let range = LIMITER_CEILING - LIMITER_KNEE;
        let compressed = LIMITER_KNEE + range * (excess / (excess + range));
        compressed.copysign(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attenuates_loud_tracks_and_limits_peaks() {
        let mut norm = LoudnessNormalizer::default();
        // Track mastered +6 dB hotter than -14 LUFS -> ~0.5x linear attenuation
        norm.set_track_loudness_db(Some(6.0));
        assert!((norm.linear_gain() - 0.5012).abs() < 0.01);

        let mut buf = [0.8_f32, -0.8_f32];
        norm.process_in_place(&mut buf);
        assert!((buf[0] - 0.401).abs() < 0.01);

        // Quiet track boosted +12 dB -> soft limiter prevents > 0.99 clipping
        norm.set_track_loudness_db(Some(-12.0));
        let mut hot = [0.7_f32, -0.7_f32];
        norm.process_in_place(&mut hot);
        assert!(hot[0] < LIMITER_CEILING && hot[0] > LIMITER_KNEE);
        assert!(hot[1] > -LIMITER_CEILING && hot[1] < -LIMITER_KNEE);
    }
}
