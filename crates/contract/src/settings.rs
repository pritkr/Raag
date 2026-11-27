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
pub enum EqPreset {
    #[default]
    Flat,
    BassBoost,
    #[serde(alias = "vocal_boost")]
    Vocal,
    #[serde(alias = "treble_boost")]
    Acoustic,
    LateNight,
    Custom,
}

impl EqPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::BassBoost => "Bass Boost",
            Self::Vocal => "Vocal",
            Self::Acoustic => "Acoustic",
            Self::LateNight => "Late Night",
            Self::Custom => "Custom",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Flat => Self::BassBoost,
            Self::BassBoost => Self::Vocal,
            Self::Vocal => Self::Acoustic,
            Self::Acoustic => Self::LateNight,
            Self::LateNight | Self::Custom => Self::Flat,
        }
    }

    pub fn gains_db(self) -> [f32; 10] {
        match self {
            Self::Flat | Self::Custom => [0.0; 10],
            Self::BassBoost => [4.5, 4.0, 3.0, 1.5, 0.0, 0.0, 0.5, 1.0, 1.0, 0.5],
            Self::Vocal => [-1.5, -1.0, 0.0, 1.5, 3.0, 3.5, 2.5, 1.0, 0.0, -0.5],
            Self::Acoustic => [1.5, 1.0, 0.5, 0.0, 1.0, 1.5, 2.0, 2.5, 2.0, 1.0],
            Self::LateNight => [2.0, 1.5, 0.5, 0.0, -0.5, 0.5, 1.0, -1.0, -2.5, -3.5],
        }
    }
}

/// Alias preserving `PersistedEqPreset` naming in persisted settings code.
pub type PersistedEqPreset = EqPreset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineQualityPreference {
    DataSaver64,
    #[default]
    Standard160,
    High256,
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
    pub playback_speed: f32,
    pub preserve_pitch: bool,
    pub offline_mode: bool,
    pub wifi_only_downloads: bool,
    pub offline_quality: OfflineQualityPreference,
    pub keep_liked_songs_offline: bool,
    pub offline_quota_bytes: u64,
    pub volume: f32, // 0.0..=1.6
    pub autoplay_radio: bool,
    pub atr_watchtime_sync: bool,
    pub update_channel: String,        // "stable" | "beta"
    pub local_music_dirs: Vec<String>, // persisted folder roots (consumed by W2C)
}

impl Default for SettingsV1 {
    fn default() -> Self {
        Self {
            schema_version: 1,
            theme: PersistedTheme::System,
            reduced_motion: false,
            sponsorblock_enabled: true,
            sponsorblock_categories: vec!["music_offtopic".into(), "sponsor".into()],
            loudness_normalization: true,
            silence_skip_enabled: false,
            eq_preset: PersistedEqPreset::Flat,
            eq_gains_db: [0.0; 10],
            crossfade_ms: 0,
            playback_speed: 1.0,
            preserve_pitch: true,
            offline_mode: false,
            wifi_only_downloads: cfg!(any(target_os = "android", target_os = "ios")),
            offline_quality: OfflineQualityPreference::High256,
            keep_liked_songs_offline: false,
            offline_quota_bytes: 5 * 1024 * 1024 * 1024,
            volume: 1.0,
            autoplay_radio: true,
            atr_watchtime_sync: true,
            update_channel: "stable".into(),
            local_music_dirs: Vec::new(),
        }
    }
}
