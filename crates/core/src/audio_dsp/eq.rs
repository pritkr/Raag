//! 10-band parametric equalizer built with Audio EQ Cookbook biquad peaking filters
//! (Direct Form II Transposed) at ISO octave center frequencies (31 Hz – 16 kHz).

use std::f32::consts::PI;

/// ISO 10-band graphic/parametric EQ center frequencies in Hz.
pub const EQ_BAND_FREQS_HZ: [f32; 10] = [
    31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

const DEFAULT_Q: f32 = 1.414_213_5; // 1-octave peaking Q
const MIN_BAND_DB: f32 = -12.0;
const MAX_BAND_DB: f32 = 12.0;

pub use resonance_contract::EqPreset;

#[derive(Debug, Clone, Copy)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    fn passthrough() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }

    fn peaking_eq(sample_rate: f32, freq_hz: f32, q: f32, gain_db: f32) -> Self {
        let clamped_db = gain_db.clamp(MIN_BAND_DB, MAX_BAND_DB);
        if clamped_db.abs() < 0.01 || freq_hz >= sample_rate * 0.49 {
            return Self::passthrough();
        }
        let a = 10.0_f32.powf(clamped_db / 40.0);
        let w0 = 2.0 * PI * (freq_hz / sample_rate);
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BiquadState {
    z1: f32,
    z2: f32,
}

impl BiquadState {
    #[inline]
    fn process(&mut self, c: &BiquadCoeffs, x: f32) -> f32 {
        let y = c.b0 * x + self.z1;
        let z1 = c.b1 * x - c.a1 * y + self.z2;
        let z2 = c.b2 * x - c.a2 * y;
        // Flush subnormal/denormal values to zero to prevent x86/ARM CPU spikes on silence.
        self.z1 = if z1.abs() < 1e-18 { 0.0 } else { z1 };
        self.z2 = if z2.abs() < 1e-18 { 0.0 } else { z2 };
        if y.abs() < 1e-18 { 0.0 } else { y }
    }
}

const SMOOTH_SUBBLOCK_FRAMES: usize = 128;
const SMOOTH_ALPHA: f32 = 0.08;

/// Stereo/multi-channel 10-band parametric EQ processor (zero heap allocations in audio loop).
#[derive(Debug, Clone)]
pub struct TenBandEq {
    pub enabled: bool,
    pub auto_headroom: bool,
    sample_rate: f32,
    channels: usize,
    gains_db: [f32; 10],
    target_gains_db: [f32; 10],
    coeffs: [BiquadCoeffs; 10],
    // Per-channel, per-band DF2T state (up to 2 channels inline; Vec for >2)
    states: Vec<[BiquadState; 10]>,
}

impl TenBandEq {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let sr = (sample_rate.max(8_000)) as f32;
        let ch = channels.max(1);
        let coeffs = [BiquadCoeffs::passthrough(); 10];
        Self {
            enabled: false,
            auto_headroom: true,
            sample_rate: sr,
            channels: ch,
            gains_db: [0.0; 10],
            target_gains_db: [0.0; 10],
            coeffs,
            states: vec![[BiquadState::default(); 10]; ch],
        }
    }

    pub fn set_gains_db(&mut self, gains_db: [f32; 10]) {
        for (i, &g) in gains_db.iter().enumerate() {
            let clamped = g.clamp(MIN_BAND_DB, MAX_BAND_DB);
            self.gains_db[i] = clamped;
            self.target_gains_db[i] = clamped;
            self.coeffs[i] =
                BiquadCoeffs::peaking_eq(self.sample_rate, EQ_BAND_FREQS_HZ[i], DEFAULT_Q, clamped);
        }
    }

    /// Sets target gains for smooth click-free coefficient ramping over ~15 ms sub-blocks.
    pub fn set_target_gains_db(&mut self, gains_db: [f32; 10]) {
        for (i, &g) in gains_db.iter().enumerate() {
            self.target_gains_db[i] = g.clamp(MIN_BAND_DB, MAX_BAND_DB);
        }
    }

    pub fn set_band_db(&mut self, band_index: usize, gain_db: f32) {
        if band_index >= 10 {
            return;
        }
        let clamped = gain_db.clamp(MIN_BAND_DB, MAX_BAND_DB);
        self.gains_db[band_index] = clamped;
        self.target_gains_db[band_index] = clamped;
        self.coeffs[band_index] = BiquadCoeffs::peaking_eq(
            self.sample_rate,
            EQ_BAND_FREQS_HZ[band_index],
            DEFAULT_Q,
            clamped,
        );
    }

    pub fn apply_preset(&mut self, preset: EqPreset) {
        self.enabled = preset != EqPreset::Flat;
        self.set_gains_db(preset.gains_db());
    }

    pub fn gains_db(&self) -> [f32; 10] {
        self.gains_db
    }

    /// Returns linear pre-amp headroom attenuation (`<= 1.0`) compensating for positive EQ boost.
    pub fn preamp_linear(&self) -> f32 {
        if !self.enabled || !self.auto_headroom {
            return 1.0;
        }
        let max_pos = self.gains_db.iter().copied().fold(0.0_f32, f32::max);
        if max_pos <= 0.05 {
            1.0
        } else {
            10.0_f32.powf(-max_pos / 20.0)
        }
    }

    pub fn reset_state(&mut self) {
        for ch in &mut self.states {
            *ch = [BiquadState::default(); 10];
        }
    }

    fn step_smoothing(&mut self) {
        for i in 0..10 {
            let diff = self.target_gains_db[i] - self.gains_db[i];
            if diff.abs() > 0.01 {
                self.gains_db[i] += diff * SMOOTH_ALPHA;
                self.coeffs[i] = BiquadCoeffs::peaking_eq(
                    self.sample_rate,
                    EQ_BAND_FREQS_HZ[i],
                    DEFAULT_Q,
                    self.gains_db[i],
                );
            } else if diff != 0.0 {
                self.gains_db[i] = self.target_gains_db[i];
                self.coeffs[i] = BiquadCoeffs::peaking_eq(
                    self.sample_rate,
                    EQ_BAND_FREQS_HZ[i],
                    DEFAULT_Q,
                    self.gains_db[i],
                );
            }
        }
    }

    /// Processes interleaved PCM frames in-place with zero allocations.
    pub fn process_interleaved(&mut self, interleaved: &mut [f32]) {
        if !self.enabled {
            return;
        }
        let ch_count = self.channels;
        let subblock_samples = SMOOTH_SUBBLOCK_FRAMES * ch_count;
        for subblock in interleaved.chunks_mut(subblock_samples) {
            self.step_smoothing();
            for frame in subblock.chunks_exact_mut(ch_count) {
                for (ch_idx, sample) in frame.iter_mut().enumerate() {
                    let mut x = *sample;
                    let st = &mut self.states[ch_idx];
                    for band in 0..10 {
                        if self.gains_db[band].abs() >= 0.01 {
                            x = st[band].process(&self.coeffs[band], x);
                        }
                    }
                    *sample = x;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boosts_target_band_sine_wave_by_expected_db() {
        let sr = 48_000;
        let mut eq = TenBandEq::new(sr, 1);
        eq.enabled = true;
        // Boost 1000 Hz (band 5) by +6 dB
        eq.set_band_db(5, 6.0);

        let n = 4800;
        let mut signal: Vec<f32> = (0..n)
            .map(|i| 0.25 * (2.0 * PI * 1000.0 * (i as f32) / (sr as f32)).sin())
            .collect();
        let rms_in = (signal[n / 2..].iter().map(|x| x * x).sum::<f32>() / (n / 2) as f32).sqrt();

        eq.process_interleaved(&mut signal);
        let rms_out = (signal[n / 2..].iter().map(|x| x * x).sum::<f32>() / (n / 2) as f32).sqrt();
        let measured_db = 20.0 * (rms_out / rms_in).log10();
        assert!(
            (measured_db - 6.0).abs() < 0.35,
            "expected ~+6 dB boost at 1kHz, got {measured_db:.2} dB"
        );
    }
}
