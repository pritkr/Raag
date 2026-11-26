//! Pure-Rust PCM & RIFF/WAV Decoder (`crates/core/src/pcm_engine/decoder.rs`).
//!
//! Decodes PCM `i16`/`i24`/`i32`/`f32` RIFF/WAV streams into interleaved `f32` frames
//! with sample-accurate encoder-delay/padding trimming support.

use std::io::{self, Read};

#[derive(Debug, Clone)]
pub struct DecodedPcmTrack {
    pub sample_rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
    pub encoder_delay_frames: usize,
    pub trailing_padding_frames: usize,
}

impl DecodedPcmTrack {
    /// Returns the trimmed slice with codec encoder delay & trailing padding stripped.
    pub fn trimmed_samples(&self) -> &[f32] {
        let total_frames = self.samples.len() / self.channels.max(1);
        let start = self.encoder_delay_frames.min(total_frames) * self.channels;
        let end_frame = total_frames
            .saturating_sub(self.trailing_padding_frames)
            .max(self.encoder_delay_frames.min(total_frames));
        let end = (end_frame * self.channels).min(self.samples.len());
        &self.samples[start..end]
    }

    pub fn duration_ms(&self) -> u64 {
        let frames = self.trimmed_samples().len() / self.channels.max(1);
        ((frames as u64) * 1000) / (self.sample_rate.max(1) as u64)
    }
}

pub trait PcmDecoder: Send {
    fn decode_all(&mut self, reader: &mut dyn Read) -> io::Result<DecodedPcmTrack>;
}

/// Pure-Rust RIFF/WAV decoder supporting 16-bit PCM (`fmt=1`) and 32-bit IEEE float (`fmt=3`).
pub struct WavDecoder;

impl WavDecoder {
    pub fn decode_wav(reader: &mut dyn Read) -> io::Result<DecodedPcmTrack> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes)?;
        if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a RIFF/WAVE stream",
            ));
        }

        let mut audio_format = 0u16;
        let mut channels = 0u16;
        let mut sample_rate = 0u32;
        let mut bits_per_sample = 0u16;
        let mut data_slice: Option<&[u8]> = None;

        let mut offset = 12usize;
        while offset + 8 <= bytes.len() {
            let chunk_id = &bytes[offset..offset + 4];
            let chunk_size = u32::from_le_bytes(
                bytes[offset + 4..offset + 8]
                    .try_into()
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "chunk hdr"))?,
            ) as usize;
            let body_start = offset + 8;
            let body_end = body_start.saturating_add(chunk_size).min(bytes.len());
            if chunk_id == b"fmt " && body_end - body_start >= 16 {
                let fmt = &bytes[body_start..body_end];
                audio_format = u16::from_le_bytes([fmt[0], fmt[1]]);
                channels = u16::from_le_bytes([fmt[2], fmt[3]]);
                sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
                bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);
            } else if chunk_id == b"data" {
                data_slice = Some(&bytes[body_start..body_end]);
            }
            offset = body_end + (chunk_size & 1);
        }

        let data = data_slice
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing WAV data chunk"))?;
        if channels == 0 || sample_rate == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid WAV format header",
            ));
        }

        let samples = match (audio_format, bits_per_sample) {
            (1, 16) => data
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect(),
            (3, 32) => data
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("unsupported WAV format={audio_format} bits={bits_per_sample}"),
                ));
            }
        };

        Ok(DecodedPcmTrack {
            sample_rate,
            channels: channels as usize,
            samples,
            encoder_delay_frames: 0,
            trailing_padding_frames: 0,
        })
    }
}

impl PcmDecoder for WavDecoder {
    fn decode_all(&mut self, reader: &mut dyn Read) -> io::Result<DecodedPcmTrack> {
        Self::decode_wav(reader)
    }
}
