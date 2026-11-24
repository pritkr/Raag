//! Dynamic Artwork-Driven Ambient Color Extraction (Off-UI-Thread & Cached by Track ID).
//!
//! Extracts dominant and muted atmospheric backdrop colors from album artwork RGB pixels
//! (quantized saturation-weighted bucket clustering) or computes a deterministic curated
//! editorial palette for monogram/fallback artwork. Every dark/light ambient tone is
//! luminance-clamped so foreground typography strictly preserves WCAG AA contrast (`>= 4.5:1`).

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};

/// Calibrated atmospheric ambient palette for one track, safe for both Dark and Light themes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbientPalette {
    /// Deep atmospheric backdrop tint for Dark mode (`#RRGGBB` -> `[r, g, b]`).
    pub dark_backdrop: [u8; 3],
    /// Subtle elevated glow / artwork halo accent for Dark mode.
    pub dark_accent: [u8; 3],
    /// Soft tinted parchment/glass backdrop for Light mode.
    pub light_backdrop: [u8; 3],
    /// Muted editorial accent tint for Light mode.
    pub light_accent: [u8; 3],
}

impl Default for AmbientPalette {
    fn default() -> Self {
        Self {
            dark_backdrop: [26, 15, 20],
            dark_accent: [86, 26, 38],
            light_backdrop: [250, 241, 242],
            light_accent: [236, 205, 210],
        }
    }
}

/// Curated editorial palettes (dark_backdrop, dark_accent, light_backdrop, light_accent)
/// calibrated so white text (`#f4f5f7`) exceeds 12:1 on dark_backdrop and ink (`#111214`)
/// exceeds 13:1 on light_backdrop.
const EDITORIAL_PALETTES: [AmbientPalette; 6] = [
    // Deep Crimson Velvet
    AmbientPalette {
        dark_backdrop: [30, 14, 19],
        dark_accent: [98, 28, 42],
        light_backdrop: [252, 240, 242],
        light_accent: [242, 208, 214],
    },
    // Midnight Tokyo Indigo
    AmbientPalette {
        dark_backdrop: [15, 19, 34],
        dark_accent: [38, 54, 108],
        light_backdrop: [238, 242, 252],
        light_accent: [208, 218, 244],
    },
    // Warm Amber Tape
    AmbientPalette {
        dark_backdrop: [31, 22, 13],
        dark_accent: [96, 60, 24],
        light_backdrop: [252, 245, 235],
        light_accent: [242, 222, 194],
    },
    // Deep Subterranean Emerald
    AmbientPalette {
        dark_backdrop: [13, 27, 24],
        dark_accent: [26, 82, 70],
        light_backdrop: [237, 249, 245],
        light_accent: [204, 234, 224],
    },
    // Nocturne Plum
    AmbientPalette {
        dark_backdrop: [25, 15, 32],
        dark_accent: [76, 38, 104],
        light_backdrop: [246, 240, 252],
        light_accent: [226, 210, 244],
    },
    // Slate Cobalt
    AmbientPalette {
        dark_backdrop: [14, 23, 30],
        dark_accent: [34, 68, 94],
        light_backdrop: [238, 246, 251],
        light_accent: [208, 226, 240],
    },
];

static PALETTE_CACHE: OnceLock<RwLock<HashMap<String, AmbientPalette>>> = OnceLock::new();

fn cache() -> &'static RwLock<HashMap<String, AmbientPalette>> {
    PALETTE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Returns a cached ambient palette for `track_id`, computing a deterministic editorial
/// palette if real artwork pixels have not yet been analyzed.
pub fn palette_for_track(track_id: &str, seed_text: &str) -> AmbientPalette {
    if let Ok(guard) = cache().read()
        && let Some(hit) = guard.get(track_id)
    {
        return *hit;
    }

    let mut hash: u64 = 14695981039346656037;
    for b in track_id.bytes().chain(seed_text.bytes()) {
        hash ^= b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    let palette = EDITORIAL_PALETTES[(hash as usize) % EDITORIAL_PALETTES.len()];
    if let Ok(mut guard) = cache().write() {
        guard.insert(track_id.to_owned(), palette);
    }
    palette
}

/// Extracts dominant & muted ambient tones from raw RGB24 pixel samples (`[r, g, b, ...]`)
/// using fast 4-bit quantized saturation-weighted bucket clustering, clamps luminance for
/// WCAG AA text contrast in both Dark and Light themes, and caches by `track_id`.
pub fn extract_and_cache_from_rgb(track_id: &str, rgb_pixels: &[u8]) -> AmbientPalette {
    if rgb_pixels.len() < 3 {
        return palette_for_track(track_id, track_id);
    }

    // 16x16x16 quantized RGB histogram weighted by chroma (saturation) so vibrant album art
    // colors dominate over grey borders/letterboxing.
    let mut buckets: HashMap<(u8, u8, u8), (u64, u64, u64, u64)> = HashMap::new();
    let step = (rgb_pixels.len() / 3 / 512).max(1);
    let mut idx = 0;
    while idx + 2 < rgb_pixels.len() {
        let r = rgb_pixels[idx];
        let g = rgb_pixels[idx + 1];
        let b = rgb_pixels[idx + 2];
        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        let chroma = (max_c - min_c) as u64;
        let weight = 1 + chroma * 2;
        let key = (r >> 4, g >> 4, b >> 4);
        let entry = buckets.entry(key).or_insert((0, 0, 0, 0));
        entry.0 += r as u64 * weight;
        entry.1 += g as u64 * weight;
        entry.2 += b as u64 * weight;
        entry.3 += weight;
        idx += step * 3;
    }

    let dominant = buckets
        .into_values()
        .max_by_key(|v| v.3)
        .filter(|v| v.3 > 0)
        .map(|(rs, gs, bs, w)| [(rs / w) as u8, (gs / w) as u8, (bs / w) as u8])
        .unwrap_or([180, 48, 68]);

    let palette = calibrate_ambient_palette(dominant);
    if let Ok(mut guard) = cache().write() {
        guard.insert(track_id.to_owned(), palette);
    }
    palette
}

/// Calibrates any extracted dominant `[r, g, b]` into WCAG-safe Dark & Light ambient surfaces.
pub fn calibrate_ambient_palette(rgb: [u8; 3]) -> AmbientPalette {
    let blend = |base: [u8; 3], tint: [u8; 3], alpha: f32| -> [u8; 3] {
        [
            ((base[0] as f32) * (1.0 - alpha) + (tint[0] as f32) * alpha)
                .round()
                .clamp(0.0, 255.0) as u8,
            ((base[1] as f32) * (1.0 - alpha) + (tint[1] as f32) * alpha)
                .round()
                .clamp(0.0, 255.0) as u8,
            ((base[2] as f32) * (1.0 - alpha) + (tint[2] as f32) * alpha)
                .round()
                .clamp(0.0, 255.0) as u8,
        ]
    };

    AmbientPalette {
        // Dark mode: 14% dominant tint mixed into obsidian (#0a0b0d) -> always >= 11:1 with #f4f5f7
        dark_backdrop: blend([10, 11, 13], rgb, 0.14),
        // Dark accent halo: 36% dominant tint mixed into #14161c
        dark_accent: blend([20, 22, 28], rgb, 0.36),
        // Light mode: 8% dominant tint mixed into warm alabaster (#faf9f7) -> always >= 13:1 with #111214
        light_backdrop: blend([250, 249, 247], rgb, 0.08),
        // Light accent halo: 22% dominant tint mixed into #f0eee9
        light_accent: blend([240, 238, 233], rgb, 0.22),
    }
}
