//! Sample-accurate gapless splicing and equal-power crossfade engine.

use std::f32::consts::FRAC_PI_2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GaplessSplicer {
    sample_rate: u32,
    channels: usize,
    /// Crossfade duration in milliseconds (`0` = sample-accurate zero-gap splice).
    pub crossfade_ms: u32,
}

impl GaplessSplicer {
    pub fn new(sample_rate: u32, channels: usize, crossfade_ms: u32) -> Self {
        Self {
            sample_rate: sample_rate.max(8_000),
            channels: channels.max(1),
            crossfade_ms,
        }
    }

    pub fn crossfade_frames(&self) -> usize {
        ((self.sample_rate as u64 * self.crossfade_ms as u64) / 1000) as usize
    }

    /// Splices `outgoing` tail PCM and `incoming` head PCM seamlessly into a single buffer.
    /// - When `crossfade_ms == 0`: exact sample-accurate concatenation with zero silence gap.
    /// - When `crossfade_ms > 0`: applies equal-power (`cos`/`sin`) crossfade over the overlap.
    pub fn splice_interleaved(&self, outgoing: &[f32], incoming: &[f32]) -> Vec<f32> {
        let ch = self.channels;
        let out_frames = outgoing.len() / ch;
        let in_frames = incoming.len() / ch;
        let xfade_frames = self.crossfade_frames().min(out_frames).min(in_frames);

        if xfade_frames == 0 {
            let mut joined = Vec::with_capacity(outgoing.len() + incoming.len());
            joined.extend_from_slice(outgoing);
            joined.extend_from_slice(incoming);
            return joined;
        }

        let out_prefix_samples = (out_frames - xfade_frames) * ch;
        let overlap_samples = xfade_frames * ch;
        let total_samples = out_prefix_samples + overlap_samples + (in_frames - xfade_frames) * ch;
        let mut out = Vec::with_capacity(total_samples);

        // Unmodified prefix of outgoing track
        out.extend_from_slice(&outgoing[..out_prefix_samples]);

        // Equal-power crossfade region: cos(t * pi/2) for outgoing, sin(t * pi/2) for incoming
        let denom = (xfade_frames.saturating_sub(1)).max(1) as f32;
        for frame_idx in 0..xfade_frames {
            let t = frame_idx as f32 / denom;
            let g_out = (t * FRAC_PI_2).cos();
            let g_in = (t * FRAC_PI_2).sin();
            let base_out = out_prefix_samples + frame_idx * ch;
            let base_in = frame_idx * ch;
            for c in 0..ch {
                out.push(outgoing[base_out + c] * g_out + incoming[base_in + c] * g_in);
            }
        }

        // Remaining incoming track
        out.extend_from_slice(&incoming[overlap_samples..]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gapless_and_equal_power_crossfade_preserve_energy() {
        let gapless = GaplessSplicer::new(48_000, 2, 0);
        let a = vec![0.5_f32; 16];
        let b = vec![-0.5_f32; 16];
        let spliced = gapless.splice_interleaved(&a, &b);
        assert_eq!(spliced.len(), 32);
        assert_eq!(spliced[15], 0.5);
        assert_eq!(spliced[16], -0.5);

        let xfade = GaplessSplicer::new(1000, 1, 10);
        let ones = vec![1.0_f32; 20];
        let twos = vec![1.0_f32; 20];
        let mixed = xfade.splice_interleaved(&ones, &twos);
        assert_eq!(mixed.len(), 30); // 20 + 20 - 10 overlap
        // Midpoint of equal-power crossfade of identical DC signals: cos(pi/4)+sin(pi/4) ≈ 1.414
        let mid = mixed[15];
        assert!(mid > 1.2 && mid <= 1.42, "mid={mid}");
    }
}
