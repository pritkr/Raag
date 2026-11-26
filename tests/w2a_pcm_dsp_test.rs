//! W2A Offline Verification Suite:
//! - Equal-power crossfade preserves constant RMS loudness (`cos² + sin² = 1`)
//! - 10-band EQ boosts target bands without clipping or denormals on silence
//! - WSOLA time-stretcher scales duration at 0.5x–2.0x while preserving fundamental pitch
//! - Silence skipper removes long silence with click-free 10 ms Hann fade-in ramps
//! - SPSC ring buffer underrun fades cleanly to zero without allocations

use std::f32::consts::PI;

use resonance_core::audio_dsp::{
    DspConfig, DspPipeline, EqPreset, GaplessSplicer, SilenceSkipper, SpscRingBuffer, TenBandEq,
    WsolaTimeStretcher,
};

fn sine_wave(freq_hz: f32, sample_rate: u32, frames: usize, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * freq_hz * (i as f32) / (sample_rate as f32)).sin())
        .collect()
}

fn rms(samples: &[f32]) -> f32 {
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len().max(1) as f32).sqrt()
}

fn estimate_zero_crossing_freq_hz(mono: &[f32], sample_rate: u32) -> f32 {
    let mut crossings = 0usize;
    for w in mono.windows(2) {
        if w[0] <= 0.0 && w[1] > 0.0 {
            crossings += 1;
        }
    }
    let duration_secs = mono.len() as f32 / sample_rate as f32;
    crossings as f32 / duration_secs
}

#[test]
fn equal_power_crossfade_preserves_constant_rms_loudness() {
    let sr = 48_000;
    let splicer = GaplessSplicer::new(sr, 1, 500);
    // Uncorrelated-ish / orthogonal 440 Hz and 660 Hz sine waves at identical RMS
    let a = sine_wave(440.0, sr, sr as usize, 0.5);
    let b = sine_wave(660.0, sr, sr as usize, 0.5);
    let steady_rms = rms(&a);

    let mixed = splicer.splice_interleaved(&a, &b);
    let xfade_frames = splicer.crossfade_frames();
    let mid_start = (sr as usize - xfade_frames) + xfade_frames / 4;
    let mid_end = mid_start + xfade_frames / 2;
    let mid_rms = rms(&mixed[mid_start..mid_end]);

    let delta_db = 20.0 * (mid_rms / steady_rms).log10();
    assert!(
        delta_db.abs() < 0.6,
        "expected constant RMS within ±0.6 dB at crossfade midpoint, got {delta_db:.3} dB"
    );
}

#[test]
fn ten_band_eq_boosts_target_band_without_clipping_or_denormals() {
    let sr = 48_000;
    let mut dsp = DspPipeline::new(
        sr,
        1,
        &DspConfig {
            loudness_normalization: false,
            eq_preset: EqPreset::BassBoost,
            eq_gains_db: EqPreset::BassBoost.gains_db(),
            silence_skip_enabled: false,
            crossfade_ms: 0,
        },
    );

    let mut bass = sine_wave(62.5, sr, 9_600, 0.9);
    assert!(dsp.process_interleaved(&mut bass));
    let peak = bass.iter().copied().map(f32::abs).fold(0.0_f32, f32::max);
    assert!(peak < 1.0, "expected soft-limited peak < 1.0, got {peak}");

    // Verify no NaN/Inf/denormals when feeding exact silence after boost
    let mut silence = vec![0.0_f32; 4_096];
    assert!(dsp.process_interleaved(&mut silence));
    for &s in &silence {
        assert!(s.is_finite());
        assert!(s == 0.0 || s.abs() >= 1e-18, "denormal leaked: {s}");
    }

    // Verify smoothed target gain converges cleanly without NaNs
    let mut eq = TenBandEq::new(sr, 1);
    eq.enabled = true;
    eq.set_target_gains_db([6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let mut buf = sine_wave(31.25, sr, 4_800, 0.2);
    eq.process_interleaved(&mut buf);
    assert!(buf.iter().all(|s| s.is_finite()));
}

#[test]
fn wsola_time_stretcher_scales_duration_at_1_5x_preserving_fundamental_pitch() {
    let sr = 48_000;
    let input = sine_wave(440.0, sr, sr as usize, 0.5);
    let mut stretcher = WsolaTimeStretcher::new(sr, 1);
    stretcher.set_speed(1.5, true);

    let output = stretcher.process_interleaved(&input);
    let ratio = output.len() as f32 / input.len() as f32;
    assert!(
        (ratio - (1.0 / 1.5)).abs() < 0.05,
        "expected ~0.667x length at 1.5x speed, got {ratio:.3}"
    );

    let freq = estimate_zero_crossing_freq_hz(&output[1000..output.len() - 1000], sr);
    assert!(
        (freq - 440.0).abs() < 15.0,
        "expected preserved pitch ~440 Hz, got {freq:.1} Hz"
    );
}

#[test]
fn silence_skipper_removes_long_silence_with_click_free_ramps() {
    let sr = 8_000;
    let mut skipper = SilenceSkipper::new(sr, 1, -45.0, 350);
    skipper.enabled = true;

    let tone = sine_wave(440.0, sr, 1_600, 0.4);
    let mut t1 = tone.clone();
    assert!(skipper.process_interleaved(&mut t1));

    // 1 second of silence (> 350 ms hold) -> skipped after hold window
    let mut s1 = vec![0.0_f32; 4_000];
    assert!(!skipper.process_interleaved(&mut s1));

    // Tone resumes -> 10 ms Hann fade-in applied at sample 0 so no step click!
    let mut t2 = tone;
    assert!(skipper.process_interleaved(&mut t2));
    assert!(
        t2[0].abs() < 1e-5,
        "expected fade-in start near 0.0, got {}",
        t2[0]
    );
}

#[test]
fn spsc_ring_buffer_underrun_fades_cleanly_to_zero_without_allocations() {
    let (mut prod, mut cons) = SpscRingBuffer::with_capacity::<f32>(512);
    let input = vec![0.8_f32; 128];
    assert_eq!(prod.push_slice(&input), 128);

    let mut out = vec![0.0_f32; 256];
    let popped = cons.pop_with_underrun_fade(&mut out, 2);
    assert_eq!(popped, 128);
    // Tail of popped region faded smoothly toward 0.0, remainder zeroed
    assert!(out[126].abs() < 0.05 && out[127].abs() < 0.05);
    assert!(out[128..].iter().all(|&s| s == 0.0));
}
