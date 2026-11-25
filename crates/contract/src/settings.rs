//! Persisted user settings contract (`SettingsV1`).

use serde::{Deserialize, Serialize};

pub const SETTINGS_METADATA_KEY: &str = "settings:v1";
/// Effectively non-expiring TTL (100 years) so settings never expire from SWR metadata_cache.
pub const SETTINGS_TTL_SECS: u64 = 100 * 365 * 24 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedTheme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedEqPreset {
    #[default]
    Flat,
    BassBoost,
    VocalBoost,
    TrebleBoost,
    LateNight,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsV1 {
    pub schema_version: u32,
    pub theme: PersistedTheme,
    pub reduced_motion: bool,
    pub sponsorblock_enabled: bool,
    pub sponsorblock_categories: Vec<String>,
    pub loudness_normalization: bool,
    pub silence_skip_enabled: bool,
    pub eq_preset: PersistedEqPreset,
    pub eq_gains_db: [f32; 10],
    pub crossfade_ms: u32,
    pub offline_mode: bool,
    pub volume: f32,                   // clamped 0.0..=1.0
    pub offline_quota_bytes: u64,      // default 5 GiB; 0 = Unlimited (consumed by H2.2)
    pub update_channel: String,        // "stable" | "beta" (consumed by LT-2)
    pub local_music_dirs: Vec<String>, // persisted folder roots (consumed by H3.2)
}

impl Default for SettingsV1 {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: PersistedTheme::System,
            reduced_motion: false,
            sponsorblock_enabled: true,
            sponsorblock_categories: vec![
                "music_offtopic".into(),
                "sponsor".into(),
                "intro".into(),
                "outro".into(),
            ],
            loudness_normalization: true,
            silence_skip_enabled: false,
            eq_preset: PersistedEqPreset::Flat,
            eq_gains_db: [0.0; 10],
            crossfade_ms: 0,
            offline_mode: false,
            volume: 0.85,
            offline_quota_bytes: 5 * 1024 * 1024 * 1024,
            update_channel: "stable".into(),
            local_music_dirs: Vec::new(),
        }
    }
}
