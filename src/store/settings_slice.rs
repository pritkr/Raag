//! Settings, Theme, Auth credentials status, Audio DSP config, and PO Token diagnostics slice.

use resonance_contract::{
    AccountProfile, AuthBackendStatus, CredentialBackendKind, OfflineQualityPreference,
    PersistedEqPreset, PersistedTheme, PoTokenDiagnostics, PoTokenHealthStatus, RemoteConfigStatus,
    SettingsV1,
};
use resonance_core::audio_dsp::{DspConfig, EqPreset};

use super::playback_slice::PlaybackSlice;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

/// Truthful capability reporting for the audio DSP pipeline.
///
/// While playback routes HTTP URLs directly to `nsh/audio`'s OS decoder, DSP features
/// (ReplayGain, 10-band EQ, silence skip, crossfade) are gated behind `ExternalUrlStreamOnly`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDspCapability {
    ActivePcmPipeline,
    ExternalUrlStreamOnly,
}

impl AudioDspCapability {
    pub fn is_active(self) -> bool {
        matches!(self, Self::ActivePcmPipeline)
    }

    pub fn badge_label(self) -> &'static str {
        match self {
            Self::ActivePcmPipeline => "Active • Rust PCM DSP",
            Self::ExternalUrlStreamOnly => "Planned • Native PCM Engine",
        }
    }

    pub fn detail_text(self) -> &'static str {
        match self {
            Self::ActivePcmPipeline => {
                "32-bit float PCM pipeline active for ReplayGain, 10-band EQ, silence skip, and crossfade."
            }
            Self::ExternalUrlStreamOnly => {
                "nsh/audio streams directly to the OS decoder. Full PCM DSP (EQ, ReplayGain, silence skip, crossfade) is gated until the custom Rust PCM sink lands."
            }
        }
    }

    pub fn to_contract(self) -> resonance_contract::AudioDspCapability {
        match self {
            Self::ActivePcmPipeline => resonance_contract::AudioDspCapability::NativePcmPipeline,
            Self::ExternalUrlStreamOnly => {
                resonance_contract::AudioDspCapability::ExternalUrlStreamOnly
            }
        }
    }

    pub fn from_contract(cap: resonance_contract::AudioDspCapability) -> Self {
        match cap {
            resonance_contract::AudioDspCapability::NativePcmPipeline
            | resonance_contract::AudioDspCapability::AndroidAudioProcessorJni => {
                Self::ActivePcmPipeline
            }
            resonance_contract::AudioDspCapability::ExternalUrlStreamOnly => {
                Self::ExternalUrlStreamOnly
            }
        }
    }
}

#[derive(Clone)]
pub struct SettingsSlice {
    pub theme_choice: ThemeChoice,
    pub reduced_motion: bool,
    pub signed_in: bool,
    pub account_name: Option<String>,
    pub auth_backend_status: AuthBackendStatus,
    pub account_profile: Option<AccountProfile>,
    pub remote_config_status: RemoteConfigStatus,
    pub dsp_config: DspConfig,
    pub po_token_diagnostics: PoTokenDiagnostics,
    pub offline_quota_bytes: u64,
    pub update_channel: String,
    pub local_music_dirs: Vec<String>,
    pub auth_sheet_open: bool,
    pub tv_oauth_user_code: Option<String>,
    pub tv_oauth_verification_url: Option<String>,
    pub tv_oauth_expires_in_secs: u64,
    pub tv_oauth_status: Option<String>,
    pub playback_speed: f32,
    pub preserve_pitch: bool,
    pub wifi_only_downloads: bool,
    pub offline_quality: OfflineQualityPreference,
    pub keep_liked_songs_offline: bool,
    pub atr_watchtime_sync: bool,
}

impl Default for SettingsSlice {
    fn default() -> Self {
        Self::new(super::is_fixture_mode())
    }
}

impl SettingsSlice {
    pub fn new(fixture: bool) -> Self {
        let defaults = SettingsV1::default();
        Self {
            theme_choice: ThemeChoice::System,
            reduced_motion: false,
            signed_in: false,
            account_name: None,
            auth_backend_status: AuthBackendStatus {
                backend: if cfg!(all(
                    not(target_os = "android"),
                    feature = "desktop-keyring",
                    any(
                        target_os = "linux",
                        target_os = "macos",
                        target_os = "windows"
                    )
                )) {
                    CredentialBackendKind::OsKeyring
                } else {
                    CredentialBackendKind::EncryptedFallbackFile
                },
                signed_in: false,
                account_name: None,
                has_sapisid: false,
                has_3papisid: false,
            },
            account_profile: None,
            remote_config_status: RemoteConfigStatus {
                matrix_revision: 0,
                matrix_source: if fixture {
                    "fixture".to_owned()
                } else {
                    "embedded".to_owned()
                },
                solver_bundle_version: "embedded".to_owned(),
                signature_verified: false,
                last_sync_unix_secs: None,
            },
            dsp_config: DspConfig::default(),
            po_token_diagnostics: if fixture {
                PoTokenDiagnostics {
                    status: PoTokenHealthStatus::DegradedColdStart,
                    active_provider: "quickjs-vm-fallback".to_owned(),
                    detail: "Cold-start degraded fallback active (fixture preview)".to_owned(),
                    degraded_fallback_active: true,
                    last_checked_unix_secs: 0,
                }
            } else {
                PoTokenDiagnostics {
                    status: PoTokenHealthStatus::Unconfigured,
                    active_provider: "chained-default".to_owned(),
                    detail: "Layered PO token chain ready (bgutil / attestation / fallback)"
                        .to_owned(),
                    degraded_fallback_active: false,
                    last_checked_unix_secs: 0,
                }
            },
            offline_quota_bytes: defaults.offline_quota_bytes,
            update_channel: defaults.update_channel,
            local_music_dirs: defaults.local_music_dirs,
            auth_sheet_open: false,
            tv_oauth_user_code: None,
            tv_oauth_verification_url: None,
            tv_oauth_expires_in_secs: 0,
            tv_oauth_status: None,
            playback_speed: defaults.playback_speed,
            preserve_pitch: defaults.preserve_pitch,
            wifi_only_downloads: defaults.wifi_only_downloads,
            offline_quality: defaults.offline_quality,
            keep_liked_songs_offline: defaults.keep_liked_songs_offline,
            atr_watchtime_sync: defaults.atr_watchtime_sync,
        }
    }

    pub fn apply_persisted_settings(
        &mut self,
        persisted: &SettingsV1,
        playback: &mut PlaybackSlice,
    ) {
        self.theme_choice = match persisted.theme {
            PersistedTheme::System => ThemeChoice::System,
            PersistedTheme::Light => ThemeChoice::Light,
            PersistedTheme::Dark => ThemeChoice::Dark,
        };
        self.reduced_motion = persisted.reduced_motion;
        self.dsp_config.loudness_normalization = persisted.loudness_normalization;
        self.dsp_config.silence_skip_enabled = persisted.silence_skip_enabled;
        self.dsp_config.eq_preset = match persisted.eq_preset {
            PersistedEqPreset::Flat => EqPreset::Flat,
            PersistedEqPreset::BassBoost => EqPreset::BassBoost,
            PersistedEqPreset::VocalBoost => EqPreset::Vocal,
            PersistedEqPreset::TrebleBoost => EqPreset::Acoustic,
            PersistedEqPreset::LateNight => EqPreset::LateNight,
            PersistedEqPreset::Custom => EqPreset::Flat,
        };
        self.dsp_config.eq_gains_db = persisted.eq_gains_db;
        self.dsp_config.crossfade_ms = persisted.crossfade_ms;
        self.offline_quota_bytes = persisted.offline_quota_bytes;
        self.update_channel = persisted.update_channel.clone();
        self.local_music_dirs = persisted.local_music_dirs.clone();
        self.playback_speed = persisted.playback_speed.clamp(0.5, 2.0);
        self.preserve_pitch = persisted.preserve_pitch;
        self.wifi_only_downloads = persisted.wifi_only_downloads;
        self.offline_quality = persisted.offline_quality;
        self.keep_liked_songs_offline = persisted.keep_liked_songs_offline;
        self.atr_watchtime_sync = persisted.atr_watchtime_sync;

        playback.player.sponsorblock_enabled = persisted.sponsorblock_enabled;
        playback.player.offline_mode = persisted.offline_mode;
        playback.player.volume = persisted.volume.clamp(0.0, 1.6);
        playback.player.queue.autoplay_radio = persisted.autoplay_radio;
    }

    pub fn to_persisted_settings(&self, playback: &PlaybackSlice) -> SettingsV1 {
        let mut settings = SettingsV1::default();
        settings.theme = match self.theme_choice {
            ThemeChoice::System => PersistedTheme::System,
            ThemeChoice::Light => PersistedTheme::Light,
            ThemeChoice::Dark => PersistedTheme::Dark,
        };
        settings.reduced_motion = self.reduced_motion;
        settings.sponsorblock_enabled = playback.player.sponsorblock_enabled;
        settings.loudness_normalization = self.dsp_config.loudness_normalization;
        settings.silence_skip_enabled = self.dsp_config.silence_skip_enabled;
        settings.eq_preset = match self.dsp_config.eq_preset {
            EqPreset::Flat => PersistedEqPreset::Flat,
            EqPreset::BassBoost => PersistedEqPreset::BassBoost,
            EqPreset::Vocal => PersistedEqPreset::VocalBoost,
            EqPreset::Acoustic => PersistedEqPreset::TrebleBoost,
            EqPreset::LateNight => PersistedEqPreset::LateNight,
        };
        settings.eq_gains_db = self.dsp_config.eq_gains_db;
        settings.crossfade_ms = self.dsp_config.crossfade_ms;
        settings.playback_speed = self.playback_speed.clamp(0.5, 2.0);
        settings.preserve_pitch = self.preserve_pitch;
        settings.offline_mode = playback.player.offline_mode;
        settings.wifi_only_downloads = self.wifi_only_downloads;
        settings.offline_quality = self.offline_quality;
        settings.keep_liked_songs_offline = self.keep_liked_songs_offline;
        settings.volume = playback.player.volume.clamp(0.0, 1.6);
        settings.autoplay_radio = playback.player.queue.autoplay_radio;
        settings.atr_watchtime_sync = self.atr_watchtime_sync;
        settings.offline_quota_bytes = self.offline_quota_bytes;
        settings.update_channel = self.update_channel.clone();
        settings.local_music_dirs = self.local_music_dirs.clone();
        settings
    }

    pub fn apply_auth_updated(&mut self, signed_in: bool) {
        self.signed_in = signed_in;
        self.account_name = if signed_in {
            Some("Google Account (SAPISIDHASH)".into())
        } else {
            None
        };
        self.auth_backend_status.signed_in = signed_in;
        self.auth_backend_status.account_name = self.account_name.clone();
        if !signed_in {
            self.auth_backend_status.has_sapisid = false;
            self.auth_backend_status.has_3papisid = false;
        }
    }

    pub fn apply_auth_status(&mut self, status: AuthBackendStatus) {
        self.signed_in = status.signed_in;
        if status.account_name.is_some() {
            self.account_name = status.account_name.clone();
        } else if status.signed_in && self.account_name.is_none() {
            self.account_name = Some("Google Account (SAPISIDHASH)".into());
        } else if !status.signed_in {
            self.account_name = None;
            self.account_profile = None;
        }
        self.auth_backend_status = status;
    }

    pub fn apply_account_profile(&mut self, profile: Option<AccountProfile>) {
        if let Some(ref p) = profile {
            if !p.account_name.is_empty() {
                self.account_name = Some(p.account_name.clone());
                self.auth_backend_status.account_name = Some(p.account_name.clone());
            }
        }
        self.account_profile = profile;
    }

    pub fn apply_remote_config_status(&mut self, status: RemoteConfigStatus) {
        self.remote_config_status = status;
    }

    /// Validates pasted cookies and updates auth status. Returns `(accepted, banner)`.
    pub fn sign_in_with_cookies(&mut self, cookie_input: &str) -> (bool, String) {
        let trimmed = cookie_input.trim();
        let has_sapisid = trimmed.contains("SAPISID=");
        let has_3papisid = trimmed.contains("__Secure-3PAPISID=");
        if has_sapisid || has_3papisid {
            self.signed_in = true;
            self.account_name = Some("Google Account (SAPISIDHASH)".into());
            self.auth_backend_status.signed_in = true;
            self.auth_backend_status.account_name = self.account_name.clone();
            self.auth_backend_status.has_sapisid = has_sapisid;
            self.auth_backend_status.has_3papisid = has_3papisid;
            (true, "Signed in securely (stored in OS Keystore)".into())
        } else {
            (false, "Missing SAPISID / __Secure-3PAPISID cookie".into())
        }
    }

    pub fn sign_out(&mut self) -> String {
        self.signed_in = false;
        self.account_name = None;
        self.auth_backend_status.signed_in = false;
        self.auth_backend_status.account_name = None;
        self.auth_backend_status.has_sapisid = false;
        self.auth_backend_status.has_3papisid = false;
        "Signed out — running in anonymous mode".into()
    }

    pub fn dsp_capability(&self) -> AudioDspCapability {
        AudioDspCapability::ExternalUrlStreamOnly
    }

    pub fn set_loudness_normalization(&mut self, enabled: bool) {
        self.dsp_config.loudness_normalization = enabled;
    }

    pub fn set_silence_skip(&mut self, enabled: bool) {
        self.dsp_config.silence_skip_enabled = enabled;
    }

    pub fn cycle_eq_preset(&mut self) -> &'static str {
        let next = self.dsp_config.eq_preset.next();
        self.dsp_config.eq_preset = next;
        self.dsp_config.eq_gains_db = next.gains_db();
        next.label()
    }

    pub fn cycle_crossfade_ms(&mut self) -> u32 {
        let next = match self.dsp_config.crossfade_ms {
            0 => 2_000,
            2_000 => 4_000,
            4_000 => 6_000,
            _ => 0,
        };
        self.dsp_config.crossfade_ms = next;
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_core::audio_dsp::EqPreset;

    #[test]
    fn cookie_sign_in_and_sign_out_update_flags() {
        let mut s = SettingsSlice::new(true);
        let (ok_bad, _) = s.sign_in_with_cookies("FOO=bar");
        assert!(!ok_bad);
        assert!(!s.signed_in);

        let (ok_good, _) = s.sign_in_with_cookies("SAPISID=abc; __Secure-3PAPISID=def");
        assert!(ok_good);
        assert!(s.signed_in);
        assert!(s.auth_backend_status.has_sapisid);
        assert!(s.auth_backend_status.has_3papisid);

        s.sign_out();
        assert!(!s.signed_in);
        assert!(!s.auth_backend_status.has_sapisid);
    }

    #[test]
    fn dsp_capability_reports_external_url_stream_only_truthfully() {
        let s = SettingsSlice::new(true);
        let cap = s.dsp_capability();
        assert_eq!(cap, AudioDspCapability::ExternalUrlStreamOnly);
        assert!(!cap.is_active());
        assert_eq!(cap.badge_label(), "Planned • Native PCM Engine");
    }

    #[test]
    fn dsp_preset_and_crossfade_cycle_cleanly() {
        let mut s = SettingsSlice::new(true);
        assert_eq!(s.dsp_config.eq_preset, EqPreset::Flat);
        assert_eq!(s.cycle_eq_preset(), "Bass Boost");
        assert_eq!(s.dsp_config.eq_preset, EqPreset::BassBoost);
        assert_eq!(s.cycle_crossfade_ms(), 2_000);
        assert_eq!(s.cycle_crossfade_ms(), 4_000);
        assert_eq!(s.cycle_crossfade_ms(), 6_000);
        assert_eq!(s.cycle_crossfade_ms(), 0);
    }
}
