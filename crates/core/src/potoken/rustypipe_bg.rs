//! Tier 2 Desktop PO-Token Provider (`rustypipe-botguard` CLI binary).
//!
//! Discovers `rustypipe-botguard` from `RESONANCE_BOTGUARD_BIN`, `<data_dir>/bin/rustypipe-botguard`,
//! or `PATH`. Strictly validates `video_id` / `visitor_data` bindings against `[A-Za-z0-9_=%-]{1,256}`
//! before spawning to prevent CLI flag or shell metacharacter injection.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use resonance_contract::{
    PoToken, PoTokenDiagnostics, PoTokenError, PoTokenHealthStatus, PoTokenProvider,
    PoTokenRequest, PoTokenScope,
};
use serde::Deserialize;

use crate::innertube::auth::redact_secrets;
use crate::unix_secs;

const BOTGUARD_TIMEOUT: Duration = Duration::from_secs(8);
const DEFAULT_BOTGUARD_TTL_SECS: u64 = 6 * 3600;

/// Validates that a BotGuard binding (`video_id` or `visitorData`) contains only safe
/// characters (`[A-Za-z0-9_=%-]{1,256}`) and no leading `-` flag prefix.
pub fn validate_botguard_binding(binding: &str) -> Result<&str, PoTokenError> {
    if binding.is_empty() || binding.len() > 256 {
        return Err(PoTokenError::MintFailed(
            "botguard binding must be 1..=256 characters".to_owned(),
        ));
    }
    if binding.starts_with('-') {
        return Err(PoTokenError::MintFailed(
            "botguard binding must not start with '-'".to_owned(),
        ));
    }
    let valid = binding
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'=' | b'%' | b'-'));
    if !valid {
        return Err(PoTokenError::MintFailed(
            "botguard binding contains disallowed characters".to_owned(),
        ));
    }
    Ok(binding)
}

#[derive(Debug, Deserialize)]
struct BotguardCliOutput {
    #[serde(alias = "poToken", alias = "po_token", alias = "token")]
    po_token: String,
    #[serde(default, alias = "expiresAt", alias = "expires_at_unix_secs")]
    expires_at: Option<u64>,
}

/// Optional Tier 2 Desktop provider wrapping `rustypipe-botguard`.
pub struct RustyPipeBotguardProvider {
    binary_path: PathBuf,
}

impl RustyPipeBotguardProvider {
    pub fn new(binary_path: impl Into<PathBuf>) -> Self {
        Self {
            binary_path: binary_path.into(),
        }
    }

    /// Discovers `rustypipe-botguard` from `RESONANCE_BOTGUARD_BIN`, `<data_dir>/bin/`, or `PATH`.
    pub fn discover(data_dir: Option<&Path>) -> Option<Self> {
        if let Ok(env_bin) = std::env::var("RESONANCE_BOTGUARD_BIN") {
            let p = PathBuf::from(env_bin.trim());
            if p.is_file() {
                return Some(Self::new(p));
            }
        }
        if let Some(dir) = data_dir {
            let candidate = dir.join("bin").join(bin_name());
            if candidate.is_file() {
                return Some(Self::new(candidate));
            }
        }
        if let Ok(path_var) = std::env::var("PATH") {
            for entry in std::env::split_paths(&path_var) {
                let candidate = entry.join(bin_name());
                if candidate.is_file() {
                    return Some(Self::new(candidate));
                }
            }
        }
        None
    }

    pub fn binary_path(&self) -> &Path {
        &self.binary_path
    }
}

fn bin_name() -> &'static str {
    if cfg!(windows) {
        "rustypipe-botguard.exe"
    } else {
        "rustypipe-botguard"
    }
}

#[async_trait]
impl PoTokenProvider for RustyPipeBotguardProvider {
    async fn mint_token(&self, req: &PoTokenRequest) -> Result<PoToken, PoTokenError> {
        let raw_binding = match req.scope {
            PoTokenScope::Player | PoTokenScope::Subs => req.video_id.as_str(),
            PoTokenScope::Gvs => req.identity_key().unwrap_or(req.video_id.as_str()),
        };
        let binding = validate_botguard_binding(raw_binding)?.to_owned();

        #[cfg(target_os = "android")]
        {
            let _ = binding;
            return Err(PoTokenError::Unavailable(
                "rustypipe-botguard is desktop-only".to_owned(),
            ));
        }

        #[cfg(not(target_os = "android"))]
        {
            let mut cmd = tokio::process::Command::new(&self.binary_path);
            cmd.arg("--output")
                .arg("json")
                .arg("--")
                .arg(&binding)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let output = tokio::time::timeout(BOTGUARD_TIMEOUT, cmd.output())
                .await
                .map_err(|_| PoTokenError::Timeout {
                    timeout_ms: BOTGUARD_TIMEOUT.as_millis() as u64,
                })?
                .map_err(|e| {
                    PoTokenError::Unavailable(redact_secrets(&format!(
                        "rustypipe-botguard spawn failed: {e}"
                    )))
                })?;

            if !output.status.success() {
                let stderr = redact_secrets(&String::from_utf8_lossy(&output.stderr));
                return Err(PoTokenError::MintFailed(format!(
                    "rustypipe-botguard exited with {}: {}",
                    output.status,
                    stderr.trim()
                )));
            }

            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if stdout.is_empty() {
                return Err(PoTokenError::MintFailed(
                    "rustypipe-botguard returned empty output".to_owned(),
                ));
            }

            let (value, expires_at) =
                if let Ok(parsed) = serde_json::from_str::<BotguardCliOutput>(&stdout) {
                    (
                        parsed.po_token,
                        parsed
                            .expires_at
                            .unwrap_or_else(|| unix_secs() + DEFAULT_BOTGUARD_TTL_SECS),
                    )
                } else {
                    (stdout, unix_secs() + DEFAULT_BOTGUARD_TTL_SECS)
                };

            if value.trim().is_empty() {
                return Err(PoTokenError::MintFailed(
                    "rustypipe-botguard returned blank poToken".to_owned(),
                ));
            }

            Ok(PoToken {
                value,
                scope: req.scope,
                expires_at_unix_secs: expires_at,
            })
        }
    }

    fn invalidate(&self, _scope: PoTokenScope) {}

    fn provider_name(&self) -> &'static str {
        "rustypipe-botguard"
    }

    fn diagnostics(&self) -> PoTokenDiagnostics {
        PoTokenDiagnostics {
            status: PoTokenHealthStatus::Healthy,
            active_provider: "rustypipe-botguard".to_owned(),
            detail: format!("Binary: {}", self.binary_path.display()),
            degraded_fallback_active: false,
            last_checked_unix_secs: unix_secs(),
        }
    }
}
