//! Settings, Theme, Auth credentials status, Audio DSP config, and PO Token diagnostics slice.

use resonance_contract::{
    AuthBackendStatus, CredentialBackendKind, PoTokenDiagnostics, PoTokenHealthStatus,
};
use resonance_core::audio_dsp::DspConfig;

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
}

impl SettingsSlice {
    pub fn new(fixture: bool) -> Self {
        Self {
            theme_choice: ThemeChoice::System,
            reduced_motion: false,
            signed_in: false,
            account_name: None,
            auth_backend_status: AuthBackendStatus {
                backend: if cfg!(all(
                    not(target_os = "android"),
                    feature = "desktop-keyring",
                    any(target_os = "linux", target_os = "macos", target_os = "windows")
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
        }
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
