//! Data-driven InnerTube client viability matrix & PO-token policy engine.
//!
//! Replaces hard-coded client chains with a remote-updatable `client_matrix.json`
//! generated/refreshed by the nightly `client_probe` binary.

use std::fs;
use std::path::Path;

use resonance_contract::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

use crate::innertube::clients::{AuthMode, ClientConfig, ClientKind};

/// Embedded baseline matrix shipped with the app; overridden at runtime whenever
/// a fresher `client_matrix.json` exists in `<data_dir>/config/client_matrix.json`.
pub const EMBEDDED_CLIENT_MATRIX_JSON: &str =
    include_str!("../../../../config/client_matrix.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoTokenPolicy {
    NotRequired,
    #[serde(rename = "required")]
    Required,
    /// Web clients (`WEB_REMIX`) require a GVS PO token for free/anonymous sessions,
    /// while YouTube Music Premium subscribers bypass the GVS PO token requirement.
    RequiredForWebFreeBypassedByPremium,
    /// iOS clients require an `iOSGuard` PoToken.
    RequiredIosGuard,
    Optional,
}

impl PoTokenPolicy {
    pub fn requires_gvs_po_token(self, tier: AccountTier) -> bool {
        match self {
            Self::Required => true,
            Self::RequiredForWebFreeBypassedByPremium => !tier.is_premium(),
            Self::NotRequired | Self::Optional | Self::RequiredIosGuard => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountTier {
    Anonymous,
    FreeSignedIn,
    PremiumSignedIn,
}

impl AccountTier {
    pub fn is_signed_in(self) -> bool {
        !matches!(self, Self::Anonymous)
    }

    pub fn is_premium(self) -> bool {
        matches!(self, Self::PremiumSignedIn)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeVideoSpec {
    pub id: String,
    pub label: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMatrixEntry {
    pub kind: String,
    pub enabled: bool,
    pub priority_signed_in: u32,
    pub priority_anonymous: u32,
    pub auth_mode: String,
    pub po_token_policy: PoTokenPolicy,
    pub requires_ejs_solver: bool,
    pub probe_pass_cookies_on: bool,
    pub probe_pass_cookies_off: bool,
    #[serde(default)]
    pub notes: String,
}

impl ClientMatrixEntry {
    pub fn client_kind(&self) -> Option<ClientKind> {
        match self.kind.as_str() {
            "web_remix" => Some(ClientKind::WebRemix),
            "vision_os" => Some(ClientKind::VisionOs),
            "android_vr" => Some(ClientKind::AndroidVr),
            "tv_samsung" => Some(ClientKind::TvSamsung),
            "ios" => Some(ClientKind::Ios),
            _ => None,
        }
    }

    /// Evaluates whether this client can legally satisfy a playback request under
    /// the current account tier and PO-token availability.
    pub fn is_eligible(
        &self,
        tier: AccountTier,
        has_gvs_po_token: bool,
        has_ios_guard_po_token: bool,
        has_tv_oauth: bool,
    ) -> bool {
        if !self.enabled {
            return false;
        }
        let Some(kind) = self.client_kind() else {
            return false;
        };
        let cfg = ClientConfig::for_kind(kind);

        // Enforce strict per-client auth isolation
        match cfg.auth_mode {
            AuthMode::TvOAuthOnly if !has_tv_oauth => return false,
            AuthMode::AnonymousOnly if tier.is_signed_in() => {
                // Anonymous-only clients can still be used as an unauthenticated fallback
                // ONLY if probe_pass_cookies_off is true, and cookies are NEVER sent.
                if !self.probe_pass_cookies_off {
                    return false;
                }
            }
            _ => {}
        }

        // Enforce explicit PO-token policy
        match self.po_token_policy {
            PoTokenPolicy::NotRequired | PoTokenPolicy::Optional => true,
            PoTokenPolicy::Required => has_gvs_po_token,
            PoTokenPolicy::RequiredForWebFreeBypassedByPremium => {
                tier.is_premium() || has_gvs_po_token
            }
            PoTokenPolicy::RequiredIosGuard => has_ios_guard_po_token,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMatrix {
    pub schema_version: u32,
    pub updated_at_unix_secs: u64,
    pub source: String,
    #[serde(default)]
    pub po_token_policy_notes: String,
    #[serde(default)]
    pub probe_videos: Vec<ProbeVideoSpec>,
    pub clients: Vec<ClientMatrixEntry>,
}

impl ClientMatrix {
    /// Loads the embedded default `client_matrix.json`.
    pub fn embedded_default() -> CoreResult<Self> {
        serde_json::from_str(EMBEDDED_CLIENT_MATRIX_JSON).map_err(|e| {
            CoreError::Provider(resonance_contract::ProviderError::Parse(format!(
                "invalid embedded client_matrix.json: {e}"
            )))
        })
    }

    /// Loads a remote-updated `client_matrix.json` from `override_path` if present,
    /// falling back cleanly to the embedded baseline if missing or corrupted.
    pub fn load_with_override(override_path: &Path) -> CoreResult<Self> {
        if override_path.exists()
            && let Ok(raw) = fs::read_to_string(override_path)
            && let Ok(parsed) = serde_json::from_str::<Self>(&raw)
        {
            return Ok(parsed);
        }
        Self::embedded_default()
    }

    /// Saves a freshly probed matrix atomically (`.tmp` -> rename).
    pub fn save_atomic(&self, path: &Path) -> CoreResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                CoreError::Cache(resonance_contract::CacheError::Io(format!(
                    "create matrix dir: {e}"
                )))
            })?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| {
            CoreError::Cache(resonance_contract::CacheError::Io(format!(
                "serialize client_matrix: {e}"
            )))
        })?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| {
            CoreError::Cache(resonance_contract::CacheError::Io(format!(
                "write matrix tmp: {e}"
            )))
        })?;
        fs::rename(&tmp, path).map_err(|e| {
            CoreError::Cache(resonance_contract::CacheError::Io(format!(
                "rename matrix: {e}"
            )))
        })?;
        Ok(())
    }

    /// Looks up the matrix entry for a given [`ClientKind`].
    pub fn entry_for(&self, kind: ClientKind) -> Option<&ClientMatrixEntry> {
        self.clients
            .iter()
            .find(|entry| entry.client_kind() == Some(kind))
    }

    /// Computes the ordered playback resolution chain from the matrix for the given
    /// account tier and PO-token state.
    pub fn playback_chain(
        &self,
        tier: AccountTier,
        has_gvs_po_token: bool,
        has_ios_guard_po_token: bool,
        has_tv_oauth: bool,
    ) -> Vec<&'static ClientConfig> {
        let mut candidates: Vec<(u32, &'static ClientConfig)> = self
            .clients
            .iter()
            .filter(|e| {
                e.is_eligible(tier, has_gvs_po_token, has_ios_guard_po_token, has_tv_oauth)
            })
            .filter_map(|e| {
                let kind = e.client_kind()?;
                let prio = if tier.is_signed_in() {
                    e.priority_signed_in
                } else {
                    e.priority_anonymous
                };
                Some((prio, ClientConfig::for_kind(kind)))
            })
            .collect();
        candidates.sort_by_key(|(prio, _)| *prio);
        candidates.into_iter().map(|(_, cfg)| cfg).collect()
    }
}
