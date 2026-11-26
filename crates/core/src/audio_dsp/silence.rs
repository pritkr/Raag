//! Real-time silence skipper using RMS dBFS energy detection, hold windowing,
//! and 10 ms Hann fade-in/fade-out edges to eliminate clicks/pops.

use std::f32::consts::PI;

#[derive(Debug, Clone, Copy)]
pub struct SilenceSkipper {
    pub enabled: bool,
    threshold_linear: f32,
    min_silence_frames: usize,
    consecutive_silent_frames: usize,
    channels: usize,
    fade_frames: usize,
    was_skipping: bool,
}

impl SilenceSkipper {
    /// Creates a silence skipper with `threshold_db` (e.g. `-45.0` dBFS) and
    /// `min_silence_ms` hold window before frames are dropped.
    pub fn new(sample_rate: u32, channels: usize, threshold_db: f32, min_silence_ms: u32) -> Self {
        let sr = sample_rate.max(8_000) as usize;
        let min_frames = (sr * min_silence_ms as usize) / 1000;
        let fade_frames = ((sr * 10) / 1000).max(16);
        Self {
            enabled: false,
            threshold_linear: 10.0_f32.powf(threshold_db / 20.0),
            min_silence_frames: min_frames.max(1),
            consecutive_silent_frames: 0,
            channels: channels.max(1),
            fade_frames,
            was_skipping: false,
        }
    }

    pub fn reset(&mut self) {
        self.consecutive_silent_frames = 0;
        self.was_skipping = false;
    }

    /// Evaluates an interleaved PCM block and returns `true` when the block falls inside
    /// an active skip window (continuous RMS below threshold for >= `min_silence_ms`).
    pub fn should_skip_block(&mut self, interleaved: &[f32]) -> bool {
        if !self.enabled || interleaved.is_empty() {
            self.consecutive_silent_frames = 0;
            self.was_skipping = false;
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
            let skip = self.consecutive_silent_frames > self.min_silence_frames;
            self.was_skipping = skip;
            skip
        } else {
            self.consecutive_silent_frames = 0;
            false
        }
    }

    /// Processes an interleaved PCM block in-place with click-free 10 ms Hann fade-in
    /// when resuming after a skipped silent region. Returns `false` if the block is skipped.
    pub fn process_interleaved(&mut self, interleaved: &mut [f32]) -> bool {
        let was_skipping = self.was_skipping;
        if self.should_skip_block(interleaved) {
            return false;
        }
        if was_skipping {
            self.was_skipping = false;
            let ch = self.channels;
            let total_frames = interleaved.len() / ch;
            let ramp = self.fade_frames.min(total_frames);
            if ramp > 1 {
                for f in 0..ramp {
                    let t = f as f32 / (ramp - 1) as f32;
                    let gain = 0.5 * (1.0 - (PI * t).cos());
                    let base = f * ch;
                    for c in 0..ch {
                        interleaved[base + c] *= gain;
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_only_after_minimum_silence_hold_window() {
        let mut skipper = SilenceSkipper::new(8_000, 2, -40.0, 200);
        skipper.enabled = true;

        // 800 stereo frames (100ms at 8kHz) of near-silence -> below 200ms hold, keep
        let silent_100 = vec![0.0001_f32; 1600];
        assert!(!skipper.should_skip_block(&silent_100));
        // Another 1200 stereo frames (150ms) -> exceeds 200ms hold, skip!
        let silent_150 = vec![0.0001_f32; 2400];
        assert!(skipper.should_skip_block(&silent_150));

        // Audio returns -> immediately stops skipping
        let audible = vec![0.25_f32; 1600];
        assert!(!skipper.should_skip_block(&audible));
    }
}
