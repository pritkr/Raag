//! Pure-Rust WSOLA (Waveform Similarity Overlap-Add) Pitch-Preserving Time-Stretcher.
//!
//! - `speed == 1.0`: 0-copy bypass.
//! - `preserve_pitch == true`: WSOLA with 40 ms Hann window and ±8 ms cross-correlation
//!   search window (`0.5×` – `2.0×`).
//! - `preserve_pitch == false`: linear-interpolated tape-speed resampling.

use std::f32::consts::PI;

#[derive(Debug, Clone)]
pub struct WsolaTimeStretcher {
    sample_rate: u32,
    channels: usize,
    speed: f32,
    preserve_pitch: bool,
    window_frames: usize,
    hop_out_frames: usize,
    search_frames: usize,
    hann: Vec<f32>,
}

impl WsolaTimeStretcher {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let sr = sample_rate.max(8_000);
        let ch = channels.max(1);
        let window_frames = ((sr as usize * 40) / 1000).max(64) & !1;
        let hop_out_frames = window_frames / 2;
        let search_frames = ((sr as usize * 8) / 1000).max(8);
        let hann = (0..window_frames)
            .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (window_frames - 1) as f32).cos()))
            .collect();
        Self {
            sample_rate: sr,
            channels: ch,
            speed: 1.0,
            preserve_pitch: true,
            window_frames,
            hop_out_frames,
            search_frames,
            hann,
        }
    }

    pub fn set_speed(&mut self, speed: f32, preserve_pitch: bool) {
        self.speed = speed.clamp(0.5, 2.0);
        self.preserve_pitch = preserve_pitch;
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn preserve_pitch(&self) -> bool {
        self.preserve_pitch
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Processes an interleaved `f32` buffer and returns time-stretched interleaved frames.
    /// When `speed == 1.0`, returns a direct clone with zero time/pitch modification.
    pub fn process_interleaved(&self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() || (self.speed - 1.0).abs() < 1e-4 {
            return input.to_vec();
        }

        if !self.preserve_pitch {
            return self.tape_speed_resample(input);
        }

        let ch = self.channels;
        let in_frames = input.len() / ch;
        if in_frames <= self.window_frames {
            return input.to_vec();
        }

        let out_frames_est = ((in_frames as f64) / (self.speed as f64)).round() as usize;
        let out_frames_est = out_frames_est.max(self.window_frames);
        let mut out = vec![0.0_f32; out_frames_est * ch];
        let mut norm = vec![0.0_f32; out_frames_est];

        let mut out_pos = 0usize;
        let mut prev_in_pos = 0usize;

        while out_pos + self.window_frames <= out_frames_est {
            let nominal_in = ((out_pos as f64) * (self.speed as f64)).round() as usize;
            let best_in = if out_pos == 0 {
                0
            } else {
                let target_prev_end = prev_in_pos + self.hop_out_frames;
                let min_k = nominal_in.saturating_sub(self.search_frames);
                let max_k = (nominal_in + self.search_frames)
                    .min(in_frames.saturating_sub(self.window_frames));
                if min_k >= max_k {
                    nominal_in.min(in_frames.saturating_sub(self.window_frames))
                } else {
                    let ref_start =
                        target_prev_end.min(in_frames.saturating_sub(self.window_frames));
                    let mut best_k = min_k;
                    let mut best_corr = f32::NEG_INFINITY;
                    let cmp_len = self.hop_out_frames.min(self.window_frames);
                    for k in min_k..=max_k {
                        let mut corr = 0.0_f32;
                        for f in 0..cmp_len {
                            let a = input[(ref_start + f) * ch];
                            let b = input[(k + f) * ch];
                            corr += a * b;
                        }
                        if corr > best_corr {
                            best_corr = corr;
                            best_k = k;
                        }
                    }
                    best_k
                }
            };

            if best_in + self.window_frames > in_frames {
                break;
            }

            for f in 0..self.window_frames {
                let w = self.hann[f];
                norm[out_pos + f] += w;
                let in_base = (best_in + f) * ch;
                let out_base = (out_pos + f) * ch;
                for c in 0..ch {
                    out[out_base + c] += input[in_base + c] * w;
                }
            }

            prev_in_pos = best_in;
            out_pos += self.hop_out_frames;
        }

        for (f, &n) in norm.iter().enumerate() {
            if n > 1e-6 {
                let inv = 1.0 / n;
                let base = f * ch;
                for c in 0..ch {
                    out[base + c] *= inv;
                }
            }
        }
        out
    }

    fn tape_speed_resample(&self, input: &[f32]) -> Vec<f32> {
        let ch = self.channels;
        let in_frames = input.len() / ch;
        if in_frames < 2 {
            return input.to_vec();
        }
        let out_frames = ((in_frames as f64) / (self.speed as f64)).round().max(1.0) as usize;
        let mut out = Vec::with_capacity(out_frames * ch);
        for i in 0..out_frames {
            let src = (i as f64) * (self.speed as f64);
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
