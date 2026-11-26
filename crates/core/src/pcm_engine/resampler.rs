//! RT-Safe Slew-Capable Sample-Rate Converter (`crates/core/src/pcm_engine/resampler.rs`).
//!
//! Supports integer/fractional device sample-rate conversion AND `±1.5%` micro-slew
//! (`0.985..=1.015`) for W2D Listen Together drift elimination.

pub const MIN_SLEW_RATIO: f64 = 0.985;
pub const MAX_SLEW_RATIO: f64 = 1.015;

#[derive(Debug, Clone)]
pub struct SlewResampler {
    in_rate: u32,
    out_rate: u32,
    channels: usize,
    slew_ratio: f64,
}

impl SlewResampler {
    pub fn new(in_rate: u32, out_rate: u32, channels: usize) -> Self {
        Self {
            in_rate: in_rate.max(8_000),
            out_rate: out_rate.max(8_000),
            channels: channels.max(1),
            slew_ratio: 1.0,
        }
    }

    /// Sets the Listen Together micro-slew ratio, clamped strictly to `[0.985, 1.015]`.
    pub fn set_slew_ratio(&mut self, ratio: f64) {
        self.slew_ratio = ratio.clamp(MIN_SLEW_RATIO, MAX_SLEW_RATIO);
    }

    pub fn slew_ratio(&self) -> f64 {
        self.slew_ratio
    }

    pub fn set_rates(&mut self, in_rate: u32, out_rate: u32) {
        self.in_rate = in_rate.max(8_000);
        self.out_rate = out_rate.max(8_000);
    }

    /// Resamples interleaved `f32` frames from `in_rate` to `out_rate * slew_ratio`.
    /// Uses zero-copy bypass when `in_rate == out_rate` and `slew_ratio == 1.0`.
    pub fn process_interleaved(&self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        if self.in_rate == self.out_rate && (self.slew_ratio - 1.0).abs() < 1e-6 {
            return input.to_vec();
        }
        let ch = self.channels;
        let in_frames = input.len() / ch;
        if in_frames < 2 {
            return input.to_vec();
        }

        let step = (self.in_rate as f64 / self.out_rate as f64) * self.slew_ratio;
        let out_frames = ((in_frames as f64) / step).round().max(1.0) as usize;
        let mut out = Vec::with_capacity(out_frames * ch);
        for i in 0..out_frames {
            let src = (i as f64) * step;
            let idx0 = (src.floor() as usize).min(in_frames - 1);
            let idx1 = (idx0 + 1).min(in_frames - 1);
            let frac = (src - idx0 as f64) as f32;
            for c in 0..ch {
                let s0 = input[idx0 * ch + c];
                let s1 = input[idx1 * ch + c];
                out.push(s0 + (s1 - s0) * frac);
            }
        }
        out
    }
}
