//! Real-time silence skipper using RMS dBFS energy detection and hold windowing.

#[derive(Debug, Clone, Copy)]
pub struct SilenceSkipper {
    pub enabled: bool,
    threshold_linear: f32,
    min_silence_frames: usize,
    consecutive_silent_frames: usize,
    channels: usize,
}

impl SilenceSkipper {
    /// Creates a silence skipper with `threshold_db` (e.g. `-45.0` dBFS) and
    /// `min_silence_ms` hold window before frames are dropped.
    pub fn new(sample_rate: u32, channels: usize, threshold_db: f32, min_silence_ms: u32) -> Self {
        let sr = sample_rate.max(8_000) as usize;
        let min_frames = (sr * min_silence_ms as usize) / 1000;
        Self {
            enabled: false,
            threshold_linear: 10.0_f32.powf(threshold_db / 20.0),
            min_silence_frames: min_frames.max(1),
            consecutive_silent_frames: 0,
            channels: channels.max(1),
        }
    }

    pub fn reset(&mut self) {
        self.consecutive_silent_frames = 0;
    }

    /// Evaluates an interleaved PCM block and returns `true` when the block falls inside
    /// an active skip window (continuous RMS below threshold for >= `min_silence_ms`).
    pub fn should_skip_block(&mut self, interleaved: &[f32]) -> bool {
        if !self.enabled || interleaved.is_empty() {
            self.consecutive_silent_frames = 0;
            return false;
        }
        let frames = interleaved.len() / self.channels;
        if frames == 0 {
            return false;
        }
        let sum_sq: f32 = interleaved.iter().map(|s| s * s).sum();
        let rms = (sum_sq / interleaved.len() as f32).sqrt();
        if rms < self.threshold_linear {
            self.consecutive_silent_frames = self.consecutive_silent_frames.saturating_add(frames);
            self.consecutive_silent_frames > self.min_silence_frames
        } else {
            self.consecutive_silent_frames = 0;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_only_after_minimum_silence_hold_window() {
        let mut skipper = SilenceSkipper::new(1000, 2, -40.0, 200);
        skipper.enabled = true;

        // 100 stereo frames of near-silence -> below 200ms hold, keep
        let silent_100 = vec![0.0001_f32; 200];
        assert!(!skipper.should_skip_block(&silent_100));
        // Another 150 stereo frames -> exceeds 200ms hold, skip!
        let silent_150 = vec![0.0001_f32; 300];
        assert!(skipper.should_skip_block(&silent_150));

        // Audio returns -> immediately stops skipping
        let audible = vec![0.25_f32; 200];
        assert!(!skipper.should_skip_block(&audible));
    }
}
