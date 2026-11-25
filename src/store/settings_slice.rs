//! Settings, Theme, Auth credentials status, Audio DSP config, and PO Token diagnostics slice.

use resonance_contract::{
    AuthBackendStatus, CredentialBackendKind, PersistedEqPreset, PersistedTheme,
    PoTokenDiagnostics, PoTokenHealthStatus, SettingsV1,
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

#[derive(Clone)]
pub struct SettingsSlice {
    pub theme_choice: ThemeChoice,
    pub reduced_motion: bool,
    pub signed_in: bool,
    pub account_name: Option<String>,
    pub auth_backend_status: AuthBackendStatus,
    pub dsp_config: DspConfig,
    pub po_token_diagnostics: PoTokenDiagnostics,
    pub offline_quota_bytes: u64,
    pub update_channel: String,
    pub local_music_dirs: Vec<String>,
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

        playback.player.sponsorblock_enabled = persisted.sponsorblock_enabled;
        playback.player.offline_mode = persisted.offline_mode;
        playback.player.volume = persisted.volume.clamp(0.0, 1.0);
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
        settings.offline_mode = playback.player.offline_mode;
        settings.volume = playback.player.volume.clamp(0.0, 1.0);
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
        }
        self.auth_backend_status = status;
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
